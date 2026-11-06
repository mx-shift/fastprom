//! Busy-wait delays on the `mcycle` counter.

/// Wait about `us` microseconds at a core clock of `core_hz`.
///
/// Bounded even if `mcycle` does not count (a core with it inhibited):
/// every pass of the loop takes at least a cycle, so it gives up after as
/// many passes as the cycles it waits for.
pub fn us(core_hz: u32, us: u32) {
    let cycles = (core_hz / 1_000_000).saturating_mul(us);
    let start = riscv::register::mcycle::read();
    for _ in 0..cycles {
        if (riscv::register::mcycle::read().wrapping_sub(start) as u32) >= cycles {
            break;
        }
    }
}

/// Something to do every `us` microseconds from a loop that does other
/// work between checks. Bounded like [`us`]: if `mcycle` does not count, it
/// falls due after as many checks as the cycles in a period.
#[derive(Debug)]
pub struct Every {
    period: u32,
    start: u32,
    checks: u32,
}

impl Every {
    /// A period of `us` at a core clock of `core_hz`, starting now.
    pub fn new(core_hz: u32, us: u32) -> Self {
        Self {
            period: (core_hz / 1_000_000).saturating_mul(us),
            start: now(),
            checks: 0,
        }
    }

    /// Whether a period has passed since the last time this said so.
    pub fn due(&mut self) -> bool {
        self.checks = self.checks.saturating_add(1);
        let t = now();
        if t.wrapping_sub(self.start) >= self.period || self.checks >= self.period {
            self.start = t;
            self.checks = 0;
            true
        } else {
            false
        }
    }
}

fn now() -> u32 {
    riscv::register::mcycle::read() as u32
}
