//! UART0 on PIN_20 / PIN_21: the boot report, transmit only.
//!
//! Register sequence from AGM's `UART_InitBaud` (framework-agrv_sdk
//! src/uart.c), divisors from [`fastprom_core::baud`].

use core::fmt;

use fastprom_core::baud::Divisors;

use crate::regs::{sys, uart0};

/// Polls of the flag register before a byte is dropped: about 10 ms at
/// 100 MHz, several byte times at any sane baud rate.
const TX_POLLS: u32 = 1_000_000;

/// UART0, 8N1, FIFOs on, interrupts and DMA off.
#[derive(Debug)]
pub struct Uart0 {
    _owned: (),
}

impl Uart0 {
    /// Start UART0 at `baud` from a peripheral clock of `pclk_hz`, or
    /// `None` if the PL011 cannot divide down to it.
    pub fn new(pclk_hz: u32, baud: u32) -> Option<Self> {
        let div = Divisors::new(pclk_hz, baud)?;
        sys::APB_CLKENABLE.modify(0, sys::APB_UART0);
        uart0::CR.write(0);
        uart0::IBRD.write(u32::from(div.ibrd()));
        uart0::FBRD.write(u32::from(div.fbrd()));
        // LCR_H after the divisors: the PL011 latches them on this write.
        uart0::LCR_H.write(uart0::LCR_H_8N1_FIFO);
        uart0::ICR.write(uart0::ICR_ALL);
        uart0::IMSC.write(0);
        uart0::CR.write(uart0::CR_ENABLE_TX_RX);
        uart0::DMACR.write(0);
        Some(Self { _owned: () })
    }

    fn put(&mut self, byte: u8) {
        // A wedged UART must not hang the boot: drop the byte instead.
        if (0..TX_POLLS).any(|_| uart0::FR.read() & uart0::FR_TXFF == 0) {
            uart0::DR.write(u32::from(byte));
        }
    }

    /// Wait (bounded) until everything queued has gone out.
    pub fn flush(&mut self) {
        let _drained = (0..TX_POLLS).any(|_| uart0::FR.read() & uart0::FR_BUSY == 0);
    }
}

impl fmt::Write for Uart0 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.put(b'\r');
            }
            self.put(b);
        }
        Ok(())
    }
}
