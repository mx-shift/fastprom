//! The endpoint plan, and how the controller numbers endpoints.
//!
//! The AG32's controller has four endpoints, each with an OUT and an IN
//! half (`usb.h` USB_ENDPOINTS 4, `ENDPTCTRL[USB_ENDPOINTS]`), every packet
//! at most 64 bytes at full speed (`USB_MAX_PACKET_LEN`, `USB_FS` 1). The
//! device needs:
//!
//! | endpoint | OUT | IN |
//! |---|---|---|
//! | 0 | control | control |
//! | 1 | update bulk | update bulk |
//! | 2 | CDC data bulk | CDC data bulk |
//! | 3 | (unused) | CDC notification, interrupt |
//!
//! so the application's update and CDC-ACM functions fit, with one OUT
//! half to spare. The DFU stage uses endpoints 0 and 1 only.

/// Endpoints the controller has, each with an OUT and an IN half.
pub const ENDPOINTS: u8 = 4;
/// Queue heads: one per endpoint half (AGM's TinyUSB port, `agrv2k.h`
/// QHD_MAX = USB_ENDPOINTS * 2).
pub const QUEUE_HEADS: usize = ENDPOINTS as usize * 2;
/// The largest packet at full speed, and every endpoint's buffer size.
pub const MAX_PACKET: u16 = 64;

/// The update interface's bulk OUT endpoint address.
pub const UPDATE_OUT: u8 = 0x01;
/// The update interface's bulk IN endpoint address.
pub const UPDATE_IN: u8 = 0x81;
/// CDC-ACM data, bulk OUT.
pub const CDC_DATA_OUT: u8 = 0x02;
/// CDC-ACM data, bulk IN.
pub const CDC_DATA_IN: u8 = 0x82;
/// CDC-ACM notification, interrupt IN.
pub const CDC_NOTIFY_IN: u8 = 0x83;
/// The notification endpoint's packet size: a SERIAL_STATE notification is
/// 10 bytes (CDC PSTN 1.2, 6.5.4).
pub const CDC_NOTIFY_PACKET: u16 = 16;
/// Its polling interval in frames (ms): the slowest, as nothing urgent is
/// ever sent on it.
pub const CDC_NOTIFY_INTERVAL: u8 = 255;

/// An endpoint's transfer type. The discriminant is bmAttributes' type
/// field, which is also ENDPTCTRL's (AGM's dcd_edpt_open:
/// `bmAttributes.xfer << 2`). No isochronous: none is needed, and AGM's
/// port refuses them ("TODO: support ISO").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpType {
    /// Control (endpoint 0 only).
    Control = 0,
    /// Bulk.
    Bulk = 2,
    /// Interrupt.
    Interrupt = 3,
}

/// One endpoint half of the device: address, type, packet size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpConfig {
    /// USB endpoint address (bit 7 set for IN).
    pub addr: u8,
    /// Transfer type.
    pub kind: EpType,
    /// wMaxPacketSize.
    pub max_packet: u16,
}

const fn ep(addr: u8, kind: EpType, max_packet: u16) -> EpConfig {
    EpConfig {
        addr,
        kind,
        max_packet,
    }
}

/// The DFU stage's endpoints: control, update.
pub const PLAN_DFU: [EpConfig; 4] = [
    ep(0x00, EpType::Control, MAX_PACKET),
    ep(0x80, EpType::Control, MAX_PACKET),
    ep(UPDATE_OUT, EpType::Bulk, MAX_PACKET),
    ep(UPDATE_IN, EpType::Bulk, MAX_PACKET),
];

/// The application's: control, update, CDC-ACM.
pub const PLAN_APP: [EpConfig; 7] = [
    ep(0x00, EpType::Control, MAX_PACKET),
    ep(0x80, EpType::Control, MAX_PACKET),
    ep(UPDATE_OUT, EpType::Bulk, MAX_PACKET),
    ep(UPDATE_IN, EpType::Bulk, MAX_PACKET),
    ep(CDC_DATA_OUT, EpType::Bulk, MAX_PACKET),
    ep(CDC_DATA_IN, EpType::Bulk, MAX_PACKET),
    ep(CDC_NOTIFY_IN, EpType::Interrupt, CDC_NOTIFY_PACKET),
];

/// One half of one endpoint, as the controller indexes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpIndex {
    num: u8,
    is_in: bool,
}

impl EpIndex {
    /// Endpoint 0 OUT: where SETUP packets land.
    pub const EP0_OUT: Self = Self {
        num: 0,
        is_in: false,
    };
    /// Endpoint 0 IN.
    pub const EP0_IN: Self = Self {
        num: 0,
        is_in: true,
    };

    /// The half named by a USB endpoint address (bit 7 set for IN), if the
    /// controller has it.
    pub const fn from_address(addr: u8) -> Option<Self> {
        let num = addr & 0x0F;
        if num >= ENDPOINTS || addr & 0x70 != 0 {
            return None;
        }
        Some(Self {
            num,
            is_in: addr & 0x80 != 0,
        })
    }

    /// Endpoint `num`'s OUT (`is_in` false) or IN half, if the controller
    /// has it.
    pub const fn new(num: u8, is_in: bool) -> Option<Self> {
        if num >= ENDPOINTS {
            return None;
        }
        Some(Self { num, is_in })
    }

    /// The endpoint number, 0..ENDPOINTS.
    pub const fn num(self) -> u8 {
        self.num
    }

    /// Whether this is the IN (device to host) half.
    pub const fn is_in(self) -> bool {
        self.is_in
    }

    /// The USB endpoint address.
    pub const fn address(self) -> u8 {
        self.num | if self.is_in { 0x80 } else { 0 }
    }

    /// The queue head's index in the endpoint list: OUT and IN of each
    /// endpoint in turn (`agrv2k.h` dcd_ep_num2idx: 2 * epnum + dir).
    pub const fn queue_head(self) -> usize {
        2 * self.num as usize + self.is_in as usize
    }

    /// This half's bit in ENDPTSETUPSTAT, ENDPTPRIME, ENDPTFLUSH,
    /// ENDPTSTATUS and ENDPTCOMPLETE: receive (OUT) halves in bits 0..15,
    /// transmit (IN) halves in bits 16..31 (`agrv2k.h` dcd_ep_idx2bit:
    /// ep_idx / 2 + (ep_idx % 2 ? 16 : 0)).
    pub const fn bit(self) -> u32 {
        1 << (self.num as u32 + if self.is_in { 16 } else { 0 })
    }

    /// The shift of this half's fields in its ENDPTCTRL register: receive
    /// at 0, transmit at 16 (`usb.h` USB_ENDPT_OFFSET).
    pub const fn ctrl_shift(self) -> u32 {
        if self.is_in {
            16
        } else {
            0
        }
    }
}

/// The OUT halves' bits of an ENDPTCOMPLETE-shaped word, as the bus's
/// per-endpoint mask (bit n for endpoint n).
pub const fn out_mask(bits: u32) -> u16 {
    (bits & 0xFFFF) as u16
}

/// The IN halves' bits likewise.
pub const fn in_mask(bits: u32) -> u16 {
    (bits >> 16) as u16
}
