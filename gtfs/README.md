# gtfs

An allocation-free `#![no_std]` Rust crate in the `timetable` workspace for streaming GTFS timetable ZIPs.
Poznań is the first verified official provider. The ZIP, CSV and query layers
work with other cities' standard GTFS feeds; provider discovery currently covers
Poznań only. This is an initial implementation, not a complete GTFS validator.

The library has no `std` or `alloc` feature, no executor, and no HTTP dependency.
It takes `embedded_io_async` readers, seekable storage and caller-owned buffers.
The examples use **reqwless 0.14** for HTTP. The runnable host example uses
Tokio/Rustls for verified TLS; those dependencies are development-only.

## Run it

From the `timetable` workspace root (the `main/` worktree):

```sh
# Small range requests to a version-pinned MobilityDatabase snapshot.
cargo run -p gtfs --release --example poznan -- remote latest routes

# Discover official archives and immutable mirror snapshots.
cargo run -p gtfs --release --example poznan -- archives
cargo run -p gtfs --release --example poznan -- mirror-snapshots

# Download one official archive, then reuse it without further network traffic.
cargo run -p gtfs --release --example poznan -- download 20261015_20261031.zip poznan.zip
cargo run -p gtfs --release --example poznan -- routes poznan.zip
cargo run -p gtfs --release --example poznan -- stops poznan.zip 4
cargo run -p gtfs --release --example poznan -- departures poznan.zip 4 20261017 129

# No ZIP cache or mirror required: two forward-only official HTTP streams.
cargo run -p gtfs --release --example poznan -- original 20261015_20261031.zip departures 4 20261017 129
```

The dates above are a reproducible example, not a promise that historical files
remain available. List archives first. `latest` can contain **future service**;
the mirror timestamp is its fetch date, not the timetable's start date. Pick the
snapshot covering your service day. Stop and route IDs are exact GTFS IDs, which
can differ from displayed line numbers or stop names.

For remote stop and departure queries, replace `latest` with a URL returned by
`mirror-snapshots`:

```text
poznan remote IMMUTABLE_SNAPSHOT_URL stops ROUTE_ID
poznan remote IMMUTABLE_SNAPSHOT_URL departures ROUTE_ID YYYYMMDD STOP_ID
```

`stops` lists the union of platforms across both directions and all trip variants,
including depot journeys. Each journey has its own ordered path: use
`StopTime.trip_id` and `sequence`. A route does not necessarily have one canonical
stop sequence. `departures` emits feed order, preserves pickup restrictions and
does not sort or deduplicate events.

## Efficient downloads

[ZTM Poznań's official listing](https://www.ztm.poznan.pl/otwarte-dane/gtfsfiles/)
publishes dated ZIPs through `getGTFSFile?file=YYYYMMDD_YYYYMMDD.zip`. Omitting
`file` selects the publisher's latest archive. `poznan::Archives` discovers names
from the listing a chunk at a time, without loading the HTML into memory.

[MobilityDatabase feed mdb-982](https://mobilitydatabase.org/feeds/gtfs/mdb-982)
mirrors the official ZIPs at immutable `files.mobilitydatabase.org` URLs.
`poznan::MirrorSnapshots` streams those links from the mirror page. Both listing
parsers depend on the publishers' current HTML and filename formats; a page
change can require updating discovery. Explicit pinned URLs remain usable.

Measured on **2026-10-17** using actual reqwless requests:

| Operation | HTTP requests | Downloaded body bytes |
| --- | ---: | ---: |
| Official archive `20261015_20261031.zip` | 1 | 12,392,540 |
| All routes from mirror snapshot `202610170128` | 4 | 23,010 |
| Line 4 / stop 129 / service date 20261017, snapshot `202610150044` | 10 | 8,153,098 |

Counts exclude TLS and HTTP headers, and the mirror listing fetch when using
`latest`. The route query returned 213 routes. The dated query returned 63 events,
matching an independent Python GTFS join. These are observations for those
snapshots, not fixed performance guarantees.

The official endpoint ignored `Range: bytes=0-31` and returned the whole archive
with HTTP 200. The mirror returned HTTP 206 with exactly 32 bytes. The remote
example probes range support, reads the ZIP central directory with a small cache,
then makes one header request and **one streaming payload request per table**.
It validates status, Content-Range, identity encoding, object length and member
CRC/size. Range requests use the same immutable snapshot throughout; mutable
URLs need ETag/If-Match protection in the transport implementation.

ZIP entries are compressed independently, but rows inside `stop_times.txt` are
not. A mirror can save downloading `shapes.txt` and unrelated members. It cannot
serve only the rows for one line or stop using ordinary HTTP ranges. An initial
departure query still downloads/inflates that whole member, retaining only
selected trip IDs and emitting only matching stop events. To make later queries
cheap, persist emitted events for your selected stops during a feed update.
A row-level download API would require a derived, indexed mirror.

## Direct official streams and mirror independence

Keep `zip::Manifest::from_directory(...)` and the selected `TripIndex` after
mirror bootstrap. `stream::collect(original_http_body, &manifest, &trips, stops,
workspace, output)` then parses the original ZIP in **one forward-only pass**.
It needs `Read`, never `Seek`, and never retains the ZIP in RAM or a file.
`stream::scan` can also stream route/stop metadata straight from the original.
Pass `Table::Routes` and `Table::Stops`, then show rows in the UI or store them in
flash. Large catalogues should stay in flash; keep selected IDs in RAM.

Trip IDs can change between snapshots. The manifest checks the uncompressed
CRC32 and size of each known GTFS table, so a mirror may recompress the same
content without breaking compatibility. A different snapshot returns
`SnapshotMismatch`; CRC32 is a version/corruption check, not authentication.
Verified TLS supplies transport authentication.

Without matching metadata, `stream::prepare` takes a **first original pass** to
build only selected routes' trips, service weekdays and exceptions. A **second
original pass** supplies the filtered departures. Calendars, exceptions and
trips may appear in any ZIP member order. Use the same dated URL for both passes;
the second pass rejects a changed archive. There is no mirror dependency in this
path, and no need for the original server to support byte ranges.

```text
# Bootstrap from the mirror, then query the original ZIP directly.
poznan fallback IMMUTABLE_MIRROR_URL 20261015_20261031.zip 4 20261017 129

# Never contact the mirror.
poznan original 20261015_20261031.zip departures 4 20261017 129
poznan original 20261015_20261031.zip routes
poznan original 20261015_20261031.zip stops
```

`fallback` rebuilds from two original streams if mirror bootstrap is unavailable
or its snapshot differs. `original ... stops` lists all platform/station metadata;
line-specific stop unions use the prepared trip index and stop-time events.
Library callers can retain the prepared index and manifest for later refreshes
without contacting the mirror again. No archive or metadata persistence format
is imposed on the application's flash/NVS storage.

`query::DepartureBuffer` stores a bounded selected board, which can be sorted and
read repeatedly without rescanning the ZIP. `stream::collect` clears provisional
events on CRC/footer errors, snapshot mismatch, overflow or cancellation. For
larger selections, stream events to a staged flash transaction using
`stream::departures`; commit only after success. Known-size shapes are skipped
without inflation. The central directory, end record, ZIP comment and HTTP body
EOF are checked before success.

The official-only metadata pass must fit selected routes' trips across **all
service days** until every calendar has been read; date filtering follows that
pass. Overflow returns `IndexFull`. Multi-route selections use the same API.
`TripIndex::cursor` caches consecutive matches and misses, avoiding repeated
binary searches through grouped stop-time rows; ungrouped feeds remain correct.

For the official endpoint, download a pinned archive once to flash/SD and reuse
`Feed` over `Read + Seek`. `Feed` seeks directly to required members, so it never
inflates shapes. The actual Poznań archive places `stop_times.txt` before
`trips.txt`; an ordinary forward-only pass cannot join them without retained
state, caching or a second download.

## Embedded API

```rust
use embedded_io_async::{Read, Seek};
use gtfs::{Error, Feed, Id, IdSet, TripIndex, TripSlot, zip};

async fn selected_departures<R: Read + Seek>(flash: R) -> Result<(), Error> {
    let mut feed = Feed::open(flash, &mut [0; 1024]).await?;
    let mut workspace = zip::Workspace::new();

    // Stream all routes; no collection is built by the library.
    feed.routes(&mut workspace, |route| {
        let _ = (route.id, route.short_name, route.long_name);
        Ok(())
    }).await?;

    let mut service_slots = [Id::<32>::EMPTY; 128];
    let mut services = IdSet::new(&mut service_slots);
    feed.services_on("20261017".parse()?, &mut services, &mut workspace).await?;

    let mut trip_slots = [TripSlot::<32>::EMPTY; 1024];
    let mut trips = TripIndex::new(&mut trip_slots);
    feed.select_trips(&["4"], None, Some(&services), &mut trips, &mut workspace).await?;

    feed.departures(&trips, &["129"], &mut workspace, |departure| {
        // Write to your bounded display queue or persistent timetable cache.
        // departure.event.departure is optional; times may exceed 24:00:00.
        let _ = departure;
        Ok(())
    }).await?;
    Ok(())
}
```

`services_on` applies weekly calendars and added/removed service exceptions.
Calendar-dates-only feeds are supported. Pass `services=None` to `select_trips`
to enumerate every service-day variant; because the service-ID bound is generic,
use `select_trips::<32, 32>(...)` when passing a bare `None`. `direction=None`
selects both directions. `route_stops` builds a bounded union of stop IDs;
`named_stops` resolves their names and coordinates from `stops.txt`. Multi-line
queries take a slice of route IDs and report the matching `route_index`.

See [examples/embedded.rs](examples/embedded.rs) for a board-independent
`HttpClient<TcpConnect, Dns>` download function and direct response-body route
extraction. Configure your board's reqwless client with verified TLS and enable
its TLS feature in your firmware's dependencies. Its HTTP, TLS and copy buffers
are supplied by the caller; the library does not provide a board network stack.
The host CLI instead gives reqwless an already verified Rustls stream and uses
`Request`/`Response` directly.
`examples/embedded.rs::collect_official` demonstrates the single-pass fallback
with a configured board `HttpClient` and a bounded departure buffer.

For your own range transport, implement `http::RangeFetch`. Use `RangeReader`
only to inspect `zip::Directory`; then fetch `Member::header_range()`, derive
`Member::data_range()`, and pass that payload stream to `zip::Reader::data` and
`csv::Reader`. Reuse `TripIndex::consider` for streaming trip selection. The host
example implements this complete flow in [examples/support/mod.rs](examples/support/mod.rs).

## Memory and supported formats

Memory use is independent of archive size. On the tested 64-bit host,
`zip::Workspace` is 43,272 bytes (including the required 32 KiB DEFLATE dictionary),
the default CSV reader is about 10 KiB, and each `TripSlot<32>` is 56 bytes.
Additional ZIP input buffering is 2 KiB. Index storage scales with selected
trips/stops, not all city trips. Use `core::mem::size_of` on your own target.
Firmware should place large buffers/indexes in suitably sized task/static
storage rather than assuming a small stack can hold them. All buffers and
indexes remain caller-owned; no hidden heap is used.

Before publishing a downloaded ZIP cache, reopen the provisional storage and run
`zip::verify` with a workspace, nonempty scratch buffer and one caller-owned
`zip::VerificationSlot` per archive entry. It verifies every payload and matches
directory names, flags, methods, sizes, CRCs and offsets to the local entries;
insufficient slot capacity returns `IndexFull`.
The host download example supplies 64 slots. `Feed::open` and `Directory::read`
index the directory and do not verify payloads. Commit storage only after full
verification succeeds and the returned manifest contains routes, stops, trips,
stop times and at least one of `calendar.txt` or `calendar_dates.txt`. ZIP
verification checks archive integrity; it does not validate GTFS completeness.

The examples now default to **2,048 trip slots, 128 service slots and 256 cached
events**. The compiler-derived 32-bit RISC-V baseline plus explicit firmware
reserves is about **317 KB of a 400,000-byte budget**. A 4,096-trip profile exceeds
that budget. The host benchmark instruments the allocator and requires zero
allocations and reallocations during both passes.

See [PERFORMANCE.md](../PERFORMANCE.md) for raw timings, ABI-specific task sizes,
reserved TLS/network/stack memory and reproduction commands. These budgets
include owned async task state, not just the DEFLATE dictionary. Network buffers,
call stack and application reserves must be checked against the actual board;
host timings and a byte-stream size probe do not measure MCU speed or driver RAM.

Defaults allow 8 KiB unescaped CSV rows, 48 columns and 128-byte ZIP names.
`csv::Reader<R, ROW, FIELDS>` lets callers tune CSV bounds; `Feed::scan` uses the
defaults. IDs have a configurable maximum byte length. Capacity exhaustion and
oversized records return errors, never truncated matches. After any query error,
discard partial output/index contents. Rebuild indexes whenever the snapshot
changes; GTFS IDs are not stable between feed versions.

Supported ZIP entries: stored and raw DEFLATE, with CRC32 and size validation at
EOF; forward DEFLATE data descriptors; seekable stored descriptors using central
directory sizes. ZIP64, encryption, multi-disk archives and other compression
methods return errors. Skipping entries with known sizes does not inflate them
or verify their CRC. A forward stored entry with unknown size is unsupported.
Callbacks can receive rows before the final CRC is known; stage persisted output
until the full scan succeeds. Cancelling an async read can consume input; drop
the parser/response and reopen the pinned member rather than resuming it.

CSV handles BOMs, arbitrary chunk boundaries, CRLF, quotes, escaped quotes and
embedded newlines. It uses csv-core's permissive syntax parser and separately
validates fields and required headers. Unknown columns are ignored. The typed
query layer covers routes, stops, trips, stop times, calendar and calendar dates.
It does not expand `frequencies.txt`, GTFS Flex/location groups, realtime feeds,
or perform timezone/DST conversion. `Time` preserves service-day hours above 24;
an early-morning wall-clock departure board may also need the previous service
day. Missing intermediate stop times remain `None` and are not interpolated.

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
rustup target add riscv32imac-unknown-none-elf
cargo check -p gtfs --lib --locked --target riscv32imac-unknown-none-elf
python3 checks/check_ram.py
cargo run -p gtfs --release --example bench_stream -- gtfs/tests/fixtures/gtfs.zip 4 20261017 S1 1
```

The integration fixtures are small synthetic feeds, generated by
`python3 gtfs/tests/make_fixtures.py`. Tests exercise calendar joins, route variants,
one-byte transport chunks, dictionary wrapping, descriptors, corrupt/truncated
archives, CSV/ID limits, mirror discovery and selective ranges. Live downloads
are manual checks, so normal tests do not need network access.
Streaming regressions also cover reordered calendars/trips, stale snapshots,
provisional-buffer rollback, cancellation and original ZIP footer truncation.

Format references: [GTFS Schedule](https://gtfs.org/documentation/schedule/reference/),
[ZIP APPNOTE](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT),
[reqwless](https://docs.rs/reqwless/0.14.0/reqwless/).

Codex review policy is declared in [`.runnerless-ci.ts`](../.runnerless-ci.ts).
See [Runnerless setup and bootstrap enforcement](../docs/runnerless.md) for activation
and required-check details.

Licensed under MIT or Apache-2.0, at your option.
