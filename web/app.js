import { encodeSettingsPartition, manifestWithSettings, validateConfig, walkingChance } from "./config.js";
import { catalogIndex } from "./catalog-search.js";
const $ = (id) => document.getElementById(id);
const element = (tag, text, className) => {
  const e = document.createElement(tag);
  if (text !== void 0) e.textContent = text;
  if (className) e.className = className;
  return e;
};
const emptyGroup = (name) => ({ name, days: 0, start: 360, end: 660, picks: [] });
const state = { automatic: true, groups: [emptyGroup("Morning"), emptyGroup("Evening")], active: 0, editing: null, catalog: null, index: null, undo: null, release: null, releaseGeneration: 0, firmwareUrl: null, urls: [] };
const group = () => state.groups[state.active];
const minute = (value) => {
  const [h, m] = value.split(":").map(Number);
  return h * 60 + m;
};
const hhmm = (value) => `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(value % 60).padStart(2, "0")}`;
function message(text, error = false) {
  $("status").textContent = text;
  $("status").classList.toggle("error", error);
  $("status").setAttribute("role", error ? "alert" : "status");
}
function forgetInstall() {
  $("installer").manifest = "";
  $("installer").removeAttribute("manifest");
  $("install-area").hidden = true;
  for (const u of state.urls.splice(0)) URL.revokeObjectURL(u);
}
function changed() {
  state.undo = null;
  $("undo-area").hidden = true;
  forgetInstall();
  preview();
  if (state.release) message("Your settings stay here. Prepare when you\u2019re ready.");
}
function removalSnapshot() {
  return { groups: structuredClone(state.groups), active: state.active };
}
function offerUndo(snapshot, text) {
  state.undo = snapshot;
  $("undo-message").textContent = text;
  $("undo-area").hidden = false;
}
$("undo-removal").addEventListener("click", () => {
  if (!state.undo) return;
  state.groups = state.undo.groups;
  state.active = state.undo.active;
  renderGroup();
  changed();
  $("group-name").focus();
});
function preview() {
  const g = group(), p = g.picks[0];
  $("preview-group").textContent = g.name || "Your group";
  $("preview-journey").hidden = !p;
  $("preview-empty").hidden = !!p;
  $("preview-mode").textContent = p ? `09:41 / ${state.automatic && g.days ? "SCHEDULED" : "MANUAL"}` : "YOUR GROUP / NO STOPS";
  if (!p) return;
  $("preview-line").textContent = p.line;
  $("preview-direction").textContent = p.headsign || "Either direction";
  $("preview-stop").textContent = p.stop_name;
  const left = 12 - p.walk - p.margin;
  const chance = walkingChance(12, p.walk, p.spread, p.margin);
  $("preview-leave").textContent = chance === 0 ? "Not enough walk time" : left > 0 ? `Leave in ${left} min` : "Leave now";
  $("preview-leave").classList.toggle("urgent", left <= 0);
  $("preview-walk").textContent = `${p.walk}m walk \xB7 ${chance}% walk est.`;
  const next = g.picks[1] ?? p;
  $("preview-next-line").textContent = `${next.line} \xB7 ${next.headsign || "Either direction"}`;
  $("preview-last-line").textContent = `${p.line} \xB7 ${p.headsign || "Either direction"}`;
}
function renderTabs() {
  const tabs = $("group-tabs");
  tabs.replaceChildren();
  state.groups.forEach((g, i) => {
    const b = element("button", g.name || "Your group", "group-tab");
    b.type = "button";
    b.role = "tab";
    b.id = `group-tab-${i}`;
    b.tabIndex = i === state.active ? 0 : -1;
    b.setAttribute("aria-controls", "group-panel");
    b.setAttribute("aria-selected", String(i === state.active));
    b.addEventListener("click", () => {
      state.active = i;
      renderGroup();
      $(`group-tab-${i}`).focus();
    });
    b.addEventListener("keydown", (event) => {
      const count = state.groups.length;
      const index = { ArrowRight: (i + 1) % count, ArrowLeft: (i + count - 1) % count, Home: 0, End: count - 1 }[event.key];
      if (index === undefined) return;
      event.preventDefault();
      state.active = index;
      renderGroup();
      $(`group-tab-${index}`).focus();
    });
    tabs.append(b);
  });
  $("add-group").hidden = state.groups.length >= 4;
  $("group-panel").setAttribute("aria-labelledby", `group-tab-${state.active}`);
}
$("add-group").addEventListener("click", () => {
  if (state.groups.length >= 4) return;
  state.groups.push(emptyGroup(`Group ${state.groups.length + 1}`));
  state.active = state.groups.length - 1;
  renderGroup();
  changed();
  $("group-name").focus();
});
function groupNameError(value) {
  if (!value.trim()) return "Give this group a name.";
  if (new TextEncoder().encode(value).length > 24) return "Shorten this name so it fits on the display.";
  return /\p{Cc}/u.test(value) ? "Use a name without invisible characters." : "";
}
function validateGroupName() {
  const name = $("group-name");
  name.setCustomValidity(groupNameError(name.value));
}
function renderGroup() {
  renderTabs();
  const g = group();
  $("group-name").value = g.name;
  validateGroupName();
  $("remove-group").hidden = state.groups.length < 2;
  $("scheduled").checked = g.days !== 0;
  $("schedule").hidden = g.days === 0;
  $("start-time").value = hhmm(g.start);
  $("end-time").value = hhmm(g.end);
  $("start-time").disabled = $("end-time").disabled = g.days === 0;
  $("days").replaceChildren();
  ["M", "T", "W", "T", "F", "S", "S"].forEach((name, i) => {
    const b = element("button", name, "day-button");
    b.type = "button";
    b.setAttribute("aria-label", ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][i]);
    b.setAttribute("aria-pressed", String((g.days & 1 << i) !== 0));
    b.addEventListener("click", () => {
      g.days ^= 1 << i;
      renderGroup();
      changed();
      $("days").children[i].focus();
    });
    $("days").append(b);
  });
  const rows = $("picks");
  rows.replaceChildren();
  if (g.picks.length === 0) rows.append(element("p", "Start with a line and the stop you walk to. You can change it on the display later.", "empty-picks"));
  g.picks.forEach((p, i) => {
    const row = element("div", void 0, "pick-row"), info = element("div", void 0, "pick-info");
    info.append(element("strong", p.headsign || "Either direction"), element("small", p.stop_name));
    const edit = element("button", "Edit", "pick-edit");
    edit.type = "button";
    edit.disabled = !state.catalog;
    edit.setAttribute("aria-label", `Edit ${p.line} at ${p.stop_name}`);
    edit.addEventListener("click", () => openPick(i));
    row.append(element("span", p.line, "pick-line"), info, element("span", `${p.walk}m walk`, "pick-walk"), edit);
    rows.append(row);
  });
  $("add-pick").hidden = g.picks.length >= 4;
  $("add-pick").disabled = !state.catalog;
  preview();
}
$("group-name").addEventListener("input", () => {
  group().name = $("group-name").value;
  validateGroupName();
  renderTabs();
  changed();
});
$("remove-group").addEventListener("click", () => {
  if (state.groups.length > 1) {
    const snapshot = removalSnapshot();
    state.groups.splice(state.active, 1);
    state.active = Math.min(state.active, state.groups.length - 1);
    renderGroup();
    changed();
    offerUndo(snapshot, "Group removed.");
    $("group-name").focus();
  }
});
$("scheduled").addEventListener("change", () => {
  group().days = $("scheduled").checked ? 31 : 0;
  renderGroup();
  changed();
});
for (const id of ["start-time", "end-time"]) $(id).addEventListener("input", () => {
  if ($(id).value) group()[id === "start-time" ? "start" : "end"] = minute($(id).value);
  changed();
});
$("preview-group").addEventListener("click", () => {
  state.active = (state.active + 1) % state.groups.length;
  renderGroup();
});
$("add-pick").addEventListener("click", () => openPick(null));
$("cancel-pick").addEventListener("click", () => $("pick-dialog").close());
$("show-password").addEventListener("click", () => {
  const show = $("wifi-password").type === "password";
  $("wifi-password").type = show ? "text" : "password";
  $("show-password").textContent = show ? "Hide" : "Show";
  $("show-password").setAttribute("aria-label", show ? "Hide password" : "Show password");
  $("show-password").setAttribute("aria-pressed", String(show));
});
for (const id of ["wifi-ssid", "wifi-password"]) $(id).addEventListener("input", changed);
function fillRoutes(selected = $("route-select").value) {
  const direction = Number($("direction-select").value), stop = $("stop-select").value;
  const matches = state.index.routes($("route-search").value);
  $("route-select").replaceChildren(new Option(matches.length ? "Choose a line" : "No matching lines", ""));
  for (const i of matches) $("route-select").append(new Option(state.catalog.routes[i].line, String(i)));
  $("route-select").value = selected !== "" && matches.includes(Number(selected)) ? selected : "";
  $("route-results").textContent = matches.length ? `${matches.length} ${matches.length === 1 ? "line" : "lines"} · search with or without Polish accents` : "No lines match. Try a line number or another destination.";
  fillDirections(direction, stop);
}
function fillDirections(value = -1, stop = $("stop-select").value) {
  const selected = $("route-select").value, route = selected === "" ? -1 : Number(selected);
  const r = state.catalog.routes[route];
  $("direction-select").replaceChildren(new Option("Either direction", "-1"));
  const directions = state.index.directions(route);
  if (r) for (const i of directions) $("direction-select").append(new Option(r.headsigns[i] || `Direction ${i + 1}`, String(i)));
  $("direction-select").value = String(directions.includes(value) ? value : -1);
  $("direction-select").disabled = !r;
  fillStops(stop);
}
function fillStops(selected = $("stop-select").value) {
  if (!state.catalog) return;
  const value = $("route-select").value, route = value === "" ? -1 : Number(value), direction = Number($("direction-select").value);
  const stops = state.index.stops(route, direction, $("stop-search").value);
  $("stop-select").replaceChildren(new Option(value === "" ? "Choose a line first" : stops.length ? "Choose a boarding stop" : "No matching stops", ""));
  for (const s of stops) {
    const stop = state.catalog.stops[s];
    $("stop-select").append(new Option(`${stop.name}${stop.code ? ` \xB7 ${stop.code}` : ""}`, String(s)));
  }
  $("stop-select").value = selected !== "" && stops.includes(Number(selected)) ? String(selected) : "";
  $("stop-select").disabled = value === "";
  $("stop-search").disabled = value === "";
  $("stop-results").textContent = value === "" ? "Choose your line, then its direction and boarding stop." : stops.length ? `${stops.length} boarding ${stops.length === 1 ? "stop" : "stops"} · names and platform codes` : "No boarding stops match. Clear the search or choose another direction.";
  updatePickAvailability();
}
function updatePickAvailability() {
  $("save-pick").disabled = $("route-select").value === "" || $("stop-select").value === "";
}
function openPick(index) {
  state.editing = index;
  $("pick-error").hidden = true;
  $("delete-pick").hidden = index === null;
  if (!state.catalog) {
    message("The route catalogue is still loading. Please try again shortly.", true);
    return;
  }
  const p = index === null ? null : group().picks[index];
  $("route-search").value = $("stop-search").value = "";
  const route = p ? state.catalog.routes.findIndex((r) => r.id === p.route && r.line === p.line) : -1;
  const stop = p ? state.catalog.stops.findIndex((s) => s.id === p.stop && (p.stop_code ? s.code === p.stop_code : s.name === p.stop_name)) : -1;
  fillRoutes(route < 0 ? "" : String(route));
  fillDirections(p?.direction ?? -1, stop < 0 ? "" : String(stop));
  if (p && (route < 0 || $("stop-select").value === "")) {
    $("pick-error").textContent = "This saved choice needs an updated line or stop. Choose it from the current catalogue.";
    $("pick-error").hidden = false;
  }
  $("walk-minutes").value = p?.walk ?? 6;
  $("walk-spread").value = p?.spread ?? 1;
  $("walk-margin").value = p?.margin ?? 1;
  $("pick-dialog").showModal();
}
$("route-search").addEventListener("input", () => fillRoutes());
$("stop-search").addEventListener("input", () => fillStops());
for (const [search, select] of [["route-search", "route-select"], ["stop-search", "stop-select"]]) {
  $(search).addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    $(select).focus();
  });
}
$("route-select").addEventListener("change", () => fillDirections());
$("direction-select").addEventListener("change", () => fillStops());
$("stop-select").addEventListener("change", updatePickAvailability);
$("pick-form").addEventListener("submit", (event) => {
  event.preventDefault();
  try {
    const route = Number($("route-select").value), stop = Number($("stop-select").value);
    const r = state.catalog.routes[route], s = state.catalog.stops[stop], direction = Number($("direction-select").value);
    if (!r || !s || !$("route-select").value || !$("stop-select").value || !state.index.stops(route, direction).includes(stop)) throw new Error("Choose a line and one of its boarding stops.");
    const p = { route: r.id, line: r.line, stop: s.id, stop_name: s.name, stop_code: s.code, headsign: direction < 0 ? "Both directions" : r.headsigns[direction], direction, walk: Number($("walk-minutes").value), spread: Number($("walk-spread").value), margin: Number($("walk-margin").value) };
    const draft = structuredClone(state.groups);
    if (state.editing === null) draft[state.active].picks.push(p);
    else draft[state.active].picks[state.editing] = p;
    validateConfig({ wifi_ssid: "validation", wifi_password: "", automatic: state.automatic, groups: draft });
    state.groups = draft;
    $("pick-dialog").close();
    renderGroup();
    changed();
    const index = state.editing ?? group().picks.length - 1;
    $("picks").children[index]?.querySelector("button")?.focus();
  } catch (e) {
    $("pick-error").textContent = e.message;
    $("pick-error").hidden = false;
  }
});
$("delete-pick").addEventListener("click", () => {
  const snapshot = removalSnapshot();
  if (state.editing !== null) group().picks.splice(state.editing, 1);
  $("pick-dialog").close();
  renderGroup();
  changed();
  offerUndo(snapshot, "Line and stop removed.");
  $("add-pick").focus();
});
function downloadJson(data, name) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2) + "\n"], { type: "application/json" }));
  const a = element("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1e3);
}
$("export-groups").addEventListener("click", () => {
  try {
    validateConfig({ wifi_ssid: "validation", wifi_password: "", automatic: state.automatic, groups: state.groups });
    downloadJson({ version: 1, automatic: state.automatic, groups: state.groups }, "timetable-groups.json");
  } catch (e) {
    message(e.message, true);
  }
});
$("import-groups").addEventListener("change", async (event) => {
  try {
    const f = event.target.files[0];
    if (!f) return;
    if (f.size > 16384) throw new Error("That group file is too large.");
    const data = JSON.parse(await f.text());
    if (data.version !== 1) throw new Error("Unsupported group file version.");
    validateConfig({ wifi_ssid: "validation", wifi_password: "", automatic: data.automatic, groups: data.groups });
    state.groups = data.groups;
    state.automatic = data.automatic;
    state.active = 0;
    renderGroup();
    changed();
  } catch (e) {
    message(e.message, true);
  } finally {
    event.target.value = "";
  }
});
$("settings").addEventListener("submit", (event) => {
  event.preventDefault();
  forgetInstall();
  try {
    const invalid = state.groups.findIndex((g) => groupNameError(g.name));
    if (invalid >= 0) {
      state.active = invalid;
      renderGroup();
      $("group-name").focus();
      $("group-name").reportValidity();
      throw new Error("Give each group a short name for the display.");
    }
    if (!state.release || !state.firmwareUrl) throw new Error("The published firmware is not available yet.");
    const settings = { wifi_ssid: $("wifi-ssid").value, wifi_password: $("wifi-password").value, automatic: state.automatic, groups: state.groups };
    const image = encodeSettingsPartition(settings), settingsUrl = URL.createObjectURL(new Blob([image], { type: "application/octet-stream" }));
    state.urls.push(settingsUrl);
    const manifest = manifestWithSettings(state.release, state.firmwareUrl, settingsUrl), url = URL.createObjectURL(new Blob([JSON.stringify(manifest)], { type: "application/json" }));
    state.urls.push(url);
    $("installer").manifest = url;
    $("install-area").hidden = false;
    message("Ready. Choose Install on display, then select its USB port.");
    $("install-area").scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth", block: "nearest" });
  } catch (e) {
    message(e.message, true);
  }
});
async function loadCatalog() {
  $("retry-catalog").hidden = true;
  $("retry-catalog").disabled = true;
  $("catalog-feedback").hidden = false;
  $("catalog-feedback").textContent = "Loading line and stop choices…";
  try {
    const response = await fetch("./catalog.json", { credentials: "omit" });
    if (!response.ok) throw new Error("Could not load the Poznań route choices. Try again when your connection is back.");
    const c = await response.json();
    if (!c || !Array.isArray(c.routes) || !Array.isArray(c.stops) || !Array.isArray(c.links) || c.routes.length > 65535 || c.stops.length > 65535 || typeof c.date !== "string" || !/^\d{8}$/.test(c.date)) throw new Error("The route catalogue is invalid. Try downloading it again.");
    const index = catalogIndex(c);
    state.catalog = c;
    state.index = index;
    $("catalog-status").textContent = `${c.routes.length} lines · ${c.stops.length.toLocaleString()} stops · ZTM Poznań · catalogue ${c.date.slice(0, 4)}-${c.date.slice(4, 6)}-${c.date.slice(6)}`;
    $("catalog-feedback").hidden = true;
    $("add-pick").disabled = false;
    document.querySelectorAll(".pick-edit").forEach((button) => { button.disabled = false; });
  } catch (e) {
    $("catalog-feedback").textContent = e.message;
    $("retry-catalog").hidden = false;
  } finally {
    $("retry-catalog").disabled = false;
  }
}
async function loadRelease() {
  const generation = ++state.releaseGeneration;
  forgetInstall();
  $("prepare").disabled = true;
  $("retry-release").hidden = true;
  $("retry-release").disabled = true;
  $("revision").hidden = true;
  $("download").hidden = true;
  $("download").removeAttribute("href");
  $("digest").textContent = "";
  $("no-download").hidden = false;
  $("release-state").replaceChildren(element("i"), document.createTextNode("Checking firmware"));
  message("Checking the latest firmware…");
  try {
    const response = await fetch("./latest.json", { cache: "no-store", credentials: "omit" });
    if (!response.ok) throw new Error("The firmware build could not be loaded. Keep editing your groups and retry below.");
    const candidate = await response.json();
    if (generation !== state.releaseGeneration) return;
    manifestWithSettings(candidate, "blob:firmware", "blob:settings");
    $("release-state").replaceChildren(element("i"), document.createTextNode("Downloading firmware"));
    message("Downloading the display app. Your groups stay editable.");
    const binary = await fetch("./timetable-display.bin", { cache: "no-store", credentials: "omit" });
    if (!binary.ok) throw new Error("The firmware download failed. Try the download again.");
    const bytes = await binary.arrayBuffer();
    if (generation !== state.releaseGeneration) return;
    if (bytes.byteLength !== candidate.size) throw new Error("The firmware changed during the download. Retry for the newest build.");
    $("release-state").replaceChildren(element("i"), document.createTextNode("Verifying firmware"));
    const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((b) => b.toString(16).padStart(2, "0")).join("");
    if (generation !== state.releaseGeneration) return;
    if (digest !== candidate.sha256) throw new Error("The firmware checksum didn’t match. Retry the download before installing.");
    if (state.firmwareUrl) URL.revokeObjectURL(state.firmwareUrl);
    state.firmwareUrl = URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" }));
    state.release = candidate;
    $("release-state").replaceChildren(element("i"), document.createTextNode("Firmware verified"));
    $("release-state").classList.remove("pending");
    $("revision").textContent = candidate.revision.slice(0, 8);
    $("revision").href = `https://github.com/no-std-rs/timetable/commit/${candidate.revision}`;
    $("revision").hidden = false;
    $("digest").textContent = `SHA-256 ${candidate.sha256}`;
    $("download").href = state.firmwareUrl;
    $("download").hidden = false;
    $("no-download").hidden = true;
    const supported = !!navigator.serial && window.isSecureContext;
    $("prepare").disabled = !supported;
    message(supported ? "Firmware verified. Add your Wi-Fi settings, then prepare your display." : "USB installation needs desktop Chrome or Edge over HTTPS or localhost. You can still download the firmware and save your groups.");
  } catch (e) {
    if (generation !== state.releaseGeneration) return;
    if (state.firmwareUrl) URL.revokeObjectURL(state.firmwareUrl);
    state.firmwareUrl = null;
    state.release = null;
    $("download").hidden = true;
    $("download").removeAttribute("href");
    $("release-state").replaceChildren(element("i"), document.createTextNode("Firmware unavailable"));
    $("release-state").classList.add("pending");
    $("retry-release").hidden = false;
    $("retry-release").disabled = false;
    message(e.message, true);
  }
}
$("retry-catalog").addEventListener("click", loadCatalog);
$("retry-release").addEventListener("click", loadRelease);
window.addEventListener("pagehide", () => {
  ++state.releaseGeneration;
  forgetInstall();
  if (state.firmwareUrl) URL.revokeObjectURL(state.firmwareUrl);
  state.firmwareUrl = null;
  state.release = null;
  $("wifi-ssid").value = "";
  $("wifi-password").value = "";
  $("wifi-password").type = "password";
  $("show-password").textContent = "Show";
  $("show-password").setAttribute("aria-label", "Show password");
  $("show-password").setAttribute("aria-pressed", "false");
  $("prepare").disabled = true;
  $("download").hidden = true;
  $("download").removeAttribute("href");
});
window.addEventListener("pageshow", (event) => {
  if (event.persisted) loadRelease();
});
renderGroup();
loadCatalog();
loadRelease();
