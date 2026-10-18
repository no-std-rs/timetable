# timetable-display

Allocation-free, `no_std` application model, GTFS selection builder, flash
catalogue reader and 240×240 tile renderer. Hardware and networking adapters are separate from this crate. A companion
firmware and browser installer are introduced by a following PR.

The four group slots each contain up to four line/stop choices. Each choice has
an optional direction filter, walking time, walking variation and boarding
buffer. Group schedules use Europe/Warsaw civil time, weekday masks and windows
that may cross midnight. Overlaps use the first group. Manual overrides last
two hours once the clock is known. With no scheduled groups, manual selection
remains active.

The board recommends the earliest scheduled departure whose walking estimate
is at least 50%, with two later alternatives. If none meets that threshold, it
shows the earliest departure and its lower estimate. “Leave in” subtracts the
usual walking duration and boarding buffer. Walking duration is modelled as
uniform in `[max(0, walk - variation), walk + variation]`; the percentage is the
fraction of that interval that fits before the scheduled departure, after the
boarding buffer. It is not a calibrated catch probability or vehicle prediction.
The actual trip or stop-specific headsign distinguishes branches; `*` denotes
conditional pickup. No-boarding and untimed events are omitted.

`feed::Builder` uses three verified forward passes through a pinned original ZIP:
resolve line names/stop codes and calendars, select active trips, then collect
future departures. Yesterday supplies 25:xx departures, and tomorrow is included
when the six-hour horizon crosses midnight. Mask bits 0/1/2 refer to yesterday,
today and tomorrow; `day_mask()` reports which dates need archives.
The device retains a six-hour horizon, not a whole day's timetable.
Selected trip IDs remain exact in a 16 KiB shared byte buffer; there is a
1,024-trip limit, 64 distinct service IDs across the date window (including
removals), a 1 KiB shared headsign buffer and
512 departure slots. Capacity failures are errors. IDs are never shortened;
display labels may be clipped to the screen's supported Latin-2 font.

Callbacks and new query results remain provisional until footer and snapshot
validation finish. Error or cancellation rolls back the current departure pass.
Firmware swaps results into the visible app only after the entire refresh
succeeds and the selected settings still match its revision. The prior
validated snapshot survives a failed background refresh.

The settings protocol is versioned, bounded to 8 KiB, and CRC-protected over
generation, length and JSON. Firmware alternates two flash banks and verifies
the new bank before considering it committed. The committed wire fixture is shared with the companion browser encoder.

```sh
cargo test -p timetable-display --all-targets --locked
cargo check -p timetable-display --lib --locked --target riscv32imc-unknown-none-elf
cargo run -p timetable-display --example preview -- /tmp/timetable-preview.ppm
cargo run -p timetable-display --release --example query -- /path/to/official.zip 4 MT01
```
