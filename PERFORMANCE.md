# Streaming performance and the 400 KB RAM budget

Measured on 2026-10-17 with rustc 1.97.1, release builds, x86_64 Linux,
AMD Ryzen 9 7900. All files were already downloaded; CPU/file timings below
exclude network. MCU timing and actual driver/TLS RAM have not been measured.

## Workload and correctness

Official archive: `20261015_20261031.zip`, 12,392,540 compressed bytes.
SHA-256: `acefc4f09e70b091d81404ee127803c00468e03e9c8e42f996728ddf5d23a558`.
The [official listing](https://www.ztm.poznan.pl/otwarte-dane/gtfsfiles/) supplies
the dated download; the mirror is [MobilityDatabase mdb-982](https://mobilitydatabase.org/feeds/gtfs/mdb-982).

The ZIP contains 1,204,882 stop-time rows (72,647,206 uncompressed bytes).
Line ID `4` has 769 trips across all service days, of which 133 run on service
date `20261017`. Platform ID `129` returns 63 departure events. The bounded
cache's sorted checksum is `a71a50c86748e8c0` in every recorded round.
Results match the seekable implementation and an independent Python GTFS join.

The original-only pipeline reads the ZIP twice with `Read` alone. Pass 1 skips
stop times/shapes as compressed bytes and retains selected trips/services.
Pass 2 inflates stop times, filters with the trip index, and stages selected
events. Both passes validate the known table identities and ZIP footer.
There is no complete archive buffer, seek operation, or temporary ZIP file.

## Host timings

Seven measured rounds after one warmup, from
[the raw CSV](benchmarks/poznan-20261015.csv):

| Operation | Median |
| --- | ---: |
| Original metadata preparation | 16.255 ms |
| Original query with retained metadata | 249.696 ms |
| Complete original-only bootstrap and query | 266.917 ms |

Other runs on this shared host ranged down to about 184 ms for the same query;
these values are observations, not MCU latency guarantees. The final CSV keeps
all seven samples rather than reporting the fastest one. Consecutive trip-ID
matches **and misses** are cached; an ungrouped feed still uses exact binary
search and produces the same results.

Each measured round reported **0 heap allocations, 0 reallocations and 0
allocated bytes**, including both passes, CRC/footer checking, event staging
and sorting. The benchmark uses `stats_alloc` around the parser workflow, after
opening the source files and warming the executor. CLI arguments, benchmark
statistics and the host HTTP/TLS implementation are outside that region.
Because no allocation/reallocation occurs, the parser workflow's additional
peak heap use is zero. This does not claim that a Tokio/Rustls host process
fits an MCU's RAM.

## RAM

The tested bounded profile allocates room for **2,048 selected trips, 128 service
IDs, 256 selected events and 32-byte IDs**, independently of how many slots the
sample uses. Its async state owns the ZIP workspace, CSV state and all those
arrays. This counts complete compiler-generated future storage, including
nested futures; it is not merely a sum of public struct sizes.

`checks/ram` compiles a complete `#![no_std]` workflow with byte-slice readers.
`checks/check_ram.py` extracts compiler-folded future sizes from LLVM IR for
the target ABI. No target execution or allocator is required.

| Trip capacity | RISC-V 32-bit task state | Explicit reserves | Total | Fits 400,000 B |
| ---: | ---: | ---: | ---: | :---: |
| 1,024 | 132,584 B | 139,264 B | 271,848 B | Yes |
| **2,048** | **177,640 B** | **139,264 B** | **316,904 B** | **Yes** |
| 4,096 | 267,752 B | 139,264 B | 407,016 B | No |

Raw compiler results: [RISC-V](benchmarks/ram-riscv32.json),
[x86_64](benchmarks/ram-x86_64.json). The equivalent 2,048-trip x86_64
slice-reader probe totals 346,432 B. The actual host file benchmark has larger
source/timing bookkeeping and measures 235,512 B of task state, for a total
budget of **374,776 B** with the same reserves.

The reserves are explicit assumptions, changeable through `memory::Reserves`:

| Reserve | Bytes |
| --- | ---: |
| TLS receive | 16,384 |
| TLS transmit | 4,096 |
| HTTP headers/body buffering | 4,096 |
| Network stack and packet buffers | 32,768 |
| Polling/call stack and construction temporaries | 65,536 |
| Other application state/UI | 16,384 |
| **Total** | **139,264** |

The 32-bit baseline leaves **83,096 B** within 400,000 B. A real board transport
changes the future layout and may exceed these allowances, so measure its stack
high-water and account its driver, TLS and executor storage before deployment.
The probe is an ABI baseline, not a full firmware map or measured physical peak.
Place the large async task in appropriately sized static/executor storage;
constructing it on a small call stack would defeat the budget.

Storing every city's raw stop/route catalogue in RAM is outside this profile.
Stream catalogue rows to the UI or retain them in flash, keeping only selected
IDs and a small board in RAM. Cache overflow and ID/record limits return errors;
they never silently drop departures. `stream::collect` clears a provisional
board on error or cancellation. The bootstrap's trip capacity must fit selected
lines across all service days before date filtering.

CI checks the 2,048-trip 32-bit profile against **400,000 decimal bytes**, and
runs a release fixture benchmark that asserts zero parser allocations.

## Live reqwless checks

These manual checks used verified TLS and HTTP response bodies directly:

| Path | Requests | Successful ZIP/table body bytes | Observed wall time |
| --- | ---: | ---: | ---: |
| Official-only bootstrap + query | 2 | 24,785,080 | 1.755 s |
| Mirror metadata bootstrap + one original pass | 9 | 12,585,809 | 5.396 s |
| Unavailable mirror URL, then official-only fallback | 3 | 24,785,080 | 2.084 s |

All returned the same 63 events. The unsuccessful probe's response body is not
included in the ZIP/table byte count. First-use mirror bootstrap has more TLS
requests; retaining the manifest and selected trip index avoids those requests
on later original refreshes. A stale manifest triggers an original-only rebuild.
These timings vary with network/server load. The official endpoint still sends
the complete ZIP; streaming bounds RAM, not total transferred bytes.

## Reproduce

```sh
cargo run -p gtfs --release --example poznan -- download 20261015_20261031.zip poznan.zip
cargo run -p gtfs --release --example bench_stream -- poznan.zip 4 20261017 129 7
rustup target add riscv32imac-unknown-none-elf
python3 checks/check_ram.py
python3 checks/check_ram.py --target x86_64-unknown-linux-gnu

# Tiny, network-independent regression used in CI:
cargo run -p gtfs --release --example bench_stream -- gtfs/tests/fixtures/gtfs.zip 4 20261017 S1 1

# Direct streams with no ZIP file created:
cargo run -p gtfs --release --example poznan -- original 20261015_20261031.zip departures 4 20261017 129
cargo run -p gtfs --release --example poznan -- fallback https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip 20261015_20261031.zip 4 20261017 129
```

Historical links may disappear; list dated archives/snapshots before choosing a
different dataset. Increase capacity only after running the RAM check for that
configuration. The 4,096-trip benchmark profile is deliberately rejected by the
400,000-byte budget check.

## Catalogue picker lookup

Measured on 2026-10-18 with rustc 1.97.1, release builds and the same x86_64 host.
Seven rounds follow one warmup. Each round performs 25,440 lookups across all
212 routes and three direction filters, counting stops and locating the final
stop as the picker does. Initialization, rendering and I/O are excluded.

| Implementation | Median per round |
| --- | ---: |
| [Linear scan](benchmarks/catalog-linear.csv) | 120.592 ms |
| [Validated ordered-link lookup](benchmarks/catalog-indexed.csv) | 64.477 ms |

The ordered path finds a route's link range with binary searches and then reads
only those stops from flash. It retains no heap index or catalogue RAM copy.
Unordered catalogues still work through the original linear fallback. Every
round had checksum `36980740`, zero allocations and zero reallocations. These
host observations do not measure ESP32 screen latency or physical RAM high-water.

```sh
cargo run -p timetable-display --release --example bench_catalog
```

## Linked firmware RAM reduction

Measured on 2026-10-18 with the pinned Rust 1.95.0 release build for
`riscv32imc-unknown-none-elf`, from main `fa842824` and the optimized source.
These are linked IRAM/DRAM bytes, not a physical high-water measurement.
Raw section reports: [before](benchmarks/firmware-ram-before.json),
[after](benchmarks/firmware-ram-after.json).

| Account | Before | After |
| --- | ---: | ---: |
| Static RAM, including the 65,536-byte heap | 293,180 B | 267,788 B |
| Separate call-stack reserve | 24,576 B | 24,576 B |
| **Budgeted total** | **317,756 B** | **292,364 B** |
| Linker-available call stack | 27,328 B | 52,720 B |

The saving is **25,392 B (8.0%)**, with no reduction to parser row/field bounds,
TLS record or TCP buffers, trip/service capacities, or the 512-event horizon.
Both retained and pending departure caches use four-byte entries rather than
full timestamp structs. One shared `i64` epoch reconstructs exact event times;
checked offsets preserve the inclusive six-hour boundary and dates after 2038.
The panel renders directly into wire-format bytes borrowed from settings
scratch, and TLS borrows its connection and certificate verifier across awaits.
Allocator high-water tracking adds a small amount of bookkeeping to the result.

Full-capacity cache comparisons, invalid/overflowing timestamps, simultaneous
events, DST/25:xx queries and cancellation/footer rollback are covered by tests.
All six device screens match a full-framebuffer reference pixel for pixel when
rendered through the shared scratch; representative board previews also match
the pre-change output byte for byte. The panel's complete SPI payload is checked.

The same original archive above, line `4`, platform `MAGA42`, UTC `1792215000`
(2026-10-17 05:30), produces 20 identical departures before and after, with
zero allocations/reallocations. Seven alternating host release rounds after
warmup measured 254 ms and 253 ms median respectively; this shows no material
query-time change on this host, not an MCU timing result. The host compiler was
Rust 1.97.1. [Raw rounds](benchmarks/firmware-query-ram.csv) include the output
SHA-256 `91dfc20ade8a66aa59dc561347b3419bcd8f1459e4664c6142ca793fe97846f3`.

CI enforces a 300,000-byte firmware regression ceiling in addition to the
400,000-byte target and unchanged 24 KiB stack reserve. The ABI display-query
probe separately reports a 2,064-byte departure cache and 250,256-byte combined
query/reserve budget. Serial logs expose allocator current and peak-since-boot
usage after startup and refresh attempts; real board measurements are still
needed for stack high-water and a smaller radio/RSA heap allowance.

```sh
sh tools/build-device-artifact.sh
python3 checks/check_ram.py
cargo run -p timetable-display --release --example query -- /path/to/20261015_20261031.zip 4 MAGA42 1792215000
cd firmware
cargo test --lib --tests --locked --target x86_64-unknown-linux-gnu
```

## Streaming lookup and scheduling

Measured on 2026-10-18 against main `c588c861`, with Rust 1.97.1 host release
builds and the same dated original ZIP, line `4`, platform `MAGA42`, UTC
`1792215000`. Seven alternating rounds follow a warmup. Consecutive exact trip
hits and misses are cached in a 32-byte key; interleaved rows still use binary
search and long unselected IDs are never shortened.

| Display query pipeline | Median host time |
| --- | ---: |
| Binary search on every stop-time row | 270 ms |
| Consecutive hit/miss cache | 201 ms |

That is **25.6% less host time**, with 20 identical departures and zero
allocations/reallocations in every round. [Raw CSV](benchmarks/device-trip-cache.csv)
includes output SHA-256
`91dfc20ade8a66aa59dc561347b3419bcd8f1459e4664c6142ca793fe97846f3`.
The host query excludes network transport and does not establish MCU latency.
Regression archives compare grouped/interleaved rows, repeated misses and a
long missing ID sharing a selected ID's complete 32-byte prefix.

The firmware's HTTP reader caps reads and yields after each 2 KiB budget,
allowing other ready executor tasks to run when a download keeps returning
ready bytes. This is a cooperative scheduling point, not a wall-clock bound:
TLS handshakes, decompression and record callbacks also perform work. Tests
explicitly poll an always-ready source, verify the wake/Pending boundary,
reconstruct all bytes, and exercise partial reads, EOF, empty buffers and I/O
errors. Retry signals preserve the published board and never write settings.

The pinned Rust 1.95.0 linked firmware now budgets **292,516 B**, including the
unchanged 65,536-byte heap and 24,576-byte stack reserve. That is 152 B above
PR #6 and below the enforced 300,000-byte ceiling. [Raw linked report](benchmarks/device-responsive-ram.json)
records 267,940 B static RAM and 52,568 B linker-available stack. The isolated
32-bit display ABI query/reserve probe totals 250,312 B; core GTFS probe values
are unchanged. Physical peak memory and touch latency remain unmeasured.

## Tile rendering

Measured on 2026-10-18 against main `6782dd78`, on the same Ryzen 9 7900 host
using pinned Rust 1.95.0, firmware's release profile (`opt-level=s`, fat LTO)
and `firmware/examples/bench_render.rs`. Each sample draws 512 complete
240×240 frames through the actual fifteen 16-row tiles. Seven alternating
before/after rounds follow a 64-frame warmup for each binary. The board contains
512 cached events. Timings exclude SPI, network, startup and pixel checksumming.

| Screen | Before, µs/frame | After, µs/frame | Less CPU time |
| --- | ---: | ---: | ---: |
| Board | 346.965 | 111.642 | 67.8% |
| Groups | 158.994 | 71.721 | 54.9% |
| Picks | 222.342 | 78.135 | 64.9% |
| Routes | 269.596 | 91.261 | 66.1% |
| Stops | 446.402 | 293.076 | 34.3% |
| Walk | 294.001 | 90.152 | 69.3% |

[Raw rounds and full-image checksums](benchmarks/device-tile-render.csv) retain
all samples. Every screen has identical pixel checksums before and after.
The tile now reports its real absolute bounding rectangle. The renderer skips
invisible text and styled primitives before rasterizing them, including the
scaled countdown. It still renders every tile and sends the same complete
image; this does not claim reduced SPI traffic or measured physical frame time.
Tests compare tiled/full-frame pixels for empty and active boards, offline and
loading status, conditional boarding, a three-digit countdown and both line
fonts, as well as every editor screen. A separate allocation-instrumented test
checks that off-screen glyphs are culled without heap allocation/reallocation.

The [linked C3 report](benchmarks/device-tile-ram.json) budgets **292,532 B**,
16 B above the previous release, with 267,956 B static RAM, an unchanged
65,536-byte heap and 24,576-byte stack reserve. The linker leaves 52,552 B
available for the call stack. The 300,000-byte regression ceiling and parser/UI
capacities are unchanged. The isolated display query/reserve probe remains
250,312 B. These are compiler/linker bounds; hardware peak RAM and touch timing
remain unmeasured.

## Battery retention

The optional sleep/tap mode retains a fixed 4,096-byte RTC image, including all
512 departures and 32 headsigns in the existing 1,024-byte text pool. A full
snapshot occupies 3,184 encoded bytes; credentials are excluded. Restore checks
CRC, configuration identity, bounds and the RTC tick interval before publishing.
Repeated short sleeps preserve fractional seconds and the refresh deadline.

The pinned Rust 1.95.0 linked budget is **296,756 B**, including the unchanged
65,536-byte heap and 24,576-byte stack reserve. Static RAM including RTC is
272,180 B; the linker leaves 52,424 B for the call stack. This is 4,224 B above
the previous release and below the unchanged 300,000-byte ceiling. The
[raw section report](benchmarks/device-battery-ram.json) includes RTC retention.
Synchronous in-place application initialization avoids retaining a redundant
7.5 KiB aggregate in the main async task. The isolated query probe is unchanged.
See [battery mode](docs/battery.md) for policy and sources. Physical peak memory,
board current, battery life, sleep-clock drift and wake reliability remain
unmeasured; no battery-hour estimate is inferred from the linked-memory bound.

## Boot-stack correction

A physical report of an immediate black screen and BOOT-only recovery prompted
a linked-code audit. Before the fix, cold main plus settings decode needed at
least 80,736 bytes, and main plus network construction at least 69,344 bytes,
versus 52,424 bytes available. Static RAM checks did not expose those temporary
stack allocations. The previous static-plus-24-KiB figure must not be read as a
whole-runtime peak bound.

The fixed [stack report](benchmarks/device-boot-stack.json) checks progressive
settings decoding, constant ZIP workspace storage and query snapshots. Boot
settings uses a 14,480-byte frame sum; partition loading uses 17,920. Both fit
24 KiB with 4 KiB headroom. A conservative audited TLS sum is 43,680 bytes plus
that headroom, below 52,184 available bytes. The guard uses release DWARF CFI and
is a named-path regression audit, not a complete runtime or physical bound.

The [RAM report](benchmarks/device-boot-ram.json) records 272,420 bytes static
including the unchanged 64 KiB heap, 296,996 bytes with the unchanged minimum
24 KiB reserve, and 324,604 bytes including all linker-available stack. The
existing 300,000-byte static/minimum-reserve ceiling remains unchanged; the full
linked-memory account separately fits 400,000 bytes. Hardware cold boot, USB
reflashing and stack high-water remain unverified. See [recovery and analysis](docs/boot-recovery.md).

## Timetable freshness

The compact board footer distinguishes the last successful check from the source
update date. A tap shows full timestamps in Poznań time. Failed updates and RTC
cache restores preserve both values. HTTP Last-Modified is preferred; when it is
absent, the forward ZIP parser reads fixed-size UTC Info-ZIP UT metadata from the
central directory. It exposes the newest recognized table timestamp only after
directory, EOCD and body EOF validation. Multiple archives use their oldest
source time. Missing, malformed or ambiguous metadata remains unknown.

The pinned original ZIP above returns UTC Unix timestamp `1791966267` from its
tables, independently of its service-validity filename. Seven alternating
before/after host rounds against main `d4b662c4`, Rust 1.97.1 release, retain all
63 events and checksum `a71a50c86748e8c0`, with zero allocations, reallocations
and allocated bytes in every round. [Raw rounds](benchmarks/device-freshness-stream.csv)
show metadata medians 16.508 → 17.515 ms and query medians 275.166 → 267.590 ms.
These shared-host samples do not establish a speed improvement or MCU timings.
The actual host workflow future grows 144 bytes to 235,656 bytes; the isolated
32-bit GTFS profile grows 48 bytes to 316,952 bytes including reserves. The display
query/reserve probe grows 80 bytes to 250,392 bytes.

The [linked RAM report](benchmarks/device-freshness-ram.json), built with firmware's
pinned Rust 1.95.0, records **272,604 B static**, 184 bytes above the boot-fixed
base, including the unchanged 64 KiB heap. With the unchanged 24 KiB minimum
stack reserve, the regression account is **297,180 B**, below 300,000 B. Including
all 52,000 bytes of available stack, the linked account remains **324,604 B**.
The [stack audit](benchmarks/device-freshness-stack.json) records a conservative
44,032-byte TLS path plus 4,096-byte headroom, below that available stack. The
guard recognizes the pinned compiler's outlined HTTP request frame as well as
the previous timeout frame, summing both if present. Its limits are unchanged;
both current and boot-fixed base ELFs pass, and the original overflowing ELF
still fails. These are compiler/linker checks, not physical RAM high-water data.

## Connection diagnostics

Stage and failure-reason logging adds **544 B** of static RAM to the freshness
release, built with the same pinned Rust 1.95.0 release toolchain. The
[linked report](benchmarks/device-diagnostics-ram.json) records **273,148 B**
static including the unchanged 64 KiB heap; the unchanged minimum 24 KiB stack
reserve gives **297,724 B**, below the unchanged 300,000-byte ceiling. The full
linked account remains **324,604 B** including all 51,456 bytes available for
stack. The [stack audit](benchmarks/device-diagnostics-stack.json) still bounds
the named TLS path at 44,032 B plus 4,096 B headroom. Parser capacities, runtime
buffers and connection policy are unchanged. Physical peak RAM, cold boot and
Wi-Fi connectivity remain unmeasured; log-stage coverage does not prove a
physical connection issue has been fixed.

## TLS heap exhaustion correction

A physical device completed Wi-Fi association, DHCP and NTP, then aborted on a
1,024-byte allocation during RSA certificate verification. Its startup heap
sample was **46,376 B current / 46,548 B peak / 65,536 B capacity**, leaving
19,160 B at that point; this sample predates the handshake and is not its peak.
The real ZTM chain reproduced exhaustion in an offline first-fit arena of that
size. The unchanged verifier completes in a **24,576-byte** arena at **22,328 B
peak** with no retained allocations across repeated checks. These are x86_64
host allocator measurements using the firmware's pinned crypto dependencies,
not measurements of the radio or the device's combined runtime peak.

The radio now uses two static RX, two dynamic RX and two dynamic TX buffers,
two-entry RX/TX queues and no A-MPDU aggregation. Compared with the default ten
static RX buffers, this is expected to release approximately 12.8 KB of persistent
heap; the new physical value remains to be measured. Smaller queues and disabled
aggregation can reduce throughput. The device performs one streaming transfer
at a time, and retains the original TCP/TLS buffers, parser limits and TLS
certificate/hostname/time/signature verification. The shared heap remains 64 KiB.
See the pinned [radio buffer configuration](https://docs.espressif.com/projects/rust/esp-radio/0.18.0/esp32c3/esp_radio/wifi/struct.ControllerConfig.html)
and Espressif's [Wi-Fi buffer guidance](https://docs.espressif.com/projects/esp-idf/en/v6.1-rc1/esp32c3/api-guides/wifi-driver/overview.html).

A verifier wrapper checks for 24 KiB free immediately before certificate or
handshake-signature verification. Insufficient memory returns a TLS error and
uses the existing refresh retry path. This admission check is not a reservation
against concurrent radio allocations or a fragmentation proof. Bounded radio
queues reduce that competition; physical testing is still required. TLS logs
report free heap before negotiation and current/peak heap after a successful
handshake, and include the TLS failure reason on failure.

The [linked RAM report](benchmarks/device-tls-heap-ram.json) records **273,316 B**
static and **297,892 B** with the unchanged minimum 24 KiB stack reserve, below
the unchanged 300,000-byte regression ceiling. The full linked account remains
**324,604 B** including all 51,288 available stack bytes. The
[stack audit](benchmarks/device-tls-heap-stack.json) retains the 44,032-byte TLS
path plus 4,096-byte headroom. Physical combined peak, post-fix connectivity and
throughput remain unmeasured.

## HTTP request flush and physical heap follow-up

The post-fix physical report verifies successful association, DHCP, NTP and TLS.
Startup allocator usage falls from 46,376 B to **33,368 B**. The observed peak
through TLS and the subsequent HTTP failure is **59,728 / 65,536 B**, leaving
**5,808 B** at that peak. This is a real heap sample, not a stack measurement or
whole-device high-water bound; sustained ZIP downloads remain to be measured.

The next failure exposed a separate transport defect: `reqwless::Request` writes
headers without flushing, while `embedded-tls` buffers short application writes
and does not flush them when reading. The device therefore waited for a reply
before actually sending its GET. The shared sender now flushes before response
parsing. It adds no heap allocation or buffer. New offline tests reproduce the
old failure with a buffered peer and check partial writes, body reading and
write/flush failure propagation. The host diagnostic and device compile the same
sender and stream adapter. CI typechecks the diagnostic but never downloads feeds.

The live comparison against ZTM reproduces the old path's timeout. The fixed
sender reads the complete official listing (470 archive links), selects
`20261019_20261031.zip`, and streams its complete body while extracting **213
routes and 3,261 stops**. Selected table CRCs, the directory and body EOF validate;
other table payloads are skipped. Observed host durations are 0.618 s for listing
and 1.016 s for the ZIP pass, including TCP/TLS. Host TCP buffers and scheduling
differ from the device, so these are not C3 throughput measurements. No fixed
fixture or CI test depends on those live values.

The [RAM report](benchmarks/device-http-flush-ram.json) records **273,404 B static**
(+88 B), **297,980 B** with the unchanged minimum 24 KiB stack reserve, below the
unchanged 300,000-byte ceiling. Including all 51,200 linker-available stack bytes,
the full linked account remains **324,604 B**. The
[stack audit](benchmarks/device-http-flush-stack.json) keeps the 44,032-byte TLS
path plus 4,096 B headroom. Heap capacity and parser/TLS buffers are unchanged.


## Selective firmware updates and always-on radio idle

Measured on 2026-10-19 with the firmware's pinned Rust 1.95.0, release
`opt-level=s`/fat LTO, x86_64 Linux. `probe_device` calls the same
`download::refresh`, query builder, reqwless sender and embedded-tls verifier as
the C3. Both sequential live runs use line `4`, platform `MT44`, fixed query UTC
`1792367882`, a hard **65,536 B first-fit allocator** with **34,320 B reserved**
for the observed pre-handshake radio/application baseline, and a **20% host CPU
duty upper bound**. There is no system-allocator fallback during the operation;
an exhausted arena aborts. OS socket buffers, native stack and code are excluded.
CPU duty is enforced between cooperative polls using thread CPU time; it is not
ESP32 instruction, power or network emulation. Network waits can keep CPU usage
below that limit without extra sleeping.

| Same selection and query time | Mirror primary | Official fallback |
| --- | ---: | ---: |
| HTTP requests | 33 | 7 |
| Received body bytes | 16,957,797 | 71,092,577 |
| Wall time, one host run | 20,857 ms | 7,208 ms |
| Thread CPU time | 863 ms | 1,006 ms |
| Peak heap including reserve | 56,648 B | 56,648 B |
| Retained allocations | 0 B | 0 B |
| Published departures | 18 | 18 |

[Raw results and limits](benchmarks/device-mirror-live.json). Both paths produce
checksum `043c5d4a37aade22`, hashing sorted UTC times, group/choice, boarding
condition and resolved destination text. Source update UTC is `1791966267` in
both cases; the mirror's HTTP modification time is not source freshness.
The host update future is 47,384 B, with separate caller-owned Builder 30,392 B,
ZIP Workspace 43,272 B and Config 4,672 B. These host type sizes are not C3
static RAM or a stack bound.

The mirror transfers **76.1% fewer body bytes**, but is **slower on this fast
host connection** because it makes more HTTPS connections. These single-run
times establish neither a latency improvement nor ESP32 throughput. Two
snapshots are needed at this feed handover; each required table's payload is
read once per selected snapshot. Shapes and unused members are skipped. The
official fallback retains three forward ZIP passes per required archive. A
mirror failure, including one after table callbacks, restarts staging before
that fallback; cancellation never publishes partial departures. Offline tests
cover range/header/body validation, corruption, calendar exceptions and times
after 24:00. CI does not contact the live feeds.

The always-visible mode now runs the CPU at **80 MHz**, enables minimum modem
power saving during active Wi-Fi, then drops the controller, runner and sockets
after each attempt. Successful updates wait **three hours**, with NTP before
each new query; daily correction depends on an available network. Manual
refresh and configuration changes can wake the service sooner. The LCD and
touch remain active. Optional battery mode additionally blanks and deep-sleeps
as before. Current, temperature, clock drift and physical refresh latency still
need measurement on the board; no energy-saving percentage is claimed.

The [linked C3 RAM report](benchmarks/device-eco-mirror-ram.json) counts
**272,636 B static** including the unchanged 64 KiB heap, **768 B less** than the
HTTP-flush base. The unchanged 24 KiB minimum stack reserve yields **297,212 B**,
below the unchanged 300,000 B regression ceiling. Static RAM plus all **51,968 B**
of available stack remains **324,604 B**, below 400,000 B. Short GETs use 1 KiB
TCP/TLS transmit buffers; the 4 KiB TCP and 16,640 B TLS receive buffers, parser
limits and 512-event capacity remain unchanged. Directory reads borrow the idle
ZIP dictionary for scratch, without an additional buffer.

The [linked stack audit](benchmarks/device-eco-mirror-stack.json) includes
`TaskStorage::poll`, `Select::poll`, `update_once`, mirror/fetch and lower TLS
frames. The TLS path is **46,688 B + 4,096 B headroom < 51,968 B**; inflation is
22,752 B before headroom. A regression checks the exact bound, missing selection
frame and additional outlined session. An async wrapper that duplicated a large
owned network future was rejected by the unchanged RAM/stack guards; the final
selection constructor owns only one copy. These scoped checks are not a complete
call-graph proof or physical high-water measurement. The isolated RISC-V display
query/reserve probe is 250,488 B; the core GTFS probe remains 316,952 B.

```sh
cd firmware
cargo run --locked --release --example probe_device --target x86_64-unknown-linux-gnu -- 4 MT44 20 1792367882
cargo run --locked --release --example probe_device --target x86_64-unknown-linux-gnu -- 4 MT44 20 1792367882 --official
```

## Persistent ranges and software crypto

The mirror now uses two verified TLS connections per query: one listing
connection and one persistent files connection. Each requested table is a
single local-record range, including the local name, extra fields and ZIP data
descriptor. Directory order need not match physical order. Unused tables remain
skipped, CRC/length/identity checks remain mandatory, and any cancellation,
callback error, truncated body or unsupported framing poisons reuse. The official
streaming fallback still uses its established three-pass policy.

A paired live comparison used the same fixed UTC `1792367882`, line `4`, platform
`MT44`, pinned compiler/profile and actual firmware download/parser pipeline.
Each run had a hard 65,536-byte first-fit heap, 34,320 bytes reserved for the
radio baseline, no allocator fallback, and at most 20% host CPU duty. After one
warm-up pair, three alternating measured pairs produced:

| Quantity | PR #15 base | Persistent ranges + ChaCha20 |
| --- | ---: | ---: |
| HTTP requests | 33 | 19 |
| TLS connections | 33 | 2 |
| Body bytes | 16,957,798 | 16,958,206 |
| Median wall time | 19,108 ms | 6,469 ms |
| Median host CPU time | 600 ms | 411 ms |
| Heap peak including radio reserve | 56,648 B | 56,648 B |
| Retained allocations | 0 B | 0 B |
| Published departures | 18 | 18 |

All rounds matched checksum `043c5d4a37aade22`. The extra 408 bytes are local
record metadata included in the combined ranges. The **2.95× host wall-time
speedup is not an ESP32 throughput prediction**: host socket buffers, native
execution and stack are outside this test. [Raw rounds](benchmarks/device-throughput-live.json).
Reproduce each side with `firmware/examples/probe_device`, building the base
`c7d40e7de9fbae640351d2cd08a9ac064adc4a1f` and current source separately:

```sh
cd firmware
cargo run --locked --release --example probe_device --target x86_64-unknown-linux-gnu -- 4 MT44 20 1792367882
# Compatibility check of the independently verified official fallback:
cargo run --locked --release --example probe_device --target x86_64-unknown-linux-gnu -- 4 MT44 20 1792367882 --official
```

A separate allocation-free RV32IMC QEMU probe measures retired instructions,
not C3 cycles or network speed. On a 16 KiB record with five-byte TLS AAD,
RustCrypto ChaCha20-Poly1305 decryption takes 1,204,870 instructions versus
4,538,949 for AES-128-GCM, including per-record setup. The whole-row UTF-8 CSV
validation reduces the fixed 512-row parse from 1,484,038 to 1,105,727 instructions
(25.5%). Malformed field boundaries remain rejected.
[Raw results](benchmarks/device-throughput-rv32.json) and
[probe/reproduction details](checks/device-cpu/README.md).

Production TLS now negotiates ChaCha20-Poly1305 with the existing certificate,
hostname, time and signature checks. TCP receive grows from 4 to 6 KiB while
TLS receive remains 16,640 bytes and both transmit buffers remain 1 KiB. Active
transfers disable modem power saving; the 80 MHz CPU and complete radio teardown
between three-hour updates remain. Hardware throughput and energy must be
measured on the board; no physical speed, power or temperature claim is made.

The actual renderer shows four departures, exact `m:ss` countdowns, platform,
walking deadline and `HH:MM:SS` clock. A monotonic eight-phase spinner redraws
every 500 ms during association, NTP or table download, even without UTC; idle
boards redraw once per second. Rendering remains allocation-free and tiled.

The [linked report](benchmarks/device-throughput-ram.json) counts **274,036 B
static RAM**, including the unchanged 64 KiB heap, and **298,612 B** with the
unchanged minimum 24 KiB stack reserve, below the 300,000-byte ratchet. Static
RAM plus all 50,568 linker-available stack bytes is **324,604 B**, below 400,000.
The [scoped DWARF audit](benchmarks/device-throughput-stack.json) accounts for
the new session callers: TLS **40,560 B**, mirror record inflation **29,136 B**,
official inflation **19,872 B**, each separately with 4,096 B headroom. This is
a conservative envelope including both HTTP wrapper frame families, even when
a specialization inlines one into its caller; it is not a complete call-graph
proof or a physical stack high-water measurement.


## Persistent selected timetable cache

The shared device query now stores all-period selected GTFS in transactional flash
banks. [Cache format, capacity and cold-boot semantics](docs/persistent-cache.md).
The release `probe_device --cache PATH` uses the same parser, selector, verified
TLS transport and persistent range session. Its file-backed NOR model checks
1-to-0 programming and counts erase/program calls. The hard 65,536 B first-fit
heap includes a 34,320 B radio reservation; the executor limits host CPU duty to
20%. Query UTC1792367882, line4, stopMT44 are fixed across paired rounds.

Three paired rounds on 2026-10-19 produced the same 18 departures and checksum
`043c5d4a37aade22` in every mode. Medians from
[all nine raw observations](benchmarks/device-cache-live.json):

| Mode | Wall ms | CPU ms | HTTP requests | Body bytes | Flash erase/program calls |
| --- | ---: | ---: | ---: | ---: | ---: |
| Normal streaming | 6,458 | 463 | 19 | 16,958,205 | — |
| Unchanged source check + replay | 2,308 | 13 | 7 | 466,338 | 0 / 0 |
| Cold-cache replay only | 6 | 1 | 0 | 0 | 0 / 0 |

The unchanged check transfers **97.25% fewer body bytes**. Its remaining discovery
HTML and coverage requests are visible in the logs. Both network modes peak at
56,648 B bounded heap including the radio reservation; replay peaks at the
reservation alone, with no query allocation. All runs retained zero allocations.
The paired rounds preceded the review fixes. A post-review confirmation produced
the same checksum in all four modes: streaming 6,023 ms, unchanged check 2,352 ms,
replay 7 ms, and first fill 12,349 ms. The first fill made 41 requests and read
49,740,526 body bytes, with 22 erases and 337 program calls (86,040 bytes). Its
read-only preflight now checks the exact active query before writing, adding a
trip/stop-times scan to the comparison and write passes. A too-large active query
therefore retries with zero flash writes. Subsequent identical checks still
transfer 466,338 bytes and cause zero erases/programs. These post-review timings
are single observations, not paired medians; both sets are in the raw report.

These are **host measurements**, excluding host OS socket buffers and stack.
The file model does not emulate ESP32 flash latency; 6 ms replay excludes boot,
DHCP and clock synchronization. The same cached tables work after the initial
six-hour window; offline fixtures cover exceptions, times after 24:00, duplicate
platform codes, snapshot handovers, quoted fields, more than 1,024 all-period
trips, oversized selections and interrupted writes. Active query limits remain.

[The linked C3 report](benchmarks/device-cache-ram.json) counts **275,420 B static**
including the unchanged 64 KiB heap, plus the unchanged 24 KiB minimum reserve:
**299,996 B**, below the unchanged 300,000 B regression ceiling. The complete
linked account, including all **49,184 B available stack**, is **324,604 B**,
below 400,000 B. [The scoped stack audit](benchmarks/device-cache-stack.json)
includes cache replay, validation and synchronous flash frames: TLS 43,056 B plus
4,096 B headroom fits. These remain compiler/linker estimates and audited paths,
not physical stack high-water or a complete call-graph proof. No buffer, heap,
stack reserve, active trip capacity or departure capacity was reduced.


## Fused cache validation and recording

The first cache fill previously scanned the large trip/stop-times tables three
times: exact active-query validation, all-period comparison, then writing. The
query now records all-period rows while building its exact active index, leaving
**two scans per table**. Capacity, complete query validation and read-only
comparison still finish before the first erase. An unchanged source still skips
the large tables entirely; replay and flash wear semantics are unchanged.

Three paired fresh fills against main `7de44a7`, using the same pinned compiler,
locks, release profile, UTC/line/stop, verified TLS, 65,536 B bounded heap with
34,320 B radio reservation, and 20% host CPU duty produced these medians:

| Mode | Wall ms | CPU ms | HTTP requests | Body bytes | Flash erase/program calls |
| --- | ---: | ---: | ---: | ---: | ---: |
| Previous first fill | 11,867 | 1,244 | 41 | 49,740,483 | 22 / 337 |
| Fused first fill | 10,658 | 1,048 | 37 | 33,452,421 | 22 / 337 |
| Fused unchanged check + replay | 1,655 | 11 | 7 | 466,295 | 0 / 0 |
| Fused replay only | 4 | 0 | 0 | 0 | 0 / 0 |

First fill transfers **32.75% fewer body bytes**, with **10.19% less wall time**
and **15.76% less CPU time** in these host rounds. All 12 observations returned
18 departures with checksum `043c5d4a37aade22`, without fallback or retained
allocations. Each fill programmed the same 86,040 bytes. Both first-fill variants
and the unchanged check peaked at **56,648 B heap**, including the radio reserve;
replay needed only that reserve. [Raw rounds](benchmarks/device-fused-cache-live.json).
The probe commands are documented in [the device README](firmware/README.md).

The 16 KiB filter is caller-owned and independent of the exact active trip
index. The device obtains it fallibly from its existing heap **inside the
verified TLS session**, after certificate allocations have been freed. It is
released before any fallback handshake. The core remains allocation-free, the
heap capacity remains 64 KiB, and allocation failure falls back without writing
flash. Fixtures cover filter release after completion, error and cancellation,
zero extra scans on unchanged checks, and rejection before any erase.

[The linked C3 report](benchmarks/device-fused-cache-ram.json) counts **275,316 B
static**, including the unchanged heap. With the unchanged 24 KiB minimum stack
reserve, the regression account is **299,892 B**, below 300,000 B. Static RAM
plus all **49,288 B available stack** is **324,604 B**, below 400,000 B.
[The scoped audit](benchmarks/device-fused-cache-stack.json) includes the
compiler-outlined network-session caller: TLS **43,088 B + 4,096 B headroom**
fits the available stack. No buffer, query capacity or memory limit was reduced.

These are host timings and linked/audited RAM bounds, not physical ESP32-C3
throughput, energy or stack high-water measurements. File-backed NOR excludes
physical flash latency; replay excludes boot, DHCP and NTP. Fewer downloaded
bytes reduce work but do not establish a higher radio transfer rate. Association
logs now include RSSI to help diagnose the next physical-device run.


## Embassy scheduling and decoded-work budgets

The radio runner and query are sibling futures in a cooperative `select`.
Previously, the HTTP reader yielded only after 2,048 **compressed** bytes; a
high-expansion ZIP member could decode and parse many rows before returning
Pending. The configured Wi-Fi receive queue contains two packets. The pinned
driver's full-queue path rejects incoming packets, but no physical packet loss
has been measured here. Long query polls are a demonstrated scheduling risk,
not proof that they explain the reported 64–69 KiB/s.

The TLS-plaintext budget is now **512 bytes**, and mirror members have a second
**512-byte decoded CSV budget**. Both wake the parent before returning Pending,
allowing the network runner to be polled again. The original fallback also gets
the smaller plaintext budget. No network buffer, Wi-Fi queue, CPU clock, query
capacity, TLS verification or heap allowance changed.

The same pinned compiler/profile and allocation-free QEMU RV32 probe run the
real inflater/parser against fixed normal and high-expansion CSV fixtures.
Compare baseline `decoded=false` with candidate `decoded=true` in
[the raw instruction report](benchmarks/device-embassy-scheduling-rv32.json):

| Fixture | Previous max poll instructions | New max poll instructions | Previous total | New total |
| --- | ---: | ---: | ---: | ---: |
| 512 public sample rows | 978,217 | 390,587 | 1,837,486 | 1,851,718 |
| Same rows repeated 16 times | 11,973,272 | 491,830 | 21,420,795 | 21,715,515 |

Worst uninterrupted work falls **60.1%** and **95.9%** on these inputs, while
total instructions rise **0.77%** and **1.38%**. This is retired software work,
not C3 cycles, wall time or a universal latency bound: individual TLS record
decryption, inflater calls, flash operations and callbacks remain synchronous.
[Probe reproduction and fixture provenance](checks/device-cpu/README.md).
The actual host mirror-range regression checks repeated decoded yields even
when every HTTP read is immediately ready, and verifies all 8,192 rows.

Three paired fresh-cache host rounds against main `1fd98a2`, with a 65,536 B
bounded heap (34,320 B reserved radio) and 20% CPU duty, retain 18 departures and
checksum `043c5d4a37aade22`, 37 requests, 22 erases and 337 programs (86,040 B).
Discovery HTML varied by 43 B; selected-table transfer work is unchanged.
Median first-fill wall time is **11,308 → 10,973 ms**, while host CPU time rises
**994 → 1,315 ms**. The host's per-poll `clock_gettime`/duty limiter also runs
more often; this is not an isolated Embassy/decoder CPU measurement. These
rounds establish bounded heap and output/wear consistency, not improved C3
throughput. Unchanged checks and replay still make zero flash writes.
[All raw observations](benchmarks/device-embassy-scheduling-live.json).

The device now logs cumulative **elapsed poll work**, longest completed poll,
and rendering/SPI tile work for each radio session. Verified HTTP/query timing
starts after TLS certificate verification; Pending waits are excluded. Interrupt
and RTOS preemption time and in-query logging are included, so these are not
pure CPU utilization percentages. Counter updates occur after completed polls;
progress lines can lag the currently executing poll. No per-packet logging or
allocation is added. This instrumentation is needed to distinguish long polls
from waiting for remote TCP data on the physical device.

[The linked RAM report](benchmarks/device-embassy-scheduling-ram.json) includes
the unchanged 64 KiB heap and 24 KiB minimum stack reserve: **299,972 B**, below
the unchanged 300,000 B ratchet. The full account is **324,604 B** including all
**49,208 B available stack**. [The scoped stack audit](benchmarks/device-embassy-scheduling-stack.json)
counts TLS **43,168 B + 4,096 B headroom**, which fits. Poll instrumentation
borrows pinned futures instead of copying their large async state. These are
linked/audited bounds, not physical high-water measurements.


## Compact board and cache before Wi-Fi

The round board now shows six departures instead of four, while retaining the
large next-departure countdown, stop/platform, walking advice and freshness footer.
Lower rows move inward to fit the circle. Cold startup validates and queries the
existing flash bank before initializing Wi-Fi. Without known UTC it displays the
saved date and absolute departure times; it never uses the saved check time as
current UTC. NTP enables countdowns and a fresh query. An offline saved board also
survives battery tap wakes without losing its remaining retry deadline.

Five paired host rounds compare main `e5bd025` with this layout and clockless
replay, using the same persisted selection (line 4, MT44), pinned compiler and
release profile. The probe has the same hard 65,536 B heap, 34,320 B radio reserve
and 20% CPU duty limit. Both versions return 18 departures and checksum
`043c5d4a37aade22`, read 172,170 B, make zero network requests, and perform
**zero erases/programs**. Both need only the reserved radio heap, with no retained
allocation. Median replay wall time is **5 ms** for each version; median reported
CPU time is **1 ms**. File-backed reads exclude physical flash latency, boot,
Wi-Fi and NTP; these figures are not physical cold-boot measurements.

The actual 15-tile renderer, with 512 events and 512 frames per screen, takes
median **108,396 → 122,047 ns per board frame** on this host, a **12.6%** increase
for two extra visible departures. Other screen pixel checksums are unchanged;
board pixels intentionally differ. SPI is excluded. This records the cost of the
denser layout rather than claiming a throughput or power improvement.
[Raw paired rounds](benchmarks/device-compact-startup-host.json).

[Linked RAM](benchmarks/device-compact-startup-ram.json) is **275,412 B static**
including the unchanged 64 KiB heap; adding the unchanged 24 KiB minimum stack
reserve gives **299,988 B**, below the unchanged 300,000 B ratchet. All available
stack gives **324,604 B** total, below 400,000 B. The
[expanded stack audit](benchmarks/device-compact-startup-stack.json) counts cold
cache validation/replay separately (**9,008 / 9,888 B**) and TLS at **44,768 B**,
each with 4,096 B headroom within the 49,192 B available stack. Cache context is
borrowed between futures; no buffer, query capacity or memory allowance changed.
These are linked/audited bounds, not a complete call graph or hardware high-water.


## Shared Rust UI in WebAssembly

The installer uses the actual `timetable-display` App, touch state machine,
catalogue and renderer via a separate allocation-free `no_std` browser adapter.
It paints a 240×240 RGBA canvas, with browser UTC and visibly simulated
Wi-Fi/NTP/update/offline states. This is an interactive device demo with sample
departures, not a live feed client. Only groups and automatic switching enter
WASM; actual Wi-Fi credentials and power preferences stay in the installer.

The module has no host imports, threads or allocator. Its private linear memory
is fixed at **2,097,152 B**, including stack, embedded catalogue, a **230,400 B**
RGBA framebuffer, **8 KiB** input and **16 KiB** snapshot output. This is browser
adapter memory, separate from the unchanged device limits and from total
browser/JavaScript-engine memory. Rendering pauses outside the viewport or in
hidden tabs. Active demos redraw at most twice per second.

[Actual WASM observations](benchmarks/web-wasm-preview.json) record Node/V8
compilation/instantiation and five warm render rounds, excluding Canvas
presentation, JSON snapshots and network download. These host timings establish
that the real module runs; they are not a physical ESP32 or browser latency
bound. The byte size is recorded for the measured candidate.

Extracting featured-departure selection into the shared model preserves every
screen checksum. Five paired Rust 1.95 release host runs, 512 frames per screen,
show median board rendering **208,117 → 194,911 ns**; timing varies with host load
and is not evidence of a device speedup. [Raw paired rounds](benchmarks/device-wasm-preview-render.json).
The actual C3 artifact retains **275,412 B static**, **299,988 B** with the minimum
stack reserve and **324,604 B** including all available stack. All audited stack
bounds are unchanged, including TLS **44,768 B + 4,096 B** headroom.

Tests compare all pixels across 22 native/WASM scenario and gesture steps,
covering all seven screens. They also check zero-allocation edits, clockless
countdown exclusion, rejected configuration atomicity, exact escaped text,
maximum settings plus a draft, fixed memory and absent imports. Browser tests
cover scaled touch, keyboard Save/Cancel, explicit Apply/Reset, credential and
power-preference isolation, PL/EN, and retry after a failed module load.
