#!/usr/bin/env python3
"""Describe the exact merged bytes used by the browser installer and GitHub Pages."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
CONFIG_OFFSET = 0x3F0000
CACHE_OFFSET = 0x300000


def package(directory):
    directory = Path(directory)
    binary = directory.joinpath('timetable-display.bin').read_bytes()
    if not 0x10000 <= len(binary) < CACHE_OFFSET or binary[0] != 0xE9:
        raise ValueError('expected an ESP32-C3 merged factory image below the timetable cache partition')
    # The first image is the C3 bootloader: DIO, 4 MB, 80 MHz, C3 chip id.
    if binary[2] != 2 or binary[3] != 0x2F or int.from_bytes(binary[12:14], 'little') != 5:
        raise ValueError('unexpected chip, flash mode, size, or frequency')
    partitions = {}
    for offset in range(0x8000, 0x8C00, 32):
        row = binary[offset:offset + 32]
        if row[:2] != b'\xaa\x50':
            break
        name = row[12:28].split(b'\0', 1)[0].decode('ascii')
        partitions[name] = (row[2], row[3], int.from_bytes(row[4:8], 'little'), int.from_bytes(row[8:12], 'little'))
    if partitions.get('ttcache') != (0x40, 0, CACHE_OFFSET, 0xF0000):
        raise ValueError('missing dedicated timetable cache partition')
    if partitions.get('factory') != (0, 0, 0x10000, 0x2F0000) or partitions.get('timetable') != (1, 6, CONFIG_OFFSET, 0x10000):
        raise ValueError('the settings partition does not match the browser and firmware layout')
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=normal'], cwd=ROOT, text=True))
    checksum = hashlib.sha256(binary).hexdigest()
    metadata = {
        'name': 'timetable', 'board': 'GUITION ESP32-2424S012-V1.0',
        'chip': 'ESP32-C3', 'revision': revision, 'dirty': dirty,
        'sha256': checksum, 'size': len(binary), 'flash_size': 4 * 1024 * 1024,
        'config_offset': CONFIG_OFFSET, 'config_version': 2,
    }
    # Locate the compiler-opaque ROM slot by a valid envelope, not the magic
    # string alone (the parser may also contain that string as a constant).
    slots = []
    pos = 0x10000
    while (pos := binary.find(b'TTBND001', pos)) >= 0:
        length = int.from_bytes(binary[pos + 8:pos + 12], 'little')
        payload = binary[pos + 16:pos + 16 + length]
        if 4 < length <= 1024 * 1024 - 16 and len(payload) == length and payload[4:12] == b'TTCAT001' and zlib.crc32(payload) == int.from_bytes(binary[pos + 12:pos + 16], 'little') and pos + 1024 * 1024 <= len(binary):
            slots.append(pos)
        pos += 8
    if len(slots) != 1:
        raise ValueError('expected exactly one validated immutable catalogue slot')
    metadata['catalog_slot'] = {'offset': slots[0], 'capacity': 1024 * 1024, 'format': 'TTBND001'}
    manifest = {
        'name': metadata['name'], 'version': f'main-{revision[:12]}',
        'new_install_prompt_erase': True, 'new_install_improv_wait_time': 0,
        'builds': [{'chipFamily': 'ESP32-C3', 'parts': [{'path': 'timetable-display.bin', 'offset': 0}]}],
    }
    for filename, data in [('latest.json', metadata), ('manifest.json', manifest)]:
        directory.joinpath(filename).write_text(json.dumps(data, indent=2) + '\n')
    directory.joinpath('SHA256SUMS').write_text(f'{checksum}  timetable-display.bin\n')
    print(f'Packaged {len(binary)} bytes from {revision}; sha256={checksum}; dirty={dirty}')


if __name__ == '__main__':
    package(sys.argv[1])
