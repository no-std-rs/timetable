# Poznań browse catalogue

`catalog.bin` and `catalog.json` describe the same routes, physical stops and
directional route/stop membership. The JSON also has browser-only ordered trip
patterns reconstructed by sorting each trip's numeric `stop_sequence`. They
preserve branches, repeated visits, per-stop headsigns and pickup restrictions.
Ordering and per-call overrides follow the [GTFS stop-times specification](https://gtfs.org/documentation/schedule/reference/#stop_timestxt).
`patterns.calls` stores `[stop_index, pickup_type, optional_stop_headsign]`;
Browser records also retain full `place_name` identity and optional `lat`/`lon`
coordinates for local journey grouping and walkable-stop suggestions.
Indices address `stops` followed by `via_stops`. The latter also retains
referenced parent stations for journey labels and search, alongside graph-only
non-boarding platforms and is never offered by ordinary departure boarding selection. Journey targets
can include these alighting platforms. `patterns.trips` counts
matching trips in the snapshot, for initial preview ordering; it is not a daily
frequency or an assertion that a path runs today. Flash format and bytes are
unchanged by the browser extension. They contain no departure times. Source:
[ZTM's official GTFS archive listing](https://www.ztm.poznan.pl/otwarte-dane/gtfsfiles/).
The generation date and source ZIP SHA-256 are in `provenance.json`.

To refresh, download an explicit official snapshot and run from the repo root:

```sh
python3 tools/build-catalog.py /path/to/official.zip catalog --date YYYYMMDD
cargo test -p timetable-display --locked
```

Commit the three generated files together. Firmware builds and regression tests
use them offline; CI does not download a live feed. Rebuilding the image updates
the device's browse catalogue. Departures always come from the currently valid
original feed, and saved selections are resolved by public line name and stop
code before use. Exact route IDs disambiguate duplicate line names when the
public name still matches; stale IDs use a unique public name. Stop codes are
exact and bounded to 16 bytes. Stops without codes use their exact ID and the
same normalized, clipped name as the catalogue. Drop-off-only directional
links are excluded from boarding selection.

The flash format has a 32-byte header (`TTCAT001`, date, counts and payload CRC),
168-byte route records, 112-byte stop records and 6-byte route/stop links. IDs
are exact UTF-8, up to 32 bytes; overflowing IDs fail generation. Human labels
may be clipped on a UTF-8 boundary to fixed display fields. Direction flags
use bits 1/2 for directions 0/1 and bit 4 for trips without `direction_id`;
directionless trips are offered only under either direction. The reader
validates lengths, CRC, strings and link bounds before browsing flash in place.
