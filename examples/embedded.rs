//! Board-independent, allocator-free reqwless example functions.
//! Supply your network stack, DNS, verified TLS configuration and flash writer.
use embedded_io_async::{Read, Write};
use embedded_nal_async::{Dns, TcpConnect};
use reqwless::{
    client::HttpClient,
    request::{Method, RequestBuilder},
};
use timetable::{Error, Table, TripIndex, csv, query::DepartureBuffer, stream, zip};

/// Everything retained from mirror bootstrap is caller-owned and bounded.
/// Keep route/stop names in flash or stream them to the UI as needed.
pub struct OfficialQuery<'a, 'storage> {
    pub manifest: &'a zip::Manifest,
    pub trips: &'a TripIndex<'storage, 32>,
    pub stops: &'a [&'a str],
    pub workspace: &'a mut zip::Workspace,
    pub output: &'a mut DepartureBuffer<'storage, 32>,
}

/// A single official HTTP body, directly decoded without a complete ZIP cache.
/// The prepared trip index and manifest must describe the same feed snapshot.
/// `stream::collect` publishes no partial cache on mismatch, error or cancellation.
pub async fn collect_official<T: TcpConnect, D: Dns>(
    client: &mut HttpClient<'_, T, D>,
    url: &str,
    query: OfficialQuery<'_, '_>,
    http_buffer: &mut [u8],
) -> Result<(), Error> {
    query.output.clear();
    let headers = [("Accept-Encoding", "identity")];
    let mut request = client
        .request(Method::GET, url)
        .await
        .map_err(|_| Error::Io(embedded_io_async::ErrorKind::ConnectionAborted))?
        .headers(&headers);
    let response = request
        .send(http_buffer)
        .await
        .map_err(|_| Error::Io(embedded_io_async::ErrorKind::ConnectionAborted))?;
    if response.status.0 != 200 {
        return Err(Error::InvalidZip);
    }
    for (name, value) in response.headers() {
        if name.eq_ignore_ascii_case("content-encoding") && value != b"identity" {
            return Err(Error::InvalidZip);
        }
    }
    stream::collect(
        response.body().reader(),
        query.manifest,
        query.trips,
        query.stops,
        query.workspace,
        query.output,
    )
    .await
}

/// Stream a pinned official archive to flash/SD using constant RAM. The caller
/// commits the new cache snapshot only after validating it with `Feed::open`.
pub async fn download<T: TcpConnect, D: Dns, W: Write>(
    client: &mut HttpClient<'_, T, D>,
    url: &str,
    storage: &mut W,
    http_buffer: &mut [u8],
    copy_buffer: &mut [u8],
) -> Result<u64, Error> {
    if copy_buffer.is_empty() {
        return Err(Error::BufferTooSmall);
    }
    let headers = [("Accept-Encoding", "identity")];
    let mut request = client
        .request(Method::GET, url)
        .await
        .map_err(|_| Error::Io(embedded_io_async::ErrorKind::ConnectionAborted))?
        .headers(&headers);
    let response = request
        .send(http_buffer)
        .await
        .map_err(|_| Error::Io(embedded_io_async::ErrorKind::ConnectionAborted))?;
    if response.status.0 != 200 {
        return Err(Error::InvalidZip);
    }
    let mut body = response.body().reader();
    let mut total = 0;
    loop {
        let n = body
            .read(copy_buffer)
            .await
            .map_err(|_| Error::UnexpectedEof)?;
        if n == 0 {
            break;
        }
        storage
            .write_all(&copy_buffer[..n])
            .await
            .map_err(|_| Error::Io(embedded_io_async::ErrorKind::WriteZero))?;
        total += n as u64;
    }
    storage
        .flush()
        .await
        .map_err(|_| Error::Io(embedded_io_async::ErrorKind::Other))?;
    Ok(total)
}

/// Extract routes directly from a response body without caching the ZIP.
/// Other members are skipped as compressed bytes; no shapes are inflated.
pub async fn stream_routes<R: Read>(
    body: R,
    workspace: &mut zip::Workspace,
    mut visit: impl FnMut(timetable::Route<'_>) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut archive = zip::Reader::new(body, workspace);
    while let Some(entry) = archive.next_entry().await? {
        if entry.table() == Some(Table::Routes) {
            let mut rows = csv::Reader::<_>::new(&mut archive, Table::Routes);
            while let Some(row) = rows.next().await? {
                visit(row.route()?)?;
            }
            return Ok(());
        }
    }
    Err(Error::MissingTable(Table::Routes))
}

fn main() {
    println!(
        "Use download() and stream_routes() with your board's network and flash adapters; see README.md."
    );
}
