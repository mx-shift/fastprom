// A fake Web Serial (navigator.serial's shape) over the simulated board's
// UART: a USB-UART adapter on J3's fixture. With BOOT0 held high the simulated
// chip's mask-ROM loader answers on it (fastprom-host's model), and after
// the page's GO, fastprom-core's recovery loader. The page talks to it
// exactly as to a real port, through serial.js.

/** How often the fake polls the board when it has nothing to read. */
const POLL_MS = 5;

class FakePort {
  constructor(serial) {
    this.serial = serial;
    this.options = null;
    this.readable = null;
    this.writable = null;
  }

  getInfo() {
    // A CP2102's, as a common 3.3 V adapter reports it.
    return { usbVendorId: 0x10c4, usbProductId: 0xea60 };
  }

  async open(options) {
    if (this.options) throw new DOMException('The port is already open.', 'InvalidStateError');
    this.options = options;
    const sim = this.serial.sim;
    const tick = () => this.serial.tick();
    let open = true;
    let wake = null;
    this.readable = new ReadableStream({
      pull: async (controller) => {
        for (;;) {
          if (!open) {
            controller.close();
            return;
          }
          const b = sim.serialRead(4096);
          if (b.length) {
            controller.enqueue(b);
            return;
          }
          // Quiet on the wire: time passes for the board too, until the
          // page writes (which wakes this at once) or the poll comes round.
          const start = performance.now();
          await new Promise((r) => {
            wake = r;
            setTimeout(r, POLL_MS);
          });
          wake = null;
          sim.serialIdle(Math.round(performance.now() - start));
        }
      },
      cancel: () => {
        open = false;
      },
    });
    this.writable = new WritableStream({
      write: (chunk) => {
        if (!open) throw new DOMException('The port is closed.', 'NetworkError');
        sim.serialWrite(new Uint8Array(chunk));
        wake?.();
        tick();
      },
    });
    this.close = async () => {
      open = false;
      this.options = null;
      this.readable = null;
      this.writable = null;
    };
  }

  async close() {
    this.options = null;
  }
}

/** navigator.serial's shape, with one adapter plugged in. */
export class FakeSerial extends EventTarget {
  constructor(sim, tick) {
    super();
    this.sim = sim;
    this.tick = tick;
    this.port = new FakePort(this);
    this.permitted = false;
  }

  async getPorts() {
    return this.permitted ? [this.port] : [];
  }

  async requestPort() {
    this.permitted = true;
    return this.port;
  }
}
