//! The AG32 registers the FastPROM firmware uses.
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
    pub const RST_CNTL: Reg = unsafe { Reg::new(RST_CNTL_ADDR) };
    /// RST_CNTL's address, for the DFU stage's boot check (assembly).
    pub const RST_CNTL_ADDR: usize = BASE + 0x04;
    pub const CLK_CNTL: Reg = unsafe { Reg::new(BASE + 0x0C) };
    /// MISC_CNTL: the HSI configuration (system.h SYS_SetHSIConfig writes
    /// it whole).
    pub const MISC_CNTL: Reg = unsafe { Reg::new(BASE + 0x18) };
    pub const PBUS_DIVIDER: Reg = unsafe { Reg::new(BASE + 0x38) };
    pub const APB_CLKENABLE: Reg = unsafe { Reg::new(BASE + 0x60) };
    pub const AHB_CLKENABLE: Reg = unsafe { Reg::new(BASE + 0x70) };
    /// DEVICE_ID (0x100, SYS_GetDeviceID): 0x4020_0001 on the AG32VF303
    /// (AgRV_pio boards/agrv2k_303.json `device_id`).
    pub const DEVICE_ID: Reg = unsafe { Reg::new(BASE + 0x100) };

    // RST_CNTL bits: system.h SYS_RSTF_POR, SYS_RST_REMOVE, SYS_RST_SFT.
    /// Set by a power-on reset; sticky until RST_REMOVE.
    pub const RSTF_POR: u32 = 1 << 27;
    /// Write 1 to clear the reset flags (SYS_ClearResetFlags).
    pub const RST_REMOVE: u32 = 1 << 24;
    /// Write 1 to reset the chip (SYS_SoftwareReset).
    pub const RST_SFT: u32 = 1 << 0;
    /// FCB reset disable (system.h SYS_RST_FCB_DIS, SYS_DisableFpgaReset).
    /// Clear, as after power-on, a reset resets the FCB too, which then
    /// configures the fabric from flash again (docs/10-updates.md, "DFU
    /// mode", has the evidence).
    pub const RST_FCB_DIS: u32 = 1 << 2;

    /// MISC_CNTL's value for the fastest HSI: AGM's DFU writes it before
    /// loading its fabric (examples/dfu/src/dfu.c, SYS_SetHSIConfig(0xf)),
    /// as its OpenOCD script does before fast JTAG ("Set HSI to fastest",
    /// platforms/AgRV/etc/agrv2k.cfg).
    pub const HSI_CONFIG_FASTEST: u32 = 0xF;

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
    pub const APB_FCB0: u32 = 1 << 0;
    pub const APB_GPIO6: u32 = 1 << 10;
    pub const APB_GPIO7: u32 = 1 << 11;
    pub const APB_UART0: u32 = 1 << 21;

    /// AHB_CLKENABLE: AltaRiscv.h AHB_MASK_USB0. peripherals.h
    /// PERIPHERAL_USB_ENABLE sets this and only this.
    pub const AHB_USB0: u32 = 1 << 1;
}

/// The flash controller: flash.h FLASH_TypeDef at FLASH (0x4000_1000).
/// An STM32F1-style FPEC in front of the in-package SPI NOR (flash.c reads
/// its JEDEC ID and SFDP through FLASH_FlexRead).
pub mod flash {
    use super::Reg;

    const BASE: usize = 0x4000_1000;
    // SAFETY: FLASH_TypeDef registers, 32-bit.
    pub const KEYR: Reg = unsafe { Reg::new(BASE + 0x04) };
    /// OPTKEYR (0x08): KEY1 then KEY2 sets CR's OPTWRE (flash.c
    /// FLASH_OptionByteUnlock).
    pub const OPTKEYR: Reg = unsafe { Reg::new(BASE + 0x08) };
    pub const SR: Reg = unsafe { Reg::new(BASE + 0x0C) };
    pub const CR: Reg = unsafe { Reg::new(BASE + 0x10) };
    pub const AR: Reg = unsafe { Reg::new(BASE + 0x14) };
    pub const READ_CTRL: Reg = unsafe { Reg::new(BASE + 0x2C) };
    pub const READ_DATA: Reg = unsafe { Reg::new(BASE + 0x30) };

    /// READ_CTRL for flash.c FLASH_FlexRead without an address: the
    /// command, data bytes << 8, dummy bytes << 16 (has_addr << 31 clear).
    pub const fn read_ctrl(cmd: u8, data_bytes: u8, dummy_bytes: u8) -> u32 {
        (cmd as u32) | ((data_bytes as u32) << 8) | ((dummy_bytes as u32) << 16)
    }
    /// Read Unique ID, the command flash.c FLASH_GetUniqueID sends.
    pub const CMD_READ_UNIQUE_ID: u8 = 0x4B;

    /// flash.h FLASH_KEY1, FLASH_KEY2: written to KEYR in turn to unlock.
    pub const KEY1: u32 = 0x4567_0123;
    pub const KEY2: u32 = 0xCDEF_89AB;

    // SR bits: FLASH_SR_*. The flags clear by writing 1 (flash.h
    // FLASH_ClearProgramError, FLASH_ClearEndOfOperation).
    pub const SR_BSY: u32 = 1 << 0;
    pub const SR_PGERR: u32 = 1 << 2;
    pub const SR_WRPRTERR: u32 = 1 << 4;
    pub const SR_EOP: u32 = 1 << 5;

    // CR bits: FLASH_CR_*.
    pub const CR_PG: u32 = 1 << 0;
    pub const CR_SER: u32 = 1 << 1;
    /// FLASH_CR_OPTPG: with PG, a store to the option bytes programs them
    /// (FLASH_EnableOptionByteProgram).
    pub const CR_OPTPG: u32 = 1 << 4;
    /// FLASH_CR_OPTER: STRT erases the option bytes (FLASH_EraseOptionByte).
    pub const CR_OPTER: u32 = 1 << 5;
    /// FLASH_CR_OPTWRE: the option bytes are unlocked.
    pub const CR_OPTWRE: u32 = 1 << 9;
    /// FLASH_CR_ERASE_MASK: SER, MER, BER, OPTER.
    pub const CR_ERASE_MASK: u32 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5);
    pub const CR_STRT: u32 = 1 << 6;
    pub const CR_LOCK: u32 = 1 << 7;
    /// FLASH_CR_READ: a flexible read (FLASH_FlexRead).
    pub const CR_READ: u32 = 1 << 14;
    /// FLASH_DisableProgram clears PG, FASTPG and OPTPG together.
    pub const CR_PROGRAM_MASK: u32 = (1 << 0) | (1 << 4) | (1 << 15);
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

    /// UART0 TX, PIN_20: GPIO7 bit 6. Switch 2, image-select bit 1.
    pub const TX_MASK: u32 = 1 << 6;
    /// UART0 RX, PIN_21: GPIO6 bit 1. Switch 1, image-select bit 0.
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
    /// UART_FR_RXFE: the receive FIFO is empty.
    pub const FR_RXFE: u32 = 1 << 4;
    pub const FR_TXFF: u32 = 1 << 5;
    /// 8 data bits (UART_LCR_DATABITS_8), FIFOs on (UART_LCR_FIFO_16).
    pub const LCR_H_8N1_FIFO: u32 = (3 << 5) | (1 << 4);
    /// The same with even parity (UART_LCR_PARITY_EVEN: PEN | EPS), as the
    /// mask ROM's loader and AGM's booter run UART0: 8E1.
    pub const LCR_H_8E1_FIFO: u32 = LCR_H_8N1_FIFO | (1 << 1) | (1 << 2);
    /// DR's error bits above the data byte: framing, parity, break, overrun.
    pub const DR_ERRORS: u32 = 0xF << 8;
    pub const CR_ENABLE_TX_RX: u32 = (1 << 0) | (1 << 8) | (1 << 9);
    pub const ICR_ALL: u32 = 0x7FF;
}

/// The fabric configuration block: fcb.h FCB_TypeDef at AltaRiscv.h
/// FCB0_BASE, the only FCB (fcb.h `#define FCB FCB0`); its APB clock is
/// APB_MASK_FCB0.
pub mod fcb {
    use super::Reg;

    const BASE: usize = 0x4001_0000;
    // SAFETY: FCB_TypeDef registers, 32-bit; touched only with the FCB's APB
    // clock on (fastprom_hal::fcb).
    pub const CTRL: Reg = unsafe { Reg::new(BASE) }; // offset 0x00
    /// AUTO (0x0c): one bitstream word per write in AUTO mode (fcb.c
    /// FCB_AutoConfig, FCB_AutoDecompressEncrypt).
    pub const AUTO: Reg = unsafe { Reg::new(BASE + 0x0C) };
    pub const STAT: Reg = unsafe { Reg::new(BASE + 0x10) };

    /// CTRL: configure from the words written to AUTO (FCB_CTRL_AUTO);
    /// writing CTRL = 0 ends it.
    pub const CTRL_AUTO: u32 = 1 << 6;
    /// STAT: the fabric is configured and running (FCB_STAT_ACTIVE).
    pub const STAT_ACTIVE: u32 = 1 << 1;
    /// STAT: FCB_STAT_ERR_ID | _ERR_HEADER | _ERR_CRC (FCB_STAT_ERR_ALL).
    pub const STAT_ERR_ALL: u32 = (1 << 4) | (1 << 5) | (1 << 6);
}

/// The fabric's AHB slave window: board.h MMIO_BASE, the CPU's mem_ahb
/// port into the fabric (hdl/fastprom_ahb_slave.v behind it).
pub mod fabric {
    pub const BASE: usize = 0x6000_0000;
}

/// The controller's registers (`usb.h` USB_TypeDef at `AltaRiscv.h`
/// USB0_BASE 0x4100_1000).
pub mod usb {
    use super::Reg;

    const BASE: usize = 0x4100_1000;
    // SAFETY (all): USB_TypeDef registers, 32-bit, at the offsets usb.h
    // gives; the controller's AHB clock is on before any is touched
    // (Ag32Usb::init).
    pub const USBCMD: Reg = unsafe { Reg::new(BASE + 0x140) };
    pub const USBSTS: Reg = unsafe { Reg::new(BASE + 0x144) };
    pub const USBINTR: Reg = unsafe { Reg::new(BASE + 0x148) };
    pub const DEVICEADDR: Reg = unsafe { Reg::new(BASE + 0x154) };
    pub const ENDPOINTLISTADDR: Reg = unsafe { Reg::new(BASE + 0x158) };
    pub const PORTSC: Reg = unsafe { Reg::new(BASE + 0x184) };
    pub const ENDPTNAK: Reg = unsafe { Reg::new(BASE + 0x178) };
    pub const ENDPTNAKEN: Reg = unsafe { Reg::new(BASE + 0x17C) };
    pub const USBMODE: Reg = unsafe { Reg::new(BASE + 0x1A8) };
    pub const ENDPTSETUPSTAT: Reg = unsafe { Reg::new(BASE + 0x1AC) };
    pub const ENDPTPRIME: Reg = unsafe { Reg::new(BASE + 0x1B0) };
    pub const ENDPTFLUSH: Reg = unsafe { Reg::new(BASE + 0x1B4) };
    pub const ENDPTSTATUS: Reg = unsafe { Reg::new(BASE + 0x1B8) };
    pub const ENDPTCOMPLETE: Reg = unsafe { Reg::new(BASE + 0x1BC) };
    // SAFETY: ENDPTCTRL[0..4] at 0x1C0, one per endpoint (USB_ENDPOINTS 4).
    pub const ENDPTCTRL: [Reg; 4] = unsafe {
        [
            Reg::new(BASE + 0x1C0),
            Reg::new(BASE + 0x1C4),
            Reg::new(BASE + 0x1C8),
            Reg::new(BASE + 0x1CC),
        ]
    };

    // USBCMD.
    /// Run/Stop (USB_CMD_RS). In device mode, run connects the D+ pull-up:
    /// AGM's dcd_connect is USB_Run and dcd_disconnect USB_Stop.
    pub const CMD_RS: u32 = 1 << 0;
    /// Controller reset, cleared by the controller when done (USB_CMD_RST,
    /// usb.c USB_Reset).
    pub const CMD_RST: u32 = 1 << 1;
    /// Setup tripwire (USB_CMD_SUTW).
    pub const CMD_SUTW: u32 = 1 << 13;
    /// Interrupt threshold control, bits 23:16 (USB_CMD_ITC).
    pub const CMD_ITC: u32 = 0xFF << 16;

    // USBSTS, write 1 to clear.
    /// A TD with IOC finished, or a SETUP arrived (USB_INT_UI).
    pub const STS_UI: u32 = 1 << 0;
    /// Error (USB_INT_UEI).
    pub const STS_UEI: u32 = 1 << 1;
    /// Port change (USB_INT_PCI).
    pub const STS_PCI: u32 = 1 << 2;
    /// System error (USB_INT_SEI).
    pub const STS_SEI: u32 = 1 << 4;
    /// USB reset received (USB_INT_URI).
    pub const STS_URI: u32 = 1 << 6;
    /// SOF received (USB_INT_SRI).
    pub const STS_SRI: u32 = 1 << 7;
    /// Suspend (USB_INT_SLI).
    pub const STS_SLI: u32 = 1 << 8;
    /// The event bits this driver clears.
    pub const STS_EVENTS: u32 = STS_UI | STS_UEI | STS_PCI | STS_SEI | STS_URI | STS_SRI | STS_SLI;

    // DEVICEADDR (usb.h USB_SetDeviceAddress: addr << 25 | 1 << 24).
    pub const ADDR_SHIFT: u32 = 25;
    /// USBADRA: apply the address after the next IN (the status stage).
    pub const ADDR_ADVANCE: u32 = 1 << 24;

    /// PORTSC port reset (USB_PORTSC_PR): set while the host resets the
    /// bus (usb.c USB_BusReset ends with USB_PortWaitForReset on it).
    pub const PORTSC_PR: u32 = 1 << 8;

    // USBMODE.
    /// Controller mode, device (USB_MODE_CM, USB_MODE_DEIVCE 0x2).
    pub const MODE_DEVICE: u32 = 0x2;
    /// Setup lockout off (USB_MODE_SLOM): the setup buffer is read with
    /// the tripwire instead.
    pub const MODE_SLOM: u32 = 1 << 3;

    // ENDPTCTRL, per half at shift 0 (RX, OUT) or 16 (TX, IN): usb.h
    // USB_ENDPTCTRL_STALL / _TYPE / _DTR / _EN.
    pub const CTRL_STALL: u32 = 1 << 0;
    pub const CTRL_TYPE_SHIFT: u32 = 2;
    pub const CTRL_TYPE: u32 = 3 << CTRL_TYPE_SHIFT;
    pub const CTRL_TOGGLE_RESET: u32 = 1 << 6;
    pub const CTRL_ENABLE: u32 = 1 << 7;
    /// The bulk type (USB_ENDPT_TYPE_BULK, 2 << 2).
    pub const CTRL_TYPE_BULK: u32 = 2 << CTRL_TYPE_SHIFT;
}
