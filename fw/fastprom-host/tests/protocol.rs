//! The client against the simulated board: refusals, the recovery rule
//! (reset and send again) after packets lost or damaged on the wire and
//! after a stale answer, a device that speaks another protocol version,
//! and finding the update interface in both firmwares' descriptors.

use std::collections::VecDeque;

use fastprom_core::header::{BoardRev, Part, Version};
use fastprom_core::memmap::{APP_BASE, OPTION_BASE, TABLE_BASE};
use fastprom_core::update::proto::{self, Command, HEADER_LEN, VERSION};
use fastprom_core::update::Status;
use fastprom_host::client::{Client, Error, ATTEMPTS};
use fastprom_host::rom::Geometry;
use fastprom_host::sim::{block_on, SimConfig, SimDevice, SimTransport, UsbFault};
use fastprom_host::transport::{Transport, TransportError};

fn device() -> SimDevice {
    SimDevice::factory(
        SimConfig {
            board_rev: BoardRev::new("A").unwrap(),
            dfu_version: Version::default(),
            part: Part::new("512x8").unwrap(),
            geometry: Geometry {
                depth: 512,
                width: 8,
            },
        },
        Version::default(),
    )
}

/// Refusals of each kind: each is the device's status, and leaves the
/// interface ready for the next request.
#[test]
fn refusals_are_reported_and_the_next_request_works() {
    let mut d = device();
    let mut c = Client::new(SimTransport(&mut d));
    block_on(c.reset()).unwrap();
    let e = block_on(c.write(OPTION_BASE, &[0; 256], &mut |_| ())).unwrap_err();
    assert_eq!(e.status(), Some(Status::NotPermitted), "{e}");
    let e = block_on(c.read(0x2000_0000, 64, &mut |_| ())).unwrap_err();
    assert_eq!(e.status(), Some(Status::InvalidAddress), "{e}");
    let e = block_on(c.erase(TABLE_BASE + 256, 4096)).unwrap_err();
    assert_eq!(e.status(), Some(Status::BadAlignment), "{e}");
    assert!(e.to_string().contains("ERASE"), "{e}");
    let e = block_on(c.erase(APP_BASE, 4096)).unwrap_err();
    assert_eq!(e.status(), Some(Status::NotPermitted), "{e}");
    let e = block_on(c.commit(0)).unwrap_err();
    assert_eq!(
        e.status(),
        Some(Status::NotPermitted),
        "the application has no COMMIT"
    );
    // Not whole pages: refused here, before anything is sent.
    let e = block_on(c.write(TABLE_BASE, &[0; 100], &mut |_| ())).unwrap_err();
    assert!(matches!(e, Error::Check(_)), "{e}");
    // A malformed request reaches the device and is refused there.
    let e = block_on(c.run(Command::Reboot, &[9])).unwrap_err();
    assert_eq!(e.status(), Some(Status::BadRequest), "{e}");
    // After all of that, requests work, and nothing needed a resend.
    let info = block_on(c.info()).unwrap();
    assert_eq!(info.board_rev, "A");
    let back = block_on(c.read(TABLE_BASE, 4, &mut |_| ())).unwrap();
    assert_eq!(back, 0x4650_524Du32.to_le_bytes());
    assert_eq!(c.resyncs(), 0);
    assert!(c.transport().0.idle());
}

fn table_update(c: &mut Client<SimTransport<'_>>) -> Vec<u8> {
    let page: Vec<u8> = (0..=255).collect();
    let data: Vec<u8> = [page.clone(), page.clone()].concat();
    block_on(c.erase(TABLE_BASE, 4096)).unwrap();
    block_on(c.write(TABLE_BASE, &data, &mut |_| ())).unwrap();
    let back = block_on(c.read(TABLE_BASE, 512, &mut |_| ())).unwrap();
    assert_eq!(back, data);
    data
}

/// A packet lost or damaged in either direction, at every packet of a small
/// table update: the client resets and sends the request again, and the
/// update ends as an undisturbed one does.
#[test]
fn lost_and_damaged_packets_are_recovered_from() {
    // How many packets each way an undisturbed run takes: ERASE 1 out 1
    // in; each WRITE 5 out (276 bytes) 1 in; the READ 1 out, 9 in (528
    // bytes).
    let (outs, ins) = (1 + 2 * 5 + 1, 1 + 2 + 9);
    assert_eq!((outs, ins), (12, 12));
    let faults: Vec<UsbFault> = (1..=outs)
        .flat_map(|n| {
            [
                UsbFault::DropOut { after: n },
                UsbFault::CorruptOut { after: n },
            ]
        })
        .chain((1..=ins).flat_map(|n| {
            [
                UsbFault::DropIn { after: n },
                UsbFault::CorruptIn { after: n },
            ]
        }))
        .collect();
    let mut recovered = 0;
    let mut timeouts = Vec::new();
    for fault in faults {
        let mut d = device();
        d.inject_usb_fault(fault);
        let mut c = Client::new(SimTransport(&mut d));
        let page: Vec<u8> = (0..=255).collect();
        let data: Vec<u8> = [page.clone(), page.clone()].concat();
        let run = (|| {
            block_on(c.erase(TABLE_BASE, 4096))?;
            block_on(c.write(TABLE_BASE, &data, &mut |_| ()))?;
            block_on(c.read(TABLE_BASE, 512, &mut |_| ()))
        })();
        match run {
            Ok(back) => {
                assert_eq!(back, data, "{fault:?}");
                assert_eq!(c.resyncs(), 1, "{fault:?}: one resend");
                recovered += 1;
            }
            // A transfer whose last (short) packet is lost never ends: a
            // request's leaves the device waiting for the rest, an
            // answer's the host. That is a timeout, which ends the
            // operation (a page's pending transfer cannot be taken back);
            // the next session starts with a reset.
            Err(Error::Transport(TransportError::Other(m))) => {
                assert_eq!(m, "timeout", "{fault:?}");
                timeouts.push(fault);
                block_on(c.reset()).unwrap();
                assert_eq!(table_update(&mut c), data, "{fault:?}: after a reset");
            }
            Err(e) => panic!("{fault:?}: {e}"),
        }
    }
    // Every fault but the loss of a transfer's last packet with nothing
    // before it to show: ERASE's request, each WRITE's fifth packet, READ's
    // request; the one-packet answers to ERASE and the two WRITEs. (The
    // READ's answer losing its last packet still arrives short of its
    // length, and is sent again.)
    assert_eq!(
        timeouts,
        [
            UsbFault::DropOut { after: 1 },
            UsbFault::DropOut { after: 6 },
            UsbFault::DropOut { after: 11 },
            UsbFault::DropOut { after: 12 },
            UsbFault::DropIn { after: 1 },
            UsbFault::DropIn { after: 2 },
            UsbFault::DropIn { after: 3 },
        ]
    );
    assert_eq!(recovered, 2 * 2 * 12 - 7);
}

/// A transport that hands back one stale answer (the last one seen) before
/// the real one: an answer to an earlier request still in the pipe.
struct Stale<'a> {
    inner: SimTransport<'a>,
    last: Vec<u8>,
    replay: bool,
    queue: VecDeque<Vec<u8>>,
}

impl Transport for Stale<'_> {
    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.inner.bulk_out(data).await
    }
    async fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, TransportError> {
        if let Some(r) = self.queue.pop_front() {
            return Ok(r);
        }
        let r = self.inner.bulk_in(len).await?;
        if std::mem::take(&mut self.replay) {
            self.queue.push_back(r);
            return Ok(self.last.clone());
        }
        self.last = r.clone();
        Ok(r)
    }
    async fn reset(&mut self) -> Result<(), TransportError> {
        self.queue.clear();
        self.inner.reset().await
    }
}

#[test]
fn a_stale_answer_is_not_taken_for_the_current_one() {
    let mut d = device();
    let mut c = Client::new(Stale {
        inner: SimTransport(&mut d),
        last: Vec::new(),
        replay: false,
        queue: VecDeque::new(),
    });
    let info = block_on(c.info()).unwrap();
    c.transport().replay = true;
    // The INFO answer again, while a READ is asked: another sequence
    // number and command. Reset, the READ again, its own answer.
    let back = block_on(c.read(TABLE_BASE, 4, &mut |_| ())).unwrap();
    assert_eq!(back, 0x4650_524Du32.to_le_bytes());
    assert_eq!(c.resyncs(), 1);
    assert_eq!(block_on(c.info()).unwrap(), info);
}

/// A transport that answers every request with a frame of its own making.
struct Fake {
    answer: fn(seq: u16, cmd: u8) -> Vec<u8>,
    seq: u16,
    cmd: u8,
    resets: u32,
}

impl Transport for Fake {
    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), TransportError> {
        if let Ok((h, _)) = proto::decode(data) {
            self.seq = h.seq;
            self.cmd = h.command;
        }
        Ok(())
    }
    async fn bulk_in(&mut self, _len: usize) -> Result<Vec<u8>, TransportError> {
        Ok((self.answer)(self.seq, self.cmd))
    }
    async fn reset(&mut self) -> Result<(), TransportError> {
        self.resets += 1;
        Ok(())
    }
}

fn frame(version: u8, cmd: u8, status: u8, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = proto::Header::new(version, cmd, status, seq, payload)
        .encode()
        .to_vec();
    f.extend_from_slice(payload);
    f
}

fn fake(answer: fn(u16, u8) -> Vec<u8>) -> Client<Fake> {
    Client::new(Fake {
        answer,
        seq: 0,
        cmd: 0,
        resets: 0,
    })
}

#[test]
fn a_device_speaking_another_version_is_refused() {
    let mut c = fake(|seq, cmd| frame(VERSION + 1, cmd, Status::Version as u8, seq, &[]));
    let e = block_on(c.info()).unwrap_err();
    assert_eq!(
        e,
        Error::Version {
            device: VERSION + 1
        }
    );
    assert!(e.to_string().contains("version 2"), "{e}");
    assert_eq!(c.transport().resets, 0, "refused at once, not retried");
    // Even when its answer would otherwise be fine.
    let mut c = fake(|seq, cmd| frame(VERSION + 1, cmd, 0, seq, &[]));
    assert!(matches!(
        block_on(c.load()),
        Err(Error::Version { device: 2 })
    ));
}

#[test]
fn answers_that_never_make_sense_end_after_every_attempt() {
    // Damaged every time.
    let mut c = fake(|seq, cmd| {
        let mut f = frame(VERSION, cmd, 0, seq, &[]);
        f[13] ^= 1;
        f
    });
    let e = block_on(c.load()).unwrap_err();
    assert!(matches!(&e, Error::Protocol(m) if m.contains("CRC")), "{e}");
    assert_eq!(c.transport().resets, ATTEMPTS as u32 - 1);
    // Always another sequence number.
    let mut c = fake(|seq, cmd| frame(VERSION, cmd, 0, seq.wrapping_add(7), &[]));
    assert!(matches!(block_on(c.load()), Err(Error::Protocol(_))));
    // Always "it arrived damaged".
    let mut c = fake(|seq, cmd| frame(VERSION, cmd, Status::BadFrame as u8, seq, &[]));
    let e = block_on(c.load()).unwrap_err();
    assert!(
        matches!(&e, Error::Protocol(m) if m.contains("damaged")),
        "{e}"
    );
    // A status this host does not know: not retried.
    let mut c = fake(|seq, cmd| frame(VERSION, cmd, 200, seq, &[]));
    assert!(matches!(block_on(c.load()), Err(Error::Protocol(_))));
    assert_eq!(c.transport().resets, 0);
    // A payload where none is due, and a READ of the wrong length.
    let mut c = fake(|seq, cmd| frame(VERSION, cmd, 0, seq, &[1, 2, 3]));
    assert!(matches!(block_on(c.load()), Err(Error::Protocol(_))));
    assert!(matches!(
        block_on(c.read(TABLE_BASE, 4, &mut |_| ())),
        Err(Error::Protocol(_))
    ));
}

/// What goes on the wire is the device's own format: a frame `proto`
/// decodes, the sequence number new for every request.
#[test]
fn requests_are_frames_with_fresh_sequence_numbers() {
    struct Record(Vec<Vec<u8>>);
    impl Transport for Record {
        async fn bulk_out(&mut self, data: &[u8]) -> Result<(), TransportError> {
            self.0.push(data.to_vec());
            Ok(())
        }
        async fn bulk_in(&mut self, _len: usize) -> Result<Vec<u8>, TransportError> {
            let (h, _) = proto::decode(self.0.last().unwrap()).unwrap();
            Ok(frame(VERSION, h.command, 0, h.seq, &[]))
        }
        async fn reset(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
    }
    let mut c = Client::new(Record(Vec::new()));
    block_on(c.erase(TABLE_BASE, 8192)).unwrap();
    block_on(c.commit(1)).unwrap();
    let sent = &c.transport().0;
    assert_eq!(sent.len(), 3);
    let decoded: Vec<_> = sent.iter().map(|f| proto::decode(f).unwrap()).collect();
    assert_eq!(decoded[0].0.command, Command::Erase as u8);
    assert_eq!(decoded[0].1, TABLE_BASE.to_le_bytes());
    assert_eq!(decoded[1].1, (TABLE_BASE + 4096).to_le_bytes());
    assert_eq!(
        (decoded[2].0.command, decoded[2].1),
        (Command::Commit as u8, &[1u8][..])
    );
    let seqs: Vec<u16> = decoded.iter().map(|(h, _)| h.seq).collect();
    assert_eq!(seqs, [1, 2, 3]);
    assert!(sent
        .iter()
        .all(|f| f.len() >= HEADER_LEN && f[4] == VERSION));
}

/// The page's rule for finding the update interface, on the descriptors
/// the firmware sends (fastprom-usb's), and the simulator enumerating with
/// them.
#[test]
fn finds_the_update_interface_in_both_firmwares() {
    use fastprom_host::transport::find_update_interface;
    use fastprom_usb::desc::{CONFIG_APP, CONFIG_DFU};
    use fastprom_usb::ids::UPDATE_INTERFACE;
    for config in [&CONFIG_APP[..], &CONFIG_DFU[..]] {
        let found = find_update_interface(config).unwrap();
        assert_eq!(
            (found.interface, found.ep_out, found.ep_in),
            (UPDATE_INTERFACE, 0x01, 0x81)
        );
    }
    assert_eq!(UPDATE_INTERFACE, 0);
    assert!(find_update_interface(&[9, 2, 9, 0]).is_none());
    assert!(find_update_interface(&[]).is_none());
    // A vendor interface without its two bulk endpoints is not it.
    let mut lonely = vec![9, 2, 41, 0, 2, 1, 0, 0x80, 50];
    lonely.extend([9, 4, 0, 0, 0, 0xFF, 0, 0, 0]);
    lonely.extend([9, 4, 1, 0, 2, 0xFF, 0, 0, 0]);
    lonely.extend([7, 5, 0x02, 2, 64, 0, 0, 7, 5, 0x82, 2, 64, 0, 0]);
    assert_eq!(find_update_interface(&lonely).unwrap().interface, 1);

    let mut d = device();
    assert_eq!(d.configuration_descriptor(), &CONFIG_APP[..]);
    // bcdDevice, little-endian at offset 12.
    assert_eq!(&d.device_descriptor()[12..14], &[0x00, 0x01]);
    let serial = d.serial();
    assert_eq!(serial.len(), 32);
    block_on(fastprom_host::update::enter_dfu(&mut Client::new(
        SimTransport(&mut d),
    )))
    .unwrap();
    d.complete_reboot();
    assert_eq!(d.configuration_descriptor(), &CONFIG_DFU[..]);
    assert_eq!(&d.device_descriptor()[12..14], &[0x01, 0x00]);
    assert_eq!(d.serial(), serial, "the same serial in both");
}
