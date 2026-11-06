//! Who the device says it is (`docs/10-updates.md`, "USB device"), and its
//! interface numbers.

/// pid.codes' vendor ID.
pub const VID: u16 = 0x1209;
/// pid.codes' test PID, until one is allocated for FastPROM. The
/// application and the DFU stage share it.
pub const PID: u16 = 0x0001;
/// `bMaxPacketSize0`: the largest at full speed, so a control transfer
/// takes the fewest packets.
pub const MAX_PACKET_0: u8 = 64;
/// Manufacturer string.
pub const MANUFACTURER: &str = "FastPROM";
/// bMaxPower in mA: one unit load (`DESIGN_NOTES.md`, "USB: 100 mA
/// declared, polled").
pub const MAX_POWER_MA: u8 = 100;

/// The update interface: 0 in both firmwares, the DFU stage's only one.
pub const UPDATE_INTERFACE: u8 = 0;
/// The application's CDC communications interface.
pub const CDC_COMM_INTERFACE: u8 = 1;
/// The application's CDC data interface.
pub const CDC_DATA_INTERFACE: u8 = 2;

/// Which firmware is running: they enumerate differently but as the same
/// device (VID, PID and serial number), so a browser given the device once
/// finds it again after the reboot into DFU mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// The application: the update interface 0, CDC-ACM on 1-2.
    Application,
    /// The DFU stage: the update interface 0, only.
    Dfu,
}

impl Variant {
    /// Product string.
    pub const fn product(self) -> &'static str {
        match self {
            Self::Application => "FastPROM",
            Self::Dfu => "FastPROM (DFU)",
        }
    }

    /// bcdDevice. Different for the two, so that a host which caches what
    /// it learned about a device by VID, PID and revision (Windows does,
    /// for its OS descriptors) does not carry the application's
    /// composite-device answers over to the DFU stage's single interface.
    pub const fn bcd_device(self) -> u16 {
        match self {
            Self::Application => 0x0100,
            Self::Dfu => 0x0001,
        }
    }

    /// Whether the configuration has more than one function, and so uses
    /// an interface association descriptor and the IAD device class
    /// (0xEF/0x02/0x01).
    pub const fn composite(self) -> bool {
        matches!(self, Self::Application)
    }

    /// The update interface's number: the same in both.
    pub const fn update_interface(self) -> u8 {
        UPDATE_INTERFACE
    }

    /// How many interfaces the configuration has.
    pub const fn interfaces(self) -> u8 {
        match self {
            Self::Application => 3,
            Self::Dfu => 1,
        }
    }
}

/// The serial number: the 128-bit unique ID (`fastprom_hal::flash::Ag32Flash::unique_id`) as 32
/// upper-case hex digits, word by word in FLASH_GetUniqueID's order, each
/// word most significant digit first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Serial([u8; 32]);

/// The serial number's string descriptor length: 2 + 32 UTF-16 units.
pub const SERIAL_DESCRIPTOR_LEN: usize = 2 + 32 * 2;

impl Serial {
    /// From the unique ID's four words.
    pub const fn from_uid(uid: [u32; 4]) -> Self {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = [b'0'; 32];
        let mut i = 0;
        while i < 32 {
            let word = uid[i / 8];
            let nibble = (word >> (28 - 4 * (i % 8))) & 0xF;
            out[i] = HEX[nibble as usize];
            i += 1;
        }
        Self(out)
    }

    /// The ASCII digits.
    pub const fn digits(&self) -> &[u8; 32] {
        &self.0
    }

    /// Its string descriptor, into `out`; the length written (0 if `out`
    /// is too short).
    pub fn descriptor(&self, out: &mut [u8]) -> usize {
        if out.len() < SERIAL_DESCRIPTOR_LEN {
            return 0;
        }
        // Byte by byte, so that no slice bound can fail.
        for (i, o) in out.iter_mut().take(SERIAL_DESCRIPTOR_LEN).enumerate() {
            *o = match i {
                0 => SERIAL_DESCRIPTOR_LEN as u8,
                1 => 0x03,
                _ if i % 2 == 0 => self.0.get((i - 2) / 2).copied().unwrap_or(0),
                _ => 0,
            };
        }
        SERIAL_DESCRIPTOR_LEN
    }
}
