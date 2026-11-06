//! The host side of ST's AN3155 USART boot loader protocol, as the AG32's
//! mask ROM and FastPROM's recovery loader speak it
//! (`fastprom_core::recovery` has the numbers and the evidence).
//!
//! One command at a time over a [`SerialPort`]: `cmd`, `!cmd`, ACK; then
//! the command's phases, each acknowledged. The mask ROM wants AGM's
//! memory-mode byte after the address of a read or write
//! ([`Loader::mask_rom`]); the recovery loader does not.
//!
//! When a command fails part way (a NACK, or nothing for its timeout),
//! [`Loader::resync`] gets loader and host back in step: it waits out the
//! recovery loader's idle timeout, then sends sync bytes until one is
//! answered, ACK or NACK. A sync byte the loader takes as the rest of a
//! frame is harmless: that frame's checksum then fails and is NACKed.

use fastprom_core::recovery::server::IDLE_MS;
use fastprom_core::recovery::{
    address_frame, xor, ACK, CMD_ERASE, CMD_ERASE_OPTIONS, CMD_GET, CMD_GID, CMD_GO, CMD_READ,
    CMD_READOUT_UNPROTECT, CMD_RESET, CMD_WRITE, MAX_FRAME, NACK, SYNC,
};

use crate::serial::{SerialError, SerialPort};

/// How long a loader has to acknowledge a command or a phase.
pub const ACK_MS: u32 = 500;
/// Per sector erased (a 4 KiB SPI NOR sector erase is 45 to 400 ms).
pub const ERASE_MS_PER_SECTOR: u32 = 500;
/// An option-byte erase or write: programming read protection off may erase
/// the whole chip first (AGM's uart_master.c).
pub const OPTIONS_MS: u32 = 30_000;
/// `READOUT UNPROTECT`: a whole-chip erase.
pub const UNPROTECT_MS: u32 = 30_000;
/// Sync bytes sent before giving up on an answer. Enough to fill a whole
/// write frame the loader may still be waiting for the rest of.
pub const SYNC_ATTEMPTS: u32 = 300;
/// How long each sync byte waits for its answer.
pub const SYNC_WAIT_MS: u32 = 40;

/// Why a command did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderError {
    /// No answer within the timeout.
    Timeout(String),
    /// Refused.
    Nack(String),
    /// An answer that is not this protocol's.
    Unexpected(String),
    /// The port itself failed.
    Port(SerialError),
}

impl LoaderError {
    /// A failure that resynchronising and trying again can fix.
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Timeout(_) | Self::Nack(_) | Self::Unexpected(_))
    }
}

impl core::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Timeout(what) => write!(f, "no answer to {what}"),
            Self::Nack(what) => write!(f, "the boot loader refused {what}"),
            Self::Unexpected(what) => write!(f, "{what}"),
            Self::Port(e) => write!(f, "{e}"),
        }
    }
}

impl From<SerialError> for LoaderError {
    fn from(e: SerialError) -> Self {
        Self::Port(e)
    }
}

/// What `GET` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Get {
    pub version: u8,
    pub commands: Vec<u8>,
}

/// A boot loader on the other end of a serial port.
pub struct Loader<P> {
    port: P,
    mmode: Option<u8>,
}

impl<P: SerialPort> Loader<P> {
    /// The recovery loader's dialect: plain AN3155.
    pub const fn new(port: P) -> Self {
        Self { port, mmode: None }
    }

    /// Talk to the mask ROM: `mode` after each read's and write's address.
    pub fn mask_rom(&mut self, mode: u8) {
        self.mmode = Some(mode);
    }

    /// Plain AN3155 again (the recovery loader).
    pub fn plain(&mut self) {
        self.mmode = None;
    }

    pub fn port(&mut self) -> &mut P {
        &mut self.port
    }

    async fn read_exact(&mut self, n: usize, ms: u32, what: &str) -> Result<Vec<u8>, LoaderError> {
        let got = self.port.read(n, ms).await?;
        if got.len() < n {
            return Err(LoaderError::Timeout(what.into()));
        }
        Ok(got)
    }

    async fn ack(&mut self, ms: u32, what: &str) -> Result<(), LoaderError> {
        match self.read_exact(1, ms, what).await?.first() {
            Some(&ACK) => Ok(()),
            Some(&NACK) => Err(LoaderError::Nack(what.into())),
            Some(b) => Err(LoaderError::Unexpected(format!(
                "the boot loader answered {what} with {b:#04x}, not ACK or NACK"
            ))),
            None => Err(LoaderError::Timeout(what.into())),
        }
    }

    async fn command(&mut self, cmd: u8, what: &str) -> Result<(), LoaderError> {
        self.port.write(&[cmd, !cmd]).await?;
        self.ack(ACK_MS, what).await
    }

    async fn address(&mut self, addr: u32, what: &str) -> Result<(), LoaderError> {
        self.port.write(&address_frame(addr)).await?;
        self.ack(ACK_MS, what).await?;
        if let Some(m) = self.mmode {
            self.port.write(&[m, !m]).await?;
            self.ack(ACK_MS, what).await?;
        }
        Ok(())
    }

    /// The first sync after reset (the loader measures the baud rate on
    /// it), or after a failure: send sync bytes until one is answered.
    /// `Ok(true)` for ACK (a loader waiting for its first sync, or the
    /// recovery loader), `Ok(false)` for NACK (a loader that took it as a
    /// command and is now in step).
    ///
    /// # Errors
    ///
    /// No answer to any of [`SYNC_ATTEMPTS`].
    pub async fn sync(&mut self) -> Result<bool, LoaderError> {
        self.port.discard().await?;
        for _ in 0..SYNC_ATTEMPTS {
            self.port.write(&[SYNC]).await?;
            match self.port.read(1, SYNC_WAIT_MS).await?.first() {
                Some(&ACK) => {
                    self.port.discard().await?;
                    return Ok(true);
                }
                Some(&NACK) => {
                    self.port.discard().await?;
                    return Ok(false);
                }
                _ => {}
            }
        }
        Err(LoaderError::Timeout("the sync byte".into()))
    }

    /// After a failed command: wait out the recovery loader's idle timeout,
    /// drop what is left, and sync.
    ///
    /// # Errors
    ///
    /// As [`Loader::sync`].
    pub async fn resync(&mut self) -> Result<(), LoaderError> {
        self.port.sleep(IDLE_MS + IDLE_MS / 2).await;
        self.sync().await.map(|_| ())
    }

    /// `GET`: the version and the commands.
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn get(&mut self) -> Result<Get, LoaderError> {
        self.command(CMD_GET, "GET").await?;
        let n = self.read_exact(1, ACK_MS, "GET").await?[0];
        let body = self.read_exact(usize::from(n) + 1, ACK_MS, "GET").await?;
        self.ack(ACK_MS, "GET").await?;
        Ok(Get {
            version: body[0],
            commands: body[1..].to_vec(),
        })
    }

    /// `GID`: the device ID, as the bytes say it (two or four).
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn gid(&mut self) -> Result<u32, LoaderError> {
        self.command(CMD_GID, "GID").await?;
        let n = self.read_exact(1, ACK_MS, "GID").await?[0];
        if !matches!(n, 1 | 3) {
            return Err(LoaderError::Unexpected(format!(
                "GID answered {} bytes, not 2 or 4",
                usize::from(n) + 1
            )));
        }
        let id = self
            .read_exact(usize::from(n) + 1, ACK_MS, "GID")
            .await?
            .iter()
            .fold(0u32, |a, &b| (a << 8) | u32::from(b));
        self.ack(ACK_MS, "GID").await?;
        Ok(id)
    }

    /// `READ MEMORY`: `len` bytes (1 to 256) at `addr`.
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn read(&mut self, addr: u32, len: usize) -> Result<Vec<u8>, LoaderError> {
        let what = format!("a read of {len} bytes at {addr:#010x}");
        if !(1..=MAX_FRAME).contains(&len) {
            return Err(LoaderError::Unexpected(what));
        }
        self.command(CMD_READ, &what).await?;
        self.address(addr, &what).await?;
        let n = (len - 1) as u8;
        self.port.write(&[n, !n]).await?;
        self.ack(ACK_MS, &what).await?;
        self.read_exact(len, ACK_MS, &what).await
    }

    /// `WRITE MEMORY`: `data` (1 to 256 bytes) at `addr`, acknowledged when
    /// written (and, by the recovery loader, read back).
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn write(&mut self, addr: u32, data: &[u8], ms: u32) -> Result<(), LoaderError> {
        let what = format!("a write of {} bytes at {addr:#010x}", data.len());
        if !(1..=MAX_FRAME).contains(&data.len()) {
            return Err(LoaderError::Unexpected(what));
        }
        self.command(CMD_WRITE, &what).await?;
        self.address(addr, &what).await?;
        let mut frame = Vec::with_capacity(data.len() + 2);
        frame.push((data.len() - 1) as u8);
        frame.extend_from_slice(data);
        frame.push(xor(&frame));
        self.port.write(&frame).await?;
        self.ack(ms, &what).await
    }

    /// `GO`: jump to `addr`. The loader answers, then runs what is there.
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn go(&mut self, addr: u32) -> Result<(), LoaderError> {
        let what = format!("GO {addr:#010x}");
        self.command(CMD_GO, &what).await?;
        self.port.write(&address_frame(addr)).await?;
        self.ack(ACK_MS, &what).await
    }

    /// `EXTENDED ERASE` of the listed 4 KiB sectors (by number).
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn erase(&mut self, sectors: &[u16]) -> Result<(), LoaderError> {
        let what = format!("an erase of {} sectors from {}", sectors.len(), sectors[0]);
        self.command(CMD_ERASE, &what).await?;
        let mut frame = ((sectors.len() - 1) as u16).to_be_bytes().to_vec();
        for s in sectors {
            frame.extend_from_slice(&s.to_be_bytes());
        }
        frame.push(xor(&frame));
        self.port.write(&frame).await?;
        let ms = ERASE_MS_PER_SECTOR * sectors.len() as u32 + ACK_MS;
        self.ack(ms, &what).await
    }

    /// AGM's erase of the option bytes.
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn erase_options(&mut self) -> Result<(), LoaderError> {
        self.command(CMD_ERASE_OPTIONS, "the option-byte erase")
            .await?;
        self.ack(OPTIONS_MS, "the option-byte erase").await
    }

    /// `READOUT UNPROTECT`: the chip erases its flash and drops read
    /// protection, then resets (sync again after it).
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn readout_unprotect(&mut self) -> Result<(), LoaderError> {
        self.command(CMD_READOUT_UNPROTECT, "READOUT UNPROTECT")
            .await?;
        self.ack(UNPROTECT_MS, "READOUT UNPROTECT").await
    }

    /// AGM's reset: two ACKs, then the chip resets.
    ///
    /// # Errors
    ///
    /// As the loader answers.
    pub async fn reset(&mut self) -> Result<(), LoaderError> {
        self.command(CMD_RESET, "the reset").await?;
        self.ack(ACK_MS, "the reset").await
    }
}
