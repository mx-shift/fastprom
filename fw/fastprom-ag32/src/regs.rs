//! The AG32 registers this firmware uses.
//!
//! From AGM's AgRV_pio SDK, `packages/framework-agrv_sdk/src/` (file named
//! per block), with the board's pin routing from `tools/gen_pins.py`.

use crate::mmio::Reg;

/// Clocks the firmware assumes.
pub mod hz {
    /// HSI, the internal RC the core starts on: system.h
    /// BOARD_HSI_FREQUENCY (nominal; AGM states no accuracy).
    pub const HSI: u32 = 10_000_000;
    /// PLL output 0, the system clock: `SYSCLK 100` in the .ve that
    /// tools/gen_pins.py writes (gen_vlog's top.hx BOARD_PLL_FREQUENCY).
    /// The PLL itself is configured by the fabric bitstream from the 12 MHz
    /// oscillator on OSC_IN (`HSECLK 12`).
    pub const PLL: u32 = 100_000_000;
    /// The flash's fastest clock: flash.h FLASH_MAX_FREQ.
    pub const FLASH_MAX: u32 = 100_000_000;
}

/// System control: system.h SYS_ControlTypeDef at SYSTEMCTRL_BASE.
pub mod sys {
    use super::Reg;

    const BASE: usize = 0x0300_0000;
    // SAFETY: SYS_ControlTypeDef registers, 32-bit, always accessible.
    pub const CLK_CNTL: Reg = unsafe { Reg::new(BASE + 0x0C) };
    pub const PBUS_DIVIDER: Reg = unsafe { Reg::new(BASE + 0x38) };
    pub const APB_CLKENABLE: Reg = unsafe { Reg::new(BASE + 0x60) };

    // CLK_CNTL bits.
    pub const CLK_SOURCE_MASK: u32 = 0b11;
    pub const CLK_SOURCE_PLL: u32 = 2;
    pub const CLK_HSE_ON: u32 = 1 << 2;
    pub const CLK_HSE_BYP: u32 = 1 << 3;
    pub const CLK_HSE_RDY: u32 = 1 << 4;
    pub const CLK_PLL_ON: u32 = 1 << 5;
    pub const CLK_PLL_RDY: u32 = 1 << 6;
    /// SYS_SCLK_DIV_HIGH / _LOW: the flash clock divider, both nibbles.
    pub const SCLK_DIV_HIGH_SHIFT: u32 = 8;
    pub const SCLK_DIV_LOW_SHIFT: u32 = 12;
    pub const SCLK_DIV_MASK: u32 = 0xF;

    // APB_CLKENABLE bits: AltaRiscv.h APB_MASK_*.
    pub const APB_GPIO6: u32 = 1 << 10;
    pub const APB_GPIO7: u32 = 1 << 11;
    pub const APB_UART0: u32 = 1 << 21;
}

/// The image-select pins, as GPIO (gpio.h GPIO_TypeDef, an ARM PL061).
///
/// The MCU reaches a package pin only through the fabric's routing:
/// tools/gen_pins.py's .ve wires PIN_20 to GPIO7 bit 6 and PIN_21 to GPIO6
/// bit 1, the bits UART0's alternate function uses (AltaRiscv.h
/// UART0_UARTTXD_AF_GPIO 7, mask 1 << 6; UART0_UARTRXD_AF_GPIO 6, mask
/// 1 << 1). The pads have no internal pull (Supra's CFG_PULL_UP = 0), so the
/// board's 100 k pull-downs set an open switch's level.
pub mod pins {
    use super::Reg;

    const GPIO6: usize = 0x4001_A000;
    const GPIO7: usize = 0x4001_B000;
    /// PL061 GPIODATA is address-masked: offset (mask << 2) reads and
    /// writes only the bits in mask.
    const fn data(base: usize, mask: u32) -> usize {
        base + ((mask as usize) << 2)
    }
    const DIR: usize = 0x400;
    const AFSEL: usize = 0x420;

    /// UART0 TX, PIN_20: GPIO7 bit 6. Switch B, image-select bit 0.
    pub const TX_MASK: u32 = 1 << 6;
    /// UART0 RX, PIN_21: GPIO6 bit 1. Switch A, image-select bit 1.
    pub const RX_MASK: u32 = 1 << 1;

    // SAFETY: GPIO_TypeDef registers of GPIO6 and GPIO7, 32-bit; the data
    // addresses are inside GpioDATA[256].
    pub const TX_DATA: Reg = unsafe { Reg::new(data(GPIO7, TX_MASK)) };
    pub const TX_DIR: Reg = unsafe { Reg::new(GPIO7 + DIR) };
    pub const TX_AFSEL: Reg = unsafe { Reg::new(GPIO7 + AFSEL) };
    pub const RX_DATA: Reg = unsafe { Reg::new(data(GPIO6, RX_MASK)) };
    pub const RX_DIR: Reg = unsafe { Reg::new(GPIO6 + DIR) };
    pub const RX_AFSEL: Reg = unsafe { Reg::new(GPIO6 + AFSEL) };
}

/// UART0 (uart.h UART_TypeDef, an ARM PL011) at UART0_BASE.
pub mod uart0 {
    use super::Reg;

    const BASE: usize = 0x4002_5000;
    // SAFETY: UART_TypeDef registers, 32-bit.
    pub const DR: Reg = unsafe { Reg::new(BASE) }; // offset 0x00
    pub const FR: Reg = unsafe { Reg::new(BASE + 0x18) };
    pub const IBRD: Reg = unsafe { Reg::new(BASE + 0x24) };
    pub const FBRD: Reg = unsafe { Reg::new(BASE + 0x28) };
    pub const LCR_H: Reg = unsafe { Reg::new(BASE + 0x2C) };
    pub const CR: Reg = unsafe { Reg::new(BASE + 0x30) };
    pub const IMSC: Reg = unsafe { Reg::new(BASE + 0x38) };
    pub const ICR: Reg = unsafe { Reg::new(BASE + 0x44) };
    pub const DMACR: Reg = unsafe { Reg::new(BASE + 0x48) };

    pub const FR_BUSY: u32 = 1 << 3;
    pub const FR_TXFF: u32 = 1 << 5;
    /// 8 data bits (UART_LCR_DATABITS_8), FIFOs on (UART_LCR_FIFO_16).
    pub const LCR_H_8N1_FIFO: u32 = (3 << 5) | (1 << 4);
    pub const CR_ENABLE_TX_RX: u32 = (1 << 0) | (1 << 8) | (1 << 9);
    pub const ICR_ALL: u32 = 0x7FF;
}

/// The fabric's AHB slave window: board.h MMIO_BASE, the CPU's mem_ahb
/// port into the fabric (hdl/fastprom_ahb_slave.v behind it).
pub mod fabric {
    pub const BASE: usize = 0x6000_0000;
}
