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
| Original metadata preparation | 17.778 ms |
| Original query with retained metadata | 293.391 ms |
| Complete original-only bootstrap and query | 308.529 ms |

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
| 1,024 | 132,536 B | 139,264 B | 271,800 B | Yes |
| **2,048** | **177,592 B** | **139,264 B** | **316,856 B** | **Yes** |
| 4,096 | 267,704 B | 139,264 B | 406,968 B | No |

Raw compiler results: [RISC-V](benchmarks/ram-riscv32.json),
[x86_64](benchmarks/ram-x86_64.json). The equivalent 2,048-trip x86_64
slice-reader probe totals 346,384 B. The actual host file benchmark has larger
source/timing bookkeeping and measures 235,368 B of task state, for a total
budget of **374,632 B** with the same reserves.

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

The 32-bit baseline leaves **83,144 B** within 400,000 B. A real board transport
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
cargo run --release --example poznan -- download 20261015_20261031.zip poznan.zip
cargo run --release --example bench_stream -- poznan.zip 4 20261017 129 7
rustup target add riscv32imac-unknown-none-elf
python3 checks/check_ram.py
python3 checks/check_ram.py --target x86_64-unknown-linux-gnu

# Tiny, network-independent regression used in CI:
cargo run --release --example bench_stream -- tests/fixtures/gtfs.zip 4 20261017 S1 1

# Direct streams with no ZIP file created:
cargo run --release --example poznan -- original 20261015_20261031.zip departures 4 20261017 129
cargo run --release --example poznan -- fallback https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip 20261015_20261031.zip 4 20261017 129
```

Historical links may disappear; list dated archives/snapshots before choosing a
different dataset. Increase capacity only after running the RAM check for that
configuration. The 4,096-trip benchmark profile is deliberately rejected by the
400,000-byte budget check.
