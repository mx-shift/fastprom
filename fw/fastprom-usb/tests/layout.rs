//! The controller's endpoint numbering and its in-memory structures,
//! against AGM's SDK (`usb.h`) and TinyUSB port (`agrv2k.h`).

use core::mem::{align_of, offset_of, size_of};

use fastprom_usb::dqh::{self, Buffer, EndpointMemory, Overlay, Qh, Td};
use fastprom_usb::eps::{self, EpIndex};

#[test]
fn every_half_maps_as_agrv2k_h_does() {
    // (address, dcd_ep_num2idx, dcd_ep_idx2bit, ENDPTCTRL shift)
    let table = [
        (0x00, 0, 0, 0),
        (0x80, 1, 16, 16),
        (0x01, 2, 1, 0),
        (0x81, 3, 17, 16),
        (0x02, 4, 2, 0),
        (0x82, 5, 18, 16),
        (0x03, 6, 3, 0),
        (0x83, 7, 19, 16),
    ];
    for (addr, idx, bit, shift) in table {
        let ep = EpIndex::from_address(addr).unwrap();
        assert_eq!(ep.address(), addr);
        assert_eq!(ep.queue_head(), idx, "{addr:#04x}");
        // agrv2k.h dcd_ep_idx2bit: ep_idx / 2 + (ep_idx % 2 ? 16 : 0).
        assert_eq!(idx / 2 + if idx % 2 == 1 { 16 } else { 0 }, bit);
        assert_eq!(ep.bit(), 1 << bit, "{addr:#04x}");
        assert_eq!(ep.ctrl_shift(), shift);
        assert_eq!(EpIndex::new(addr & 0x0F, addr & 0x80 != 0), Some(ep));
    }
    assert_eq!(EpIndex::EP0_OUT.queue_head(), 0);
    assert_eq!(EpIndex::EP0_IN.queue_head(), 1);
}

#[test]
fn halves_the_controller_lacks_are_refused() {
    for addr in [0x04, 0x84, 0x0F, 0x8F, 0x10, 0x40] {
        assert_eq!(EpIndex::from_address(addr), None, "{addr:#04x}");
    }
    assert_eq!(EpIndex::new(4, false), None);
}

#[test]
fn the_endpoint_plan_fits_four_endpoints() {
    let used = [
        eps::UPDATE_OUT,
        eps::UPDATE_IN,
        eps::CDC_DATA_OUT,
        eps::CDC_DATA_IN,
        eps::CDC_NOTIFY_IN,
    ];
    let mut halves: Vec<usize> = used
        .iter()
        .map(|&a| EpIndex::from_address(a).unwrap().queue_head())
        .collect();
    halves.sort_unstable();
    halves.dedup();
    assert_eq!(halves.len(), used.len(), "two functions share a half");
    assert!(halves.iter().all(|h| (2..eps::QUEUE_HEADS).contains(h)));
    assert_eq!(eps::QUEUE_HEADS, 8);
}

#[test]
fn masks_split_out_and_in() {
    assert_eq!(eps::out_mask(0x0005_000A), 0x000A);
    assert_eq!(eps::in_mask(0x0005_000A), 0x0005);
}

#[test]
fn structures_are_usb_h_s() {
    // USB_DTD: 8 words, 32-byte aligned.
    assert_eq!(size_of::<Td>(), 32);
    assert_eq!(align_of::<Td>(), 32);
    assert_eq!(offset_of!(Td, token), 0x04);
    assert_eq!(offset_of!(Td, buffer), 0x08);
    assert_eq!(offset_of!(Td, expected), 0x1C);
    // USB_DQH: 64 bytes, overlay at 0x08, setup buffer at 0x28.
    assert_eq!(size_of::<Qh>(), 64);
    assert_eq!(align_of::<Qh>(), 64);
    assert_eq!(offset_of!(Qh, current), 0x04);
    assert_eq!(offset_of!(Qh, overlay), 0x08);
    assert_eq!(offset_of!(Qh, overlay) + offset_of!(Overlay, token), 0x0C);
    assert_eq!(size_of::<Overlay>(), 32);
    assert_eq!(offset_of!(Qh, setup), 0x28);
    assert_eq!(offset_of!(Qh, reserved), 0x30);
    // The endpoint list first, 4 KiB aligned; buffers 64-byte aligned, so
    // none crosses a 4 KiB page.
    assert_eq!(align_of::<EndpointMemory>(), 4096);
    assert_eq!(offset_of!(EndpointMemory, qh), 0);
    assert_eq!(offset_of!(EndpointMemory, td) % 32, 0);
    assert_eq!(offset_of!(EndpointMemory, buffer) % 64, 0);
    assert_eq!(size_of::<Buffer>(), 64);
    let end = offset_of!(EndpointMemory, buffer) + 8 * 64;
    assert!(end <= 4096, "a buffer would cross into the next page");
}

#[test]
fn queue_head_info_word() {
    // MaxPacketLength 64 at bits 26:16, ZLT disabled (bit 29), IOS (15).
    assert_eq!(dqh::qh_info(64, true), (64 << 16) | (1 << 29) | (1 << 15));
    assert_eq!(dqh::qh_info(64, false), 0x2040_0000);
    assert_eq!(dqh::qh_info(16, false), 0x2010_0000);
}

#[test]
fn transfer_token() {
    // TotalBytes at 30:16, IOC (15), Active (7).
    assert_eq!(dqh::td_token(64), 0x0040_8080);
    assert_eq!(dqh::td_token(0), 0x0000_8080);
    // 64 asked, 7 left: 57 moved.
    let after = (7 << 16) | (1 << 15);
    assert_eq!(dqh::td_transferred(64, after), 57);
    assert_eq!(dqh::td_transferred(64, 1 << 15), 64);
    assert!(!dqh::td_failed(after));
    for bit in [3, 5, 6] {
        assert!(dqh::td_failed(1 << bit), "bit {bit}");
    }
}

#[test]
fn buffer_page_pointers() {
    assert_eq!(
        dqh::buffer_pages(0x2000_1040),
        [
            0x2000_1040,
            0x2000_2000,
            0x2000_3000,
            0x2000_4000,
            0x2000_5000
        ]
    );
}
