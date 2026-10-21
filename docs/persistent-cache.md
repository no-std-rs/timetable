# Persistent selected timetable

The device saves selected GTFS tables for the feed's full service period, rather
than writing a moving six-hour departure list. On cold boot it validates and
replays saved data **before starting Wi-Fi**. Until NTP succeeds, the board shows
absolute times with the saved date and **CLOCK PENDING**, never countdowns or
walking advice. After clock synchronization it re-queries the cache at current
UTC before checking the mirror. Walking time, direction, group
order and schedules are applied when querying; changing these settings needs no
cache write. A new physical platform or line requires matching cached identities
or a download. Exact platform IDs remain distinct even when stop codes repeat.

An unchanged source is checked using snapshot directories and coverage only.
When source fingerprints change, a read-only streaming comparison filters the
selected tables. Unrelated routes, repacking and timestamp-only changes cause
zero erases and zero programs. A changed selection first completes this same
capacity/validation preflight, including the exact active-trip/departure query,
then streams into the inactive bank. Changed selected content needs two scans
of each required table: comparison/preflight and writing. The
exact active query runs alongside recording in each scan, so it needs no extra
trip/stop-times download. With an existing bank, complete read-only preflight
still rejects oversized or invalid data before erasing flash.

An empty cache checks metadata and trip-index bounds first, then reads the large
stop-times table only once while writing provisional selected rows. Complete
HTTP/ZIP, CRC and query validation still precede commit and publication. A late
stop-time error, overflow or cancellation can leave uncommitted flash writes;
they never become a usable bank, and the next attempt can rebuild it. Early
metadata/trip capacity errors still cause no writes. No additional RAM buffer or
compressed-stream checkpoint is needed.

Two 480 KiB banks occupy the dedicated 960 KiB `ttcache` partition at `0x300000`
(custom partition type `0x40`, subtype zero). Each reserves a 4 KiB header sector;
487,424 payload bytes remain per bank. The factory app ends at `0x300000`;
settings stay at `0x3f0000`, with the existing settings protocol. Install the
complete new factory image to obtain this partition table. If an older layout
still overlaps the cache area, the device disables flash caching and streams
normally. It never erases that area based on firmware assumptions alone.

Canonical CSV preserves calendars, exceptions, after-midnight times, headsigns
and all-period selected-route trips. A caller supplies a 16 KiB Bloom filter
while the exact query retains its active trip index; false positives can retain
extra rows, while the exact query join removes them. The core remains `no_std`
and allocation-free. The device fallibly borrows this space from its existing
64 KiB heap after TLS certificate verification has released its allocations. It
releases the filter on completion, error or cancellation, before opening any
fallback connection. An unavailable filter uses the normal streaming query
without flash writes. Existing active trip, service, heading and departure limits
still fail explicitly. If selected
data exceed a bank, the normal streaming query remains available and the
previous bank is retained. Existing-bank preflight prevents writes on overflow;
an empty-bank attempt may have written provisional rows before a late overflow.

A bank becomes visible only after HTTP/ZIP completion, selected-query validation,
header/payload CRCs and a final commit word. Cancellation, obsolete settings,
interrupted writes and corrupt newer banks preserve the previous valid bank.
A storage read error prevents choosing any write target, preserving unknown banks.
Flash and settings share one serialized storage owner. Replay and validation
yield to the Embassy executor; physical flash program/erase calls are synchronous.

Successful check times update in RAM every refresh. Flash stores the actual
successful completion time only when committing changed content; a cold boot
shows that persisted time until the next successful check. This avoids writing
a timestamp every three hours. Source age includes only archives contributing to
the current window. True power loss still needs NTP before accurate countdowns;
flash replay does not manufacture a clock. Deep-sleep retention keeps its
existing clock and departure fast path.

See [performance evidence](../PERFORMANCE.md#fused-cache-validation-and-recording)
and [the constrained example](../firmware/examples/probe_device.rs). Host replay
time excludes boot, DHCP and NTP; physical cold-boot latency and flash endurance
have not been measured on a board.
