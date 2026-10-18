Synthetic GTFS and generated catalogue, reproduced by `../make_fixtures.py`.
The generator's provider metadata is a format fixture; these invented routes,
names and departures are not official Poznań data. The archive tests ordinary
next-day departures, stop-specific headsign rollback, duplicate public line
names, clipped/normalized stop names and a drop-off-only terminus.
`spring-dst.zip` shifts the service dates to the spring clock change, when the
next GTFS service day starts before civil midnight.
