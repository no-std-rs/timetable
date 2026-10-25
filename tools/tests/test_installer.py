"""Assembled websites must contain the modules imported by their entry points."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import re
import runpy
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


class InstallerTests(unittest.TestCase):
    def test_assembled_site_contains_its_relative_javascript_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for part in ('tools', 'web/dist/android-adb', 'catalog', 'android/dist', 'firmware'):
                (root / part).mkdir(parents=True)
            script = root / 'tools/assemble-installer.py'
            shutil.copyfile(ROOT / 'tools/assemble-installer.py', script)
            for source in (ROOT / 'web').iterdir():
                if source.is_file() and source.suffix in ('.js', '.html', '.css', '.svg'):
                    shutil.copyfile(source, root / 'web' / source.name)
            for name in ('installer.js', 'device-preview.wasm'):
                (root / 'web/dist' / name).write_bytes(b'fixture')
            for name in ('catalog.json', 'feeds.json'):
                (root / 'catalog' / name).write_text('{}')
            apk = b'offline APK fixture'
            (root / 'android/dist/timetable.apk').write_bytes(apk)
            (root / 'android/dist/latest.json').write_text(json.dumps({
                'size': len(apk), 'sha256': hashlib.sha256(apk).hexdigest()}))
            for name in ('timetable-display.bin', 'latest.json', 'manifest.json', 'SHA256SUMS', 'RAM.json', 'STACK.json'):
                (root / 'firmware' / name).write_bytes(b'fixture')
            for board in ('waveshare-c6', 'waveshare-c6-landscape'):
                (root / 'firmware/devices' / board).mkdir(parents=True)
            site = root / 'site'
            with patch.object(sys, 'argv', [str(script), str(root / 'firmware'), str(site)]), contextlib.redirect_stdout(io.StringIO()):
                runpy.run_path(str(script), run_name='__main__')
            imports = re.compile(r'''(?:from\s*|import\s*\(|new\s+URL\()\s*["'](\./[^"']+\.js)["']''')
            for module in site.glob('*.js'):
                for relative in imports.findall(module.read_text()):
                    self.assertTrue((module.parent / relative).is_file(), f'{module.name} imports missing {relative}')
