"""Small offline feeds with reused identities, different places and timezones."""
from pathlib import Path
import json
import zipfile

root = Path(__file__).parent / 'fixtures'
root.mkdir(exist_ok=True)
feeds = []
for id, country, city, zone, provider, stop in [
    ('demo-ny', 'US', 'New York', 'America/New_York', 'Metro New York', 'Broadway'),
    ('demo-kolkata', 'IN', 'Kolkata', 'Asia/Kolkata', 'Kolkata Transit', 'Park Street'),
    ('demo-jp', 'JP', 'Tokyo', 'Asia/Tokyo', 'Tokyo Transit', '東京駅'),
]:
    files = {
        'agency.txt': f'agency_id,agency_name,agency_url,agency_timezone,agency_lang\nA,{provider},https://example.com,{zone},en\n',
        'routes.txt': 'route_id,agency_id,route_short_name,route_type\nR,A,10,3\n',
        'stops.txt': f'stop_id,stop_name,stop_code,stop_lat,stop_lon\nS1,{stop},{"番一" if id == "demo-jp" else "P1"},0,0\nS2,Central,P2,0,0\nS3,Branch,P3,0,0\nS4,Terminal,P4,0,0\n',
        'trips.txt': 'route_id,service_id,trip_id,trip_headsign,direction_id\nR,DAILY,T1,North,0\nR,DAILY,T2,South,1\nR,DAILY,T3,North branch,0\n',
        'calendar.txt': 'service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\nDAILY,1,1,1,1,1,1,1,20260101,20271231\n',
        # Deliberately interleaved trip rows; path reconstruction cannot assume grouping.
        'stop_times.txt': 'trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type\nT1,09:00:00,09:00:00,S1,1,0\nT3,09:15:00,09:15:00,S1,1,0\nT2,18:00:00,18:00:00,S2,1,0\nT3,09:20:00,09:20:00,S3,2,0\nT1,09:10:00,09:10:00,S4,2,1\nT3,09:30:00,09:30:00,S4,3,1\nT2,18:10:00,18:10:00,S1,2,0\n',
    }
    with zipfile.ZipFile(root / f'{id}.zip', 'w') as archive:
        for name, content in files.items():
            info = zipfile.ZipInfo(name, (2026, 10, 21, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = 3
            info.external_attr = 0o600 << 16
            archive.writestr(info, content.encode())
    feeds.append(dict(id=id,country=country,city=city,region='',provider=provider,name='',status='active',mirror=f'https://files.mobilitydatabase.org/{id}/latest.zip'))
(root / 'feeds.json').write_text(json.dumps({'date':'2026-10-21','feeds':feeds},indent=2)+'\n')

# Shared browser/device compatibility and real raw-DEFLATE regressions.
with zipfile.ZipFile(root / 'demo-ny.zip') as source:
    original = {name: source.read(name) for name in source.namelist()}
for filename, replacements, compression in [
    ('long-line.zip', {name: data.replace(b',10,3', b',ABCDEFGHI,3').replace(b'T1', b'x'*254+b'1').replace(b'T3', b'x'*254+b'3') for name,data in original.items()}, zipfile.ZIP_STORED),
    ('bad-service.zip', {name: data.replace(b'DAILY', b'12345678-1234-1234-1234-123456789abc') for name, data in original.items()}, zipfile.ZIP_STORED),
    ('deflated.zip', {}, zipfile.ZIP_DEFLATED),
    ('frequencies-empty-stored.zip', {'frequencies.txt': b'trip_id,start_time,end_time,headway_secs\r\n'}, zipfile.ZIP_STORED),
    ('frequencies-zero.zip', {'frequencies.txt': b''}, zipfile.ZIP_STORED),
    ('frequencies-empty-deflated.zip', {'frequencies.txt': b'\xef\xbb\xbftrip_id,start_time,end_time,headway_secs\r\n\r\n'}, zipfile.ZIP_DEFLATED),
    ('frequencies-populated.zip', {'frequencies.txt': b'trip_id,start_time,end_time,headway_secs\nT1,08:00:00,10:00:00,600\n'}, zipfile.ZIP_DEFLATED),
]:
    with zipfile.ZipFile(root / filename, 'w') as archive:
        for name, data in {**original, **replacements}.items():
            info = zipfile.ZipInfo(name, (2026, 10, 21, 0, 0, 0))
            info.compress_type = compression
            info.create_system = 3
            info.external_attr = 0o600 << 16
            archive.writestr(info, replacements.get(name, data))
