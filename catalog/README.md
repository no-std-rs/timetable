# Poznań browse catalogue

`catalog.bin` and `catalog.json` describe the same routes, physical stops and
directional route/stop membership. They contain no departure times. Source:
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
preserve both known directions and trips without `direction_id`. The reader
validates lengths, CRC, strings and link bounds before browsing flash in place.
