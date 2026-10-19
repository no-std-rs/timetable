# Persistent selected timetable

The device saves selected GTFS tables for the feed's full service period, rather
than writing a moving six-hour departure list. After acquiring valid UTC, it
replays that data before checking the mirror. Walking time, direction, group
order and schedules are applied when querying; changing these settings needs no
cache write. A new physical platform or line requires matching cached identities
or a download. Exact platform IDs remain distinct even when stop codes repeat.

An unchanged source is checked using snapshot directories and coverage only.
When source fingerprints change, a read-only streaming comparison filters the
selected tables. Unrelated routes, repacking and timestamp-only changes cause
zero erases and zero programs. A changed selection first completes this same
capacity/validation preflight, then streams into the inactive bank. First fill
and changed selected content therefore need two table passes. This extra initial
work avoids repeated flash wear from oversized or invalid data.

Two 480 KiB banks occupy the dedicated 960 KiB `ttcache` partition at `0x300000`
(custom partition type `0x40`, subtype zero). Each reserves a 4 KiB header sector;
487,424 payload bytes remain per bank. The factory app ends at `0x300000`;
settings stay at `0x3f0000`, with the existing settings protocol. Install the
complete new factory image to obtain this partition table. If an older layout
still overlaps the cache area, the device disables flash caching and streams
normally. It never erases that area based on firmware assumptions alone.

Canonical CSV preserves calendars, exceptions, after-midnight times, headsigns
and all-period selected-route trips. A Bloom filter borrows the existing 16 KiB
trip-ID scratch; false positives can retain extra rows, while the exact query
join removes them. No new large RAM buffer or allocator is added. Existing active
trip, service, heading and departure limits still fail explicitly. If selected
data exceed a bank, the normal streaming query remains available with no flash
writes and the previous bank retained.

A bank becomes visible only after HTTP/ZIP completion, selected-query validation,
header/payload CRCs and a final commit word. Cancellation, obsolete settings,
interrupted writes and corrupt newer banks preserve the previous valid bank.
Flash and settings share one serialized storage owner. Replay and validation
yield to the Embassy executor; physical flash program/erase calls are synchronous.

Successful check times update in RAM every refresh. Flash stores the actual
successful completion time only when committing changed content; a cold boot
shows that persisted time until the next successful check. This avoids writing
a timestamp every three hours. Source age includes only archives contributing to
the current window. True power loss still needs NTP before accurate countdowns;
flash replay does not manufacture a clock. Deep-sleep retention keeps its
existing clock and departure fast path.

See [performance evidence](../PERFORMANCE.md#persistent-selected-timetable-cache)
and [the constrained example](../firmware/examples/probe_device.rs). Host replay
time excludes boot, DHCP and NTP; physical cold-boot latency and flash endurance
have not been measured on a board.
