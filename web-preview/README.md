# Shared device UI in WebAssembly

This `no_std`, allocation-free browser adapter runs `timetable-display::App`,
`ui::tap` / `swipe`, the firmware catalogue, group scheduling, provisional
route/stop/walking editors and `render::draw_animated` directly. Canvas displays
the real 240×240 RGB565 pixels expanded to RGBA. CSS supplies only the case.

The browser supplies pointer/keyboard input and UTC from its own clock. Demo
hardware controls expose connected, cached startup, first Wi-Fi connection, NTP
sync, update and offline scenarios. These statuses, timetable refreshes and
sample departures are **simulated**: the demo does not query transit feeds,
connect Wi-Fi or send NTP packets. Source-update time stays unknown. Countdown
and walking advice remain disabled while the simulated clock is unknown.
Opening a group from the installer uses the device’s manual override to keep
that group visible. The device Groups screen can resume automatic schedules.
Sample departures are generated on configuration/mock refresh, not every tick.

Canvas: tap to navigate; hold 700 ms to edit; swipe groups or route/stop pages.
Keyboard: arrows swipe, Enter opens groups, E edits, Esc goes back, 1–4 select
list rows. In Walk, 1/2/3 choose walking time/spread/margin, +/- adjust, Enter
saves and Esc cancels. Pointer cancellation does not commit a tap.

Demo saves update its own committed settings. **Use demo changes** explicitly
copies groups and automatic mode to the installer; **Reset demo** loads the
installer settings again. Drafts stay provisional. Actual Wi-Fi/password/power
settings never enter WASM and are preserved when applying a demo save. A failed
module load leaves the installer usable and has its own retry.

## Build and test

```sh
rustup target add wasm32-unknown-unknown --toolchain 1.95.0
sh tools/build-web-preview.sh
cargo test -p timetable-web-preview --locked
npm --prefix web test
```

`npm --prefix web run build` also builds WASM. No wasm-bindgen CLI or JavaScript
UI reimplementation is needed. The small synchronous ABI has bounded 8 KiB
input buffer and 16 KiB output buffer, scalar gestures and a fixed RGBA framebuffer. Settings are
parsed and validated provisionally before replacing Config and App together.
The module has no host imports/callbacks, threads, shared memory or allocator.
Its private UnsafeCell state is accessed only during non-reentrant synchronous
exports; JS reads views between calls and reacquires them every time.

WASM linear memory is capped at **2,097,152 bytes** including stack, catalogue,
state and framebuffer; its framebuffer alone is 230,400 bytes. These browser-only
reserves are separate from ESP32 RAM and do not describe total browser/JS-engine
memory. Firmware does not depend on this crate. Its normal 400,000-byte total
and 300,000-byte minimum-reserve limits remain unchanged.

Tests compare all RGBA pixels from real WASM against native Rust across 22
scenario/gesture steps covering every screen. Native tests check zero allocation,
Save/Cancel semantics and clockless state; ABI tests check rejected malformed
input, bounded memory and absent imports. Browser tests exercise actual scaled
canvas gestures, keyboard, Apply/Reset isolation, language switching and retry.
