//! Forward-only ZIP extraction and a small seekable index of GTFS members.
//!
//! Stored and raw DEFLATE entries are supported, including DEFLATE data
//! descriptors. ZIP64, encryption and multi-disk archives return explicit errors.
//! Reading an entry through EOF verifies its CRC and size. Skipping an entry
//! avoids decompression when its compressed size is known and does not verify CRC.

use crc32fast::Hasher;
use embedded_io_async::{Error as _, ErrorType, Read, Seek, SeekFrom};
use miniz_oxide::inflate::{
    TINFLStatus,
    core::{DecompressorOxide, decompress, inflate_flags::TINFL_FLAG_HAS_MORE_INPUT},
};

use crate::{Error, Table, read_exact};

const LOCAL: u32 = 0x04034b50;
const CENTRAL: u32 = 0x02014b50;
const END: u32 = 0x06054b50;
const DESCRIPTOR: u32 = 0x08074b50;
const INPUT: usize = 2048;

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Reusable DEFLATE state. The 32 KiB dictionary is required by the ZIP format.
/// Keep this in caller-owned memory and reuse it between entries and queries.
pub struct Workspace {
    dictionary: [u8; 32768],
    inflater: Option<DecompressorOxide>,
}

impl Workspace {
    pub const fn new() -> Self {
        Self {
            dictionary: [0; 32768],
            inflater: None,
        }
    }

    fn reset(&mut self) {
        self.inflater
            .get_or_insert_with(DecompressorOxide::new)
            .init();
    }

    /// Reuse the dictionary as caller-owned scratch while no ZIP reader borrows
    /// this workspace. The next reader resets the inflater before decoding.
    pub fn scratch(&mut self) -> &mut [u8] {
        &mut self.dictionary
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

/// Local member metadata. Sizes are unknown until EOF when a data descriptor is used.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    name: [u8; 128],
    name_len: usize,
    pub compression: u16,
    flags: u16,
    pub compressed_size: Option<u32>,
    pub uncompressed_size: Option<u32>,
    crc: Option<u32>,
    descriptor: bool,
}

impl Entry {
    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len]
    }

    pub fn table(&self) -> Option<Table> {
        Table::from_filename(self.name())
    }

    pub fn fingerprint(&self) -> Option<Fingerprint> {
        Some(Fingerprint {
            crc32: self.crc?,
            uncompressed_size: self.uncompressed_size?,
        })
    }
}

/// Identity of uncompressed table bytes. This also matches a recompressed mirror.
/// CRC32 detects version changes/corruption; it is not a cryptographic identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub crc32: u32,
    pub uncompressed_size: u32,
}

/// Small snapshot descriptor retained alongside selected routes/trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Manifest {
    tables: [Option<Fingerprint>; Table::COUNT],
    updated_at: Option<i64>,
}

impl Manifest {
    pub const EMPTY: Self = Self {
        tables: [None; Table::COUNT],
        updated_at: None,
    };
    /// Reconstruct persisted table identities. The caller must validate its
    /// storage and table bytes before treating this descriptor as trusted.
    pub fn from_fingerprints(
        tables: impl IntoIterator<Item = (Table, Fingerprint)>,
    ) -> Result<Self, Error> {
        let mut manifest = Self::EMPTY;
        for (table, fingerprint) in tables {
            manifest.record(table, fingerprint)?;
        }
        Ok(manifest)
    }
    pub fn from_directory(directory: &Directory) -> Self {
        let mut result = Self::EMPTY;
        for table in Table::ALL {
            result.tables[table as usize] = directory.member(table).map(|m| m.fingerprint());
        }
        result.updated_at = directory.updated_at;
        result
    }
    pub fn get(&self, table: Table) -> Option<Fingerprint> {
        self.tables[table as usize]
    }
    /// Latest UTC modification time among recognized GTFS tables, from central
    /// Info-ZIP UT fields. Unknown if any table lacks valid metadata. Available
    /// after a complete forward scan/verification or validated directory scan.
    /// This advisory metadata is separate from uncompressed snapshot identity.
    pub fn updated_at(&self) -> Option<i64> {
        self.updated_at
    }
    pub(crate) fn set_updated_at(&mut self, updated_at: Option<i64>) {
        self.updated_at = updated_at;
    }
    pub(crate) fn record(&mut self, table: Table, fingerprint: Fingerprint) -> Result<(), Error> {
        if self.tables[table as usize].replace(fingerprint).is_some() {
            return Err(Error::DuplicateTable(table));
        }
        Ok(())
    }
    /// Compare uncompressed table identities, independently of advisory times.
    pub fn check(&self, other: &Self) -> Result<(), Error> {
        for table in Table::ALL {
            if self.get(table) != other.get(table) {
                return Err(Error::SnapshotMismatch(table));
            }
        }
        Ok(())
    }
}

/// Caller-owned metadata for one verified ZIP member, including non-GTFS files.
/// Supply one slot per archive entry; insufficient capacity returns `IndexFull`.
#[derive(Clone, Copy)]
pub struct VerificationSlot {
    entry: Option<Entry>,
    offset: u32,
    seen: bool,
}

impl VerificationSlot {
    pub const EMPTY: Self = Self {
        entry: None,
        offset: 0,
        seen: false,
    };
}

/// Verify every member's decompressed size and CRC, including non-GTFS files,
/// then match all directory names, flags, methods, sizes, CRCs and offsets to the
/// verified local entries, and validate the EOCD and body EOF. Use before
/// publishing a cached ZIP. This does not validate GTFS table contents.
pub async fn verify<R: Read>(
    source: R,
    workspace: &mut Workspace,
    scratch: &mut [u8],
    entries: &mut [VerificationSlot],
) -> Result<Manifest, Error> {
    if scratch.is_empty() {
        return Err(Error::BufferTooSmall);
    }
    let mut archive = Reader::new(source, workspace);
    let mut observed = Manifest::EMPTY;
    let mut count = 0;
    loop {
        let offset = archive.position;
        let Some(entry) = archive.next_entry().await? else {
            break;
        };
        if count == entries.len() {
            return Err(Error::IndexFull);
        }
        while archive.read(scratch).await? != 0 {}
        let verified = archive.current_entry().ok_or(Error::InvalidZip)?;
        entries[count] = VerificationSlot {
            entry: Some(verified),
            offset: u32::try_from(offset).map_err(|_| Error::UnsupportedZip)?,
            seen: false,
        };
        count += 1;
        if let Some(table) = entry.table() {
            let fingerprint = verified.fingerprint().ok_or(Error::InvalidZip)?;
            observed.record(table, fingerprint)?;
        }
    }
    archive
        .finish_archive_checked(&observed, Some(&mut entries[..count]))
        .await?;
    observed.set_updated_at(archive.updated_at);
    Ok(observed)
}

/// Stream a ZIP directly from an `embedded_io_async::Read`, including an HTTP body.
/// `Read` returns decompressed bytes for the current entry only; call `next_entry`
/// to advance. Dropping a reader does not drain or verify the remaining bytes.
pub struct Reader<'a, R> {
    source: R,
    workspace: &'a mut Workspace,
    input: [u8; INPUT],
    start: usize,
    end: usize,
    current: Option<Entry>,
    remaining: Option<u32>,
    compressed: u64,
    uncompressed: u64,
    crc: Hasher,
    pending_start: usize,
    pending_end: usize,
    dictionary_pos: usize,
    done: bool,
    archive_done: bool,
    position: u64,
    entries: u32,
    tail: u32,
    footer_finished: bool,
    next_signature: Option<u32>,
    updated_at: Option<i64>,
}

impl<'a, R: Read> Reader<'a, R> {
    pub fn new(source: R, workspace: &'a mut Workspace) -> Self {
        Self {
            source,
            workspace,
            input: [0; INPUT],
            start: 0,
            end: 0,
            current: None,
            remaining: Some(0),
            compressed: 0,
            uncompressed: 0,
            crc: Hasher::new(),
            pending_start: 0,
            pending_end: 0,
            dictionary_pos: 0,
            done: true,
            archive_done: false,
            position: 0,
            entries: 0,
            tail: 0,
            footer_finished: false,
            next_signature: None,
            updated_at: None,
        }
    }

    pub fn into_inner(self) -> R {
        self.source
    }

    /// Includes sizes/CRC discovered from a data descriptor after reading EOF.
    pub fn current_entry(&self) -> Option<Entry> {
        self.current
    }

    /// Source-provided table timestamp, only available after `finish_archive`.
    pub fn updated_at(&self) -> Option<i64> {
        self.updated_at
    }

    /// Decode only a member's compressed payload, e.g. a single HTTP range body.
    /// The caller must validate the range and use `Member::data_range` first.
    pub fn data(source: R, workspace: &'a mut Workspace, member: Member) -> Self {
        let mut reader = Self::new(source, workspace);
        let mut name = [0; 128];
        let bytes = member.table.filename().as_bytes();
        name[..bytes.len()].copy_from_slice(bytes);
        reader.current = Some(Entry {
            name,
            name_len: bytes.len(),
            compression: member.compression,
            flags: member.flags,
            compressed_size: Some(member.compressed_size),
            uncompressed_size: Some(member.uncompressed_size),
            crc: Some(member.crc),
            descriptor: false,
        });
        reader.remaining = Some(member.compressed_size);
        reader.done = false;
        reader.workspace.reset();
        reader
    }

    /// Open a complete indexed local record from a forward-only range stream.
    /// Use `Member::record_range`; local name, flags, sizes and CRC must match
    /// the validated directory. Extra fields and descriptors are streamed.
    pub async fn record(
        source: R,
        workspace: &'a mut Workspace,
        member: Member,
    ) -> Result<Self, Error> {
        let mut reader = Self::new(source, workspace);
        let entry = reader.next_entry().await?.ok_or(Error::InvalidZip)?;
        if entry.table() != Some(member.table)
            || entry.flags != member.flags
            || entry.compression != member.compression
            || entry
                .compressed_size
                .is_some_and(|n| n != member.compressed_size)
            || entry
                .uncompressed_size
                .is_some_and(|n| n != member.uncompressed_size)
            || entry.crc.is_some_and(|n| n != member.crc)
            || member.offset as u64 + reader.position + member.compressed_size as u64
                > member.record_end as u64
        {
            return Err(Error::InvalidZip);
        }
        let entry = reader.current.as_mut().unwrap();
        entry.crc = Some(member.crc);
        entry.compressed_size = Some(member.compressed_size);
        entry.uncompressed_size = Some(member.uncompressed_size);
        reader.remaining = Some(member.compressed_size);
        Ok(reader)
    }

    async fn fill(&mut self) -> Result<(), Error> {
        if self.start == self.end {
            self.start = 0;
            self.end = self
                .source
                .read(&mut self.input)
                .await
                .map_err(|e| Error::Io(e.kind()))?;
            if self.end == 0 {
                return Err(Error::UnexpectedEof);
            }
        }
        Ok(())
    }

    async fn exact(&mut self, mut out: &mut [u8]) -> Result<(), Error> {
        while !out.is_empty() {
            self.fill().await?;
            let n = out.len().min(self.end - self.start);
            out[..n].copy_from_slice(&self.input[self.start..self.start + n]);
            self.start += n;
            self.position += n as u64;
            out = &mut out[n..];
        }
        Ok(())
    }

    async fn discard(&mut self, mut count: u64) -> Result<(), Error> {
        while count != 0 {
            self.fill().await?;
            let n = count.min((self.end - self.start) as u64) as usize;
            self.start += n;
            self.position += n as u64;
            count -= n as u64;
        }
        Ok(())
    }

    /// Advance, skipping any unread part of the previous entry.
    pub async fn next_entry(&mut self) -> Result<Option<Entry>, Error> {
        self.skip_entry().await?;
        if self.archive_done {
            return Ok(None);
        }
        let mut h = [0; 30];
        if let Some(signature) = self.next_signature.take() {
            h[..4].copy_from_slice(&signature.to_le_bytes());
            self.position += 4;
        } else {
            self.exact(&mut h[..4]).await?;
        }
        match u32_at(&h, 0) {
            CENTRAL | END => {
                self.archive_done = true;
                self.tail = u32_at(&h, 0);
                self.current = None;
                return Ok(None);
            }
            LOCAL => {}
            _ => return Err(Error::InvalidZip),
        }
        self.exact(&mut h[4..]).await?;
        let flags = u16_at(&h, 6);
        check_format(flags, u16_at(&h, 8))?;
        if u16_at(&h, 4) > 20 || u32_at(&h, 18) == u32::MAX || u32_at(&h, 22) == u32::MAX {
            return Err(Error::UnsupportedZip);
        }
        let name_len = u16_at(&h, 26) as usize;
        if name_len > 128 {
            return Err(Error::NameTooLong);
        }
        let descriptor = flags & 8 != 0;
        let mut entry = Entry {
            name: [0; 128],
            name_len,
            compression: u16_at(&h, 8),
            flags,
            compressed_size: (!descriptor).then(|| u32_at(&h, 18)),
            uncompressed_size: (!descriptor).then(|| u32_at(&h, 22)),
            crc: (!descriptor).then(|| u32_at(&h, 14)),
            descriptor,
        };
        self.exact(&mut entry.name[..name_len]).await?;
        self.discard(u16_at(&h, 28) as u64).await?;
        self.current = Some(entry);
        self.entries += 1;
        self.remaining = entry.compressed_size;
        self.compressed = 0;
        self.uncompressed = 0;
        self.crc = Hasher::new();
        self.pending_start = 0;
        self.pending_end = 0;
        self.dictionary_pos = 0;
        self.done = false;
        self.workspace.reset();
        Ok(Some(entry))
    }

    async fn descriptor(&mut self, expected: (u32, u32, u32)) -> Result<(u32, u32, u32), Error> {
        let mut h = [0; 12];
        self.exact(&mut h).await?;
        let unsigned = (u32_at(&h, 0), u32_at(&h, 4), u32_at(&h, 8));
        if unsigned.0 != DESCRIPTOR {
            return Ok(unsigned);
        }
        if unsigned == expected && expected != (DESCRIPTOR, DESCRIPTOR, DESCRIPTOR) {
            return Ok(unsigned);
        }
        let mut last = [0; 4];
        self.exact(&mut last).await?;
        let last = u32_at(&last, 0);
        if unsigned == expected && last != DESCRIPTOR {
            // All three words equal the marker: a fourth marker is a signed
            // descriptor's size; any other word belongs to the next ZIP record.
            self.next_signature = Some(last);
            self.position -= 4;
            return Ok(unsigned);
        }
        Ok((unsigned.1, unsigned.2, last))
    }

    /// Skip without inflating when sizes are known. CRC is checked only for fully read entries.
    pub async fn skip_entry(&mut self) -> Result<(), Error> {
        if self.done {
            self.pending_start = self.pending_end;
            return Ok(());
        }
        if let Some(n) = self.remaining {
            self.discard(n as u64).await?;
            if self.current.is_some_and(|e| e.descriptor) {
                let entry = self.current.ok_or(Error::InvalidZip)?;
                let expected = (
                    entry.crc.ok_or(Error::InvalidZip)?,
                    entry.compressed_size.ok_or(Error::InvalidZip)?,
                    entry.uncompressed_size.ok_or(Error::InvalidZip)?,
                );
                if self.descriptor(expected).await? != expected {
                    return Err(Error::InvalidZip);
                }
            }
            self.remaining = Some(0);
            self.pending_start = self.pending_end;
            self.done = true;
        } else {
            let mut buf = [0; 512];
            while self.read(&mut buf).await? != 0 {}
        }
        Ok(())
    }

    async fn finish(&mut self) -> Result<(), Error> {
        let entry = self.current.ok_or(Error::InvalidZip)?;
        if self.remaining.is_some_and(|n| n != 0) {
            return Err(Error::SizeMismatch);
        }
        let (crc, compressed, uncompressed) = if entry.descriptor {
            self.descriptor((
                self.crc.clone().finalize(),
                u32::try_from(self.compressed).map_err(|_| Error::SizeMismatch)?,
                u32::try_from(self.uncompressed).map_err(|_| Error::SizeMismatch)?,
            ))
            .await?
        } else {
            (
                entry.crc.unwrap(),
                entry.compressed_size.unwrap(),
                entry.uncompressed_size.unwrap(),
            )
        };
        if self.compressed != compressed as u64
            || self.uncompressed != uncompressed as u64
            || entry.compressed_size.is_some_and(|n| n != compressed)
            || entry.uncompressed_size.is_some_and(|n| n != uncompressed)
        {
            return Err(Error::SizeMismatch);
        }
        if self.crc.clone().finalize() != crc || entry.crc.is_some_and(|c| c != crc) {
            return Err(Error::CrcMismatch);
        }
        let entry = self.current.as_mut().unwrap();
        entry.crc = Some(crc);
        entry.compressed_size = Some(compressed);
        entry.uncompressed_size = Some(uncompressed);
        self.done = true;
        Ok(())
    }

    /// Finish a whole forward ZIP: validate its central directory against the
    /// observed local table headers, EOCD offsets/counts, comment and body EOF.
    /// Call after `next_entry()` returned `None`, only on a stream starting at 0.
    pub async fn finish_archive(&mut self, observed: &Manifest) -> Result<(), Error> {
        self.finish_archive_checked(observed, None).await
    }

    // Central UT fields contain only mtime even when access/creation flag bits
    // refer to the local header. Unsupported or malformed optional metadata is
    // unknown, without rejecting otherwise supported ZIP payloads. No allocation
    // proportional to extra-field length (which may be up to 65535 bytes).
    async fn extended_timestamp(&mut self, mut remaining: u16) -> Result<Option<i64>, Error> {
        let mut modified = None;
        let mut seen = false;
        let mut valid = true;
        while remaining >= 4 {
            let mut h = [0; 4];
            self.exact(&mut h).await?;
            remaining -= 4;
            let length = u16_at(&h, 2);
            if length > remaining {
                self.discard(u64::from(remaining)).await?;
                return Ok(None);
            }
            if u16_at(&h, 0) == 0x5455 {
                valid &= !seen;
                seen = true;
                if length == 5 {
                    let mut time = [0; 5];
                    self.exact(&mut time).await?;
                    if time[0] & 1 != 0 && time[0] & !7 == 0 {
                        modified = Some(i64::from(u32_at(&time, 1) as i32));
                    } else {
                        valid = false;
                    }
                } else {
                    valid = false;
                    self.discard(u64::from(length)).await?;
                }
            } else {
                self.discard(u64::from(length)).await?;
            }
            remaining -= length;
        }
        self.discard(u64::from(remaining)).await?;
        Ok(modified.filter(|_| valid && remaining == 0))
    }

    async fn finish_archive_checked(
        &mut self,
        observed: &Manifest,
        mut verified: Option<&mut [VerificationSlot]>,
    ) -> Result<(), Error> {
        if self.footer_finished {
            return Ok(());
        }
        if !self.archive_done {
            return Err(Error::InvalidZip);
        }
        let directory_start = self.position - 4;
        let mut directory = Manifest::EMPTY;
        let mut newest = None::<i64>;
        let mut complete = true;
        let mut count = 0u32;
        let mut signature = self.tail;
        while signature == CENTRAL {
            let mut h = [0; 46];
            h[..4].copy_from_slice(&CENTRAL.to_le_bytes());
            self.exact(&mut h[4..]).await?;
            if u16_at(&h, 6) > 20
                || u16_at(&h, 34) != 0
                || u32_at(&h, 20) == u32::MAX
                || u32_at(&h, 24) == u32::MAX
            {
                return Err(Error::UnsupportedZip);
            }
            check_format(u16_at(&h, 8), u16_at(&h, 10))?;
            let name_len = u16_at(&h, 28) as usize;
            if name_len > 128 {
                return Err(Error::NameTooLong);
            }
            let mut name = [0; 128];
            self.exact(&mut name[..name_len]).await?;
            if let Some(slots) = verified.as_deref_mut() {
                let slot = slots
                    .iter_mut()
                    .find(|slot| slot.offset == u32_at(&h, 42))
                    .ok_or(Error::InvalidZip)?;
                let entry = slot.entry.ok_or(Error::InvalidZip)?;
                if slot.seen
                    || entry.name() != &name[..name_len]
                    || entry.flags != u16_at(&h, 8)
                    || entry.compression != u16_at(&h, 10)
                    || entry.crc != Some(u32_at(&h, 16))
                    || entry.compressed_size != Some(u32_at(&h, 20))
                    || entry.uncompressed_size != Some(u32_at(&h, 24))
                {
                    return Err(Error::InvalidZip);
                }
                slot.seen = true;
            }
            let table = Table::from_filename(&name[..name_len]);
            if let Some(table) = table {
                directory.record(
                    table,
                    Fingerprint {
                        crc32: u32_at(&h, 16),
                        uncompressed_size: u32_at(&h, 24),
                    },
                )?;
            }
            if u32_at(&h, 42) as u64 >= directory_start {
                return Err(Error::InvalidZip);
            }
            let modified = self.extended_timestamp(u16_at(&h, 30)).await?;
            if table.is_some() {
                complete &= modified.is_some();
                if let Some(stamp) = modified {
                    newest = Some(newest.map_or(stamp, |old| old.max(stamp)));
                }
            }
            self.discard(u64::from(u16_at(&h, 32))).await?;
            count += 1;
            let mut sig = [0; 4];
            self.exact(&mut sig).await?;
            signature = u32_at(&sig, 0);
        }
        if signature != END {
            return Err(Error::InvalidZip);
        }
        let directory_size = self.position - 4 - directory_start;
        let mut h = [0; 22];
        h[..4].copy_from_slice(&END.to_le_bytes());
        self.exact(&mut h[4..]).await?;
        if u16_at(&h, 4) != 0 || u16_at(&h, 6) != 0 || u16_at(&h, 10) == u16::MAX {
            return Err(Error::UnsupportedZip);
        }
        if u16_at(&h, 8) as u32 != count
            || u16_at(&h, 10) as u32 != count
            || count != self.entries
            || u32_at(&h, 12) as u64 != directory_size
            || u32_at(&h, 16) as u64 != directory_start
        {
            return Err(Error::InvalidZip);
        }
        observed.check(&directory).map_err(|_| Error::InvalidZip)?;
        if verified.is_some_and(|slots| slots.iter().any(|slot| !slot.seen)) {
            return Err(Error::InvalidZip);
        }
        self.discard(u16_at(&h, 20) as u64).await?;
        if self.start != self.end {
            return Err(Error::InvalidZip);
        }
        let n = self
            .source
            .read(&mut self.input[..1])
            .await
            .map_err(|e| Error::Io(e.kind()))?;
        if n != 0 {
            return Err(Error::InvalidZip);
        }
        self.footer_finished = true;
        self.updated_at = newest.filter(|_| complete);
        Ok(())
    }
}

impl<'a, R: Read + Seek> Reader<'a, R> {
    /// Open one indexed member, seeking past all unrelated compressed tables.
    pub async fn member(
        mut source: R,
        workspace: &'a mut Workspace,
        member: Member,
    ) -> Result<Self, Error> {
        source
            .seek(SeekFrom::Start(member.offset as u64))
            .await
            .map_err(|e| Error::Io(e.kind()))?;
        let mut reader = Self::new(source, workspace);
        let entry = reader.next_entry().await?.ok_or(Error::InvalidZip)?;
        if entry.table() != Some(member.table)
            || entry.compression != member.compression
            || entry.descriptor != (member.flags & 8 != 0)
            || entry
                .compressed_size
                .is_some_and(|n| n != member.compressed_size)
            || entry
                .uncompressed_size
                .is_some_and(|n| n != member.uncompressed_size)
            || entry.crc.is_some_and(|n| n != member.crc)
        {
            return Err(Error::InvalidZip);
        }
        let data_start = member.offset as u64 + 30 + entry.name_len as u64;
        // The local extra field has already been consumed. Account for buffered bytes
        // to find the actual source position without assuming a fixed extra length.
        let physical = reader
            .source
            .stream_position()
            .await
            .map_err(|e| Error::Io(e.kind()))?;
        let logical = physical - (reader.end - reader.start) as u64;
        if logical < data_start
            || logical + member.compressed_size as u64 > member.directory_offset as u64
        {
            return Err(Error::InvalidZip);
        }
        let entry = reader.current.as_mut().unwrap();
        entry.crc = Some(member.crc);
        entry.compressed_size = Some(member.compressed_size);
        entry.uncompressed_size = Some(member.uncompressed_size);
        reader.remaining = Some(member.compressed_size);
        Ok(reader)
    }
}

impl<R> ErrorType for Reader<'_, R> {
    type Error = Error;
}

impl<R: Read> Read for Reader<'_, R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pending_start < self.pending_end {
                let n = out.len().min(self.pending_end - self.pending_start);
                out[..n].copy_from_slice(
                    &self.workspace.dictionary[self.pending_start..self.pending_start + n],
                );
                self.pending_start += n;
                return Ok(n);
            }
            if self.done {
                return Ok(0);
            }
            let entry = self.current.ok_or(Error::InvalidZip)?;
            if entry.compression == 0 {
                let remaining = self.remaining.ok_or(Error::UnsupportedZip)?;
                if remaining == 0 {
                    self.finish().await?;
                    return Ok(0);
                }
                self.fill().await?;
                let n = out.len().min(self.end - self.start).min(remaining as usize);
                out[..n].copy_from_slice(&self.input[self.start..self.start + n]);
                self.start += n;
                self.position += n as u64;
                self.remaining = Some(remaining - n as u32);
                self.compressed += n as u64;
                self.uncompressed += n as u64;
                self.crc.update(&out[..n]);
                return Ok(n);
            }
            if self.remaining != Some(0) {
                self.fill().await?;
            }
            let available = self.remaining.map_or(self.end - self.start, |n| {
                (n as usize).min(self.end - self.start)
            });
            let flags = if self.remaining.is_none_or(|n| n as usize > available) {
                TINFL_FLAG_HAS_MORE_INPUT
            } else {
                0
            };
            let pos = self.dictionary_pos;
            let (status, consumed, produced) = decompress(
                self.workspace.inflater.as_mut().unwrap(),
                &self.input[self.start..self.start + available],
                &mut self.workspace.dictionary,
                pos,
                flags,
            );
            self.start += consumed;
            self.position += consumed as u64;
            if let Some(n) = self.remaining.as_mut() {
                *n -= consumed as u32;
            }
            self.compressed += consumed as u64;
            self.uncompressed += produced as u64;
            if entry
                .uncompressed_size
                .is_some_and(|n| self.uncompressed > n as u64)
            {
                return Err(Error::SizeMismatch);
            }
            self.crc
                .update(&self.workspace.dictionary[pos..pos + produced]);
            self.pending_start = pos;
            self.pending_end = pos + produced;
            self.dictionary_pos = (pos + produced) & 32767;
            match status {
                TINFLStatus::Done => self.finish().await?,
                TINFLStatus::NeedsMoreInput | TINFLStatus::HasMoreOutput
                    if consumed != 0 || produced != 0 => {}
                _ => return Err(Error::InvalidDeflate),
            }
        }
    }
}

fn check_format(flags: u16, compression: u16) -> Result<(), Error> {
    if flags & 0x2041 != 0 {
        return Err(Error::EncryptedZip);
    }
    // Compression options, descriptor and UTF-8 filenames are the supported flags.
    if flags & !0x080e != 0 {
        return Err(Error::UnsupportedZip);
    }
    if compression != 0 && compression != 8 {
        return Err(Error::UnsupportedCompression(compression));
    }
    Ok(())
}

/// Trusted only after `Directory::read` has validated the central directory.
#[derive(Debug, Clone, Copy)]
pub struct Member {
    table: Table,
    offset: u32,
    directory_offset: u32,
    record_end: u32,
    compression: u16,
    flags: u16,
    compressed_size: u32,
    uncompressed_size: u32,
    crc: u32,
}

impl Member {
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint {
            crc32: self.crc,
            uncompressed_size: self.uncompressed_size,
        }
    }
    pub fn compressed_size(&self) -> u32 {
        self.compressed_size
    }
    pub fn uncompressed_size(&self) -> u32 {
        self.uncompressed_size
    }
    pub fn table(&self) -> Table {
        self.table
    }
    /// Fetch these 30 local-header bytes before calculating the payload range.
    pub fn header_range(&self) -> crate::http::ByteRange {
        crate::http::ByteRange::new(self.offset as u64, self.offset as u64 + 29)
            .expect("u32 ZIP offset")
    }
    /// Local header, name, extras, payload and descriptor up to the next physical
    /// record. One HTTP range replaces separate header and payload requests.
    /// The ambiguous all-marker descriptor also includes four lookahead bytes
    /// from the next record to distinguish its signed and unsigned forms.
    pub fn record_range(&self) -> crate::http::ByteRange {
        let lookahead = if self.flags & 8 != 0
            && (self.crc, self.compressed_size, self.uncompressed_size)
                == (DESCRIPTOR, DESCRIPTOR, DESCRIPTOR)
        {
            4
        } else {
            0
        };
        crate::http::ByteRange::new(self.offset as u64, self.record_end as u64 - 1 + lookahead)
            .expect("validated local record bounds")
    }
    pub fn data_range(&self, header: &[u8; 30]) -> Result<crate::http::ByteRange, Error> {
        if u32_at(header, 0) != LOCAL
            || u16_at(header, 4) > 20
            || u16_at(header, 6) != self.flags
            || u16_at(header, 8) != self.compression
            || u16_at(header, 26) as usize != self.table.filename().len()
        {
            return Err(Error::InvalidZip);
        }
        if self.flags & 8 == 0
            && (u32_at(header, 14) != self.crc
                || u32_at(header, 18) != self.compressed_size
                || u32_at(header, 22) != self.uncompressed_size)
        {
            return Err(Error::InvalidZip);
        }
        let start = self.offset as u64 + 30 + u16_at(header, 26) as u64 + u16_at(header, 28) as u64;
        let end = start + self.compressed_size as u64;
        if end > self.directory_offset as u64 || self.compressed_size == 0 {
            return Err(Error::InvalidZip);
        }
        crate::http::ByteRange::new(start, end - 1)
    }
}

/// Stores only the seven tables needed for route, stop and calendar queries.
#[derive(Debug)]
pub struct Directory {
    members: [Option<Member>; Table::COUNT],
    updated_at: Option<i64>,
}

impl Directory {
    pub fn member(&self, table: Table) -> Option<Member> {
        self.members[table as usize]
    }

    /// Read the ZIP directory with caller-owned scratch (at least 22 bytes).
    /// A 1 KiB buffer efficiently handles even the maximum ZIP comment length.
    pub async fn read<R: Read + Seek>(source: &mut R, scratch: &mut [u8]) -> Result<Self, Error> {
        if scratch.len() < 22 {
            return Err(Error::BufferTooSmall);
        }
        let length = source
            .seek(SeekFrom::End(0))
            .await
            .map_err(|e| Error::Io(e.kind()))?;
        if length < 22 {
            return Err(Error::InvalidZip);
        }
        if length > u32::MAX as u64 {
            return Err(Error::UnsupportedZip);
        }
        let lower = length.saturating_sub(65557);
        let mut end = length;
        let mut candidate_error = Error::InvalidZip;
        loop {
            let start = lower.max(end.saturating_sub(scratch.len() as u64));
            let n = (end - start) as usize;
            source
                .seek(SeekFrom::Start(start))
                .await
                .map_err(|e| Error::Io(e.kind()))?;
            read_exact(source, &mut scratch[..n]).await?;
            for i in (0..=n - 22).rev() {
                let b = &scratch[i..];
                if u32_at(b, 0) != END || start + i as u64 + 22 + u16_at(b, 20) as u64 != length {
                    continue;
                }
                match Self::read_candidate(source, b, start + i as u64).await {
                    Ok(directory) => return Ok(directory),
                    Err(Error::Io(kind)) => return Err(Error::Io(kind)),
                    Err(error) => candidate_error = error,
                }
            }
            if start == lower {
                return Err(candidate_error);
            }
            end = start + 21;
        }
    }

    async fn read_candidate<R: Read + Seek>(
        source: &mut R,
        b: &[u8],
        end_offset: u64,
    ) -> Result<Self, Error> {
        if u16_at(b, 4) != 0
            || u16_at(b, 6) != 0
            || u16_at(b, 8) != u16_at(b, 10)
            || u16_at(b, 10) == u16::MAX
            || u32_at(b, 12) == u32::MAX
            || u32_at(b, 16) == u32::MAX
        {
            return Err(Error::UnsupportedZip);
        }
        let offset = u32_at(b, 16);
        let size = u32_at(b, 12);
        let count = u16_at(b, 10);
        if offset as u64 + size as u64 != end_offset {
            return Err(Error::InvalidZip);
        }
        let mut result = Self {
            members: [None; Table::COUNT],
            updated_at: None,
        };
        let mut missing_timestamp = false;
        let mut pos = offset as u64;
        for _ in 0..count {
            if pos + 46 > end_offset {
                return Err(Error::InvalidZip);
            }
            source
                .seek(SeekFrom::Start(pos))
                .await
                .map_err(|e| Error::Io(e.kind()))?;
            let mut h = [0; 46];
            read_exact(source, &mut h).await?;
            if u32_at(&h, 0) != CENTRAL {
                return Err(Error::InvalidZip);
            }
            if u16_at(&h, 34) != 0 {
                return Err(Error::UnsupportedZip);
            }
            let name_len = u16_at(&h, 28) as usize;
            pos += 46 + name_len as u64 + u16_at(&h, 30) as u64 + u16_at(&h, 32) as u64;
            if pos > end_offset {
                return Err(Error::InvalidZip);
            }
            if name_len > 128 {
                continue;
            }
            let mut name = [0; 128];
            read_exact(source, &mut name[..name_len]).await?;
            if let Some(table) = Table::from_filename(&name[..name_len]) {
                check_format(u16_at(&h, 8), u16_at(&h, 10))?;
                if u16_at(&h, 6) > 20
                    || u32_at(&h, 20) == u32::MAX
                    || u32_at(&h, 24) == u32::MAX
                    || u32_at(&h, 42) == u32::MAX
                {
                    return Err(Error::UnsupportedZip);
                }
                let member = Member {
                    table,
                    offset: u32_at(&h, 42),
                    directory_offset: offset,
                    record_end: offset,
                    flags: u16_at(&h, 8),
                    compression: u16_at(&h, 10),
                    compressed_size: u32_at(&h, 20),
                    uncompressed_size: u32_at(&h, 24),
                    crc: u32_at(&h, 16),
                };
                if member.offset as u64 + 30 + name_len as u64 + member.compressed_size as u64
                    > offset as u64
                {
                    return Err(Error::InvalidZip);
                }
                if result.members[table as usize].replace(member).is_some() {
                    return Err(Error::DuplicateTable(table));
                }
                let modified = directory_timestamp(source, u16_at(&h, 30)).await?;
                missing_timestamp |= modified.is_none();
                if let Some(modified) = modified {
                    result.updated_at =
                        Some(result.updated_at.map_or(modified, |old| old.max(modified)));
                }
            }
        }
        if pos != end_offset {
            return Err(Error::InvalidZip);
        }
        // Directory order need not match physical order. A second bounded scan
        // finds the next header, including unknown tables, without retaining
        // every member or assuming equal central/local extra-field lengths.
        // Small directories remain in RangeReader's existing cache.
        let mut pos = offset as u64;
        for _ in 0..count {
            source
                .seek(SeekFrom::Start(pos))
                .await
                .map_err(|e| Error::Io(e.kind()))?;
            let mut h = [0; 46];
            read_exact(source, &mut h).await?;
            let next = u32_at(&h, 42);
            for member in result.members.iter_mut().flatten() {
                if next > member.offset && next < member.record_end {
                    member.record_end = next;
                }
            }
            pos += 46 + u16_at(&h, 28) as u64 + u16_at(&h, 30) as u64 + u16_at(&h, 32) as u64;
        }
        for member in result.members.iter().flatten() {
            if member.offset as u64
                + 30
                + member.table.filename().len() as u64
                + member.compressed_size as u64
                > member.record_end as u64
            {
                return Err(Error::InvalidZip);
            }
        }
        if missing_timestamp {
            result.updated_at = None;
        }
        Ok(result)
    }
}

// Central Info-ZIP UT mtime has the same advisory semantics as forward scans.
// Seeking over other extras avoids allocating space for a 65535-byte field.
async fn directory_timestamp<R: Read + Seek>(
    source: &mut R,
    mut remaining: u16,
) -> Result<Option<i64>, Error> {
    let mut modified = None;
    let mut seen = false;
    let mut valid = true;
    while remaining >= 4 {
        let mut header = [0; 4];
        read_exact(source, &mut header).await?;
        remaining -= 4;
        let length = u16_at(&header, 2);
        if length > remaining {
            return Ok(None);
        }
        if u16_at(&header, 0) == 0x5455 {
            valid &= !seen;
            seen = true;
            if length == 5 {
                let mut time = [0; 5];
                read_exact(source, &mut time).await?;
                if time[0] & 1 != 0 && time[0] & !7 == 0 {
                    modified = Some(i64::from(u32_at(&time, 1) as i32));
                } else {
                    valid = false;
                }
            } else {
                valid = false;
                source
                    .seek(SeekFrom::Current(i64::from(length)))
                    .await
                    .map_err(|e| Error::Io(e.kind()))?;
            }
        } else {
            source
                .seek(SeekFrom::Current(i64::from(length)))
                .await
                .map_err(|e| Error::Io(e.kind()))?;
        }
        remaining -= length;
    }
    Ok(modified.filter(|_| valid && remaining == 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bytes<'a>(&'a [u8]);
    impl ErrorType for Bytes<'_> {
        type Error = Error;
    }
    impl Read for Bytes<'_> {
        async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error> {
            if out.is_empty() || self.0.is_empty() {
                return Ok(0);
            }
            out[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }

    #[test]
    fn record_range_includes_only_required_descriptor_lookahead() {
        let mut member = Member {
            table: Table::StopTimes,
            offset: 100,
            directory_offset: u32::MAX,
            record_end: u32::MAX - 46,
            compression: 0,
            flags: 8,
            compressed_size: DESCRIPTOR,
            uncompressed_size: DESCRIPTOR,
            crc: DESCRIPTOR,
        };
        assert_eq!(member.record_range().start(), 100);
        assert_eq!(
            member.record_range().end(),
            u64::from(member.record_end) + 3
        );
        member.crc = 0;
        assert_eq!(
            member.record_range().end(),
            u64::from(member.record_end) - 1
        );
        member.crc = DESCRIPTOR;
        member.flags = 0;
        assert_eq!(
            member.record_range().end(),
            u64::from(member.record_end) - 1
        );
    }

    #[test]
    fn equal_marker_crc_and_sizes_preserve_the_following_record() {
        // Exercise the descriptor framing directly, without constructing a
        // 0x08074b50-byte payload to give the size fields this boundary value.
        futures::executor::block_on(async {
            for signed in [false, true] {
                let size = if signed { 16 } else { 12 };
                let mut bytes = [0u8; 38];
                for word in bytes[..size].as_chunks_mut::<4>().0 {
                    word.copy_from_slice(&DESCRIPTOR.to_le_bytes());
                }
                bytes[size..size + 4].copy_from_slice(&END.to_le_bytes());
                bytes[size + 16..size + 20].copy_from_slice(&(size as u32).to_le_bytes());
                let mut workspace = Workspace::new();
                let mut reader = Reader::new(Bytes(&bytes[..size + 22]), &mut workspace);
                let expected = (DESCRIPTOR, DESCRIPTOR, DESCRIPTOR);
                assert_eq!(reader.descriptor(expected).await.unwrap(), expected);
                assert!(reader.next_entry().await.unwrap().is_none());
                reader.finish_archive(&Manifest::EMPTY).await.unwrap();
            }
        });
    }
}
