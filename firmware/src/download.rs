//! Shared, bounded download pipeline used by the device and host diagnostic.
use crate::sources::Archives;
use embedded_io_async::Read;
use gtfs::{
    Date, Table, csv,
    http::{ByteRange, RangeFetch, RangeReader},
    poznan::MirrorSnapshot,
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
#[derive(Default)]
pub struct Metadata {
    pub length: Option<u64>,
    pub modified: Option<i64>,
}
#[allow(async_fn_in_trait)]
pub trait Http {
    /// A range must be exact, HTTP 206, identity encoded and from the pinned
    /// object length. Full requests require HTTP 200. Reject duplicate headers.
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error>;
    fn stage(&self, _message: &str) {}
    fn fallback(&self, _error: &Error) {
        self.stage("MobilityDatabase failed; using official ZTM fallback");
    }
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
impl Operation for Snapshots {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error> {
        let mut listing = gtfs::poznan::MirrorSnapshots::new(body);
        while let Some(snapshot) = listing.next().await? {
            if self.0.contains(&snapshot) {
                continue;
            }
            if self.0.is_full() {
                if self.0.last().unwrap().url() >= snapshot.url() {
                    continue;
                }
                self.0.pop();
            }
            self.0.push(snapshot).unwrap();
            self.0.sort_unstable_by(|a, b| b.url().cmp(a.url()));
        }
        if self.0.is_empty() {
            return Err(Error::MissingArchive);
        }
        Ok(())
    }
}
struct Range<'a, H> {
    http: &'a H,
    url: &'a str,
    length: u64,
}
impl<H: Http> RangeFetch for Range<'_, H> {
    async fn fetch(&mut self, range: ByteRange, out: &mut [u8]) -> Result<(), gtfs::Error> {
        self.http
            .fetch(self.url, Some((range, Some(self.length))), &mut Fill(out))
            .await
            .map_err(Error::feed)?;
        Ok(())
    }
}
struct Payload<'a, V> {
    member: zip::Member,
    workspace: &'a mut zip::Workspace,
    visit: V,
}
impl<V: FnMut(csv::Record<'_>) -> Result<(), gtfs::Error>> Operation for Payload<'_, V> {
    async fn read<R: Read>(&mut self, mut body: R) -> Result<(), Error> {
        {
            let zip = zip::Reader::data(&mut body, self.workspace, self.member);
            let mut rows = csv::Reader::<_>::new(zip, self.member.table());
            while let Some(row) = rows.next().await? {
                (self.visit)(row)?;
            }
        }
        end(&mut body).await
    }
}
pub struct Mirror<'a, H> {
    http: &'a H,
    snapshot: MirrorSnapshot,
    length: u64,
    directory: zip::Directory,
}
impl<'a, H: Http> Mirror<'a, H> {
    pub async fn open(
        http: &'a H,
        snapshot: MirrorSnapshot,
        workspace: &mut zip::Workspace,
    ) -> Result<Self, Error> {
        let meta = http
            .fetch(
                snapshot.url(),
                Some((ByteRange::new(0, 0)?, None)),
                &mut Fill(&mut [0]),
            )
            .await?;
        let length = meta.length.ok_or(Error::Http)?;
        let fetcher = Range {
            http,
            url: snapshot.url(),
            length,
        };
        let (cache, scratch) = workspace.scratch()[..3072].split_at_mut(2048);
        let mut reader = RangeReader::new(fetcher, length, cache)?;
        let directory = zip::Directory::read(&mut reader, scratch).await?;
        Ok(Self {
            http,
            snapshot,
            length,
            directory,
        })
    }
    async fn table(
        &mut self,
        table: Table,
        workspace: &mut zip::Workspace,
        visit: impl FnMut(csv::Record<'_>) -> Result<(), gtfs::Error>,
    ) -> Result<(), Error> {
        let member = self
            .directory
            .member(table)
            .ok_or(gtfs::Error::MissingTable(table))?;
        self.http.stage(table.filename());
        let mut header = [0; 30];
        self.http
            .fetch(
                self.snapshot.url(),
                Some((member.header_range(), Some(self.length))),
                &mut Fill(&mut header),
            )
            .await?;
        let range = member.data_range(&header)?;
        self.http
            .fetch(
                self.snapshot.url(),
                Some((range, Some(self.length))),
                &mut Payload {
                    member,
                    workspace,
                    visit,
                },
            )
            .await?;
        Ok(())
    }
    pub async fn coverage(
        &mut self,
        dates: [Date; 3],
        workspace: &mut zip::Workspace,
    ) -> Result<u8, Error> {
        let mut mask = None;
        self.table(Table::FeedInfo, workspace, |row| {
            let start: Date = row.required(csv::Column::FeedStartDate)?.parse()?;
            let end: Date = row.required(csv::Column::FeedEndDate)?.parse()?;
            if start > end || mask.is_some() {
                return Err(gtfs::Error::InvalidDate);
            }
            mask = Some(dates.iter().enumerate().fold(0, |m, (i, d)| {
                m | if start <= *d && *d <= end { 1 << i } else { 0 }
            }));
            Ok(())
        })
        .await?;
        mask.ok_or(Error::MissingArchive)
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
    http.fetch(gtfs::poznan::MIRROR_PAGE, None, &mut candidates)
        .await?;
    let mut remaining = builder.day_mask();
    let mut times = timetable_display::freshness::SourceTimes::default();
    for snapshot in candidates.0 {
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
        Err(error) => {
            http.fallback(&error);
            builder.begin(now)?;
            official(http, builder, workspace, config).await
        }
    }
}
