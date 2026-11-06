//! Every descriptor the device gives, byte for byte, fetched through
//! the device stack over the model controller as a host would fetch it.
//!
//! The expected bytes are written out by hand from the specifications
//! (USB 2.0 chapter 9, USB 3.2 9.6.2 for BOS, CDC 1.2 / PSTN 1.2, the
//! WebUSB specification 3.2, Microsoft's OS 2.0 descriptors
//! specification), not computed by the code under test.

mod common;

use common::{setup, Device, Host, Stalled, SERIAL};
use fastprom_usb::msos;
use fastprom_usb::Variant;

const DEVICE: u8 = 1;
const CONFIGURATION: u8 = 2;
const STRING: u8 = 3;
const BOS: u8 = 15;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn string_descriptor(s: &str) -> Vec<u8> {
    let body = utf16(s);
    let mut d = vec![(body.len() + 2) as u8, 0x03];
    d.extend(body);
    d
}

#[rustfmt::skip]
const DEVICE_APP: [u8; 18] = [
    18, 0x01,       // bLength, DEVICE
    0x10, 0x02,     // bcdUSB 2.10
    0xEF, 0x02, 0x01, // IAD device class
    64,             // bMaxPacketSize0
    0x09, 0x12,     // idVendor 0x1209
    0x01, 0x00,     // idProduct 0x0001
    0x00, 0x01,     // bcdDevice 1.00
    1, 2, 3,        // iManufacturer, iProduct, iSerialNumber
    1,              // bNumConfigurations
];

#[rustfmt::skip]
const DEVICE_DFU: [u8; 18] = [
    18, 0x01, 0x10, 0x02,
    0x00, 0x00, 0x00, // class per interface
    64, 0x09, 0x12, 0x01, 0x00,
    0x01, 0x00,     // bcdDevice 0.01
    1, 2, 3, 1,
];

/// The update interface, 0 in both firmwares.
#[rustfmt::skip]
const UPDATE_INTERFACE: [u8; 23] = [
    9, 0x04, 0, 0, 2, 0xFF, 0x00, 0x00, 0,  // interface 0, 2 endpoints, vendor
    7, 0x05, 0x01, 0x02, 64, 0, 0,          // EP1 OUT bulk 64
    7, 0x05, 0x81, 0x02, 64, 0, 0,          // EP1 IN bulk 64
];

#[rustfmt::skip]
const CDC_FUNCTION: [u8; 66] = [
    8, 0x0B, 1, 2, 0x02, 0x02, 0x00, 0,     // IAD: interfaces 1-2, CDC ACM
    9, 0x04, 1, 0, 1, 0x02, 0x02, 0x00, 0,  // interface 1, 1 endpoint, CDC ACM
    5, 0x24, 0x00, 0x10, 0x01,              // header, bcdCDC 1.10
    5, 0x24, 0x01, 0x00, 2,                 // call management, data interface 2
    4, 0x24, 0x02, 0x02,                    // ACM, line coding + line state
    5, 0x24, 0x06, 1, 2,                    // union: 1 controls 2
    7, 0x05, 0x83, 0x03, 16, 0, 255,        // EP3 IN interrupt 16, 255 ms
    9, 0x04, 2, 0, 2, 0x0A, 0x00, 0x00, 0,  // interface 2, 2 endpoints, CDC data
    7, 0x05, 0x02, 0x02, 64, 0, 0,          // EP2 OUT bulk 64
    7, 0x05, 0x82, 0x02, 64, 0, 0,          // EP2 IN bulk 64
];

fn configuration_app() -> Vec<u8> {
    // 9 + 23 + 66 = 98 bytes, 3 interfaces, configuration 1, bus
    // powered, 100 mA.
    let mut d = vec![9, 0x02, 98, 0, 3, 1, 0, 0x80, 50];
    d.extend(UPDATE_INTERFACE);
    d.extend(CDC_FUNCTION);
    d
}

fn configuration_dfu() -> Vec<u8> {
    let mut d = vec![9, 0x02, 32, 0, 1, 1, 0, 0x80, 50];
    d.extend(UPDATE_INTERFACE);
    d
}

#[rustfmt::skip]
const WEBUSB_UUID: [u8; 16] = [
    // {3408B638-09A9-47A0-8BFD-A0768815B665}
    0x38, 0xB6, 0x08, 0x34, 0xA9, 0x09, 0xA0, 0x47,
    0x8B, 0xFD, 0xA0, 0x76, 0x88, 0x15, 0xB6, 0x65,
];

#[rustfmt::skip]
const MS_OS_20_UUID: [u8; 16] = [
    // {D8DD60DF-4589-4CC7-9CD2-659D9E648A9F}
    0xDF, 0x60, 0xDD, 0xD8, 0x89, 0x45, 0xC7, 0x4C,
    0x9C, 0xD2, 0x65, 0x9D, 0x9E, 0x64, 0x8A, 0x9F,
];

fn bos(set_len: u16) -> Vec<u8> {
    // BOS header, USB 2.0 extension (no LPM), WebUSB
    // (24 bytes), MS OS 2.0 (28 bytes): 5 + 7 + 24 + 28 = 64.
    let mut d = vec![5, 0x0F, 64, 0, 3];
    d.extend([7, 0x10, 0x02, 0, 0, 0, 0]);
    d.extend([24, 0x10, 0x05, 0]);
    d.extend(WEBUSB_UUID);
    d.extend([0x00, 0x01, 0x01, 0]); // bcdVersion 1.00, vendor code 1, no landing page
    d.extend([28, 0x10, 0x05, 0]);
    d.extend(MS_OS_20_UUID);
    d.extend([0x00, 0x00, 0x03, 0x06]); // Windows 8.1
    d.extend(set_len.to_le_bytes());
    d.extend([0x02, 0]); // vendor code 2, no alternate enumeration
    d
}

fn ms_features() -> Vec<u8> {
    // Compatible ID: WINUSB, no sub-compatible ID.
    let mut d = vec![20, 0, 0x03, 0];
    d.extend(b"WINUSB\0\0");
    d.extend([0; 8]);
    // Registry property: REG_MULTI_SZ DeviceInterfaceGUIDs, 132 bytes.
    let name = utf16("DeviceInterfaceGUIDs\0");
    let data = utf16("{B4F828EE-BE3A-4C34-A662-FE495ADE6F92}\0\0");
    assert_eq!((name.len(), data.len()), (42, 80));
    d.extend([132, 0, 0x04, 0, 0x07, 0, 42, 0]);
    d.extend(name);
    d.extend([80, 0]);
    d.extend(data);
    d
}

fn ms_set_app() -> Vec<u8> {
    // 10 + 8 + 8 + 20 + 132 = 178.
    let mut d = vec![10, 0, 0x00, 0, 0x00, 0x00, 0x03, 0x06, 178, 0];
    d.extend([8, 0, 0x01, 0, 0, 0, 168, 0]); // configuration 0, 8 + 160
    d.extend([8, 0, 0x02, 0, 0, 0, 160, 0]); // interface 0 (update), 8 + 20 + 132
    d.extend(ms_features());
    d
}

fn ms_set_dfu() -> Vec<u8> {
    // 10 + 20 + 132 = 162, no subsets.
    let mut d = vec![10, 0, 0x00, 0, 0x00, 0x00, 0x03, 0x06, 162, 0];
    d.extend(ms_features());
    d
}

fn host(variant: Variant) -> (Device, common::Wire) {
    let (dev, wire) = Device::new(variant);
    (dev, wire)
}

#[test]
fn device_descriptor_application() {
    let (mut dev, wire) = host(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    assert_eq!(h.get_descriptor(DEVICE, 0, 18).unwrap(), DEVICE_APP);
    // A host's first request asks for 64 and gets the 18.
    assert_eq!(h.get_descriptor(DEVICE, 0, 64).unwrap(), DEVICE_APP);
}

#[test]
fn device_descriptor_dfu() {
    let (mut dev, wire) = host(Variant::Dfu);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    assert_eq!(h.get_descriptor(DEVICE, 0, 18).unwrap(), DEVICE_DFU);
}

#[test]
fn configuration_descriptor_application() {
    let (mut dev, wire) = host(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    // The host reads the 9-byte header for wTotalLength, then the whole.
    let head = h.get_descriptor(CONFIGURATION, 0, 9).unwrap();
    assert_eq!(head, configuration_app()[..9]);
    let total = u16::from_le_bytes([head[2], head[3]]);
    assert_eq!(total, 98);
    let all = h.get_descriptor(CONFIGURATION, 0, total).unwrap();
    assert_eq!(all, configuration_app());
    assert_eq!(all.len(), 98);
}

#[test]
fn configuration_descriptor_dfu() {
    let (mut dev, wire) = host(Variant::Dfu);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    let all = h.get_descriptor(CONFIGURATION, 0, 255).unwrap();
    assert_eq!(all, configuration_dfu());
}

#[test]
fn bos_application() {
    let (mut dev, wire) = host(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    let head = h.get_descriptor(BOS, 0, 5).unwrap();
    assert_eq!(head, [5, 0x0F, 64, 0, 3]);
    // Exactly 64 bytes, one full packet: asked for more, the device ends
    // the transfer with a zero-length packet.
    assert_eq!(h.get_descriptor(BOS, 0, 255).unwrap(), bos(178));
    assert_eq!(h.get_descriptor(BOS, 0, 64).unwrap(), bos(178));
}

#[test]
fn bos_dfu() {
    let (mut dev, wire) = host(Variant::Dfu);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    assert_eq!(h.get_descriptor(BOS, 0, 255).unwrap(), bos(162));
}

#[test]
fn ms_os_20_set_application() {
    let (mut dev, wire) = host(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    // bmRequestType 0xC0 (device to host, vendor, device), bMS_VendorCode,
    // wIndex 7, wLength from the BOS capability.
    let set = h.control_in(setup(0xC0, 0x02, 0, 7, 178)).unwrap();
    assert_eq!(set, ms_set_app());
    assert_eq!(set.len(), 178);
    assert_eq!(set, msos::SET_COMPOSITE);
}

#[test]
fn ms_os_20_set_dfu() {
    let (mut dev, wire) = host(Variant::Dfu);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    let set = h.control_in(setup(0xC0, 0x02, 0, 7, 162)).unwrap();
    assert_eq!(set, ms_set_dfu());
    assert_eq!(set, msos::SET_SINGLE);
}

#[test]
fn webusb_get_url_is_refused() {
    let (mut dev, wire) = host(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    assert_eq!(h.control_in(setup(0xC0, 0x01, 1, 2, 255)), Err(Stalled));
    // And the device still answers afterwards.
    assert_eq!(h.get_descriptor(DEVICE, 0, 18).unwrap(), DEVICE_APP);
}

#[test]
fn strings() {
    for variant in [Variant::Application, Variant::Dfu] {
        let (mut dev, wire) = host(variant);
        let mut h = Host {
            dev: &mut dev,
            wire,
        };
        h.enumerate();
        assert_eq!(
            h.get_descriptor(STRING, 0, 255).unwrap(),
            [4, 0x03, 0x09, 0x04]
        );
        assert_eq!(
            h.get_descriptor(STRING, 1, 255).unwrap(),
            string_descriptor("FastPROM")
        );
        let product = match variant {
            Variant::Application => "FastPROM",
            Variant::Dfu => "FastPROM (DFU)",
        };
        assert_eq!(
            h.get_descriptor(STRING, 2, 255).unwrap(),
            string_descriptor(product)
        );
        let serial = h.get_descriptor(STRING, 3, 255).unwrap();
        assert_eq!(serial, string_descriptor(SERIAL));
        assert_eq!(serial.len(), 66);
    }
}

#[test]
fn serial_number_from_unique_id() {
    let s = fastprom_usb::Serial::from_uid([0x0123_4567, 0x89AB_CDEF, 0x0011_2233, 0x4455_6677]);
    assert_eq!(s.digits(), SERIAL.as_bytes());
    let z = fastprom_usb::Serial::from_uid([0; 4]);
    assert_eq!(z.digits(), b"00000000000000000000000000000000");
    let f = fastprom_usb::Serial::from_uid([u32::MAX; 4]);
    assert_eq!(f.digits(), b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
}

#[test]
fn capability_constants_agree_with_the_sets() {
    assert_eq!(msos::SET_COMPOSITE_LEN, 178);
    assert_eq!(msos::SET_SINGLE_LEN, 162);
    assert_eq!(msos::WEBUSB_CAPABILITY.len() + 3, 24);
    assert_eq!(
        msos::MS_OS_20_CAPABILITY_COMPOSITE[21..23],
        178u16.to_le_bytes()
    );
    assert_eq!(
        msos::MS_OS_20_CAPABILITY_SINGLE[21..23],
        162u16.to_le_bytes()
    );
    assert_eq!(msos::WEBUSB_CAPABILITY[1..17], WEBUSB_UUID);
}
