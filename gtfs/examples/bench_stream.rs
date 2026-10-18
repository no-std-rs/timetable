//! Repeatable local-file streaming/allocator/RAM benchmark. Network is measured
//! separately by the reqwless example. All files are opened before allocation tracking.
use embedded_io_async::{ErrorType, Read};
use futures::executor::block_on;
use gtfs::{
    Error, TripIndex, TripSlot, memory,
    query::{DepartureBuffer, DepartureSlot},
    stream, zip,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{alloc::System, fs::File, io, mem::size_of_val, time::Instant};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

struct ForwardFile {
    file: File,
    bytes: u64,
}
impl ForwardFile {
    fn open(path: &str) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            bytes: 0,
        })
    }
}
impl ErrorType for ForwardFile {
    type Error = io::Error;
}
impl Read for ForwardFile {
    async fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = io::Read::read(&mut self.file, out)?;
        self.bytes += n as u64;
        Ok(n)
    }
}

struct Metrics {
    prepare_ms: f64,
    query_ms: f64,
    trips: usize,
    services: usize,
    events: usize,
    checksum: u64,
    prepare_future: usize,
    query_future: usize,
    source_updated_at: Option<i64>,
}

async fn workflow<const TRIPS: usize>(
    first: &mut ForwardFile,
    second: &mut ForwardFile,
    selection: stream::Selection<'_>,
    stops: &[&str],
) -> Result<Metrics, Error> {
    let mut workspace = zip::Workspace::new();
    let mut trip_slots = [TripSlot::<32>::EMPTY; TRIPS];
    let mut trips = TripIndex::new(&mut trip_slots);
    let mut service_slots = [stream::ServiceSlot::<32>::EMPTY; 128];
    let mut services = stream::Services::new(&mut service_slots);
    let mut event_slots = [DepartureSlot::<32>::EMPTY; 256];
    let mut output = DepartureBuffer::new(&mut event_slots);
    let begin = Instant::now();
    let prepare = stream::prepare(first, selection, &mut trips, &mut services, &mut workspace);
    let prepare_future = size_of_val(&prepare);
    let manifest = prepare.await?;
    let prepare_ms = begin.elapsed().as_secs_f64() * 1000.0;
    let begin = Instant::now();
    let query = stream::collect(
        second,
        &manifest,
        &trips,
        stops,
        &mut workspace,
        &mut output,
    );
    let query_future = size_of_val(&query);
    query.await?;
    let query_ms = begin.elapsed().as_secs_f64() * 1000.0;
    output.sort();
    let mut checksum = 0xcbf29ce484222325u64;
    for event in output.as_slice() {
        for b in event
            .trip_id
            .as_str()
            .bytes()
            .chain(event.stop_id.as_str().bytes())
            .chain(
                event
                    .departure
                    .map(|t| t.seconds())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            )
            .chain(event.sequence.to_le_bytes())
        {
            checksum = (checksum ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    Ok(Metrics {
        prepare_ms,
        query_ms,
        trips: trips.len(),
        services: services.len(),
        events: output.len(),
        checksum,
        prepare_future,
        query_future,
        source_updated_at: manifest.updated_at(),
    })
}

fn bench<const TRIPS: usize>(
    path: &str,
    selection: stream::Selection<'_>,
    stops: &[&str],
    rounds: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    block_on(async {}); // Warm the executor's thread-local bookkeeping.
    let mut results = Vec::with_capacity(rounds);
    println!(
        "round,prepare_ms,query_ms,prepare_bytes,query_bytes,trips,services,events,allocations,reallocations,allocated_bytes,task_bytes,reserve_bytes,budget_bytes,remaining_bytes,checksum"
    );
    for round in 0..=rounds {
        let mut first = ForwardFile::open(path)?;
        let mut second = ForwardFile::open(path)?;
        let future = workflow::<TRIPS>(&mut first, &mut second, selection, stops);
        let usage =
            memory::Usage::check(&future, 0, memory::Reserves::EMBEDDED, memory::DEVICE_RAM)?;
        let region = Region::new(GLOBAL);
        let result = block_on(future);
        let stats = region.change();
        let result = result?;
        assert_eq!(stats.allocations, 0, "streaming allocated");
        assert_eq!(stats.reallocations, 0, "streaming reallocated");
        assert_eq!(stats.bytes_allocated, 0, "streaming heap bytes");
        if round == 0 {
            eprintln!(
                "warmup: task={} B + firmware reserves={} B = {} / {} B; nested prepare/query frames={}/{} B",
                usage.task_state,
                usage.reserves,
                usage.total,
                memory::DEVICE_RAM,
                result.prepare_future,
                result.query_future
            );
            eprintln!(
                "source table modification time (UTC Unix seconds): {:?}",
                result.source_updated_at
            );
            continue;
        }
        println!(
            "{round},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{},{},{},{:016x}",
            result.prepare_ms,
            result.query_ms,
            first.bytes,
            second.bytes,
            result.trips,
            result.services,
            result.events,
            stats.allocations,
            stats.reallocations,
            stats.bytes_allocated,
            usage.task_state,
            usage.reserves,
            usage.total,
            usage.remaining,
            result.checksum
        );
        results.push((
            result.prepare_ms,
            result.query_ms,
            result.prepare_ms + result.query_ms,
        ));
    }
    fn median(v: &mut [f64]) -> f64 {
        v.sort_unstable_by(f64::total_cmp);
        let n = v.len();
        if n.is_multiple_of(2) {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        } else {
            v[n / 2]
        }
    }
    let mut prepare: Vec<_> = results.iter().map(|r| r.0).collect();
    let mut query: Vec<_> = results.iter().map(|r| r.1).collect();
    let mut total: Vec<_> = results.iter().map(|r| r.2).collect();
    eprintln!(
        "median ({rounds} rounds after warmup): metadata={:.3} ms; cached-metadata original query={:.3} ms; official-only total={:.3} ms",
        median(&mut prepare),
        median(&mut query),
        median(&mut total)
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        return Err(
            "usage: bench_stream ZIP ROUTE_ID YYYYMMDD STOP_ID [ROUNDS=7] [TRIP_CAPACITY=2048]"
                .into(),
        );
    }
    let routes = [args[1].as_str()];
    let stops = [args[3].as_str()];
    let selection = stream::Selection {
        routes: &routes,
        direction: None,
        date: Some(args[2].parse()?),
    };
    let rounds = args
        .get(4)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(7usize);
    if rounds == 0 {
        return Err("rounds must be positive".into());
    }
    match args.get(5).map(String::as_str).unwrap_or("2048") {
        "1024" => bench::<1024>(&args[0], selection, &stops, rounds),
        "2048" => bench::<2048>(&args[0], selection, &stops, rounds),
        "4096" => bench::<4096>(&args[0], selection, &stops, rounds),
        _ => Err("supported capacities: 1024, 2048, 4096".into()),
    }
}
