//! Official Poznań feed discovery. Enumerating archives never downloads them.

use crate::{Date, Error};
use core::str::FromStr;
use embedded_io_async::{Error as _, Read};

pub const LIST_URL: &str = "https://www.ztm.poznan.pl/otwarte-dane/gtfsfiles/";
pub const LATEST_URL: &str = "https://www.ztm.poznan.pl/pl/dla-deweloperow/getGTFSFile";
/// A third-party mirror of the official feed, with immutable snapshot links.
pub const MIRROR_PAGE: &str = "https://mobilitydatabase.org/feeds/gtfs/mdb-982";

const MIRROR_PREFIX: &str = "https://files.mobilitydatabase.org/mdb-982/";
const MIRROR_LEN: usize = MIRROR_PREFIX.len() + 45;

/// Immutable MobilityDatabase snapshot URL. The timestamp is the mirror's
/// fetch time, not the service validity interval; inspect the feed calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirrorSnapshot([u8; MIRROR_LEN]);

impl MirrorSnapshot {
    pub fn url(&self) -> &str {
        core::str::from_utf8(&self.0).expect("validated ASCII snapshot URL")
    }
}

impl FromStr for MirrorSnapshot {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        let tail = s
            .strip_prefix(MIRROR_PREFIX)
            .ok_or(Error::InvalidArchiveName)?
            .as_bytes();
        if tail.len() != 45
            || &tail[..8] != b"mdb-982-"
            || tail[20] != b'/'
            || &tail[21..29] != b"mdb-982-"
            || &tail[41..] != b".zip"
            || tail[8..20] != tail[29..41]
            || !tail[8..20].iter().all(u8::is_ascii_digit)
        {
            return Err(Error::InvalidArchiveName);
        }
        let stamp = core::str::from_utf8(&tail[8..20]).map_err(|_| Error::InvalidArchiveName)?;
        stamp[..8]
            .parse::<Date>()
            .map_err(|_| Error::InvalidArchiveName)?;
        if stamp[8..10]
            .parse::<u8>()
            .map_err(|_| Error::InvalidArchiveName)?
            >= 24
            || stamp[10..]
                .parse::<u8>()
                .map_err(|_| Error::InvalidArchiveName)?
                >= 60
        {
            return Err(Error::InvalidArchiveName);
        }
        let mut bytes = [0; MIRROR_LEN];
        bytes.copy_from_slice(s.as_bytes());
        Ok(Self(bytes))
    }
}

/// Discover pinned mirror URLs from `MIRROR_PAGE` without loading HTML into
/// RAM. Consecutive duplicate links are suppressed. Page layout is not an API.
pub struct MirrorSnapshots<R> {
    source: R,
    input: [u8; 1024],
    pos: usize,
    len: usize,
    window: [u8; MIRROR_LEN],
    filled: usize,
    last: Option<MirrorSnapshot>,
}

impl<R: Read> MirrorSnapshots<R> {
    pub fn new(source: R) -> Self {
        Self {
            source,
            input: [0; 1024],
            pos: 0,
            len: 0,
            window: [0; MIRROR_LEN],
            filled: 0,
            last: None,
        }
    }
    pub async fn next(&mut self) -> Result<Option<MirrorSnapshot>, Error> {
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
            if self.filled < MIRROR_LEN {
                self.window[self.filled] = byte;
                self.filled += 1;
            } else {
                self.window.copy_within(1.., 0);
                self.window[MIRROR_LEN - 1] = byte;
            }
            if self.filled == MIRROR_LEN
                && byte == b'p'
                && let Ok(s) = core::str::from_utf8(&self.window)
                && let Ok(snapshot) = s.parse::<MirrorSnapshot>()
                && self.last != Some(snapshot)
            {
                self.last = Some(snapshot);
                return Ok(Some(snapshot));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveName([u8; 21]);

impl ArchiveName {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).expect("validated archive name")
    }
    pub fn start(&self) -> Date {
        self.as_str()[..8].parse().expect("validated date")
    }
    pub fn end(&self) -> Date {
        self.as_str()[9..17].parse().expect("validated date")
    }
    pub fn covers(&self, date: Date) -> bool {
        self.start() <= date && date <= self.end()
    }

    /// Write a pinned, official download URL into a caller-provided buffer.
    pub fn url<'a>(&self, buf: &'a mut [u8]) -> Result<&'a str, Error> {
        let prefix = LATEST_URL.as_bytes();
        let len = prefix.len() + 6 + 21;
        if buf.len() < len {
            return Err(Error::BufferTooSmall);
        }
        buf[..prefix.len()].copy_from_slice(prefix);
        buf[prefix.len()..prefix.len() + 6].copy_from_slice(b"?file=");
        buf[prefix.len() + 6..len].copy_from_slice(&self.0);
        Ok(core::str::from_utf8(&buf[..len]).expect("ASCII URL"))
    }
}

impl FromStr for ArchiveName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        let b = s.as_bytes();
        if b.len() != 21
            || b[8] != b'_'
            || &b[17..] != b".zip"
            || !b[..8].iter().chain(b[9..17].iter()).all(u8::is_ascii_digit)
        {
            return Err(Error::InvalidArchiveName);
        }
        let start: Date = s[..8].parse().map_err(|_| Error::InvalidArchiveName)?;
        let end: Date = s[9..17].parse().map_err(|_| Error::InvalidArchiveName)?;
        if start > end {
            return Err(Error::InvalidArchiveName);
        }
        let mut name = [0; 21];
        name.copy_from_slice(b);
        Ok(Self(name))
    }
}

/// Scans the official listing for dated archive names across arbitrary chunk
/// boundaries. Consecutive repeats (label + link) are suppressed. The caller
/// decides which snapshots to download and may need global deduplication if the
/// publisher changes its page layout.
pub struct Archives<R> {
    source: R,
    input: [u8; 1024],
    pos: usize,
    len: usize,
    window: [u8; 21],
    filled: usize,
    last: Option<ArchiveName>,
}

impl<R: Read> Archives<R> {
    pub fn new(source: R) -> Self {
        Self {
            source,
            input: [0; 1024],
            pos: 0,
            len: 0,
            window: [0; 21],
            filled: 0,
            last: None,
        }
    }
    pub async fn next(&mut self) -> Result<Option<ArchiveName>, Error> {
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
            if self.filled < 21 {
                self.window[self.filled] = byte;
                self.filled += 1;
            } else {
                self.window.copy_within(1.., 0);
                self.window[20] = byte;
            }
            if self.filled == 21
                && byte == b'p'
                && let Ok(s) = core::str::from_utf8(&self.window)
                && let Ok(name) = s.parse::<ArchiveName>()
                && self.last != Some(name)
            {
                self.last = Some(name);
                return Ok(Some(name));
            }
        }
    }
}
