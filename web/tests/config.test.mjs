import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { encodeConfig, encodeSettingsPartition, validateConfig, crc32, walkingChance, manifestWithSettings, CONFIG_OFFSET } from "../config.js";
const fixture = JSON.parse(await readFile(new URL("../../display/tests/settings.json", import.meta.url), "utf8"));
test("settings header protects the exact UTF-8 JSON and generation", () => {
  const image = encodeConfig(fixture, 4294967295), v = new DataView(image.buffer);
  assert.equal(new TextDecoder().decode(image.subarray(0, 8)), "TTABLE01");
  assert.equal(image.length, 8192);
  assert.equal(v.getUint32(8, true), 4294967295);
  const length = v.getUint32(12, true), checksum = v.getUint32(16, true);
  v.setUint32(16, 0, true);
  assert.equal(crc32(image.subarray(0, 24 + length)), checksum);
  assert.deepEqual(JSON.parse(new TextDecoder().decode(image.subarray(24, 24 + length))), fixture);
});
test("invalid settings fail before USB preparation", () => {
  for (const change of [(c) => c.groups.push(...c.groups, c.groups[0]), (c) => c.groups[0].picks[0].direction = 2, (c) => c.groups[0].picks[0].walk = 61, (c) => c.wifi_ssid = "\u0105".repeat(17), (c) => c.groups[0].name = "", (c) => c.groups[0].days = 128, (c) => c.groups[0].picks[0].route = "x".repeat(33), (c) => c.wifi_password = "short"]) {
    const c = structuredClone(fixture);
    change(c);
    assert.throws(() => validateConfig(c));
  }
});
test("walking estimate matches the Rust uniform model", () => {
  assert.equal(walkingChance(5, 6, 2, 1), 0);
  assert.equal(walkingChance(7, 6, 2, 1), 50);
  assert.equal(walkingChance(9, 6, 2, 1), 100);
  assert.equal(walkingChance(7, 6, 0, 1), 100);
  assert.equal(walkingChance(6, 6, 0, 1), 0);
});
test("manifest refuses a wrong board, layout or settings protocol", () => {
  const r = { chip: "ESP32-C3", board: "GUITION ESP32-2424S012-V1.0", flash_size: 4194304, config_offset: CONFIG_OFFSET, config_version: 2, revision: "a".repeat(40), sha256: "b".repeat(64), size: 1e5 };
  const m = manifestWithSettings(r, "blob:firmware", "blob:settings");
  assert.deepEqual(m.builds[0].parts.map((p) => p.offset), [0, CONFIG_OFFSET]);
  for (const bad of [{ chip: "ESP32" }, { config_offset: 0 }, { config_version: 1 }, { size: CONFIG_OFFSET }, { sha256: "x" }]) assert.throws(() => manifestWithSettings({ ...r, ...bad }, "a", "b"));
});

const wireFixture = await readFile(new URL("../../display/tests/settings.bin", import.meta.url));
test("battery and escaped browser settings match the Rust wire regression", async () => {
  const c = JSON.parse(await readFile(new URL("../../display/tests/settings-escaped.json", import.meta.url), "utf8"));
  const image = await readFile(new URL("../../display/tests/settings-escaped.bin", import.meta.url));
  assert.deepEqual(Buffer.from(encodeConfig(c)), image);
  for (const battery of [null, 1, "true"]) assert.throws(() => validateConfig({ ...c, battery }));
});
test("browser settings match the committed Rust decoder fixture", () => { assert.deepEqual(Buffer.from(encodeConfig(fixture)), wireFixture); });
test("USB settings replace both banks so an old generation cannot take precedence", () => {
  const partition = encodeSettingsPartition(fixture);
  assert.equal(partition.length, 65536);
  assert.deepEqual(Buffer.from(partition.subarray(0, 8192)), wireFixture);
  assert.ok(partition.subarray(8192).every((b) => b === 255));
});
