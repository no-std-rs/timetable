# RV32 instruction-cost probe

A standalone `no_std`, allocation-free QEMU **virt** program built with the
firmware's pinned Rust compiler/profile. It compares RustCrypto AES-128-GCM and
ChaCha20-Poly1305 on 16,384-byte records with five bytes of TLS additional data,
including the per-record key setup performed by embedded-tls. Every decrypted
record is checked byte for byte. It also parses a fixed 512-row GTFS CSV sample
through the actual `gtfs::csv::Reader`, checking row count and selected-field
checksum. QEMU-only UART, entry stack and exit addresses never enter firmware.

From the repository root (requires `qemu-system-riscv32`, e.g. Ubuntu's
`qemu-system-misc` package):

```sh
python3 tools/bench-device-cpu.py --baseline c7d40e7de9fbae640351d2cd08a9ac064adc4a1f --output /tmp/device-cpu.json
```

The runner builds the **same probe source, lockfile, compiler, profile and input**
in isolated temporary baseline/candidate trees and runs deterministic QEMU
instruction counting. The baseline comes from the requested git commit; the
candidate uses current sources. Both cipher implementations run in both builds.
The script rejects differing firmware/probe crypto versions and incorrect CSV
outputs. CI compiles/formats the probe but does not need QEMU or live feeds.

Retired instructions are reproducible software work, **not C3 cycles, physical
throughput, power, peak memory, or hardware acceleration measurements**. In
particular, this model excludes caches, Wi-Fi, network latency and the display.
The production firmware uses ChaCha20-Poly1305, full verified TLS, and a separate
64 KiB first-fit heap regression; this benchmark has no allocator.

`stop_times.csv` contains a header and 512 public Poznań stop-time records.
SHA-256: `68138cf8e96e1c05b6258f070593a5234bc5abbb507f8f4f26795704a584b855`.
It is fixed test data, not a current timetable or a live-feed dependency.

Fixture provenance: immutable snapshot
`mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip` from
`https://files.mobilitydatabase.org/` (11,207,002 bytes; SHA-256
`5b063fd41489371cf364c45b2e411e88bfc0fba16a4c9261fe0e8b95ee7bf906`).
The original `stop_times.txt` is 72,547,076 bytes, SHA-256
`98fab6fd2f2639769deba3f3348083041ea149c0328565501caa779711736ab5`.
To reproduce the fixture, decode UTF-8 preserving the BOM, read with Python
`csv.reader(newline="")`, take the header and first 512 records, then serialize
with `csv.writer(lineterminator="\n")` and default Excel minimal quoting.
This normalizes CSV quoting and newlines; it is not a verbatim byte prefix.
