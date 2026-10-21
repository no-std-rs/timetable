import test from "node:test";
import assert from "node:assert/strict";
import { catalogIndex, searchText } from "../catalog-search.js";
import { pathGraph } from "../route-topology.js";
import { readFile } from "node:fs/promises";

const catalog = {
  routes: [
    { id: "R40", line: "40", headsigns: ["Łódź Żółkiewskiego", "Rynek"] },
    { id: "R4", line: "4", headsigns: ["Dębiec", "Śródka"] },
    { id: "RX", line: "X", headsigns: ["", ""] },
  ],
  stops: [
    { id: "S0", name: "Łąkowa", code: "LAK01" },
    { id: "S1", name: "Śródka", code: "SRO02" },
    { id: "S2", name: "Żurawia", code: "ZUR03" },
  ],
  links: [[0, 0, 3], [1, 0, 1], [1, 1, 2], [2, 2, 4]],
};
test("Polish search supports case, accents and ł", () => {
  assert.equal(searchText("ŁÓDŹ Żółć"), "lodz zolc");
  const index = catalogIndex(catalog);
  assert.deepEqual(index.routes("  LODZ  zolki  "), [0]);
  assert.deepEqual(index.routes("srodka"), [1]);
  assert.deepEqual(index.routes("4"), [1, 0]);
  assert.deepEqual(index.routes("missing"), []);
});
test("stop identity uses the feed's Turkish case rules", () => {
  const stops = ["ISIK", "ısık", "isik", "İSİK"].map((name, i) => ({id: String(i), name, code: String(i)}));
  const index = catalogIndex({...catalog, language: "tr", stops, links: stops.map((_, i) => [0, i, 1])});
  assert.equal(index.groupOf[0], index.groupOf[1]);
  assert.equal(index.groupOf[2], index.groupOf[3]);
  assert.notEqual(index.groupOf[0], index.groupOf[2]);
});
test("Turkish searches match lowercase stops, line labels and destinations", () => {
  const c = {language:"tr", routes:[{id:"R",line:"IŞIK",headsigns:["İZMİR",""]}],
    stops:[{id:"S",name:"IŞIK",code:""}],links:[[0,0,1]]};
  const index = catalogIndex(c);
  assert.deepEqual(index.stops(-1,-1,"ışık"), [0]);
  assert.deepEqual(index.routes("ışık"), [0]);
  assert.deepEqual(index.routes("izmir"), [0]);
  assert.deepEqual(index.routes("İZMİR"), [0]);
});
test("boarding stop searches preserve route and direction membership", () => {
  const index = catalogIndex(catalog);
  assert.deepEqual(index.stops(1, -1, "lak"), [0]);
  assert.deepEqual(index.stops(1, 0, "sro02"), []);
  assert.deepEqual(index.stops(1, 1, "sro02"), [1]);
  assert.deepEqual(index.stops(0, -1, "srodka"), []);
  assert.deepEqual(index.stops(2, -1), [2]);
  assert.deepEqual(index.directions(2), []);
  assert.deepEqual(index.stops(-1), [0, 1, 2]);
});
test("stop-first search finds platforms globally and restricts lines and directions", () => {
  const index = catalogIndex(catalog);
  assert.deepEqual(index.stops(-1, -1, "LAKOWA lak01"), [0]);
  assert.deepEqual(index.stops(-1, -1, "srodka"), [1]);
  assert.deepEqual(index.routes("", 0), [1, 0]);
  assert.deepEqual(index.routes("4", 0), [1, 0]);
  assert.deepEqual(index.routes("", 1), [1]);
  assert.deepEqual(index.directions(1, 0), [0]);
  assert.deepEqual(index.directions(1, 1), [1]);
  assert.deepEqual(index.directions(2, 2), []);
  const duplicate = catalogIndex({ ...catalog, links: [...catalog.links, [1, 0, 2]] });
  assert.deepEqual(duplicate.stops(1), [0, 1]);
  assert.deepEqual(duplicate.routes("", 0), [1, 0]);
  assert.deepEqual(duplicate.directions(1, 0), [0, 1]);
});
test("malformed catalogue links cannot become selectable choices", () => {
  for (const link of [[0, 99, 1], [-1, 0, 1], [0, 0, 8], null, [0, 0]]) {
    assert.throws(() => catalogIndex({ ...catalog, links: [link] }), /invalid stop link/);
  }
  assert.throws(() => catalogIndex({ ...catalog, routes: [null] }), /catalogue is invalid/);
});

const pathsCatalog = {
  routes: [{ id: "R", line: "4", headsigns: ["East", "West"] }],
  stops: [
    { id: "OUT", name: "Central", code: "C01" },
    { id: "IN", name: "Central", code: "C02" },
    { id: "NEXT", name: "East street", code: "E01" },
    { id: "VIA", name: "Branch", code: "B01" },
    { id: "WEST", name: "West street", code: "W01" },
  ],
  via_stops: [{ id: "END", name: "East terminal", code: "E99" }],
  links: [[0, 0, 1], [0, 1, 2], [0, 2, 1], [0, 3, 1], [0, 4, 2]],
  patterns: [
    { route: 0, direction: 0, headsign: "East", calls: [[0, 0], [2, 0], [5, 1]], trips: 10 },
    { route: 0, direction: 0, headsign: "East", calls: [[0, 0], [3, 2], [2, 0], [5, 1]], trips: 2 },
    { route: 0, direction: 1, headsign: "West", calls: [[1, 0], [4, 0]], trips: 12 },
    { route: 0, direction: 0, headsign: "East", calls: [[0, 0], [2, 0], [0, 0, "Loop East"], [5, 1]], trips: 1 },
  ],
};
test("same-name platforms use actual ordered paths, stop headsigns and boarding calls", () => {
  const index = catalogIndex(pathsCatalog), t = index.topology;
  assert.deepEqual(index.stops(0, -1, "central"), [0, 1]);
  assert.equal(t.contexts(0, 0, 0)[0].headsign, "East");
  assert.equal(t.contexts(0, 0, 0)[0].next, 2);
  assert.equal(t.contexts(1, 0, 1)[0].headsign, "West");
  assert.equal(t.contexts(1, 0, 1)[0].next, 4);
  assert.ok(t.contexts(0).some(c => c.headsign === "Loop East" && c.next === 5));
  assert.deepEqual(t.contexts(5), []);
  assert.ok(!index.stops(0).includes(5));
  assert.deepEqual(t.paths(0, 0), [0, 1, 3]);
  assert.deepEqual(t.paths(0, 1), [2]);
});
test("route graph shares forks and rejoins without collapsing repeated visits", () => {
  const graph = pathGraph(pathsCatalog.patterns, [0, 1, 3]);
  const straight = graph.paths.get(0), branch = graph.paths.get(1), loop = graph.paths.get(3);
  assert.equal(straight[0], branch[0]);
  assert.equal(straight.at(-1), branch.at(-1));
  const simple = pathGraph(pathsCatalog.patterns, [0, 1]);
  assert.equal(simple.paths.get(0).at(-2), simple.paths.get(1).at(-2));
  assert.notEqual(loop[0], loop[2]);
  const actualEdges = new Set([...graph.paths.values()].flatMap(path => path.slice(1).map((n, i) => `${path[i]}:${n}`)));
  graph.nodes.forEach((node, i) => node.children.forEach(child => {
    assert.ok(actualEdges.has(`${i}:${child}`));
    assert.ok(graph.nodes[child].rank > node.rank);
  }));
});
test("malformed paths cannot offer a platform outside boarding membership", () => {
  for (const patch of [{ direction: 2 }, { route: -1 }, { calls: [[99, 0]] }, { calls: [[5, 0]] }, { calls: [[1, 0]] }, { calls: [[0, 4]] }, { calls: [[0, 0, null]] }, { trips: 0 }]) {
    assert.throws(() => catalogIndex({ ...pathsCatalog, patterns: [{ ...pathsCatalog.patterns[0], ...patch }] }));
  }
});
test("different destination signs still share physical trunks and suffixes", async () => {
  const patterns = pathsCatalog.patterns.slice(0, 2).map((p, i) => ({ ...p, calls: p.calls.map(c => [c[0], c[1], i ? "Other destination" : "Main destination"]) }));
  const g = pathGraph(patterns, [0, 1]);
  assert.equal(g.paths.get(0)[0], g.paths.get(1)[0]);
  assert.equal(g.paths.get(0).at(-2), g.paths.get(1).at(-2));
  const actual = JSON.parse(await readFile(new URL("../../catalog/catalog.json", import.meta.url)));
  const route = actual.routes.findIndex(r => r.line === "1");
  const candidates = actual.patterns.map((p, id) => ({ p, id })).filter(({ p }) => p.route === route && p.direction === 0);
  let checked = false;
  for (const a of candidates) for (const b of candidates) {
    if (a.id >= b.id || a.p.calls[0][2] === b.p.calls[0][2]) continue;
    let count = 0;
    while (a.p.calls[count] && b.p.calls[count] && a.p.calls[count][0] === b.p.calls[count][0] && a.p.calls[count][1] === b.p.calls[count][1]) count++;
    if (count < 20) continue;
    const graph = pathGraph(actual.patterns, [a.id, b.id]);
    assert.deepEqual(graph.paths.get(a.id).slice(0, count), graph.paths.get(b.id).slice(0, count));
    checked = true;
  }
  assert.ok(checked, "official line 1 has a long common trunk with different destination signs");
});

test("named stops unite platform lines and direction choices without merging physical IDs", () => {
  const index = catalogIndex(pathsCatalog), group = index.groupOf[0];
  assert.equal(index.groupOf[1], group);
  assert.deepEqual(index.groups[group].stops, [0, 1]);
  assert.deepEqual(index.stopGroups(-1, "central"), [group]);
  assert.deepEqual(index.stopGroups(-1, "c02"), [group]);
  assert.deepEqual(index.groupRoutes("", group), [0]);
  assert.deepEqual(index.boardingChoices(0, group).map(c => [c.stop, c.direction]), [[0, 0], [1, 1]]);
  assert.ok(index.boardingChoices(0, group).every(c => !c.all));
  const c = structuredClone(pathsCatalog); c.links.push([0, 1, 1]);
  const mixed = catalogIndex(c).boardingChoices(0, group);
  assert.ok(mixed.some(c => c.stop === 1 && c.direction === 0));
  assert.ok(mixed.some(c => c.stop === 1 && c.direction === -1 && c.all));
  assert.equal(new Set(mixed.map(c => c.key)).size, mixed.length);
  const unsigned = catalogIndex(catalog);
  assert.deepEqual(unsigned.boardingChoices(2, unsigned.groupOf[2]).map(c => [c.stop, c.direction, c.all]), [[2, -1, false]]);
});
test("stop grouping keeps distinct accented names and unions routes across platforms", () => {
  const c = structuredClone(catalog);
  c.stops.push({ id: "S3", name: "ŁĄKOWA", code: "LAK02" }, { id: "S4", name: "Lakowa", code: "X01" });
  c.links.push([2, 3, 4], [2, 4, 4]);
  const index = catalogIndex(c);
  assert.equal(index.groupOf[0], index.groupOf[3]);
  assert.notEqual(index.groupOf[0], index.groupOf[4]);
  assert.deepEqual(index.groupRoutes("", index.groupOf[0]), [1, 0, 2]);
  assert.deepEqual(index.boardingChoices(2, index.groupOf[0]).map(c => c.stop), [3]);
});

test("shuffled catalogues browse in Polish alphabetical and natural line order without changing IDs", () => {
  const names = ["Żurawia", "Zwycięstwa", "Śródka", "Słowiańska", "Łąkowa", "Lubelska", "AWF", "Admiralska", "Źródlana", "Ćwikłowo", "Czarna", "Lasek"];
  const c = {
    routes: ["T10", "100", "10", "T2", "2"].map((line, i) => ({id:`R${i}`,line,headsigns:["East","West"]})),
    stops: names.map((name, i) => ({id:`S${i}`,name,code:`P${i}`})),
    links: names.map((_,i)=>[0,i,3]).concat([1,2,3,4].map(route=>[route,0,1])),
  };
  const before = structuredClone(c), index = catalogIndex(c);
  const ordered = ["Admiralska", "AWF", "Czarna", "Ćwikłowo", "Lasek", "Lubelska", "Łąkowa", "Słowiańska", "Śródka", "Zwycięstwa", "Źródlana", "Żurawia"];
  assert.deepEqual(index.stops(-1).map(i=>c.stops[i].name),ordered);
  assert.deepEqual(index.stops(0,1).map(i=>c.stops[i].name),ordered);
  assert.deepEqual(index.stopGroups().map(i=>index.groups[i].name),ordered);
  assert.deepEqual(index.routes().map(i=>c.routes[i].line),["2","10","100","T2","T10"]);
  assert.deepEqual(index.groupRoutes("",index.groupOf[0]).map(i=>c.routes[i].line),["2","10","100","T2","T10"]);
  assert.deepEqual(c,before); // Stable catalogue identities still link routes, platforms and paths.
});
