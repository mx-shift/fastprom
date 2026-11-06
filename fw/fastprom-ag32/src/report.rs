//! The boot report on UART0 and the USB serial port, for bring-up and the
//! field.
//!
//! Lines are plain text at 115200 8N1 on J3 (TX pad 1). A failure line
//! starts `FAIL Ennn`, with the stable code from
//! [`fastprom_core::FailureCode`], then the error and each of its causes,
//! and is repeated every two seconds so an adapter attached later still
//! sees it.

use core::error::Error;
use core::fmt::{self, Write};

use fastprom_core::{select, FabricError, FailureCode, Released, Report, SwitchSample};

use fastprom_core::memmap::TABLE_BASE;
use fastprom_hal::clock::{ClockError, Clocks};

use crate::error::FwError;
use crate::fabric::Ag32Fabric;
use crate::uart::Uart0;
use fastprom_usb::fifo::Fifo;

/// What the report keeps for the USB serial port: the banner and outcome
/// are written before USB starts, so they wait here, and every later line
/// passes through. A full log drops what does not fit.
pub const LOG_LEN: usize = 512;

/// The copy of the report kept for USB.
struct Log(Fifo<LOG_LEN>);

impl Write for Log {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.push(s.as_bytes());
        Ok(())
    }
}

/// Where the report goes: UART0 if it could be started, and a log for the
/// USB serial port (`docs/11-usb-driver.md`).
pub struct Console {
    uart: Option<Uart0>,
    log: Log,
}

impl Console {
    pub const fn new(uart: Option<Uart0>) -> Self {
        Self {
            uart,
            log: Log(Fifo::new()),
        }
    }

    /// Write one formatted chunk. Uart0 never fails a write (it drops
    /// bytes it cannot send), nor does the log, and a console has nowhere
    /// to report its own failure, so the results are dropped here and only
    /// here.
    fn put(&mut self, args: fmt::Arguments<'_>) {
        if let Some(uart) = &mut self.uart {
            let _infallible = uart.write_fmt(args);
        }
        let _infallible = self.log.write_fmt(args);
    }

    pub fn flush(&mut self) {
        if let Some(uart) = &mut self.uart {
            uart.flush();
        }
    }

    /// Hand what the log holds to `sink` (the USB serial port), which says
    /// how much it took.
    pub fn drain_log(&mut self, sink: impl FnOnce(&[u8]) -> usize) {
        let mut chunk = [0u8; 64];
        let n = self.log.0.peek(&mut chunk);
        if n != 0 {
            let taken = sink(chunk.get(..n).unwrap_or(&[]));
            self.log.0.consume(taken);
        }
    }
}

macro_rules! say {
    ($c:expr, $($arg:tt)*) => { $c.put(format_args!($($arg)*)) };
}

fn closed(level: bool) -> &'static str {
    if level {
        "closed"
    } else {
        "open"
    }
}

/// Who we are, what the switch reads, how the clocks came up, and whether
/// the fabric answered.
pub fn banner(
    c: &mut Console,
    samples: &[SwitchSample],
    clocks: &Result<Clocks, ClockError>,
    claim: &Result<Released<'_, Ag32Fabric>, FabricError>,
) {
    say!(
        c,
        "\nFastPROM firmware {}, AG32VF303 BRAM mode\n",
        env!("CARGO_PKG_VERSION")
    );
    match (samples.first(), select::decide(samples)) {
        (Some(s), Ok(bank)) => say!(
            c,
            "SW1: A (RX, PIN_21) {}, B (TX, PIN_20) {}: image {}\n",
            closed(s.rx),
            closed(s.tx),
            bank
        ),
        (_, Err(e)) => say!(c, "SW1: {e}\n"),
        (None, Ok(_)) => {}
    }
    match clocks {
        Ok(k) => say!(
            c,
            "clock: {} MHz from the PLL (12 MHz oscillator, HSE bypass)\n",
            k.core_hz() / 1_000_000
        ),
        Err(e) => say!(c, "clock: still on HSI, about 10 MHz ({e})\n"),
    }
    match claim {
        Ok(r) => say!(
            c,
            "fabric: FastPROM logic, {}-word block RAM, outputs released\n",
            r.depth()
        ),
        Err(e) => say!(c, "fabric: {e}\n"),
    }
}

/// The result of the load.
pub fn outcome(c: &mut Console, result: &Result<Report<'_>, FwError>) {
    match result {
        Ok(r) => say!(
            c,
            "OK: serving image {} '{}' ({} of {}, {}x{}, part {}), crc32 {:#010x} at {:#010x}, READY set\n",
            r.loaded.bank(),
            r.name,
            r.loaded.bank().number() + 1,
            r.count,
            r.loaded.depth(),
            r.width.bits(),
            r.part,
            r.loaded.crc32(),
            TABLE_BASE
        ),
        Err(e) => failure(c, e),
    }
    c.flush();
}

/// `FAIL Ennn: error: cause: cause`, then where the module stands.
pub fn failure(c: &mut Console, e: &FwError) {
    say!(c, "FAIL E{:02}: {e}", e.code());
    let mut cause: Option<&dyn Error> = e.source();
    while let Some(err) = cause {
        say!(c, ": {err}");
        cause = err.source();
    }
    say!(
        c,
        "\n  socket outputs stay released (high impedance); idle\n"
    );
    c.flush();
}
