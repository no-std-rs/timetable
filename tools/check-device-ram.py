#!/usr/bin/env python3
"""Measure the linked C3 firmware, including its radio, TLS tasks and bounded heap.

IRAM and DRAM are aliases: skip linker dummy reservations, then count the actual
allocated sections once. This is a linked-memory budget, not a hardware stack or
allocator high-water measurement.
"""
import json
from pathlib import Path
import struct
import sys


def measure(path):
    data = Path(path).read_bytes()
    if data[:7] != b'\x7fELF\x01\x01\x01':
        raise ValueError('expected a little-endian ELF32 image')
    header = struct.unpack_from('<16sHHIIIIIHHHHHH', data)
    if header[2] != 243:
        raise ValueError('expected a RISC-V image')
    sections = [struct.unpack_from('<IIIIIIIIII', data, header[6] + i * header[11]) for i in range(header[12])]
    names = sections[header[13]]
    strings = data[names[4]:names[4] + names[5]]
    selected = {}
    stack = 0
    for section in sections:
        name = strings[section[0]:].split(b'\0', 1)[0].decode()
        address, size = section[3], section[5]
        if not section[2] & 2 or name.endswith('_dummy'):
            continue
        if name == '.stack':
            stack += size
        elif 0x3FC80000 <= address < 0x3FCE0000 or 0x40380000 <= address < 0x403E0000 or 0x50000000 <= address < 0x50002000:
            selected[name] = size
    static = sum(selected.values())
    reserve = 24 * 1024
    if static + reserve > 400000 or stack < reserve:
        raise ValueError(f'C3 RAM budget failed: {static} static bytes, {stack} stack bytes; reserve {reserve}')
    return {'target': 'riscv32imc-unknown-none-elf', 'sections': selected, 'static_bytes_including_65536_byte_heap': static, 'stack_available_bytes': stack, 'stack_reserve_bytes': reserve, 'budgeted_bytes': static + reserve, 'limit_bytes': 400000, 'fits': True, 'hardware_high_water_measured': False}


if __name__ == '__main__':
    result = json.dumps(measure(sys.argv[1]), indent=2) + '\n'
    if len(sys.argv) > 2:
        Path(sys.argv[2]).write_text(result)
    print(result, end='')
