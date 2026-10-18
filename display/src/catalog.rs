//! Read a compact route/stop catalogue directly from flash; no RAM copy.
use crate::config::crc32;

const HEADER: usize = 32;
const ROUTE: usize = 168;
const STOP: usize = 112;
const LINK: usize = 6;

#[derive(Clone, Copy)]
pub struct Catalog<'a> {
    bytes: &'a [u8],
    routes: usize,
    stops: usize,
    links: usize,
    sorted_links: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Route<'a> {
    pub id: &'a str,
    pub line: &'a str,
    pub headsigns: [&'a str; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct Stop<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub code: &'a str,
}

fn text(bytes: &[u8]) -> Option<&str> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    if bytes[end..].iter().any(|b| *b != 0) {
        return None;
    }
    let value = core::str::from_utf8(&bytes[..end]).ok()?;
    (!value.chars().any(char::is_control)).then_some(value)
}
fn word(bytes: &[u8], offset: usize) -> usize {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]]).into()
}

impl<'a> Catalog<'a> {
    pub fn new(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < HEADER || &bytes[..8] != b"TTCAT001" || bytes[28..32] != [0; 4] {
            return None;
        }
        let routes = word(bytes, 16);
        let stops = word(bytes, 18);
        let links = u32::from_le_bytes(bytes[20..24].try_into().ok()?) as usize;
        let len = HEADER
            .checked_add(routes * ROUTE)?
            .checked_add(stops * STOP)?
            .checked_add(links.checked_mul(LINK)?)?;
        if bytes.len() != len
            || crc32(&bytes[HEADER..]) != u32::from_le_bytes(bytes[24..28].try_into().ok()?)
        {
            return None;
        }
        let mut result = Self {
            bytes,
            routes,
            stops,
            links,
            sorted_links: true,
        };
        for i in 0..routes {
            let r = result.route(i)?;
            if r.id.is_empty() || r.line.is_empty() {
                return None;
            }
        }
        for i in 0..stops {
            if result.stop(i)?.id.is_empty() {
                return None;
            }
        }
        let mut previous_route = 0;
        for link in bytes[HEADER + routes * ROUTE + stops * STOP..]
            .as_chunks::<LINK>()
            .0
        {
            if word(link, 0) >= routes
                || word(link, 2) >= stops
                || !(1..=7).contains(&link[4])
                || link[5] != 0
            {
                return None;
            }
            let route = word(link, 0);
            result.sorted_links &= route >= previous_route;
            previous_route = route;
        }
        Some(result)
    }
    pub fn date(&self) -> &'a str {
        core::str::from_utf8(&self.bytes[8..16]).unwrap_or("unknown")
    }
    pub fn route_count(&self) -> usize {
        self.routes
    }
    pub fn stop_count(&self) -> usize {
        self.stops
    }
    pub fn route(&self, index: usize) -> Option<Route<'a>> {
        if index >= self.routes {
            return None;
        }
        let p = HEADER + index * ROUTE;
        let b = &self.bytes[p..p + ROUTE];
        Some(Route {
            id: text(&b[..32])?,
            line: text(&b[32..40])?,
            headsigns: [text(&b[40..104])?, text(&b[104..168])?],
        })
    }
    pub fn stop(&self, index: usize) -> Option<Stop<'a>> {
        if index >= self.stops {
            return None;
        }
        let p = HEADER + self.routes * ROUTE + index * STOP;
        let b = &self.bytes[p..p + STOP];
        Some(Stop {
            id: text(&b[..32])?,
            name: text(&b[32..96])?,
            code: text(&b[96..112])?,
        })
    }
    pub fn route_stops(
        &self,
        route: usize,
        direction: i8,
    ) -> impl Iterator<Item = (usize, Stop<'a>)> + '_ {
        let mut links = self.bytes[HEADER + self.routes * ROUTE + self.stops * STOP..]
            .as_chunks::<LINK>()
            .0;
        if self.sorted_links {
            let start = links.partition_point(|link| word(link, 0) < route);
            let count = links[start..].partition_point(|link| word(link, 0) == route);
            links = &links[start..start + count];
        }
        links.iter().take(self.links).filter_map(move |b| {
            if word(b, 0) != route
                || (direction >= 0 && (direction > 1 || b[4] & (1 << direction) == 0))
            {
                return None;
            }
            let index = word(b, 2);
            Some((index, self.stop(index)?))
        })
    }
}
