#!/usr/bin/env python3
"""Compare the parser/ciphers on RV32, without claiming ESP32 wall-clock timing."""
import argparse
import io
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def measure(directory):
    subprocess.run(['cargo', 'build', '--locked', '--release'], cwd=directory, check=True, timeout=180)
    elf = directory / 'target/riscv32imc-unknown-none-elf/release/timetable-device-cpu'
    output = subprocess.check_output([
        'qemu-system-riscv32', '-machine', 'virt', '-cpu', 'rv32', '-bios', 'none',
        '-nographic', '-no-reboot', '-icount', 'shift=0,align=off,sleep=off',
        '-kernel', str(elf)], cwd=directory, text=True, timeout=60)
    csv = [tuple(map(int, values)) for values in re.findall(
        r'CSV current rows=(\d+) sum=(\d+) retired_inst=(\d+)', output)]
    if len(csv) != 3 or any((rows, checksum) != (512, 6130) for rows, checksum, _ in csv):
        raise ValueError('unexpected parser output')
    ciphers = {}
    for name in ('AES128GCM', 'CHACHA20POLY1305'):
        rounds = [tuple(map(int, values)) for values in re.findall(
            rf'{name} record\d+ encrypt_inst=(\d+) decrypt_inst=(\d+)', output)]
        if len(rounds) != 4:
            raise ValueError(f'missing {name} rounds')
        ciphers[name] = [{'encrypt': enc, 'decrypt': dec} for enc, dec in rounds]
    zip_rounds = [dict(fixture=fixture, decoded=mode == 'true', rows=int(rows), checksum=int(checksum),
                       instructions=int(total), max_poll_instructions=int(longest), polls=int(polls))
                  for fixture, mode, rows, checksum, total, longest, polls in re.findall(
                      r'ZIP fixture=(normal|repeated) decoded=(false|true) rows=(\d+) sum=(\d+) total_inst=(\d+) max_poll_inst=(\d+) polls=(\d+)', output)]
    expected = {'normal': (512, 6130), 'repeated': (8192, 98080)}
    if (len(zip_rounds) != 4 or len({(r['fixture'], r['decoded']) for r in zip_rounds}) != 4
            or any((r['rows'], r['checksum']) != expected[r['fixture']] for r in zip_rounds)):
        raise ValueError('unexpected compressed scheduling output')
    return {'zip_scheduling': zip_rounds, 'csv': [{'rows': r, 'checksum': c, 'instructions': n} for r, c, n in csv],
            'ciphers': ciphers, 'raw': output}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', default='c7d40e7de9fbae640351d2cd08a9ac064adc4a1f')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not shutil.which('qemu-system-riscv32'):
        parser.error('install qemu-system-riscv32 (Debian/Ubuntu: qemu-system-misc)')
    baseline = subprocess.check_output(['git', 'rev-parse', '--verify', '--end-of-options',
                                        f'{args.baseline}^{{commit}}'], cwd=ROOT, text=True).strip()
    # The performance probe must use the production crypto versions, not a
    # newer dependency selected by an unlocked benchmark-only resolution.
    def versions(path):
        return {p['name']: p['version'] for p in tomllib.loads(path.read_text())['package']}
    firmware = versions(ROOT / 'firmware/Cargo.lock')
    probe = versions(ROOT / 'checks/device-cpu/Cargo.lock')
    for package in ('aes-gcm', 'aes', 'ghash', 'chacha20poly1305', 'chacha20',
                    'poly1305', 'generic-array', 'crypto-common'):
        if firmware[package] != probe[package]:
            raise ValueError(f'probe/firmware version mismatch: {package}')
    result = {'baseline_commit': baseline,
              'method': 'QEMU virt RV32IMC retired instructions, deterministic icount; not C3 cycles/power/throughput',
              'profile': 'pinned firmware compiler, opt-level=s, fat LTO, codegen-units=1; no allocator',
              'record_bytes': 16384, 'aad_bytes': 5}
    with tempfile.TemporaryDirectory(prefix='timetable-device-cpu-') as temp:
        root = Path(temp)
        archive = subprocess.check_output(['git', 'archive', baseline, 'gtfs', 'firmware/src/cooperative.rs'], cwd=ROOT)
        (root / 'baseline').mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as source:
            source.extractall(root / 'baseline', filter='data')
        (root / 'candidate/firmware/src').mkdir(parents=True)
        shutil.copy2(ROOT / 'firmware/src/cooperative.rs', root / 'candidate/firmware/src/cooperative.rs')
        shutil.copytree(ROOT / 'gtfs', root / 'candidate/gtfs',
                        ignore=shutil.ignore_patterns('target', '__pycache__'))
        for name in ('baseline', 'candidate'):
            directory = root / name / 'checks/device-cpu'
            shutil.copytree(ROOT / 'checks/device-cpu', directory,
                            ignore=shutil.ignore_patterns('target'))
            result[name] = measure(directory)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(args.output)


if __name__ == '__main__':
    main()
