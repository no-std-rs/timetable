# Development and review

The library must compile without `std` or `alloc`, including its runtime
dependencies. Host networking, TLS and executors belong in examples or development
dependencies. Keep buffers caller-owned and capacities explicit; overflow must
return an error rather than silently truncating GTFS results.

Validate a ZIP snapshot before publishing a completed timetable update. Mirror
metadata and original streams must describe the same uncompressed GTFS tables.
Keep cancellation and errors from exposing provisional departure-cache contents.
HTTP range requests for one query must use one immutable snapshot.

The embedded workflow targets 400,000 bytes of total RAM. Changes to buffers,
capacities or async state must pass `python3 checks/check_ram.py`; document the
firmware reserves separately from the compiler-measured future. Do not describe
the ABI estimate as a measurement on physical hardware.

Before pushing, run the commands in the README's development section. Regression
fixtures and CI must work without downloading live transit feeds. Verify findings
against supported GTFS behavior, including service exceptions and times after
24:00, before changing parser or query semantics.
