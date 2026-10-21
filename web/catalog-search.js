import { routeTopology } from "./route-topology.js";
// Poznań labels use Polish collation in either page language; line numbers sort naturally.
export const compareLabels = new Intl.Collator("pl", { numeric: true, sensitivity: "base" }).compare;
// Build both directions once; stop-first browsing never rescans all route links.
export function searchText(value) {
  return value.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[łŁ]/g, "l").toLowerCase();
}

export function catalogIndex(catalog) {
  const compareLabels = new Intl.Collator(catalog?.language || "pl", { numeric: true, sensitivity: "base" }).compare;
  const text = (value) => typeof value === "string";
  if (!catalog || !Array.isArray(catalog.routes) || !Array.isArray(catalog.stops) || !Array.isArray(catalog.links)
      || !catalog.routes.every((route) => route && text(route.id) && text(route.line) && Array.isArray(route.headsigns) && route.headsigns.length === 2 && route.headsigns.every(text))
      || !catalog.stops.every((stop) => stop && text(stop.id) && text(stop.name) && text(stop.code))) throw new Error("The route catalogue is invalid. Try downloading it again.");
  const routes = catalog.routes.map((route) => searchText(`${route.line} ${route.headsigns.join(" ")}`));
  const stops = catalog.stops.map((stop) => searchText(`${stop.name} ${stop.code}`));
  const compareRoutes = (a, b) => compareLabels(catalog.routes[a].line, catalog.routes[b].line) || compareLabels(catalog.routes[a].id, catalog.routes[b].id) || a - b;
  const compareStops = (a, b) => compareLabels(catalog.stops[a].name, catalog.stops[b].name) || compareLabels(catalog.stops[a].code, catalog.stops[b].code) || compareLabels(catalog.stops[a].id, catalog.stops[b].id) || a - b;
  const links = catalog.routes.map(() => []);
  const serving = catalog.stops.map(() => new Set());
  const groups = [], groupOf = [], byName = new Map();
  catalog.stops.forEach((stop, i) => {
    // Identity keeps Polish accents: search normalization is deliberately wider.
    const name = stop.name.normalize("NFC").trim().replace(/\s+/g, " ").toLocaleLowerCase("pl");
    if (!byName.has(name)) {
      byName.set(name, groups.length);
      groups.push({ name: stop.name, stops: [] });
    }
    const group = byName.get(name);
    groups[group].stops.push(i);
    groupOf[i] = group;
  });
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
  const index = {
    groups, groupOf, compareLabels,
    topology: routeTopology(catalog),
    routes(query = "", stop = -1) {
      const words = terms(query);
      return routes.flatMap((text, i) => (stop < 0 || serving[stop]?.has(i)) && matches(text, words) ? [i] : [])
        .sort(compareRoutes);
    },
    stops(route, direction = -1, query = "") {
      const words = terms(query);
      const candidates = route < 0
        ? serving.flatMap((routes, stop) => routes.size ? [stop] : [])
        : [...new Set((links[route] ?? []).filter(({ bits }) => direction < 0 || bits & 1 << direction).map(({ stop }) => stop))];
      return candidates.filter((stop) => matches(stops[stop], words)).sort(compareStops);
    },
    directions(route, stop = -1) {
      return [0, 1].filter((direction) => (links[route] ?? []).some((link) => (stop < 0 || link.stop === stop) && link.bits & 1 << direction));
    },
    stopGroups(route = -1, query = "") {
      return [...new Set(index.stops(route, -1, query).map(stop => groupOf[stop]))];
    },
    groupRoutes(query = "", group = -1) {
      const allowed = group < 0 ? null : new Set((groups[group]?.stops ?? []).flatMap(stop => [...serving[stop]]));
      return index.routes(query).filter(route => !allowed || allowed.has(route));
    },
    boardingChoices(route, group) {
      const members = new Set(groups[group]?.stops ?? []), bits = new Map();
      for (const link of links[route] ?? []) if (members.has(link.stop)) bits.set(link.stop, (bits.get(link.stop) ?? 0) | link.bits);
      return [...bits].flatMap(([stop, mask]) => {
        const directions = [0, 1].filter(d => mask & 1 << d);
        // -1 always queries one exact platform. Never combine different stop IDs
        // into the firmware's one-stop slot, even when their names match.
        if (mask & 4 || (mask & 3) === 3) directions.push(-1);
        return directions.map(direction => ({ stop, direction, all: direction < 0 && !!(mask & 3), key: `${stop}:${direction}` }));
      });
    },
  };
  return index;
}
