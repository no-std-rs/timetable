import { device, releaseDevice, DEFAULT_DEVICE } from "./devices.js";
const CONFIG_OFFSET = 4128768;
const IMAGE_SIZE = 8192;
const HEADER = 24;
const utf8 = new TextEncoder();
const exact = (object, fields) => object && typeof object === "object" && !Array.isArray(object) && Object.keys(object).length === fields.length && fields.every((k) => Object.hasOwn(object, k));
const integer = (value, min, max) => Number.isInteger(value) && value >= min && value <= max;
const text = (value, min, max) => typeof value === "string" && utf8.encode(value).length >= min && utf8.encode(value).length <= max && !/\p{Cc}/u.test(value);
function validateConfig(c) {
  const fields = ["wifi_ssid", "wifi_password", "automatic", "groups"];
  if (c && Object.hasOwn(c, "battery")) fields.push("battery");
  if (!exact(c, fields) || typeof c.automatic !== "boolean" || c.battery !== undefined && typeof c.battery !== "boolean" || !Array.isArray(c.groups) || c.groups.length < 1 || c.groups.length > 4) throw new Error("Use one to four departure groups and a valid power mode.");
  if (typeof c.wifi_ssid !== "string" || c.wifi_ssid.includes("\0") || utf8.encode(c.wifi_ssid).length < 1 || utf8.encode(c.wifi_ssid).length > 32) throw new Error("The Wi-Fi name must fit in 32 UTF-8 bytes.");
  const p = c.wifi_password;
  if (typeof p !== "string" || p.includes("\0") || !(p === "" || utf8.encode(p).length >= 8 && utf8.encode(p).length <= 63 || /^[a-fA-F0-9]{64}$/.test(p))) throw new Error("Use an 8\u201363 byte Wi-Fi password, or leave it empty for an open network.");
  for (const g of c.groups) {
    const fields = ["name", "days", "start", "end", "picks"];
    if (g && Object.hasOwn(g, "journey")) fields.push("journey");
    if (!exact(g, fields) || !text(g.name, 1, 24) || !g.name.trim() || !integer(g.days, 0, 127) || !integer(g.start, 0, 1439) || !integer(g.end, 0, 1439) || !Array.isArray(g.picks) || g.picks.length > 4) throw new Error("Check the group name (up to 24 UTF-8 bytes), schedule and four-choice limit.");
    if (g.journey != null) validateJourney(g.journey);
    for (const p2 of g.picks) {
      if (!exact(p2, ["route", "line", "stop", "stop_name", "stop_code", "headsign", "direction", "walk", "spread", "margin"]) || !text(p2.route, 1, 32) || !text(p2.line, 1, 8) || !text(p2.stop, 1, 32) || !text(p2.stop_name, 0, 64) || !text(p2.stop_code, 0, 16) || !text(p2.headsign, 0, 64) || !integer(p2.direction, -1, 1) || !integer(p2.walk, 0, 60) || !integer(p2.spread, 0, 15) || !integer(p2.margin, 0, 10)) throw new Error("Check the line, stop and walking times. Walk: 0\u201360 min; variation: 0\u201315; boarding buffer: 0\u201310.");
    }
  }
  return c;
}
export function validateJourney(j, allowEmpty = false) {
  if (!exact(j,["origins","targets","transfer_minutes","max_transfers"]) || !integer(j.transfer_minutes,0,30) || !integer(j.max_transfers,0,2)) throw new Error("Check the journey stops, walking times and transfer settings.");
  for (const stops of [j.origins,j.targets]) {
    if (!Array.isArray(stops) || stops.length < (allowEmpty?0:1) || stops.length>4) throw new Error("Add one to four starting stops and one to four target stops.");
    const ids = new Set();
    for (const s of stops) {
      const fields=["stop","walk"];if(s && Object.hasOwn(s,"stop_name"))fields.push("stop_name");if(s && Object.hasOwn(s,"stop_code"))fields.push("stop_code");
      if (!exact(s,fields) || !text(s.stop,1,32) || !text(s.stop_name??"",0,64) || !text(s.stop_code??"",0,16) || !integer(s.walk,0,60) || ids.has(s.stop)) throw new Error("Check the journey stops, walking times and transfer settings.");
      ids.add(s.stop);
    }
  }
  return j;
}
function crc32(data) {
  let crc = 4294967295;
  for (const byte of data) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) crc = crc >>> 1 ^ 3988292384 & -(crc & 1);
  }
  return (crc ^ 4294967295) >>> 0;
}
function encodeConfig(config, generation = 0) {
  validateConfig(config);
  if (!integer(generation, 0, 4294967295)) throw new Error("Invalid settings generation.");
  const payload = utf8.encode(JSON.stringify(config));
  if (payload.length > IMAGE_SIZE - HEADER) throw new Error("These settings exceed the device\u2019s memory limit. Use shorter labels or fewer choices.");
  const image = new Uint8Array(IMAGE_SIZE).fill(255);
  image.set(utf8.encode("TTABLE01"));
  const view = new DataView(image.buffer);
  view.setUint32(8, generation, true);
  view.setUint32(12, payload.length, true);
  view.setUint32(16, 0, true);
  view.setUint32(20, 0, true);
  image.set(payload, HEADER);
  view.setUint32(16, crc32(image.subarray(0, HEADER + payload.length)), true);
  return image;
}
// Replace both banks even when the flasher's optional full-chip erase is off.
// Otherwise an older on-device bank with a higher generation would win at boot.
function encodeSettingsPartition(config) {
  const partition = new Uint8Array(65536).fill(255);
  partition.set(encodeConfig(config));
  return partition;
}
function manifestWithSettings(release, firmware, settings, selectedDevice = DEFAULT_DEVICE) {
  const board = device(selectedDevice);
  if (releaseDevice(release) !== board) throw new Error("This firmware does not match the selected display or settings format.");
  if (release.config_offset !== CONFIG_OFFSET || release.config_version !== 2 || !/^[0-9a-f]{40}$/.test(release.revision) || !/^[0-9a-f]{64}$/.test(release.sha256) || !integer(release.size, 65536, CONFIG_OFFSET - 1)) throw new Error("This firmware does not match the selected display or settings format.");
  return { name: "timetable", version: release.revision.slice(0, 12), new_install_prompt_erase: true, new_install_improv_wait_time: 0, builds: [{ chipFamily: board.chip, parts: [{ path: firmware, offset: 0 }, { path: settings, offset: CONFIG_OFFSET }] }] };
}
function walkingChance(minutes, walk, spread, margin) {
  const available = minutes - margin, low = Math.max(0, walk - spread), high = walk + spread;
  return available < low ? 0 : available >= high ? 100 : Math.floor((available - low) * 100 / (high - low));
}
export {
  CONFIG_OFFSET,
  IMAGE_SIZE,
  crc32,
  encodeConfig,
  encodeSettingsPartition,
  manifestWithSettings,
  validateConfig,
  walkingChance
};
