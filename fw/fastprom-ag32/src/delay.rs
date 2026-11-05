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
