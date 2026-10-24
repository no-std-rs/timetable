//! Query a memory-mapped original ZIP with the existing bounded Rust parser.
use embedded_io_async::{ErrorType, Read, Seek, SeekFrom};
use gtfs::{Error, Feed, Table, csv, zip};
use timetable_display::feed::{Tables, ZonedBuilder as Builder};

#[derive(Clone, Copy)]
pub struct SliceReader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> SliceReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
}
impl ErrorType for SliceReader<'_> {
    type Error = Error;
}
impl Read for SliceReader<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        let tail = self.bytes.get(self.position..).unwrap_or_default();
        let n = tail.len().min(out.len());
        out[..n].copy_from_slice(&tail[..n]);
        self.position += n;
        Ok(n)
    }
}
impl Seek for SliceReader<'_> {
    async fn seek(&mut self, from: SeekFrom) -> Result<u64, Error> {
        let n = match from {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => (self.position as u64)
                .checked_add_signed(n)
                .ok_or(Error::InvalidRange)?,
            SeekFrom::End(n) => (self.bytes.len() as u64)
                .checked_add_signed(n)
                .ok_or(Error::InvalidRange)?,
        };
        self.position = n.try_into().map_err(|_| Error::InvalidRange)?;
        Ok(n)
    }
}
struct Archive<'a>(Feed<SliceReader<'a>>);
impl Tables for Archive<'_> {
    async fn scan(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, Error> {
        let manifest = zip::Manifest::from_directory(self.0.directory());
        if let Some(expected) = expected {
            expected.check(&manifest)?;
        }
        for table in tables {
            if self.0.directory().member(*table).is_some() {
                self.0.scan(*table, workspace, &mut visit).await?;
            }
        }
        Ok(manifest)
    }
}

/// Nothing is published until ZIP/footer, CSV, identities and capacities pass.
/// The caller keeps the prior snapshot if this returns an error.
pub async fn query(
    settings: &[u8],
    archive: &[u8],
    now: i64,
    checked_at: i64,
    out: &mut [u8],
) -> Result<usize, &'static str> {
    let settings_value = crate::Settings::decode(settings)?;
    let zone = settings_value.timezone();
    let mut journeys = timetable_display::journey::Cache::default();
    let mut builder = Builder::new(
        zone.service_date(now)
            .ok_or("Clock is outside timezone coverage")?,
    );
    builder.zone = zone;
    builder.journeys = Some(&mut journeys);
    builder.begin(now).map_err(|_| "Invalid date")?;
    let config = settings_value.config();
    let mut workspace = zip::Workspace::new();
    // Validate the entire local-entry/directory/footer structure. Known-size
    // unselected members are skipped; selected member CRCs are verified below.
    let observed = gtfs::stream::scan(SliceReader::new(archive), &[], None, &mut workspace, |_| {
        Ok(())
    })
    .await
    .map_err(|_| "Invalid or unsupported timetable ZIP")?;
    let mut source = Archive(
        Feed::open(SliceReader::new(archive), &mut [0; 2048])
            .await
            .map_err(|_| "Invalid timetable directory")?,
    );
    observed
        .check(&zip::Manifest::from_directory(source.0.directory()))
        .map_err(|_| "Changed timetable snapshot")?;
    builder
        .calendars_from(&mut source, 7, &config, &mut workspace)
        .await
        .map_err(|_| "Timetable metadata does not fit or is invalid")?;
    builder
        .trips_from(&mut source, &config, &mut workspace)
        .await
        .map_err(|_| "Selected trips do not fit or are invalid")?;
    builder
        .departures_from(&mut source, &config, &mut workspace)
        .await
        .map_err(|_| "Departures do not fit or are invalid")?;
    builder
        .journeys_from(&mut source, 7, &config, &mut workspace)
        .await
        .map_err(
            |_| "Journey search exceeded device limits or the feed has unsupported transfer rules",
        )?;
    let mut app = timetable_display::App::default();
    app.publish(builder.events, builder.headsigns, now);
    app.journeys = core::mem::take(builder.journeys.as_deref_mut().unwrap());
    app.freshness.checked_at = Some(checked_at);
    app.freshness.source_updated_at = observed.updated_at();
    crate::snapshot::encode(settings, &app, out)
}

/// Only for immediately-ready local-memory I/O. Pending is an error, never a spin.
pub fn immediate<T>(future: impl core::future::Future<Output = T>) -> Result<T, &'static str> {
    let mut future = core::pin::pin!(future);
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        core::task::Poll::Ready(value) => Ok(value),
        core::task::Poll::Pending => Err("Unexpected asynchronous local I/O"),
    }
}
