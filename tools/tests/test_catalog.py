"""Offline topology regressions: exact calls, branches, loops and terminals."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('builder', Path(__file__).resolve().parents[1] / 'build-catalog.py')
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class CatalogPaths(unittest.TestCase):
    def generate(self, times, overrides=None):
        files = {
            'routes.txt': 'route_id,route_short_name\nR,4\n',
            'stops.txt': 'stop_id,stop_name,stop_code\nA,Central,C01\nB,Central,C02\nX,Branch,X01\nZ,Terminal,Z01\n',
            'trips.txt': 'trip_id,route_id,direction_id,trip_headsign\nT1,R,0,East\nT2,R,0,East\nT3,R,1,West\nT4,R,,Loop\n',
            'stop_times.txt': 'trip_id,stop_sequence,stop_id,pickup_type,stop_headsign\n' + times,
        }
        files.update(overrides or {})
        with tempfile.TemporaryDirectory() as temp:
            source, destination = Path(temp) / 'feed.zip', Path(temp) / 'catalog'
            with zipfile.ZipFile(source, 'w') as archive:
                for name, content in files.items():
                    archive.writestr(name, content)
            with contextlib.redirect_stdout(io.StringIO()):
                builder.build(source, destination, '20261015')
            return json.loads((destination / 'catalog.json').read_text()), (destination / 'catalog.bin').read_bytes()

    def test_numeric_sequence_deduplication_and_dropoff_only_terminal(self):
        rows = ['T1,10,Z,1,', 'T1,2,A,0,Local East', 'T2,2,A,0,Local East', 'T2,10,Z,1,']
        catalog, binary = self.generate('\n'.join(rows) + '\n')
        reordered, reordered_binary = self.generate('\n'.join(reversed(rows)) + '\n')
        self.assertEqual(binary, reordered_binary)
        self.assertEqual(catalog['patterns'], reordered['patterns'])
        self.assertEqual([s['id'] for s in catalog['stops']], ['A'])
        self.assertEqual([s['id'] for s in catalog['via_stops']], ['Z'])
        self.assertEqual(catalog['patterns'], [{'route': 0, 'direction': 0, 'headsign': 'East', 'calls': [[0, 0, 'Local East'], [1, 1]], 'trips': 2}])

    def test_opposite_platforms_branches_loop_and_unspecified_direction(self):
        catalog, _ = self.generate('T1,1,A,0,\nT1,2,Z,1,\nT2,1,A,0,\nT2,2,X,2,\nT2,3,Z,1,\nT3,1,B,0,\nT3,2,Z,1,\nT4,1,A,0,\nT4,2,X,3,Loop override\nT4,3,A,0,\n')
        self.assertEqual(len(catalog['patterns']), 4)
        ids = [s['id'] for s in catalog['stops'] + catalog['via_stops']]
        loop = next(p for p in catalog['patterns'] if p['direction'] == -1)
        self.assertEqual([ids[c[0]] for c in loop['calls']], ['A', 'X', 'A'])
        self.assertEqual(loop['calls'][1][1:], [3, 'Loop override'])
        west = next(p for p in catalog['patterns'] if p['direction'] == 1)
        self.assertEqual(ids[west['calls'][0][0]], 'B')

    def test_browser_coordinates_and_full_place_names_preserve_flash_catalogue(self):
        times = 'T1,1,A,0,\nT1,2,Z,1,\n'
        plain, binary = self.generate(times)
        extended, same_binary = self.generate(times, {
            'stops.txt': 'stop_id,stop_name,stop_code,stop_lat,stop_lon\nA,Central,C01,52.4,16.9\nZ,Terminal,Z01,nan,16.9\n'})
        self.assertEqual(binary, same_binary)
        self.assertEqual(extended['stops'][0]['place_name'], 'Central')
        self.assertEqual((extended['stops'][0]['lat'], extended['stops'][0]['lon']), (52.4, 16.9))
        self.assertNotIn('lat', extended['via_stops'][0])
        self.assertNotIn('lat', plain['stops'][0])

    def test_duplicate_stop_sequence_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'duplicate trip stop_sequence'):
            self.generate('T1,1,A,0,\nT1,1,B,0,\n')

    def test_polish_alphabetical_browsing_preserves_exact_links_and_travel_order(self):
        names = ['Żurawia', 'Zwycięstwa', 'Śródka', 'Słowiańska', 'Łąkowa', 'Lubelska',
                 'AWF', 'Admiralska', 'Źródlana', 'Ćwikłowo', 'Czarna', 'Lasek']
        lines = ['T10', '100', '10', 'T2', '2']
        catalog, binary = self.generate(
            ''.join(f'T0,{i + 1},S{i},0,\n' for i in range(len(names)))
            + ''.join(f'T{i},1,S0,0,\n' for i in range(1, len(lines))),
            {'routes.txt': 'route_id,route_short_name\n' + ''.join(f'R{i},{line}\n' for i, line in enumerate(lines)),
             'trips.txt': 'trip_id,route_id,direction_id,trip_headsign\n' + ''.join(f'T{i},R{i},0,East\n' for i in range(len(lines))),
             'stops.txt': 'stop_id,stop_name,stop_code\n' + ''.join(f'S{i},{name},P{i}\n' for i, name in enumerate(names))})
        self.assertEqual([r['line'] for r in catalog['routes']], ['2', '10', '100', 'T2', 'T10'])
        self.assertEqual([s['name'] for s in catalog['stops']],
                         ['Admiralska', 'AWF', 'Czarna', 'Ćwikłowo', 'Lasek', 'Lubelska',
                          'Łąkowa', 'Słowiańska', 'Śródka', 'Zwycięstwa', 'Źródlana', 'Żurawia'])
        # Flash records have exactly the same order, without a runtime sort/index.
        offset = 32 + len(lines) * 168
        self.assertEqual([binary[offset + i * 112:offset + i * 112 + 32].rstrip(b'\0').decode()
                          for i in range(len(names))], [s['id'] for s in catalog['stops']])
        route = next(i for i, r in enumerate(catalog['routes']) if r['id'] == 'R0')
        pattern = next(p for p in catalog['patterns'] if p['route'] == route)
        self.assertEqual([catalog['stops'][c[0]]['name'] for c in pattern['calls']], names)
        self.assertEqual({catalog['stops'][s]['id'] for r, s, _ in catalog['links'] if r == route},
                         {f'S{i}' for i in range(len(names))})


if __name__ == '__main__':
    unittest.main()
