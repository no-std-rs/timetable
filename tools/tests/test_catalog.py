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
    def generate(self, times):
        files = {
            'routes.txt': 'route_id,route_short_name\nR,4\n',
            'stops.txt': 'stop_id,stop_name,stop_code\nA,Central,C01\nB,Central,C02\nX,Branch,X01\nZ,Terminal,Z01\n',
            'trips.txt': 'trip_id,route_id,direction_id,trip_headsign\nT1,R,0,East\nT2,R,0,East\nT3,R,1,West\nT4,R,,Loop\n',
            'stop_times.txt': 'trip_id,stop_sequence,stop_id,pickup_type,stop_headsign\n' + times,
        }
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

    def test_duplicate_stop_sequence_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'duplicate trip stop_sequence'):
            self.generate('T1,1,A,0,\nT1,1,B,0,\n')


if __name__ == '__main__':
    unittest.main()
