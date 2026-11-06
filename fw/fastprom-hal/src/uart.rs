//! UART0 both ways, 8E1, for the recovery loader: the framing the mask
//! ROM's serial loader uses (AGM's booter `UART_InitBaud(...,
//! UART_LCR_PARITY_EVEN, UART_LCR_FIFO_16)`, its flasher's `-m 8e1`), so
//! the host keeps its port as it was for the mask ROM.
//!
//! The pins: PIN_20 (TX) and PIN_21 (RX) through UART0's alternate function
//! (gpio.h GPIO_AF_ENABLE, AGM's booter does the same after the mask ROM
//! jumps to it), with GPIO6's and GPIO7's clocks on. Register sequence from
//! uart.c `UART_InitBaud`, divisors from [`fastprom_core::baud`].

use fastprom_core::baud::Divisors;

use crate::regs::{pins, sys, uart0};

/// Polls of the flag register before a byte is dropped, or a flush given
/// up: about 10 ms at 100 MHz, many byte times at any baud rate here.
const TX_POLLS: u32 = 1_000_000;

/// UART0, 8E1, FIFOs on, interrupts and DMA off.
#[derive(Debug)]
pub struct Uart0 {
    _owned: (),
}

impl Uart0 {
    /// Take PIN_20 and PIN_21 and start UART0 at `baud` from a peripheral
    /// clock of `pclk_hz`, or `None` if the PL011 cannot divide down to it.
    pub fn new(pclk_hz: u32, baud: u32) -> Option<Self> {
        let div = Divisors::new(pclk_hz, baud)?;
        sys::APB_CLKENABLE.modify(0, sys::APB_GPIO6 | sys::APB_GPIO7 | sys::APB_UART0);
        pins::TX_AFSEL.modify(0, pins::TX_MASK);
        pins::RX_AFSEL.modify(0, pins::RX_MASK);
        uart0::CR.write(0);
        uart0::IBRD.write(u32::from(div.ibrd()));
        uart0::FBRD.write(u32::from(div.fbrd()));
        // LCR_H after the divisors: the PL011 latches them on this write.
        uart0::LCR_H.write(uart0::LCR_H_8E1_FIFO);
        uart0::ICR.write(uart0::ICR_ALL);
        uart0::IMSC.write(0);
        uart0::CR.write(uart0::CR_ENABLE_TX_RX);
        uart0::DMACR.write(0);
        Some(Self { _owned: () })
    }

    /// A received byte, if one is waiting. A byte with a parity or framing
    /// error is returned all the same: the protocol's checksums refuse
    /// what it was part of, and dropping it would put the host's lengths
    /// out of step.
    pub fn read(&mut self) -> Option<u8> {
        if uart0::FR.read() & uart0::FR_RXFE != 0 {
            return None;
        }
        let d = uart0::DR.read();
        if d & uart0::DR_ERRORS != 0 {
            uart0::ICR.write(uart0::ICR_ALL);
        }
        Some(d as u8)
    }

    /// Queue `bytes`, waiting (bounded) for room in the FIFO.
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            // A wedged UART must not hang the loader: drop the byte instead.
            if (0..TX_POLLS).any(|_| uart0::FR.read() & uart0::FR_TXFF == 0) {
                uart0::DR.write(u32::from(b));
            }
        }
    }
}

/// Wait (bounded) until everything queued on UART0 has gone out: before a
/// reset, so the host hears the acknowledgement.
pub fn flush() {
    let _drained = (0..TX_POLLS).any(|_| uart0::FR.read() & uart0::FR_BUSY == 0);
}
