// WebUSB for the update page: find the update interface, and hand the
// WebAssembly (fastprom-web) a link with the three transfers it needs. The
// protocol itself (frames, sequence numbers, the reset-and-send-again rule)
// is Rust (fastprom-host's client). The same code runs over the simulated
// board's fake WebUSB (sim-usb.js).

/** bcdDevice from WebUSB's three version fields. */
export function bcdOf(d) {
  return (d.deviceVersionMajor << 8) | (d.deviceVersionMinor << 4) | d.deviceVersionSubminor;
}

/**
 * The update interface: the first vendor interface (FF/00/00) with a bulk
 * OUT and a bulk IN endpoint. It is interface 0 in both firmwares, but the
 * number is not assumed (fastprom-host's transport::find_update_interface
 * is the same rule).
 */
export function findUpdateInterface(configuration) {
  for (const iface of configuration.interfaces) {
    const alt = iface.alternate ?? iface.alternates[0];
    if (alt.interfaceClass !== 0xff || alt.interfaceSubclass !== 0 || alt.interfaceProtocol !== 0) {
      continue;
    }
    const bulk = (dir) => alt.endpoints.find((e) => e.type === 'bulk' && e.direction === dir);
    const out = bulk('out');
    const inn = bulk('in');
    if (out && inn) {
      return { interfaceNumber: iface.interfaceNumber, epOut: out.endpointNumber, epIn: inn.endpointNumber };
    }
  }
  return null;
}

/** How long a transfer may take before the page gives up on the device. */
const TIMEOUT_MS = 30000;

function timeout(promise, what) {
  let timer;
  const t = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`the device did not answer (${what})`)), TIMEOUT_MS);
  });
  return Promise.race([promise, t]).finally(() => clearTimeout(timer));
}

/** One claimed update interface: fastprom-web's `Link`. */
export class UpdateLink {
  constructor(device, found) {
    this.device = device;
    this.iface = found.interfaceNumber;
    this.epOut = found.epOut;
    this.epIn = found.epIn;
  }

  static async open(device) {
    if (!device.opened) await device.open();
    if (device.configuration === null) await device.selectConfiguration(1);
    const found = findUpdateInterface(device.configuration);
    if (!found) throw new Error('this device has no FastPROM update interface');
    await device.claimInterface(found.interfaceNumber);
    return new UpdateLink(device, found);
  }

  // The WebAssembly's buffers are views into its memory, which may move
  // while a transfer is pending: each is copied before the first await.
  async bulkOut(data) {
    const copy = new Uint8Array(data);
    const r = await timeout(this.device.transferOut(this.epOut, copy), 'bulk OUT');
    return r.status;
  }

  async bulkIn(len) {
    const r = await timeout(this.device.transferIn(this.epIn, len), 'bulk IN');
    if (r.status === 'stall') return null;
    if (r.status !== 'ok') throw new Error(`bulk IN: ${r.status}`);
    return new Uint8Array(r.data.buffer.slice(r.data.byteOffset, r.data.byteOffset + r.data.byteLength));
  }

  /** The protocol's reset: a vendor request to the interface, no data. */
  async reset(request) {
    const setup = { requestType: 'vendor', recipient: 'interface', request, value: 0, index: this.iface };
    const r = await timeout(this.device.controlTransferOut(setup), 'reset');
    return r.status;
  }

  /** Let the device go; quietly, since it may already have. */
  async close() {
    try {
      await this.device.releaseInterface(this.iface);
    } catch {
      // gone already
    }
    try {
      await this.device.close();
    } catch {
      // gone already
    }
  }
}

/**
 * Wait for the board with `serial` to enumerate with `bcd` (after a
 * reboot, the application's or the DFU stage's): WebUSB keeps the
 * permission, so getDevices finds it without asking the user again.
 */
export async function waitForDevice(usb, serial, bcd, ms = 20000) {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    const found = (await usb.getDevices()).find((d) => d.serialNumber === serial && bcdOf(d) === bcd);
    if (found) return found;
    await new Promise((r) => setTimeout(r, 200));
  }
  return null;
}
