#!/usr/bin/env python3
"""Assemble a static installer with the exact checked firmware bytes."""
from pathlib import Path
import shutil
import sys

root = Path(__file__).resolve().parents[1]
firmware, site = map(Path, sys.argv[1:3])
site.mkdir(parents=True, exist_ok=True)
for name in ['index.html', 'style.css', 'app.js', 'config.js']:
    shutil.copyfile(root / 'web' / name, site / name)
shutil.copyfile(root / 'web/dist/installer.js', site / 'installer.js')
shutil.copyfile(root / 'catalog/catalog.json', site / 'catalog.json')
for name in ['timetable-display.bin', 'latest.json', 'manifest.json', 'SHA256SUMS', 'RAM.json']:
    shutil.copyfile(firmware / name, site / name)
(site / '.nojekyll').touch()
print(site)
