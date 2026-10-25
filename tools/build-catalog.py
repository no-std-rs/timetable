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
import math
from pathlib import Path
import re
import struct
import unicodedata
import zipfile
import zlib


def alphabetical_key(value):
    """Deterministic Polish collation, independent of the build host's locale."""
    accents = {'ą': ('a', 1), 'ć': ('c', 1), 'ę': ('e', 1), 'ł': ('l', 1),
               'ń': ('n', 1), 'ó': ('o', 1), 'ś': ('s', 1), 'ź': ('z', 1), 'ż': ('z', 2)}
    def weight(char):
        base, extra = accents.get(char, (char, 0))
        return ord(base) * 4 + extra
    normalized = unicodedata.normalize('NFC', ' '.join(value.split()).casefold())
    return tuple((0, int(part)) if part.isascii() and part.isdigit()
                 else (1, tuple(map(weight, part)))
                 for part in re.split(r'([0-9]+)', normalized) if part)


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
        trip_calls = defaultdict(list)
        heads = defaultdict(Counter)
        links = defaultdict(int)
        for trip in rows('trips.txt'):
            direction = trip.get('direction_id', '')
            if direction not in ('', '0', '1'):
                raise ValueError('invalid direction_id')
            bit = 4 if direction == '' else 1 << int(direction)
            trips[trip['trip_id']] = (trip['route_id'], bit, trip.get('trip_headsign', ''))
            if direction:
                heads[(trip['route_id'], int(direction))][trip.get('trip_headsign', '')] += 1
        for row in rows('stop_times.txt'):
            pickup = int(row.get('pickup_type') or '0')
            sequence = int(row['stop_sequence'])
            if pickup not in range(4) or sequence < 0 or row['stop_id'] not in stops:
                raise ValueError('invalid stop time')
            trip_calls[row['trip_id']].append((sequence, row['stop_id'], pickup, row.get('stop_headsign', '')))
            if pickup == 1:
                continue
            route, bit, _ = trips[row['trip_id']]
            links[(route, row['stop_id'])] |= bit
    # Physical order is also device browsing order: sort offline, never in RAM.
    route_ids = sorted(routes, key=lambda key: (alphabetical_key(routes[key].get('route_short_name') or key), key))
    stop_ids = sorted({s for _, s in links}, key=lambda key: (alphabetical_key(stops[key].get('stop_name', '')), alphabetical_key(stops[key].get('stop_code', '')), key))
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
    def browser_stop(s, key, name, code):
        browser_stop = {'id': key, 'name': name, 'code': code,
                        'place_name': s.get('stop_name') or key,
                        'location_type': int(s.get('location_type') or '0')}
        try:
            lat, lon = float(s.get('stop_lat', '')), float(s.get('stop_lon', ''))
            if math.isfinite(lat) and math.isfinite(lon) and abs(lat) <= 90 and abs(lon) <= 180:
                browser_stop.update(lat=lat, lon=lon)
        except ValueError:
            pass
        return browser_stop
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
        browser_stops.append(browser_stop(s, key, name, code))
    # Browser-only route geometry: keep the flash catalogue byte-for-byte stable.
    # Non-boarding terminals still belong in a path, but never become choices.
    via_ids = sorted({call[1] for calls in trip_calls.values() for call in calls} - si.keys())
    via_stops = [browser_stop(stops[key], key, label(stops[key].get('stop_name', ''), 64), stops[key].get('stop_code', '')) for key in via_ids]
    graph_indices = {**si, **{key: len(si) + i for i, key in enumerate(via_ids)}}
    patterns = Counter()
    for trip, calls in trip_calls.items():
        route, bit, headsign = trips[trip]
        calls.sort()
        if len({call[0] for call in calls}) != len(calls):
            raise ValueError('duplicate trip stop_sequence')
        ordered = tuple((graph_indices[stop], pickup, label(override, 64)) for _, stop, pickup, override in calls)
        patterns[(ri[route], -1 if bit == 4 else bit - 1, label(headsign, 64), ordered)] += 1
    browser_patterns = [
        {'route': route, 'direction': direction, 'headsign': headsign,
         'calls': [[stop, pickup, override] if override else [stop, pickup] for stop, pickup, override in calls], 'trips': count}
        for (route, direction, headsign, calls), count in sorted(patterns.items())
    ]
    browser_links = [[ri[r], si[s], bits] for (r, s), bits in sorted(links.items(), key=lambda item: (ri[item[0][0]], si[item[0][1]]))]
    for route, stop, bits in browser_links:
        payload += struct.pack('<HHBB', route, stop, bits, 0)
    header = b'TTCAT001' + date.encode('ascii') + struct.pack('<HHIII', len(ri), len(si), len(links), zlib.crc32(payload), 0)
    metadata = {'provider': 'ZTM Poznań', 'date': date, 'source': 'https://www.ztm.poznan.pl/otwarte-dane/gtfsfiles/', 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest()}
    destination.joinpath('catalog.bin').write_bytes(header + payload)
    destination.joinpath('catalog.json').write_text(json.dumps({**metadata, 'routes': browser_routes, 'stops': browser_stops, 'links': browser_links, 'via_stops': via_stops, 'patterns': browser_patterns}, ensure_ascii=False, separators=(',', ':')) + '\n')
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
