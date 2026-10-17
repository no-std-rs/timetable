"""Reproduce tiny synthetic feeds; no network or real timetable data required."""
import io
from pathlib import Path
import zipfile
import zlib

ROOT = Path(__file__).parent / "fixtures"
FILES = {
    "routes.txt": '\ufeffroute_long_name,route_id,route_short_name,route_type,route_desc\r\n"North, South",4,4,0,"Quoted ""legend""\nnext line"\r\nOther,5,5,3,\r\n',
    "shapes.txt": "unused\n" * 10000,
    "stops.txt": "stop_id,stop_name,stop_code,stop_lat,stop_lon,parent_station,location_type\r\nS1,Święty Marcin,SW01,52.4,16.9,P,0\r\nS2,Most Teatralny,MT02,52.41,16.91,P,0\r\nS3,Branch,B03,52.42,16.92,,0\r\nP,Station,,,,,1\r\n",
    # Deliberately before trips.txt, just like the official Poznań archive.
    "stop_times.txt": "trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,drop_off_type\r\nT_A,25:10:00,25:10:00,S1,0,0,1\r\nT_A,25:20:00,25:20:00,S2,2,1,0\r\nT_B,08:00:00,08:00:00,S2,0,0,0\r\nT_B,08:10:00,08:10:00,S1,10,0,0\r\nT_C,09:00:00,09:00:00,S1,0,0,0\r\nT_C,,,S3,1,,\r\nT_D,06:00:00,06:00:00,S1,1,0,0\r\nT_OTHER,07:00:00,07:00:00,S1,1,0,0\r\nT_REM,10:00:00,10:00:00,S1,1,0,0\r\n",
    "trips.txt": "route_id,service_id,trip_id,trip_headsign,direction_id\r\n4,ALL,T_A,North,0\r\n4,ALL,T_B,South,1\r\n4,EXTRA,T_C,Branch,0\r\n4,WEEK,T_D,North,0\r\n5,ALL,T_OTHER,Other,\r\n4,REMOVED,T_REM,North,0\r\n",
    "calendar.txt": "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\r\nALL,1,1,1,1,1,1,1,20261015,20261031\r\nWEEK,1,1,1,1,1,0,0,20261015,20261031\r\nREMOVED,1,1,1,1,1,1,1,20261015,20261031\r\n",
    "calendar_dates.txt": "service_id,date,exception_type\r\nREMOVED,20261017,2\r\nEXTRA,20261017,1\r\n",
}

class ForwardOnly(io.BytesIO):
    def seek(self, *args):
        raise io.UnsupportedOperation("forward only")

def archive(name, files, method=zipfile.ZIP_DEFLATED, descriptors=False, comment=b""):
    out = ForwardOnly() if descriptors else io.BytesIO()
    with zipfile.ZipFile(out, "w") as z:
        z.comment = comment
        for path, content in files.items():
            info = zipfile.ZipInfo(path, date_time=(2026, 10, 17, 0, 0, 0))
            info.compress_type = method
            z.writestr(info, content.encode() if isinstance(content, str) else content)
    (ROOT / name).write_bytes(out.getvalue())

archive("gtfs.zip", FILES)
archive("descriptor.zip", FILES, descriptors=True)
archive("stored-descriptor.zip", FILES, zipfile.ZIP_STORED, descriptors=True)
archive("comment.zip", {"routes.txt": FILES["routes.txt"]}, comment=b"x" * 137 + b"PK\x05\x06not-an-end")
archive("stored.zip", {"routes.txt": FILES["routes.txt"]}, zipfile.ZIP_STORED)
archive("calendar-dates-only.zip", {k:v for k,v in FILES.items() if k != "calendar.txt"})
archive("no-stop-times.zip", {k:v for k,v in FILES.items() if k != "stop_times.txt"})
archive("no-calendar.zip", {k:v for k,v in FILES.items() if k not in ["calendar.txt", "calendar_dates.txt"]})
archive("shuffled.zip", {k:FILES[k] for k in ["trips.txt", "calendar_dates.txt", "stop_times.txt", "routes.txt", "calendar.txt", "stops.txt", "shapes.txt"]})
archive("changed-trips.zip", {k:(v.replace("4,ALL,T_A", "5,ALL,T_A") if k == "trips.txt" else v) for k,v in FILES.items()})

# This four-byte CRC preimage exercises a legal ambiguity in data descriptors.
marker_crc_payload = bytes.fromhex("ac0a7ad5")
assert zlib.crc32(marker_crc_payload) == 0x08074b50
archive("marker-crc-signed.zip", {"rare-crc.bin": marker_crc_payload}, descriptors=True)
signed = (ROOT / "marker-crc-signed.zip").read_bytes()
name_size = int.from_bytes(signed[26:28], "little")
extra_size = int.from_bytes(signed[28:30], "little")
with zipfile.ZipFile(io.BytesIO(signed)) as z:
    compressed_size = z.infolist()[0].compress_size
descriptor = 30 + name_size + extra_size + compressed_size
assert signed[descriptor:descriptor + 4] == b"PK\x07\x08"
unsigned = bytearray(signed[:descriptor] + signed[descriptor + 4:])
directory = int.from_bytes(unsigned[-6:-2], "little") - 4
unsigned[-6:-2] = directory.to_bytes(4, "little")
(ROOT / "marker-crc-unsigned.zip").write_bytes(unsigned)
