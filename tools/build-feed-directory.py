#!/usr/bin/env python3
"""Normalize the public feeds_v2.csv into an offline installer directory."""
import argparse
import csv
import datetime
import json
from pathlib import Path


def build(source, output, date):
    feeds = []
    with Path(source).open(encoding='utf-8-sig', newline='') as file:
        for row in csv.DictReader(file):
            if row['data_type'] != 'gtfs':
                continue
            feeds.append({
                'id': row['id'], 'country': row['location.country_code'],
                'city': row['location.municipality'], 'region': row['location.subdivision_name'],
                'provider': row['provider'], 'name': row['name'], 'status': row['status'],
                'mirror': row['urls.latest'], 'original': row['urls.direct_download'],
                'authentication': row['urls.authentication_type'], 'license': row['urls.license'],
            })
    data = {'source': 'https://files.mobilitydatabase.org/feeds_v2.csv', 'date': date, 'feeds': feeds}
    Path(output).write_text(json.dumps(data, ensure_ascii=False, separators=(',', ':')) + '\n')
    return len(feeds)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('csv'); parser.add_argument('output'); parser.add_argument('--date', default=str(datetime.date.today()))
    args = parser.parse_args()
    print(build(args.csv, args.output, args.date), 'GTFS timetable feeds')
