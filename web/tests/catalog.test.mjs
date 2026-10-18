import test from "node:test";
import assert from "node:assert/strict";
import { catalogIndex, searchText } from "../catalog-search.js";

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
test("boarding stop searches preserve route and direction membership", () => {
  const index = catalogIndex(catalog);
  assert.deepEqual(index.stops(1, -1, "lak"), [0]);
  assert.deepEqual(index.stops(1, 0, "sro02"), []);
  assert.deepEqual(index.stops(1, 1, "sro02"), [1]);
  assert.deepEqual(index.stops(0, -1, "srodka"), []);
  assert.deepEqual(index.stops(2, -1), [2]);
  assert.deepEqual(index.directions(2), []);
  assert.deepEqual(index.stops(-1), []);
});
test("malformed catalogue links cannot become selectable choices", () => {
  for (const link of [[0, 99, 1], [-1, 0, 1], [0, 0, 8], null, [0, 0]]) {
    assert.throws(() => catalogIndex({ ...catalog, links: [link] }), /invalid stop link/);
  }
  assert.throws(() => catalogIndex({ ...catalog, routes: [null] }), /catalogue is invalid/);
});
