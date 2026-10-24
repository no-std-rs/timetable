import { device } from "./devices.js";
import { validateConfig, validateJourney } from "./config.js";
import { validFeedId } from "./feed-directory.js";
export const SETTINGS_KEY = "timetable.settings.v1";
export const REMEMBER_KEY = "timetable.remember";
const bytes = value => new TextEncoder().encode(value).length;
// A bounded browser draft may contain an unfinished Wi-Fi password/group name.
// The actual USB image still goes through the complete device validator.
export function validateDraft(draft) {
  if (!draft || draft.version !== 1 || !Number.isInteger(draft.active) || !draft.config) throw new Error("Invalid saved settings.");
  if (draft.feed_id !== undefined && !validFeedId(draft.feed_id)) throw new Error("Invalid saved settings.");
  if (draft.device_id !== undefined) device(draft.device_id);
  if (draft.orientation !== undefined && !["round", "portrait", "landscape"].includes(draft.orientation)) throw new Error("Invalid saved settings.");
  const c = draft.config;
  if (typeof c.wifi_ssid !== "string" || bytes(c.wifi_ssid) > 128 || c.wifi_ssid.includes("\0")
      || typeof c.wifi_password !== "string" || bytes(c.wifi_password) > 256 || c.wifi_password.includes("\0")
      || !Array.isArray(c.groups) || c.groups.some(g => !g || typeof g.name !== "string" || [...g.name].length > 24 || /\p{Cc}/u.test(g.name))
      || draft.active < 0 || draft.active >= c.groups.length) throw new Error("Invalid saved settings.");
  for (const g of c.groups) if (g.journey) validateJourney(g.journey,true);
  validateConfig({ ...c, wifi_ssid: "draft", wifi_password: "", groups: c.groups.map(g => { const copy={ ...g, name: "draft" };delete copy.journey;return copy; }) });
  return draft;
}
export function loadDraft(storage) {
  const text = storage.getItem(SETTINGS_KEY);
  if (!text) return null;
  if (text.length > 20000) throw new Error("Invalid saved settings.");
  return validateDraft(JSON.parse(text));
}
export function saveDraft(storage, draft) {
  validateDraft(draft);
  storage.setItem(SETTINGS_KEY, JSON.stringify(draft));
  storage.setItem(`${SETTINGS_KEY}.${draft.feed_id || "mdb-982"}`, JSON.stringify(draft));
}
export function loadFeedDraft(storage, id) {
  if (!validFeedId(id)) return null;
  const text = storage.getItem(`${SETTINGS_KEY}.${id}`);
  if (!text) return null;
  if (text.length > 20000) throw new Error("Invalid saved settings.");
  const draft = validateDraft(JSON.parse(text));
  if ((draft.feed_id || "mdb-982") !== id) throw new Error("Invalid saved settings.");
  return draft;
}
export function forgetDraft(storage) {
  // Deletion must precede preference writes: a full storage quota can reject
  // setItem even while removeItem can still erase the credential snapshot.
  storage.removeItem(SETTINGS_KEY);
  // Erase credentials in every remembered network before writing preferences.
  const keys = [];
  for (let i=0;i<storage.length;i++) { const key=storage.key(i); if(key?.startsWith(`${SETTINGS_KEY}.`)) keys.push(key); }
  for (const key of keys) storage.removeItem(key);
  storage.setItem(REMEMBER_KEY, "false");
}
