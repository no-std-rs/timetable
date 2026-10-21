"""Release identity/geometry regressions independent of compiler artifacts."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zlib

spec = importlib.util.spec_from_file_location('package_device', Path(__file__).parents[1] / 'package-device.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


def factory(chip, flash_byte):
    binary = bytearray(0x120000)
    binary[0] = binary[0x10000] = 0xE9
    binary[2] = 2
    binary[3] = flash_byte
    binary[12:14] = binary[0x1000C:0x1000E] = chip.to_bytes(2, 'little')
    for i, (name, typ, subtype, start, size) in enumerate([
        ('factory', 0, 0, 0x10000, 0x2F0000),
        ('ttcache', 0x40, 0, 0x300000, 0xF0000),
        ('timetable', 1, 6, 0x3F0000, 0x10000),
    ]):
        offset = 0x8000 + i * 32
        binary[offset:offset + 4] = bytes([0xaa, 0x50, typ, subtype])
        binary[offset + 4:offset + 8] = start.to_bytes(4, 'little')
        binary[offset + 8:offset + 12] = size.to_bytes(4, 'little')
        binary[offset + 12:offset + 12 + len(name)] = name.encode()
    payload = (8).to_bytes(4, 'little') + b'TTCAT001'
    envelope = b'TTBND001' + len(payload).to_bytes(4, 'little') + zlib.crc32(payload).to_bytes(4, 'little') + payload
    binary[0x10020:0x10020 + len(envelope)] = envelope
    return binary


class PackageTests(unittest.TestCase):
    def test_each_board_keeps_its_identity_after_partition_parsing(self):
        for board, name, chip, flash, geometry in [
            ('guition-c3', 'GUITION ESP32-2424S012-V1.0', 5, 0x2F, 4194304),
            ('waveshare-c6', 'Waveshare ESP32-C6-Touch-LCD-1.47', 13, 0x30, 8388608),
        ]:
            with self.subTest(board=board), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                path.joinpath('timetable-display.bin').write_bytes(factory(chip, flash))
                with patch.object(package.subprocess, 'check_output', side_effect=['a' * 40, '']):
                    package.package(path, board)
                metadata = json.loads(path.joinpath('latest.json').read_text())
                manifest = json.loads(path.joinpath('manifest.json').read_text())
                self.assertEqual(metadata['board'], name)
                self.assertEqual(metadata['flash_size'], geometry)
                self.assertEqual(manifest['builds'][0]['chipFamily'], metadata['chip'])
                self.assertFalse(metadata['dirty'])

    def test_wrong_application_chip_or_frequency_never_publishes(self):
        for field in ['app', 'frequency']:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                image = factory(13, 0x30)
                image[0x1000C if field == 'app' else 3] = 5 if field == 'app' else 0x3F
                path = Path(directory)
                path.joinpath('timetable-display.bin').write_bytes(image)
                with self.assertRaises(ValueError):
                    package.package(path, 'waveshare-c6')
                self.assertFalse(path.joinpath('latest.json').exists())
