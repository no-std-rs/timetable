//! Allocation-free discovery for any public MobilityDatabase timetable mirror.
use crate::{Date, Error};
use embedded_io_async::{Error as _, Read};

pub const MAX_ID: usize = 80;
pub const URL_BYTES: usize = 320;
const PREFIX: &str = "https://files.mobilitydatabase.org/";

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Compact token: do not retain eight repeated, potentially 300-byte URLs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Snapshot {
    stamp: [u8; 12],
    id: [u8; MAX_ID],
    len: u8,
}
impl core::str::FromStr for Snapshot {
    type Err = Error;
    fn from_str(url: &str) -> Result<Self, Error> {
        let bad = Error::InvalidArchiveName;
        let mut path = url.strip_prefix(PREFIX).ok_or(bad)?.split('/');
        let id = path.next().ok_or(bad)?;
        let folder = path.next().ok_or(bad)?;
        let file = path.next().ok_or(bad)?;
        if path.next().is_some() || !valid_id(id) || file.strip_suffix(".zip") != Some(folder) {
            return Err(bad);
        }
        let stamp = folder
            .strip_prefix(id)
            .and_then(|s| s.strip_prefix('-'))
            .ok_or(bad)?;
        if stamp.len() != 12 || !stamp.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad);
        }
        stamp[..8].parse::<Date>().map_err(|_| bad)?;
        if stamp[8..10].parse::<u8>().map_err(|_| bad)? >= 24
            || stamp[10..].parse::<u8>().map_err(|_| bad)? >= 60
        {
            return Err(bad);
        }
        let mut value = Self {
            stamp: [0; 12],
            id: [0; MAX_ID],
            len: id.len() as u8,
        };
        value.stamp.copy_from_slice(stamp.as_bytes());
        value.id[..id.len()].copy_from_slice(id.as_bytes());
        Ok(value)
    }
}
impl Snapshot {
    pub fn feed_id(&self) -> &str {
        core::str::from_utf8(&self.id[..usize::from(self.len)]).unwrap()
    }
    pub fn url<'a>(&self, out: &'a mut [u8; URL_BYTES]) -> &'a str {
        let id = self.feed_id().as_bytes();
        let mut pos = 0;
        for bytes in [
            PREFIX.as_bytes(),
            id,
            b"/",
            id,
            b"-",
            &self.stamp,
            b"/",
            id,
            b"-",
            &self.stamp,
            b".zip",
        ] {
            out[pos..pos + bytes.len()].copy_from_slice(bytes);
            pos += bytes.len();
        }
        core::str::from_utf8(&out[..pos]).unwrap()
    }
}

/// Parse pinned links across arbitrary read boundaries; callers select the feed.
pub struct Snapshots<R> {
    source: R,
    input: [u8; 1024],
    pos: usize,
    len: usize,
    url: [u8; URL_BYTES],
    used: usize,
    prefix: usize,
}
impl<R: Read> Snapshots<R> {
    pub fn new(source: R) -> Self {
        Self {
            source,
            input: [0; 1024],
            pos: 0,
            len: 0,
            url: [0; URL_BYTES],
            used: 0,
            prefix: 0,
        }
    }
    pub async fn next(&mut self) -> Result<Option<Snapshot>, Error> {
        loop {
            if self.pos == self.len {
                self.len = self
                    .source
                    .read(&mut self.input)
                    .await
                    .map_err(|e| Error::Io(e.kind()))?;
                self.pos = 0;
                if self.len == 0 {
                    return Ok(None);
                }
            }
            let byte = self.input[self.pos];
            self.pos += 1;
            if self.used == 0 {
                if byte == PREFIX.as_bytes()[self.prefix] {
                    self.prefix += 1;
                } else {
                    self.prefix = usize::from(byte == b'h');
                }
                if self.prefix == PREFIX.len() {
                    self.url[..PREFIX.len()].copy_from_slice(PREFIX.as_bytes());
                    self.used = PREFIX.len();
                    self.prefix = 0;
                }
            } else if self.used == URL_BYTES
                || byte <= 32
                || matches!(byte, b'"' | b'\'' | b'<' | b'>' | b'\\')
            {
                self.used = 0;
                self.prefix = 0;
            } else {
                self.url[self.used] = byte;
                self.used += 1;
                if self.url[..self.used].ends_with(b".zip") {
                    let result = core::str::from_utf8(&self.url[..self.used])
                        .ok()
                        .and_then(|s| s.parse().ok());
                    self.used = 0;
                    if result.is_some() {
                        return Ok(result);
                    }
                }
            }
        }
    }
}
