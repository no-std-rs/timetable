# Boot failure and USB recovery

A physical-device report described an immediate black screen before battery
mode existed, followed by failure to reflash normally. Holding BOOT allowed
another firmware to be installed. The flashed revision and serial failure log
are unknown; recovery with another image does not by itself identify the cause
of the failed automatic USB connection.

## Confirmed software defects

The pinned Rust 1.95.0 C3 release before this fix had only 52,424 bytes available
for its call stack. Linked instructions and DWARF frames showed these nested
minimum requirements, excluding further callees and executor overhead:

| Cold-boot path | Frame sum |
| --- | ---: |
| Main poll + settings decode | 24,192 + 56,544 = 80,736 B |
| Main poll + network startup | 24,192 + 45,152 = 69,344 B |

Both exceed available memory before the first board frame. The settings schema's
nested derived deserialization creates large aggregate temporaries. Network
startup also constructed a 43,242-byte ZIP workspace on stack before copying
it into its static cell. `StaticCell::init_with` did not eliminate that temporary.
Counting static buffers and a nominal stack reserve could not catch these bugs.
An earlier pre-battery PR #6 ELF has a 59,104-byte main frame alone against
52,720 bytes available, plus a reachable 56,464-byte settings decoder. This
confirms that the stack defect predates the sleep feature.

The fix fills provisional settings progressively, one group and choice at a
time, directly in static application storage. It keeps required/duplicate/unknown
field checks, escaped strings, generation rollover and corrupt-bank fallback.
The ZIP dictionary is initialized statically, with its inflater created on first
use. Settings snapshots copy into query-owned static storage, reducing runtime
poll-frame temporaries too. Capacities and the 64 KiB heap are unchanged.

A visible “Getting ready...” screen now precedes settings/network startup on
normal boots. Timer-only battery wakes stay dark. Serial logs identify reset
reason and startup stages without printing credentials.

## Compiled checks and limits

`tools/build-device-artifact.sh` now runs `tools/check-device-stack.py` against
the actual release ELF, using GNU `readelf` (binutils) and retained DWARF CFI.
It rejects missing audited symbols/metadata, individual frames above 16 KiB,
audited boot paths above 24 KiB including 4 KiB headroom, and audited runtime
paths whose frame sum plus headroom exceeds linker-available stack. The
original binary is rejected for its 56,544-byte frame.

The [current scoped report](../benchmarks/device-boot-stack.json) includes
14,480 bytes for settings startup, 17,920 for partition loading and a conservative
43,680-byte TLS path sum, before the explicit 4,096-byte headroom. A conservative
sum uses the largest matching generic-function frame even when the exact
inspected call uses a smaller monomorphization. This is an audit of named paths,
not a complete call graph, interrupt bound or physical high-water measurement.

The [RAM report](../benchmarks/device-boot-ram.json) keeps the existing
static-plus-24-KiB-minimum-reserve account at 296,996 bytes, below the unchanged
300,000-byte regression ceiling. That number is not a whole-runtime peak: the
audited TLS path already needs more than 24 KiB. Static RAM is 272,420 bytes;
adding all 52,184 available stack bytes gives 324,604 bytes, below 400,000.

## Direct recovery

Hold **BOOT**, disconnect/reconnect USB, then release BOOT. Select the newly
enumerated port and install the fixed image directly from the USB installer.
This starts the chip's ROM download mode and bypasses application startup.
There is no need to install unrelated firmware first. With a separate reset
button, hold BOOT while pressing/resetting EN instead.

The app contains no intentional USB-pin reassignment, USB disable or eFuse
programming. ESP32-C3 USB uses GPIO18/19. Deep sleep disconnects USB, but cannot
explain a report predating battery mode. See Espressif's
[USB Serial/JTAG limitations](https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-guides/usb-serial-jtag-console.html)
and [download-mode instructions](https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/get-started/flashing-troubleshooting.html).

The startup defects are proven in compiled code. Successful cold boot and
normal USB reflashing of the affected physical board still require verification;
host tests, CI, browser checks and linked audits do not establish that result.
