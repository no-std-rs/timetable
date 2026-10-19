#!/usr/bin/env python3
"""Build fixed compressed scheduling fixtures from the existing public CSV sample."""
from pathlib import Path
import zipfile

root = Path(__file__).resolve().parent
csv = (root / 'stop_times.csv').read_bytes()
header, rows = csv.split(b'\n', 1)
for name, data in [('stop_times.zip', csv),
                   ('stop_times-repeated.zip', header + b'\n' + rows * 16)]:
    member = zipfile.ZipInfo('stop_times.txt', (2026, 10, 17, 1, 28, 0))
    member.compress_type = zipfile.ZIP_DEFLATED
    with zipfile.ZipFile(root / name, 'w') as archive:
        archive.writestr(member, data, compresslevel=9)
