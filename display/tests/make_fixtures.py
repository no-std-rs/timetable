"""Synthetic display edge cases: next-day service, labels and boarding links."""
from pathlib import Path
import importlib.util
import zipfile
import struct

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
def write_archive(name, tables, updated=None):
    with zipfile.ZipFile(destination / name, 'w') as archive:
        for path, content in tables.items():
            info = zipfile.ZipInfo(path, date_time=(2026, 10, 17, 0, 0, 0))
            # Stored members make committed wire fixtures independent of zlib.
            # The shared gtfs fixture suite separately exercises deflate streams.
            if updated is not None:
                info.extra = struct.pack('<HHBi', 0x5455, 5, 1, updated)
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = 3
            info.create_version = 20
            info.extract_version = 20
            info.external_attr = 0o600 << 16
            archive.writestr(info, content.encode())


write_archive('edge.zip', files)
write_archive('mirror.zip', {**files, 'feed_info.txt': 'feed_start_date,feed_end_date\n20261015,20261031\n'})
write_archive('mirror-start-only.zip', {**files, 'feed_info.txt': 'feed_start_date,feed_end_date\n20261001,\n'})
write_archive('mirror-end-only.zip', {**files, 'feed_info.txt': 'feed_start_date,feed_end_date\n,20261115\n'})
write_archive('mirror-tomorrow.zip', {**files, 'feed_info.txt': 'feed_start_date,feed_end_date\n20261018,20261031\n'})
cache_files = {**files, 'feed_info.txt': 'feed_start_date,feed_end_date\n20261015,20261031\n'}
cache_files['calendar.txt'] += 'ALWAYS,1,1,1,1,1,1,1,20261015,20261031\n'
cache_files['trips.txt'] += 'R4,ALWAYS,DAILY,Daytime,0\n'
cache_files['stop_times.txt'] += 'DAILY,12:00:00,12:00:00,S1,1,0,Daytime\nDAILY,26:15:00,26:15:00,S1,2,0,Nighttime\n'
write_archive('cache-full.zip', cache_files)
write_archive('cache-long-line.zip', {**cache_files, 'routes.txt': cache_files['routes.txt'].replace('R4,4,0', 'R4,ABCDEFGHI,0')})
write_archive('cache-unrelated.zip', {**cache_files, 'trips.txt': cache_files['trips.txt'].replace('R5,TODAY,T3,Unknown,', 'R5,TODAY,T3,Unused changed,')})
write_archive('cache-changed.zip', {**cache_files, 'stop_times.txt': cache_files['stop_times.txt'].replace('Platform branch', 'Changed branch')})
write_archive('cache-priority-old.zip', {**cache_files, 'stop_times.txt': cache_files['stop_times.txt'].replace('T2,00:30:00,00:30:00', 'T2,00:45:00,00:45:00')})
write_archive('cache-priority-new.zip', {**cache_files, 'feed_info.txt': 'feed_start_date,feed_end_date\n20261018,20261031\n'})
duplicate_code = {**cache_files}
duplicate_code['stop_times.txt'] = duplicate_code['stop_times.txt'].replace('T3,23:50:00,23:50:00,S3,1,0,', 'T3,23:40:00,23:40:00,S2,1,0,Code first\nT3,23:50:00,23:50:00,S3,2,0,Code second')
write_archive('cache-code.zip', duplicate_code)
quoted = {**cache_files}
quoted['trips.txt'] = quoted['trips.txt'].replace('Trip branch', '"Trip, ""east""\nbranch"')
write_archive('cache-quoted.zip', quoted)
many = {**cache_files}
many['calendar_dates.txt'] += 'FUTURE,20261020,1\n'
many['trips.txt'] += ''.join(f'R4,FUTURE,F{i:04},Future,0\n' for i in range(1500))
many['stop_times.txt'] += ''.join(f'F{i:04},12:00:00,12:00:00,S1,{i+1},0,Future\n' for i in range(1500))
write_archive('cache-many.zip', many)
# U64611 is a deterministic four-bit Bloom false positive for the selected
# T1/T2/DAILY/F0000..F1499 set. Its malformed row belongs to unselected R5.
bloom_false_positive = {**many}
bloom_false_positive['trips.txt'] += 'R5,ALWAYS,U64611,Unselected,0\n'
bloom_false_positive['stop_times.txt'] += 'U64611,bad,bad,S1,bad,bad,Unselected\n'
write_archive('cache-bloom.zip', bloom_false_positive)
oversize = {**many, 'trips.txt': many['trips.txt'].replace(',Future,0', ',' + 'x'*1000 + ',0')}
write_archive('cache-oversize.zip', oversize)
write_archive('cache-stamped.zip', {**cache_files, 'feed_info.txt': 'feed_start_date,feed_end_date\n20261018,20261031\n'}, updated=1792200000)
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
