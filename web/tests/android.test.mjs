import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { tabletSettings, checkAndroidRelease, supportsTablet } from "../android-installer.js";
const fixture = JSON.parse(readFileSync(new URL("../../tablet/tests/fixtures/settings.json", import.meta.url)));
const source = { feed_id: fixture.feed_id, timezone: "Europe/Warsaw", groups: fixture.groups, automatic: fixture.automatic, wifi_password: "never send this" };
const options = { keep_awake: true, camera_motion: false, idle_seconds: 120, launch_on_boot: false };
test("Android shares group validation and includes real zone transitions without Wi-Fi credentials", () => {
  const result = tabletSettings(source, options, Date.UTC(2026, 9, 17));
  assert.equal(tabletSettings({ ...source, feed_id: "city_bus-gtfs" }, options).feed_id, "city_bus-gtfs");
  assert.deepEqual(result.groups, fixture.groups); assert.notEqual(result.groups, source.groups);
  assert.ok(result.zone.length > 8 && result.zone.length <= 404);
  assert.deepEqual(Object.keys(result).sort(), Object.keys(fixture).sort());
  assert.ok(!JSON.stringify(result).includes("never send this"));
  assert.throws(() => tabletSettings({ ...source, timezone: null }, options));
  assert.throws(() => tabletSettings({ ...source, groups: [{ ...fixture.groups[0], picks: [] }] }, options));
  assert.throws(() => tabletSettings(source, { ...options, keep_awake: false, camera_motion: true }));
  assert.throws(() => tabletSettings(source, { ...options, idle_seconds: 29 }));
  const bad = structuredClone(source); bad.groups[0].picks[0].walk = 100;
  assert.throws(() => tabletSettings(bad, options));
});
test("APK metadata and tablet capability gate reject unsupported installation targets", () => {
  const release = { version: 1, package: "org.nostdrs.timetable", min_sdk: 21, abis: ["armeabi-v7a", "arm64-v8a"], file: "timetable.apk", size: 300000, sha256: "a".repeat(64), certificate_sha256: "b".repeat(64), signing: "development" };
  assert.equal(checkAndroidRelease(release), release);
  for (const change of [{ file: "../other.apk" }, { sha256: "wrong" }, { package: "other" }, { size: 33 * 1024 * 1024 }, { abis: ["arm64-v8a"] }, { signing: "unknown" }]) assert.throws(() => checkAndroidRelease({ ...release, ...change }));
  assert.ok(supportsTablet({ sdk: 22, abi: "armeabi-v7a\n" }));
  assert.ok(!supportsTablet({ sdk: 19, abi: "armeabi-v7a" })); assert.ok(!supportsTablet({ sdk: 28, abi: "x86" }));
});
