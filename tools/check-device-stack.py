#!/usr/bin/env python3
"""Audit critical linked C3 frames, including nested startup and TLS paths.

Uses release DWARF CFI, not type/future sizes. This is a regression guard for
audited paths with explicit headroom, not a complete call graph or hardware peak.
Requires GNU readelf (binutils); no Python packages or nightly compiler options.
"""
import json
from pathlib import Path
import re
import struct
import subprocess
import sys


def symbols(path):
    data = Path(path).read_bytes()
    if data[:7] != b'\x7fELF\x01\x01\x01':
        raise ValueError('expected little-endian ELF32')
    header = struct.unpack_from('<16sHHIIIIIHHHHHH', data)
    if header[2] != 243:
        raise ValueError('expected RISC-V')
    sections = [struct.unpack_from('<IIIIIIIIII', data, header[6] + i * header[11]) for i in range(header[12])]
    strings = sections[header[13]]
    names = data[strings[4]:strings[4] + strings[5]]
    stack = next(s[5] for s in sections if names[s[0]:].split(b'\0', 1)[0] == b'.stack')
    result = {}
    for section in sections:
        if section[1] != 2:  # SHT_SYMTAB
            continue
        strings = sections[section[6]]
        strings = data[strings[4]:strings[4] + strings[5]]
        for offset in range(section[4], section[4] + section[5], section[9]):
            name, address, _, info, _, _ = struct.unpack_from('<IIIBBH', data, offset)
            if info & 15 == 2 and address:
                result.setdefault(address, []).append(strings[name:].split(b'\0', 1)[0].decode())
    return result, stack


def linked_frames(path):
    names, stack = symbols(path)
    output = subprocess.check_output(['readelf', '--debug-dump=frames', '--wide', str(path)], text=True)
    frames = {}
    address = None
    for line in output.splitlines():
        match = re.search(r'FDE .*pc=([0-9a-fA-F]+)\.\.', line)
        if match:
            address = int(match[1], 16)
            frames.setdefault(address, 0)
        match = re.search(r'DW_CFA_def_cfa_offset: (\d+)', line)
        if match and address:
            frames[address] = max(frames[address], int(match[1]))
    if not frames:
        raise ValueError('missing linked unwind frame metadata')
    return [(name, frames[address]) for address, aliases in names.items() if address in frames for name in aliases], stack


def audit(frames, stack):
    largest = max(size for _, size in frames)
    if largest > 16384:
        raise ValueError(f'single linked frame {largest} exceeds 16384 bytes')
    def frame(*parts):
        matches = [size for name, size in frames if all(part in name for part in parts)]
        if not matches:
            raise ValueError(f'missing audited function frame: {parts}')
        # Generic monomorphizations can differ; use the largest matching frame.
        return max(matches)

    main = frame('timetable_display', '__main')
    initialize = frame('timetable_display', 'initialize_state')
    load = frame('timetable_display4load')
    decode = frame('config', 'decode_into')
    parser = decode + sum(frame('config', kind, method) for kind, method in [
        ('ConfigSeed', 'deserialize'), ('ConfigSeed', 'visit_map'),
        ('Groups', 'deserialize'), ('Groups', 'visit_seq'),
        ('GroupSeed', 'deserialize'), ('GroupSeed', 'visit_map'),
        ('Picks', 'deserialize'), ('Picks', 'visit_seq'),
    ])
    nor_read = frame('esp_storage', 'ReadNorFlash', '4read')
    task = frame('TaskStorage', '4poll')
    timeout = frame('TimeoutFuture', '4poll')
    paths = {
        'boot_state': main + initialize + frame('create_state'),
        'boot_settings': main + initialize + load + frame('settings', 'load_banks') + parser,
        'boot_partition': main + initialize + load + frame('has_settings_partition') + nor_read,
        'boot_network': main + frame('start_network'),
        'save_validation': main + frame('timetable_display4save') + frame('config6decode') + parser,
        'runtime_inflate': task + timeout + frame('gtfs', 'next_entry'),
        'runtime_tls': task + timeout + frame('embedded_tls', 'TlsConnection', '4open')
            + frame('process_server_verify') + frame('pki', 'verify_signature')
            + frame('der', 'from_der') + frame('ContextSpecific', 'decode_explicit')
            + frame('SliceReader', '6decode') + frame('header', 'Header', '6decode')
            + frame('Tag', 'decode_with_constructed_bit') + frame('parse_parts')
            + frame('Reader', 'read_byte') + frame('Reader', 'read_into'),
    }
    margin = 4096
    for name, size in paths.items():
        limit = 24576 if name.startswith('boot_') else stack
        if size + margin > limit:
            raise ValueError(f'{name}: {size} linked frame bytes + {margin} headroom exceeds {limit}')
    return {
        'target': 'riscv32imc-unknown-none-elf',
        'method': 'linked DWARF CFI; conservative sums on audited paths',
        'paths_bytes': paths, 'headroom_bytes': margin,
        'boot_limit_bytes': 24576, 'stack_available_bytes': stack,
        'largest_frame_bytes': largest, 'single_frame_limit_bytes': 16384,
        'complete_call_graph': False, 'hardware_high_water_measured': False,
        'fits_audited_paths': True,
    }


if __name__ == '__main__':
    result = audit(*linked_frames(sys.argv[1]))
    encoded = json.dumps(result, indent=2) + '\n'
    if len(sys.argv) > 2:
        Path(sys.argv[2]).write_text(encoded)
    print(encoded, end='')
