//! The BOS platform capabilities and the Microsoft OS 2.0 descriptor set.
//!
//! - WebUSB: the platform capability of the WebUSB specification (WICG,
//!   "WebUSB API", 1.0, section 3.2), so Chrome knows the device speaks
//!   WebUSB. No landing page (`iLandingPage` 0): Chrome no longer shows one,
//!   and the page's address is not settled.
//! - Microsoft OS 2.0 ("Microsoft OS 2.0 Descriptors Specification", 2013):
//!   the platform capability, and the descriptor set it points to, which
//!   gives the update interface the compatible ID `WINUSB` (Windows binds
//!   WinUSB itself, no driver package) and a `DeviceInterfaceGUIDs`
//!   property (what Chrome and libusb open the interface by).
//!
//! The application is a composite device, so its set puts the compatible
//! ID in a function subset for the update interface, 0; the DFU stage has one
//! interface and is not composite, so its set has no subsets (a function
//! subset is for composite devices only, as the specification says).

use crate::constbuf::Out;
use crate::ids::{Variant, UPDATE_INTERFACE};

/// `bRequest` the host uses to ask for the WebUSB landing page (unused).
pub const WEBUSB_VENDOR_CODE: u8 = 0x01;
/// `bRequest` the host uses to fetch the MS OS 2.0 descriptor set.
pub const MS_VENDOR_CODE: u8 = 0x02;
/// `wIndex` of that request: MS_OS_20_DESCRIPTOR_INDEX.
pub const MS_OS_20_DESCRIPTOR_INDEX: u16 = 7;
/// `wIndex` of WebUSB's GET_URL.
pub const WEBUSB_GET_URL: u16 = 2;

/// The WebUSB platform capability UUID,
/// {3408B638-09A9-47A0-8BFD-A0768815B665}, in the descriptor's byte order
/// (the first three fields little-endian).
pub const WEBUSB_UUID: [u8; 16] = [
    0x38, 0xB6, 0x08, 0x34, 0xA9, 0x09, 0xA0, 0x47, 0x8B, 0xFD, 0xA0, 0x76, 0x88, 0x15, 0xB6, 0x65,
];
/// The MS OS 2.0 platform capability UUID,
/// {D8DD60DF-4589-4CC7-9CD2-659D9E648A9F}, likewise.
pub const MS_OS_20_UUID: [u8; 16] = [
    0xDF, 0x60, 0xDD, 0xD8, 0x89, 0x45, 0xC7, 0x4C, 0x9C, 0xD2, 0x65, 0x9D, 0x9E, 0x64, 0x8A, 0x9F,
];
/// Windows 8.1, the first to read MS OS 2.0 descriptors
/// (NTDDI_WINBLUE).
pub const WINDOWS_VERSION: u32 = 0x0603_0000;
/// The update interface's device interface GUID. Generated for FastPROM
/// (random, version 4); any fixed value would do, as long as it never
/// changes.
pub const DEVICE_INTERFACE_GUID: &str = "{B4F828EE-BE3A-4C34-A662-FE495ADE6F92}";

const PROPERTY_NAME: &str = "DeviceInterfaceGUIDs";
/// REG_MULTI_SZ.
const REG_MULTI_SZ: u16 = 7;

// Descriptor types of the set (MS OS 2.0 spec, table 9).
const SET_HEADER: u16 = 0x00;
const SUBSET_HEADER_CONFIGURATION: u16 = 0x01;
const SUBSET_HEADER_FUNCTION: u16 = 0x02;
const FEATURE_COMPATIBLE_ID: u16 = 0x03;
const FEATURE_REG_PROPERTY: u16 = 0x04;

const HEADER_LEN: usize = 10;
const SUBSET_LEN: usize = 8;
const COMPATIBLE_ID_LEN: usize = 20;
/// UTF-16, NUL-terminated.
const NAME_BYTES: usize = (PROPERTY_NAME.len() + 1) * 2;
/// UTF-16, the one string NUL-terminated and the list NUL-terminated.
const DATA_BYTES: usize = (DEVICE_INTERFACE_GUID.len() + 2) * 2;
const PROPERTY_LEN: usize = 10 + NAME_BYTES + DATA_BYTES;
const FUNCTION_LEN: usize = SUBSET_LEN + COMPATIBLE_ID_LEN + PROPERTY_LEN;

/// The application's set: header, configuration subset, function subset
/// for interface 0, compatible ID, registry property.
pub const SET_COMPOSITE_LEN: usize = HEADER_LEN + SUBSET_LEN + FUNCTION_LEN;
/// The DFU stage's set: header, compatible ID, registry property.
pub const SET_SINGLE_LEN: usize = HEADER_LEN + COMPATIBLE_ID_LEN + PROPERTY_LEN;

/// The application's descriptor set.
pub static SET_COMPOSITE: [u8; SET_COMPOSITE_LEN] = build_set(true);
/// The DFU stage's descriptor set.
pub static SET_SINGLE: [u8; SET_SINGLE_LEN] = build_set(false);

/// The descriptor set for `variant`.
pub fn set(variant: Variant) -> &'static [u8] {
    if variant.composite() {
        &SET_COMPOSITE
    } else {
        &SET_SINGLE
    }
}

/// The WebUSB platform capability's body, after bLength,
/// bDescriptorType and bDevCapabilityType (which `desc.rs` writes
/// before it): bReserved, the UUID, bcdVersion 1.00, bVendorCode,
/// iLandingPage.
pub const WEBUSB_CAPABILITY: [u8; 21] = Out::<21>::new()
    .byte(0)
    .bytes(&WEBUSB_UUID)
    .u16(0x0100)
    .byte(WEBUSB_VENDOR_CODE)
    .byte(0)
    .finish();

/// The application's MS OS 2.0 platform capability body.
pub const MS_OS_20_CAPABILITY_COMPOSITE: [u8; 25] = ms_os_20_capability_for_len(SET_COMPOSITE_LEN);
/// The DFU stage's.
pub const MS_OS_20_CAPABILITY_SINGLE: [u8; 25] = ms_os_20_capability_for_len(SET_SINGLE_LEN);

/// The MS OS 2.0 platform capability's body for `variant`.
pub fn ms_os_20_capability_for(variant: Variant) -> &'static [u8; 25] {
    if variant.composite() {
        &MS_OS_20_CAPABILITY_COMPOSITE
    } else {
        &MS_OS_20_CAPABILITY_SINGLE
    }
}

/// The MS OS 2.0 platform capability's body: bReserved, the UUID, then one
/// descriptor set information block - dwWindowsVersion,
/// wMSOSDescriptorSetTotalLength, bMS_VendorCode, bAltEnumCode (0, no
/// alternate enumeration). Use it only in constants.
pub const fn ms_os_20_capability_for_len(len: usize) -> [u8; 25] {
    Out::<25>::new()
        .byte(0)
        .bytes(&MS_OS_20_UUID)
        .u32(WINDOWS_VERSION)
        .u16(len as u16)
        .byte(MS_VENDOR_CODE)
        .byte(0)
        .finish()
}

const fn build_set<const N: usize>(composite: bool) -> [u8; N] {
    let mut out = Out::<N>::new()
        .u16(HEADER_LEN as u16)
        .u16(SET_HEADER)
        .u32(WINDOWS_VERSION)
        .u16(N as u16);
    if composite {
        out = out
            // Configuration subset: bConfigurationValue is the
            // configuration's index here, 0 for the first and only.
            .u16(SUBSET_LEN as u16)
            .u16(SUBSET_HEADER_CONFIGURATION)
            .byte(0)
            .byte(0)
            .u16((SUBSET_LEN + FUNCTION_LEN) as u16)
            // Function subset: the update interface, the first of the
            // function's interfaces (bFirstInterface).
            .u16(SUBSET_LEN as u16)
            .u16(SUBSET_HEADER_FUNCTION)
            .byte(UPDATE_INTERFACE)
            .byte(0)
            .u16(FUNCTION_LEN as u16);
    }
    out.u16(COMPATIBLE_ID_LEN as u16)
        .u16(FEATURE_COMPATIBLE_ID)
        .bytes(b"WINUSB\0\0")
        .bytes(&[0; 8])
        .u16(PROPERTY_LEN as u16)
        .u16(FEATURE_REG_PROPERTY)
        .u16(REG_MULTI_SZ)
        .u16(NAME_BYTES as u16)
        .utf16(PROPERTY_NAME)
        .u16(0)
        .u16(DATA_BYTES as u16)
        .utf16(DEVICE_INTERFACE_GUID)
        .u16(0)
        .u16(0)
        .finish()
}
