#!/usr/bin/env python3
"""Build the same flash/browser catalogue from an explicit official GTFS ZIP.

No network requests. IDs are exact and bounded; only human display labels may
be shortened. Timetable departures are never embedded in the catalogue.
"""
import argparse
from collections import Counter, defaultdict
import csv
import hashlib
import io
import json
from pathlib import Path
import struct
import zipfile
import zlib


def build(source, destination, date):
    source = Path(source)
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(source) as archive:
        def rows(name):
            with archive.open(name) as file:
                yield from csv.DictReader(io.TextIOWrapper(file, encoding='utf-8-sig', newline=''))
        routes = {r['route_id']: r for r in rows('routes.txt')}
        stops = {s['stop_id']: s for s in rows('stops.txt')}
        trips = {}
        heads = defaultdict(Counter)
        links = defaultdict(int)
        for trip in rows('trips.txt'):
            direction = trip.get('direction_id', '')
            if direction not in ('', '0', '1'):
                raise ValueError('invalid direction_id')
            bit = 4 if direction == '' else 1 << int(direction)
            trips[trip['trip_id']] = (trip['route_id'], bit)
            if direction:
                heads[(trip['route_id'], int(direction))][trip.get('trip_headsign', '')] += 1
        for row in rows('stop_times.txt'):
            if row.get('pickup_type') == '1':
                continue
            route, bit = trips[row['trip_id']]
            links[(route, row['stop_id'])] |= bit
    route_ids = sorted(routes, key=lambda key: (routes[key].get('route_short_name', '').zfill(8), key))
    stop_ids = sorted({s for _, s in links}, key=lambda key: (stops[key].get('stop_name', ''), stops[key].get('stop_code', ''), key))
    ri, si = {v: i for i, v in enumerate(route_ids)}, {v: i for i, v in enumerate(stop_ids)}
    if max(len(ri), len(si)) > 65535:
        raise ValueError('catalogue exceeds u16 index capacity')
    def clean(value):
        return ' '.join(value.replace('\0', '').split())
    def field(value, size, label=False):
        value = clean(value) if label else value
        data = value.encode('utf-8')
        if len(data) > size:
            if not label:
                raise ValueError(f'ID or line label exceeds {size} bytes: {value!r}')
            data = data[:size - 3].decode('utf-8', errors='ignore').encode('utf-8') + b'...'
        if b'\0' in data:
            raise ValueError('NUL in field')
        return data.ljust(size, b'\0')
    def label(value, size):
        return field(value, size, True).rstrip(b'\0').decode('utf-8')
    browser_routes, browser_stops = [], []
    payload = bytearray()
    for key in route_ids:
        r = routes[key]
        directions = [label(heads[(key, d)].most_common(1)[0][0], 64) if heads[(key, d)] else '' for d in [0, 1]]
        line = r.get('route_short_name') or key
        payload += field(key, 32) + field(line, 8) + b''.join(field(h, 64, True) for h in directions)
        browser_routes.append({'id': key, 'line': line, 'headsigns': directions})
    for key in stop_ids:
        s = stops[key]
        name, code = label(s.get('stop_name', ''), 64), s.get('stop_code', '')
        payload += field(key, 32) + field(name, 64) + field(code, 16)
        browser_stops.append({'id': key, 'name': name, 'code': code})
    browser_links = [[ri[r], si[s], bits] for (r, s), bits in sorted(links.items(), key=lambda item: (ri[item[0][0]], si[item[0][1]]))]
    for route, stop, bits in browser_links:
        payload += struct.pack('<HHBB', route, stop, bits, 0)
    header = b'TTCAT001' + date.encode('ascii') + struct.pack('<HHIII', len(ri), len(si), len(links), zlib.crc32(payload), 0)
    metadata = {'provider': 'ZTM Poznań', 'date': date, 'source': 'https://www.ztm.poznan.pl/otwarte-dane/gtfsfiles/', 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest()}
    destination.joinpath('catalog.bin').write_bytes(header + payload)
    destination.joinpath('catalog.json').write_text(json.dumps({**metadata, 'routes': browser_routes, 'stops': browser_stops, 'links': browser_links}, ensure_ascii=False, separators=(',', ':')) + '\n')
    destination.joinpath('provenance.json').write_text(json.dumps(metadata, indent=2, ensure_ascii=False) + '\n')
    print(f'{len(ri)} routes, {len(si)} stops, {len(links)} route/stop links; {len(header + payload)} flash bytes')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('zip')
    parser.add_argument('destination')
    parser.add_argument('--date', required=True)
    args = parser.parse_args()
    import datetime
    datetime.datetime.strptime(args.date, '%Y%m%d')
    build(args.zip, args.destination, args.date)
