// Build once after loading the public catalogue; typing only scans one route's links.
export function searchText(value) {
  return value.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[łŁ]/g, "l").toLowerCase();
}

export function catalogIndex(catalog) {
  const text = (value) => typeof value === "string";
  if (!catalog || !Array.isArray(catalog.routes) || !Array.isArray(catalog.stops) || !Array.isArray(catalog.links)
      || !catalog.routes.every((route) => route && text(route.id) && text(route.line) && Array.isArray(route.headsigns) && route.headsigns.length === 2 && route.headsigns.every(text))
      || !catalog.stops.every((stop) => stop && text(stop.id) && text(stop.name) && text(stop.code))) throw new Error("The route catalogue is invalid. Try downloading it again.");
  const routes = catalog.routes.map((route) => searchText(`${route.line} ${route.headsigns.join(" ")}`));
  const lines = catalog.routes.map((route) => searchText(route.line));
  const stops = catalog.stops.map((stop) => searchText(`${stop.name} ${stop.code}`));
  const links = catalog.routes.map(() => []);
  for (const link of catalog.links) {
    if (!Array.isArray(link) || link.length !== 3) throw new Error("The route catalogue has an invalid stop link.");
    const [route, stop, bits] = link;
    if (!Number.isInteger(route) || !Number.isInteger(stop) || !Number.isInteger(bits)
        || !links[route] || !catalog.stops[stop] || bits < 1 || bits > 7) throw new Error("The route catalogue has an invalid stop link.");
    links[route].push({ stop, bits });
  }
  const terms = (query) => searchText(query).trim().split(/\s+/).filter(Boolean);
  const matches = (text, query) => query.every((term) => text.includes(term));
  return {
    routes(query = "") {
      const words = terms(query), exact = searchText(query).trim();
      return routes.flatMap((text, i) => matches(text, words) ? [i] : [])
        .sort((a, b) => Number(lines[b] === exact) - Number(lines[a] === exact));
    },
    stops(route, direction = -1, query = "") {
      const words = terms(query);
      return (links[route] ?? []).filter(({ stop, bits }) => (direction < 0 || bits & 1 << direction) && matches(stops[stop], words)).map(({ stop }) => stop);
    },
    directions(route) {
      return [0, 1].filter((direction) => (links[route] ?? []).some(({ bits }) => bits & 1 << direction));
    },
  };
}
