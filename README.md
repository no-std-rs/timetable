# timetable

A Cargo workspace for allocation-free, `no_std` public transport timetable
libraries.

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
rustup target add riscv32imac-unknown-none-elf
cargo check -p gtfs --lib --locked --target riscv32imac-unknown-none-elf
cargo fmt --manifest-path checks/ram/Cargo.toml --check
python3 checks/check_ram.py
cargo run -p gtfs --release --example bench_stream -- gtfs/tests/fixtures/gtfs.zip 4 20261017 S1 1
```

The RAM probe is an isolated `no_std` consumer of the `gtfs` crate. It checks the
complete embedded query future and explicit firmware reserves against 400,000
bytes. The benchmark checks that parsing allocates no heap memory. Normal tests
use synthetic fixtures and need no live transit feeds.

Licensed under MIT or Apache-2.0, at your option.
