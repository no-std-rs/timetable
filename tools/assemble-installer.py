#!/usr/bin/env python3
"""Assemble a static installer with the exact checked firmware bytes."""
from pathlib import Path
import shutil
import sys

root = Path(__file__).resolve().parents[1]
firmware, site = map(Path, sys.argv[1:3])
site.mkdir(parents=True, exist_ok=True)
for name in ['index.html', 'style.css', 'app.js', 'config.js', 'catalog-search.js', 'route-topology.js', 'i18n.js', 'settings-store.js', 'serial-logs.js', 'favicon.svg', 'device-preview.js', 'feed-directory.js', 'csv-stream.js', 'zip-archive.js', 'feed-loader.js', 'feed-worker.js', 'feed-bundle.js', 'firmware-image.js', 'network-picker.js', 'devices.js']:
    shutil.copyfile(root / 'web' / name, site / name)
shutil.copyfile(root / 'web/dist/installer.js', site / 'installer.js')
shutil.copyfile(root / 'web/dist/device-preview.wasm', site / 'device-preview.wasm')
shutil.copyfile(root / 'catalog/catalog.json', site / 'catalog.json')
shutil.copyfile(root / 'catalog/feeds.json', site / 'feeds.json')
for name in ['timetable-display.bin', 'latest.json', 'manifest.json', 'SHA256SUMS', 'RAM.json', 'STACK.json']:
    shutil.copyfile(firmware / name, site / name)
# The default round build retains its existing URLs. Additional targets have
# complete, independently verified release bundles under devices/<id>/.
extra = firmware / 'devices' / 'waveshare-c6'
if not extra.is_dir():
    raise ValueError('missing required Waveshare C6 firmware bundle')
shutil.copytree(extra, site / 'devices' / 'waveshare-c6', dirs_exist_ok=True)
(site / '.nojekyll').touch()
print(site)
