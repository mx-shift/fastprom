//! The standard descriptors, built at compile time for each variant.
//!
//! The device is fixed - one configuration, known functions - so every
//! descriptor but the serial number's is a constant (USB 2.0 chapter 9; the
//! BOS of USB 3.2 9.6.2; CDC 1.2 and its PSTN subclass for the ACM
//! function). The bytes are checked by hand in
//! `tests/descriptors.rs`.

use crate::constbuf::Out;
use crate::eps::{
    CDC_DATA_IN, CDC_DATA_OUT, CDC_NOTIFY_IN, CDC_NOTIFY_INTERVAL, CDC_NOTIFY_PACKET, MAX_PACKET,
    UPDATE_IN, UPDATE_OUT,
};
use crate::ids::{
    Variant, CDC_COMM_INTERFACE, CDC_DATA_INTERFACE, MANUFACTURER, MAX_PACKET_0, MAX_POWER_MA, PID,
    UPDATE_INTERFACE, VID,
};
use crate::msos::{
    ms_os_20_capability_for_len, SET_COMPOSITE_LEN, SET_SINGLE_LEN, WEBUSB_CAPABILITY,
};

// Descriptor types.
/// DEVICE.
pub const DEVICE: u8 = 1;
/// CONFIGURATION.
pub const CONFIGURATION: u8 = 2;
/// STRING.
pub const STRING: u8 = 3;
const INTERFACE: u8 = 4;
const ENDPOINT: u8 = 5;
const IAD: u8 = 11;
/// BOS.
pub const BOS: u8 = 15;
const DEVICE_CAPABILITY: u8 = 16;
const CS_INTERFACE: u8 = 0x24;

/// The vendor class, subclass and protocol, for the update interface.
pub const VENDOR_CLASS: [u8; 3] = [0xFF, 0x00, 0x00];

/// bConfigurationValue of the one configuration.
pub const CONFIGURATION_VALUE: u8 = 1;

const fn device(variant: Variant) -> [u8; 18] {
    let class: [u8; 3] = if variant.composite() {
        [0xEF, 0x02, 0x01]
    } else {
        [0, 0, 0]
    };
    Out::<18>::new()
        .byte(18)
        .byte(DEVICE)
        .u16(0x0210)
        .bytes(&class)
        .byte(MAX_PACKET_0)
        .u16(VID)
        .u16(PID)
        .u16(variant.bcd_device())
        .bytes(&[1, 2, 3, 1])
        .finish()
}

/// The application's device descriptor.
pub const DEVICE_APP: [u8; 18] = device(Variant::Application);
/// The DFU stage's.
pub const DEVICE_DFU: [u8; 18] = device(Variant::Dfu);

const fn interface<const N: usize>(o: Out<N>, num: u8, eps: u8, class: [u8; 3]) -> Out<N> {
    o.byte(9)
        .byte(INTERFACE)
        .byte(num)
        .byte(0)
        .byte(eps)
        .bytes(&class)
        .byte(0)
}

const fn endpoint<const N: usize>(o: Out<N>, addr: u8, attr: u8, mps: u16, interval: u8) -> Out<N> {
    o.byte(7)
        .byte(ENDPOINT)
        .byte(addr)
        .byte(attr)
        .u16(mps)
        .byte(interval)
}

const fn config_header<const N: usize>(variant: Variant) -> Out<N> {
    Out::<N>::new()
        .byte(9)
        .byte(CONFIGURATION)
        .u16(N as u16)
        .byte(variant.interfaces())
        .byte(CONFIGURATION_VALUE)
        .byte(0)
        .byte(0x80) // bus powered, no remote wakeup
        .byte(MAX_POWER_MA / 2)
}

const fn update<const N: usize>(o: Out<N>) -> Out<N> {
    let o = interface(o, UPDATE_INTERFACE, 2, VENDOR_CLASS);
    let o = endpoint(o, UPDATE_OUT, 0x02, MAX_PACKET, 0);
    endpoint(o, UPDATE_IN, 0x02, MAX_PACKET, 0)
}

/// The application's configuration: 9 + update 23 + CDC 66.
pub const CONFIG_APP_LEN: usize = 98;
/// The DFU stage's: 9 + update 23.
pub const CONFIG_DFU_LEN: usize = 32;

const fn config_app() -> [u8; CONFIG_APP_LEN] {
    let o = config_header::<CONFIG_APP_LEN>(Variant::Application);
    let o = update(o);
    // CDC-ACM: IAD, communications interface with its functional
    // descriptors and notification endpoint, data interface.
    let o = o
        .byte(8)
        .byte(IAD)
        .byte(CDC_COMM_INTERFACE)
        .byte(2)
        .bytes(&[0x02, 0x02, 0x00, 0]);
    let o = interface(o, CDC_COMM_INTERFACE, 1, [0x02, 0x02, 0x00]);
    let o = o
        // Header, bcdCDC 1.10.
        .bytes(&[5, CS_INTERFACE, 0x00, 0x10, 0x01])
        // Call management: none, data interface.
        .bytes(&[5, CS_INTERFACE, 0x01, 0x00, CDC_DATA_INTERFACE])
        // ACM: Set/Get_Line_Coding, Set_Control_Line_State, Serial_State.
        .bytes(&[4, CS_INTERFACE, 0x02, 0x02])
        // Union: the communications interface controls the data one.
        .bytes(&[
            5,
            CS_INTERFACE,
            0x06,
            CDC_COMM_INTERFACE,
            CDC_DATA_INTERFACE,
        ]);
    let o = endpoint(
        o,
        CDC_NOTIFY_IN,
        0x03,
        CDC_NOTIFY_PACKET,
        CDC_NOTIFY_INTERVAL,
    );
    let o = interface(o, CDC_DATA_INTERFACE, 2, [0x0A, 0x00, 0x00]);
    let o = endpoint(o, CDC_DATA_OUT, 0x02, MAX_PACKET, 0);
    endpoint(o, CDC_DATA_IN, 0x02, MAX_PACKET, 0).finish()
}

/// The application's configuration descriptor, all of it.
pub const CONFIG_APP: [u8; CONFIG_APP_LEN] = config_app();
/// The DFU stage's.
pub const CONFIG_DFU: [u8; CONFIG_DFU_LEN] =
    update(config_header::<CONFIG_DFU_LEN>(Variant::Dfu)).finish();

/// BOS: header, USB 2.0 extension (no LPM), WebUSB, MS OS 2.0.
pub const BOS_LEN: usize = 5 + 7 + 24 + 28;

const fn bos(set_len: usize) -> [u8; BOS_LEN] {
    Out::<BOS_LEN>::new()
        .byte(5)
        .byte(BOS)
        .u16(BOS_LEN as u16)
        .byte(3)
        .bytes(&[7, DEVICE_CAPABILITY, 0x02, 0, 0, 0, 0])
        .byte(24)
        .byte(DEVICE_CAPABILITY)
        .byte(0x05)
        .bytes(&WEBUSB_CAPABILITY)
        .byte(28)
        .byte(DEVICE_CAPABILITY)
        .byte(0x05)
        .bytes(&ms_os_20_capability_for_len(set_len))
        .finish()
}

/// The application's BOS.
pub const BOS_APP: [u8; BOS_LEN] = bos(SET_COMPOSITE_LEN);
/// The DFU stage's.
pub const BOS_DFU: [u8; BOS_LEN] = bos(SET_SINGLE_LEN);

const fn string<const N: usize>(s: &str) -> [u8; N] {
    Out::<N>::new().byte(N as u8).byte(STRING).utf16(s).finish()
}

/// String 0: the languages, US English only.
pub const LANGUAGES: [u8; 4] = [4, STRING, 0x09, 0x04];
/// String 1.
pub const MANUFACTURER_STRING: [u8; 2 + 2 * MANUFACTURER.len()] = string(MANUFACTURER);
const PRODUCT_APP: &str = Variant::Application.product();
const PRODUCT_DFU: &str = Variant::Dfu.product();
/// String 2, the application's.
pub const PRODUCT_STRING_APP: [u8; 2 + 2 * PRODUCT_APP.len()] = string(PRODUCT_APP);
/// String 2, the DFU stage's.
pub const PRODUCT_STRING_DFU: [u8; 2 + 2 * PRODUCT_DFU.len()] = string(PRODUCT_DFU);

/// The constant descriptors of one variant.
#[derive(Debug, Clone, Copy)]
pub struct Descriptors {
    /// Device.
    pub device: &'static [u8],
    /// Configuration, whole.
    pub configuration: &'static [u8],
    /// BOS, whole.
    pub bos: &'static [u8],
    /// String 2.
    pub product: &'static [u8],
    /// The MS OS 2.0 descriptor set.
    pub ms_os_20_set: &'static [u8],
}

static APP: Descriptors = Descriptors {
    device: &DEVICE_APP,
    configuration: &CONFIG_APP,
    bos: &BOS_APP,
    product: &PRODUCT_STRING_APP,
    ms_os_20_set: &crate::msos::SET_COMPOSITE,
};

static DFU: Descriptors = Descriptors {
    device: &DEVICE_DFU,
    configuration: &CONFIG_DFU,
    bos: &BOS_DFU,
    product: &PRODUCT_STRING_DFU,
    ms_os_20_set: &crate::msos::SET_SINGLE,
};

/// `variant`'s descriptors.
pub fn of(variant: Variant) -> &'static Descriptors {
    match variant {
        Variant::Application => &APP,
        Variant::Dfu => &DFU,
    }
}
