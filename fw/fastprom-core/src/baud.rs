//! UART0 baud-rate divisors (an ARM PL011).
//!
//! The formula is AGM's `UART_Init` (AgRV_pio
//! `packages/framework-agrv_sdk/src/uart.c`): a 16x oversampled integer part
//! and a 6-bit fraction, rounded, with a fraction that rounds up to 64
//! carried into the integer part.

/// The `IBRD` / `FBRD` register values for one baud rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divisors {
    ibrd: u16,
    fbrd: u8,
}

impl Divisors {
    /// Divisors for `baud` from a peripheral clock of `pclk_hz`, or `None`
    /// if the PL011 cannot divide down that far (the integer part must be
    /// 1 to 65535).
    pub fn new(pclk_hz: u32, baud: u32) -> Option<Self> {
        if baud == 0 {
            return None;
        }
        let pclk = u64::from(pclk_hz);
        let baud = u64::from(baud);
        let mut ibrd = pclk / baud / 16;
        let rem = pclk - ibrd * baud * 16;
        let mut fbrd = (rem * 4 + baud / 2) / baud;
        if fbrd >= 64 {
            ibrd += 1;
            fbrd = 0;
        }
        Some(Self {
            ibrd: u16::try_from(ibrd).ok().filter(|&i| i != 0)?,
            fbrd: u8::try_from(fbrd).ok()?,
        })
    }

    /// The integer divisor register, `IBRD`.
    pub const fn ibrd(self) -> u16 {
        self.ibrd
    }

    /// The fractional divisor register, `FBRD` (0 to 63).
    pub const fn fbrd(self) -> u8 {
        self.fbrd
    }

    /// The baud rate these divisors actually give from `pclk_hz`.
    pub fn actual_baud(self, pclk_hz: u32) -> u32 {
        // baud = pclk / (16 * (ibrd + fbrd / 64)) = 4 * pclk / (64 * ibrd + fbrd)
        // ibrd >= 1, so div >= 64 and the quotient is at most pclk_hz / 16:
        // the conversion cannot saturate.
        let div = 64 * u64::from(self.ibrd) + u64::from(self.fbrd);
        u32::try_from(4 * u64::from(pclk_hz) / div).unwrap_or(u32::MAX)
    }
}
