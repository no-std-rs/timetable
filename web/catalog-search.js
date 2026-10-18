import { routeTopology } from "./route-topology.js";
// Build both directions once; stop-first browsing never rescans all route links.
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
  const names = catalog.stops.map((stop) => searchText(stop.name));
  const codes = catalog.stops.map((stop) => searchText(stop.code));
  const links = catalog.routes.map(() => []);
  const serving = catalog.stops.map(() => new Set());
  for (const link of catalog.links) {
    if (!Array.isArray(link) || link.length !== 3) throw new Error("The route catalogue has an invalid stop link.");
    const [route, stop, bits] = link;
    if (!Number.isInteger(route) || !Number.isInteger(stop) || !Number.isInteger(bits)
        || !links[route] || !catalog.stops[stop] || bits < 1 || bits > 7) throw new Error("The route catalogue has an invalid stop link.");
    links[route].push({ stop, bits });
    serving[stop].add(route);
  }
  const terms = (query) => searchText(query).trim().split(/\s+/).filter(Boolean);
  const matches = (text, query) => query.every((term) => text.includes(term));
  return {
    topology: routeTopology(catalog),
    routes(query = "", stop = -1) {
      const words = terms(query), exact = searchText(query).trim();
      return routes.flatMap((text, i) => (stop < 0 || serving[stop]?.has(i)) && matches(text, words) ? [i] : [])
        .sort((a, b) => Number(lines[b] === exact) - Number(lines[a] === exact));
    },
    stops(route, direction = -1, query = "") {
      const words = terms(query), exact = searchText(query).trim();
      const candidates = route < 0
        ? serving.flatMap((routes, stop) => routes.size ? [stop] : [])
        : [...new Set((links[route] ?? []).filter(({ bits }) => direction < 0 || bits & 1 << direction).map(({ stop }) => stop))];
      const rank = (stop) => Number(codes[stop] === exact) * 4 + Number(names[stop] === exact) * 2 + Number(names[stop].startsWith(exact));
      return candidates.filter((stop) => matches(stops[stop], words)).sort((a, b) => rank(b) - rank(a));
    },
    directions(route, stop = -1) {
      return [0, 1].filter((direction) => (links[route] ?? []).some((link) => (stop < 0 || link.stop === stop) && link.bits & 1 << direction));
    },
  };
}
