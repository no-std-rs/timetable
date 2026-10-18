// Exact ordered trip patterns, including repeated visits and non-boarding calls.
export function routeTopology(catalog) {
  const via = catalog.via_stops ?? [];
  const patterns = catalog.patterns ?? [];
  if (!Array.isArray(via) || !Array.isArray(patterns)) throw new Error("Invalid route paths.");
  const stops = [...catalog.stops, ...via];
  const byRoute = catalog.routes.map(() => []), byStop = catalog.stops.map(() => []);
  const membership = new Map();
  for (const [r, s, bits] of catalog.links) membership.set(`${r}:${s}`, (membership.get(`${r}:${s}`) ?? 0) | bits);
  if (!stops.every(s => s && [s.id, s.name, s.code].every(v => typeof v === "string"))) throw new Error("Invalid route paths.");
  patterns.forEach((p, id) => {
    if (!p || !Number.isInteger(p.route) || !byRoute[p.route] || ![-1, 0, 1].includes(p.direction)
        || typeof p.headsign !== "string" || !Number.isInteger(p.trips) || p.trips < 1 || !Array.isArray(p.calls) || !p.calls.length) throw new Error("Invalid route path.");
    p.calls.forEach((call, at) => {
      if (!Array.isArray(call) || ![2, 3].includes(call.length) || !Number.isInteger(call[0]) || !stops[call[0]]
          || !Number.isInteger(call[1]) || call[1] < 0 || call[1] > 3 || (call.length === 3 && typeof call[2] !== "string")) throw new Error("Invalid route call.");
      if (call[1] !== 1) {
        const bit = p.direction < 0 ? 4 : 1 << p.direction;
        if (!((membership.get(`${p.route}:${call[0]}`) ?? 0) & bit)) throw new Error("Route path contradicts boarding membership.");
        byStop[call[0]].push({ id, at });
      }
    });
    byRoute[p.route].push(id);
  });
  const paths = (route, direction = -1) => (byRoute[route] ?? []).filter(id => direction < 0 || patterns[id].direction === direction)
    .sort((a, b) => patterns[b].trips - patterns[a].trips || a - b);
  const contexts = (stop, route = -1, direction = -1) => {
    const distinct = new Map();
    for (const { id, at } of byStop[stop] ?? []) {
      const p = patterns[id];
      if ((route >= 0 && p.route !== route) || (direction >= 0 && p.direction !== direction)) continue;
      const next = p.calls[at + 1]?.[0];
      const headsign = p.calls[at][2] || p.headsign || stops[p.calls.at(-1)[0]].name;
      const key = JSON.stringify([p.route, p.direction, headsign, next]);
      const entry = distinct.get(key) ?? { route: p.route, direction: p.direction, headsign, next, trips: 0 };
      entry.trips += p.trips;
      distinct.set(key, entry);
    }
    return [...distinct.values()].sort((a, b) => b.trips - a.trips || a.route - b.route);
  };
  return { stops, patterns, paths, contexts };
}

// A prefix trie with identical suffixes shared. Every edge comes from a real
// trip; repeated stops stay distinct occurrences and cannot create graph cycles.
export function pathGraph(patterns, ids) {
  const root = { children: new Map() }, paths = new Map();
  for (const id of ids) {
    let parent = root;
    const path = [];
    for (const call of patterns[id].calls) {
      // Destination text can differ along a shared trunk. Keep it on the
      // original pattern, outside physical graph identity.
      const key = JSON.stringify(call.slice(0, 2));
      if (!parent.children.has(key)) parent.children.set(key, { call, children: new Map() });
      parent = parent.children.get(key);
      path.push(parent);
    }
    paths.set(id, path);
  }
  const nodes = [], suffixes = new Map();
  function intern(node) {
    const children = [...node.children.values()].map(intern).sort((a, b) => a - b);
    const key = JSON.stringify([node.call.slice(0, 2), children]);
    if (!suffixes.has(key)) {
      suffixes.set(key, nodes.length);
      nodes.push({ call: node.call, children, lanes: [], rank: 0 });
    }
    node.id = suffixes.get(key);
    return node.id;
  }
  [...root.children.values()].forEach(intern);
  const ordered = ids.map(id => paths.get(id).map(node => node.id));
  ordered.forEach((path, lane) => path.forEach(id => nodes[id].lanes.push(lane)));
  // Children are interned first, so reverse order is topological.
  for (let i = nodes.length - 1; i >= 0; i--) for (const child of nodes[i].children) nodes[child].rank = Math.max(nodes[child].rank, nodes[i].rank + 1);
  const depth = Math.max(1, ...nodes.map(n => n.rank)), lanes = Math.max(1, ids.length - 1);
  nodes.forEach(n => {
    n.x = 16 + n.rank / depth * 568;
    n.y = 18 + (n.lanes.reduce((a, b) => a + b, 0) / n.lanes.length) / lanes * 124;
  });
  return { nodes, paths: new Map(ids.map((id, i) => [id, ordered[i]])) };
}
