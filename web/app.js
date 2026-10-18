import { encodeSettingsPartition, manifestWithSettings, validateConfig, walkingChance } from "./config.js";
const $ = (id) => document.getElementById(id);
const element = (tag, text, className) => {
  const e = document.createElement(tag);
  if (text !== void 0) e.textContent = text;
  if (className) e.className = className;
  return e;
};
const emptyGroup = (name) => ({ name, days: 0, start: 360, end: 660, picks: [] });
const state = { automatic: true, groups: [emptyGroup("Morning"), emptyGroup("Evening")], active: 0, editing: null, catalog: null, release: null, firmwareUrl: null, urls: [] };
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
  forgetInstall();
  preview();
  if (state.release) message("Your settings stay here. Prepare when you\u2019re ready.");
}
function preview() {
  const g = group(), p = g.picks[0] ?? { line: "4", headsign: "G\xF3rczyn PKM", stop_name: "Most Teatralny", walk: 6, spread: 1, margin: 1 };
  $("preview-group").textContent = g.name || "Your group";
  $("preview-line").textContent = p.line;
  $("preview-direction").textContent = p.headsign || "Either direction";
  $("preview-stop").textContent = p.stop_name;
  const left = 12 - p.walk - p.margin;
  $("preview-leave").textContent = left > 0 ? `Leave in ${left} min` : "Leave now";
  $("preview-leave").classList.toggle("urgent", left <= 0);
  $("preview-walk").textContent = `${p.walk}m walk \xB7 ${walkingChance(12, p.walk, p.spread, p.margin)}% walk est.`;
  const next = g.picks[1] ?? { line: "12", headsign: "Staro\u0142\u0119ka PKM" };
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
    b.setAttribute("aria-selected", String(i === state.active));
    b.addEventListener("click", () => {
      state.active = i;
      renderGroup();
    });
    tabs.append(b);
  });
  if (state.groups.length < 4) {
    const b = element("button", "+", "add-group");
    b.type = "button";
    b.setAttribute("aria-label", "Add group");
    b.addEventListener("click", () => {
      state.groups.push(emptyGroup(`Group ${state.groups.length + 1}`));
      state.active = state.groups.length - 1;
      renderGroup();
      changed();
    });
    tabs.append(b);
  }
}
function renderGroup() {
  renderTabs();
  const g = group();
  $("group-name").value = g.name;
  $("remove-group").hidden = state.groups.length < 2;
  $("scheduled").checked = g.days !== 0;
  $("schedule").hidden = g.days === 0;
  $("start-time").value = hhmm(g.start);
  $("end-time").value = hhmm(g.end);
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
    edit.setAttribute("aria-label", `Edit ${p.line} at ${p.stop_name}`);
    edit.addEventListener("click", () => openPick(i));
    row.append(element("span", p.line, "pick-line"), info, element("span", `${p.walk}m walk`, "pick-walk"), edit);
    rows.append(row);
  });
  $("add-pick").hidden = g.picks.length >= 4;
  preview();
}
$("group-name").addEventListener("input", () => {
  group().name = $("group-name").value;
  renderTabs();
  changed();
});
$("remove-group").addEventListener("click", () => {
  if (state.groups.length > 1) {
    state.groups.splice(state.active, 1);
    state.active = Math.min(state.active, state.groups.length - 1);
    renderGroup();
    changed();
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
});
for (const id of ["wifi-ssid", "wifi-password"]) $(id).addEventListener("input", changed);
function fillDirections(value = -1) {
  const r = state.catalog.routes[Number($("route-select").value)];
  $("direction-select").replaceChildren(new Option("Either direction", "-1"));
  if (r) r.headsigns.forEach((name, i) => $("direction-select").append(new Option(name || `Direction ${i + 1}`, String(i))));
  $("direction-select").value = String(value);
  fillStops();
}
function fillStops(selected) {
  if (!state.catalog) return;
  const route = Number($("route-select").value), direction = Number($("direction-select").value);
  $("stop-select").replaceChildren();
  for (const [r, s, bits] of state.catalog.links) {
    if (r !== route || direction >= 0 && !(bits & 1 << direction)) continue;
    const stop = state.catalog.stops[s];
    $("stop-select").append(new Option(`${stop.name}${stop.code ? ` \xB7 ${stop.code}` : ""}`, String(s)));
  }
  if (selected !== void 0) $("stop-select").value = String(selected);
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
  $("route-select").replaceChildren(...state.catalog.routes.map((r, i) => new Option(r.line, String(i))));
  if (p) $("route-select").value = String(state.catalog.routes.findIndex((r) => r.id === p.route));
  fillDirections(p?.direction ?? -1);
  if (p) fillStops(state.catalog.stops.findIndex((s) => s.id === p.stop));
  $("walk-minutes").value = p?.walk ?? 6;
  $("walk-spread").value = p?.spread ?? 1;
  $("walk-margin").value = p?.margin ?? 1;
  $("pick-dialog").showModal();
}
$("route-select").addEventListener("change", () => fillDirections());
$("direction-select").addEventListener("change", () => fillStops());
$("pick-form").addEventListener("submit", (event) => {
  event.preventDefault();
  try {
    const r = state.catalog.routes[Number($("route-select").value)], s = state.catalog.stops[Number($("stop-select").value)], direction = Number($("direction-select").value);
    if (!r || !s || !$("stop-select").value) throw new Error("Choose a line and one of its stops.");
    const p = { route: r.id, line: r.line, stop: s.id, stop_name: s.name, stop_code: s.code, headsign: direction < 0 ? "Both directions" : r.headsigns[direction], direction, walk: Number($("walk-minutes").value), spread: Number($("walk-spread").value), margin: Number($("walk-margin").value) };
    const draft = structuredClone(state.groups);
    if (state.editing === null) draft[state.active].picks.push(p);
    else draft[state.active].picks[state.editing] = p;
    validateConfig({ wifi_ssid: "validation", wifi_password: "", automatic: state.automatic, groups: draft });
    state.groups = draft;
    $("pick-dialog").close();
    renderGroup();
    changed();
  } catch (e) {
    $("pick-error").textContent = e.message;
    $("pick-error").hidden = false;
  }
});
$("delete-pick").addEventListener("click", () => {
  if (state.editing !== null) group().picks.splice(state.editing, 1);
  $("pick-dialog").close();
  renderGroup();
  changed();
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
    if (!state.release || !state.firmwareUrl) throw new Error("The published firmware is not available yet.");
    const settings = { wifi_ssid: $("wifi-ssid").value, wifi_password: $("wifi-password").value, automatic: state.automatic, groups: state.groups };
    const image = encodeSettingsPartition(settings), settingsUrl = URL.createObjectURL(new Blob([image], { type: "application/octet-stream" }));
    state.urls.push(settingsUrl);
    const manifest = manifestWithSettings(state.release, state.firmwareUrl, settingsUrl), url = URL.createObjectURL(new Blob([JSON.stringify(manifest)], { type: "application/json" }));
    state.urls.push(url);
    $("installer").manifest = url;
    $("install-area").hidden = false;
    message("Ready. Choose Install on display, then select its USB port.");
    $("install-area").scrollIntoView({ behavior: "smooth", block: "nearest" });
  } catch (e) {
    message(e.message, true);
  }
});
async function loadCatalog() {
  const response = await fetch("./catalog.json", { credentials: "omit" });
  if (!response.ok) throw new Error("Could not load the Pozna\u0144 route catalogue.");
  const c = await response.json();
  if (!Array.isArray(c.routes) || !Array.isArray(c.stops) || !Array.isArray(c.links) || c.routes.length > 65535 || c.stops.length > 65535) throw new Error("The route catalogue is invalid.");
  state.catalog = c;
  $("catalog-status").textContent = `${c.routes.length} lines \xB7 ${c.stops.length.toLocaleString()} stops \xB7 ZTM Pozna\u0144 \xB7 catalogue ${c.date.slice(0, 4)}-${c.date.slice(4, 6)}-${c.date.slice(6)}`;
}
async function loadRelease() {
  const response = await fetch("./latest.json", { cache: "no-store", credentials: "omit" });
  if (!response.ok) throw new Error("The first firmware build hasn\u2019t been published yet. You can already explore the display and save your groups.");
  const candidate = await response.json();
  manifestWithSettings(candidate, "blob:firmware", "blob:settings");
  const binary = await fetch("./timetable-display.bin", { cache: "no-store", credentials: "omit" });
  if (!binary.ok) throw new Error("The firmware download failed. Reload to retry.");
  const bytes = await binary.arrayBuffer();
  if (bytes.byteLength !== candidate.size) throw new Error("The firmware size changed. Reload for the newest build.");
  const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((b) => b.toString(16).padStart(2, "0")).join("");
  if (digest !== candidate.sha256) throw new Error("The firmware checksum didn\u2019t match. Installation is disabled.");
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
}
window.addEventListener("pagehide", () => {
  forgetInstall();
  if (state.firmwareUrl) URL.revokeObjectURL(state.firmwareUrl);
  state.firmwareUrl = null;
  state.release = null;
  $("wifi-ssid").value = "";
  $("wifi-password").value = "";
  $("prepare").disabled = true;
  $("download").hidden = true;
  $("download").removeAttribute("href");
});
window.addEventListener("pageshow", (event) => {
  if (event.persisted) loadRelease().catch((e) => message(e.message, true));
});
renderGroup();
loadCatalog().catch((e) => {
  $("catalog-status").textContent = e.message;
  message(e.message, true);
});
loadRelease().catch((e) => {
  $("release-state").replaceChildren(element("i"), document.createTextNode("Preview ready \xB7 firmware pending"));
  $("release-state").classList.add("pending");
  message(e.message, true);
});
