//! Borrowed network metadata stored beside the flash catalogue, never in heap.
use crate::{clock::Zone, config::crc32};

pub const SLOT_BYTES: usize = 1024 * 1024;
pub const BUNDLE_MAGIC: &[u8; 8] = b"TTBND001";
const PROFILE_MAGIC: &[u8; 8] = b"TTPRF001";
const FIELDS: usize = 8 + 80 + 64 + 96;

#[derive(Clone, Copy)]
pub struct Profile<'a>(&'a [u8]);
impl<'a> Profile<'a> {
    pub const DEFAULT: Self = Self(&[]);
    pub fn new(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < FIELDS + 2 || bytes.get(..8)? != PROFILE_MAGIC {
            return None;
        }
        let value = Self(bytes);
        let length = u16::from_le_bytes(bytes.get(FIELDS..FIELDS + 2)?.try_into().ok()?) as usize;
        if length < 20 {
            return None;
        }
        bytes.get(FIELDS + 2..FIELDS + 2 + length)?;
        if !gtfs::mobility::valid_id(value.feed_id()?)
            || value.timezone()?.is_empty()
            || value.name()?.is_empty()
            || !value.zone().valid()
            || !value.glyphs_valid()
        {
            return None;
        }
        Some(value)
    }
    fn field(&self, start: usize, length: usize) -> Option<&'a str> {
        let b = self.0.get(start..start + length)?;
        let end = b.iter().position(|b| *b == 0).unwrap_or(b.len());
        if b[end..].iter().any(|b| *b != 0) {
            return None;
        }
        let text = core::str::from_utf8(&b[..end]).ok()?;
        (!text.chars().any(char::is_control)).then_some(text)
    }
    pub fn feed_id(&self) -> Option<&'a str> {
        if self.0.is_empty() {
            Some("mdb-982")
        } else {
            self.field(8, 80)
        }
    }
    pub fn timezone(&self) -> Option<&'a str> {
        if self.0.is_empty() {
            Some("Europe/Warsaw")
        } else {
            self.field(88, 64)
        }
    }
    pub fn name(&self) -> Option<&'a str> {
        if self.0.is_empty() {
            Some("ZTM Poznan")
        } else {
            self.field(152, 96)
        }
    }
    pub fn zone(&self) -> Zone<'a> {
        if self.0.is_empty() {
            Zone::WARSAW
        } else {
            Zone::from_bytes(&self.0[FIELDS + 2..self.zone_end()])
        }
    }
    fn zone_end(&self) -> usize {
        FIELDS + 2 + usize::from(u16::from_le_bytes([self.0[FIELDS], self.0[FIELDS + 1]]))
    }
    fn glyph_bytes(&self) -> &'a [u8] {
        if self.0.is_empty() {
            &[]
        } else {
            &self.0[self.zone_end()..]
        }
    }
    fn glyphs_valid(&self) -> bool {
        let bytes = self.glyph_bytes();
        if bytes.is_empty() {
            return true;
        }
        if bytes.get(..8) != Some(b"TTGLY001") || bytes.len() < 10 {
            return false;
        }
        let count = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        if count > 4096 || bytes.len() != 10 + count * 24 {
            return false;
        }
        let mut previous = 0;
        for row in bytes[10..].as_chunks::<24>().0 {
            let code = u32::from_le_bytes(row[..4].try_into().unwrap());
            if code <= previous
                || char::from_u32(code).is_none_or(char::is_control)
                || row[4..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|r| u16::from_le_bytes(*r) > 1023)
            {
                return false;
            }
            previous = code;
        }
        true
    }
    pub fn has_glyphs(&self) -> bool {
        !self.glyph_bytes().is_empty()
    }
    /// One 10×10 glyph, as ten little-endian ten-bit rows.
    pub fn glyph(&self, c: char) -> Option<&'a [u8]> {
        let bytes = self.glyph_bytes();
        if bytes.is_empty() {
            return None;
        }
        let rows = bytes[10..].as_chunks::<24>().0;
        let index = rows
            .binary_search_by_key(&(c as u32), |r| {
                u32::from_le_bytes(r[..4].try_into().unwrap())
            })
            .ok()?;
        Some(&rows[index][4..])
    }
    /// Bind the feed and timezone rules; glyph changes do not invalidate data.
    /// An empty default profile keeps old Poznań caches compatible.
    pub fn identity(&self) -> &'a [u8] {
        if self.0.is_empty() {
            &[]
        } else {
            &self.0[..self.zone_end()]
        }
    }
}

/// Checked envelope; the unused end of the fixed ROM slot is ignored.
pub fn bundle(bytes: &[u8]) -> Option<(&[u8], Profile<'_>)> {
    if bytes.get(..8)? != BUNDLE_MAGIC {
        return None;
    }
    let length = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
    if length > SLOT_BYTES - 16 {
        return None;
    }
    let payload = bytes.get(16..16usize.checked_add(length)?)?;
    if crc32(payload) != u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) {
        return None;
    }
    let catalog_len = u32::from_le_bytes(payload.get(..4)?.try_into().ok()?) as usize;
    let end = 4usize.checked_add(catalog_len)?;
    let catalog = payload.get(4..end)?;
    let metadata = payload.get(end..)?;
    let profile = if metadata.is_empty() {
        Profile::DEFAULT
    } else {
        Profile::new(metadata)?
    };
    Some((catalog, profile))
}

/// Borrow just metadata after the immutable envelope was validated at boot.
pub fn metadata(bytes: &[u8]) -> Option<Profile<'_>> {
    if bytes.get(..8)? != BUNDLE_MAGIC {
        return None;
    }
    let length = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
    if length > SLOT_BYTES - 16 {
        return None;
    }
    let payload = bytes.get(16..16usize.checked_add(length)?)?;
    let catalog_len = u32::from_le_bytes(payload.get(..4)?.try_into().ok()?) as usize;
    let metadata = payload.get(4usize.checked_add(catalog_len)?..)?;
    if metadata.is_empty() {
        Some(Profile::DEFAULT)
    } else {
        Profile::new(metadata)
    }
}
