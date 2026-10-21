#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod csv;
pub mod gtfs;
pub mod http;
pub mod memory;
pub mod mobility;
pub mod poznan;
pub mod query;
pub mod stream;
pub mod zip;

use embedded_io_async::{Error as _, ErrorKind, Read};

pub use gtfs::{Calendar, CalendarDate, Date, Route, Stop, StopTime, Table, Time, Trip};
pub use query::{Feed, Id, IdSet, TripIndex, TripSlot};

/// Failures are explicit: fixed buffers and indexes never silently truncate data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io(ErrorKind),
    UnexpectedEof,
    InvalidZip,
    UnsupportedZip,
    UnsupportedCompression(u16),
    EncryptedZip,
    NameTooLong,
    CrcMismatch,
    SizeMismatch,
    InvalidDeflate,
    RecordTooLong,
    TooManyFields,
    InvalidUtf8,
    MissingColumn(&'static str),
    InvalidField(&'static str),
    WrongTable,
    MissingTable(Table),
    DuplicateTable(Table),
    IdTooLong,
    IndexFull,
    IndexNotSorted,
    DuplicateTrip,
    InvalidDate,
    InvalidTime,
    InvalidArchiveName,
    BufferTooSmall,
    InvalidRange,
    RangeNotSupported,
    SnapshotMismatch(Table),
    RamBudgetExceeded,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for Error {}

impl embedded_io_async::Error for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io(kind) => *kind,
            Self::UnexpectedEof => ErrorKind::InvalidData,
            _ => ErrorKind::InvalidData,
        }
    }
}

pub(crate) async fn read_exact<R: Read>(reader: &mut R, mut buf: &mut [u8]) -> Result<(), Error> {
    while !buf.is_empty() {
        let n = reader.read(buf).await.map_err(|e| Error::Io(e.kind()))?;
        if n == 0 {
            return Err(Error::UnexpectedEof);
        }
        buf = &mut buf[n..];
    }
    Ok(())
}
