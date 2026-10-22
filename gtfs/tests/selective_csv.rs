use embedded_io_async::{ErrorType, Read};
use futures::executor::block_on;
use gtfs::{
    Error, Table,
    csv::{Column, Reader},
    zip,
};

struct Input<'a> {
    bytes: &'a [u8],
    chunk: usize,
}
impl ErrorType for Input<'_> {
    type Error = core::convert::Infallible;
}
impl Read for Input<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = out.len().min(self.bytes.len()).min(self.chunk);
        out[..n].copy_from_slice(&self.bytes[..n]);
        self.bytes = &self.bytes[n..];
        Ok(n)
    }
}

#[test]
fn selected_rows_match_strict_parser_across_chunks_quotes_and_terminators() {
    let bytes = concat!(
        "\u{feff}trip_id,arrival_time,departure_time,stop_id,stop_sequence,stop_headsign\r\n",
        "\"skip\",05:00:00,05:00:00,other,1,\"no\"\n",
        "\"a\",25:00:00,25:00:00,keep,2,\"POZNAŃ\"\r\n",
        "\"skip2\",05:00:00,05:00:00,other,3,\"quoted\nignored\"\r",
        "\"b,quote\",25:30:00,25:30:00,\"keep\",4,\"a\"\"b\r\nc\"\n",
        "\"skip3\",05:00:00,05:00:00,other,5,\"n\"\"o\"\r\n\n",
        "\"skip4\",05:00:00,05:00:00,other,6,odd\"quote\n",
        "\"c\",26:00:00,26:00:00,keep,6,\"last\""
    )
    .as_bytes();
    for chunk in (1..=100).chain([512, 1024]) {
        block_on(async {
            let mut strict = Reader::<_>::new(Input { bytes, chunk }, Table::StopTimes);
            let mut expected = Vec::new();
            while let Some(row) = strict.next().await.unwrap() {
                if row.get(Column::StopId) == Some("keep") {
                    expected.push((
                        row.required(Column::TripId).unwrap().to_owned(),
                        row.get(Column::StopHeadsign).unwrap().to_owned(),
                    ));
                }
            }
            let mut selective = Reader::<_>::new(Input { bytes, chunk }, Table::StopTimes);
            let mut actual = Vec::new();
            let mut calls = 0;
            while let Some(row) = selective
                .next_matching(Column::StopId, &mut |id: &[u8]| {
                    calls += 1;
                    id == b"keep"
                })
                .await
                .unwrap()
            {
                actual.push((
                    row.required(Column::TripId).unwrap().to_owned(),
                    row.get(Column::StopHeadsign).unwrap().to_owned(),
                ));
            }
            assert_eq!(actual, expected, "chunk {chunk}");
            assert_eq!(actual.len(), 3);
            assert_eq!(calls, 7, "evaluate each nonempty key once, chunk {chunk}");
            let mut bulk = Reader::<_>::new(Input { bytes, chunk }, Table::StopTimes);
            let mut filter = gtfs::csv::Prefilter::new(
                |id| id == b"keep",
                |block| gtfs::csv::may_contain(block, b"keep"),
            );
            let mut actual = Vec::new();
            while let Some(row) = bulk
                .next_matching(Column::StopId, &mut filter)
                .await
                .unwrap()
            {
                actual.push((
                    row.required(Column::TripId).unwrap().to_owned(),
                    row.get(Column::StopHeadsign).unwrap().to_owned(),
                ));
            }
            assert_eq!(actual, expected, "bulk chunk {chunk}");
        });
    }
}

#[test]
fn ignored_values_are_not_validated_but_selected_rows_and_headers_are() {
    let bytes = b"trip_id,arrival_time,departure_time,stop_id,stop_sequence\nnoise,x,x,other,\xff,extra\nselected,25:00:00,25:00:00,keep,1\n";
    for chunk in [1, 7, 512, 1024] {
        block_on(async {
            let mut strict = Reader::<_>::new(Input { bytes, chunk }, Table::StopTimes);
            assert!(matches!(strict.next().await, Err(Error::InvalidUtf8)));
            let mut selective = Reader::<_>::new(Input { bytes, chunk }, Table::StopTimes);
            let row = selective
                .next_matching(Column::StopId, &mut |id: &[u8]| id == b"keep")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.stop_time().unwrap().trip_id, "selected");
            assert!(
                selective
                    .next_matching(Column::StopId, &mut |id: &[u8]| id == b"keep")
                    .await
                    .unwrap()
                    .is_none()
            );
            let bytes = b"trip_id,arrival_time,departure_time,stop_id,stop_sequence\nselected,25:00:00,25:00:00,keep,\xff\n";
            let mut selective = Reader::<_>::new(Input { bytes, chunk }, Table::StopTimes);
            assert!(matches!(
                selective
                    .next_matching(Column::StopId, &mut |id: &[u8]| id == b"keep")
                    .await,
                Err(Error::InvalidUtf8)
            ));
            let bytes = b"trip_id,arrival_time,departure_time,stop_id,stop_sequence\nselected,25:00:00,25:00:00,keep,1,extra\n";
            let mut selective = Reader::<_>::new(Input { bytes, chunk }, Table::StopTimes);
            assert!(matches!(
                selective
                    .next_matching(Column::StopId, &mut |id: &[u8]| id == b"keep")
                    .await,
                Err(Error::InvalidField("CSV record width"))
            ));
            let mut selective = Reader::<_>::new(
                Input {
                    bytes: b"stop_id\nother\n",
                    chunk,
                },
                Table::StopTimes,
            );
            assert!(matches!(
                selective
                    .next_matching(Column::StopId, &mut |_: &[u8]| false)
                    .await,
                Err(Error::MissingColumn(_))
            ));
        });
    }
}

#[test]
fn rejecting_every_record_still_detects_a_bad_zip_crc() {
    block_on(async {
        let mut bytes = include_bytes!("../../checks/device-cpu/stop_times.zip").to_vec();
        bytes[14] ^= 1; // Corrupt expected local CRC, preserving the CSV payload.
        let mut workspace = zip::Workspace::new();
        let mut zip = zip::Reader::new(
            Input {
                bytes: &bytes,
                chunk: 37,
            },
            &mut workspace,
        );
        assert_eq!(
            zip.next_entry().await.unwrap().unwrap().table(),
            Some(Table::StopTimes)
        );
        let mut selective = Reader::<_>::new(zip, Table::StopTimes);
        let mut filter = gtfs::csv::Prefilter::new(|_| false, |_| false);
        assert!(matches!(
            selective.next_matching(Column::StopId, &mut filter).await,
            Err(Error::Io(embedded_io_async::ErrorKind::InvalidData))
        ));
    });
}

#[test]
fn block_false_positives_and_escaped_identity_never_drop_selected_rows() {
    let bytes = b"route_id,route_type,route_short_name\nother,3,keep\nkeep,3,actual\n";
    let quoted = b"route_id,route_type,route_short_name\nother,3,\"a\"\"b\"\n\"a\"\"b\",3,actual\n";
    for (bytes, key) in [
        (bytes.as_slice(), b"keep".as_slice()),
        (quoted.as_slice(), b"a\"b".as_slice()),
    ] {
        for chunk in [1, 3, 7, 31, 512, 1024] {
            block_on(async {
                let mut rows = Reader::<_>::new(Input { bytes, chunk }, Table::Routes);
                let mut filter = gtfs::csv::Prefilter::new(
                    |id| id == key,
                    |block| gtfs::csv::may_contain(block, key),
                );
                let row = rows
                    .next_matching(Column::RouteId, &mut filter)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(row.get(Column::RouteShortName), Some("actual"));
                assert!(
                    rows.next_matching(Column::RouteId, &mut filter)
                        .await
                        .unwrap()
                        .is_none()
                );
            });
        }
    }
}
