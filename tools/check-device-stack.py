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
    # In the pinned compiler these same network frames can be outlined as
    # TimeoutFuture::poll or Http::request. Verify the retained call boundary;
    # sum both families if they coexist, so nesting is never undercounted.
    request_frames = []
    for parts in [('TimeoutFuture', '4poll'), ('timetable_display9transport4Http7request',)]:
        matches = [size for name, size in frames if all(part in name for part in parts)]
        if matches:
            request_frames.append(max(matches))
    if not request_frames:
        raise ValueError('missing audited timeout/request frame')
    request = sum(request_frames)
    # The mirror pipeline adds outlined coroutine callers above the transport.
    # A fetch monomorphization inlines request and calls TLS.open directly;
    # the official branch retains request. They are alternatives, not nested.
    # The compiler can retain Select::poll, network_session, or both as the
    # actual network caller above update_once. Count all surviving families.
    session_frames = []
    for parts in [('embassy_futures', 'select', 'Select', '4poll'),
                  ('__network_service_task_inner_function', 'closure'),
                  ('timetable_display15network_session', 'closure'),
                  ('wake_gate', 'Gated', '4poll')]:
        outlined = [size for name, size in frames if all(part in name for part in parts)]
        if outlined:
            session_frames.append(max(outlined))
    if not session_frames:
        raise ValueError('missing audited function frame: network session/select')
    # Select can be wholly inlined into network_session or Gated::poll. Gated
    # runner/query polls are siblings: count the largest gate once. If other
    # caller families survive too, retain them in this conservative envelope.
    session = sum(session_frames)
    lower_tls = sum(frame(*parts) for parts in [
        ('process_server_verify',), ('pki', 'verify_signature'),
        ('der', 'from_der'), ('ContextSpecific', 'decode_explicit'),
        ('SliceReader', '6decode'), ('header', 'Header', '6decode'),
        ('Tag', 'decode_with_constructed_bit'), ('parse_parts',),
        ('Reader', 'read_byte'), ('Reader', 'read_into'),
    ])
    paths = {
        'boot_state': main + initialize + frame('create_state'),
        'boot_settings': main + initialize + load + frame('settings', 'load_banks') + parser,
        'boot_partition': main + initialize + load + frame('has_settings_partition') + nor_read,
        'boot_network': main + frame('start_network'),
        'save_validation': main + frame('timetable_display4save') + frame('config6decode') + parser,
        'runtime_network_setup': task + session + frame('open_network'),
    }
    scoped = [size for name, size in frames if 'download7refresh' in name and 'closure' in name]
    cached = [size for name, size in frames if 'cache5build' in name and 'closure' in name]
    if cached:
        # Cache profile: refresh/session are inlined into update_once and
        # download::mirror. These are real retained callers in the pinned ELF.
        # LLVM may inline update_once completely into the task/session polls
        # already counted above. Count every surviving coroutine/poll boundary;
        # an absent outline contributes no additional stack frame.
        update = max((size for name, size in frames
                      if 'timetable_display11update_once' in name and 'closure' in name), default=0)
        worker = task + session + update + max(scoped, default=0)
        base = worker + frame('download6mirror', 'closure')
        transport = frame('transport', 'mirror_tables', 'closure')
        # The normal query may be outlined as its coroutine or our explicit
        # poll boundary. Count both if both survive; neither is part of TLS.open.
        normal_frames = []
        for parts in [('download13mirror_tables', 'closure'), ('download11poll_mirror',)]:
            outlined = [size for name, size in frames if all(p in name for p in parts)]
            if outlined:
                normal_frames.append(max(outlined))
        if not normal_frames:
            raise ValueError('missing audited normal mirror query frame')
        normal = sum(normal_frames)
        coverage = frame('download', 'coverage_dates', 'closure')
        timed = frame('transport', 'Timed', '5fetch', 'closure')
        timed = max(timed, max((size for name, size in frames
                               if all(p in name for p in ('transport', 'Timed', '9unchanged', 'closure'))), default=0))
        client_frames = []
        for method in ('5fetch', '11fetch_inner', '9unchanged'):
            outlined = [size for name, size in frames
                        if all(p in name for p in ('http_session', 'Connection', method, 'closure'))]
            if outlined:
                client_frames.append(max(outlined))
        if not client_frames:
            raise ValueError('missing audited HTTP session frame')
        http = timed + sum(client_frames)
        table_worker = max(max(cached), transport + normal)
        mirror_open = sum(max((size for name, size in frames
                               if all(part in name for part in ('download', 'Mirror', method, 'closure'))), default=0)
                          for method in ('4open', '19open_with_validator'))
        outlined_open = [size for name, size in frames
                         if all(part in name for part in ('embedded_tls', 'TlsConnection', '4open'))]
        stages = max(frame('feed', 'Builder', stage) for stage in
                     ('calendars_from', 'trips_from', 'departures_from'))
        scan_context = max(coverage, stages)
        flash_read = (frame('CacheFlash', '4read') + nor_read
                      + frame('spiflash_read'))
        flash_write = sum(frame(*parts) for parts in [
            ('CacheFlash', '5write'), ('esp_storage', 'NorFlash', '5write'),
            ('internal_write',), ('spiflash_write',),
        ])
        flash_erase = sum(frame(*parts) for parts in [
            ('CacheFlash', '5erase'), ('esp_storage', 'NorFlash', '5erase'),
            ('internal_erase_sector',), ('spiflash_erase_sector',),
        ])
        writer = sum(frame('cache', 'Writer', method) for method in ('3row', '6append', '5flush'))
        # Cold replay runs before open_network, without update_once/mirror.
        # Count optional coroutine outlines if compiler decisions change.
        cold = task + session + max(
            (size for name, size in frames if 'restore_saved_board' in name and 'closure' in name),
            default=0)
        query_sources = max(
            (size for name, size in frames if 'query_sources' in name and 'closure' in name),
            default=0)
        # ZIP constructor and row callback/flash programming are alternatives:
        # Reader::record has returned before the CSV visitor writes a row.
        paths.update({
            'boot_partition': main + initialize + load + frame('has_settings_partition') + frame('read_partition_table') + frame('spiflash_read'),
            'boot_cache_partition': main + frame('has_cache_partition') + frame('read_partition_table') + frame('spiflash_read'),
            'runtime_settings_copy': worker + frame('copy_query') + frame('config', 'Config', '10clone_from'),
            'runtime_mirror_record': base + http + table_worker + scan_context + frame('gtfs', 'zip', '6record', 'closure'),
            'runtime_http_headers': base + http + table_worker + scan_context + frame('response', 'Response', '7headers'),
            'runtime_conditional_headers': base + http + frame('response', 'Response', '7headers') + sum(
                max((size for name, size in frames if part in name and 'closure' in name), default=0)
                for part in ('cache7matches', 'cache13mirror_tables')),
            # Optional frequencies are decoded during Mirror::open, before
            # coverage/calendar scans. Keep HTTP outlines in this conservative
            # envelope even when that specialization is inlined into open.
            'runtime_frequency_record': base + table_worker + mirror_open + http + frame('gtfs', 'zip', '6record', 'closure'),
            'runtime_frequency_headers': base + table_worker + mirror_open + http + frame('response', 'Response', '7headers'),
            'runtime_official_inflate': worker + http + frame('gtfs', 'next_entry'),
            'runtime_tls': base + transport + request + max(outlined_open, default=0) + lower_tls,
            'runtime_cache_flash': base + http + max(cached) + stages + writer + max(flash_read, flash_write, flash_erase),
            'runtime_cache_replay': base + frame('cache7restore', 'closure') + query_sources + stages + frame('csv', '4next', 'closure') + frame('csv', '8raw_next', 'closure') + flash_read,
            'runtime_cache_validation': base + frame('cache4load', 'closure') + frame('cache8validate', 'closure') + flash_read,
            'runtime_cache_cold_validation': cold + frame('cache4load', 'closure') + frame('cache8validate', 'closure') + flash_read,
            'runtime_cache_cold_replay': cold + frame('cache7restore', 'closure') + query_sources + stages + frame('csv', '4next', 'closure') + frame('csv', '8raw_next', 'closure') + flash_read,
        })
        # Journey scratch is allocated only after TLS verification. Its query
        # and table-scan coroutines remain live during record/header parsing;
        # count every surviving wrapper family, using the largest specialization.
        journey = [size for name, size in frames
                   if 'journey' in name and 'Search' in name and ('5query' in name or 'query_using' in name) and 'closure' in name
                   and 'drop_in_place' not in name]
        if journey:
            phase = max((size for name, size in frames
                         if 'journey' in name and 'Search' in name and 'closure' in name
                         and 'drop_in_place' not in name
                         and any(method in name for method in
                                 ('collect_calls', 'metadata', 'stations', 'group_places', 'publish'))), default=0)
            wrappers = sum(max((size for name, size in frames
                                if all(part in name for part in parts)
                                and 'drop_in_place' not in name), default=0)
                           for parts in [('http_session', 'journey_plan', 'closure'),
                                         ('transport', 'journey_plan', 'closure'),
                                         ('feed', 'journeys_with', 'closure')])
            query = base + transport + normal + wrappers + max(journey)
            paths.update({
                'runtime_journey_scratch': base + transport + normal + wrappers
                    + frame('transport', 'journey_scratch'),
                'runtime_journey_record': query + phase + http
                    + frame('gtfs', 'zip', '6record', 'closure'),
                'runtime_journey_headers': query + phase + http
                    + frame('response', 'Response', '7headers'),
                'runtime_journey_official': query + phase + http
                    + frame('gtfs', 'next_entry'),
            })
    elif scoped:
        # Pinned scoped-session ELF: update_once and session are inlined into
        # Select/refresh. TLS.open is in TimeoutFuture::poll, BEFORE the
        # SessionOperation starts reading tables. These paths are alternatives.
        outlined_update = [size for name, size in frames
                           if 'timetable_display11update_once' in name and 'closure' in name]
        outlined_open = [size for name, size in frames
                         if all(part in name for part in ('embedded_tls', 'TlsConnection', '4open'))]
        base = task + session + max(scoped) + max(outlined_update, default=0)
        # Timed awaits Connection, which awaits headers or a body operation.
        # Specializations can inline either layer into their caller. Include
        # both outlined frame families as a conservative envelope anyway, so
        # later outlining cannot silently weaken the transfer-path guard.
        http = (frame('transport', 'Timed', '5fetch', 'closure')
                + frame('http_session', 'Connection', '5fetch', 'closure'))
        paths.update({
            'runtime_settings_copy': task + session + frame('copy_query') + frame('config', 'Config', '10clone_from'),
            'runtime_mirror_record': base + http + frame('download13mirror_tables', 'closure') + frame('gtfs', 'zip', '6record', 'closure'),
            'runtime_official_inflate': base + http + frame('gtfs', 'next_entry'),
            'runtime_http_headers': base + http + frame('download13mirror_tables', 'closure') + frame('response', 'Response', '7headers'),
            'runtime_tls': base + request + max(outlined_open, default=0) + lower_tls,
        })
    else:
        # Legacy connection-per-request profile remains covered for regressions.
        update_frames = []
        for parts in [('timetable_display11update_once', 'closure'), ('timetable_display11poll_update',)]:
            outlined = [size for name, size in frames if all(p in name for p in parts)]
            if outlined:
                update_frames.append(max(outlined))
        if not update_frames:
            raise ValueError('missing audited update poll frame')
        update = sum(update_frames)
        worker = session + update
        mirror = frame('timetable_firmware', 'download6mirror', 'closure')
        fetch = frame('transport', 'download', '5fetch', 'closure')
        runtime_callers = task + worker + max(request, mirror + fetch)
        paths.update({
            'runtime_settings_copy': task + worker + frame('copy_query'),
            'runtime_inflate': runtime_callers + max(frame('gtfs', 'next_entry'), frame('gtfs', 'zip', '4data')),
            'runtime_tls': runtime_callers + frame('embedded_tls', 'TlsConnection', '4open') + lower_tls,
        })
    margin = 4096
    publish = max((size for name, size in frames if 'publish_departures' in name), default=0)
    if publish:
        # Cache publication happens before/after TLS, never inside verification.
        paths['runtime_publish'] = task + session + max(
            (size for name, size in frames if 'closure' in name and
             any(caller in name for caller in ('update_once', 'restore_saved_board'))),
            default=0) + publish
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
    if "riscv32imac-unknown-none-elf" in sys.argv[1]:
        result["target"] = "riscv32imac-unknown-none-elf"
    encoded = json.dumps(result, indent=2) + '\n'
    if len(sys.argv) > 2:
        Path(sys.argv[2]).write_text(encoded)
    print(encoded, end='')
