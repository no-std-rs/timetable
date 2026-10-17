//! Runnable host example. For allocator-free firmware networking see embedded.rs.
mod support;

use gtfs::{
    Date, Error, Feed, Id, IdSet, Table, TripIndex, TripSlot, poznan,
    query::{DepartureBuffer, DepartureSlot},
    stream, zip,
};
use std::fs::File;
use support::{AnyError, Counting, FileIo, Http, member};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AnyError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut http = Http::new();
    match args.first().map(String::as_str) {
        Some("archives") => {
            let mut stream = http.connect(poznan::LIST_URL).await?;
            let (host, path) = support::endpoint(poznan::LIST_URL)?;
            let mut rx = [0; 4096];
            let response = support::get(
                &mut stream,
                host,
                path,
                &[("Accept-Encoding", "identity"), ("Connection", "close")],
                &mut rx,
            )
            .await?;
            if response.status.0 != 200 {
                return Err(format!("HTTP {}", response.status.0).into());
            }
            let body = Counting {
                source: response.body().reader(),
                bytes: &mut http.bytes,
            };
            let mut archives = poznan::Archives::new(body);
            while let Some(name) = archives.next().await? {
                println!("{}", name.as_str());
            }
        }
        Some("download") if args.len() == 3 => {
            let mut buf = [0; 192];
            let name: poznan::ArchiveName = args[1].parse()?;
            http.download(name.url(&mut buf)?, &args[2]).await?;
        }
        Some("mirror-snapshots") => {
            for snapshot in mirror_snapshots(&mut http).await? {
                println!("{}", snapshot.url());
            }
        }
        Some("remote") if args.len() >= 3 => {
            if args[1] == "latest" {
                let snapshots = mirror_snapshots(&mut http).await?;
                let snapshot = snapshots.first().ok_or("no mirror snapshot links found")?;
                eprintln!("Pinned mirror: {}", snapshot.url());
                remote(&mut http, snapshot.url(), &args[2..]).await?;
            } else {
                remote(&mut http, &args[1], &args[2..]).await?;
            }
        }
        Some("original") if args.len() >= 3 => original(&mut http, &args[1..]).await?,
        Some("fallback") if args.len() == 6 => fallback(&mut http, &args[1..]).await?,
        Some("routes" | "stops" | "departures") if args.len() >= 2 => local(&args).await?,
        _ => return Err(USAGE.into()),
    }
    if http.requests != 0 {
        eprintln!(
            "HTTP: {} requests; {} body bytes",
            http.requests, http.bytes
        );
    }
    Ok(())
}

const USAGE: &str = "usage:
  poznan archives
  poznan mirror-snapshots
  poznan download YYYYMMDD_YYYYMMDD.zip FILE
  poznan routes FILE
  poznan stops FILE ROUTE_ID
  poznan departures FILE ROUTE_ID YYYYMMDD STOP_ID
  poznan remote latest routes
  poznan remote IMMUTABLE_MIRROR_URL routes
  poznan remote IMMUTABLE_MIRROR_URL stops ROUTE_ID
  poznan remote IMMUTABLE_MIRROR_URL departures ROUTE_ID YYYYMMDD STOP_ID
  poznan original ARCHIVE_NAME routes
  poznan original ARCHIVE_NAME stops
  poznan original ARCHIVE_NAME departures ROUTE_ID YYYYMMDD STOP_ID
  poznan fallback MIRROR_URL ARCHIVE_NAME ROUTE_ID YYYYMMDD STOP_ID";

async fn mirror_snapshots(http: &mut Http) -> Result<Vec<poznan::MirrorSnapshot>, AnyError> {
    let mut stream = http.connect(poznan::MIRROR_PAGE).await?;
    let (host, path) = support::endpoint(poznan::MIRROR_PAGE)?;
    let mut rx = [0; 4096];
    let response = support::get(
        &mut stream,
        host,
        path,
        &[("Accept-Encoding", "identity"), ("Connection", "close")],
        &mut rx,
    )
    .await?;
    if response.status.0 != 200 {
        return Err(format!("HTTP {}", response.status.0).into());
    }
    let body = Counting {
        source: response.body().reader(),
        bytes: &mut http.bytes,
    };
    let mut listing = poznan::MirrorSnapshots::new(body);
    // Only the host CLI collects these; the library yields one at a time.
    let mut snapshots = Vec::new();
    while let Some(snapshot) = listing.next().await? {
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}

async fn local(args: &[String]) -> Result<(), AnyError> {
    let mut feed = Feed::open(FileIo(File::open(&args[1])?), &mut [0; 1024]).await?;
    let mut work = zip::Workspace::new();
    if args[0] == "routes" {
        feed.routes(&mut work, |r| {
            println!("{}\t{}\t{}", r.id, r.short_name, r.long_name);
            Ok(())
        })
        .await?;
        return Ok(());
    }
    let route = args.get(2).ok_or("missing ROUTE_ID (list routes first)")?;
    let mut service_storage = [Id::<32>::EMPTY; 128];
    let mut services = IdSet::new(&mut service_storage);
    let dated = args[0] == "departures";
    if dated {
        let date: Date = args.get(3).ok_or("missing YYYYMMDD")?.parse()?;
        feed.services_on(date, &mut services, &mut work).await?;
    }
    let mut trip_storage = [TripSlot::<32>::EMPTY; 2048];
    let mut trips = TripIndex::new(&mut trip_storage);
    feed.select_trips(
        &[route],
        None,
        dated.then_some(&services),
        &mut trips,
        &mut work,
    )
    .await?;
    eprintln!("Selected {} trips", trips.len());
    if dated {
        let stop = args.get(4).ok_or("missing STOP_ID (use stops first)")?;
        feed.departures(&trips, &[stop], &mut work, print_departure)
            .await?;
    } else {
        let mut storage = [Id::<32>::EMPTY; 512];
        let mut stops = IdSet::new(&mut storage);
        feed.route_stops(&trips, &mut stops, &mut work).await?;
        feed.named_stops(&stops, &mut work, |s| {
            println!(
                "{}\t{}\t{}",
                s.id,
                s.code.unwrap_or(""),
                s.name.unwrap_or("")
            );
            Ok(())
        })
        .await?;
    }
    Ok(())
}

async fn remote(http: &mut Http, url: &str, args: &[String]) -> Result<(), AnyError> {
    // Pin an immutable version URL. Do not combine byte ranges from a moving
    // "latest" endpoint. The example validates status, range and object length.
    let length = http.probe(url).await?;
    let dir = http.directory(url, length).await?;
    let mut work = zip::Workspace::new();
    if args[0] == "routes" {
        http.scan(url, length, member(&dir, Table::Routes)?, &mut work, |r| {
            let r = r.route()?;
            println!("{}\t{}\t{}", r.id, r.short_name, r.long_name);
            Ok(())
        })
        .await?;
        return Ok(());
    }
    let dated = match args[0].as_str() {
        "departures" => true,
        "stops" => false,
        _ => return Err("remote operation must be routes, stops or departures".into()),
    };
    let route = args.get(1).ok_or("missing ROUTE_ID")?;
    let mut service_storage = [Id::<32>::EMPTY; 128];
    let mut services = IdSet::new(&mut service_storage);
    let date = if dated {
        Some(args.get(2).ok_or("missing YYYYMMDD")?.parse()?)
    } else {
        None
    };
    let mut trip_storage = [TripSlot::<32>::EMPTY; 2048];
    let mut trips = TripIndex::new(&mut trip_storage);
    select_mirror(
        http,
        Mirror {
            url,
            length,
            directory: &dir,
        },
        stream::Selection {
            routes: &[route],
            direction: None,
            date,
        },
        &mut trips,
        &mut services,
        &mut work,
    )
    .await?;
    eprintln!("Selected {} trips", trips.len());
    if trips.is_empty() {
        return Ok(());
    }
    let mut storage = [Id::<32>::EMPTY; 512];
    let mut stops = IdSet::new(&mut storage);
    let stop = if dated {
        Some(args.get(3).ok_or("missing STOP_ID")?.as_str())
    } else {
        None
    };
    let mut cursor = trips.cursor()?;
    http.scan(
        url,
        length,
        member(&dir, Table::StopTimes)?,
        &mut work,
        |row| {
            if let Some(trip) = cursor.find(row.required(gtfs::csv::Column::TripId)?)? {
                let id = row.required(gtfs::csv::Column::StopId)?;
                if dated {
                    if stop == Some(id) {
                        print_departure(gtfs::query::Departure {
                            route_index: trip.route_index,
                            direction: trip.direction,
                            event: row.stop_time()?,
                        })?;
                    }
                } else {
                    stops.insert(id)?;
                }
            }
            Ok(())
        },
    )
    .await?;
    if !dated {
        http.scan(url, length, member(&dir, Table::Stops)?, &mut work, |row| {
            if stops.contains(row.required(gtfs::csv::Column::StopId)?) {
                let s = row.stop()?;
                println!(
                    "{}\t{}\t{}",
                    s.id,
                    s.code.unwrap_or(""),
                    s.name.unwrap_or("")
                );
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

struct Mirror<'a> {
    url: &'a str,
    length: u64,
    directory: &'a zip::Directory,
}

async fn select_mirror(
    http: &mut Http,
    source: Mirror<'_>,
    selection: stream::Selection<'_>,
    trips: &mut TripIndex<'_, 32>,
    services: &mut IdSet<'_, 32>,
    workspace: &mut zip::Workspace,
) -> Result<zip::Manifest, AnyError> {
    services.clear();
    trips.clear();
    if let Some(date) = selection.date {
        if source.directory.member(Table::Calendar).is_none()
            && source.directory.member(Table::CalendarDates).is_none()
        {
            return Err(Error::MissingTable(Table::Calendar).into());
        }
        if let Some(m) = source.directory.member(Table::Calendar) {
            http.scan(source.url, source.length, m, workspace, |row| {
                let c = row.calendar()?;
                if c.runs_on(date) {
                    services.insert(c.service_id)?;
                }
                Ok(())
            })
            .await?;
        }
        if let Some(m) = source.directory.member(Table::CalendarDates) {
            http.scan(source.url, source.length, m, workspace, |row| {
                let c = row.calendar_date()?;
                if c.date == date {
                    if c.added {
                        services.insert(c.service_id)?;
                    } else {
                        services.remove(c.service_id);
                    }
                }
                Ok(())
            })
            .await?;
        }
    }
    http.scan(
        source.url,
        source.length,
        member(source.directory, Table::Trips)?,
        workspace,
        |row| {
            trips.consider(
                row.trip()?,
                selection.routes,
                selection.direction,
                selection.date.map(|_| &*services),
            )
        },
    )
    .await?;
    trips.finish()?;
    Ok(zip::Manifest::from_directory(source.directory))
}

async fn original(http: &mut Http, args: &[String]) -> Result<(), AnyError> {
    let archive: poznan::ArchiveName = args[0].parse()?;
    let mut url = [0; 192];
    let url = archive.url(&mut url)?;
    let mut work = zip::Workspace::new();
    if args[1] == "routes" || args[1] == "stops" {
        let table = if args[1] == "routes" {
            Table::Routes
        } else {
            Table::Stops
        };
        http.scan_original(url, &[table], &mut work, |row| {
            if table == Table::Routes {
                let r = row.route()?;
                println!("{}\t{}\t{}", r.id, r.short_name, r.long_name);
            } else {
                let s = row.stop()?;
                println!(
                    "{}\t{}\t{}",
                    s.id,
                    s.code.unwrap_or(""),
                    s.name.unwrap_or("")
                );
            }
            Ok(())
        })
        .await?;
        return Ok(());
    }
    if args[1] != "departures" || args.len() != 5 {
        return Err(USAGE.into());
    }
    let routes = [args[2].as_str()];
    let stops = [args[4].as_str()];
    let selection = stream::Selection {
        routes: &routes,
        direction: None,
        date: Some(args[3].parse()?),
    };
    let mut trip_slots = [TripSlot::<32>::EMPTY; 2048];
    let mut trips = TripIndex::new(&mut trip_slots);
    let mut service_slots = [stream::ServiceSlot::<32>::EMPTY; 128];
    let mut services = stream::Services::new(&mut service_slots);
    let mut events = [DepartureSlot::<32>::EMPTY; 256];
    let mut output = DepartureBuffer::new(&mut events);
    let begin = std::time::Instant::now();
    let manifest = http
        .prepare_original(url, selection, &mut trips, &mut services, &mut work)
        .await?;
    eprintln!("Prepared {} trips from original ZIP", trips.len());
    http.collect_original(url, &manifest, &trips, &stops, &mut work, &mut output)
        .await?;
    eprintln!("Official streaming: {:.3} s", begin.elapsed().as_secs_f64());
    print_buffer(&mut output);
    Ok(())
}

async fn fallback(http: &mut Http, args: &[String]) -> Result<(), AnyError> {
    let archive: poznan::ArchiveName = args[1].parse()?;
    let mut original_url = [0; 192];
    let original_url = archive.url(&mut original_url)?;
    let routes = [args[2].as_str()];
    let stops = [args[4].as_str()];
    let selection = stream::Selection {
        routes: &routes,
        direction: None,
        date: Some(args[3].parse()?),
    };
    let mut work = zip::Workspace::new();
    let mut trip_slots = [TripSlot::<32>::EMPTY; 2048];
    let mut trips = TripIndex::new(&mut trip_slots);
    let mut events = [DepartureSlot::<32>::EMPTY; 256];
    let mut output = DepartureBuffer::new(&mut events);
    let begin = std::time::Instant::now();
    let cached = async {
        let length = http.probe(&args[0]).await?;
        let directory = http.directory(&args[0], length).await?;
        let mut service_slots = [Id::<32>::EMPTY; 128];
        let mut services = IdSet::new(&mut service_slots);
        select_mirror(
            http,
            Mirror {
                url: &args[0],
                length,
                directory: &directory,
            },
            selection,
            &mut trips,
            &mut services,
            &mut work,
        )
        .await
    }
    .await;
    match cached {
        Ok(manifest) => match http
            .collect_original(
                original_url,
                &manifest,
                &trips,
                &stops,
                &mut work,
                &mut output,
            )
            .await
        {
            Ok(()) => {
                eprintln!(
                    "Used mirror metadata + one original ZIP stream ({:.3} s)",
                    begin.elapsed().as_secs_f64()
                );
                print_buffer(&mut output);
                return Ok(());
            }
            Err(error)
                if matches!(
                    error.downcast_ref::<Error>(),
                    Some(Error::SnapshotMismatch(_))
                ) =>
            {
                eprintln!("Cached snapshot differs; rebuilding from the pinned original")
            }
            Err(error) => return Err(error),
        },
        Err(error)
            if matches!(
                error.downcast_ref::<Error>(),
                Some(Error::IndexFull | Error::IdTooLong)
            ) =>
        {
            return Err(error);
        }
        Err(error) => eprintln!("Mirror unavailable ({error}); using original-only bootstrap"),
    }
    let mut service_slots = [stream::ServiceSlot::<32>::EMPTY; 128];
    let mut services = stream::Services::new(&mut service_slots);
    let manifest = http
        .prepare_original(
            original_url,
            selection,
            &mut trips,
            &mut services,
            &mut work,
        )
        .await?;
    http.collect_original(
        original_url,
        &manifest,
        &trips,
        &stops,
        &mut work,
        &mut output,
    )
    .await?;
    eprintln!(
        "Official-only fallback ({:.3} s)",
        begin.elapsed().as_secs_f64()
    );
    print_buffer(&mut output);
    Ok(())
}

fn print_buffer(output: &mut DepartureBuffer<'_, 32>) {
    output.sort();
    for d in output.as_slice() {
        if let Some(time) = d.departure {
            println!(
                "{}\t{}\tdirection={:?}\tpickup={}",
                time,
                d.trip_id.as_str(),
                d.direction,
                d.pickup_type
            );
        }
    }
}

fn print_departure(d: gtfs::query::Departure<'_>) -> Result<(), Error> {
    // GTFS pickup_type=1 means no boarding. Keep the restriction visible here.
    if let Some(time) = d.event.departure {
        println!(
            "{}\t{}\tdirection={:?}\tpickup={}",
            time, d.event.trip_id, d.direction, d.event.pickup_type
        );
    }
    Ok(())
}
