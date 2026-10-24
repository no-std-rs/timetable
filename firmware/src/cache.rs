//! Selected, all-period GTFS in two power-loss-safe flash banks.
//! No timestamps or moving departure windows are periodically written.
use crate::cooperative::Cooperative;
use embedded_io_async::{ErrorType, Read};
use gtfs::{
    Date, Error, Table,
    csv::{Column as C, Record},
    zip,
};
use timetable_display::{
    Config,
    config::Pick,
    feed::{Builder, Tables},
    freshness::Freshness,
};

pub const OFFSET: u32 = 0x300000;
pub const SIZE: u32 = 0xf0000;
pub const BANK_SIZE: u32 = SIZE / 2;
pub const FILTER_BYTES: usize = 16 * 1024;
const SECTOR: u32 = 4096;
const SOURCE_BYTES: usize = 384;
const SOURCE_V2_BYTES: usize = 352;
const TABLE_END: usize = 20 + Table::COUNT * 20;
const HEADER: usize = 48 + SOURCES * SOURCE_BYTES;
const SOURCES: usize = 3;
const COMMIT: u32 = 0x434f4d54;
const EMPTY: i64 = i64::MIN;

/// Calls are synchronous and bounded. Implementations must serialize access to
/// the real flash, allow byte reads, and enforce the dedicated partition bounds.
#[allow(async_fn_in_trait)]
pub trait Storage {
    /// Stable installed feed/timezone identity, borrowed from ROM.
    fn identity(&self) -> &[u8] {
        &[]
    }
    fn read(&self, offset: u32, out: &mut [u8]) -> Result<(), Error>;
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), Error>;
    fn erase(&self, start: u32, end: u32) -> Result<(), Error>;
    /// Bounded source metadata, acquired only for comparing/writing a bank.
    /// Device adapters may place it in their existing reserved heap after TLS
    /// verification; the cache library itself never allocates.
    fn metadata(&self) -> Result<impl AsMut<[Metadata]>, Error> {
        Ok([Metadata::default()])
    }
    /// Temporary caller-owned filter. The device acquires this inside its
    /// verified session, after certificate-verification allocations are freed.
    /// Returning an error disables cache writes and uses the ordinary query.
    fn scratch(&self) -> Result<impl AsMut<[u8]>, Error> {
        Err::<[u8; 0], _>(Error::BufferTooSmall)
    }
    async fn current(&self) -> bool {
        true
    }
    /// Successful-check time sampled immediately before the commit. Offline
    /// fixtures may keep their explicit query epoch by leaving this unknown.
    fn checked_at(&self) -> Option<i64> {
        None
    }
    /// Consume approval for replacing this recently committed bank. Called
    /// after read-only validation/comparison and before any erase or program.
    /// The caller can handle `CacheUpdateRequired` outside the HTTP session,
    /// then retry with approval bound to the previous commit timestamp.
    fn approve_update(&self, _previous: i64) -> bool {
        false
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Block {
    offset: u32,
    length: u32,
    crc: u32,
}
#[derive(Clone, Copy, Default)]
struct Source {
    start: u32,
    end: u32,
    present: u32,
    updated: i64,
    original: [[u32; 2]; Table::COUNT],
    tables: [Block; Table::COUNT],
    snapshot: Option<gtfs::mobility::Snapshot>,
    validator: crate::download::Validator,
}
/// Caller-owned scratch for three source identities and one 256-byte write buffer.
pub struct Metadata {
    sources: [Source; SOURCES],
    buffer: [u8; 256],
}
impl Default for Metadata {
    fn default() -> Self {
        Self {
            sources: [Source::default(); SOURCES],
            buffer: [0; 256],
        }
    }
}
impl Source {
    fn mask(&self, dates: [Date; 3]) -> u8 {
        dates.iter().enumerate().fold(0, |m, (i, d)| {
            m | if self.start <= date(*d) && date(*d) <= self.end {
                1 << i
            } else {
                0
            }
        })
    }
    fn matches(&self, manifest: &zip::Manifest, start: Date, end: Date) -> bool {
        self.start == date(start)
            && self.end == date(end)
            && Table::ALL.into_iter().all(|t| {
                manifest.get(t).map(|f| [f.crc32, f.uncompressed_size])
                    == (self.present & (1 << t as usize) != 0).then_some(self.original[t as usize])
            })
    }
    fn manifest(&self) -> Result<zip::Manifest, Error> {
        zip::Manifest::from_fingerprints(Table::ALL.into_iter().filter_map(|t| {
            let b = self.tables[t as usize];
            (b.length != 0).then_some((
                t,
                zip::Fingerprint {
                    crc32: b.crc,
                    uncompressed_size: b.length,
                },
            ))
        }))
    }
    fn encode(&self, out: &mut [u8]) {
        if let Some(snapshot) = self.snapshot {
            out[TABLE_END..TABLE_END + 12].copy_from_slice(snapshot.timestamp().as_bytes());
            let id = snapshot.feed_id().as_bytes();
            out[TABLE_END + 12] = id.len() as u8;
            out[TABLE_END + 13..TABLE_END + 13 + id.len()].copy_from_slice(id);
        }
        self.validator
            .encode(&mut out[TABLE_END + 93..TABLE_END + 159]);
        put(out, 0, self.start);
        put(out, 4, self.end);
        put(out, 8, self.present);
        out[12..20].copy_from_slice(&self.updated.to_le_bytes());
        for t in Table::ALL {
            let i = t as usize;
            put(out, 20 + i * 8, self.original[i][0]);
            put(out, 24 + i * 8, self.original[i][1]);
            let b = self.tables[i];
            put(out, 20 + Table::COUNT * 8 + i * 12, b.offset);
            put(out, 24 + Table::COUNT * 8 + i * 12, b.length);
            put(out, 28 + Table::COUNT * 8 + i * 12, b.crc);
        }
    }
    fn decode(bytes: &[u8]) -> Self {
        let tables = if bytes.len() >= SOURCE_BYTES {
            Table::COUNT
        } else {
            7
        };
        let table_end = 20 + tables * 20;
        let mut s = Self {
            start: get(bytes, 0),
            end: get(bytes, 4),
            present: get(bytes, 8),
            updated: i64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            ..Self::default()
        };
        for t in Table::ALL.into_iter().take(tables) {
            let i = t as usize;
            s.original[i] = [get(bytes, 20 + i * 8), get(bytes, 24 + i * 8)];
            s.tables[i] = Block {
                offset: get(bytes, 20 + tables * 8 + i * 12),
                length: get(bytes, 24 + tables * 8 + i * 12),
                crc: get(bytes, 28 + tables * 8 + i * 12),
            };
        }
        if bytes.len() >= SOURCE_V2_BYTES {
            let len = bytes[table_end + 12] as usize;
            if len <= gtfs::mobility::MAX_ID {
                s.snapshot = core::str::from_utf8(&bytes[table_end + 13..table_end + 13 + len])
                    .ok()
                    .zip(core::str::from_utf8(&bytes[table_end..table_end + 12]).ok())
                    .and_then(|(id, stamp)| gtfs::mobility::Snapshot::from_parts(id, stamp).ok());
                if s.snapshot.is_some() {
                    s.validator =
                        crate::download::Validator::decode(&bytes[table_end + 93..table_end + 159])
                            .for_ranges();
                }
            }
        }
        s
    }
}
fn source_size(version: u32) -> usize {
    match version {
        1 => 160,
        2 => SOURCE_V2_BYTES,
        _ => SOURCE_BYTES,
    }
}
fn date(d: Date) -> u32 {
    u32::from(d.year()) * 10000 + u32::from(d.month()) * 100 + u32::from(d.day())
}
fn valid_date(d: u32) -> bool {
    d <= 99991231 && Date::new((d / 10000) as u16, (d / 100 % 100) as u8, (d % 100) as u8).is_ok()
}
fn get(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn put(b: &mut [u8], i: usize, n: u32) {
    b[i..i + 4].copy_from_slice(&n.to_le_bytes());
}
#[derive(Clone, Copy, Default)]
pub struct Bank {
    offset: u32,
    generation: u32,
    length: u32,
    key_length: u32,
    count: u32,
    checked: i64,
    version: u32,
}
impl Bank {
    /// Query reference for a clearly labelled saved-board preview. This is not
    /// a current clock and must never be installed as the device's UTC.
    pub fn checked_at(self) -> i64 {
        self.checked
    }
    fn source(&self, flash: &impl Storage, i: usize) -> Result<Source, Error> {
        if i >= self.count as usize {
            return Err(Error::InvalidZip);
        }
        let mut b = [0; SOURCE_BYTES];
        let size = source_size(self.version);
        flash.read(self.offset + 48 + (i * size) as u32, &mut b[..size])?;
        Ok(Source::decode(&b[..size]))
    }
    fn fresh(&self, flash: &impl Storage, builder: &Builder, now: i64) -> Result<Freshness, Error> {
        let mut times = timetable_display::freshness::SourceTimes::default();
        let mut remaining = builder.day_mask();
        for i in 0..self.count as usize {
            let s = self.source(flash, i)?;
            let mask = s.mask(builder.dates) & remaining;
            if mask == 0 {
                continue;
            }
            times.observe(
                (s.updated != EMPTY && (0..=now).contains(&s.updated)).then_some(s.updated),
            );
            remaining &= !mask;
        }
        Ok(Freshness {
            checked_at: (self.checked <= now).then_some(self.checked),
            source_updated_at: times.updated_at(),
        })
    }
}

async fn validate(flash: &impl Storage, offset: u32) -> Result<Bank, Error> {
    let mut b = [0; HEADER];
    flash.read(offset, &mut b)?;
    if &b[..4] != b"TTFC" || !matches!(get(&b, 4), 1..=3) || get(&b, 44) != COMMIT {
        return Err(Error::InvalidZip);
    }
    let version = get(&b, 4);
    let size = source_size(version);
    let header = 48 + SOURCES * size;
    let expected = get(&b, 40);
    b[40..48].fill(0);
    if crc32fast::hash(&b[..header]) != expected {
        return Err(Error::CrcMismatch);
    }
    let bank = Bank {
        offset,
        generation: get(&b, 8),
        version,
        length: get(&b, 12),
        key_length: get(&b, 24),
        count: get(&b, 20),
        checked: i64::from_le_bytes(b[32..40].try_into().unwrap()),
    };
    if bank.length == 0
        || bank.length > BANK_SIZE - SECTOR
        || bank.key_length == 0
        || bank.key_length > bank.length
        || !(1..=SOURCES as u32).contains(&bank.count)
        || bank.checked < 0
    {
        return Err(Error::InvalidZip);
    }
    for i in 0..bank.count as usize {
        let s = Source::decode(&b[48 + i * size..48 + (i + 1) * size]);
        if !valid_date(s.start)
            || !valid_date(s.end)
            || s.start > s.end
            || s.present & !((1 << Table::COUNT) - 1) != 0
            || (s.updated != EMPTY && !(0..=bank.checked).contains(&s.updated))
        {
            return Err(Error::InvalidZip);
        }
        for block in s.tables {
            if block.length != 0
                && (block.offset < bank.key_length
                    || block
                        .offset
                        .checked_add(block.length)
                        .is_none_or(|end| end > bank.length))
            {
                return Err(Error::InvalidZip);
            }
        }
    }
    let mut hash = crc32fast::Hasher::new();
    let mut buf = [0; 256];
    let mut pos = 0;
    while pos < bank.length {
        let n = buf.len().min((bank.length - pos) as usize);
        flash.read(offset + SECTOR + pos, &mut buf[..n])?;
        hash.update(&buf[..n]);
        pos += n as u32;
        if pos % 2048 == 0 {
            embassy_futures::yield_now().await;
        }
    }
    if hash.finalize() != get(&b, 16) {
        return Err(Error::CrcMismatch);
    }
    Ok(bank)
}
/// Full payload validation precedes use; a torn/corrupt newer bank falls back.
pub async fn load(flash: &impl Storage) -> Result<Option<Bank>, Error> {
    let mut generations = [0; 2];
    for (i, g) in generations.iter_mut().enumerate() {
        let mut b = [0; 4];
        flash.read(OFFSET + i as u32 * BANK_SIZE + 8, &mut b)?;
        *g = get(&b, 0);
    }
    let order = if timetable_display::config::newer(generations[1], generations[0]) {
        [1, 0]
    } else {
        [0, 1]
    };
    for i in order {
        match validate(flash, OFFSET + i * BANK_SIZE).await {
            Ok(bank) => return Ok(Some(bank)),
            Err(error @ Error::Io(_)) => return Err(error),
            Err(_) => {}
        }
    }
    Ok(None)
}
fn key_fields(p: &Pick) -> [&str; 5] {
    [
        &p.line,
        &p.route,
        &p.stop_code,
        &p.stop,
        if p.stop_code.is_empty() {
            &p.stop_name
        } else {
            ""
        },
    ]
}
fn key(
    config: &Config,
    identity: &[u8],
    mut write: impl FnMut(&[u8]) -> Result<(), Error>,
) -> Result<u32, Error> {
    let prefix = if identity.is_empty() {
        0
    } else {
        let length: u16 = identity
            .len()
            .try_into()
            .map_err(|_| Error::BufferTooSmall)?;
        write(&[255])?;
        write(&length.to_le_bytes())?;
        for chunk in identity.chunks(64) {
            write(chunk)?;
        }
        3 + u32::from(length)
    };
    let mut picks = [None; 16];
    let mut n = 0;
    for p in config.groups.iter().flat_map(|g| &g.picks) {
        picks[n] = Some(p);
        n += 1;
    }
    picks[..n].sort_unstable_by_key(|p| key_fields(p.unwrap()));
    let count = picks[..n]
        .iter()
        .enumerate()
        .filter(|(i, p)| *i == 0 || key_fields(p.unwrap()) != key_fields(picks[*i - 1].unwrap()))
        .count();
    write(&[count as u8])?;
    let mut len = 1 + prefix;
    for i in 0..n {
        let fields = key_fields(picks[i].unwrap());
        if i > 0 && fields == key_fields(picks[i - 1].unwrap()) {
            continue;
        }
        for field in fields {
            write(&[field.len() as u8])?;
            write(field.as_bytes())?;
            len += 1 + field.len() as u32;
        }
    }
    Ok(len)
}
fn same_key(bank: Bank, flash: &impl Storage, config: &Config) -> Result<bool, Error> {
    let mut pos = 0;
    let mut equal = true;
    let len = key(config, flash.identity(), |bytes| {
        let mut old = [0; 64];
        if bytes.len() > old.len() || pos + bytes.len() as u32 > bank.key_length {
            equal = false;
        } else {
            flash.read(bank.offset + SECTOR + pos, &mut old[..bytes.len()])?;
            equal &= old[..bytes.len()] == *bytes;
        }
        pos += bytes.len() as u32;
        Ok(())
    })?;
    Ok(equal && len == bank.key_length)
}
struct FlashRead<'a, F> {
    flash: &'a F,
    offset: u32,
    remaining: u32,
}
impl<F> ErrorType for FlashRead<'_, F> {
    type Error = Error;
}
impl<F: Storage> Read for FlashRead<'_, F> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        let n = out.len().min(self.remaining as usize);
        if n != 0 {
            self.flash.read(self.offset, &mut out[..n])?;
            self.offset += n as u32;
            self.remaining -= n as u32;
        }
        Ok(n)
    }
}
struct Cached<'a, F> {
    flash: &'a F,
    bank: u32,
    source: Source,
}
impl<F: Storage> Tables for Cached<'_, F> {
    async fn scan(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        _: &mut zip::Workspace,
        mut visit: impl FnMut(Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, Error> {
        let manifest = self.source.manifest()?;
        if let Some(expected) = expected {
            expected.check(&manifest)?;
        }
        for &t in tables {
            let b = self.source.tables[t as usize];
            if b.length == 0 {
                continue;
            }
            let read = Cooperative::new(FlashRead {
                flash: self.flash,
                offset: self.bank + SECTOR + b.offset,
                remaining: b.length,
            });
            let mut rows = gtfs::csv::Reader::<_>::new(read, t);
            while let Some(row) = rows.next().await? {
                visit(row)?;
            }
        }
        Ok(manifest)
    }
}
async fn query_sources(
    flash: &impl Storage,
    bank: u32,
    count: usize,
    mut source_at: impl FnMut(usize) -> Result<Source, Error>,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    config: &Config,
) -> Result<(), Error> {
    let now = builder.now;
    builder.begin(now)?;
    let mut remaining = builder.day_mask();
    for i in 0..count {
        let s = source_at(i)?;
        let mask = s.mask(builder.dates) & remaining;
        if mask == 0 {
            continue;
        }
        let mut source = Cached {
            flash,
            bank,
            source: s,
        };
        builder
            .calendars_from(&mut source, mask, config, workspace)
            .await?;
        builder.trips_from(&mut source, config, workspace).await?;
        builder
            .departures_from(&mut source, config, workspace)
            .await?;
        remaining &= !mask;
        if remaining == 0 {
            return Ok(());
        }
    }
    Err(Error::InvalidDate)
}
/// Query the selected all-period tables after UTC is established. No writes.
pub async fn restore(
    flash: &impl Storage,
    bank: Bank,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    config: &Config,
    now: i64,
) -> Result<Freshness, Error> {
    if !same_key(bank, flash, config)? {
        return Err(Error::InvalidField("cache selection"));
    }
    builder.begin(now)?;
    query_sources(
        flash,
        bank.offset,
        bank.count as usize,
        |i| bank.source(flash, i),
        builder,
        workspace,
        config,
    )
    .await?;
    bank.fresh(flash, builder, builder.now)
}

struct Writer<'a, F> {
    flash: &'a F,
    active: Option<Bank>,
    compare: bool,
    different: bool,
    pos: u32,
    used: usize,
    buffer: &'a mut [u8; 256],
    hash: crc32fast::Hasher,
    table_hash: crc32fast::Hasher,
    sources: &'a mut [Source; SOURCES],
    count: u8,
    now: i64,
}
impl<'a, F: Storage> Writer<'a, F> {
    fn new(
        flash: &'a F,
        active: Option<Bank>,
        compare: bool,
        now: i64,
        sources: &'a mut [Source; SOURCES],
        buffer: &'a mut [u8; 256],
    ) -> Self {
        sources.fill(Source::default());
        Self {
            flash,
            active,
            compare,
            different: false,
            pos: 0,
            used: 0,
            buffer,
            hash: crc32fast::Hasher::new(),
            table_hash: crc32fast::Hasher::new(),
            sources,
            count: 0,
            now,
        }
    }
    fn target(&self) -> u32 {
        self.active.map_or(OFFSET, |bank| {
            if bank.offset == OFFSET {
                OFFSET + BANK_SIZE
            } else {
                OFFSET
            }
        })
    }
    fn append(&mut self, mut bytes: &[u8]) -> Result<(), Error> {
        while !bytes.is_empty() {
            let n = bytes.len().min(self.buffer.len() - self.used);
            if self.pos + self.used as u32 + n as u32 > BANK_SIZE - SECTOR {
                return Err(Error::BufferTooSmall);
            }
            self.hash.update(&bytes[..n]);
            self.table_hash.update(&bytes[..n]);
            self.buffer[self.used..self.used + n].copy_from_slice(&bytes[..n]);
            self.used += n;
            bytes = &bytes[n..];
            if self.used == self.buffer.len() {
                self.flush()?;
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        if self.used == 0 {
            return Ok(());
        }
        if self.compare {
            if let Some(active) = self.active {
                let mut old = [0; 256];
                if self.pos + self.used as u32 > active.length {
                    self.different = true
                } else if !self.different {
                    self.flash
                        .read(active.offset + SECTOR + self.pos, &mut old[..self.used])?;
                    self.different |= old[..self.used] != self.buffer[..self.used];
                }
            } else {
                self.different = true
            }
        } else {
            if self.pos == 0 {
                self.flash.erase(self.target(), self.target() + SECTOR)?;
            }
            let offset = SECTOR + self.pos;
            let aligned = (self.used + 3) & !3;
            // Completed writes already identify the erased payload sectors.
            let mut erased = SECTOR + self.pos.div_ceil(SECTOR) * SECTOR;
            while erased < offset + aligned as u32 {
                self.flash
                    .erase(self.target() + erased, self.target() + erased + SECTOR)?;
                erased += SECTOR;
            }
            self.buffer[self.used..aligned].fill(0xff);
            self.flash
                .write(self.target() + offset, &self.buffer[..aligned])?;
        }
        self.pos += self.used as u32;
        self.used = 0;
        Ok(())
    }
    fn key_length(&self) -> u32 {
        // Source coverage occupies eight bytes immediately before its first
        // table. The validated metadata pass always records routes first.
        self.sources[0].tables[Table::Routes as usize].offset - 8
    }
    fn position(&self) -> u32 {
        self.pos + self.used as u32
    }
    fn start(&mut self, t: Table) -> Result<(), Error> {
        self.sources[self.count as usize].tables[t as usize].offset = self.position();
        self.table_hash = crc32fast::Hasher::new();
        for (i, c) in columns(t).iter().enumerate() {
            if i != 0 {
                self.append(b",")?
            }
            self.append(c.name().as_bytes())?;
        }
        self.append(b"\n")
    }
    fn row(&mut self, row: Record<'_>) -> Result<(), Error> {
        for (i, &c) in columns(row.table()).iter().enumerate() {
            if i != 0 {
                self.append(b",")?
            }
            let value = row.get(c).unwrap_or("");
            let quote = value
                .bytes()
                .any(|c| matches!(c, b'"' | b',' | b'\r' | b'\n'));
            if quote {
                self.append(b"\"")?
            }
            for part in value.split_inclusive('"') {
                self.append(part.as_bytes())?;
                if part.ends_with('"') {
                    self.append(b"\"")?
                }
            }
            if quote {
                self.append(b"\"")?
            }
        }
        self.append(b"\n")
    }
    fn end(&mut self, t: Table) {
        let offset = self.sources[self.count as usize].tables[t as usize].offset;
        self.sources[self.count as usize].tables[t as usize] = Block {
            offset,
            length: self.position() - offset,
            crc: self.table_hash.clone().finalize(),
        };
    }
    fn unchanged(&mut self) -> Result<bool, Error> {
        self.flush()?;
        let Some(active) = self.active else {
            return Ok(false);
        };
        if self.different
            || self.pos != active.length
            || u32::from(self.count) != active.count
            || self.key_length() != active.key_length
        {
            return Ok(false);
        }
        for i in 0..self.count as usize {
            let old = active.source(self.flash, i)?;
            let new = self.sources[i];
            if old.start != new.start || old.end != new.end || old.tables != new.tables {
                return Ok(false);
            }
        }
        Ok(true)
    }
    async fn commit(&mut self) -> Result<Bank, Error> {
        if self.compare || !self.flash.current().await {
            return Err(Error::InvalidField("obsolete cache update"));
        }
        self.flush()?;
        let bank = Bank {
            offset: self.target(),
            generation: self.active.map_or(0, |b| b.generation.wrapping_add(1)),
            length: self.pos,
            key_length: self.key_length(),
            count: self.count as u32,
            version: 3,
            checked: self
                .flash
                .checked_at()
                .filter(|stamp| *stamp >= self.now)
                .unwrap_or(self.now),
        };
        let mut h = [0; HEADER];
        h[..4].copy_from_slice(b"TTFC");
        put(&mut h, 4, 3);
        put(&mut h, 8, bank.generation);
        put(&mut h, 12, bank.length);
        put(&mut h, 16, self.hash.clone().finalize());
        put(&mut h, 20, bank.count);
        put(&mut h, 24, bank.key_length);
        h[32..40].copy_from_slice(&bank.checked.to_le_bytes());
        for (i, s) in self.sources[..self.count as usize].iter().enumerate() {
            s.encode(&mut h[48 + i * SOURCE_BYTES..48 + (i + 1) * SOURCE_BYTES]);
        }
        let crc = crc32fast::hash(&h);
        put(&mut h, 40, crc);
        h[44..48].fill(0xff);
        self.flash.write(bank.offset, &h)?;
        // Commit word last. Read-back validation is performed by the caller.
        self.flash.write(bank.offset + 44, &COMMIT.to_le_bytes())?;
        Ok(bank)
    }
}
fn columns(t: Table) -> &'static [C] {
    match t {
        Table::Routes => &[C::RouteId, C::RouteShortName, C::RouteType],
        Table::Stops => &[C::StopId, C::StopCode, C::StopName],
        Table::Trips => &[
            C::RouteId,
            C::ServiceId,
            C::TripId,
            C::TripHeadsign,
            C::DirectionId,
        ],
        Table::StopTimes => &[
            C::TripId,
            C::StopId,
            C::StopSequence,
            C::ArrivalTime,
            C::DepartureTime,
            C::PickupType,
            C::DropOffType,
            C::StopHeadsign,
        ],
        Table::Calendar => &[
            C::ServiceId,
            C::Monday,
            C::Tuesday,
            C::Wednesday,
            C::Thursday,
            C::Friday,
            C::Saturday,
            C::Sunday,
            C::StartDate,
            C::EndDate,
        ],
        Table::CalendarDates => &[C::ServiceId, C::Date, C::ExceptionType],
        Table::FeedInfo => &[C::FeedStartDate, C::FeedEndDate],
        Table::Transfers => &[
            C::FromStopId,
            C::ToStopId,
            C::TransferType,
            C::MinTransferTime,
            C::FromRouteId,
            C::ToRouteId,
            C::FromTripId,
            C::ToTripId,
        ],
    }
}

fn metadata_selected(row: Record<'_>, config: &Config) -> Result<bool, Error> {
    match row.table() {
        Table::Routes => {
            let r = row.route()?;
            Ok(config
                .groups
                .iter()
                .flat_map(|g| &g.picks)
                .any(|p| timetable_display::feed::route_matches(r, p)))
        }
        Table::Stops => {
            let s = row.stop()?;
            Ok(config.groups.iter().flat_map(|g| &g.picks).any(|p| {
                if p.stop_code.is_empty() {
                    s.id == p.stop
                } else {
                    s.code == Some(p.stop_code.as_str())
                }
            }))
        }
        _ => Ok(true),
    }
}
struct Recording<'a, 'b, 'c, H, F> {
    source: &'a mut crate::download::Mirror<'b, H>,
    writer: &'a mut Writer<'c, F>,
    config: &'a Config,
}
impl<H: crate::download::Http, F: Storage> Tables for Recording<'_, '_, '_, H, F> {
    async fn scan(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, Error> {
        let manifest = self.source.manifest();
        if let Some(expected) = expected {
            expected.check(&manifest)?;
        }
        for &t in tables {
            if manifest.get(t).is_none() {
                continue;
            }
            self.writer.start(t)?;
            self.source
                .scan(&[t], Some(&manifest), workspace, |row| {
                    if metadata_selected(row, self.config)? {
                        self.writer.row(row)?;
                    }
                    visit(row)
                })
                .await?;
            self.writer.end(t);
        }
        Ok(manifest)
    }
}
fn bloom(bits: &mut [u8], id: &str, insert: bool) -> bool {
    let hash = crc32fast::hash(id.as_bytes());
    let step = hash.rotate_left(13) | 1;
    let mut present = true;
    for i in 0..4_u32 {
        let bit = hash.wrapping_add(i.wrapping_mul(step)) as usize % (bits.len() * 8);
        let byte = &mut bits[bit / 8];
        let mask = 1 << (bit % 8);
        present &= *byte & mask != 0;
        if insert {
            *byte |= mask;
        }
    }
    present
}
async fn build<H: crate::download::Http, F: Storage>(
    http: &H,
    snapshots: &crate::download::Snapshots,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    config: &Config,
    recording: (&mut Writer<'_, F>, &mut [u8]),
    departures: bool,
) -> Result<(), crate::download::Error> {
    let (writer, bits) = recording;
    builder.begin(writer.now)?;
    key(config, writer.flash.identity(), |b| writer.append(b))?;
    let mut remaining = builder.day_mask();
    for &snapshot in &snapshots.0 {
        let mut source = crate::download::Mirror::open(http, snapshot, workspace).await?;
        let mask = {
            let (start, end) = source.coverage_dates(workspace).await?;
            let manifest = source.manifest();
            let mut info = Source {
                start: date(start),
                end: date(end),
                updated: manifest
                    .updated_at()
                    .filter(|t| (0..=builder.now).contains(t))
                    .unwrap_or(EMPTY),
                ..Source::default()
            };
            info.snapshot = Some(snapshot);
            info.validator = source.validator();
            let mask = info.mask(builder.dates) & remaining;
            if mask == 0 {
                continue;
            }
            if writer.count as usize == SOURCES {
                return Err(Error::BufferTooSmall.into());
            }
            for t in Table::ALL {
                if let Some(f) = manifest.get(t) {
                    info.present |= 1 << t as usize;
                    info.original[t as usize] = [f.crc32, f.uncompressed_size];
                }
            }
            writer.sources[writer.count as usize] = info;
            writer.append(&info.start.to_le_bytes())?;
            writer.append(&info.end.to_le_bytes())?;
            mask
        };
        builder
            .calendars_from(
                &mut Recording {
                    source: &mut source,
                    writer,
                    config,
                },
                mask,
                config,
                workspace,
            )
            .await?;
        bits.fill(0);
        writer.start(Table::Trips)?;
        builder
            .trips_from_recording(&mut source, config, workspace, |row, selection| {
                let trip = row.trip()?;
                if selection.route(trip.route_id) {
                    bloom(bits, trip.id, true);
                    writer.row(row)?;
                }
                Ok(())
            })
            .await?;
        writer.end(Table::Trips);
        if departures {
            writer.start(Table::StopTimes)?;
            builder
                .departures_from_recording(&mut source, config, workspace, |row, selection| {
                    if selection.stop(row.get(C::StopId).unwrap_or(""))
                        && bloom(bits, row.get(C::TripId).unwrap_or(""), false)
                    {
                        // Bloom hits can be unrelated. The exact join validates
                        // real selected rows during this provisional pass.
                        writer.row(row)?;
                    }
                    Ok(())
                })
                .await?;
            writer.end(Table::StopTimes);
        }
        writer.count += 1;
        remaining &= !mask;
        if remaining == 0 {
            writer.flush()?;
            return Ok(());
        }
    }
    Err(crate::download::Error::MissingArchive)
}
async fn matches<H: crate::download::Http>(
    http: &H,
    snapshots: &crate::download::Snapshots,
    flash: &impl Storage,
    bank: Bank,
    builder: &Builder,
    workspace: &mut zip::Workspace,
) -> Result<bool, crate::download::Error> {
    let mut remaining = builder.day_mask();
    for &snapshot in &snapshots.0 {
        let mut url = [0; gtfs::mobility::URL_BYTES];
        let url = snapshot.url(&mut url);
        let mut unchanged_mask = 0;
        for i in 0..bank.count as usize {
            let stored = bank.source(flash, i)?;
            if stored.snapshot == Some(snapshot)
                && stored.validator.header().is_some()
                && http.unchanged(url, &stored.validator).await?
            {
                let mask = stored.mask(builder.dates) & remaining;
                // A different earlier stored source must not win any of these days.
                for day in 0..3 {
                    if mask & (1 << day) != 0 {
                        for winner in 0..=i {
                            if bank.source(flash, winner)?.mask(builder.dates) & (1 << day) != 0 {
                                if winner != i {
                                    return Ok(false);
                                }
                                break;
                            }
                        }
                    }
                }
                unchanged_mask = mask;
                break;
            }
        }
        if unchanged_mask != 0 {
            remaining &= !unchanged_mask;
            if remaining == 0 {
                return Ok(true);
            }
            continue;
        }
        let mut source = crate::download::Mirror::open(http, snapshot, workspace).await?;
        let (start, end) = source.coverage_dates(workspace).await?;
        let mask = builder.dates.iter().enumerate().fold(0, |m, (i, d)| {
            m | if start <= *d && *d <= end { 1 << i } else { 0 }
        }) & remaining;
        if mask == 0 {
            continue;
        }
        // Restore gives the first stored archive priority for each day.
        // Compare that actual winner, not just any matching older archive.
        for day in 0..3 {
            if mask & (1 << day) == 0 {
                continue;
            }
            let mut found = false;
            for i in 0..bank.count as usize {
                let stored = bank.source(flash, i)?;
                if stored.mask(builder.dates) & (1 << day) != 0 {
                    found = stored.matches(&source.manifest(), start, end);
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }
        remaining &= !mask;
        if remaining == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}
/// The mirror session calls this on one verified persistent connection.
/// A changed active cache is compared without writes, then streamed a second
/// time into the inactive bank. Unchanged selected bytes cause zero erases and
/// writes even when unrelated rows, ZIP packing or source timestamps change.
/// With no active bank, preflight only metadata/trip bounds, then stream stop
/// times once into an uncommitted bank. Complete validation still precedes use.
pub async fn mirror_tables<H: crate::download::Http, F: Storage>(
    http: &H,
    snapshots: &crate::download::Snapshots,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    config: &Config,
    flash: &F,
) -> Result<Freshness, crate::download::Error> {
    let active = load(flash).await?;
    {
        if let Some(bank) = active
            && same_key(bank, flash, config)?
            && matches(http, snapshots, flash, bank, builder, workspace).await?
        {
            http.stage("cache: source unchanged; no flash writes");
            let mut fresh = restore(flash, bank, builder, workspace, config, builder.now).await?;
            fresh.checked_at = Some(builder.now);
            return Ok(fresh);
        }
    }
    let mut allocation = flash.metadata()?;
    let metadata = allocation
        .as_mut()
        .first_mut()
        .ok_or(Error::BufferTooSmall)?;
    let mut writer = Writer::new(
        flash,
        active,
        true,
        builder.now,
        &mut metadata.sources,
        &mut metadata.buffer,
    );
    // This operation is scoped inside the already verified TLS session. The
    // caller can reuse heap space freed by certificate verification here.
    let mut scratch = flash.scratch()?;
    let bits = scratch.as_mut();
    if bits.len() < FILTER_BYTES {
        return Err(Error::BufferTooSmall.into());
    }
    let bits = &mut bits[..FILTER_BYTES];
    {
        let compare_departures = writer.active.is_some();
        http.stage(if compare_departures {
            "cache: validating and comparing selected tables; one scan per table"
        } else {
            "cache: empty; checking metadata/trip bounds before single stop_times scan"
        });
        build(
            http,
            snapshots,
            builder,
            workspace,
            config,
            (&mut writer, bits),
            compare_departures,
        )
        .await?;
        if writer.unchanged()? {
            http.stage("cache: selected contents unchanged; no flash writes");
            let mut fresh = restore(
                flash,
                writer.active.unwrap(),
                builder,
                workspace,
                config,
                writer.now,
            )
            .await?;
            fresh.checked_at = Some(writer.now);
            return Ok(fresh);
        }
    }
    if let Some(bank) = writer.active
        && writer.now.saturating_sub(bank.checked_at()) < 24 * 3600
        && !flash.approve_update(bank.checked_at())
    {
        return Err(crate::download::Error::CacheUpdateRequired(
            bank.checked_at(),
        ));
    }
    if !flash.current().await {
        return Err(Error::InvalidField("obsolete cache update").into());
    }
    http.stage("cache: selected contents changed; streaming inactive bank");
    let mut writer = Writer::new(
        flash,
        writer.active,
        false,
        writer.now,
        writer.sources,
        writer.buffer,
    );
    build(
        http,
        snapshots,
        builder,
        workspace,
        config,
        (&mut writer, bits),
        true,
    )
    .await?;
    // Verify the complete selected query before committing the new generation.
    query_sources(
        flash,
        writer.target(),
        writer.count as usize,
        |i| Ok(writer.sources[i]),
        builder,
        workspace,
        config,
    )
    .await?;
    let bank = writer.commit().await?;
    validate(flash, bank.offset).await?;
    http.stage("cache: validated bank committed");
    Ok(bank.fresh(flash, builder, bank.checked)?)
}

pub struct Http<'a, H, F> {
    pub inner: &'a H,
    pub flash: &'a F,
    pub enabled: bool,
}
impl<H: crate::download::Http, F: Storage> crate::download::Http for Http<'_, H, F> {
    async fn official_tables(
        &self,
        builder: &mut Builder,
        workspace: &mut zip::Workspace,
        config: &Config,
    ) -> Result<Freshness, crate::download::Error> {
        self.inner.official_tables(builder, workspace, config).await
    }
    async fn journey_plan(
        &self,
        builder: &mut Builder,
        source: &mut impl Tables,
        mask: u8,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        self.inner
            .journey_plan(builder, source, mask, config, workspace)
            .await
    }
    fn feed_id(&self) -> &str {
        self.inner.feed_id()
    }
    fn stage(&self, s: &str) {
        self.inner.stage(s)
    }
    fn fallback(&self, e: &crate::download::Error) {
        self.inner.fallback(e)
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(gtfs::http::ByteRange, Option<u64>)>,
        op: &mut impl crate::download::Operation,
    ) -> Result<crate::download::Metadata, crate::download::Error> {
        self.inner.fetch(url, range, op).await
    }
    async fn unchanged(
        &self,
        url: &str,
        validator: &crate::download::Validator,
    ) -> Result<bool, crate::download::Error> {
        self.inner.unchanged(url, validator).await
    }
    async fn mirror_tables(
        &self,
        snapshots: &crate::download::Snapshots,
        builder: &mut Builder,
        workspace: &mut zip::Workspace,
        config: &Config,
    ) -> Result<Freshness, crate::download::Error> {
        if !self.enabled {
            return self
                .inner
                .mirror_tables(snapshots, builder, workspace, config)
                .await;
        }
        match self
            .inner
            .mirror_cached_tables(snapshots, builder, workspace, config, self.flash)
            .await
        {
            Ok(f) => Ok(f),
            Err(crate::download::Error::Feed(Error::BufferTooSmall | Error::Io(_))) => {
                self.inner
                    .stage("cache: storage/capacity unavailable; using normal streaming query");
                builder.begin(builder.now)?;
                self.inner
                    .mirror_tables(snapshots, builder, workspace, config)
                    .await
            }
            Err(e) => Err(e),
        }
    }
}
pub struct Query<'a, F> {
    pub snapshots: &'a crate::download::Snapshots,
    pub builder: &'a mut Builder,
    pub workspace: &'a mut zip::Workspace,
    pub config: &'a Config,
    pub flash: &'a F,
}
impl<F: Storage> crate::download::SessionOperation for Query<'_, F> {
    type Output = Freshness;
    async fn run(
        &mut self,
        http: &impl crate::download::Http,
    ) -> Result<Freshness, crate::download::Error> {
        mirror_tables(
            http,
            self.snapshots,
            self.builder,
            self.workspace,
            self.config,
            self.flash,
        )
        .await
    }
}
