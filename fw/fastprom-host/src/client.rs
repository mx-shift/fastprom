//! The update protocol's client: one request at a time over a
//! [`Transport`], built and read with the device's own module
//! (`fastprom_core::update::proto`, which has the wire format).
//!
//! Each request is one bulk OUT transfer (and a zero-length packet after
//! it, should it fill its last packet), its response one bulk IN transfer
//! of [`IN_TRANSFER`] bytes, which always ends at the response's short
//! packet. The recovery rule is the protocol's: a response that does not
//! decode, that answers another sequence number or command, or that says
//! the request arrived damaged (`BadFrame`) is followed by a reset of the
//! interface and the request again, under a new sequence number, up to
//! [`ATTEMPTS`] times in all. Every command is idempotent, so a request
//! that ran before its response was lost runs again harmlessly. A response
//! in another protocol version is refused outright ([`Error::Version`]):
//! the page and the firmware must be from the same release.

use fastprom_core::update::proto::{
    self, Command, HEADER_LEN, IN_TRANSFER, PACKET, READ_MAX, VERSION, WRITE_LEN,
};
use fastprom_core::update::Status;

use crate::info::DeviceInfo;
use crate::transport::{Transport, TransportError};

/// How many times a request is sent before the client gives up on it.
pub const ATTEMPTS: usize = 3;

/// Why an operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The device refused a request with this status.
    Refused {
        command: &'static str,
        status: Status,
    },
    /// The device speaks another version of the protocol.
    Version { device: u8 },
    /// The transfer itself failed.
    Transport(TransportError),
    /// The device answered something the protocol does not allow, or
    /// nothing usable after every attempt.
    Protocol(String),
    /// A check on this side failed: the file, the board, the read-back.
    Check(String),
}

impl Error {
    /// The status the device refused with, if it refused.
    pub const fn status(&self) -> Option<Status> {
        match self {
            Self::Refused { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Whether the device went away (unplugged, rebooted, lost power).
    pub fn is_gone(&self) -> bool {
        matches!(self, Self::Transport(TransportError::Gone))
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused { command, status } => {
                write!(f, "the device refused {command}: {status}")
            }
            Self::Version { device } => write!(
                f,
                "the board speaks update protocol version {device} and this page \
                 version {VERSION}: use the page released with the board's firmware"
            ),
            Self::Transport(e) => write!(f, "USB: {e}"),
            Self::Protocol(s) => write!(f, "protocol error: {s}"),
            Self::Check(s) => f.write_str(s),
        }
    }
}

impl From<TransportError> for Error {
    fn from(e: TransportError) -> Self {
        Self::Transport(e)
    }
}

/// The client.
pub struct Client<T> {
    t: T,
    seq: u16,
    resyncs: u32,
}

impl<T: Transport> Client<T> {
    pub const fn new(t: T) -> Self {
        Self {
            t,
            seq: 0,
            resyncs: 0,
        }
    }

    /// The transport, for whoever owns the connection.
    pub fn transport(&mut self) -> &mut T {
        &mut self.t
    }

    pub fn into_inner(self) -> T {
        self.t
    }

    /// How many times a request has been sent again after a reset.
    pub const fn resyncs(&self) -> u32 {
        self.resyncs
    }

    /// Start clean: the interface reset, whatever a previous host left
    /// half done.
    ///
    /// # Errors
    ///
    /// When the transfer fails.
    pub async fn reset(&mut self) -> Result<(), Error> {
        self.t.reset().await?;
        Ok(())
    }

    /// Run one request; its response's payload.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] with the device's status, [`Error::Version`],
    /// how the transfer failed, or [`Error::Protocol`] when no attempt got
    /// a usable answer.
    pub async fn run(&mut self, cmd: Command, payload: &[u8]) -> Result<Vec<u8>, Error> {
        let mut last = String::new();
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                self.resyncs += 1;
                self.t.reset().await?;
            }
            self.seq = self.seq.wrapping_add(1);
            let mut frame = vec![0u8; HEADER_LEN + payload.len()];
            proto::encode(&mut frame, cmd as u8, 0, self.seq, payload)
                .ok_or_else(|| Error::Protocol(format!("{}: too long a request", cmd.name())))?;
            self.t.bulk_out(&frame).await?;
            if frame.len().is_multiple_of(PACKET) {
                // The frame ends with a short packet.
                self.t.bulk_out(&[]).await?;
            }
            let r = self.t.bulk_in(IN_TRANSFER).await?;
            let (h, p) = match proto::decode(&r) {
                Ok(d) => d,
                Err(e) => {
                    last = format!("{}: a response with {}", cmd.name(), e.as_str());
                    continue;
                }
            };
            if h.version != VERSION {
                return Err(Error::Version { device: h.version });
            }
            if h.seq != self.seq || h.command != cmd as u8 {
                last = format!(
                    "{}: the answer to request {} (command {}), not {}",
                    cmd.name(),
                    h.seq,
                    h.command,
                    self.seq
                );
                continue;
            }
            return match Status::from_u8(h.status) {
                Some(Status::Ok) => Ok(p.to_vec()),
                Some(Status::BadFrame) => {
                    last = format!("{}: the device received it damaged", cmd.name());
                    continue;
                }
                Some(status) => Err(Error::Refused {
                    command: cmd.name(),
                    status,
                }),
                None => Err(Error::Protocol(format!(
                    "{}: status {}, which this page does not know",
                    cmd.name(),
                    h.status
                ))),
            };
        }
        Err(Error::Protocol(format!("{last} ({ATTEMPTS} attempts)")))
    }

    /// A request whose answer has no payload.
    async fn run_empty(&mut self, cmd: Command, payload: &[u8]) -> Result<(), Error> {
        let p = self.run(cmd, payload).await?;
        if p.is_empty() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "{}: {} bytes where none were due",
                cmd.name(),
                p.len()
            )))
        }
    }

    // ---- the commands ----------------------------------------------------

    /// `READ` `len` bytes at `addr`, [`READ_MAX`] at a time; `progress`
    /// gets the bytes done so far.
    ///
    /// # Errors
    ///
    /// As [`Client::run`].
    pub async fn read(
        &mut self,
        addr: u32,
        len: u32,
        progress: &mut dyn FnMut(u32),
    ) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(len as usize);
        let mut done = 0;
        while done < len {
            let n = (len - done).min(READ_MAX as u32);
            let mut p = (addr + done).to_le_bytes().to_vec();
            p.extend_from_slice(&n.to_le_bytes());
            let data = self.run(Command::Read, &p).await?;
            if data.len() != n as usize {
                return Err(Error::Protocol(format!(
                    "READ: {} bytes where {n} were asked for",
                    data.len()
                )));
            }
            out.extend_from_slice(&data);
            done += n;
            progress(done);
        }
        Ok(out)
    }

    /// `WRITE` `data` (whole pages) at `addr`, a page at a time.
    ///
    /// # Errors
    ///
    /// [`Error::Check`] for data that is not whole pages, or as
    /// [`Client::run`].
    pub async fn write(
        &mut self,
        addr: u32,
        data: &[u8],
        progress: &mut dyn FnMut(u32),
    ) -> Result<(), Error> {
        if !data.len().is_multiple_of(WRITE_LEN) {
            return Err(Error::Check(format!(
                "{} bytes to write: not whole {WRITE_LEN}-byte pages",
                data.len()
            )));
        }
        let mut done = 0u32;
        for page in data.chunks(WRITE_LEN) {
            let mut p = (addr + done).to_le_bytes().to_vec();
            p.extend_from_slice(page);
            self.run_empty(Command::Write, &p).await?;
            done += page.len() as u32;
            progress(done);
        }
        Ok(())
    }

    /// `ERASE` `len` bytes (whole sectors) at `addr`, a sector at a time.
    ///
    /// # Errors
    ///
    /// As [`Client::run`].
    pub async fn erase(&mut self, addr: u32, len: u32) -> Result<(), Error> {
        let sector = fastprom_core::memmap::SECTOR;
        let mut done = 0;
        while done < len {
            self.run_empty(Command::Erase, &(addr + done).to_le_bytes())
                .await?;
            done += sector;
        }
        Ok(())
    }

    /// `REBOOT` into `target` (`proto::TARGET_APPLICATION` or
    /// `proto::TARGET_DFU`). The device goes once the host has its answer.
    ///
    /// # Errors
    ///
    /// As [`Client::run`].
    pub async fn reboot(&mut self, target: u8) -> Result<(), Error> {
        self.run_empty(Command::Reboot, &[target]).await
    }

    /// `INFO`.
    ///
    /// # Errors
    ///
    /// As [`Client::run`], or [`Error::Protocol`] for an answer this host
    /// does not read.
    pub async fn info(&mut self) -> Result<DeviceInfo, Error> {
        let raw = self.run(Command::Info, &[]).await?;
        DeviceInfo::decode(&raw).map_err(Error::Protocol)
    }

    /// `LOAD` (application only).
    ///
    /// # Errors
    ///
    /// As [`Client::run`].
    pub async fn load(&mut self) -> Result<(), Error> {
        self.run_empty(Command::Load, &[]).await
    }

    /// `COMMIT` (DFU stage only): `proto::TARGET_APPLICATION` or
    /// `proto::TARGET_FABRIC`.
    ///
    /// # Errors
    ///
    /// As [`Client::run`].
    pub async fn commit(&mut self, target: u8) -> Result<(), Error> {
        self.run_empty(Command::Commit, &[target]).await
    }
}
