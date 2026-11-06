// Web Serial for the page's UART recovery: a USB-UART adapter on the
// board's J3 pads, at the mask ROM's framing (115200 8E1), handed to the
// WebAssembly (fastprom-web) as a SerialLink of four calls. The same code
// runs over the simulated board's fake Web Serial (sim-serial.js).

/** The port settings recovery uses: fastprom-core recovery::BAUD, 8E1. */
export const SERIAL_OPTIONS = { baudRate: 115200, dataBits: 8, parity: 'even', stopBits: 1, flowControl: 'none' };

function gone() {
  const e = new Error('the serial port went away');
  e.name = 'Gone';
  return e;
}

/** An open port: fastprom-web's `SerialLink`. */
export class SerialLink {
  constructor(port) {
    this.port = port;
    this.buffer = [];
    this.waiters = [];
    this.closed = false;
    this.failed = null;
    this.reader = port.readable.getReader();
    this.writer = port.writable.getWriter();
    this.pump();
  }

  static async open(port) {
    await port.open(SERIAL_OPTIONS);
    return new SerialLink(port);
  }

  /** Everything that arrives goes into the buffer; waiting reads wake. */
  async pump() {
    try {
      for (;;) {
        const { value, done } = await this.reader.read();
        if (done) break;
        for (const b of value) this.buffer.push(b);
        this.wake();
      }
    } catch (e) {
      if (!this.closed) this.failed = e;
    }
    this.closed = true;
    this.wake();
  }

  wake() {
    const w = this.waiters;
    this.waiters = [];
    for (const f of w) f();
  }

  async write(data) {
    if (this.closed) throw gone();
    // A copy: WebAssembly memory may move while the write is in flight.
    try {
      await this.writer.write(new Uint8Array(data));
    } catch {
      throw gone();
    }
  }

  /** At most len bytes: as soon as that many arrived, or what arrived in timeoutMs. */
  async read(len, timeoutMs) {
    const deadline = performance.now() + timeoutMs;
    while (this.buffer.length < len && !this.closed) {
      const left = deadline - performance.now();
      if (left <= 0) break;
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, left);
        this.waiters.push(() => {
          clearTimeout(timer);
          resolve();
        });
      });
    }
    if (this.closed && this.buffer.length === 0) throw gone();
    return new Uint8Array(this.buffer.splice(0, len));
  }

  async discard() {
    this.buffer = [];
  }

  sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  async close() {
    if (this.closed && !this.reader) return;
    this.closed = true;
    try {
      await this.reader.cancel();
    } catch {
      // Already cancelled, or the port went away: nothing to release.
    }
    this.reader.releaseLock();
    this.writer.releaseLock();
    try {
      await this.port.close();
    } catch {
      // As above.
    }
    this.reader = null;
  }
}
