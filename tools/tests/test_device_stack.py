"""The transport's outlined callers must count toward its stack bound."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    'device_stack', Path(__file__).parents[1] / 'check-device-stack.py')
device_stack = importlib.util.module_from_spec(spec)
spec.loader.exec_module(device_stack)


class StackAuditTest(unittest.TestCase):
    def frames(self):
        # Runtime frame sizes from the pinned release ELF. Other audited paths
        # are deliberately small: this fixture isolates the nested TLS bound.
        frames = [(name, 128) for name in [
            'timetable_display __main', 'timetable_display initialize_state',
            'timetable_display4load', 'config decode_into',
            'esp_storage ReadNorFlash 4read', 'create_state',
            'settings load_banks', 'has_settings_partition', 'start_network',
            'timetable_display4save', 'config6decode', 'open_network', 'copy_query',
        ]]
        for kind, method in [
            ('ConfigSeed', 'deserialize'), ('ConfigSeed', 'visit_map'),
            ('Groups', 'deserialize'), ('Groups', 'visit_seq'),
            ('GroupSeed', 'deserialize'), ('GroupSeed', 'visit_map'),
            ('Picks', 'deserialize'), ('Picks', 'visit_seq'),
        ]:
            frames.append((f'config {kind} {method}', 128))
        frames += [
            ('TaskStorage 4poll', 80),
            ('embassy_futures select Select 4poll', 2560),
            ('timetable_display11update_once closure', 3488),
            ('timetable_display9transport4Http7request', 1904),
            ('timetable_firmware download6mirror closure', 4560),
            ('transport download 5fetch closure', 1376),
            ('gtfs next_entry', 10688), ('gtfs zip 4data', 10640),
            ('embedded_tls TlsConnection 4open', 1904),
            ('process_server_verify', 14560), ('pki verify_signature', 2128),
            ('der from_der', 9296), ('ContextSpecific decode_explicit', 4528),
            ('SliceReader 6decode', 1616), ('header Header 6decode', 208),
            ('Tag decode_with_constructed_bit', 80), ('parse_parts', 96),
            ('Reader read_byte', 128), ('Reader read_into', 80),
        ]
        return frames

    def test_nested_callers_and_exact_headroom(self):
        result = device_stack.audit(self.frames(), 50784)
        self.assertEqual(result['paths_bytes']['runtime_tls'], 46688)
        self.assertEqual(result['paths_bytes']['runtime_inflate'], 22752)
        with self.assertRaisesRegex(ValueError, 'runtime_tls'):
            device_stack.audit(self.frames(), 50783)

    def test_missing_select_or_oversized_frame_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'missing audited function'):
            device_stack.audit([(n, s) for n, s in self.frames()
                                if 'Select' not in n], 51968)
        with self.assertRaisesRegex(ValueError, 'single linked frame'):
            device_stack.audit(self.frames() + [('unexpected_large_frame', 16385)], 51968)

    def test_additional_outlined_session_is_not_ignored(self):
        frames = self.frames() + [('timetable_display15network_session closure', 2048)]
        with self.assertRaisesRegex(ValueError, 'runtime_tls'):
            device_stack.audit(frames, 51968)

    def test_outlined_wake_gate_counts_once_on_runtime_paths(self):
        baseline = device_stack.audit(self.frames(), 60000)['paths_bytes']
        frames = self.frames() + [('wake_gate Gated Future 4poll', 1024)]
        grown = device_stack.audit(frames, 60000)['paths_bytes']
        for path, size in baseline.items():
            self.assertEqual(grown[path], size + (1024 if path.startswith('runtime_') else 0))
        with self.assertRaisesRegex(ValueError, 'runtime_tls'):
            device_stack.audit(frames, 50784)

    def test_wake_gate_can_be_the_retained_session_poll_boundary(self):
        baseline = device_stack.audit(self.frames(), 60000)['paths_bytes']
        frames = [(n, s) for n, s in self.frames() if 'Select' not in n]
        frames += [('wake_gate Gated Future 4poll', 2352),
                   ('wake_gate Gated OtherFuture 4poll', 656)]
        grown = device_stack.audit(frames, 60000)['paths_bytes']
        for path, size in baseline.items():
            self.assertEqual(grown[path], size + (2352 - 2560 if path.startswith('runtime_') else 0))

    def test_publication_counts_cold_replay_caller_separately_from_tls(self):
        frames = self.frames() + [('publish_departures', 4096)]
        baseline = device_stack.audit(frames, 60000)['paths_bytes']
        frames.append(('restore_saved_board closure', 4096))
        grown = device_stack.audit(frames, 60000)['paths_bytes']
        self.assertEqual(grown['runtime_publish'], 80 + 2560 + 4096 + 4096)
        self.assertEqual(grown['runtime_tls'], baseline['runtime_tls'])

    def scoped_frames(self):
        old = self.frames()
        frames = [(name, size) for name, size in old if not any(part in name for part in
                  ('update_once', 'Http7request', 'download6mirror', 'transport download', 'TlsConnection', 'TaskStorage', 'Select'))]
        return frames + [
            ('TaskStorage 4poll', 112), ('embassy_futures select Select 4poll', 4176),
            ('timetable_firmware download7refresh closure', 1648),
            ('timetable_firmware download13mirror_tables closure', 6800),
            ('TimeoutFuture 4poll', 1904), ('gtfs zip 6record closure', 13152),
            ('http_session Connection 5fetch closure', 1920),
            ('transport Timed 5fetch closure', 1328),
            ('response Response 7headers', 2096), ('config Config 10clone_from', 2576),
        ]

    def test_scoped_handshake_is_separate_from_member_inflation(self):
        result = device_stack.audit(self.scoped_frames(), 44656)
        self.assertEqual(result['paths_bytes']['runtime_tls'], 40560)
        self.assertEqual(result['paths_bytes']['runtime_mirror_record'], 29136)
        self.assertEqual(result['paths_bytes']['runtime_official_inflate'], 19872)
        self.assertEqual(result['paths_bytes']['runtime_http_headers'], 18080)
        with self.assertRaisesRegex(ValueError, 'runtime_tls'):
            device_stack.audit(self.scoped_frames(), 44655)

    def test_scoped_profile_counts_new_outlined_callers(self):
        for name in ('timetable_display11update_once closure', 'embedded_tls TlsConnection 4open'):
            with self.assertRaisesRegex(ValueError, 'runtime_tls'):
                device_stack.audit(self.scoped_frames() + [(name, 1024)], 44656)

    def test_each_http_wrapper_remains_live_in_all_transfer_paths(self):
        baseline = device_stack.audit(self.scoped_frames(), 50568)['paths_bytes']
        for wrapper in ('http_session Connection', 'transport Timed'):
            frames = [(name, size + 1024 if wrapper in name else size)
                      for name, size in self.scoped_frames()]
            grown = device_stack.audit(frames, 50568)['paths_bytes']
            for path in ('runtime_mirror_record', 'runtime_official_inflate', 'runtime_http_headers'):
                self.assertEqual(grown[path], baseline[path] + 1024)
            self.assertEqual(grown['runtime_tls'], baseline['runtime_tls'])


class CacheStackAuditTest(StackAuditTest):
    def cached_frames(self):
        frames = [(n, s) for n, s in self.frames() if not any(part in n for part in
                  ('TaskStorage', 'Select', 'update_once', 'Http7request', 'TlsConnection', 'download6mirror', 'transport download'))]
        return frames + [
            ('TaskStorage 4poll', 80), ('embassy_futures select Select 4poll', 2544),
            ('timetable_display11update_once closure', 3616),
            ('timetable_firmware download6mirror closure', 1920),
            ('transport mirror_tables closure', 256), ('download13mirror_tables closure', 5824),
            ('cache5build closure', 5840), ('download coverage_dates closure', 4928),
            ('gtfs zip 6record closure', 13152), ('TimeoutFuture 4poll', 1904),
            ('transport Timed 5fetch closure', 1328), ('http_session Connection 5fetch closure', 1920),
            ('response Response 7headers', 2096), ('config Config 10clone_from', 2576),
            ('CacheFlash 4read', 32), ('esp_storage ReadNorFlash 4read', 4144), ('spiflash_read', 16),
            ('CacheFlash 5write', 32), ('esp_storage NorFlash 5write', 4144), ('internal_write', 32), ('spiflash_write', 16),
            ('CacheFlash 5erase', 32), ('esp_storage NorFlash 5erase', 32), ('internal_erase_sector', 16), ('spiflash_erase_sector', 16),
            ('cache Writer 3row', 96), ('cache Writer 6append', 80), ('cache Writer 5flush', 336),
            ('feed Builder calendars_from', 752), ('feed Builder trips_from', 304), ('feed Builder departures_from', 400),
            ('cache7restore closure', 512), ('cache4load closure', 160), ('cache8validate closure', 352),
            ('csv 4next closure', 80), ('csv 8raw_next closure', 64),
            ('has_cache_partition', 3136), ('read_partition_table', 4240),
        ]

    def test_cache_profile_covers_flash_replay_and_exact_tls_headroom(self):
        paths = device_stack.audit(self.cached_frames(), 47136)['paths_bytes']
        self.assertEqual(paths['runtime_tls'], 43040)
        self.assertEqual(paths['runtime_mirror_record'], 35568)
        self.assertEqual(paths['runtime_cache_flash'], 22736)
        self.assertEqual(paths['runtime_cache_replay'], 13760)
        self.assertEqual(paths['runtime_cache_validation'], 12864)
        with self.assertRaisesRegex(ValueError, 'runtime_tls'):
            device_stack.audit(self.cached_frames(), 47135)

    def test_frequency_open_frame_is_included_and_growth_fails_the_guard(self):
        frames = self.cached_frames() + [('download Mirror 4open closure', 9936)]
        paths = device_stack.audit(frames, 49192)['paths_bytes']
        baseline = device_stack.audit(self.cached_frames(), 49192)['paths_bytes']
        for path in ('runtime_frequency_record', 'runtime_frequency_headers'):
            self.assertEqual(paths[path], baseline[path] + 9936)
        with self.assertRaisesRegex(ValueError, 'runtime_frequency_record'):
            device_stack.audit(self.cached_frames() + [('download Mirror 4open closure', 16000)], 49192)

    def test_flash_frames_cannot_grow_without_being_counted(self):
        frames = [(n, 12000 if n in ('CacheFlash 5write', 'esp_storage NorFlash 5write', 'cache Writer 3row') else s)
                  for n, s in self.cached_frames()]
        with self.assertRaisesRegex(ValueError, 'runtime_cache_flash'):
            device_stack.audit(frames, 49192)

    def test_cached_read_growth_reaches_replay_and_validation(self):
        baseline = device_stack.audit(self.cached_frames(), 49192)['paths_bytes']
        grown = device_stack.audit([(n, s+1024 if n == 'CacheFlash 4read' else s)
                                  for n, s in self.cached_frames()], 49192)['paths_bytes']
        for path in ('runtime_cache_replay', 'runtime_cache_validation',
                     'runtime_cache_cold_replay', 'runtime_cache_cold_validation'):
            self.assertEqual(grown[path], baseline[path]+1024)
        self.assertEqual(grown['runtime_tls'], baseline['runtime_tls'])

    def test_cached_profile_counts_optional_outlined_refresh_and_tls_open(self):
        for name in ('download7refresh closure', 'embedded_tls TlsConnection 4open'):
            with self.assertRaisesRegex(ValueError, 'runtime_tls'):
                device_stack.audit(self.cached_frames() + [(name, 1024)], 47136)


    def test_exact_preflight_frames_reach_record_and_header_bounds(self):
        baseline = device_stack.audit(self.cached_frames(), 49192)['paths_bytes']
        frames = [(n, 5024 if 'departures_from' in n else s) for n, s in self.cached_frames()]
        grown = device_stack.audit(frames, 49192)['paths_bytes']
        for path in ('runtime_mirror_record', 'runtime_http_headers'):
            self.assertEqual(grown[path], baseline[path] + 96)
        self.assertEqual(grown['runtime_tls'], baseline['runtime_tls'])


    def test_explicit_poll_boundary_counts_the_normal_query_frame(self):
        frames = [(n.replace('download13mirror_tables closure', 'download11poll_mirror'), s)
                  for n, s in self.cached_frames()]
        self.assertEqual(device_stack.audit(frames, 49192)['paths_bytes'],
                         device_stack.audit(self.cached_frames(), 49192)['paths_bytes'])
        grown = device_stack.audit(frames + [('download13mirror_tables closure', 1024)], 49192)['paths_bytes']
        baseline = device_stack.audit(frames, 49192)['paths_bytes']
        self.assertEqual(grown['runtime_mirror_record'], baseline['runtime_mirror_record'] + 1024)


    def test_select_inlined_into_network_session_keeps_the_caller_frame(self):
        frames = [(n, s) for n, s in self.cached_frames() if 'select Select' not in n]
        frames.append(('timetable_display15network_session closure', 2592))
        result = device_stack.audit(frames, 49192)
        self.assertEqual(result['paths_bytes']['runtime_tls'], 43088)
        with self.assertRaisesRegex(ValueError, 'runtime_tls'):
            device_stack.audit(frames, 47183)
        with self.assertRaisesRegex(ValueError, 'missing audited function'):
            device_stack.audit([(n, s) for n, s in frames if 'network_session' not in n], 49192)


    def test_cold_cache_counts_its_own_callers_and_optional_outlines(self):
        baseline = device_stack.audit(self.cached_frames(), 49192)['paths_bytes']
        grown = device_stack.audit(self.cached_frames() + [
            ('restore_saved_board closure', 1024), ('cache query_sources closure', 512),
        ], 49192)['paths_bytes']
        self.assertEqual(grown['runtime_cache_cold_validation'], baseline['runtime_cache_cold_validation'] + 1024)
        self.assertEqual(grown['runtime_cache_cold_replay'], baseline['runtime_cache_cold_replay'] + 1536)
        self.assertEqual(grown['runtime_cache_replay'], baseline['runtime_cache_replay'] + 512)
        # update_once is not a parent of the early cold-cache branch.
        frames = [(n, s + 512 if 'update_once' in n else s) for n, s in self.cached_frames()]
        later = device_stack.audit(frames, 49192)['paths_bytes']
        self.assertEqual(later['runtime_cache_cold_replay'], baseline['runtime_cache_cold_replay'])
        self.assertEqual(later['runtime_cache_replay'], baseline['runtime_cache_replay'] + 512)
