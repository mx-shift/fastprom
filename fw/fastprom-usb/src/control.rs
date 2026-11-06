//! A SETUP packet (USB 2.0, 9.3).

/// bmRequestType bits 6:5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Defined by the USB specification.
    Standard,
    /// Defined by a device class.
    Class,
    /// Defined by the vendor.
    Vendor,
    /// Reserved.
    Reserved,
}

/// bmRequestType bits 4:0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipient {
    /// The device.
    Device,
    /// An interface (wIndex low byte).
    Interface,
    /// An endpoint (wIndex low byte).
    Endpoint,
    /// Anything else.
    Other,
}

/// The standard requests (USB 2.0, table 9-4).
pub mod std_req {
    /// GET_STATUS.
    pub const GET_STATUS: u8 = 0;
    /// CLEAR_FEATURE.
    pub const CLEAR_FEATURE: u8 = 1;
    /// SET_FEATURE.
    pub const SET_FEATURE: u8 = 3;
    /// SET_ADDRESS.
    pub const SET_ADDRESS: u8 = 5;
    /// GET_DESCRIPTOR.
    pub const GET_DESCRIPTOR: u8 = 6;
    /// GET_CONFIGURATION.
    pub const GET_CONFIGURATION: u8 = 8;
    /// SET_CONFIGURATION.
    pub const SET_CONFIGURATION: u8 = 9;
    /// GET_INTERFACE.
    pub const GET_INTERFACE: u8 = 10;
    /// SET_INTERFACE.
    pub const SET_INTERFACE: u8 = 11;
    /// The ENDPOINT_HALT feature selector.
    pub const ENDPOINT_HALT: u16 = 0;
}

/// A parsed SETUP packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// bmRequestType.
    pub request_type: u8,
    /// bRequest.
    pub request: u8,
    /// wValue.
    pub value: u16,
    /// wIndex.
    pub index: u16,
    /// wLength.
    pub length: u16,
}

impl Request {
    /// From the eight bytes.
    pub const fn parse(p: [u8; 8]) -> Self {
        Self {
            request_type: p[0],
            request: p[1],
            value: u16::from_le_bytes([p[2], p[3]]),
            index: u16::from_le_bytes([p[4], p[5]]),
            length: u16::from_le_bytes([p[6], p[7]]),
        }
    }

    /// Device to host.
    pub const fn dir_in(&self) -> bool {
        self.request_type & 0x80 != 0
    }

    /// Standard, class or vendor.
    pub const fn kind(&self) -> Kind {
        match (self.request_type >> 5) & 3 {
            0 => Kind::Standard,
            1 => Kind::Class,
            2 => Kind::Vendor,
            _ => Kind::Reserved,
        }
    }

    /// Device, interface, endpoint or other.
    pub const fn recipient(&self) -> Recipient {
        match self.request_type & 0x1F {
            0 => Recipient::Device,
            1 => Recipient::Interface,
            2 => Recipient::Endpoint,
            _ => Recipient::Other,
        }
    }

    /// wIndex's low byte: the interface or endpoint addressed.
    pub const fn target(&self) -> u8 {
        self.index as u8
    }
}

/// What a function answers to a control request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// Done (an OUT request), or this many bytes of the buffer it was
    /// given (an IN request).
    Ok(usize),
    /// A constant to send (an IN request).
    Static(&'static [u8]),
    /// Refused: stall.
    Stall,
}

/// Whether a request is a SET_FEATURE or CLEAR_FEATURE(ENDPOINT_HALT), and
/// which.
pub fn halt_request(req: &Request) -> Option<bool> {
    if req.kind() != Kind::Standard
        || req.recipient() != Recipient::Endpoint
        || req.value != std_req::ENDPOINT_HALT
    {
        return None;
    }
    match req.request {
        std_req::SET_FEATURE => Some(true),
        std_req::CLEAR_FEATURE => Some(false),
        _ => None,
    }
}
