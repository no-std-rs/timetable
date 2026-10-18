# timetable

A Cargo workspace for allocation-free, `no_std` public transport timetable
libraries.

The [`timetable-display`](display/) crate adds bounded departure groups, walking
estimates, a flash catalogue and a touch UI for a 240×240 round screen. It runs
without a standard library or allocator. Hardware and installer integration
follow in a separate PR.

The [`gtfs`](gtfs/) crate streams GTFS ZIP archives, lists routes and stops, and
filters departures with caller-owned buffers. Poznań is the first verified
provider, with MobilityDatabase range requests and a direct official-source
fallback. Its [README](gtfs/README.md) covers the API, reqwless examples and limits.

The crate's library, examples and regression fixtures live in `gtfs/`. Shared
CI, [performance measurements](PERFORMANCE.md), the target RAM probe and
[Runnerless review policy](docs/runnerless.md) live at the workspace root.

```sh
cargo run -p gtfs --release --example poznan -- archives
cargo run -p gtfs --release --example poznan -- remote latest routes
```

## Development

Run these commands from the workspace root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
python3 display/tests/make_fixtures.py
git diff --exit-code -- display/tests/fixtures
rustup target add riscv32imac-unknown-none-elf
cargo check -p gtfs --lib --locked --target riscv32imac-unknown-none-elf
cargo check -p timetable-display --lib --locked --target riscv32imac-unknown-none-elf
cargo fmt --manifest-path checks/ram/Cargo.toml --check
python3 checks/check_ram.py
cargo run -p gtfs --release --example bench_stream -- gtfs/tests/fixtures/gtfs.zip 4 20261017 S1 1
```

The RAM probe is an isolated `no_std` consumer of the `gtfs` crate. It checks the
complete embedded query future and explicit firmware reserves against 400,000
bytes. It also measures the complete display query future with explicit radio,
TLS, stack, tile and retained-app reserves. The benchmark checks that parsing
allocates no heap memory. Normal tests
use synthetic fixtures and need no live transit feeds.

Preview the actual Rust renderer without hardware:

```sh
cargo run -p timetable-display --example preview -- /tmp/timetable-preview.ppm
```

Licensed under MIT or Apache-2.0, at your option.
