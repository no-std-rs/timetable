import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SETTINGS_KEY, REMEMBER_KEY, loadDraft, saveDraft, forgetDraft } from "../settings-store.js";
import { encodeConfig } from "../config.js";
const config = JSON.parse(await readFile(new URL("../../display/tests/settings.json", import.meta.url)));
const memory = () => { const values = new Map(); return { getItem: k => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) }; };
test("remembered settings roundtrip Wi-Fi, groups and power without weakening USB validation", () => {
  const storage = memory(), draft = { version: 1, active: 0, config: structuredClone(config) };
  draft.config.battery = true; draft.config.wifi_password = "half"; draft.config.groups[0].name = "";
  saveDraft(storage, draft); assert.deepEqual(loadDraft(storage), draft);
  assert.throws(() => encodeConfig(loadDraft(storage).config));
  draft.config.wifi_password = config.wifi_password; draft.config.groups[0].name = "Dom";
  saveDraft(storage, draft); assert.doesNotThrow(() => encodeConfig(loadDraft(storage).config));
  storage.removeItem(SETTINGS_KEY); assert.equal(loadDraft(storage), null);
});
test("corrupt, oversized or invalid stored choices cannot enter the configuration", () => {
  const storage = memory();
  for (const raw of ["{", "x".repeat(20001), JSON.stringify({ version: 2, active: 0, config })]) {
    storage.setItem(SETTINGS_KEY, raw); assert.throws(() => loadDraft(storage));
  }
  for (const change of [d => d.active = 5, d => d.config.automatic = "yes", d => d.config.wifi_password = "x".repeat(257), d => d.config.groups[0].picks[0].direction = 2]) {
    const d = { version: 1, active: 0, config: structuredClone(config) }; change(d);
    assert.throws(() => saveDraft(storage, d));
  }
});
test("forget erases saved credentials even when a full quota rejects the preference write", () => {
  const storage = memory();
  saveDraft(storage, { version: 1, active: 0, config });
  storage.setItem(REMEMBER_KEY, "true");
  storage.setItem = () => { throw Object.assign(Error(), { name: "QuotaExceededError" }); };
  assert.throws(() => forgetDraft(storage), { name: "QuotaExceededError" });
  assert.equal(loadDraft(storage), null);
});


test("saved model identifiers cannot resolve inherited object properties", async()=>{
  const {device}=await import("../devices.js");
  for(const id of ["__proto__","constructor","ESP32-C6-LCD-1.47"])assert.throws(()=>device(id),/Unknown display/);
  assert.equal(device("waveshare-c6").chipId,13);
});
