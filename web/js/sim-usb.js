// A fake WebUSB (navigator.usb's shape) over the simulated board: the
// firmware's update core compiled to WebAssembly (fastprom-host's sim,
// through fastprom-web's Sim). The page talks to it exactly as it talks to
// a real board: its interfaces and endpoints are parsed from the
// configuration descriptor the firmware sends (fastprom-usb's), it vanishes
// on REBOOT and comes back with the other firmware's bcdDevice, and it
// vanishes when the simulator cuts the power.

const REBOOT_MS = 300;

function gone() {
  return new DOMException('The device was disconnected.', 'NetworkError');
}

/** WebUSB's USBConfiguration from a configuration descriptor's bytes. */
export function parseConfiguration(bytes) {
  const interfaces = [];
  let current = null;
  for (let at = 0; at + 1 < bytes.length && bytes[at] >= 2; at += bytes[at]) {
    const d = bytes.subarray(at, at + bytes[at]);
    if (d[1] === 4) {
      current = {
        alternateSetting: d[3],
        interfaceClass: d[5],
        interfaceSubclass: d[6],
        interfaceProtocol: d[7],
        interfaceName: null,
        endpoints: [],
      };
      interfaces.push({ interfaceNumber: d[2], alternate: current, alternates: [current], claimed: false });
    } else if (d[1] === 5 && current) {
      current.endpoints.push({
        endpointNumber: d[2] & 0x0f,
        direction: d[2] & 0x80 ? 'in' : 'out',
        type: ['control', 'isochronous', 'bulk', 'interrupt'][d[3] & 3],
        packetSize: d[4] | (d[5] << 8),
      });
    }
  }
  return { configurationValue: bytes[5], configurationName: null, interfaces };
}

class FakeDevice {
  constructor(usb, enumeration) {
    const sim = usb.sim;
    const dd = sim.deviceDescriptor();
    this.usb = usb;
    this.enumeration = enumeration;
    this.usbVersionMajor = dd[3];
    this.usbVersionMinor = dd[2] >> 4;
    this.usbVersionSubminor = dd[2] & 0xf;
    this.deviceClass = dd[4];
    this.deviceSubclass = dd[5];
    this.deviceProtocol = dd[6];
    this.vendorId = dd[8] | (dd[9] << 8);
    this.productId = dd[10] | (dd[11] << 8);
    this.deviceVersionMajor = dd[13];
    this.deviceVersionMinor = dd[12] >> 4;
    this.deviceVersionSubminor = dd[12] & 0xf;
    this.manufacturerName = 'FastPROM';
    this.productName = sim.bcdDevice === usb.ids.bcdDfu ? 'FastPROM (DFU)' : 'FastPROM';
    this.serialNumber = sim.serial;
    this.configurations = [parseConfiguration(sim.configurationDescriptor())];
    this.configuration = null;
    this.opened = false;
  }

  live() {
    if (!this.usb.isCurrent(this)) throw gone();
  }

  async step(fn) {
    // A transfer takes time: let the page's other work run.
    await new Promise((r) => setTimeout(r, 0));
    this.live();
    try {
      return fn(this.usb.sim);
    } catch (e) {
      if (e.name === 'NetworkError') throw gone();
      throw e;
    } finally {
      this.usb.tick();
    }
  }

  async open() {
    this.live();
    this.opened = true;
  }

  async close() {
    this.opened = false;
  }

  async forget() {
    this.usb.permitted = false;
  }

  async selectConfiguration(value) {
    this.live();
    const c = this.configurations.find((x) => x.configurationValue === value);
    if (!c) throw new DOMException('Configuration not found.', 'NotFoundError');
    this.configuration = c;
  }

  iface(n) {
    const i = this.configuration?.interfaces.find((x) => x.interfaceNumber === n);
    if (!i) throw new DOMException('Interface not found.', 'NotFoundError');
    return i;
  }

  async claimInterface(n) {
    this.live();
    if (!this.opened) throw new DOMException('The device must be opened first.', 'InvalidStateError');
    this.iface(n).claimed = true;
  }

  async releaseInterface(n) {
    this.iface(n).claimed = false;
  }

  /** The claimed interface that owns endpoint `num` in direction `dir`. */
  endpoint(num, dir) {
    const owner = this.configuration?.interfaces.find(
      (i) => i.claimed && i.alternate.endpoints.some((e) => e.endpointNumber === num && e.direction === dir),
    );
    if (!owner) throw new DOMException('The endpoint is not part of a claimed interface.', 'NotFoundError');
  }

  transferOut(num, data) {
    return this.step((sim) => {
      this.endpoint(num, 'out');
      const bytes = new Uint8Array(data.buffer ?? data, data.byteOffset ?? 0, data.byteLength);
      try {
        sim.bulkOut(bytes);
      } catch (e) {
        // The device NAKs while it has an answer unread: a real transfer
        // would wait until the page's timeout.
        if (e.name === 'UsbError') throw new DOMException(`The transfer timed out (${e.message}).`, 'TimeoutError');
        throw e;
      }
      return { status: 'ok', bytesWritten: bytes.length };
    });
  }

  transferIn(num, length) {
    return this.step((sim) => {
      this.endpoint(num, 'in');
      try {
        const b = sim.bulkIn(length);
        return { status: 'ok', data: new DataView(b.buffer, b.byteOffset, b.byteLength) };
      } catch (e) {
        // Nothing to send: a real transfer would wait until the timeout.
        if (e.name === 'UsbError') throw new DOMException(`The transfer timed out (${e.message}).`, 'TimeoutError');
        throw e;
      }
    });
  }

  /**
   * A control transfer to an interface, host to device, no data: the
   * device answers only the update interface's reset (vendor, to the
   * update interface, fastprom-usb's update.rs), and stalls anything else.
   */
  controlTransferOut(setup, data) {
    return this.step((sim) => {
      if (setup.recipient !== 'interface') {
        throw new DOMException('The simulator serves requests to an interface only.', 'NotSupportedError');
      }
      if (!this.iface(setup.index).claimed) {
        throw new DOMException('The interface is not claimed.', 'InvalidStateError');
      }
      const ok = setup.requestType === 'vendor' && setup.request === this.usb.ids.requestReset
        && setup.index === this.usb.ids.updateInterface && (data?.byteLength ?? 0) === 0;
      if (!ok) return { status: 'stall', bytesWritten: 0 };
      sim.reset();
      return { status: 'ok', bytesWritten: 0 };
    });
  }

  async reset() {
    this.live();
  }
}

/** navigator.usb's shape, over one simulated board. */
export class FakeUsb extends EventTarget {
  constructor(wasm, sim) {
    super();
    this.sim = sim;
    this.ids = JSON.parse(wasm.usbIds());
    this.permitted = false;
    this.device = null;
    this.onchange = null;
    this.tick();
  }

  isCurrent(d) {
    return this.sim.connected && this.device === d;
  }

  /** Notice the board leaving or arriving, as a USB stack would. */
  tick() {
    const status = JSON.parse(this.sim.status());
    if (!this.sim.connected) {
      if (this.device) {
        const d = this.device;
        this.device = null;
        this.dispatch('disconnect', d);
      }
      if (this.sim.rebootPending && !this.rebooting) {
        this.rebooting = true;
        setTimeout(() => {
          this.rebooting = false;
          this.sim.completeReboot();
          this.tick();
        }, REBOOT_MS);
      }
    } else if (!this.device || this.device.enumeration !== status.enumerations) {
      this.device = new FakeDevice(this, status.enumerations);
      this.dispatch('connect', this.device);
    }
    this.onchange?.(status);
  }

  dispatch(type, device) {
    const e = new Event(type);
    e.device = device;
    this.dispatchEvent(e);
  }

  async getDevices() {
    this.tick();
    return this.permitted && this.device ? [this.device] : [];
  }

  async requestDevice(options) {
    this.tick();
    const d = this.device;
    const ok = d && (options?.filters ?? []).some(
      (f) => (f.vendorId === undefined || f.vendorId === d.vendorId) && (f.productId === undefined || f.productId === d.productId),
    );
    if (!ok) throw new DOMException('No device selected.', 'NotFoundError');
    this.permitted = true;
    return d;
  }

  // The simulator's own controls, for the page's simulator panel.
  powerOff() {
    this.sim.powerOff();
    this.tick();
  }

  powerOn() {
    this.sim.powerOn();
    this.tick();
  }
}
