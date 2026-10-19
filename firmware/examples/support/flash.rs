//! Host-only file model of the dedicated NOR partition. No allocations in I/O.
use core::cell::{Cell, RefCell};
use gtfs::Error;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use timetable_firmware::cache::{self, Storage};
pub struct Flash {
    file: RefCell<File>,
    pub erases: Cell<u32>,
    pub writes: Cell<u32>,
    pub programmed: Cell<u64>,
    pub read: Cell<u64>,
}
fn error(_: std::io::Error) -> Error {
    Error::Io(embedded_io_async::ErrorKind::Other)
}
impl Flash {
    pub fn open(path: &Path) -> Self {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let len = f.metadata().unwrap().len();
        if len == 0 {
            for _ in 0..cache::SIZE / 4096 {
                f.write_all(&[0xff; 4096]).unwrap();
            }
            f.sync_all().unwrap();
        } else {
            assert_eq!(len, u64::from(cache::SIZE), "cache file partition size");
        }
        Self {
            file: RefCell::new(f),
            erases: Cell::new(0),
            writes: Cell::new(0),
            programmed: Cell::new(0),
            read: Cell::new(0),
        }
    }
    fn offset(offset: u32, n: usize) -> Result<u64, Error> {
        let at = offset
            .checked_sub(cache::OFFSET)
            .ok_or(Error::InvalidRange)?;
        if at.checked_add(n as u32).is_none_or(|end| end > cache::SIZE) {
            return Err(Error::InvalidRange);
        }
        Ok(u64::from(at))
    }
}
impl Storage for Flash {
    fn scratch(&self) -> Result<impl AsMut<[u8]>, Error> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(cache::FILTER_BYTES)
            .map_err(|_| Error::BufferTooSmall)?;
        bytes.resize(cache::FILTER_BYTES, 0);
        Ok(bytes)
    }
    fn read(&self, offset: u32, out: &mut [u8]) -> Result<(), Error> {
        let at = Self::offset(offset, out.len())?;
        let mut f = self.file.borrow_mut();
        f.seek(SeekFrom::Start(at)).map_err(error)?;
        f.read_exact(out).map_err(error)?;
        self.read.set(self.read.get() + out.len() as u64);
        Ok(())
    }
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), Error> {
        assert_eq!(offset % 4, 0);
        assert_eq!(bytes.len() % 4, 0);
        let at = Self::offset(offset, bytes.len())?;
        let mut f = self.file.borrow_mut();
        let mut pos = 0;
        for chunk in bytes.chunks(256) {
            let mut old = [0; 256];
            f.seek(SeekFrom::Start(at + pos)).map_err(error)?;
            f.read_exact(&mut old[..chunk.len()]).map_err(error)?;
            for (a, b) in old[..chunk.len()].iter().zip(chunk) {
                if a & b != *b {
                    return Err(Error::InvalidField("NOR programming requires erase"));
                }
            }
            f.seek(SeekFrom::Start(at + pos)).map_err(error)?;
            f.write_all(chunk).map_err(error)?;
            pos += chunk.len() as u64;
        }
        if (offset - cache::OFFSET) % cache::BANK_SIZE == 44 {
            f.sync_all().map_err(error)?;
        }
        self.writes.set(self.writes.get() + 1);
        self.programmed
            .set(self.programmed.get() + bytes.len() as u64);
        Ok(())
    }
    fn erase(&self, start: u32, end: u32) -> Result<(), Error> {
        if !start.is_multiple_of(4096) || end - start != 4096 {
            return Err(Error::InvalidRange);
        }
        let at = Self::offset(start, 4096)?;
        let mut f = self.file.borrow_mut();
        f.seek(SeekFrom::Start(at)).map_err(error)?;
        f.write_all(&[0xff; 4096]).map_err(error)?;
        self.erases.set(self.erases.get() + 1);
        Ok(())
    }
}
