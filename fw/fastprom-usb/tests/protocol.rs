//! The update interface and the CDC port end to end: host transactions on
//! the model controller, through the bus, the device stack, the classes and
//! the update protocol's server.

mod common;

use common::{setup, Device, Host, Reply, Stalled};
use fastprom_core::memmap::{APP_BASE, FLASH_BASE, TABLE_BASE};
use fastprom_core::update::proto::{self, Command, HEADER_LEN, IN_TRANSFER, REQUEST_RESET};
use fastprom_core::update::{BootTarget, Status};
use fastprom_usb::bus::Fault;
use fastprom_usb::cdc::DEFAULT_LINE_CODING;
use fastprom_usb::eps::EpIndex;
use fastprom_usb::Variant;

/// A request frame.
fn frame(cmd: Command, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; HEADER_LEN + payload.len()];
    proto::encode(&mut f, cmd as u8, 0, seq, payload).unwrap();
    f
}

fn read_req(seq: u16, addr: u32, len: u32) -> Vec<u8> {
    frame(
        Command::Read,
        seq,
        &[addr.to_le_bytes(), len.to_le_bytes()].concat(),
    )
}

/// The response on the bulk IN endpoint, decoded: status, sequence number,
/// payload.
fn response(h: &mut Host<'_>) -> (Status, u16, Vec<u8>) {
    let r = h.bulk_in(1, IN_TRANSFER).unwrap();
    let (hd, p) = proto::decode(&r).expect("a response frame");
    (Status::from_u8(hd.status).unwrap(), hd.seq, p.to_vec())
}

/// RESET: vendor, host to device, to the update interface (0).
const RESET: [u8; 8] = [0x41, REQUEST_RESET, 0, 0, 0, 0, 0, 0];

fn enumerated(variant: Variant) -> (Device, common::Wire) {
    let (mut dev, wire) = Device::new(variant);
    Host {
        dev: &mut dev,
        wire: wire.clone(),
    }
    .enumerate();
    (dev, wire)
}

#[test]
fn enumeration_sets_the_address_before_the_status_stage() {
    let (mut dev, wire) = Device::new(Variant::Application);
    assert!(wire.hw().initialised && wire.hw().attached);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    h.enumerate();
    assert_eq!(wire.hw().address_log, [9]);
    assert!(h.dev.configured());
    assert_eq!(h.control_in(setup(0x80, 0x08, 0, 0, 1)).unwrap(), [1]);
    assert_eq!(h.dev.alloc_fault(), None);
}

#[test]
fn a_bus_reset_opens_and_arms_every_endpoint() {
    let (dev, wire) = enumerated(Variant::Application);
    let hw = wire.hw();
    assert_eq!(hw.closed, [1, 2, 3]);
    for addr in [0x00, 0x80, 0x01, 0x81, 0x02, 0x82, 0x83] {
        let ep = EpIndex::from_address(addr).unwrap();
        assert!(hw.halves[ep.queue_head()].open.is_some(), "{addr:#04x}");
    }
    // Endpoint 3 OUT is the one half nobody uses.
    assert!(hw.halves[EpIndex::from_address(0x03).unwrap().queue_head()]
        .open
        .is_none());
    // The bulk OUT endpoints are armed for a packet.
    for addr in [0x01, 0x02] {
        let ep = EpIndex::from_address(addr).unwrap();
        assert_eq!(hw.halves[ep.queue_head()].primed, Some(64), "{addr:#04x}");
    }
    drop(hw);
    drop(dev);
}

#[test]
fn a_read_comes_back_in_packets_ending_short() {
    for variant in [Variant::Application, Variant::Dfu] {
        let (mut dev, wire) = enumerated(variant);
        let mut h = Host {
            dev: &mut dev,
            wire: wire.clone(),
        };
        assert_eq!(
            h.bulk_out(1, &read_req(7, APP_BASE + 16, 150)),
            Reply::Data(vec![])
        );
        let (s, seq, data) = response(&mut h);
        assert_eq!((s, seq), (Status::Ok, 7));
        assert_eq!(data, h.dev.board().bytes(APP_BASE + 16, 150));
        // A whole sector: 4112 bytes, 65 packets.
        h.bulk_out(1, &read_req(8, TABLE_BASE, 4096));
        let (s, seq, data) = response(&mut h);
        assert_eq!((s, seq), (Status::Ok, 8));
        assert_eq!(data, h.dev.board().bytes(TABLE_BASE, 4096));
        h.dev.steps(5);
        assert_eq!(wire.r#in(1), Reply::Nak, "nothing more");
    }
}

#[test]
fn a_response_that_fills_its_last_packet_is_closed_by_a_zero_length_one() {
    let (mut dev, wire) = enumerated(Variant::Dfu);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    // 16 + 48 = 64 bytes.
    h.bulk_out(1, &read_req(1, FLASH_BASE, 48));
    let mut packets = Vec::new();
    loop {
        h.dev.step();
        match wire.r#in(1) {
            Reply::Data(p) => {
                let short = p.len() < 64;
                packets.push(p.len());
                if short {
                    break;
                }
            }
            Reply::Nak => continue,
            Reply::Stall => panic!("stalled"),
        }
    }
    assert_eq!(packets, [64, 0]);
    h.dev.steps(5);
    assert_eq!(wire.r#in(1), Reply::Nak, "one zero-length packet, not more");
}

#[test]
fn a_refusal_is_a_response_not_a_halt() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    // READ outside the flash; ERASE of the application in the application.
    h.bulk_out(1, &read_req(5, 0x1000_0000, 4));
    assert_eq!(response(&mut h), (Status::InvalidAddress, 5, vec![]));
    h.bulk_out(1, &frame(Command::Erase, 6, &APP_BASE.to_le_bytes()));
    assert_eq!(response(&mut h), (Status::NotPermitted, 6, vec![]));
    // Neither endpoint is halted.
    assert_eq!(h.control_in(setup(0x82, 0x00, 0, 0x81, 2)).unwrap(), [0, 0]);
    assert_eq!(h.control_in(setup(0x82, 0x00, 0, 0x01, 2)).unwrap(), [0, 0]);
    // REBOOT: acknowledged, then the reboot.
    h.bulk_out(1, &frame(Command::Reboot, 7, &[1]));
    assert_eq!(response(&mut h), (Status::Ok, 7, vec![]));
    h.dev.steps(2);
    assert_eq!(*h.dev.board().reboots.borrow(), [BootTarget::Dfu]);
}

#[test]
fn a_request_waits_in_the_controller_while_a_response_is_unread() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    h.bulk_out(1, &read_req(1, TABLE_BASE, 4096));
    h.dev.steps(3);
    // The next request's packet is taken into the controller (the OUT half
    // was armed), but the class does not read it, so the half is not armed
    // again: the one after is NAKed.
    assert_eq!(
        wire.out(1, &frame(Command::Info, 2, &[])),
        Reply::Data(vec![])
    );
    h.dev.steps(3);
    assert_eq!(wire.out(1, &frame(Command::Info, 3, &[])), Reply::Nak);
    let (s, seq, data) = response(&mut h);
    assert_eq!((s, seq, data.len()), (Status::Ok, 1, 4096));
    // Now it is read, and answered.
    let (s, seq, _) = response(&mut h);
    assert_eq!((s, seq), (Status::Ok, 2));
}

#[test]
fn reset_empties_both_endpoints_and_the_server() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    // A response armed on IN, unread, and a request waiting on OUT.
    h.bulk_out(1, &read_req(1, TABLE_BASE, 4096));
    h.dev.steps(3);
    let in_half = EpIndex::from_address(0x81).unwrap().queue_head();
    assert!(wire.hw().halves[in_half].primed.is_some());
    assert_eq!(
        wire.out(1, &frame(Command::Info, 2, &[])),
        Reply::Data(vec![])
    );
    h.dev.steps(2);
    h.control_out(RESET, &[]).unwrap();
    h.dev.steps(3);
    assert!(wire.hw().halves[in_half].primed.is_none(), "flushed");
    assert_eq!(wire.r#in(1), Reply::Nak, "nothing of the old response");
    // A fresh exchange: half a request abandoned, a reset, a whole one.
    let write = frame(
        Command::Write,
        3,
        &[&TABLE_BASE.to_le_bytes()[..], &[0u8; 256]].concat(),
    );
    h.bulk_out(1, &write[..128]);
    h.control_out(RESET, &[]).unwrap();
    h.bulk_out(1, &frame(Command::Info, 4, &[]));
    let (s, seq, info) = response(&mut h);
    assert_eq!((s, seq, info.len()), (Status::Ok, 4, 144));
    // The reset only to the update interface, and only host to device with
    // no data.
    assert_eq!(
        h.control_out(setup(0x41, REQUEST_RESET, 0, 1, 0), &[]),
        Err(Stalled)
    );
    assert_eq!(
        h.control_in(setup(0xC1, REQUEST_RESET, 0, 0, 4)),
        Err(Stalled)
    );
    assert_eq!(h.control_out(setup(0x41, 0x02, 0, 0, 0), &[]), Err(Stalled));
    assert_eq!(
        h.control_out(setup(0x21, REQUEST_RESET, 0, 0, 0), &[]),
        Err(Stalled)
    );
}

#[test]
fn clear_feature_unhalts_an_endpoint_and_resets_its_toggle() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    let in_half = EpIndex::from_address(0x81).unwrap().queue_head();
    // SET_FEATURE(ENDPOINT_HALT) on 0x81 halts it, and GET_STATUS says so.
    h.control_out(setup(0x02, 0x03, 0, 0x81, 0), &[]).unwrap();
    assert!(wire.hw().halves[in_half].stalled);
    assert_eq!(h.control_in(setup(0x82, 0x00, 0, 0x81, 2)).unwrap(), [1, 0]);
    let resets = wire.hw().halves[in_half].toggle_resets;
    // CLEAR_FEATURE(ENDPOINT_HALT) clears it and resets the toggle.
    h.control_out(setup(0x02, 0x01, 0, 0x81, 0), &[]).unwrap();
    assert!(!wire.hw().halves[in_half].stalled);
    assert_eq!(wire.hw().halves[in_half].toggle_resets, resets + 1);
    // And the interface works.
    h.bulk_out(1, &frame(Command::Info, 9, &[]));
    assert_eq!(response(&mut h).0, Status::Ok);
}

#[test]
fn standard_requests_a_fixed_device_answers_and_refuses() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    // GET_STATUS: device (bus powered, no wakeup), interface, endpoint.
    assert_eq!(h.control_in(setup(0x80, 0x00, 0, 0, 2)).unwrap(), [0, 0]);
    assert_eq!(h.control_in(setup(0x81, 0x00, 0, 2, 2)).unwrap(), [0, 0]);
    assert_eq!(h.control_in(setup(0x81, 0x00, 0, 3, 2)), Err(Stalled));
    assert_eq!(h.control_in(setup(0x82, 0x00, 0, 0x82, 2)).unwrap(), [0, 0]);
    // Endpoint 3 OUT is not one of the device's.
    assert_eq!(h.control_in(setup(0x82, 0x00, 0, 0x03, 2)), Err(Stalled));
    // GET_INTERFACE: alternate setting 0; SET_INTERFACE to 0 only.
    assert_eq!(h.control_in(setup(0x81, 0x0A, 0, 1, 1)).unwrap(), [0]);
    h.control_out(setup(0x01, 0x0B, 0, 1, 0), &[]).unwrap();
    assert_eq!(h.control_out(setup(0x01, 0x0B, 1, 1, 0), &[]), Err(Stalled));
    // A full-speed-only device has no device qualifier.
    assert_eq!(h.control_in(setup(0x80, 0x06, 0x0600, 0, 10)), Err(Stalled));
    // No remote wakeup to enable; no second configuration.
    assert_eq!(h.control_out(setup(0x00, 0x03, 1, 0, 0), &[]), Err(Stalled));
    assert_eq!(h.control_out(setup(0x00, 0x09, 2, 0, 0), &[]), Err(Stalled));
    // A string the device does not have.
    assert_eq!(
        h.control_in(setup(0x80, 0x06, 0x0304, 0x0409, 255)),
        Err(Stalled)
    );
    // Deconfigured, then configured again.
    h.control_out(setup(0x00, 0x09, 0, 0, 0), &[]).unwrap();
    assert_eq!(h.control_in(setup(0x80, 0x08, 0, 0, 1)).unwrap(), [0]);
    h.control_out(setup(0x00, 0x09, 1, 0, 0), &[]).unwrap();
    assert!(h.dev.configured());
    // After every refusal, endpoint 0 still works.
    assert_eq!(
        h.control_in(setup(0x80, 0x06, 0x0100, 0, 18))
            .unwrap()
            .len(),
        18
    );
}

#[test]
fn a_short_read_of_a_long_descriptor_and_a_zero_length_answer() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    // wLength 70 of the 98-byte configuration: 64 + 6, nothing more.
    let part = h.control_in(setup(0x80, 0x06, 0x0200, 0, 70)).unwrap();
    assert_eq!(part.len(), 70);
    // wLength 0 on an IN request: an empty data stage.
    assert_eq!(
        h.control_in(setup(0x80, 0x06, 0x0100, 0, 0)).unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn a_bus_reset_ends_the_exchange_in_progress() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    h.bulk_out(1, &read_req(9, TABLE_BASE, 4096));
    h.dev.steps(2);
    h.enumerate();
    h.bulk_out(1, &read_req(10, FLASH_BASE, 4));
    let (s, seq, data) = response(&mut h);
    assert_eq!((s, seq), (Status::Ok, 10));
    assert_eq!(data, h.dev.board().bytes(FLASH_BASE, 4));
}

#[test]
fn cdc_line_coding_and_line_state() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    assert_eq!(
        h.control_in(setup(0xA1, 0x21, 0, 1, 7)).unwrap(),
        DEFAULT_LINE_CODING
    );
    let coding = [0x80, 0x25, 0, 0, 0, 0, 8]; // 9600 8N1
    h.control_out(setup(0x21, 0x20, 0, 1, 7), &coding).unwrap();
    assert_eq!(h.control_in(setup(0xA1, 0x21, 0, 1, 7)).unwrap(), coding);
    assert!(!h.dev.cdc().dtr());
    h.control_out(setup(0x21, 0x22, 0x3, 1, 0), &[]).unwrap();
    assert!(h.dev.cdc().dtr());
    assert!(h.dev.cdc().rts());
    // Not to the data interface.
    assert_eq!(h.control_in(setup(0xA1, 0x21, 0, 2, 7)), Err(Stalled));
}

#[test]
fn cdc_writes_go_a_packet_at_a_time_and_a_full_last_packet_is_closed() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire: wire.clone(),
    };
    let text: Vec<u8> = (0..70u8).collect();
    assert_eq!(h.dev.cdc().write(&text), 70);
    assert_eq!(h.bulk_in(2, 4096).unwrap(), text, "64 then 6");
    let full = [0x55u8; 64];
    h.dev.cdc().write(&full);
    // The 64 bytes, then a zero-length packet: a read of 4096 ends only on
    // a short packet, so returning at all means the zero-length one came.
    assert_eq!(h.bulk_in(2, 4096).unwrap(), full[..64]);
    h.dev.steps(2);
    assert_eq!(wire.r#in(2), Reply::Nak, "one zero-length packet, not more");
}

#[test]
fn cdc_output_queued_before_enumeration_survives_it() {
    let (mut dev, wire) = Device::new(Variant::Application);
    // The boot report, written before the host has reset or configured
    // the device.
    assert_eq!(dev.cdc().write(b"FastPROM boot report\n"), 21);
    dev.steps(3);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.enumerate();
    assert_eq!(h.bulk_in(2, 4096).unwrap(), b"FastPROM boot report\n");
}

#[test]
fn cdc_reads_what_the_host_sends() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    assert_eq!(h.bulk_out(2, b"hello"), Reply::Data(vec![]));
    h.dev.steps(2);
    let mut buf = [0u8; 16];
    let n = h.dev.cdc().read(&mut buf);
    assert_eq!(&buf[..n], b"hello");
}

#[test]
fn the_dfu_stage_has_no_cdc_endpoints() {
    let (_dev, wire) = enumerated(Variant::Dfu);
    let hw = wire.hw();
    for addr in [0x02, 0x82, 0x83] {
        let ep = EpIndex::from_address(addr).unwrap();
        assert!(hw.halves[ep.queue_head()].open.is_none(), "{addr:#04x}");
    }
}

#[test]
fn no_faults_in_normal_use() {
    let (mut dev, wire) = enumerated(Variant::Application);
    let mut h = Host {
        dev: &mut dev,
        wire,
    };
    h.bulk_out(1, &read_req(1, FLASH_BASE, 64));
    assert_eq!(response(&mut h).0, Status::Ok);
    h.control_out(RESET, &[]).unwrap();
    assert_eq!(h.dev.alloc_fault(), None::<Fault>);
}
