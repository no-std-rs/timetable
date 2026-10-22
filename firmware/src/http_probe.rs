//! Opt-in hardware comparison. Partial ZIP members are never published or cached.
use embedded_io_async::Read;
use gtfs::{Error, Table, csv, http::ByteRange, zip};
use timetable_firmware::{
    cooperative::Cooperative,
    download::{self, Http as _, Operation},
};

struct Probe<'a> {
    workspace: &'a mut zip::Workspace,
    mode: u8,
    decoded: usize,
    rows: usize,
    checksum: u32,
    blocks: usize,
    rejected_blocks: usize,
    stopped: Option<Error>,
}
impl Operation for Probe<'_> {
    async fn read<R: Read>(&mut self, mut body: R) -> Result<(), download::Error> {
        crate::wake_gate::NETWORK.reset();
        crate::wake_gate::WORK.reset();
        let mut out = [0; 512];
        if self.mode != 0 {
            let mut zip = zip::Reader::new(&mut body, self.workspace);
            zip.next_entry().await?;
            if self.mode == 1 {
                loop {
                    match zip.read(&mut out).await {
                        Ok(0) => break,
                        Ok(n) => self.decoded += n,
                        Err(e) => {
                            self.stopped = Some(e);
                            break;
                        }
                    }
                }
            } else {
                let mut csv = csv::Reader::<_>::new(Cooperative::new(zip), Table::StopTimes);
                loop {
                    let row = if self.mode == 3 {
                        csv.next_matching(
                            csv::Column::StopId,
                            &mut csv::Prefilter::new(
                                |id| id == b"4314",
                                |block| {
                                    self.blocks += 1;
                                    let possible = csv::may_contain(block, b"4314");
                                    self.rejected_blocks += usize::from(!possible);
                                    possible
                                },
                            ),
                        )
                        .await
                    } else {
                        csv.next().await
                    };
                    match row {
                        Ok(None) => break,
                        Ok(Some(row)) => {
                            if row.get(csv::Column::StopId) == Some("4314") {
                                self.rows += 1;
                                for column in [
                                    csv::Column::TripId,
                                    csv::Column::StopId,
                                    csv::Column::ArrivalTime,
                                    csv::Column::DepartureTime,
                                    csv::Column::StopSequence,
                                ] {
                                    self.checksum = self.checksum.wrapping_mul(31).wrapping_add(
                                        crc32fast::hash(row.get(column).unwrap_or("").as_bytes()),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            self.stopped = Some(e);
                            break;
                        }
                    }
                }
            }
        }
        // Drain the exact HTTP range even when the deliberately partial member
        // reaches its expected decode error. This is not ZIP validation.
        while body
            .read(&mut out)
            .await
            .map_err(|_| download::Error::Network)?
            != 0
        {}
        Ok(())
    }
}

pub async fn run(stack: embassy_net::Stack<'_>, workspace: &mut zip::Workspace) {
    let http = crate::transport::Http(stack);
    let url =
        "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip";
    let range = ByteRange::new(3070233, 3070233 + 512 * 1024 - 1).unwrap();
    // Two stage controls, then three pairs with alternating order in one binary.
    for (round, (mode, gated)) in [
        (0, true),
        (1, true),
        (2, true),
        (3, true),
        (3, true),
        (2, true),
        (2, true),
        (3, true),
    ]
    .into_iter()
    .enumerate()
    {
        crate::wake_gate::set_enabled(gated);
        crate::scheduler::reset();
        log::info!(
            "PROBE round={} mode={} gated={} partial 512 KiB (0 raw, 1 inflate, 2 strict CSV, 3 selective CSV; stop 4314)",
            round,
            mode,
            gated
        );
        let mut probe = Probe {
            workspace,
            mode,
            decoded: 0,
            rows: 0,
            checksum: 0,
            blocks: 0,
            rejected_blocks: 0,
            stopped: None,
        };
        let start = embassy_time::Instant::now();
        let result = http
            .fetch(url, Some((range, Some(11207002))), &mut probe)
            .await;
        log::info!(
            "PROBE round={} mode={} gated={} total_ms={} decoded={} rows={} checksum={:08x} decode_end={:?} result={:?}",
            round,
            mode,
            gated,
            start.elapsed().as_millis(),
            probe.decoded,
            probe.rows,
            probe.checksum,
            probe.stopped,
            result.map(|_| ())
        );
        crate::scheduler::report();
        log::info!(
            "PROBE blocks={} rejected_blocks={}",
            probe.blocks,
            probe.rejected_blocks
        );
        log::info!(
            "GATE network {:?} work {:?}",
            crate::wake_gate::NETWORK.counts(),
            crate::wake_gate::WORK.counts()
        );
    }
    crate::wake_gate::set_enabled(true);
}
