//! Reading the image-select switch, then handing its pins to UART0.

use fastprom_core::SwitchSample;

use crate::clock::Clocks;
use crate::delay;
use crate::regs::{pins, sys};

/// Time for the pins to settle once they are inputs. The 100 k pull-down
/// against the pad and trace capacitance is a few microseconds.
const SETTLE_US: u32 = 1_000;
/// Between samples, so the run spans about a millisecond.
const SPACING_US: u32 = 64;

/// Sample both switch pins `N` times as plain GPIO inputs. Must run before
/// [`give_to_uart0`]: once UART0 owns TX it drives the pin.
pub fn sample<const N: usize>(clocks: Clocks) -> [SwitchSample; N] {
    sys::APB_CLKENABLE.modify(0, sys::APB_GPIO6 | sys::APB_GPIO7);
    // Software mode (alternate function off), input.
    pins::TX_AFSEL.modify(pins::TX_MASK, 0);
    pins::RX_AFSEL.modify(pins::RX_MASK, 0);
    pins::TX_DIR.modify(pins::TX_MASK, 0);
    pins::RX_DIR.modify(pins::RX_MASK, 0);
    delay::us(clocks.core_hz(), SETTLE_US);

    core::array::from_fn(|_| {
        let s = SwitchSample {
            tx: pins::TX_DATA.read() & pins::TX_MASK != 0,
            rx: pins::RX_DATA.read() & pins::RX_MASK != 0,
        };
        delay::us(clocks.core_hz(), SPACING_US);
        s
    })
}

/// Give PIN_20 and PIN_21 to UART0's alternate function (gpio.h
/// GPIO_AF_ENABLE for UART0_UARTTXD and UART0_UARTRXD).
pub fn give_to_uart0() {
    pins::TX_AFSEL.modify(0, pins::TX_MASK);
    pins::RX_AFSEL.modify(0, pins::RX_MASK);
}
