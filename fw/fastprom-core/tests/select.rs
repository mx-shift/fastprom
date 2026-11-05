//! The image-select switch decode.

use fastprom_core::select::{decide, SAMPLES};
use fastprom_core::{Bank, SelectError, SwitchSample};

#[test]
fn bit_assignment_matches_the_board() {
    // docs/06-build-and-flash.md: switch 1 on RX is bit 0, switch 2 on TX
    // is bit 1, closed (high) is 1.
    let cases = [
        (false, false, 0),
        (false, true, 1),
        (true, false, 2),
        (true, true, 3),
    ];
    for (tx, rx, bank) in cases {
        assert_eq!(
            SwitchSample { tx, rx }.bank().number(),
            bank,
            "tx {tx} rx {rx}"
        );
    }
}

#[test]
fn steady_samples_decide() {
    for n in 0..4 {
        let bank = Bank::new(n).expect("bank");
        let s = SwitchSample {
            rx: n & 1 != 0,
            tx: n & 2 != 0,
        };
        assert_eq!(decide(&[s; SAMPLES]), Ok(bank));
        assert_eq!(decide(&[s]), Ok(bank));
    }
}

#[test]
fn any_disagreeing_sample_refuses() {
    let open = SwitchSample::default();
    for at in 1..SAMPLES {
        let mut samples = [open; SAMPLES];
        samples[at].rx = true;
        assert_eq!(
            decide(&samples),
            Err(SelectError::Unstable {
                first: Bank::new(0).expect("bank"),
                index: at,
                found: Bank::new(1).expect("bank"),
            })
        );
    }
}

#[test]
fn no_samples_refuses() {
    assert_eq!(decide(&[]), Err(SelectError::NoSamples));
}

#[test]
fn bank_range() {
    assert!((0..4).all(|n| Bank::new(n).is_some()));
    assert!((4..=255).all(|n| Bank::new(n).is_none()));
    assert_eq!(Bank::from_bits(true, false).number(), 2);
    assert_eq!(Bank::new(3).expect("bank").index(), 3);
}
