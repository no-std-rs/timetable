# Round timetable display

Rust firmware for **GUITION ESP32-2424S012-V1.0**, with an ESP32-C3, 4 MB flash,
240×240 round GC9A01 LCD and CST816 touch. It runs independently over 2.4 GHz
Wi-Fi, reading the official Poznań GTFS ZIP. No companion server is required.
The UI and installer use warm porcelain, dark ink, restrained kintsugi gold and
chilli red for the main action and departure urgency.

![Rust renderer preview](../docs/previews/device.png)

[Desktop installer preview](../docs/previews/installer-desktop.png) ·
[Mobile installer preview](../docs/previews/installer-mobile.png) ·
[Searchable stop picker](../docs/previews/installer-picker-mobile.png) ·
[Device editor](../docs/previews/device-walk.png).
These previews use illustrative departures; the device image comes from the
actual Rust renderer, not a photo of hardware.

## Install and use

The build workflow produces a `timetable-display` Actions artifact containing a
verified merged factory binary and the complete static USB installer. Main
builds deploy that installer through GitHub Pages. PRs build and test everything
without deploying. The published installer is at
<https://no-std-rs.github.io/timetable/>.

In the installer, name your groups, choose lines, directions and stops, and set
the walking time for each choice. Optionally schedule groups on selected days.
Then enter Wi-Fi settings and use desktop Chrome or Edge to install over USB.
The website verifies SHA-256 before offering the binary. It sends settings as a
separate USB image, never to a server, and clears credentials when leaving the
page. The firmware image itself contains no credentials. Installation replaces
the device's previous firmware and saved settings; device flash settings are
not encrypted.

On the display:

- **Swipe left/right:** change group, overriding automatic selection for two hours.
- **Tap the heading:** choose a group or return to the automatic schedule.
- **Hold the screen:** edit the current group. Tap a saved choice or “Add”.
- **Choose a line and stop:** swipe vertically or tap the bottom arrows for more.
  The stop page's bottom-centre control cycles either direction / 0 / 1.
- **Adjust your walk:** left/right decrements/increments walking time, its
  variation and the boarding buffer. Save commits; the top back control cancels.
  Tap the line badge to change the line, or the stop label to change the stop.
  The picker opens at the saved selection, highlights it and shows page counts.
  The bottom-centre stop control shows a destination and skips directions with
  no boarding stops. Back moves up one step; leaving the walk editor cancels its
  draft. Remove deletes a saved choice; Cancel discards a new unsaved one.

The next suitable scheduled departure is prominent; two later alternatives
remain visible. “Leave in” includes the walking time and boarding buffer. The
walking percentage uses your configured range and is explicitly an estimate,
not real-time vehicle reliability. `*` means conditional boarding.
Walking estimates of zero show “Not enough walk time” instead of “Leave now”.
Failed refreshes retain the last validated cache and show an offline indicator.
“NOT SAVED” means a settings write failed; retry Save before restarting.

The public image without a settings bank shows the USB setup screen and does
not start Wi-Fi. Group names and schedules can be configured in the installer;
line, stop, direction and walking settings can then be changed on the device.

## Data and memory

The bundled browse catalogue contains **212 routes, 3,232 boarding stops and 10,614
route/stop links**, generated from the official 2026-10-15 feed. It occupies
461,316 flash bytes and is read directly from flash. It contains no departures.
See [catalogue provenance](../catalog/README.md). Route short names and physical
stop codes are resolved against each new feed, so saved numeric GTFS IDs may
change without selecting a different line or platform. Ambiguous or missing
identities fail explicitly. Refresh the catalogue when lines/stops change.

After DHCP, SNTP supplies UTC before TLS certificate validity is checked. The
pure Rust `embedded-tls` transport verifies ZTM's hostname and pinned Certum
root; `reqwless` parses streaming HTTP responses. Compressed HTTP representations
and redirects are rejected. The official listing selects a dated immutable ZIP
for each required service day: yesterday, today, and tomorrow when its GTFS
service epoch enters the horizon. At the spring clock change this can happen
before civil midnight. Dates sharing the same latest archive reuse one set of three
passes for calendars, selected trips and departures. Each distinct archive
gets its own three-pass validation. This reduces retained metadata versus
indexing inactive services. The generic `gtfs` crate's existing two-pass and
mirror-assisted APIs remain available.

A first refresh downloads several ZIP streams and may take minutes on a slow
network. Touch and drawing run separately. A successful refresh caches six hours
of departures and refreshes after three hours; failures retry after a minute.
Config changes cancel an obsolete in-flight query. Network or validation
failure never publishes provisional results. The previous/current/next
service-day window covers Poznań's normal after-midnight trips; feeds with service times
extending past two civil days need a wider application policy.

Both application crates are `no_std` and use no allocator. Radio and RSA TLS use
a fixed **64 KiB heap**. The panel uses a **7.5 KiB tile**, with no framebuffer.
`tools/check-device-ram.py` counts actual linked IRAM/DRAM sections, including
radio, heap, all async tasks and application state; alias dummy sections are
excluded. It requires a separate 24 KiB available call stack and enforces the
400,000-byte limit. `RAM.json` in each artifact reports that exact build. This is
a linked-memory bound; stack/heap high-water and screen/radio/touch behaviour
still require the physical board. No physical flash or hardware test has been
performed as part of this implementation.

The fixed app limits are four groups, four choices each, 1,024 active trips in a
16 KiB exact ID pool, 64 distinct service IDs across the date window (including
removals), 32 headsigns in a 1 KiB pool and 512 future events. Large selections
fail visibly rather than dropping departures.

## Build and inspect

From the repository root, with `espflash 4.4.0` installed:

```sh
sh tools/build-device-artifact.sh
npm --prefix web ci
npm --prefix web run build
npm --prefix web test
python3 tools/assemble-installer.py dist/firmware dist/installer
node web/tests/server.mjs
```

Open <http://127.0.0.1:4197/>. The server serves the built binary with the source
installer; `dist/installer/` is the self-contained deployable site.

From `firmware/`:

```sh
cargo fmt --all --check
cargo test --lib --locked --target x86_64-unknown-linux-gnu
cargo clippy --locked --release --features firmware -- -D warnings
cargo build --locked --release --features firmware
# Only with a connected matching board and intentional replacement:
cargo run --locked --release --features firmware
```

The pinned Rust 1.95.0 toolchain builds the stable `riscv32imc-unknown-none-elf`
target; no ESP-IDF application or C application code is used. Espressif's radio
firmware and ROM routines are required by `esp-radio`, as in the source demo.
The bootloader/image format is compatible with ESP Web Tools.

Board wiring and panel setup come from the neighbouring router demo; see
[NOTICE](NOTICE). LCD SCK/MOSI/DC/CS/backlight are GPIO6/7/2/10/3; touch
SDA/SCL/reset are GPIO4/5/1. Touch is polled because its interrupt shares GPIO10
with LCD CS. SPI is 40 MHz. The factory partition ends at `0x3f0000`; the 64 KiB
`timetable` partition contains independent settings banks at `0x3f0000` and
`0x3f8000`. DIO/80 MHz matches this board.
