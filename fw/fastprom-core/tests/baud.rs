//! UART0 divisors against AGM's UART_Init formula (uart.c).

use fastprom_core::baud::Divisors;

/// AGM's arithmetic, transcribed from uart.c in 32-bit unsigned C.
fn sdk(pclk: u32, baud: u32) -> (u32, u32) {
    let mut ibrd = pclk / baud / 16;
    let mut fbrd = pclk - ibrd * baud * 16;
    fbrd = (fbrd * 4 + baud / 2) / baud;
    if fbrd >= 64 {
        ibrd += 1;
        fbrd = 0;
    }
    (ibrd, fbrd)
}

#[test]
fn matches_the_sdk_formula() {
    for pclk in [10_000_000, 50_000_000, 100_000_000, 12_000_000] {
        for baud in [9_600, 57_600, 115_200, 460_800, 921_600] {
            if pclk / baud / 16 == 0 {
                // Below the PL011's range; the SDK would program IBRD = 0.
                assert_eq!(Divisors::new(pclk, baud), None, "pclk {pclk} baud {baud}");
                continue;
            }
            let d = Divisors::new(pclk, baud).expect("divisible");
            assert_eq!(
                (u32::from(d.ibrd()), u32::from(d.fbrd())),
                sdk(pclk, baud),
                "pclk {pclk} baud {baud}"
            );
        }
    }
}

#[test]
fn boot_report_rate_is_accurate() {
    // 115200 from 100 MHz: 54 + 16/64.
    let d = Divisors::new(100_000_000, 115_200).expect("divisible");
    assert_eq!((d.ibrd(), d.fbrd()), (54, 16));
    let err = (i64::from(d.actual_baud(100_000_000)) - 115_200).abs();
    assert!(err * 1000 < 115_200, "{err} baud off");
}

#[test]
fn impossible_rates_are_refused() {
    assert_eq!(Divisors::new(100_000_000, 0), None);
    assert_eq!(Divisors::new(1_000, 115_200), None); // ibrd would be 0
    assert_eq!(Divisors::new(100_000_000, 10), None); // ibrd > 65535
}
