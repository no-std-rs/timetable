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
