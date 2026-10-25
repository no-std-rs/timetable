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

# Journey fixtures are synthetic: names mirror the requested use case, times
# deliberately distinguish access walks, unsafe connections and search depth.
journey = {
    'agency.txt': 'agency_name,agency_url,agency_timezone\nSynthetic transit,https://example.invalid,Europe/Warsaw\n',
    'routes.txt': 'route_id,route_short_name,route_type\nA,911,3\nD,Direct,3\nT,Transfer,3\nM,Middle,3\nF,Final,3\n',
    'stops.txt': 'stop_id,stop_name,stop_code\nA,Maków Polnych,A01\nB,Further stop,B01\nX,Interchange,X01\nY,Second interchange,Y01\nZ,Bolechowo-Os./Szkoła,Z01\nW,Other target,W01\n',
    'calendar.txt': 'service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\nALL,1,1,1,1,1,1,1,20261001,20261031\n',
    'calendar_dates.txt': 'service_id,date,exception_type\n',
    'trips.txt': 'route_id,service_id,trip_id\nA,ALL,a\nD,ALL,d\nT,ALL,tight\nT,ALL,legal\nM,ALL,m\nF,ALL,f\nA,ALL,a2\nT,ALL,t2\nD,ALL,d2\n',
}
journey_rows = [
    ('a','08:07:00','A',10),('a','08:17:00','X',20),
    ('d','08:15:00','B',1),('d','08:27:00','W',2),('d','08:35:00','Z',3),
    ('tight','08:19:00','X',1),('tight','08:25:00','Z',2),
    ('legal','08:20:00','X',1),('legal','08:30:00','Z',2),
    ('m','08:20:00','X',1),('m','08:25:00','Y',2),
    ('f','08:28:00','Y',1),('f','08:29:00','Z',2),
    ('a2','08:37:00','A',1),('a2','08:47:00','X',2),
    ('t2','08:50:00','X',1),('t2','09:00:00','Z',2),
    ('d2','08:45:00','B',1),('d2','09:05:00','Z',2),
]
def journey_calls(rows):
    return 'trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,drop_off_type\n'+''.join(f'{trip},{at},{at},{stop},{seq},0,0\n' for trip,at,stop,seq in rows)
journey['stop_times.txt'] = journey_calls(journey_rows)
write_archive('journey.zip',journey)
alternatives = {**journey,
    'routes.txt': journey['routes.txt'] + 'Q,Alt,3\n',
    'stops.txt': journey['stops.txt'] + 'P,Via park,P01\nQ,Via square,Q01\nV,Through stop,V01\n',
    'trips.txt': 'route_id,service_id,trip_id\nA,ALL,a\nA,ALL,a2\nQ,ALL,q\nT,ALL,last\n',
    'stop_times.txt': journey_calls([
        ('a','08:07:00','A',1),('a','08:10:00','P',2),('a','08:17:00','X',3),
        ('a2','08:08:00','A',1),('a2','08:11:00','Q',2),('a2','08:18:00','X',3),
        ('q','08:09:00','A',1),('q','08:19:00','X',2),
        ('last','08:22:00','X',1),('last','08:25:00','V',2),('last','08:30:00','Z',3),
    ]),
}
# The middle call cannot be used for boarding/alighting but must be displayed.
alternatives['stop_times.txt'] = alternatives['stop_times.txt'].replace('V,2,0,0', 'V,2,1,1')
write_archive('journey-alternatives.zip', alternatives)
untimed_alternatives = {**alternatives,
    'stops.txt': alternatives['stops.txt'].replace('Via park,P01', 'Via park,番二三四五六七八九十'),
    'stop_times.txt': alternatives['stop_times.txt'].replace('a,08:10:00,08:10:00,P', 'a,,,P')
        .replace('a2,08:11:00,08:11:00,Q', 'a2,,,Q')
        .replace('last,08:25:00,08:25:00,V', 'last,,08:25:00,V'),
}
write_archive('journey-untimed-alternatives.zip', untimed_alternatives)
write_archive('journey-shuffled.zip',{**journey,'stop_times.txt':journey_calls(journey_rows[::-1])})
write_archive('journey-direct.zip',{**journey,'stop_times.txt':journey['stop_times.txt'].replace('d,08:35:00,08:35:00','d,08:28:00,08:28:00')})
write_archive('journey-no-service.zip',{**journey,'calendar_dates.txt':'service_id,date,exception_type\nALL,20261017,2\n'})
write_archive('journey-forbidden.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type\nX,X,3\n'})
write_archive('journey-minimum.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type,min_transfer_time\nX,X,2,240\n'})

station = {**journey}
station['stops.txt'] = 'stop_id,stop_name,stop_code,parent_station,location_type\nA,Maków Polnych,A01,,0\nB,Further stop,B01,,0\nX,Interchange platform 1,X01,P,0\nXP,Interchange platform 2,X02,P,0\nY,Second interchange,Y01,,0\nZ,Bolechowo-Os./Szkoła,Z01,,0\nW,Other target,W01,,0\nP,Interchange,,,1\n'
station['stop_times.txt'] = journey_calls([(trip,at,'XP' if stop=='X' and trip in ('tight','legal','t2') else stop,seq) for trip,at,stop,seq in journey_rows])
write_archive('journey-station.zip',station)
write_archive('journey-station-forbidden.zip',{**station,'transfers.txt':'from_stop_id,to_stop_id,transfer_type\nP,P,3\n'})

night = journey['stop_times.txt']
for hour in (8,9):
    night = night.replace(f',{hour:02}:',f',{hour+16:02}:')
write_archive('journey-night.zip',{**journey,'stop_times.txt':night,'calendar_dates.txt':'service_id,date,exception_type\nALL,20261018,2\n'})
write_archive('journey-no-pickup.zip',{**journey,'stop_times.txt':journey['stop_times.txt'].replace('A,10,0,0','A,10,1,0')})
write_archive('journey-no-dropoff.zip',{**journey,'stop_times.txt':journey['stop_times.txt'].replace('legal,08:30:00,08:30:00,Z,2,0,0','legal,08:30:00,08:30:00,Z,2,0,1')})

write_archive('journey-handover.zip',{**journey,'feed_info.txt':'feed_start_date,feed_end_date\n20261017,20261031\n'})

write_archive('journey-boundaries.zip',{**journey,
    'trips.txt':'route_id,service_id,trip_id\nA,ALL,boundary\n',
    'stop_times.txt':'trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,drop_off_type\nboundary,07:59:00,08:07:00,A,1,0,0\nboundary,13:59:00,14:01:00,Z,2,0,0\n'})
write_archive('journey-conditional.zip',{**journey,'stop_times.txt':journey['stop_times.txt'].replace('A,10,0,0','A,10,2,0')})
write_archive('journey-remote-rule.zip',{**journey,
    'stops.txt':journey['stops.txt']+'U,Remote station,U01\nV,Other remote station,V01\n',
    'transfers.txt':'from_stop_id,to_stop_id,transfer_type,from_route_id\nU,V,0,A\nV,U,4,\n'})
write_archive('journey-scoped-rule.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type,from_route_id\nX,X,0,A\n'})
write_archive('journey-inseat-rule.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type\nX,X,4\n'})

write_archive('journey-conditional-alight.zip',{**journey,'stop_times.txt':journey['stop_times.txt'].replace('legal,08:30:00,08:30:00,Z,2,0,0','legal,08:30:00,08:30:00,Z,2,0,3')})

write_archive('journey-conditional-through.zip',{**journey,'stop_times.txt':journey['stop_times.txt'].replace('W,2,0,0','W,2,3,2')})

write_archive('journey-timed.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type\nX,X,1\n'})

write_archive('journey-scope-unrelated.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type,from_route_id,to_route_id\nX,X,3,D,T\n'})
write_archive('journey-scope-precedence.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type,from_route_id,to_route_id\nX,X,3,,\nX,X,0,A,T\n'})
write_archive('journey-scope-trip.zip',{**journey,'transfers.txt':'from_stop_id,to_stop_id,transfer_type,from_route_id,to_route_id,from_trip_id,to_trip_id\nX,X,3,A,T,,\nX,X,0,,,a,legal\n'})
write_archive('journey-scope-alternative.zip',{**journey,
    'routes.txt':journey['routes.txt']+'Q,Alt,3\n',
    'trips.txt':journey['trips.txt']+'Q,ALL,q\n',
    'stop_times.txt':journey['stop_times.txt'].replace('legal,08:20:00,08:20:00','legal,08:21:00,08:21:00')+'q,08:08:00,08:08:00,A,1,0,0\nq,08:18:00,08:18:00,X,2,0,0\n',
    'transfers.txt':'from_stop_id,to_stop_id,transfer_type,from_route_id,to_route_id\nX,X,3,A,T\n'})

for name,route in [('matching','A'),('unrelated','D')]:
    write_archive(f'journey-scope-link-{name}.zip',{**station,
        'stops.txt':station['stops.txt'].replace('XP,Interchange platform 2,X02,P,0','XP,Interchange platform 2,X02,,0'),
        'transfers.txt':f'from_stop_id,to_stop_id,transfer_type,from_route_id,to_route_id\nX,XP,0,{route},T\n'})

# One place has several platform IDs, unordered rows and spelling whitespace/case
# differences. Only the other platforms carry the fastest direct service.
places = {**journey,
    'stops.txt': journey['stops.txt'] + 'A2,  MAKÓW   POLNYCH  ,A02\nZ2,bolechowo-os./szkoła,Z02\nAP,Maków Polnych,Station\nU,Makow Polnych,U01\n',
    'routes.txt': journey['routes.txt'] + 'P,Platform,3\n',
    'trips.txt': journey['trips.txt'] + 'P,ALL,p\n',
    'stop_times.txt': journey['stop_times.txt'] + journey_calls([('p','08:09:00','A2',1),('p','08:22:00','Z2',2)]).split('\n',1)[1],
}
# A station row must not become a boarding endpoint; lack of accents is a
# different name, even though accent-insensitive search can find both.
places['stops.txt'] = 'stop_id,stop_name,stop_code,location_type\n' + ''.join(row + (',1\n' if row.startswith('AP,') else ',0\n') for row in places['stops.txt'].splitlines()[1:])
coords = {'A': (52.4, 16.9), 'A2': (52.4, 16.9001), 'B': (52.4, 16.92), 'Z': (52.5, 16.95), 'Z2': (52.5, 16.9501), 'W': (52.4, 16.91), 'X': (52.41, 16.91), 'Y': (52.4, 16.905)}
places['stops.txt'] = places['stops.txt'].splitlines()[0]+',stop_lat,stop_lon\n'+''.join(row+','+','.join(map(str,coords.get(row.split(',')[0], ('',''))))+'\n' for row in places['stops.txt'].splitlines()[1:])
write_archive('journey-places.zip',places)
write_archive('journey-places-shuffled.zip',{**places,'stops.txt': places['stops.txt'].split('\n',1)[0]+'\n'+'\n'.join(places['stops.txt'].splitlines()[1:][::-1])+'\n'})

# Interchanges are also single named places when the feed has no parent_station.
same_place = {**station, 'stops.txt': station['stops.txt'].replace('Interchange platform 1,X01,P,0', 'Interchange,X01,,0').replace('Interchange platform 2,X02,P,0','  INTERCHANGE  ,X02,,0')}
def positioned(tables):
    result={**tables}
    points={'A':(52.4,16.9),'B':(52.4,16.92),'X':(52.41,16.91),'XP':(52.41,16.9101),'Y':(52.42,16.92),'YP':(52.42,16.9201),'Z':(52.5,16.95),'W':(52.4,16.91)}
    rows=result['stops.txt'].splitlines()
    result['stops.txt']=rows[0]+',stop_lat,stop_lon\n'+''.join(row+','+','.join(map(str,points.get(row.split(',')[0],('',''))))+'\n' for row in rows[1:])
    return result
same_place=positioned(same_place)
write_archive('journey-place-transfer.zip',same_place)
write_archive('journey-place-transfer-forbidden.zip',{**same_place,'transfers.txt':'from_stop_id,to_stop_id,transfer_type\nX,XP,3\n'})
write_archive('journey-place-transfer-minimum.zip',{**same_place,'transfers.txt':'from_stop_id,to_stop_id,transfer_type,min_transfer_time\nX,XP,2,240\n'})

middle_places=positioned({**journey,
    'stops.txt': journey['stops.txt']+'XP,INTERCHANGE,X02\nYP,Second interchange,Y02\n',
    'stop_times.txt': journey_calls([('a','08:07:00','A',1),('a','08:17:00','X',2),('d','08:15:00','B',1),('d','08:35:00','Z',2),('m','08:20:00','XP',1),('m','08:25:00','YP',2),('f','08:28:00','Y',1),('f','08:29:00','Z',2)])})
write_archive('journey-place-middle.zip',middle_places)
write_archive('journey-place-middle-blocked.zip',{**middle_places,'transfers.txt':'from_stop_id,to_stop_id,transfer_type\nX,XP,3\n'})
write_archive('journey-place-middle-inseat-stops.zip',{**middle_places,'transfers.txt':'from_stop_id,to_stop_id,transfer_type\nXP,YP,3\n'})

# Homonymous platforms must not teleport to another location or declared station.
far_places={**places,'stops.txt':places['stops.txt'].replace('A2,  MAKÓW   POLNYCH  ,A02,0,52.4,16.9001','A2,  MAKÓW   POLNYCH  ,A02,0,53.4,16.9001')}
write_archive('journey-places-far.zip',far_places)
write_archive('journey-places-unlocated.zip',{**places,'stops.txt':places['stops.txt'].replace('A2,  MAKÓW   POLNYCH  ,A02,0,52.4,16.9001','A2,  MAKÓW   POLNYCH  ,A02,0,,')})
for separate in (False,True):
    rows=places['stops.txt'].splitlines()
    parents={'A':'PA','A2':'PB' if separate else 'PA'}
    text=rows[0]+',parent_station\n'+''.join(row+','+parents.get(row.split(',')[0],'')+'\n' for row in rows[1:])
    text+='PA,Station A,,1,52.4,16.9,\nPB,Station B,,1,52.4,16.9001,\n'
    if not separate:text=text.replace('A2,  MAKÓW   POLNYCH  ,A02','A2,Another platform name,A02')
    write_archive('journey-places-'+('parents' if separate else 'shared-parent')+'.zip',{**places,'stops.txt':text})
write_archive('journey-place-transfer-far.zip',{**same_place,'stops.txt':same_place['stops.txt'].replace('XP,  INTERCHANGE  ,X02,,0,52.41,16.9101','XP,  INTERCHANGE  ,X02,,0,53.41,16.9101')})

# Overlapping halos have one byte-ID-ordered partition, not one group per query.
overlap={**places,'stops.txt':places['stops.txt'].replace('A2,  MAKÓW   POLNYCH  ,A02,0,52.4,16.9001','A2,  MAKÓW   POLNYCH  ,A02,0,52.4009,16.9')+'A3,Maków Polnych,A03,0,52.4018,16.9\n'}
write_archive('journey-places-overlap.zip',overlap)
rows=overlap['stops.txt'].splitlines()
write_archive('journey-places-overlap-shuffled.zip',{**overlap,'stops.txt':rows[0]+'\n'+'\n'.join(rows[1:][::-1])+'\n'})
middle_overlap={**middle_places,'stops.txt':middle_places['stops.txt'].replace('XP,INTERCHANGE,X02,52.41,16.9101','XP,INTERCHANGE,X02,52.4109,16.91')+'XQ,Interchange,X03,52.4118,16.91\n','stop_times.txt':middle_places['stop_times.txt'].replace('a,08:17:00,08:17:00,X,2','a,08:17:00,08:17:00,XQ,2')}
write_archive('journey-place-middle-overlap.zip',middle_overlap)
# Parent label differs from every platform name; only the unselected peer's
# platform code needs Japanese glyphs in the installed profile.
glyphs={**places}
rows=places['stops.txt'].splitlines()
glyphs['stops.txt']=rows[0]+',parent_station\n'+''.join(row.replace(',A02,',',番二三四五六七八九十,').replace('X,Interchange,X01','X,'+'A'*61+'界語,X01')+(','+'PA' if row.split(',')[0] in ('A','A2') else ',')+'\n' for row in rows[1:])+'PA,Central station,,1,52.4,16.9,\n'
# A remote line exceeds the ROM trip-ID bound, forcing a projected catalogue.
# The usable peer-platform line is absent from that projection, so only the
# journey-wide glyph profile can supply its non-ASCII platform code.
long_trip='q'*256
glyphs['routes.txt']+='Q,Remote,3\n'
glyphs['trips.txt']+=f'Q,ALL,{long_trip}\n'
# Each raw CSV row stays below the parser's 8192-byte record limit. Hidden
# suffixes together exceed the 4096-glyph budget but never render on device.
hidden_name=''.join(chr(0x4e00+i) for i in range(2200))
hidden_code=''.join(chr(0x8000+i) for i in range(2200))
glyphs['stops.txt']=glyphs['stops.txt'].replace(',U01,', ','+'C'*16+hidden_code+',')
glyphs['stops.txt']+='V,'+'R'*64+hidden_name+',V01,0,53.4,17.0,\n'
glyphs['stop_times.txt']+=journey_calls([(long_trip,'08:05:00','U',1),(long_trip,'08:10:00','V',2)]).split('\n',1)[1]
write_archive('journey-places-glyphs.zip',glyphs)

# A small city reproducing the real Poznań failure on the old device bounds:
# weekday trips that do not run today still call at the chosen places, routes
# visit ~80 two-platform places, and every Maków Polnych call is a request
# stop. Walking 12 minutes to a direct 348 or riding the 911 one stop to change
# at Kresowa must both survive. Times are test data, not transit advice.
def city_fixture():
    places = {}
    def place(key, name):
        if key not in places:
            places[key] = (name, len(places))
        return key
    def seq(prefix, name, count):
        return [place(f'{prefix}{i}', f'{name} {i}') for i in range(1, count + 1)]
    A = place('A', 'Maków Polnych'); B = place('B', 'UAM Wydział Geografii')
    K = place('K', 'Kresowa'); H = place('H', 'Biedrusko/Park')
    Z = place('Z', 'Bolechowo-Os./Szkoła'); W = place('W', 'Bolechowo-Os./Wojska Polskiego')
    corridor = seq('R', 'Corridor', 8)
    lines = {
        # route: (label, stops in direction 0, first departure, headway minutes)
        'L911': ('911', seq('C', 'City', 6) + [A, K] + corridor + [H], (5, 57)),
        'L348': ('348', seq('S', 'Estate', 4) + [B, K] + corridor + [W] + seq('X', 'Beyond', 3), (6, 5)),
        'L397': ('397', [H] + seq('P', 'Promnice', 3) + [Z] + seq('Q', 'Szlachęcin', 3), (6, 10)),
        'N1': ('101', seq('M', 'Ring', 10) + [A] + seq('MM', 'Ring outer', 8), (6, 1)),
        'N2': ('202', seq('L', 'Loop', 10) + [Z] + seq('LL', 'Loop outer', 8), (6, 3)),
    }
    stops = ['stop_id,stop_name,stop_code,stop_lat,stop_lon']
    for key, (name, index) in places.items():
        for platform in (1, 2):
            stops.append(f'{key}-{platform},{name},{key}{platform:02},{52.40 + index * 0.004:.6f},{16.90 + platform * 0.0002:.6f}')
    routes = ['route_id,route_short_name,route_type'] + [f'{r},{label},3' for r, (label, _, _) in lines.items()]
    trips = ['route_id,service_id,trip_id,direction_id']
    calls = ['trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,drop_off_type']
    for service in ('SAT', 'WD'):
        for route, (label, path, (hour, minute)) in lines.items():
            for direction in (0, 1):
                ordered = path if direction == 0 else path[::-1]
                # Departures from about 06:00 cover the six hours after 08:00.
                for k in range(22):
                    trip = f'{service}-{route}-{direction}-{k}'
                    trips.append(f'{route},{service},{trip},{direction}')
                    start = hour * 60 + minute + 20 * k
                    for i, key in enumerate(ordered):
                        at = start + 2 * i
                        time = f'{at // 60:02}:{at % 60:02}:00'
                        request = 3 if key == A else 0
                        calls.append(f'{trip},{time},{time},{key}-{direction + 1},{i + 1},{request},{request}')
    return {
        'agency.txt': journey['agency.txt'],
        'routes.txt': '\n'.join(routes) + '\n',
        'stops.txt': '\n'.join(stops) + '\n',
        'calendar.txt': 'service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\nSAT,0,0,0,0,0,1,0,20261001,20261031\nWD,1,1,1,1,1,0,0,20261001,20261031\n',
        'trips.txt': '\n'.join(trips) + '\n',
        'stop_times.txt': '\n'.join(calls) + '\n',
    }
write_archive('journey-city.zip', city_fixture())
