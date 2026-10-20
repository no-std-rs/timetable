import { loadDraft, saveDraft, forgetDraft, REMEMBER_KEY } from "./settings-store.js";
import { SerialLogs } from "./serial-logs.js";
import { t, language, initLanguage, polish } from "./i18n.js";
import { encodeSettingsPartition, manifestWithSettings, validateConfig, walkingChance } from "./config.js";
import { catalogIndex } from "./catalog-search.js";
import { pathGraph } from "./route-topology.js";
initLanguage(document);
const $ = (id) => document.getElementById(id);
const element = (tag, text, className) => {
  const e = document.createElement(tag);
  if (text !== void 0) e.textContent = text;
  if (className) e.className = className;
  return e;
};
const emptyGroup = (name) => ({ name, days: 0, start: 360, end: 660, picks: [] });
const state = { automatic: true, groups: [emptyGroup(t("Morning")), emptyGroup(t("Evening"))], active: 0, editing: null, catalog: null, index: null, undo: null, release: null, releaseGeneration: 0, firmwareUrl: null, urls: [] };
let storeReady = false;
const group = () => state.groups[state.active];
const minute = (value) => {
  const [h, m] = value.split(":").map(Number);
  return h * 60 + m;
};
const hhmm = (value) => `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(value % 60).padStart(2, "0")}`;
const copies = new Map();
function copy(id, key, values = {}) {
  copies.set(id, { key, values });
  $(id).textContent = t(key, values);
}
const errorKey = (error, fallback) => Object.hasOwn(polish, error?.message) ? error.message : fallback;
let releaseText = "Checking firmware";
function releaseLabel(key) {
  releaseText = key;
  $("release-state").replaceChildren(element("i"), document.createTextNode(t(key)));
}
function message(text, error = false) {
  copy("status", text);
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
  persistSettings();
  if (state.release) message("Changes applied. Prepare when you’re ready.");
}
function removalSnapshot() {
  return { groups: structuredClone(state.groups), active: state.active, automatic: state.automatic };
}
function offerUndo(snapshot, text) {
  state.undo = snapshot;
  copy("undo-message", text);
  $("undo-area").hidden = false;
}
$("undo-removal").addEventListener("click", () => {
  if (!state.undo) return;
  state.groups = state.undo.groups;
  state.active = state.undo.active;
  state.automatic = state.undo.automatic;
  renderGroup();
  changed();
  $("group-name").focus();
});
function preview() {
  const g = group(), p = g.picks[0];
  $("preview-group").textContent = g.name || t("Your group");
  $("preview-journey").hidden = !p;
  $("preview-empty").hidden = !!p;
  $("preview-freshness").hidden = !p;
  $("preview-mode").textContent = p ? `09:41 / ${state.automatic && g.days ? t("SCHEDULED") : t("MANUAL")}` : t("YOUR GROUP / NO STOPS");
  if (!p) return;
  $("preview-line").textContent = p.line;
  $("preview-direction").textContent = p.headsign || t("Either direction");
  $("preview-stop").textContent = p.stop_name;
  const left = 12 - p.walk - p.margin;
  const chance = walkingChance(12, p.walk, p.spread, p.margin);
  $("preview-leave").textContent = chance === 0 ? t("Not enough walk time") : left > 0 ? t("Leave in {minutes} min", { minutes: left }) : t("Leave now");
  $("preview-leave").classList.toggle("urgent", left <= 0);
  $("preview-walk").textContent = t("{minutes}m walk · {chance}% walk est.", { minutes: p.walk, chance });
  const next = g.picks[1] ?? p;
  $("preview-next-line").textContent = `${next.line} \xB7 ${next.headsign || t("Either direction")}`;
  $("preview-last-line").textContent = `${p.line} \xB7 ${p.headsign || t("Either direction")}`;
  $("preview-third-line").textContent = `${next.line} \xB7 ${next.headsign || t("Either direction")}`;
  $("preview-fourth-line").textContent = `${p.line} \xB7 ${p.headsign || t("Either direction")}`;
  $("preview-fifth-line").textContent = `${next.line} \xB7 ${next.headsign || t("Either direction")}`;
}
function renderTabs() {
  const tabs = $("group-tabs");
  tabs.replaceChildren();
  state.groups.forEach((g, i) => {
    const b = element("button", g.name || t("Your group"), "group-tab");
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
  state.groups.push(emptyGroup(t("Group {number}", { number: state.groups.length + 1 })));
  state.active = state.groups.length - 1;
  renderGroup();
  changed();
  $("group-name").focus();
});
function groupNameError(value) {
  if (!value.trim()) return t("Give this group a name.");
  if (new TextEncoder().encode(value).length > 24) return t("Shorten this name so it fits on the display.");
  return /\p{Cc}/u.test(value) ? t("Use a name without invisible characters.") : "";
}
function validateGroupName() {
  const name = $("group-name");
  name.setCustomValidity(groupNameError(name.value));
}
function renderGroup(preserveInputs = false) {
  renderTabs();
  $("automatic").checked = state.automatic;
  $("automatic-hint").textContent = state.automatic ? t("Switch by time of day. Turn off to choose groups by touch.") : t("Schedules are paused. Choose groups by touch; saved time windows stay here.");
  const g = group();
  if (!preserveInputs) $("group-name").value = g.name;
  validateGroupName();
  $("remove-group").hidden = state.groups.length < 2;
  $("scheduled").checked = g.days !== 0;
  $("schedule").hidden = g.days === 0;
  if (!preserveInputs) {
    $("start-time").value = hhmm(g.start);
    $("end-time").value = hhmm(g.end);
  }
  $("start-time").disabled = $("end-time").disabled = g.days === 0;
  $("days").replaceChildren();
  (language() === "pl" ? ["Pn", "Wt", "Śr", "Cz", "Pt", "So", "Nd"] : ["M", "T", "W", "T", "F", "S", "S"]).forEach((name, i) => {
    const b = element("button", name, "day-button");
    b.type = "button";
    b.setAttribute("aria-label", [t("Monday"), t("Tuesday"), t("Wednesday"), t("Thursday"), t("Friday"), t("Saturday"), t("Sunday")][i]);
    b.setAttribute("aria-pressed", String((g.days & 1 << i) !== 0));
    b.addEventListener("click", () => {
      g.days ^= 1 << i;
      renderGroup();
      changed();
      (g.days ? $("days").children[i] : $("scheduled")).focus();
    });
    $("days").append(b);
  });
  const rows = $("picks");
  rows.replaceChildren();
  if (g.picks.length === 0) rows.append(element("p", t("Start with a line and the stop you walk to. You can change it on the display later."), "empty-picks"));
  g.picks.forEach((p, i) => {
    const row = element("div", void 0, "pick-row"), info = element("div", void 0, "pick-info");
    info.append(element("strong", p.headsign || t("Either direction")), element("small", p.stop_name));
    const edit = element("button", t("Edit"), "pick-edit");
    edit.type = "button";
    edit.disabled = !state.catalog;
    edit.setAttribute("aria-label", t("Edit {line} at {stop}", { line: p.line, stop: p.stop_name }));
    edit.addEventListener("click", () => openPick(i));
    row.append(element("span", p.line, "pick-line"), info, element("span", t("{minutes}m walk", { minutes: p.walk }), "pick-walk"), edit);
    rows.append(row);
  });
  $("add-pick").hidden = g.picks.length >= 4;
  $("add-pick").disabled = !state.catalog;
  preview();
  persistSettings();
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
$("automatic").addEventListener("change", () => {
  state.automatic = $("automatic").checked;
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
  $("show-password").textContent = show ? t("Hide") : t("Show");
  $("show-password").setAttribute("aria-label", show ? t("Hide password") : t("Show password"));
  $("show-password").setAttribute("aria-pressed", String(show));
});
for (const id of ["wifi-ssid", "wifi-password", "battery-mode"]) $(id).addEventListener("input", changed);
const picker = { group: null, stop: null, direction: -1, choice: "", choices: [], preferredStop: null, matches: [], active: -1, path: null };
const stopLabel = (i) => state.index.groups[i].name;
function closeStops() {
  $("stop-options").hidden = true;
  $("stop-search").setAttribute("aria-expanded", "false");
  $("stop-search").removeAttribute("aria-activedescendant");
  picker.active = -1;
}
function fillRoutes(selected = $("route-select").value) {
  const matches = state.index.groupRoutes($("route-search").value, picker.group ?? -1);
  $("route-select").replaceChildren(new Option(matches.length ? (picker.group === null ? t("Choose a line (or find a stop first)") : t("Choose a line at this stop")) : t("No matching lines"), ""));
  for (const i of matches) $("route-select").append(new Option(state.catalog.routes[i].line, String(i)));
  $("route-select").value = selected !== "" && matches.includes(Number(selected)) ? selected : "";
  $("route-results").textContent = matches.length ? t(matches.length === 1 ? "{count} line" : "{count} lines", { count: matches.length }) + (picker.group === null ? t(" · choose here to search its stops") : t(" serving {stop}", { stop: stopLabel(picker.group) })) : picker.group === null ? t("No lines match. Try another line number or destination.") : t("No lines match. Clear the line search to see all lines serving your stop.");
  fillDirections();
}
function choiceName(route, choice) {
  if (choice.all) return t("All departures");
  return state.index.topology.contexts(choice.stop, route, choice.direction)[0]?.headsign || state.catalog.routes[route]?.headsigns[choice.direction] || t("Destination not provided");
}
function fillDirections() {
  const selected = $("route-select").value, route = selected === "" ? -1 : Number(selected);
  picker.choices = state.index.boardingChoices(route, picker.group);
  // Preserve a saved broad query at its exact platform when editing old settings.
  if (picker.stop !== null && picker.direction < 0 && picker.choices.some(c => c.stop === picker.stop)
      && !picker.choices.some(c => c.key === picker.choice)) picker.choices.push({ stop: picker.stop, direction: -1, all: true, key: picker.choice });
  const placeholder = route < 0 ? t("Choose a line first") : picker.group === null ? t("Choose a stop first") : t("Choose a destination and platform");
  $("direction-select").replaceChildren(new Option(placeholder, ""));
  for (const choice of picker.choices) {
    const stop = state.catalog.stops[choice.stop];
    $("direction-select").append(new Option(`${choiceName(route, choice)} · ${stop.code || stop.id}`, choice.key));
  }
  let choice = picker.choices.find(c => c.key === picker.choice);
  if (!choice && picker.preferredStop !== null) {
    const preferred = picker.choices.filter(c => c.stop === picker.preferredStop);
    if (preferred.length === 1) choice = preferred[0];
  }
  if (!choice && picker.choices.length === 1) choice = picker.choices[0];
  picker.stop = choice?.stop ?? null;
  picker.direction = choice?.direction ?? -1;
  picker.choice = choice?.key ?? "";
  $("direction-select").value = picker.choice;
  $("direction-select").disabled = !picker.choices.length;
  closeStops();
  updateStopHint();
  updatePickAvailability();
  renderRouteMap();
}
function directionName(route, direction) {
  return state.index.topology.contexts(picker.stop, route, direction)[0]?.headsign || state.catalog.routes[route]?.headsigns[direction] || "";
}
function stopContext(stop, route = -1, direction = -1) {
  const contexts = state.index.topology.contexts(stop, route, direction);
  return contexts.slice(0, 2).map(c => `${route < 0 ? `${state.catalog.routes[c.route].line} ` : ""}→ ${c.headsign}${c.next !== undefined ? t(" · next {stop}", { stop: state.index.topology.stops[c.next].name }) : t(" · last stop")}`).join(" / ")
    + (contexts.length > 2 ? t(" · +{count} other paths", { count: contexts.length - 2 }) : "");
}
function updateStopHint() {
  $("stop-results").textContent = picker.group === null
    ? ($("route-select").value === "" ? t("Search any stop, then browse its lines. Polish accents are optional.") : t("Search this line’s stops. Same-name platforms are grouped together."))
    : t("{stop} · {count} platforms. Choose your line and destination.", { stop: stopLabel(picker.group), count: state.index.groups[picker.group].stops.length });
  $("platform-hint").hidden = picker.group === null;
  const stop = state.catalog.stops[picker.stop];
  $("platform-hint").textContent = stop
    ? t("Board at {platform}. {context}", { platform: stop.code || stop.id, context: stopContext(picker.stop, Number($("route-select").value), picker.direction) })
    : t("Choose a destination to find the right boarding platform.");
}
function fillStops() {
  const value = $("route-select").value, route = value === "" ? -1 : Number(value);
  const query = picker.group === null ? $("stop-search").value : "";
  picker.matches = route < 0 && !query.trim() ? [] : state.index.stopGroups(route, query);
  picker.active = -1;
  $("stop-options").replaceChildren();
  for (const i of picker.matches) {
    const group = state.index.groups[i], option = element("div", void 0, "stop-option");
    option.id = `stop-group-${i}`;
    option.dataset.group = String(i);
    option.setAttribute("role", "option");
    option.setAttribute("aria-selected", "false");
    const copy = element("span", undefined, "stop-copy");
    const contexts = [...new Set(group.stops.map(stop => stopContext(stop, route)).filter(Boolean))];
    copy.append(element("span", group.name));
    if (contexts.length) copy.append(element("small", contexts.slice(0, 2).join(" / "), "stop-context"));
    option.append(copy, element("small", t(group.stops.length === 1 ? "{count} platform" : "{count} platforms", { count: group.stops.length }), "stop-code"));
    option.addEventListener("mousedown", event => event.preventDefault());
    option.addEventListener("click", () => selectGroup(i));
    $("stop-options").append(option);
  }
  const open = picker.matches.length > 0;
  $("stop-options").hidden = !open;
  $("stop-search").setAttribute("aria-expanded", String(open));
  $("stop-search").removeAttribute("aria-activedescendant");
  $("stop-results").textContent = open ? t("{count} stops · platforms with the same name are grouped", { count: picker.matches.length }) : query.trim() ? t("No boarding stops match. Try part of a name or a platform code.") : t("Type a stop name or platform code. Polish accents are optional.");
}
function selectGroup(group, exactStop = null, direction = -1) {
  const query = $("stop-search").value;
  const exact = state.index.groups[group].stops.filter(i => state.catalog.stops[i].code.toLowerCase() === query.trim().toLowerCase());
  picker.group = group;
  picker.stop = exactStop;
  picker.direction = direction;
  picker.choice = exactStop === null ? "" : `${exactStop}:${direction}`;
  picker.preferredStop = exactStop ?? (exact.length === 1 ? exact[0] : null);
  picker.path = null;
  $("stop-search").value = stopLabel(group);
  $("route-search").value = "";
  closeStops();
  fillRoutes();
  $("stop-search").focus({ preventScroll: true });
  closeStops();
  updateStopHint();
}
function updatePickAvailability() {
  $("save-pick").disabled = $("route-select").value === "" || picker.stop === null || !picker.choice;
}
function renderRouteMap() {
  const value = $("route-select").value, route = value === "" ? -1 : Number(value), direction = picker.direction;
  const topology = state.index.topology, ids = topology.paths(route, direction);
  $("route-map").hidden = !ids.length;
  if (!ids.length) { picker.path = null; return; }
  if (!ids.includes(picker.path)) {
    picker.path = ids.find(id => topology.patterns[id].calls.some(c => c[0] === picker.stop && c[1] !== 1)) ?? ids[0];
  }
  const pathLabel = id => {
    const p = topology.patterns[id], base = topology.patterns[ids.find(other => topology.patterns[other].direction === p.direction)];
    const split = p.calls.findIndex((call, i) => call[0] !== base.calls[i]?.[0]);
    const via = p.calls[split >= 0 ? split : 0];
    return t("{headsign} · via {stop} · {count} stops", { headsign: p.headsign || topology.stops[p.calls.at(-1)[0]].name, stop: topology.stops[via[0]].name, count: p.calls.length });
  };
  $("path-select").replaceChildren(...ids.map(id => new Option(pathLabel(id), String(id))));
  $("path-select").value = String(picker.path);
  $("path-select").disabled = ids.length === 1;
  $("route-map-title").textContent = t("Follow line {line}", { line: state.catalog.routes[route].line });
  $("route-map-count").textContent = t(ids.length === 1 ? "{count} path in this snapshot" : "{count} paths in this snapshot", { count: ids.length });
  const visibleIds = ids.filter(id => topology.patterns[id].direction === topology.patterns[picker.path].direction);
  const graph = pathGraph(topology.patterns, visibleIds), svg = $("route-graph");
  svg.replaceChildren();
  const draw = (tag, attributes) => {
    const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, String(value));
    svg.append(node);
    return node;
  };
  // The detailed button list below provides names and full-size keyboard/touch
  // targets; the overview preserves all forks without crowding it with labels.
  for (const id of [...visibleIds.filter(id => id !== picker.path), picker.path]) {
    const points = graph.paths.get(id).map(n => `${graph.nodes[n].x},${graph.nodes[n].y}`).join(" ");
    const line = draw("polyline", { points, class: id === picker.path ? "graph-path active" : "graph-path" });
    line.addEventListener("click", () => { picker.path = id; renderRouteMap(); });
  }
  graph.nodes.forEach((node, n) => {
    const active = graph.paths.get(picker.path).includes(n), selected = node.call[0] === picker.stop;
    const dot = draw("circle", { cx: node.x, cy: node.y, r: selected ? 5 : active ? 3.5 : 2.5, "data-stop": node.call[0], class: `graph-stop${active ? " active" : ""}${selected ? " selected" : ""}${node.call[1] === 1 ? " no-pickup" : ""}` });
    const title = document.createElementNS(svg.namespaceURI, "title");
    title.textContent = `${topology.stops[node.call[0]].name} · ${topology.stops[node.call[0]].code}${node.call[1] === 1 ? t(" · no boarding") : ""}`;
    dot.append(title);
    if (node.call[1] !== 1) dot.addEventListener("click", () => {
      const id = active ? picker.path : visibleIds.find(id => graph.paths.get(id).includes(n));
      chooseRouteCall(id, graph.paths.get(id).indexOf(n));
    });
  });
  const p = topology.patterns[picker.path], list = $("route-stops");
  list.replaceChildren();
  p.calls.forEach((call, at) => {
    const stop = topology.stops[call[0]], row = element("li"), button = element("button", undefined, "route-stop");
    button.type = "button";
    button.dataset.stop = String(call[0]);
    button.dataset.call = String(at);
    button.disabled = call[1] === 1;
    button.setAttribute("aria-pressed", String(call[0] === picker.stop));
    const title = element("span", undefined, "route-stop-name");
    title.append(element("span", stop.name), element("small", stop.code));
    const next = p.calls[at + 1];
    const note = call[1] === 1 ? t("No boarding here") : call[1] === 2 ? t("Call to arrange boarding") : call[1] === 3 ? t("Arrange boarding with driver") : next ? t("Next: {stop}", { stop: topology.stops[next[0]].name }) : t("Last stop");
    button.append(title, element("small", note, "route-stop-note"));
    button.addEventListener("click", () => chooseRouteCall(picker.path, at));
    row.append(button);
    list.append(row);
  });
}
function chooseRouteCall(id, at) {
  const p = state.index.topology.patterns[id], call = p.calls[at];
  if (call[1] === 1) return;
  selectGroup(state.index.groupOf[call[0]], call[0], p.direction);
  picker.path = id;
  renderRouteMap();
}
$("path-select").addEventListener("change", () => {
  picker.path = Number($("path-select").value);
  // A path is a preview, not an additional filter on saved timetable queries.
  renderRouteMap();
});
function openPick(index) {
  state.editing = index;
  picker.path = null;
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
  picker.stop = stop >= 0 && state.index.stops(route, p?.direction ?? -1).includes(stop) ? stop : null;
  picker.group = picker.stop === null ? null : state.index.groupOf[picker.stop];
  picker.direction = p?.direction ?? -1;
  picker.choice = picker.stop === null ? "" : `${picker.stop}:${picker.direction}`;
  picker.preferredStop = picker.stop;
  if (picker.group !== null) $("stop-search").value = stopLabel(picker.group);
  fillRoutes(route < 0 ? "" : String(route));
  if (p && (route < 0 || picker.stop === null)) {
    copy("pick-error", "This saved choice needs an updated line or stop. Choose it from the current catalogue.");
    $("pick-error").hidden = false;
  }
  $("walk-minutes").value = p?.walk ?? 6;
  $("walk-spread").value = p?.spread ?? 1;
  $("walk-margin").value = p?.margin ?? 1;
  $("pick-dialog").showModal();
  $("stop-search").focus({ preventScroll: true });
  closeStops();
  updateStopHint();
}
$("route-search").addEventListener("input", () => fillRoutes());
$("route-search").addEventListener("keydown", (event) => {
  if (event.key !== "Enter") return;
  event.preventDefault();
  $("route-select").focus();
});
$("stop-search").addEventListener("input", () => {
  picker.group = picker.stop = picker.preferredStop = null;
  picker.choice = "";
  picker.direction = -1;
  fillRoutes();
  fillStops();
});
$("stop-search").addEventListener("focus", fillStops);
$("stop-search").addEventListener("blur", () => {
  closeStops();
  if (picker.stop !== null) updateStopHint();
});
$("stop-search").addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    if ($("stop-options").hidden) $("pick-dialog").close();
    else closeStops();
  } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if ($("stop-options").hidden) fillStops();
    if (!picker.matches.length) return;
    const step = event.key === "ArrowDown" ? 1 : -1;
    picker.active = picker.active < 0 ? (step > 0 ? 0 : picker.matches.length - 1) : (picker.active + step + picker.matches.length) % picker.matches.length;
    for (const [i, option] of [...$("stop-options").children].entries()) option.setAttribute("aria-selected", String(i === picker.active));
    const option = $("stop-options").children[picker.active];
    $("stop-search").setAttribute("aria-activedescendant", option.id);
    option.scrollIntoView({ block: "nearest", behavior: "instant" });
  } else if (event.key === "Enter") {
    event.preventDefault();
    if (!$("stop-options").hidden && picker.active >= 0) selectGroup(picker.matches[picker.active]);
    else if (picker.group !== null) $("route-select").focus();
  }
});
$("route-select").addEventListener("change", () => {
  picker.choice = "";
  picker.stop = null;
  picker.direction = -1;
  fillDirections();
});
$("direction-select").addEventListener("change", () => {
  const choice = picker.choices.find(c => c.key === $("direction-select").value);
  picker.choice = choice?.key ?? "";
  picker.stop = choice?.stop ?? null;
  picker.direction = choice?.direction ?? -1;
  picker.path = null;
  updatePickAvailability();
  closeStops();
  updateStopHint();
  renderRouteMap();
});
$("pick-form").addEventListener("submit", (event) => {
  event.preventDefault();
  try {
    const route = Number($("route-select").value), stop = picker.stop;
    const r = state.catalog.routes[route], s = state.catalog.stops[stop], direction = picker.direction;
    if (!r || !s || !$("route-select").value || stop === null || !state.index.stops(route, direction).includes(stop)) throw new Error("Choose a line and one of its boarding stops.");
    const p = { route: r.id, line: r.line, stop: s.id, stop_name: s.name, stop_code: s.code, headsign: picker.choices.find(c => c.key === picker.choice)?.all ? "" : directionName(route, direction), direction, walk: Number($("walk-minutes").value), spread: Number($("walk-spread").value), margin: Number($("walk-margin").value) };
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
    copy("pick-error", errorKey(e, "Choose a line and one of its boarding stops."));
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
    message(errorKey(e, "Check the group name (up to 24 UTF-8 bytes), schedule and four-choice limit."), true);
  }
});
$("import-button").addEventListener("click", () => $("import-groups").click());
$("import-groups").addEventListener("change", async (event) => {
  try {
    const f = event.target.files[0];
    if (!f) return;
    if (f.size > 16384) throw new Error("That group file is too large.");
    const data = JSON.parse(await f.text());
    if (data.version !== 1) throw new Error("Unsupported group file version.");
    validateConfig({ wifi_ssid: "validation", wifi_password: "", automatic: data.automatic, groups: data.groups });
    const snapshot = removalSnapshot();
    state.groups = data.groups;
    state.automatic = data.automatic;
    state.active = 0;
    renderGroup();
    changed();
    offerUndo(snapshot, "Groups imported.");
    $("group-name").focus();
  } catch (e) {
    message(e instanceof SyntaxError ? "That group file is not valid JSON." : errorKey(e, "Could not read that group file."), true);
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
    const settings = { wifi_ssid: $("wifi-ssid").value, wifi_password: $("wifi-password").value, automatic: state.automatic, battery: $("battery-mode").checked, groups: state.groups };
    const image = encodeSettingsPartition(settings), settingsUrl = URL.createObjectURL(new Blob([image], { type: "application/octet-stream" }));
    state.urls.push(settingsUrl);
    const manifest = manifestWithSettings(state.release, state.firmwareUrl, settingsUrl), url = URL.createObjectURL(new Blob([JSON.stringify(manifest)], { type: "application/json" }));
    state.urls.push(url);
    $("installer").manifest = url;
    $("install-area").hidden = false;
    message("Ready. Choose Install on display, then select its USB port.");
    $("install-area").scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth", block: "nearest" });
  } catch (e) {
    message(errorKey(e, "Check the group name (up to 24 UTF-8 bytes), schedule and four-choice limit."), true);
  }
});
async function loadCatalog() {
  $("retry-catalog").hidden = true;
  $("retry-catalog").disabled = true;
  $("catalog-feedback").hidden = false;
  copy("catalog-feedback", "Loading line and stop choices…");
  try {
    const response = await fetch("./catalog.json", { credentials: "omit" });
    if (!response.ok) throw new Error("Could not load the Poznań route choices. Try again when your connection is back.");
    const c = await response.json();
    if (!c || !Array.isArray(c.routes) || !Array.isArray(c.stops) || !Array.isArray(c.links) || c.routes.length > 65535 || c.stops.length > 65535 || typeof c.date !== "string" || !/^\d{8}$/.test(c.date)) throw new Error("The route catalogue is invalid. Try downloading it again.");
    const index = catalogIndex(c);
    state.catalog = c;
    state.index = index;
    copy("catalog-status", "{routes} lines · {stops} stops · ZTM Poznań · catalogue {date}", { routes: c.routes.length, stops: c.stops.length, date: `${c.date.slice(0, 4)}-${c.date.slice(4, 6)}-${c.date.slice(6)}` });
    $("catalog-feedback").hidden = true;
    $("add-pick").disabled = false;
    document.querySelectorAll(".pick-edit").forEach((button) => { button.disabled = false; });
  } catch (e) {
    copy("catalog-feedback", errorKey(e, "Could not load the Poznań route choices. Try again when your connection is back."));
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
  releaseLabel("Checking firmware");
  message("Checking the latest firmware…");
  try {
    const response = await fetch("./latest.json", { cache: "no-store", credentials: "omit" });
    if (!response.ok) throw new Error("The firmware build could not be loaded. Keep editing your groups and retry below.");
    const candidate = await response.json();
    if (generation !== state.releaseGeneration) return;
    manifestWithSettings(candidate, "blob:firmware", "blob:settings");
    releaseLabel("Downloading firmware");
    message("Downloading the display app. Your groups stay editable.");
    const binary = await fetch("./timetable-display.bin", { cache: "no-store", credentials: "omit" });
    if (!binary.ok) throw new Error("The firmware download failed. Try the download again.");
    const bytes = await binary.arrayBuffer();
    if (generation !== state.releaseGeneration) return;
    if (bytes.byteLength !== candidate.size) throw new Error("The firmware changed during the download. Retry for the newest build.");
    releaseLabel("Verifying firmware");
    const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((b) => b.toString(16).padStart(2, "0")).join("");
    if (generation !== state.releaseGeneration) return;
    if (digest !== candidate.sha256) throw new Error("The firmware checksum didn’t match. Retry the download before installing.");
    if (state.firmwareUrl) URL.revokeObjectURL(state.firmwareUrl);
    state.firmwareUrl = URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" }));
    state.release = candidate;
    releaseLabel("Firmware verified");
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
    releaseLabel("Firmware unavailable");
    $("release-state").classList.add("pending");
    $("retry-release").hidden = false;
    $("retry-release").disabled = false;
    message(errorKey(e, "The firmware download failed. Try the download again."), true);
  }
}
$("retry-catalog").addEventListener("click", loadCatalog);
$("retry-release").addEventListener("click", loadRelease);
window.addEventListener("pagehide", () => {
  persistSettings();
  void logs.disconnect();
  ++state.releaseGeneration;
  forgetInstall();
  if (state.firmwareUrl) URL.revokeObjectURL(state.firmwareUrl);
  state.firmwareUrl = null;
  state.release = null;
  if (!$("remember-settings").checked) {
    $("wifi-ssid").value = "";
    $("wifi-password").value = "";
  }
  $("wifi-password").type = "password";
  $("show-password").textContent = t("Show");
  $("show-password").setAttribute("aria-label", t("Show password"));
  $("show-password").setAttribute("aria-pressed", "false");
  $("prepare").disabled = true;
  $("download").hidden = true;
  $("download").removeAttribute("href");
});
window.addEventListener("pageshow", (event) => {
  if (event.persisted) loadRelease();
});
function restoreSettings() {
  try {
    $("remember-settings").checked = localStorage.getItem(REMEMBER_KEY) !== "false";
    const draft = $("remember-settings").checked ? loadDraft(localStorage) : null;
    if (!draft) return;
    state.groups = draft.config.groups;
    state.automatic = draft.config.automatic;
    state.active = draft.active;
    $("wifi-ssid").value = draft.config.wifi_ssid;
    $("wifi-password").value = draft.config.wifi_password;
    $("battery-mode").checked = draft.config.battery ?? false;
  } catch {
    copy("storage-status", "Saved settings could not be read. Your current settings are unchanged.");
  }
}
function persistSettings() {
  if (!storeReady) return;
  try {
    const remember = $("remember-settings").checked;
    if (remember) {
      localStorage.setItem(REMEMBER_KEY, "true");
      saveDraft(localStorage, { version: 1, active: state.active, config: {
      wifi_ssid: $("wifi-ssid").value, wifi_password: $("wifi-password").value,
      automatic: state.automatic, battery: $("battery-mode").checked, groups: state.groups,
      } });
    } else forgetDraft(localStorage);
    copy("storage-status", remember ? "Settings remembered in this browser." : "Settings stay only in this tab.");
  } catch {
    copy("storage-status", "This browser could not save settings. Keep this tab open or export your groups.");
  }
}
$("remember-settings").addEventListener("change", persistSettings);
$("forget-settings").addEventListener("click", () => {
  $("remember-settings").checked = false;
  persistSettings();
});
function renderLogs() {
  const supported = !!navigator.serial && window.isSecureContext;
  const busy = ["connecting", "connected", "disconnecting"].includes(logs.state);
  $("connect-logs").disabled = !supported || busy;
  $("disconnect-logs").disabled = !busy || logs.state === "disconnecting";
  $("install-action").disabled = busy;
  $("save-logs").disabled = !logs.text;
  const output = $("log-output"), follow = output.scrollTop + output.clientHeight >= output.scrollHeight - 24;
  output.textContent = logs.text;
  if (follow) output.scrollTop = output.scrollHeight;
  copy("logs-status", !supported ? "USB logs need desktop Chrome or Edge over HTTPS or localhost."
    : logs.state === "connecting" ? "Choose the display’s USB port."
    : logs.state === "connected" ? "Reading logs at 115200 baud. Disconnect logs before installing."
    : logs.state === "disconnecting" ? "Closing USB port…"
    : logs.state === "error" ? "Could not read USB logs. Close other serial monitors or the flasher and reconnect."
    : logs.state === "disconnected" ? "USB disconnected. Reconnect to continue reading." : "Not connected");
}
const logs = new SerialLogs(navigator.serial, renderLogs);
$("connect-logs").addEventListener("click", () => void logs.connect());
$("disconnect-logs").addEventListener("click", () => void logs.disconnect());
$("clear-logs").addEventListener("click", () => logs.clear());
$("save-logs").addEventListener("click", () => {
  const url = URL.createObjectURL(new Blob([logs.text], { type: "text/plain;charset=utf-8" }));
  const link = element("a");
  link.href = url;
  link.download = "timetable-device-logs.txt";
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
});
document.addEventListener("languagechange", () => {
  renderGroup(true);
  for (const [id, { key, values }] of copies) $(id).textContent = t(key, values);
  releaseLabel(releaseText);
  renderLogs();
  const shown = $("wifi-password").type === "text";
  $("show-password").textContent = t(shown ? "Hide" : "Show");
  $("show-password").setAttribute("aria-label", t(shown ? "Hide password" : "Show password"));
  if (state.catalog && $("pick-dialog").open) {
    const open = !$("stop-options").hidden;
    fillRoutes();
    if (open) fillStops();
  }
});
restoreSettings();
storeReady = true;
renderGroup();
renderLogs();
loadCatalog();
loadRelease();
