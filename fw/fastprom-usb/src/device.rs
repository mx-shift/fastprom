//! The device: endpoint 0's control pipe, the standard requests, and the
//! dispatch of everything else to the update interface and the optional
//! second function.
//!
//! In place of the `usb-device` crate, which cannot be linked without
//! panic paths (`docs/11-usb-driver.md`, "Why not usb-device"). The device
//! has one configuration and fixed functions, so the descriptors are
//! constants (`desc.rs`) and the stack is only what such a device needs
//! (USB 2.0, chapter 9):
//!
//! - **control pipe**: SETUP, an IN data stage of any length in 64-byte
//!   packets ending with a short packet - a zero-length one when the answer
//!   is shorter than wLength and a multiple of 64 - then the host's
//!   zero-length OUT status; or an OUT data stage of up to
//!   [`CONTROL_BUFFER`] bytes, then a zero-length IN status. A refused
//!   request stalls both halves of endpoint 0 until the next SETUP.
//! - **standard requests**: GET_STATUS (device, interface, endpoint),
//!   CLEAR/SET_FEATURE(ENDPOINT_HALT), SET_ADDRESS, GET_DESCRIPTOR (device,
//!   configuration, string, BOS), GET/SET_CONFIGURATION, GET/SET_INTERFACE
//!   (alternate setting 0 only). Everything else stalls, including
//!   DEVICE_QUALIFIER (a full-speed-only device has none) and remote wakeup
//!   (not offered).
//! - **vendor requests to the device**: the MS OS 2.0 descriptor set; the
//!   WebUSB landing page is refused (there is none).
//! - **class and vendor requests to an interface**: to the update
//!   interface or the second function, by interface number.

use crate::bus::{Bus, Controller, Event, Fault};
use crate::control::{halt_request, std_req, Kind, Recipient, Reply, Request};
use crate::desc::{self, Descriptors, CONFIGURATION_VALUE};
use crate::eps::{MAX_PACKET, PLAN_APP, PLAN_DFU};
use crate::ids::{Serial, Variant};
use crate::msos;
use crate::update::{Service, UpdateClass};

/// The largest OUT data stage taken, and the buffer IN answers are built
/// in (the largest is the serial number's string, 66 bytes).
pub const CONTROL_BUFFER: usize = 128;

const EP0_OUT: u8 = 0x00;
const EP0_IN: u8 = 0x80;
const PACKET: usize = MAX_PACKET as usize;

/// A function beside the update interface: CDC-ACM in the application
/// ([`crate::cdc::CdcAcm`]), nothing in the DFU stage ([`NoFunction`]).
/// Which one it is decides the [`Variant`].
pub trait Function<B: Controller> {
    /// Whether there is one: the application's descriptors if so, the DFU
    /// stage's if not.
    const PRESENT: bool;

    /// Whether interface `iface` is this function's.
    fn owns_interface(&self, _iface: u8) -> bool {
        false
    }

    /// The bus was reset, or the configuration changed.
    fn reset(&mut self) {}

    /// Once per turn of the main loop.
    fn run(&mut self, _bus: &mut Bus<B>) {}

    /// OUT halves with a packet waiting (bit n for endpoint n).
    fn endpoint_out(&mut self, _bus: &mut Bus<B>, _out: u16) {}

    /// IN halves whose packet the host took.
    fn endpoint_in_complete(&mut self, _bus: &mut Bus<B>, _done: u16) {}

    /// A class or vendor request to one of its interfaces: an OUT request
    /// with its data, or an IN request answered into `buf`.
    fn control(
        &mut self,
        _bus: &mut Bus<B>,
        _req: &Request,
        _data: &[u8],
        _buf: &mut [u8],
    ) -> Reply {
        Reply::Stall
    }
}

/// No second function: the DFU stage.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFunction;

impl<B: Controller> Function<B> for NoFunction {
    const PRESENT: bool = false;
}

/// Where an IN answer's bytes are.
#[derive(Debug, Clone, Copy)]
enum Source {
    Buffer,
    Static(&'static [u8]),
}

/// Endpoint 0's state.
#[derive(Debug, Clone, Copy)]
enum Ep0 {
    Idle,
    /// Sending `len` bytes; `sent` so far; a zero-length packet to follow.
    DataIn {
        src: Source,
        len: usize,
        sent: usize,
        zlp: bool,
    },
    /// Data sent; waiting for the host's zero-length OUT.
    StatusOut,
    /// Taking `len` bytes into the buffer; `got` so far.
    DataOut {
        req: Request,
        len: usize,
        got: usize,
    },
    /// Zero-length IN status armed.
    StatusIn,
}

/// The USB device: bus, control pipe, the update interface over `S`, and
/// `F`.
pub struct Device<B: Controller, S: Service, F: Function<B>> {
    bus: Bus<B>,
    variant: Variant,
    desc: &'static Descriptors,
    serial: Serial,
    update: UpdateClass<S>,
    func: F,
    ep0: Ep0,
    buf: [u8; CONTROL_BUFFER],
    configured: bool,
}

impl<B: Controller, S: Service, F: Function<B>> Device<B, S, F> {
    /// The device on `ctl`, serving the update protocol `service` and
    /// `func`, with this serial number. Starts the controller and connects
    /// to the bus.
    pub fn new(ctl: B, serial: Serial, service: S, func: F) -> Self {
        let variant = if F::PRESENT {
            Variant::Application
        } else {
            Variant::Dfu
        };
        let plan: &[_] = if F::PRESENT { &PLAN_APP } else { &PLAN_DFU };
        let mut bus = Bus::new(ctl, plan);
        bus.enable();
        Self {
            bus,
            variant,
            desc: desc::of(variant),
            serial,
            update: UpdateClass::new(variant.update_interface(), service),
            func,
            ep0: Ep0::Idle,
            buf: [0; CONTROL_BUFFER],
            configured: false,
        }
    }

    /// Which descriptors it has.
    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// Whether the host has configured it.
    pub fn configured(&self) -> bool {
        self.configured
    }

    /// The first thing the bus could not do, if anything.
    pub fn fault(&self) -> Option<Fault> {
        self.bus.fault()
    }

    /// The update interface.
    pub fn update(&mut self) -> &mut UpdateClass<S> {
        &mut self.update
    }

    /// The second function.
    pub fn function(&mut self) -> &mut F {
        &mut self.func
    }

    /// One turn: what the controller saw, then the functions.
    pub fn poll(&mut self) {
        match self.bus.poll() {
            Event::None => {}
            Event::Reset => {
                self.ep0 = Ep0::Idle;
                self.configured = false;
                self.update.bus_reset();
                self.func.reset();
            }
            Event::Data {
                setup,
                out,
                in_complete,
            } => {
                if setup {
                    if let Some(packet) = self.bus.read_setup() {
                        self.setup(Request::parse(packet));
                    }
                } else {
                    if out & 1 != 0 {
                        self.ep0_out();
                    }
                    // An IN completion beside a new SETUP belongs to the
                    // transfer the SETUP ended.
                    if in_complete & 1 != 0 {
                        self.ep0_in_complete();
                    }
                }
                let out = out & !1;
                let done = in_complete & !1;
                if out != 0 {
                    self.update.endpoint_out(&mut self.bus, out);
                    self.func.endpoint_out(&mut self.bus, out);
                }
                if done != 0 {
                    self.update.endpoint_in_complete(&mut self.bus, done);
                    self.func.endpoint_in_complete(&mut self.bus, done);
                }
            }
        }
        // Only once configured: before that the host reads nothing, and a
        // packet armed now would be flushed by the bus reset of
        // enumeration, its bytes gone with it.
        if self.configured {
            self.update.run(&mut self.bus);
            self.func.run(&mut self.bus);
        }
    }

    fn stall(&mut self) {
        self.bus.set_stalled(EP0_OUT, true);
        self.bus.set_stalled(EP0_IN, true);
        self.ep0 = Ep0::Idle;
    }

    fn setup(&mut self, req: Request) {
        self.ep0 = Ep0::Idle;
        let wlength = usize::from(req.length);
        if req.dir_in() {
            match self.answer_in(&req) {
                Reply::Ok(n) => self.start_in(Source::Buffer, n, wlength),
                Reply::Static(s) => self.start_in(Source::Static(s), s.len(), wlength),
                Reply::Stall => self.stall(),
            }
        } else if wlength == 0 {
            let reply = self.answer_out(&req, 0);
            self.finish_out(reply);
        } else if wlength > CONTROL_BUFFER {
            self.stall();
        } else {
            self.ep0 = Ep0::DataOut {
                req,
                len: wlength,
                got: 0,
            };
        }
    }

    fn start_in(&mut self, src: Source, total: usize, wlength: usize) {
        let len = total.min(wlength);
        // The host stops at wLength or at a short packet; an answer that is
        // shorter than it asked for and fills its last packet needs a
        // zero-length one after it. An empty answer is itself that packet.
        let zlp = len < wlength && len.is_multiple_of(PACKET) && len != 0;
        self.ep0 = Ep0::DataIn {
            src,
            len,
            sent: 0,
            zlp,
        };
        self.send_chunk();
    }

    /// The next packet of an IN data stage.
    fn send_chunk(&mut self) {
        let Ep0::DataIn {
            src,
            len,
            sent,
            zlp,
        } = self.ep0
        else {
            return;
        };
        let end = len.min(sent.saturating_add(PACKET));
        let chunk = match src {
            Source::Buffer => self.buf.get(sent..end),
            Source::Static(s) => s.get(sent..end),
        };
        let Some(chunk) = chunk else {
            self.stall();
            return;
        };
        match self.bus.write(EP0_IN, chunk) {
            Ok(n) => {
                self.ep0 = Ep0::DataIn {
                    src,
                    len,
                    sent: sent + n,
                    zlp,
                }
            }
            Err(_) => self.stall(),
        }
    }

    fn ep0_in_complete(&mut self) {
        match self.ep0 {
            Ep0::DataIn {
                src,
                len,
                sent,
                zlp,
            } => {
                if sent < len {
                    self.send_chunk();
                } else if zlp {
                    self.ep0 = Ep0::DataIn {
                        src,
                        len,
                        sent,
                        zlp: false,
                    };
                    if self.bus.write(EP0_IN, &[]).is_err() {
                        self.stall();
                    }
                } else {
                    self.ep0 = Ep0::StatusOut;
                }
            }
            Ep0::StatusIn => self.ep0 = Ep0::Idle,
            _ => {}
        }
    }

    fn ep0_out(&mut self) {
        match self.ep0 {
            Ep0::DataOut { req, len, got } => {
                let n = match self.buf.get_mut(got..len) {
                    Some(dst) => self.bus.read(EP0_OUT, dst),
                    None => {
                        self.stall();
                        return;
                    }
                };
                let Ok(n) = n else {
                    self.stall();
                    return;
                };
                let got = got + n;
                if got >= len || n < PACKET {
                    let reply = self.answer_out(&req, got);
                    self.finish_out(reply);
                } else {
                    self.ep0 = Ep0::DataOut { req, len, got };
                }
            }
            state => {
                // The host's status stage after an IN transfer (it may end
                // the data stage early this way), or a stray packet.
                let _ = self.bus.read(EP0_OUT, &mut []);
                if matches!(state, Ep0::DataIn { .. } | Ep0::StatusOut) {
                    self.ep0 = Ep0::Idle;
                }
            }
        }
    }

    fn finish_out(&mut self, reply: Reply) {
        match reply {
            Reply::Ok(_) => match self.bus.write(EP0_IN, &[]) {
                Ok(_) => self.ep0 = Ep0::StatusIn,
                Err(_) => self.stall(),
            },
            Reply::Static(_) | Reply::Stall => self.stall(),
        }
    }

    /// Class and vendor requests to an interface, to whichever owns it.
    fn route_interface(&mut self, req: &Request, data_len: usize) -> Reply {
        let iface = req.target();
        if iface == self.update.interface() {
            return self.update.control(&mut self.bus, req);
        }
        if F::PRESENT && self.func.owns_interface(iface) {
            let (data, out) = if req.dir_in() {
                (&[][..], &mut self.buf[..])
            } else {
                (self.buf.get(..data_len).unwrap_or(&[]), &mut [][..])
            };
            return self.func.control(&mut self.bus, req, data, out);
        }
        Reply::Stall
    }

    fn put(&mut self, bytes: &[u8]) -> Reply {
        match self.buf.get_mut(..bytes.len()) {
            Some(dst) => {
                dst.copy_from_slice(bytes);
                Reply::Ok(bytes.len())
            }
            None => Reply::Stall,
        }
    }

    fn interface_exists(&self, iface: u8) -> bool {
        iface < self.variant.interfaces()
    }

    fn answer_in(&mut self, req: &Request) -> Reply {
        match (req.kind(), req.recipient()) {
            (Kind::Standard, Recipient::Device) => match req.request {
                // Bus powered, no remote wakeup.
                std_req::GET_STATUS => self.put(&[0, 0]),
                std_req::GET_DESCRIPTOR => self.descriptor(req.value),
                std_req::GET_CONFIGURATION => {
                    let value = if self.configured {
                        CONFIGURATION_VALUE
                    } else {
                        0
                    };
                    self.put(&[value])
                }
                _ => Reply::Stall,
            },
            (Kind::Standard, Recipient::Interface) if self.interface_exists(req.target()) => {
                match req.request {
                    std_req::GET_STATUS => self.put(&[0, 0]),
                    std_req::GET_INTERFACE if self.configured => self.put(&[0]),
                    _ => Reply::Stall,
                }
            }
            (Kind::Standard, Recipient::Endpoint)
                if req.request == std_req::GET_STATUS && self.bus.has(req.target()) =>
            {
                let halted = u8::from(self.bus.is_stalled(req.target()));
                self.put(&[halted, 0])
            }
            (Kind::Vendor, Recipient::Device)
                if req.request == msos::MS_VENDOR_CODE
                    && req.index == msos::MS_OS_20_DESCRIPTOR_INDEX =>
            {
                Reply::Static(self.desc.ms_os_20_set)
            }
            (Kind::Class | Kind::Vendor, Recipient::Interface) => self.route_interface(req, 0),
            _ => Reply::Stall,
        }
    }

    fn descriptor(&mut self, value: u16) -> Reply {
        let [index, kind] = value.to_le_bytes();
        match (kind, index) {
            (desc::DEVICE, 0) => Reply::Static(self.desc.device),
            (desc::CONFIGURATION, 0) => Reply::Static(self.desc.configuration),
            (desc::BOS, 0) => Reply::Static(self.desc.bos),
            (desc::STRING, 0) => Reply::Static(&desc::LANGUAGES),
            (desc::STRING, 1) => Reply::Static(&desc::MANUFACTURER_STRING),
            (desc::STRING, 2) => Reply::Static(self.desc.product),
            (desc::STRING, 3) => match self.serial.descriptor(&mut self.buf) {
                0 => Reply::Stall,
                n => Reply::Ok(n),
            },
            _ => Reply::Stall,
        }
    }

    /// An OUT request, its data (if any) the first `data_len` bytes of the
    /// buffer.
    fn answer_out(&mut self, req: &Request, data_len: usize) -> Reply {
        if let Some(halt) = halt_request(req) {
            let ep = req.target();
            if ep & 0x7F == 0 {
                return Reply::Ok(0);
            }
            if self.bus.has(ep) {
                self.bus.set_stalled(ep, halt);
                return Reply::Ok(0);
            }
            return Reply::Stall;
        }
        match (req.kind(), req.recipient()) {
            (Kind::Standard, Recipient::Device) => match req.request {
                std_req::SET_ADDRESS if req.value <= 127 => {
                    self.bus.set_address(req.value as u8);
                    Reply::Ok(0)
                }
                std_req::SET_CONFIGURATION if req.value <= u16::from(CONFIGURATION_VALUE) => {
                    let configured = req.value == u16::from(CONFIGURATION_VALUE);
                    if configured != self.configured {
                        self.func.reset();
                    }
                    self.configured = configured;
                    Reply::Ok(0)
                }
                _ => Reply::Stall,
            },
            (Kind::Standard, Recipient::Interface)
                if req.request == std_req::SET_INTERFACE
                    && req.value == 0
                    && self.interface_exists(req.target()) =>
            {
                Reply::Ok(0)
            }
            (Kind::Class | Kind::Vendor, Recipient::Interface) => {
                self.route_interface(req, data_len)
            }
            _ => Reply::Stall,
        }
    }
}
