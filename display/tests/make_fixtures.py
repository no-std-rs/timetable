"""Synthetic display edge cases: next-day service, labels and boarding links."""
from pathlib import Path
import importlib.util
import zipfile

root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('catalog_builder', root / 'tools/build-catalog.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
destination = Path(__file__).parent / 'fixtures'
name = 'A  very long platform name ' + 'ą' * 30
files = {
    'routes.txt': 'route_id,route_short_name,route_type\nR4,4,0\nR5,4,0\n',
    'stops.txt': f'stop_id,stop_name,stop_code\nS1,{name},\nS2,Terminal,T02\nS3,Unknown direction,T02\n',
    'trips.txt': 'route_id,service_id,trip_id,trip_headsign,direction_id\nR4,TODAY,T1,Trip branch,0\nR4,TOMORROW,T2,Trip branch,0\nR5,TODAY,T3,Unknown,\n',
    'calendar.txt': 'service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\nTODAY,1,1,1,1,1,1,0,20261015,20261031\n',
    'calendar_dates.txt': 'service_id,date,exception_type\nTOMORROW,20261018,1\n',
    'stop_times.txt': 'trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,stop_headsign\nT1,23:45:00,23:45:00,S1,1,0,Platform branch\nT1,23:55:00,23:55:00,S2,2,1,\nT2,00:30:00,00:30:00,S1,1,0,Platform branch\nT3,23:50:00,23:50:00,S3,1,0,\n',
}
def write_archive(name, tables):
    with zipfile.ZipFile(destination / name, 'w') as archive:
        for path, content in tables.items():
            info = zipfile.ZipInfo(path, date_time=(2026, 10, 17, 0, 0, 0))
            # Stored members make committed wire fixtures independent of zlib.
            # The shared gtfs fixture suite separately exercises deflate streams.
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = 3
            info.create_version = 20
            info.extract_version = 20
            info.external_attr = 0o600 << 16
            archive.writestr(info, content.encode())


write_archive('edge.zip', files)
write_archive('spring-dst.zip', {
    path: content.replace('20261015', '20260328')
                 .replace('20261031', '20260329')
                 .replace('20261018', '20260329')
    for path, content in files.items()
})
long_trip = 'a' * 32
lookup = dict(files)
lookup['trips.txt'] += f'R4,TODAY,{long_trip},Trip branch,0\n'
rows = [
    (long_trip, '23:00:00'), (long_trip, '23:01:00'),
    ('miss', '23:02:00'), ('miss', '23:03:00'),
    ('T1', '23:04:00'), (long_trip + 'x', '23:05:00'),
    (long_trip + 'x', '23:06:00'), ('T1', '23:07:00'),
    (long_trip, '23:08:00'), ('T2', '00:30:00'),
]
for variant, ordered in [('grouped', sorted(rows)), ('shuffled', rows)]:
    lookup['stop_times.txt'] = (
        'trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,stop_headsign\n'
        + ''.join(f'{trip},{at},{at},S1,{i + 1},0,Platform branch\n'
                  for i, (trip, at) in enumerate(ordered)))
    write_archive(f'lookup-{variant}.zip', lookup)
module.build(destination / 'edge.zip', destination, '20261017')
