//! The device controller's in-memory structures: queue heads (dQH) and
//! transfer descriptors (dTD), as AGM's SDK lays them out (`usb.h`
//! USB_DQH, USB_DTD), and the words the driver writes into them.
//!
//! The bit positions are those of `usb.h`'s bitfields, which GCC allocates
//! from bit 0 upward on the little-endian RV32 target. Pure functions of
//! plain integers, so they are host-tested; `fastprom_hal::usb` does the volatile
//! accesses.

use crate::eps::{MAX_PACKET, QUEUE_HEADS};

/// A transfer descriptor: `usb.h` USB_DTD, 32 bytes, 32-byte aligned (the
/// next-dTD pointer keeps only bits 31:5; AGM's TinyUSB port declares its
/// dTDs `TU_ATTR_ALIGNED(32)`, `agrv2k.h`).
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, Default)]
pub struct Td {
    /// 0x00: next dTD pointer, or [`TERMINATE`].
    pub next: u32,
    /// 0x04: status and length ([`token`]).
    pub token: u32,
    /// 0x08: buffer page pointers ([`buffer_pages`]).
    pub buffer: [u32; 5],
    /// 0x1C: not read by the controller; the SDK keeps the transfer's
    /// length here (`ExpectedBytes`), and so does this driver.
    pub expected: u32,
}

/// A queue head: `usb.h` USB_DQH, 64 bytes, 64-byte aligned
/// (`agrv2k.h` `USB_DQH dQH[QHD_MAX] TU_ATTR_ALIGNED(64)`).
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, Default)]
pub struct Qh {
    /// 0x00: endpoint capabilities ([`qh_info`]).
    pub info: u32,
    /// 0x04: current dTD pointer, the controller's.
    pub current: u32,
    /// 0x08: the transfer overlay, a dTD's eight words that the controller
    /// works in.
    pub overlay: Overlay,
    /// 0x28: the last SETUP packet (control OUT queue heads only).
    pub setup: [u32; 2],
    /// 0x30..0x3F: reserved, and the SDK's software head/tail pointers.
    pub reserved: [u32; 4],
}

/// The dQH's transfer overlay (`usb.h` USB_DQH `Overlay`): a dTD's layout,
/// but at offset 0x08 of the queue head, so not 32-byte aligned - which is
/// why it is not a [`Td`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Overlay {
    /// 0x08: next dTD pointer.
    pub next: u32,
    /// 0x0C: status and length.
    pub token: u32,
    /// 0x10: buffer page pointers.
    pub buffer: [u32; 5],
    /// 0x24: reserved.
    pub reserved: u32,
}

impl Overlay {
    /// All zero.
    pub const ZERO: Self = Self {
        next: 0,
        token: 0,
        buffer: [0; 5],
        reserved: 0,
    };
}

impl Td {
    /// All zero.
    pub const ZERO: Self = Self {
        next: 0,
        token: 0,
        buffer: [0; 5],
        expected: 0,
    };
}

impl Qh {
    /// All zero.
    pub const ZERO: Self = Self {
        info: 0,
        current: 0,
        overlay: Overlay::ZERO,
        setup: [0; 2],
        reserved: [0; 4],
    };
}

/// One endpoint half's packet buffer.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Buffer(pub [u8; MAX_PACKET as usize]);

impl Buffer {
    /// All zero.
    pub const ZERO: Self = Self([0; MAX_PACKET as usize]);
}

impl Default for Buffer {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Everything the controller reads and writes by DMA: the endpoint list
/// (one queue head per endpoint half, in [`crate::eps::EpIndex::queue_head`]
/// order), one dTD per queue head, one packet buffer per queue head.
///
/// 4 KiB aligned, as AGM's TinyUSB port aligns its `usb_data`
/// (`agrv2k.c`, `TU_ATTR_ALIGNED(4096)`) and imxrt-usbd its queue-head list
/// (`state.rs`, `align(4096)`). ENDPOINTLISTADDR takes the list's address
/// (`usb.h` USB_SetEndPointListAddr); its alignment requirement is not
/// stated in the SDK, so the larger of the two known ones is kept. A
/// 64-byte buffer at a 64-byte boundary inside it never crosses a 4 KiB
/// page, so a dTD needs only its first page pointer.
#[repr(C, align(4096))]
#[derive(Debug, Clone, Copy, Default)]
pub struct EndpointMemory {
    /// The endpoint list.
    pub qh: [Qh; QUEUE_HEADS],
    /// One dTD per queue head.
    pub td: [Td; QUEUE_HEADS],
    /// One packet buffer per queue head.
    pub buffer: [Buffer; QUEUE_HEADS],
}

impl EndpointMemory {
    /// All zero, as it starts.
    pub const ZERO: Self = Self {
        qh: [Qh::ZERO; QUEUE_HEADS],
        td: [Td::ZERO; QUEUE_HEADS],
        buffer: [Buffer::ZERO; QUEUE_HEADS],
    };
}

/// A dTD next pointer with nothing after it (`usb.h` USB_TERMINATE).
pub const TERMINATE: u32 = 1;

// dQH info word (`usb.h` USB_DQH bitfields).
/// Interrupt on setup: bit 15 (`IntOnSetup`).
pub const QH_IOS: u32 = 1 << 15;
/// Maximum packet length: bits 26:16 (`MaxPacketLength : 11`).
pub const QH_MAX_PACKET_SHIFT: u32 = 16;
/// Zero-length termination disable: bit 29
/// (`ZeroLengthTerminationDisable`).
pub const QH_ZLT_DISABLE: u32 = 1 << 29;

// dTD token word (`usb.h` USB_DTD bitfields).
/// Transaction error: bit 3 (`TranError`).
pub const TD_TRANSACTION_ERROR: u32 = 1 << 3;
/// Data buffer error: bit 5 (`DataError`).
pub const TD_DATA_ERROR: u32 = 1 << 5;
/// Halted: bit 6 (`Halted`).
pub const TD_HALTED: u32 = 1 << 6;
/// Active: bit 7 (`Active`).
pub const TD_ACTIVE: u32 = 1 << 7;
/// Interrupt on complete: bit 15 (`IntOnComplete`).
pub const TD_IOC: u32 = 1 << 15;
/// Total bytes: bits 30:16 (`TotalBytes : 15`).
pub const TD_TOTAL_SHIFT: u32 = 16;
/// The total-bytes field's width.
pub const TD_TOTAL_MASK: u32 = 0x7FFF;

/// A queue head's info word, as `usb.h` USB_InitDQH sets it: the packet
/// size, zero-length termination disabled (the stack above sends its own
/// zero-length packets), and interrupt-on-setup for endpoint 0 OUT, as
/// AGM's port sets it there and nowhere else (`agrv2k_dcd.c`, "For OUT
/// only"). Mult is 0, as the SDK leaves it: it is for high-bandwidth
/// isochronous endpoints only.
pub const fn qh_info(max_packet: u16, setup: bool) -> u32 {
    let mps = (max_packet as u32 & 0x7FF) << QH_MAX_PACKET_SHIFT;
    mps | QH_ZLT_DISABLE | if setup { QH_IOS } else { 0 }
}

/// A dTD token for a transfer of `len` bytes: active, interrupt on
/// complete, no errors (`usb.h` USB_InitDTD and AGM's dcd_edpt_xfer,
/// `IntOnComplete = true`). The completion bit in ENDPTCOMPLETE is set only
/// for a dTD with IOC, and polling reads that bit.
pub const fn td_token(len: u16) -> u32 {
    ((len as u32 & TD_TOTAL_MASK) << TD_TOTAL_SHIFT) | TD_IOC | TD_ACTIVE
}

/// Bytes a finished dTD moved: what it was set up for, less what the
/// controller left in its total-bytes field.
pub const fn td_transferred(expected: u32, token: u32) -> usize {
    let left = (token >> TD_TOTAL_SHIFT) & TD_TOTAL_MASK;
    expected.saturating_sub(left) as usize
}

/// Whether a dTD's token reports an error or a halt.
pub const fn td_failed(token: u32) -> bool {
    token & (TD_TRANSACTION_ERROR | TD_DATA_ERROR | TD_HALTED) != 0
}

/// A dTD's five buffer page pointers for a buffer at `addr`: the address
/// itself (its low 12 bits are the offset into the first page), then the
/// following 4 KiB pages (`usb.h` USB_InitDTD).
pub const fn buffer_pages(addr: u32) -> [u32; 5] {
    let page = addr >> 12;
    [
        addr,
        page.wrapping_add(1) << 12,
        page.wrapping_add(2) << 12,
        page.wrapping_add(3) << 12,
        page.wrapping_add(4) << 12,
    ]
}
