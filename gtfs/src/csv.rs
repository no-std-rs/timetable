//! Bounded streaming CSV. Quoted commas, escaped quotes, embedded newlines,
//! CRLF, arbitrary chunk boundaries and UTF-8 BOMs are supported.
//! CSV syntax follows `csv-core`'s permissive parser; typed GTFS values and
//! required columns are validated separately.

use crate::{Error, Table};
use csv_core::ReadRecordResult;
use embedded_io_async::{Error as _, Read};

macro_rules! columns {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(usize)]
        pub enum Column { $($variant),+ }
        impl Column {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn name(self) -> &'static str { match self { $(Self::$variant => $name),+ } }
            fn from_name(name: &str) -> Option<Self> { match name { $($name => Some(Self::$variant)),+, _=>None } }
        }
    }
}

columns! {
    RouteId=>"route_id", AgencyId=>"agency_id", RouteShortName=>"route_short_name", RouteLongName=>"route_long_name",
    RouteDesc=>"route_desc", RouteType=>"route_type", StopId=>"stop_id", StopCode=>"stop_code", StopName=>"stop_name",
    StopLat=>"stop_lat", StopLon=>"stop_lon", ParentStation=>"parent_station", LocationType=>"location_type",
    TripId=>"trip_id", ServiceId=>"service_id", TripHeadsign=>"trip_headsign", DirectionId=>"direction_id",
    ArrivalTime=>"arrival_time", DepartureTime=>"departure_time", StopSequence=>"stop_sequence", StopHeadsign=>"stop_headsign",
    PickupType=>"pickup_type", DropOffType=>"drop_off_type", Monday=>"monday", Tuesday=>"tuesday", Wednesday=>"wednesday",
    Thursday=>"thursday", Friday=>"friday", Saturday=>"saturday", Sunday=>"sunday", StartDate=>"start_date", EndDate=>"end_date",
    Date=>"date", ExceptionType=>"exception_type", FeedStartDate=>"feed_start_date", FeedEndDate=>"feed_end_date", FeedVersion=>"feed_version",
}

/// Borrows the current row until the next call to `Reader::next`.
#[derive(Clone, Copy)]
pub struct Record<'a> {
    pub(crate) table: Table,
    data: &'a str,
    ends: &'a [usize],
    columns: &'a [usize; Column::ALL.len()],
}

impl<'a> Record<'a> {
    pub fn table(self) -> Table {
        self.table
    }
    /// Missing columns and empty optional fields both return `None`.
    pub fn get(self, column: Column) -> Option<&'a str> {
        let i = self.columns[column as usize];
        if i == usize::MAX {
            return None;
        }
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        let data = &self.data[start..self.ends[i]];
        if data.is_empty() { None } else { Some(data) }
    }

    pub fn required(self, column: Column) -> Result<&'a str, Error> {
        self.get(column).ok_or(Error::InvalidField(column.name()))
    }
}

/// Defaults fit the observed Poznań feed, including long `route_desc` fields.
/// Increase the const bounds for feeds with larger records or more columns.
pub struct Reader<R, const ROW: usize = 8192, const FIELDS: usize = 48> {
    source: R,
    parser: csv_core::Reader,
    input: [u8; 1024],
    start: usize,
    end: usize,
    eof: bool,
    primed: bool,
    data: [u8; ROW],
    ends: [usize; FIELDS],
    data_len: usize,
    fields: usize,
    columns: [usize; Column::ALL.len()],
    header_fields: Option<usize>,
    table: Table,
}

impl<R: Read, const ROW: usize, const FIELDS: usize> Reader<R, ROW, FIELDS> {
    pub fn new(source: R, table: Table) -> Self {
        Self {
            source,
            parser: csv_core::Reader::new(),
            input: [0; 1024],
            start: 0,
            end: 0,
            eof: false,
            primed: false,
            data: [0; ROW],
            ends: [0; FIELDS],
            data_len: 0,
            fields: 0,
            columns: [usize::MAX; Column::ALL.len()],
            header_fields: None,
            table,
        }
    }

    async fn raw_next(&mut self) -> Result<bool, Error> {
        self.data_len = 0;
        self.fields = 0;
        loop {
            if self.start == self.end && !self.eof {
                self.start = 0;
                self.end = 0;
                // csv-core needs the entire BOM together, even if the transport
                // supplies only one byte per read.
                loop {
                    let n = self
                        .source
                        .read(&mut self.input[self.end..])
                        .await
                        .map_err(|e| Error::Io(e.kind()))?;
                    self.end += n;
                    self.eof = n == 0;
                    if self.primed || self.end >= 4 || self.eof {
                        break;
                    }
                }
                self.primed = true;
            }
            // A row exactly at capacity can still have closing quotes or a
            // newline to consume. One scratch byte distinguishes those bytes
            // from actual unescaped output that would exceed the bound.
            let at_capacity = self.data_len == ROW;
            let mut overflow = [0; 1];
            let output = if at_capacity {
                &mut overflow[..]
            } else {
                &mut self.data[self.data_len..]
            };
            let (status, consumed, produced, fields) = self.parser.read_record(
                &self.input[self.start..self.end],
                output,
                &mut self.ends[self.fields..],
            );
            if at_capacity && produced != 0 {
                return Err(Error::RecordTooLong);
            }
            self.start += consumed;
            self.data_len += produced;
            self.fields += fields;
            match status {
                ReadRecordResult::InputEmpty => {}
                ReadRecordResult::OutputFull => {}
                ReadRecordResult::OutputEndsFull => return Err(Error::TooManyFields),
                ReadRecordResult::End => return Ok(false),
                ReadRecordResult::Record => {
                    return Ok(true);
                }
            }
        }
    }

    pub async fn next(&mut self) -> Result<Option<Record<'_>>, Error> {
        if self.header_fields.is_none() {
            if !self.raw_next().await? {
                return Err(Error::MissingColumn(
                    self.table.required().first().map_or("header", |c| c.name()),
                ));
            }
            let mut start = 0;
            for (i, &end) in self.ends[..self.fields].iter().enumerate() {
                let name =
                    core::str::from_utf8(&self.data[start..end]).map_err(|_| Error::InvalidUtf8)?;
                if let Some(c) = Column::from_name(name) {
                    if self.columns[c as usize] != usize::MAX {
                        return Err(Error::InvalidField(c.name()));
                    }
                    self.columns[c as usize] = i;
                }
                start = end;
            }
            for &c in self.table.required() {
                if self.columns[c as usize] == usize::MAX {
                    return Err(Error::MissingColumn(c.name()));
                }
            }
            self.header_fields = Some(self.fields);
        }
        if !self.raw_next().await? {
            return Ok(None);
        }
        let data =
            core::str::from_utf8(&self.data[..self.data_len]).map_err(|_| Error::InvalidUtf8)?;
        if self.ends[..self.fields]
            .iter()
            .any(|&end| !data.is_char_boundary(end))
        {
            return Err(Error::InvalidUtf8);
        }
        if self.header_fields != Some(self.fields) {
            return Err(Error::InvalidField("CSV record width"));
        }
        Ok(Some(Record {
            table: self.table,
            data,
            ends: &self.ends[..self.fields],
            columns: &self.columns,
        }))
    }

    pub fn into_inner(self) -> R {
        self.source
    }
}
