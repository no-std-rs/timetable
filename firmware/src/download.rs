//! Shared, bounded download pipeline used by the device and host diagnostic.
use crate::sources::Archives;
use embedded_io_async::Read;
use gtfs::{
    Date, Table, csv,
    http::{ByteRange, RangeFetch, RangeReader},
    mobility::{Snapshot as MirrorSnapshot, URL_BYTES},
    zip,
};
use timetable_display::{
    Config,
    feed::{Builder, Tables},
    freshness::Freshness,
};

#[derive(Debug)]
pub enum Error {
    Network,
    Tls,
    Http,
    Clock,
    MissingArchive,
    /// Read-only comparison found changed content needing a frequent write.
    CacheUpdateRequired(i64),
    /// The user kept the saved cache instead of approving another daily write.
    CacheUpdateDeclined,
    Feed(gtfs::Error),
}
impl From<gtfs::Error> for Error {
    fn from(e: gtfs::Error) -> Self {
        Self::Feed(e)
    }
}
impl Error {
    fn feed(self) -> gtfs::Error {
        match self {
            Self::Feed(e) => e,
            _ => gtfs::Error::Io(embedded_io_async::ErrorKind::Other),
        }
    }
}

#[allow(async_fn_in_trait)]
pub trait Operation {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error>;
}
/// A bounded server validator, retained with the validated archive identity.
#[derive(Clone, Copy)]
pub struct Validator {
    bytes: [u8; 64],
    len: u8,
    kind: u8,
}
impl Default for Validator {
    fn default() -> Self {
        Self {
            bytes: [0; 64],
            len: 0,
            kind: 0,
        }
    }
}
impl Validator {
    pub const BYTES: usize = 66;
    pub fn new(name: &str, value: &[u8]) -> Self {
        let kind = if name.eq_ignore_ascii_case("etag") {
            1
        } else if name.eq_ignore_ascii_case("last-modified") {
            2
        } else {
            0
        };
        let quoted = value.strip_prefix(b"W/").unwrap_or(value);
        if kind == 0
            || value.is_empty()
            || value.len() > 64
            || !value.iter().all(|b| (32..=126).contains(b))
            || kind == 1
                && (quoted.len() < 2
                    || quoted.first() != Some(&b'"')
                    || quoted.last() != Some(&b'"')
                    || quoted[1..quoted.len() - 1].contains(&b'"'))
        {
            return Self::default();
        }
        let mut result = Self::default();
        result.bytes[..value.len()].copy_from_slice(value);
        result.len = value.len() as u8;
        result.kind = kind;
        result
    }
    pub fn header(&self) -> Option<(&str, &str)> {
        (self.len != 0).then(|| {
            (
                if self.kind == 1 {
                    "If-None-Match"
                } else {
                    "If-Modified-Since"
                },
                core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap(),
            )
        })
    }
    pub fn verify(&self, actual: &Self) -> Result<(), Error> {
        let Some((_, expected)) = self.header() else {
            return Ok(());
        };
        let (_, value) = actual.header().ok_or(Error::Http)?;
        let equal = if self.kind == 1 {
            expected.strip_prefix("W/").unwrap_or(expected)
                == value.strip_prefix("W/").unwrap_or(value)
        } else {
            expected == value
        };
        if self.kind == actual.kind && equal {
            Ok(())
        } else {
            Err(Error::Http)
        }
    }
    pub fn encode(&self, out: &mut [u8]) {
        out[0] = self.len;
        out[1] = self.kind;
        out[2..Self::BYTES].copy_from_slice(&self.bytes);
    }
    pub fn decode(bytes: &[u8]) -> Self {
        let len = bytes[0] as usize;
        if len > 64 {
            return Self::default();
        }
        Self::new(
            match bytes[1] {
                1 => "etag",
                2 => "last-modified",
                _ => "",
            },
            &bytes[2..2 + len],
        )
    }
}
#[derive(Default)]
pub struct Metadata {
    pub length: Option<u64>,
    pub modified: Option<i64>,
    pub validator: Validator,
}
#[allow(async_fn_in_trait)]
pub trait Http {
    fn feed_id(&self) -> &str {
        "mdb-982"
    }
    /// A range must be exact, HTTP 206, identity encoded and from the pinned
    /// object length. Full requests require HTTP 200. Reject duplicate headers.
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error>;
    /// True only for a solicited 304 against this exact validated object.
    async fn unchanged(&self, _url: &str, _validator: &Validator) -> Result<bool, Error> {
        Ok(false)
    }
    /// Transports can scope one authenticated connection around all file ranges.
    /// The default keeps simple stateless transports and offline fixtures usable.
    async fn mirror_tables(
        &self,
        snapshots: &Snapshots,
        builder: &mut Builder,
        workspace: &mut zip::Workspace,
        config: &Config,
    ) -> Result<Freshness, Error>
    where
        Self: Sized,
    {
        mirror_tables(self, snapshots, builder, workspace, config).await
    }
    async fn mirror_cached_tables(
        &self,
        snapshots: &Snapshots,
        builder: &mut Builder,
        workspace: &mut zip::Workspace,
        config: &Config,
        flash: &impl crate::cache::Storage,
    ) -> Result<Freshness, Error>
    where
        Self: Sized,
    {
        crate::cache::mirror_tables(self, snapshots, builder, workspace, config, flash).await
    }
    fn stage(&self, _message: &str) {}
    fn fallback(&self, _error: &Error) {
        self.stage("MobilityDatabase failed; using official ZTM fallback");
    }
}

/// Work scoped to a transport-owned authenticated connection. Buffers stay in
/// the caller's future; no self-referential connection storage is required.
#[allow(async_fn_in_trait)]
pub trait SessionOperation {
    type Output;
    async fn run(&mut self, http: &impl Http) -> Result<Self::Output, Error>;
}
pub struct Fetch<'a, O> {
    pub url: &'a str,
    pub range: Option<(ByteRange, Option<u64>)>,
    pub operation: &'a mut O,
}
impl<O: Operation> SessionOperation for Fetch<'_, O> {
    type Output = Metadata;
    async fn run(&mut self, http: &impl Http) -> Result<Metadata, Error> {
        http.fetch(self.url, self.range, self.operation).await
    }
}
pub struct MirrorQuery<'a> {
    pub snapshots: &'a Snapshots,
    pub builder: &'a mut Builder,
    pub workspace: &'a mut zip::Workspace,
    pub config: &'a Config,
}
impl SessionOperation for MirrorQuery<'_> {
    type Output = Freshness;
    async fn run(&mut self, http: &impl Http) -> Result<Freshness, Error> {
        let mut query = core::pin::pin!(mirror_tables(
            http,
            self.snapshots,
            self.builder,
            self.workspace,
            self.config,
        ));
        core::future::poll_fn(|cx| poll_mirror(query.as_mut(), cx)).await
    }
}

// Poll the large query only after TLS.open has returned. Unlike annotating an
// async constructor, this creates a real call boundary around the query frame.
#[inline(never)]
fn poll_mirror<F: core::future::Future>(
    query: core::pin::Pin<&mut F>,
    cx: &mut core::task::Context<'_>,
) -> core::task::Poll<F::Output> {
    query.poll(cx)
}

pub struct Fill<'a>(pub &'a mut [u8]);
impl Operation for Fill<'_> {
    async fn read<R: Read>(&mut self, mut body: R) -> Result<(), Error> {
        body.read_exact(self.0)
            .await
            .map_err(|_| Error::Feed(gtfs::Error::UnexpectedEof))?;
        end(&mut body).await
    }
}
async fn end<R: Read>(body: &mut R) -> Result<(), Error> {
    if body.read(&mut [0]).await.map_err(|_| Error::Network)? != 0 {
        return Err(gtfs::Error::SizeMismatch.into());
    }
    Ok(())
}
pub struct Snapshots(pub heapless::Vec<MirrorSnapshot, 8>);
impl Snapshots {
    fn add(&mut self, snapshot: MirrorSnapshot) {
        if self.0.contains(&snapshot) {
            return;
        }
        if self.0.is_full() {
            if *self.0.last().unwrap() >= snapshot {
                return;
            }
            self.0.pop();
        }
        self.0.push(snapshot).unwrap();
        self.0.sort_unstable_by(|a, b| b.cmp(a));
    }
}
struct SnapshotListing<'a> {
    snapshots: &'a mut Snapshots,
    feed: &'a str,
}
impl Operation for SnapshotListing<'_> {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error> {
        let mut listing = gtfs::mobility::Snapshots::new(body);
        while let Some(snapshot) = listing.next().await? {
            if snapshot.feed_id() == self.feed {
                self.snapshots.add(snapshot);
            }
        }
        if self.snapshots.0.is_empty() {
            return Err(Error::MissingArchive);
        }
        Ok(())
    }
}
impl Operation for Snapshots {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error> {
        SnapshotListing {
            snapshots: self,
            feed: "mdb-982",
        }
        .read(body)
        .await
    }
}
struct Range<'a, H> {
    http: &'a H,
    url: &'a str,
    length: u64,
    validator: &'a Validator,
}
impl<H: Http> RangeFetch for Range<'_, H> {
    async fn fetch(&mut self, range: ByteRange, out: &mut [u8]) -> Result<(), gtfs::Error> {
        let meta = self
            .http
            .fetch(self.url, Some((range, Some(self.length))), &mut Fill(out))
            .await
            .map_err(Error::feed)?;
        self.validator.verify(&meta.validator).map_err(Error::feed)
    }
}
struct Payload<'a, V, F> {
    member: zip::Member,
    workspace: &'a mut zip::Workspace,
    visit: V,
    filter: F,
}
// Keep strict and matching futures separate so metadata operations retain
// their small async state instead of storing both parser modes.
trait RowFilter {
    async fn next<'a, R: Read>(
        &mut self,
        rows: &'a mut csv::Reader<R>,
    ) -> Result<Option<csv::Record<'a>>, gtfs::Error>;
}
struct AllRows;
impl RowFilter for AllRows {
    async fn next<'a, R: Read>(
        &mut self,
        rows: &'a mut csv::Reader<R>,
    ) -> Result<Option<csv::Record<'a>>, gtfs::Error> {
        rows.next().await
    }
}
struct Matching<'a, F>(csv::Column, &'a mut F);
impl<F: csv::Filter> RowFilter for Matching<'_, F> {
    async fn next<'a, R: Read>(
        &mut self,
        rows: &'a mut csv::Reader<R>,
    ) -> Result<Option<csv::Record<'a>>, gtfs::Error> {
        rows.next_matching(self.0, &mut *self.1).await
    }
}
struct Frequencies<'a> {
    directory: &'a zip::Directory,
    workspace: &'a mut zip::Workspace,
}
impl Operation for Frequencies<'_> {
    async fn read<R: Read>(&mut self, mut body: R) -> Result<(), Error> {
        self.directory
            .validate_frequencies(&mut body, self.workspace)
            .await?;
        let mut scratch = [0; 256];
        while body.read(&mut scratch).await.map_err(|_| Error::Network)? != 0 {}
        Ok(())
    }
}
impl<V: FnMut(csv::Record<'_>) -> Result<(), gtfs::Error>, F: RowFilter> Operation
    for Payload<'_, V, F>
{
    async fn read<R: Read>(&mut self, mut body: R) -> Result<(), Error> {
        {
            let zip = zip::Reader::record(&mut body, self.workspace, self.member).await?;
            // Bound decoded CSV work too: a small compressed input can expand
            // to many rows without another network read.
            let mut rows = csv::Reader::<_>::new(
                crate::cooperative::Cooperative::new(zip),
                self.member.table(),
            );
            loop {
                let row = self.filter.next(&mut rows).await?;
                let Some(row) = row else { break };
                (self.visit)(row)?;
            }
        }
        // A complete local record may include a descriptor or padding already
        // prefetched by the ZIP reader. Consume the exact range before reuse.
        let mut scratch = [0; 256];
        while body.read(&mut scratch).await.map_err(|_| Error::Network)? != 0 {}
        Ok(())
    }
}
pub struct Mirror<'a, H> {
    http: &'a H,
    snapshot: MirrorSnapshot,
    length: u64,
    directory: zip::Directory,
    validator: Validator,
}
impl<'a, H: Http> Mirror<'a, H> {
    pub async fn open(
        http: &'a H,
        snapshot: MirrorSnapshot,
        workspace: &mut zip::Workspace,
    ) -> Result<Self, Error> {
        let mut url = [0; URL_BYTES];
        let url = snapshot.url(&mut url);
        let (length, validator) = {
            let meta = http
                .fetch(
                    url,
                    Some((ByteRange::new(0, 0)?, None)),
                    &mut Fill(&mut [0]),
                )
                .await?;
            (meta.length.ok_or(Error::Http)?, meta.validator)
        };
        let fetcher = Range {
            http,
            url,
            length,
            validator: &validator,
        };
        let (cache, scratch) = workspace.scratch()[..3072].split_at_mut(2048);
        let mut reader = RangeReader::new(fetcher, length, cache)?;
        let directory = zip::Directory::read(&mut reader, scratch).await?;
        if let Some(range) = directory.frequencies_range() {
            let meta = http
                .fetch(
                    url,
                    Some((range, Some(length))),
                    &mut Frequencies {
                        directory: &directory,
                        workspace,
                    },
                )
                .await?;
            validator.verify(&meta.validator)?;
        }
        Ok(Self {
            http,
            snapshot,
            length,
            directory,
            validator,
        })
    }
    pub fn validator(&self) -> Validator {
        self.validator
    }
    async fn table(
        &mut self,
        table: Table,
        workspace: &mut zip::Workspace,
        visit: impl FnMut(csv::Record<'_>) -> Result<(), gtfs::Error>,
    ) -> Result<(), Error> {
        self.table_matching(table, workspace, AllRows, visit).await
    }
    async fn table_matching<F: RowFilter>(
        &mut self,
        table: Table,
        workspace: &mut zip::Workspace,
        filter: F,
        visit: impl FnMut(csv::Record<'_>) -> Result<(), gtfs::Error>,
    ) -> Result<(), Error> {
        let member = self
            .directory
            .member(table)
            .ok_or(gtfs::Error::MissingTable(table))?;
        self.http.stage(table.filename());
        let mut url = [0; URL_BYTES];
        let meta = self
            .http
            .fetch(
                self.snapshot.url(&mut url),
                Some((member.record_range(), Some(self.length))),
                &mut Payload {
                    member,
                    workspace,
                    visit,
                    filter,
                },
            )
            .await?;
        self.validator.verify(&meta.validator)
    }
    pub async fn coverage(
        &mut self,
        dates: [Date; 3],
        workspace: &mut zip::Workspace,
    ) -> Result<u8, Error> {
        let (start, end) = self.coverage_dates(workspace).await?;
        Ok(dates.iter().enumerate().fold(0, |m, (i, d)| {
            m | if start <= *d && *d <= end { 1 << i } else { 0 }
        }))
    }
    pub fn manifest(&self) -> zip::Manifest {
        zip::Manifest::from_directory(&self.directory)
    }
    pub async fn coverage_dates(
        &mut self,
        workspace: &mut zip::Workspace,
    ) -> Result<(Date, Date), Error> {
        let mut coverage = None;
        if self.directory.member(Table::FeedInfo).is_some() {
            self.table(Table::FeedInfo, workspace, |row| {
                let start = row.get(csv::Column::FeedStartDate).unwrap_or("");
                let end = row.get(csv::Column::FeedEndDate).unwrap_or("");
                // A partial optional pair cannot define a complete interval;
                // derive coverage from service calendars below instead.
                if start.is_empty() || end.is_empty() {
                    return Ok(());
                }
                let start: Date = start.parse()?;
                let end: Date = end.parse()?;
                if start > end || coverage.is_some() {
                    return Err(gtfs::Error::InvalidDate);
                }
                coverage = Some((start, end));
                Ok(())
            })
            .await?;
        }
        if coverage.is_none() {
            // feed_info and its dates are optional GTFS. Calendar-only feeds
            // still use pinned ranges and the exact same staged query checks.
            for table in [Table::Calendar, Table::CalendarDates] {
                if self.directory.member(table).is_none() {
                    continue;
                }
                self.table(table, workspace, |row| {
                    let (start, end): (Date, Date) = if table == Table::Calendar {
                        (
                            row.required(csv::Column::StartDate)?.parse()?,
                            row.required(csv::Column::EndDate)?.parse()?,
                        )
                    } else {
                        let date = row.required(csv::Column::Date)?.parse()?;
                        (date, date)
                    };
                    if start > end {
                        return Err(gtfs::Error::InvalidDate);
                    }
                    coverage = Some(coverage.map_or((start, end), |(a, b): (Date, Date)| {
                        (a.min(start), b.max(end))
                    }));
                    Ok(())
                })
                .await?;
            }
        }
        coverage.ok_or(Error::MissingArchive)
    }
}
impl<H: Http> Tables for Mirror<'_, H> {
    async fn scan(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(csv::Record<'_>) -> Result<(), gtfs::Error>,
    ) -> Result<zip::Manifest, gtfs::Error> {
        let manifest = zip::Manifest::from_directory(&self.directory);
        if let Some(expected) = expected {
            expected.check(&manifest)?;
        }
        for table in tables {
            if self.directory.member(*table).is_some() {
                self.table(*table, workspace, &mut visit)
                    .await
                    .map_err(Error::feed)?;
            }
        }
        Ok(manifest)
    }

    async fn scan_matching(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        column: csv::Column,
        mut keep: impl csv::Filter,
        mut visit: impl FnMut(csv::Record<'_>) -> Result<(), gtfs::Error>,
    ) -> Result<zip::Manifest, gtfs::Error> {
        let manifest = zip::Manifest::from_directory(&self.directory);
        if let Some(expected) = expected {
            expected.check(&manifest)?;
        }
        for table in tables {
            if self.directory.member(*table).is_some() {
                self.table_matching(*table, workspace, Matching(column, &mut keep), &mut visit)
                    .await
                    .map_err(Error::feed)?;
            }
        }
        Ok(manifest)
    }
}

/// No provisional mirror events survive a failure: the caller restarts the
/// official path with Builder::begin before any result can be published.
pub async fn mirror(
    http: &impl Http,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    config: &Config,
) -> Result<Freshness, Error> {
    http.stage("MobilityDatabase: discovering immutable snapshots");
    let mut candidates = Snapshots(heapless::Vec::new());
    if !gtfs::mobility::valid_id(http.feed_id()) {
        return Err(Error::MissingArchive);
    }
    use core::fmt::Write;
    let mut page = heapless::String::<128>::new();
    write!(
        page,
        "https://mobilitydatabase.org/feeds/gtfs/{}",
        http.feed_id()
    )
    .map_err(|_| Error::MissingArchive)?;
    http.fetch(
        &page,
        None,
        &mut SnapshotListing {
            snapshots: &mut candidates,
            feed: http.feed_id(),
        },
    )
    .await?;
    http.mirror_tables(&candidates, builder, workspace, config)
        .await
}

pub async fn mirror_tables<H: Http>(
    http: &H,
    candidates: &Snapshots,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    config: &Config,
) -> Result<Freshness, Error> {
    let mut remaining = builder.day_mask();
    let mut times = timetable_display::freshness::SourceTimes::default();
    for &snapshot in &candidates.0 {
        let mut source = Mirror::open(http, snapshot, workspace).await?;
        let mask = source.coverage(builder.dates, workspace).await? & remaining;
        if mask == 0 {
            continue;
        }
        builder
            .calendars_from(&mut source, mask, config, workspace)
            .await?;
        builder.trips_from(&mut source, config, workspace).await?;
        builder
            .departures_from(&mut source, config, workspace)
            .await?;
        times.observe(
            builder
                .manifest
                .as_ref()
                .and_then(zip::Manifest::updated_at)
                .filter(|stamp| (0..=builder.now).contains(stamp)),
        );
        remaining &= !mask;
        if remaining == 0 {
            // Mirror Last-Modified is a mirror fetch time. Use only the ZIP
            // table UTC timestamps, leaving missing source metadata unknown.
            return Ok(Freshness {
                checked_at: Some(builder.now),
                source_updated_at: times.updated_at(),
            });
        }
    }
    Err(Error::MissingArchive)
}

impl Operation for Archives {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error> {
        let mut listing = gtfs::poznan::Archives::new(body);
        while let Some(name) = listing.next().await? {
            self.consider(name);
        }
        Ok(())
    }
}
enum Phase {
    Calendars(u8),
    Trips,
    Departures,
}
struct Pass<'a> {
    builder: &'a mut Builder,
    workspace: &'a mut gtfs::zip::Workspace,
    config: &'a Config,
    phase: Phase,
}
impl Operation for Pass<'_> {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error> {
        match self.phase {
            Phase::Calendars(mask) => {
                self.builder
                    .calendars(body, mask, self.config, self.workspace)
                    .await?
            }
            Phase::Trips => {
                self.builder
                    .trips(body, self.config, self.workspace)
                    .await?
            }
            Phase::Departures => {
                self.builder
                    .departures(body, self.config, self.workspace)
                    .await?
            }
        }
        Ok(())
    }
}

pub async fn official(
    http: &impl Http,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    c: &Config,
) -> Result<Freshness, Error> {
    let mut listing = Archives::new(builder.dates, builder.day_mask());
    let _ = http
        .fetch(gtfs::poznan::LIST_URL, None, &mut listing)
        .await?;
    let sources = listing.sources().ok_or(Error::MissingArchive)?;
    let mut times = timetable_display::freshness::SourceTimes::default();
    for (name, mask) in sources {
        if mask == 0 {
            continue;
        }
        let name = name.ok_or(Error::MissingArchive)?;
        let mut url = [0; 192];
        let url = name.url(&mut url)?;
        for phase in [Phase::Calendars(mask), Phase::Trips, Phase::Departures] {
            http.stage(match phase {
                Phase::Calendars(_) => "calendars/routes/stops",
                Phase::Trips => "selected trips",
                Phase::Departures => "selected departures",
            });
            let modified = http
                .fetch(
                    url,
                    None,
                    &mut Pass {
                        builder,
                        workspace,
                        config: c,
                        phase,
                    },
                )
                .await?
                .modified;
            // ZTM's PHP download omits Last-Modified. Its ZIP does contain UTC
            // GTFS table timestamps; use the validated calendar-pass manifest
            // for this same uncompressed snapshot, never the service date.
            let modified = modified.or_else(|| {
                builder
                    .manifest
                    .as_ref()
                    .and_then(gtfs::zip::Manifest::updated_at)
                    .filter(|stamp| (0..=builder.now).contains(stamp))
            });
            times.observe(modified);
        }
    }
    Ok(timetable_display::freshness::Freshness {
        checked_at: Some(builder.now),
        source_updated_at: times.updated_at(),
    })
}

/// Both paths stage results in Builder; only the caller publishes on success.
pub async fn refresh(
    http: &impl Http,
    builder: &mut Builder,
    workspace: &mut zip::Workspace,
    config: &Config,
    now: i64,
) -> Result<Freshness, Error> {
    builder.begin(now)?;
    if config.groups.iter().all(|g| g.picks.is_empty()) {
        return Ok(Freshness::default());
    }
    match mirror(http, builder, workspace, config).await {
        Ok(freshness) => Ok(freshness),
        Err(error @ (Error::CacheUpdateRequired(_) | Error::CacheUpdateDeclined)) => {
            builder.begin(now)?;
            Err(error)
        }
        Err(error) => {
            if http.feed_id() != "mdb-982" {
                builder.begin(now)?;
                return Err(error);
            }
            http.fallback(&error);
            builder.begin(now)?;
            official(http, builder, workspace, config).await
        }
    }
}
