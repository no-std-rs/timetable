#![no_std]
//! Compile this complete allocator-free workflow for the device ABI. LLVM folds
//! size_of_val into a constant; check_ram.py reads it without running MCU code.
use gtfs::{
    Date, Error, TripIndex, TripSlot, memory,
    query::{DepartureBuffer, DepartureSlot},
    stream, zip,
};

pub async fn workflow<const TRIPS: usize>(metadata: &[u8], data: &[u8]) -> Result<usize, Error> {
    let mut workspace = zip::Workspace::new();
    let mut trip_slots = [TripSlot::<32>::EMPTY; TRIPS];
    let mut trips = TripIndex::new(&mut trip_slots);
    let mut service_slots = [stream::ServiceSlot::<32>::EMPTY; 128];
    let mut services = stream::Services::new(&mut service_slots);
    let mut event_slots = [DepartureSlot::<32>::EMPTY; 256];
    let mut output = DepartureBuffer::new(&mut event_slots);
    let selection = stream::Selection {
        routes: &["4"],
        date: Some(Date::new(2026, 10, 17)?),
        direction: None,
    };
    let manifest = stream::prepare(
        metadata,
        selection,
        &mut trips,
        &mut services,
        &mut workspace,
    )
    .await?;
    stream::collect(
        data,
        &manifest,
        &trips,
        &["129"],
        &mut workspace,
        &mut output,
    )
    .await?;
    output.sort();
    Ok(output.len())
}

#[inline(never)]
pub fn task_state_bytes_1024() -> usize {
    core::mem::size_of_val(&workflow::<1024>(&[], &[]))
}
#[inline(never)]
pub fn task_state_bytes_2048() -> usize {
    core::mem::size_of_val(&workflow::<2048>(&[], &[]))
}
#[inline(never)]
pub fn task_state_bytes_4096() -> usize {
    core::mem::size_of_val(&workflow::<4096>(&[], &[]))
}
#[inline(never)]
pub fn reserves_bytes() -> usize {
    let r = memory::Reserves::EMBEDDED;
    r.tls_rx + r.tls_tx + r.http + r.network + r.call_stack + r.application
}

/// Own the display query's workspace, settings and builder across every await.
/// The visible App coexists with the pending update and is counted separately.
pub async fn display_workflow(source: &[u8]) -> Result<(), Error> {
    let mut workspace = zip::Workspace::new();
    let mut journeys = timetable_display::journey::Cache::default();
    let mut builder = timetable_display::feed::ZonedBuilder::new(Date::new(2026, 10, 17)?);
    let (config, _) = {
        let mut image = *include_bytes!("../../../display/tests/settings.bin");
        timetable_display::config::decode(&mut image)
            .map_err(|_| Error::InvalidField("settings"))?
    };
    builder.journeys = Some(&mut journeys);
    builder.begin(1792260000)?;
    builder
        .calendars(source, 7, &config, &mut workspace)
        .await?;
    builder.trips(source, &config, &mut workspace).await?;
    builder.departures(source, &config, &mut workspace).await?;
    builder
        .journeys_from(&mut JourneySource(source), 7, &config, &mut workspace)
        .await?;
    core::hint::black_box(&builder.events);
    Ok(())
}

#[inline(never)]
pub fn display_task_bytes() -> usize {
    core::mem::size_of_val(&display_workflow(&[]))
}

#[inline(never)]
pub fn display_departure_cache_bytes() -> usize {
    core::mem::size_of::<timetable_display::departures::Departures>()
}

#[inline(never)]
pub fn display_reserves_bytes() -> usize {
    // Explicit allowances: radio/RSA heap, network/driver state, TLS records,
    // HTTP headers, 24 KiB stack, shared tile/settings scratch and retained UI.
    65536
        + 32768
        + 16640
        + 2304
        + 2048
        + 24576
        + core::mem::size_of::<timetable_display::App>()
        + core::mem::size_of::<timetable_display::Config>()
        + 8192 // shared settings flash image / panel tile (never concurrent)
}

struct JourneySource<'a>(&'a [u8]);
impl timetable_display::feed::Tables for JourneySource<'_> {
    async fn scan(
        &mut self,
        tables: &[gtfs::Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        visit: impl FnMut(gtfs::csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, Error> {
        stream::scan(self.0, tables, expected, workspace, visit).await
    }
}
