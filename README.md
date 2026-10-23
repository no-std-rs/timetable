# timetable

A Cargo workspace for allocation-free, `no_std` public transport timetable
libraries.

The [USB installer](https://no-std-rs.github.io/timetable/) and [touch-screen app](firmware/README.md) support the GUITION
ESP32-C3 round display and Waveshare ESP32-C6-Touch-LCD-1.47 portrait display. Set up named groups of lines and stops, choose walking times,
and switch groups by touch or an optional time-of-day schedule. Six departures
fit on the board. Choose a country, city or region and operator from the public
MobilityDatabase directory; [network setup and compatibility](web/README.md#countries-and-transport-networks) explain the device limits. Cold boots show saved times before Wi-Fi, with countdowns
enabled after NTP synchronization. The installer
keeps the screen on while releasing Wi-Fi between three-hour updates, with NTP
on every connection. It also offers [battery mode](docs/battery.md): sleep when idle, wake on tap with
retained departures and reconnect Wi-Fi only for refreshes. The
[`timetable-display`](display/) crate contains its allocation-free model,
streaming query builder and tile renderer; [`web/`](web/) contains the matching
porcelain, gold and chilli-red installer. Its interactive little screen runs
the same Rust interface through [WebAssembly](web-preview/), with visibly mocked
Wi-Fi/NTP and sample departures.

The [Android tablet app](android/) reuses the same Rust timetable with a responsive
native layout and browser WebUSB/ADB provisioning. It targets Android 5.0+ ARM
tablets, including older Fire 7 models. A JDK and a small set of pinned standalone
tools build the APK without an SDK/NDK installation; see its build and device-policy
notes before using Home, kiosk, or optional camera motion dimming.

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
python3 -m unittest discover -s tools/tests
python3 display/tests/make_fixtures.py
git diff --exit-code -- display/tests/fixtures
rustup target add riscv32imac-unknown-none-elf
cargo check -p gtfs --lib --locked --target riscv32imac-unknown-none-elf
cargo check -p timetable-display --lib --locked --target riscv32imac-unknown-none-elf
cargo check -p timetable-web-preview --lib --locked --target riscv32imac-unknown-none-elf
cargo check -p timetable-tablet --lib --locked --target riscv32imac-unknown-none-elf
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

CI runs once per pull request and again on `main`. The device workflow runs
Android, firmware and installer/browser checks concurrently, then assembles their
checked artifacts in the existing `C3 firmware and installer` gate. Rust caches
are separate per job and include each standalone Cargo workspace; main caches
also seed pull requests. Superseded PR runs are cancelled, while main device
builds and publishing stay ordered. Browser tests use four independent workers
on CI and one locally.

For the device and installer checks, see [firmware](firmware/README.md) and
[web](web/README.md). Preview the actual Rust renderer without hardware:

```sh
cargo run -p timetable-display --example preview -- /tmp/timetable-preview.ppm
```

Licensed under MIT or Apache-2.0, at your option.
