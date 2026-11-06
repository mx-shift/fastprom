//! The image table, built as `tools/mkimagetable.py --out <file>.bin`
//! builds it (`fastprom_core::layout` has the format), and the page's plan
//! of what each switch position will hold.
//!
//! `tests/table.rs` builds every table `gen_fixtures.py` had the tool write,
//! from the same source files, and requires the bytes to be identical.

use fastprom_core::crc32::crc32;
use fastprom_core::header::Part;
use fastprom_core::layout::{
    ENTRY_LEN, HEADER_LEN, MAGIC, MAX_IMAGES, OFF_DIR, OFF_HEADER_CRC, PART_LEN, VERSION,
};
use fastprom_core::{Bank, ImageTable};

use crate::rom::{self, Geometry, Rom};

/// `mkimagetable.py`'s `ascii_field`: each character that is not ASCII as
/// `?`, cut to `len - 1` bytes, NUL padded to `len`.
fn ascii_field(s: &str, len: usize) -> Vec<u8> {
    let mut v: Vec<u8> = s
        .chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
        .take(len - 1)
        .collect();
    v.resize(len, 0);
    v
}

/// The binary table for `images` (name as stored, exactly `depth` words),
/// in bank order. `part` is a `tools/partdb.yaml` key, which the page has
/// from `parts.json` with its organisation `g`; the field records the key,
/// as `mkimagetable.py --part` does.
///
/// # Errors
///
/// For a part name that cannot be a key, an organisation or count the table
/// cannot hold, an image of the wrong length, or a result the firmware's
/// parser would refuse (a name with control characters).
pub fn build(part: &str, g: Geometry, images: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    if Part::new(part).is_none() {
        return Err(format!(
            "part {part:?}: a tools/partdb.yaml key is 1 to {} printable characters",
            fastprom_core::header::PART_LEN
        ));
    }
    let g = Geometry::new(g.depth, g.width)?;
    if !matches!(images.len(), 1 | 2 | 4) {
        return Err(format!(
            "{} images: the switch has two bits, so a table holds 1, 2 or 4",
            images.len()
        ));
    }
    let mut out = Vec::with_capacity(HEADER_LEN + images.len() * g.depth as usize);
    for w in [
        MAGIC,
        VERSION,
        g.depth,
        u32::from(g.width),
        images.len() as u32,
    ] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out.extend(ascii_field(part, PART_LEN));
    for (name, words) in images {
        if words.len() != g.depth as usize {
            return Err(format!(
                "{name}: {} words, not the table's {}",
                words.len(),
                g.depth
            ));
        }
        out.extend(ascii_field(name, ENTRY_LEN - 8));
        out.extend_from_slice(&crc32(words).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    debug_assert!(out.len() <= OFF_DIR + MAX_IMAGES * ENTRY_LEN);
    out.resize(OFF_HEADER_CRC, 0);
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    for (_, words) in images {
        out.extend_from_slice(words);
    }
    // What the firmware will make of it.
    ImageTable::parse(&out).map_err(|e| format!("the firmware would refuse this table: {e}"))?;
    Ok(out)
}

/// One image of a table already in flash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub name: String,
    pub crc: u32,
    pub words: Vec<u8>,
}

/// The images of a table read from the device, and its organisation.
///
/// # Errors
///
/// The firmware parser's reason, for a table it would not serve.
pub fn current(table: &[u8]) -> Result<(Geometry, String, Vec<Current>), String> {
    let t = ImageTable::parse(table).map_err(|e| e.to_string())?;
    let g = Geometry {
        depth: t.depth(),
        width: t.width().bits(),
    };
    let images = (0..t.count().get() as u8)
        .filter_map(Bank::new)
        .filter_map(|b| t.image(b))
        .map(|i| Current {
            name: i.name().to_owned(),
            crc: i.crc32(),
            words: i.words().to_vec(),
        })
        .collect();
    Ok((g, t.part().to_owned(), images))
}

/// Image `index` of the device's table, to keep in a new table of
/// organisation `g`.
///
/// # Errors
///
/// For a table of another organisation, no such image, or an image whose
/// words do not match its CRC (kept, it would be stored with a fresh CRC
/// over damaged words).
pub fn kept(table: &[u8], index: usize, g: Geometry) -> Result<Rom, String> {
    let (tg, _, images) = current(table)?;
    if tg != g {
        return Err(format!("the board's table is {tg}, the part {g}"));
    }
    let i = images
        .into_iter()
        .nth(index)
        .ok_or_else(|| format!("the board's table has no image {index}"))?;
    if crc32(&i.words) != i.crc {
        return Err(format!(
            "the board's image {index} is damaged: give a file for it"
        ));
    }
    Ok(Rom {
        name: i.name,
        given: i.words.len(),
        crc: i.crc,
        words: i.words,
    })
}

/// What one switch position is to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// Nothing: the position is past the end of the table, or a blank
    /// image if a later position holds one.
    Empty,
    /// This image.
    Rom(Rom),
}

/// The page's plan: a slot per switch position, for one part.
#[derive(Debug, Clone)]
pub struct Plan {
    pub part: String,
    pub geometry: Geometry,
    pub slots: [Slot; MAX_IMAGES],
}

impl Plan {
    pub fn new(part: &str, geometry: Geometry) -> Self {
        Self {
            part: part.to_owned(),
            geometry,
            slots: [Slot::Empty, Slot::Empty, Slot::Empty, Slot::Empty],
        }
    }

    /// How many images the table will hold: the fewest of 1, 2 or 4 that
    /// reach the last position given an image.
    pub fn count(&self) -> usize {
        match self.slots.iter().rposition(|s| *s != Slot::Empty) {
            None => 0,
            Some(0) => 1,
            Some(1) => 2,
            Some(_) => 4,
        }
    }

    /// The images in bank order, an empty position below the last given
    /// one as a blank image.
    pub fn images(&self) -> Vec<Rom> {
        self.slots[..self.count()]
            .iter()
            .map(|s| match s {
                Slot::Rom(r) => r.clone(),
                Slot::Empty => rom::blank(self.geometry),
            })
            .collect()
    }

    /// The table.
    ///
    /// # Errors
    ///
    /// For no image at all, or as [`build`].
    pub fn build(&self) -> Result<Vec<u8>, String> {
        let images = self.images();
        if images.is_empty() {
            return Err("no image for any switch position".into());
        }
        let refs: Vec<(&str, &[u8])> = images
            .iter()
            .map(|r| (r.name.as_str(), r.words.as_slice()))
            .collect();
        build(&self.part, self.geometry, &refs)
    }
}
