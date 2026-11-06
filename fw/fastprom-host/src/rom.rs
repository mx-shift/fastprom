//! ROM files, read the way `tools/mkimagetable.py` reads them and checked
//! against the part's organisation.
//!
//! Two encodings, by the file's extension:
//!
//! - **text** (`.hex`, `.txt`, `.mem`): `tools/mkimage.py`'s output, hex
//!   words separated by white space, `//` starting a comment. Intel HEX is
//!   not this, and is refused by name.
//! - **binary** (anything else, `.bin` above all): one word per byte.
//!
//! A file shorter than the part is padded with blank words (all ones, as an
//! unprogrammed bipolar PROM cell reads), as `mkimagetable.py` pads it; a
//! longer one, or one with a word wider than the part, is refused.
//!
//! The tool reads files by the same rule (`tests/table.rs` checks the
//! tables it wrote from `.hex`, `.bin`, `.BIN` and `.rom` files against
//! these); the one difference left is that `int(token, 16)`'s extras
//! (signs, underscores) are not taken here.

use fastprom_core::crc32::crc32;
use fastprom_core::layout::ENTRY_NAME_LEN;

/// A part's organisation: words, and bits per word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub depth: u32,
    pub width: u8,
}

impl Geometry {
    /// The bits a word may use.
    pub const fn mask(self) -> u8 {
        if self.width >= 8 {
            0xFF
        } else {
            (1u8 << self.width) - 1
        }
    }

    /// An organisation the image table holds.
    ///
    /// # Errors
    ///
    /// For a depth that is not a power of two from 16 to 65536, or a width
    /// that is not 4 or 8.
    pub fn new(depth: u32, width: u8) -> Result<Self, String> {
        if !depth.is_power_of_two() || !(16..=65536).contains(&depth) {
            return Err(format!(
                "a depth of {depth} words: the image table holds a power of two from 16 to 65536"
            ));
        }
        if width != 4 && width != 8 {
            return Err(format!("{width}-bit words: the image table holds 4 or 8"));
        }
        Ok(Self { depth, width })
    }
}

impl core::fmt::Display for Geometry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}x{}", self.depth, self.width)
    }
}

/// A ROM image ready for the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rom {
    /// The name the table records: the file's name as `mkimagetable.py`
    /// stores it ([`stored_name`]).
    pub name: String,
    /// Exactly `depth` words, one per byte.
    pub words: Vec<u8>,
    /// Words the file held; the rest are blank padding.
    pub given: usize,
    /// CRC-32 of `words`, as the table records it.
    pub crc: u32,
}

impl Rom {
    /// Blank words padded on, if any.
    pub fn padded(&self) -> usize {
        self.words.len() - self.given
    }
}

/// The name a table entry records for `file_name`: its last path component,
/// each character that is not printable ASCII as `?`, cut to 39 bytes.
/// `mkimagetable.py`'s `ascii_field` with `errors="replace"`, except for
/// ASCII control characters, which the firmware's parser refuses
/// (`fastprom-core`'s `table.rs`): the tool refuses a name with one, and a
/// browser's file name with one becomes `?` here.
pub fn stored_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .take(ENTRY_NAME_LEN - 1)
        .collect()
}

fn is_text(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    [".hex", ".txt", ".mem"].iter().any(|e| lower.ends_with(e))
}

fn parse_text(bytes: &[u8]) -> Result<Vec<u32>, String> {
    if bytes.first() == Some(&b':') {
        return Err("this is Intel HEX, which the page does not read: \
                    give a binary image (.bin), one byte per word"
            .into());
    }
    let text = core::str::from_utf8(bytes).map_err(|_| "not text: give a .bin for binary")?;
    let mut words = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("");
        for tok in line.split_whitespace() {
            let digits = tok
                .strip_prefix("0x")
                .or_else(|| tok.strip_prefix("0X"))
                .unwrap_or(tok);
            let w = u32::from_str_radix(digits, 16)
                .ok()
                .filter(|_| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or_else(|| format!("line {}: {tok:?} is not a hex word", n + 1))?;
            words.push(w);
        }
    }
    Ok(words)
}

/// Read `bytes`, the contents of a file called `file_name`, as an image for
/// a part of organisation `g`.
///
/// # Errors
///
/// A message saying what is wrong with the file.
pub fn parse(file_name: &str, bytes: &[u8], g: Geometry) -> Result<Rom, String> {
    let words: Vec<u32> = if is_text(file_name) {
        parse_text(bytes)?
    } else {
        bytes.iter().map(|&b| u32::from(b)).collect()
    };
    if words.is_empty() {
        return Err("the file is empty".into());
    }
    let mask = u32::from(g.mask());
    if let Some((at, w)) = words.iter().enumerate().find(|(_, &w)| w > mask) {
        return Err(format!(
            "word {at} is {w:#x}, wider than the part's {} bits",
            g.width
        ));
    }
    if words.len() > g.depth as usize {
        return Err(format!(
            "{} words: more than the {} the part holds ({g})",
            words.len(),
            g.depth
        ));
    }
    let given = words.len();
    let mut out: Vec<u8> = words.into_iter().map(|w| w as u8).collect();
    out.resize(g.depth as usize, mask as u8);
    let crc = crc32(&out);
    Ok(Rom {
        name: stored_name(file_name),
        words: out,
        given,
        crc,
    })
}

/// A blank image: every word all ones, an unprogrammed PROM.
pub fn blank(g: Geometry) -> Rom {
    let words = vec![g.mask(); g.depth as usize];
    let crc = crc32(&words);
    Rom {
        name: "(blank)".into(),
        words,
        given: 0,
        crc,
    }
}
