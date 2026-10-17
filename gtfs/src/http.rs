//! Transport-neutral HTTP byte-range helpers. Require identity encoding and
//! validate 206 + Content-Range before feeding a response body to a ZIP decoder.

use crate::Error;
use embedded_io_async::{ErrorType, Read, Seek, SeekFrom};

/// Inclusive HTTP byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> Result<Self, Error> {
        if end < start || end == u64::MAX {
            return Err(Error::InvalidRange);
        }
        Ok(Self { start, end })
    }
    pub fn start(self) -> u64 {
        self.start
    }
    pub fn end(self) -> u64 {
        self.end
    }
    pub fn len(self) -> u64 {
        self.end - self.start + 1
    }
    pub fn is_empty(self) -> bool {
        false
    }

    pub fn header(self, out: &mut [u8]) -> Result<&str, Error> {
        struct Writer<'a> {
            buf: &'a mut [u8],
            len: usize,
        }
        impl core::fmt::Write for Writer<'_> {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                if s.len() > self.buf.len() - self.len {
                    return Err(core::fmt::Error);
                }
                self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
                self.len += s.len();
                Ok(())
            }
        }
        use core::fmt::Write;
        let mut w = Writer { buf: out, len: 0 };
        write!(w, "bytes={}-{}", self.start, self.end).map_err(|_| Error::BufferTooSmall)?;
        let len = w.len;
        Ok(core::str::from_utf8(&out[..len]).expect("ASCII range"))
    }

    /// Return the total object length, requiring an exact requested range.
    pub fn validate_content_range(self, value: &[u8]) -> Result<u64, Error> {
        let value = core::str::from_utf8(value).map_err(|_| Error::InvalidRange)?;
        let (span, total) = value
            .strip_prefix("bytes ")
            .and_then(|s| s.split_once('/'))
            .ok_or(Error::InvalidRange)?;
        let (start, end) = span.split_once('-').ok_or(Error::InvalidRange)?;
        fn decimal(s: &str) -> Result<u64, Error> {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::InvalidRange);
            }
            s.parse().map_err(|_| Error::InvalidRange)
        }
        let total = decimal(total)?;
        if decimal(start)? != self.start || decimal(end)? != self.end || self.end >= total {
            return Err(Error::InvalidRange);
        }
        Ok(total)
    }
}

/// Fill `out` from exactly one validated range. Implementations must reject
/// ignored ranges (200), changed objects, encoded bodies and truncated responses.
/// Use immutable snapshot URLs, or enforce an ETag with If-Match.
#[allow(async_fn_in_trait)]
pub trait RangeFetch {
    async fn fetch(&mut self, range: ByteRange, out: &mut [u8]) -> Result<(), Error>;
}

/// A small seekable cache for reading a remote ZIP's central directory. Stream
/// large member payloads with one separate range request instead of using this
/// adapter for table data (which would cause many small HTTP requests).
pub struct RangeReader<'a, F> {
    fetcher: F,
    length: u64,
    pos: u64,
    cache: &'a mut [u8],
    base: u64,
    valid: usize,
}

impl<'a, F> RangeReader<'a, F> {
    pub fn new(fetcher: F, length: u64, cache: &'a mut [u8]) -> Result<Self, Error> {
        if cache.is_empty() {
            return Err(Error::BufferTooSmall);
        }
        Ok(Self {
            fetcher,
            length,
            pos: 0,
            cache,
            base: 0,
            valid: 0,
        })
    }
    pub fn into_inner(self) -> F {
        self.fetcher
    }
}

impl<F> ErrorType for RangeReader<'_, F> {
    type Error = Error;
}

impl<F: RangeFetch> Read for RangeReader<'_, F> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if out.is_empty() || self.pos >= self.length {
            return Ok(0);
        }
        if self.pos < self.base || self.pos >= self.base + self.valid as u64 {
            self.base = self.pos;
            self.valid = 0;
            let n = (self.length - self.pos).min(self.cache.len() as u64) as usize;
            self.fetcher
                .fetch(
                    ByteRange::new(self.pos, self.pos + n as u64 - 1)?,
                    &mut self.cache[..n],
                )
                .await?;
            self.valid = n;
        }
        let start = (self.pos - self.base) as usize;
        let n = out.len().min(self.valid - start);
        out[..n].copy_from_slice(&self.cache[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<F> Seek for RangeReader<'_, F> {
    async fn seek(&mut self, from: SeekFrom) -> Result<u64, Error> {
        let pos = match from {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(n) => self.length.checked_add_signed(n),
            SeekFrom::Current(n) => self.pos.checked_add_signed(n),
        }
        .ok_or(Error::InvalidRange)?;
        self.pos = pos;
        Ok(pos)
    }
}
