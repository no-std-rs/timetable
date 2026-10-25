import { catalogIndex } from "../catalog-search.js";
import { test, expect } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
const catalog = JSON.parse(await readFile(new URL("../../catalog/catalog.json", import.meta.url), "utf8"));
const index = catalogIndex(catalog);
const saved = JSON.parse(await readFile(new URL("../../display/tests/settings.json", import.meta.url), "utf8"));
const firmware = Buffer.alloc(1e5, 233);
const release = { chip: "ESP32-C3", board: "GUITION ESP32-2424S012-V1.0", flash_size: 4194304, config_offset: 4128768, config_version: 2, revision: "a".repeat(40), sha256: createHash("sha256").update(firmware).digest("hex"), size: firmware.length };
async function prepare(page, { corrupt = false, catalogFailures = 0, releaseFailures = 0 } = {}) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "serial", { value: { getPorts: async () => [] }, configurable: true });
  });
  await page.route("**/latest.json", (r) => releaseFailures-- > 0 ? r.fulfill({ status: 503 }) : r.fulfill({ json: release }));
  if (catalogFailures) await page.route("**/catalog.json", (r) => catalogFailures-- > 0 ? r.fulfill({ status: 503 }) : r.fulfill({ json: catalog }));
  await page.route("**/timetable-display.bin", (r) => r.fulfill({ body: corrupt ? Buffer.alloc(1e5) : firmware, contentType: "application/octet-stream" }));
  await page.goto("/");
}
async function chooseStop(page, route, direction = 0) {
  const [, stop] = catalog.links.find(([r, , bits]) => r === route && bits & 1 << direction);
  await page.locator("#stop-search").fill(catalog.stops[stop].code || catalog.stops[stop].name);
  await page.locator(`#stop-options [data-group="${index.groupOf[stop]}"]`).click();
  await page.locator("#direction-select").selectOption(`${stop}:${direction}`);
  return stop;
}
test("groups, directions and walking times update a clean responsive preview", async ({ page }, info) => {
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#add-pick").click();
  await expect(page.locator("#pick-dialog")).toBeVisible();
  const route = catalog.routes.findIndex((r) => r.line === "4");
  await page.locator("#route-select").selectOption({ value: String(route) });
  await chooseStop(page, route);
  await expect(page.locator("#route-select")).toHaveValue(String(route));
  expect(await page.locator("#route-select option:checked").textContent()).toBe("4");
  await page.locator("#walk-minutes").fill("9");
  await page.getByRole("button", { name: "Save choice" }).click();
  await expect(page.locator("#preview-line")).toHaveText("4");
  await expect(page.locator("#preview-leave")).toHaveText("Leave in 2 min");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "board");
  await page.locator("#group-name").fill("Work / morning");
  await expect(page.locator("#preview-group")).toHaveText("Work / morning");
  await page.locator("#scheduled").check();
  await expect(page.locator("#schedule")).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
  await page.screenshot({ path: `/tmp/timetable-installer-${info.project.name}.png`, fullPage: true });
});
test("USB settings contain selected groups; credentials never enter requests", async ({ page }) => {
  const requests = [];
  page.on("request", (r) => requests.push(r.url() + " " + (r.postData() ?? "")));
  await prepare(page);
  await page.locator("#wifi-ssid").fill("private-ssid-test");
  await page.locator("#wifi-password").fill("private-password-test");
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
  const data = await page.evaluate(async () => {
    const manifest = await (await fetch(document.querySelector("#installer").manifest)).json();
    const bytes = new Uint8Array(await (await fetch(manifest.builds[0].parts[1].path)).arrayBuffer());
    const view = new DataView(bytes.buffer);
    return { magic: new TextDecoder().decode(bytes.subarray(0, 8)), length: bytes.length, settings: JSON.parse(new TextDecoder().decode(bytes.subarray(24, 24 + view.getUint32(12, true)))) };
  });
  expect(data.magic).toBe("TTABLE01");
  expect(data.length).toBe(65536);
  expect(data.settings.wifi_password).toBe("private-password-test");
  expect(data.settings.groups).toHaveLength(2);
  expect(requests.filter((r) => !r.startsWith("blob:")).join("\n")).not.toContain("private-ssid-test");
  expect(requests.filter((r) => !r.startsWith("blob:")).join("\n")).not.toContain("private-password-test");
  await page.locator("#group-name").fill("Changed");
  await expect(page.locator("#install-area")).toBeHidden();
});
test("corrupt firmware is never offered for installation", async ({ page }) => {
  await prepare(page, { corrupt: true });
  await expect(page.locator("#status")).toContainText("checksum");
  await expect(page.locator("#prepare")).toBeDisabled();
  await expect(page.locator("#download")).toBeHidden();
  await page.route("**/timetable-display.bin", (r) => r.fulfill({ body: firmware, contentType: "application/octet-stream" }));
  await page.locator("#retry-release").click();
  await expect(page.locator("#prepare")).toBeEnabled();
});
test("history restoration preserves group controls and USB settings while clearing credentials", async ({ page }) => {
  await prepare(page);
  await page.locator("#remember-settings").uncheck();
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#group-name").fill("Commute");
  await page.locator("#scheduled").check();
  await page.locator("#start-time").fill("09:15");
  await page.locator("#end-time").fill("12:30");
  await page.locator("#wifi-ssid").fill("before-history-ssid");
  await page.locator("#wifi-password").fill("before-history-password");
  await page.locator("#show-password").click();
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
  // Dispatch the persisted lifecycle directly: browser automation can disable BFCache.
  await page.evaluate(() => {
    window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: true }));
    window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }));
  });
  await expect(page.locator("#prepare")).toBeEnabled();
  await expect(page.locator("#install-area")).toBeHidden();
  await expect(page.locator("#wifi-ssid")).toHaveValue("");
  await expect(page.locator("#wifi-password")).toHaveValue("");
  await expect(page.locator("#wifi-password")).toHaveAttribute("type", "password");
  await expect(page.locator("#show-password")).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator("#group-name")).toHaveValue("Commute");
  await expect(page.locator("#preview-group")).toHaveText("Commute");
  await expect(page.locator("#scheduled")).toBeChecked();
  await expect(page.locator("#schedule")).toBeVisible();
  await expect(page.locator("#start-time")).toHaveValue("09:15");
  await expect(page.locator("#end-time")).toHaveValue("12:30");
  await page.locator("#wifi-ssid").fill("after-history-ssid");
  await page.locator("#wifi-password").fill("after-history-password");
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
  const settings = await page.evaluate(async () => {
    const manifest = await (await fetch(document.querySelector("#installer").manifest)).json();
    const bytes = new Uint8Array(await (await fetch(manifest.builds[0].parts[1].path)).arrayBuffer());
    const length = new DataView(bytes.buffer).getUint32(12, true);
    return JSON.parse(new TextDecoder().decode(bytes.subarray(24, 24 + length)));
  });
  expect(settings.wifi_ssid).toBe("after-history-ssid");
  expect(settings.wifi_password).toBe("after-history-password");
  expect(settings.groups[0]).toMatchObject({ name: "Commute", days: 31, start: 555, end: 750 });
});
test("searches keep choices explicit, scoped to the line and direction", async ({ page }, info) => {
  await prepare(page);
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#preview-line")).toHaveText("");
  await page.locator("#add-pick").click();
  await expect(page.locator("#save-pick")).toBeDisabled();
  await page.locator("#route-search").fill("4");
  const route = catalog.routes.findIndex((r) => r.line === "4");
  await expect(page.locator("#route-select option").nth(1)).toHaveAttribute("value", String(route));
  await expect(page.locator("#route-select")).toHaveValue("");
  await page.locator("#route-search").press("Enter");
  await expect(page.locator("#route-select")).toBeFocused();
  await page.locator("#route-select").selectOption({ value: String(route) });
  await expect(page.locator("#save-pick")).toBeDisabled();
  await page.locator("#stop-search").fill("no-such-stop");
  await expect(page.locator("#stop-results")).toContainText("No boarding stops match");
  const [, stop] = catalog.links.find(([r, , bits]) => r === route && bits & 1);
  await page.locator("#stop-search").fill(catalog.stops[stop].code.toLowerCase());
  await page.locator("#stop-search").fill(catalog.stops[stop].code || catalog.stops[stop].name);
  await page.locator(`#stop-options [data-group="${index.groupOf[stop]}"]`).click();
  await page.locator("#direction-select").selectOption(`${stop}:0`);
  await page.screenshot({ path: `/tmp/timetable-picker-${info.project.name}.png` });
  await page.locator("#save-pick").click();
  await expect(page.locator("#preview-line")).toHaveText("4");
  await expect(page.locator("#preview-mode")).toContainText("MANUAL");
  await expect(page.locator(".pick-edit")).toBeFocused();
  await page.locator(".pick-edit").click();
  await expect(page.locator("#stop-search")).toHaveValue(catalog.stops[stop].name);
  await page.locator("#route-search").fill("no-such-line");
  await expect(page.locator("#route-results")).toContainText("No lines match");
  await expect(page.locator("#save-pick")).toBeDisabled();
  await page.locator("#cancel-pick").click();
  await expect(page.locator("#preview-line")).toHaveText("4");
});
test("one stop box browses serving lines before a line is chosen", async ({ page }, info) => {
  await prepare(page);
  await page.locator("#add-pick").click();
  await expect(page.locator('#pick-dialog input[role="combobox"]')).toHaveCount(1);
  const route = catalog.routes.findIndex((r) => r.line === "4");
  const [, stop] = catalog.links.find(([r, s, bits]) => r === route && bits & 1 && catalog.links.some(([other, same]) => same === s && other !== route));
  await page.locator("#stop-search").fill(catalog.stops[stop].code.toLowerCase());
  await expect(page.locator("#route-select")).toHaveValue("");
  await expect(page.locator("#save-pick")).toBeDisabled();
  await page.locator("#stop-search").press("ArrowDown");
  await expect(page.locator("#stop-search")).toHaveAttribute("aria-activedescendant", `stop-group-${index.groupOf[stop]}`);
  await page.locator("#stop-search").press("Enter");
  await expect(page.locator("#stop-options")).toBeHidden();
  const routes = index.groupRoutes("", index.groupOf[stop]);
  expect(await page.locator("#route-select option").evaluateAll((options) => options.slice(1).map((o) => Number(o.value)))).toEqual(routes);
  await expect(page.locator("#route-results")).toContainText("serving");
  await expect(page.locator("#save-pick")).toBeDisabled();
  await page.locator("#route-select").selectOption({ value: String(route) });
  const choices = index.boardingChoices(route, index.groupOf[stop]).map(c => c.key);
  const options = page.locator("#direction-select option:not([value=''])");
  expect((await options.evaluateAll(options => options.map(o => o.value))).sort()).toEqual([...choices].sort());
  const destinations = await options.allTextContents();
  expect(destinations).toEqual([...destinations].sort(new Intl.Collator("pl", { numeric: true, sensitivity: "base" }).compare));
  await page.locator("#direction-select").selectOption(`${stop}:0`);
  await expect(page.locator("#save-pick")).toBeEnabled();
  await page.screenshot({ path: `/tmp/timetable-stop-first-${info.project.name}.png` });
  await page.locator("#save-pick").click();
  await expect(page.locator("#preview-stop")).toHaveText(catalog.stops[stop].name);
  await page.locator(".pick-edit").click();
  await expect(page.locator("#stop-search")).toHaveValue(catalog.stops[stop].name);
  await expect(page.locator("#direction-select")).toHaveValue(`${stop}:0`);
  await page.locator("#stop-search").fill("no-such-stop");
  await expect(page.locator("#save-pick")).toBeDisabled();
  await expect(page.locator("#stop-results")).toContainText("No boarding stops match");
  await page.locator("#stop-search").fill(catalog.stops[stop].code);
  await page.locator("#stop-search").press("Enter");
  // Typing a unique result still requires explicit selection.
  await expect(page.locator("#save-pick")).toBeDisabled();
  await page.locator("#stop-search").press("ArrowUp");
  await expect(page.locator("#stop-search")).toHaveAttribute("aria-activedescendant", `stop-group-${index.groupOf[stop]}`);
  await page.locator("#stop-search").press("Escape");
  await expect(page.locator("#stop-options")).toBeHidden();
  await expect(page.locator("#pick-dialog")).toBeVisible();
  await page.locator("#stop-search").press("ArrowDown");
  await page.locator(`#stop-options [data-group="${index.groupOf[stop]}"]`).click();
  await page.locator("#direction-select").selectOption(`${stop}:0`);
  await expect(page.locator("#save-pick")).toBeEnabled();
  await page.locator("#stop-search").press("Escape");
  await expect(page.locator("#pick-dialog")).toBeHidden();
  await expect(page.locator("#preview-stop")).toHaveText(catalog.stops[stop].name);
});
test("one named stop chooses either platform by destination and keeps route previews provisional", async ({ page }, info) => {
  await prepare(page);
  const icon = await page.request.get(await page.locator('link[rel="icon"]').getAttribute("href"));
  expect(icon.ok()).toBeTruthy(); expect(icon.headers()["content-type"]).toContain("image/svg+xml");
  await page.locator("#add-pick").click();
  const route = catalog.routes.findIndex(r => r.line === "4");
  const out = catalog.stops.findIndex(s => s.code === "MT44"), back = catalog.stops.findIndex(s => s.code === "MT71");
  const group = index.groupOf[out];
  await page.locator("#stop-search").fill("most teatralny");
  await expect(page.locator("#stop-options [role=option]")).toHaveCount(1);
  await page.locator(`#stop-options [data-group="${group}"]`).click();
  await page.locator("#route-select").selectOption({ value: String(route) });
  await expect(page.locator("#save-pick")).toBeDisabled();
  await expect(page.locator("#direction-select")).toHaveValue("");
  await expect(page.locator(`#direction-select option[value="${out}:0"]`)).toContainText("POŁABSKA");
  await expect(page.locator(`#direction-select option[value="${back}:1"]`)).toContainText("MAŁE GARBARY");
  await page.locator("#direction-select").selectOption(`${back}:1`);
  await expect(page.locator("#platform-hint")).toContainText("MT71");
  await expect(page.locator("#platform-hint")).toContainText("next");
  await expect(page.locator("#stop-search")).toHaveValue("Most Teatralny");
  await page.locator("#direction-select").selectOption(`${out}:0`);
  await expect(page.locator("#platform-hint")).toContainText("MT44");
  await expect(page.locator("#stop-search")).toHaveValue("Most Teatralny");
  await page.locator("#save-pick").click();
  await expect(page.locator("#preview-direction")).toHaveText("POŁABSKA");
  await page.locator(".pick-edit").click();
  await expect(page.locator("#direction-select")).toHaveValue(`${out}:0`);
  await page.locator("#direction-select").selectOption("");
  const returning = catalog.patterns.map((p, id) => ({ p, id })).filter(({ p }) => p.route === route && p.direction === 1 && p.calls.some(c => c[0] === back && c[1] !== 1)).sort((a, b) => b.p.trips - a.p.trips)[0];
  await page.locator("#path-select").selectOption(String(returning.id));
  await expect(page.locator("#direction-select")).toHaveValue("");
  const noPickup = returning.p.calls.findIndex(c => c[1] === 1);
  if (noPickup >= 0) await expect(page.locator(`#route-stops [data-call="${noPickup}"]`)).toBeDisabled();
  await page.locator(`#route-graph circle.active[data-stop="${back}"]`).first().click();
  await expect(page.locator("#direction-select")).toHaveValue(`${back}:1`);
  await expect(page.locator("#platform-hint")).toContainText("MT71");
  await page.screenshot({ path: `/tmp/timetable-grouped-directions-${info.project.name}.png` });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
});
test("keyboard groups and undo retain selections without overwriting later edits", async ({ page }) => {
  // Catalogue arrival refreshes the tabs while a keyboard user may already
  // have focused one. Release that response only after focus is established.
  let releaseCatalog;
  const catalogReady = new Promise(resolve => { releaseCatalog = resolve; });
  await page.route("**/catalog.json", async route => { await catalogReady; await route.fulfill({ json: catalog }); });
  await prepare(page);
  await page.getByRole("tab", { name: "Morning", exact: true }).focus();
  releaseCatalog();
  await expect(page.locator("#add-pick")).toBeEnabled();
  await expect(page.getByRole("tab", { name: "Morning", exact: true })).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "Evening", exact: true })).toBeFocused();
  await expect(page.locator("#group-name")).toHaveValue("Evening");
  await page.keyboard.press("Home");
  await expect(page.getByRole("tab", { name: "Morning", exact: true })).toBeFocused();
  await page.locator("#group-name").fill("Commute");
  await page.locator("#add-pick").click();
  const route = catalog.routes.findIndex((r) => r.line === "4");
  await page.locator("#route-select").selectOption({ value: String(route) });
  await chooseStop(page, route);
  await page.locator("#save-pick").click();
  await page.locator("#remove-group").click();
  await expect(page.locator("#group-name")).toHaveValue("Evening");
  await page.locator("#undo-removal").click();
  await expect(page.locator("#group-name")).toHaveValue("Commute");
  await expect(page.locator(".pick-edit")).toHaveCount(1);
  await page.locator(".pick-edit").click();
  await page.locator("#delete-pick").click();
  await expect(page.locator("#preview-line")).toHaveText("");
  await page.locator("#undo-removal").click();
  await expect(page.locator("#preview-line")).toHaveText("4");
  await page.locator("#remove-group").click();
  await page.locator("#group-name").fill("New plans");
  await expect(page.locator("#undo-area")).toBeHidden();
});
test("failed route and firmware downloads recover without losing group edits", async ({ page }) => {
  await prepare(page, { catalogFailures: 1, releaseFailures: 1 });
  await expect(page.locator("#retry-catalog")).toBeVisible();
  await expect(page.locator("#retry-release")).toBeVisible();
  await expect(page.locator("#add-pick")).toBeDisabled();
  await page.locator("#group-name").fill("Keep this group");
  await page.locator("#retry-catalog").click();
  await expect(page.locator("#add-pick")).toBeEnabled();
  await page.locator("#retry-release").click();
  await expect(page.locator("#prepare")).toBeEnabled();
  await expect(page.locator("#group-name")).toHaveValue("Keep this group");
  await expect(page.locator("#retry-catalog")).toBeHidden();
  await expect(page.locator("#retry-release")).toBeHidden();
});
test("verified firmware waits for a failed default catalogue to recover before preparation", async ({ page }) => {
  await prepare(page, {catalogFailures: 1});
  await expect(page.locator("#release-state")).toHaveText("Firmware verified");
  await expect(page.locator("#retry-catalog")).toBeVisible();
  await expect(page.locator("#prepare")).toBeDisabled();
  await page.locator("#retry-catalog").click();
  await expect(page.locator("#prepare")).toBeEnabled({timeout:15000});
  await page.locator("#wifi-ssid").fill("catalogue-recovery-test");
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
});
test("a display name that is too long is shown at its group before installation", async ({ page }) => {
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#group-name").fill("ą".repeat(13));
  await page.getByRole("tab", { name: "Evening", exact: true }).click();
  await page.locator("#wifi-ssid").fill("name-validation-network");
  await page.locator("#wifi-password").fill("name-validation-password");
  await page.locator("#prepare").click();
  await expect(page.locator("#group-name")).toHaveValue("ą".repeat(13));
  await expect(page.locator("#group-name")).toBeFocused();
  await expect(page.locator("#install-area")).toBeHidden();
  await page.locator("#group-name").fill("Łąkowa");
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
});

test("shared Rust preview prioritizes reachable departures and handles impossible walks", async ({ page }) => {
  await prepare(page);
  await page.locator("#add-pick").click();
  const route = catalog.routes.findIndex((r) => r.line === "4");
  await page.locator("#route-select").selectOption({ value: String(route) });
  await chooseStop(page, route);
  await page.locator("#walk-minutes").fill("12");
  await page.getByText("Leave a little room", { exact: true }).click();
  await page.locator("#walk-spread").fill("2");
  await page.locator("#walk-margin").fill("1");
  await page.locator("#save-pick").click();
  await expect(page.locator("#preview-leave")).toHaveText("Leave in 8 min");
  await expect(page.locator("#preview-walk")).toContainText("100% walk est.");
  await page.locator(".pick-edit").click();
  await page.locator("#walk-spread").fill("0");
  await page.locator("#walk-minutes").fill("60");
  await page.locator("#save-pick").click();
  await expect(page.locator("#preview-leave")).toHaveText("Not enough walk time");
  await expect(page.locator("#preview-walk")).toContainText("0% walk est.");
  await page.locator(".pick-edit").click();
  await page.locator("#walk-minutes").fill("0");
  await page.locator("#walk-margin").fill("0");
  await page.locator("#save-pick").click();
  await expect(page.locator("#preview-leave")).toHaveText("Leave in 12 min");
  await expect(page.locator("#preview-walk")).toContainText("100% walk est.");
});

test("weekday toggles keep keyboard focus and serialize the chosen days", async ({ page }) => {
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#scheduled").check();
  await page.getByRole("button", { name: "Monday", exact: true }).focus();
  await page.keyboard.press("Space");
  await expect(page.getByRole("button", { name: "Monday", exact: true })).toBeFocused();
  await expect(page.getByRole("button", { name: "Monday", exact: true })).toHaveAttribute("aria-pressed", "false");
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "Tuesday", exact: true })).toBeFocused();
  await page.keyboard.press("Space");
  await expect(page.getByRole("button", { name: "Tuesday", exact: true })).toBeFocused();
  await page.locator("#wifi-ssid").fill("weekday-test-network");
  await page.locator("#prepare").click();
  const days = await page.evaluate(async () => {
    const manifest = await (await fetch(document.querySelector("#installer").manifest)).json();
    const bytes = await (await fetch(manifest.builds[0].parts[1].path)).arrayBuffer();
    return JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 24, new DataView(bytes).getUint32(12, true)))).groups[0].days;
  });
  expect(days).toBe(28);
  await page.getByRole("button", { name: "Wednesday", exact: true }).focus();
  for (const day of ["Wednesday", "Thursday", "Friday"]) {
    await expect(page.getByRole("button", { name: day, exact: true })).toBeFocused();
    await page.keyboard.press("Space");
    if (day !== "Friday") await page.keyboard.press("Tab");
  }
  await expect(page.locator("#scheduled")).not.toBeChecked();
  await expect(page.locator("#schedule")).toBeHidden();
  await expect(page.locator("#scheduled")).toBeFocused();
  await expect(page.locator("#install-area")).toBeHidden();
  await page.keyboard.press("Space");
  await expect(page.locator("#scheduled")).toBeChecked();
  await expect(page.locator("#schedule")).toBeVisible();
});

test("imported manual mode can resume scheduling, roundtrip and undo", async ({ page, isMobile }) => {
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#group-name").fill("Before import");
  await page.locator("#wifi-ssid").fill("kept-private-network");
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
  // Exercise the desktop keyboard and mobile pointer file-picker entry points.
  // Arm native picker interception before the awaited focus/keyboard actions.
  const chooser = page.waitForEvent("filechooser");
  await page.locator("#import-button").focus();
  await expect(page.locator("#import-button")).toBeFocused();
  if (isMobile) await page.locator("#import-button").click();
  else await page.locator("#import-button").press("Enter");
  await (await chooser).setFiles({ name: "manual.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify({ version: 1, automatic: false, groups: saved.groups })) });
  await expect(page.locator("#automatic")).not.toBeChecked();
  await expect(page.locator("#automatic-hint")).toContainText("Schedules are paused");
  await expect(page.locator("#preview-mode")).toContainText("MANUAL");
  await expect(page.locator("#scheduled")).toBeChecked();
  await expect(page.locator("#group-name")).toBeFocused();
  await expect(page.locator("#install-area")).toBeHidden();
  await expect(page.locator("#wifi-ssid")).toHaveValue("kept-private-network");
  const download = page.waitForEvent("download");
  await page.locator("#export-groups").click();
  const exported = JSON.parse(await readFile(await (await download).path(), "utf8"));
  expect(exported).toEqual({ version: 1, automatic: false, groups: saved.groups });
  await page.locator("#undo-removal").click();
  await expect(page.locator("#group-name")).toHaveValue("Before import");
  await expect(page.locator("#automatic")).toBeChecked();
  await expect(page.locator("#preview-line")).toHaveText("");
  // Import the same file again, then explicitly resume all saved schedules.
  await page.locator("#import-groups").setInputFiles({ name: "manual.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(exported)) });
  await expect(page.locator("#automatic")).not.toBeChecked();
  await page.locator("#automatic").check();
  // The selected-group preview begins with the device's manual override.
  await expect(page.locator("#preview-mode")).toContainText("MANUAL");
  await deviceTap(page,120,24);
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "groups");
  await deviceTap(page,120,212); // Resume automatic schedules in the real UI.
  await expect(page.locator("#preview-mode")).toContainText("SCHEDULED");
  await expect(page.locator("#undo-area")).toBeHidden();
  await page.locator("#prepare").click();
  const settings = await page.evaluate(async () => {
    const manifest = await (await fetch(document.querySelector("#installer").manifest)).json();
    const bytes = await (await fetch(manifest.builds[0].parts[1].path)).arrayBuffer();
    return JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 24, new DataView(bytes).getUint32(12, true))));
  });
  expect(settings.automatic).toBe(true);
  expect(settings.groups).toEqual(saved.groups);
  expect(settings.wifi_ssid).toBe("kept-private-network");
  await page.locator("#automatic").uncheck();
  await expect(page.locator("#install-area")).toBeHidden();
  await expect(page.locator("#scheduled")).toBeChecked();
  await expect(page.locator("#preview-mode")).toContainText("MANUAL");
});

test("invalid group imports preserve settings and an existing undo", async ({ page }) => {
  await prepare(page);
  await page.locator("#group-name").fill("Keep my plans");
  await page.locator("#remove-group").click();
  await expect(page.locator("#undo-area")).toBeVisible();
  for (const body of ["{", JSON.stringify({ version: 1, automatic: false, groups: [] })]) {
    await page.locator("#import-groups").setInputFiles({ name: "bad.json", mimeType: "application/json", buffer: Buffer.from(body) });
    await expect(page.locator("#status")).toHaveAttribute("role", "alert");
    await expect(page.locator("#group-name")).toHaveValue("Evening");
    await expect(page.locator("#automatic")).toBeChecked();
    await expect(page.locator("#undo-area")).toBeVisible();
  }
  await page.locator("#undo-removal").click();
  await expect(page.locator("#group-name")).toHaveValue("Keep my plans");
});

test("battery mode is explicit, keeps groups and reaches USB settings", async ({ page }) => {
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await expect(page.locator("#battery-mode")).not.toBeChecked();
  await page.locator("#wifi-ssid").fill('Home "2.4"');
  await page.locator("#wifi-password").fill('pa\\ss"word');
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
  await page.locator("#battery-mode").check();
  await expect(page.locator("#install-area")).toBeHidden();
  await expect(page.locator("#battery-hint")).toContainText("Tap to wake");
  await page.locator("#import-groups").setInputFiles({ name: "groups.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify({ version: 1, automatic: saved.automatic, groups: saved.groups })) });
  await expect(page.locator(".pick-edit")).toHaveCount(1);
  await expect(page.locator("#battery-mode")).toBeChecked();
  await page.locator("#prepare").click();
  const settings = await page.evaluate(async () => {
    const manifest = await (await fetch(document.querySelector("#installer").manifest)).json();
    const bytes = await (await fetch(manifest.builds[0].parts[1].path)).arrayBuffer();
    return JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 24, new DataView(bytes).getUint32(12, true))));
  });
  expect(settings.battery).toBe(true);
  expect(settings.wifi_ssid).toBe('Home "2.4"');
  expect(settings.wifi_password).toBe('pa\\ss"word');
  expect(settings.groups).toEqual(saved.groups);
  await page.locator("#battery-mode").uncheck();
  await expect(page.locator("#install-area")).toBeHidden();
});

test("PL and EN translate the page and picker while preserving drafts and prepared settings", async ({ page }, info) => {
  await prepare(page);
  await page.locator("#wifi-ssid").fill("Home-private");
  await page.locator("#wifi-password").fill("private-password");
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "pl");
  await expect(page.locator("#prepare")).toContainText("Przygotuj");
  await expect(page.locator("#wifi-ssid")).toHaveAttribute("placeholder", "Twoja sieć 2,4 GHz");
  await expect(page.locator("#group-name")).toHaveValue("Morning"); // An editable name is user data.
  await page.locator("#add-pick").click();
  await expect(page.locator("#pick-title")).toHaveText("Linia. Przystanek. Twoje tempo.");
  await page.locator("#stop-search").fill("most teatralny");
  await expect(page.locator("#stop-options [role=option]")).toHaveCount(1);
  const out = catalog.stops.findIndex(s => s.code === "MT44"), route = catalog.routes.findIndex(r => r.line === "4");
  await page.locator(`#stop-options [data-group="${index.groupOf[out]}"]`).click();
  await page.locator("#route-select").selectOption({ value: String(route) });
  await page.locator("#direction-select").selectOption(`${out}:0`);
  await page.locator("#walk-minutes").fill("11");
  await expect(page.locator("#platform-hint")).toContainText("Wsiądź na słupku MT44");
  await page.locator("#pick-dialog [data-language=en]").click();
  await expect(page.locator("#pick-title")).toHaveText("A line. A stop. Your pace.");
  await expect(page.locator("#walk-minutes")).toHaveValue("11");
  await expect(page.locator("#direction-select")).toHaveValue(`${out}:0`);
  await page.locator("#save-pick").click();
  await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("#install-area")).toBeVisible();
  await expect(page.locator("#wifi-password")).toHaveValue("private-password");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("lang", "pl");
  await expect(page.locator("#wifi-ssid")).toHaveValue("Home-private");
  await expect(page.locator("#preview-line")).toHaveText("4");
  await expect(page.locator("#preview-walk")).toContainText("11 min pieszo");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({ path: `/tmp/timetable-polish-${info.project.name}.png`, fullPage: true });
});

test("remembered settings reload locally and forgetting prevents later credential reuse", async ({ page }) => {
  const requests = []; page.on("request", r => requests.push(r.url() + (r.postData() ?? "")));
  await prepare(page);
  await page.locator("#wifi-ssid").fill("private-remember-ssid");
  await page.locator("#wifi-password").fill("private-remember-password");
  await page.locator("#group-name").fill("Saved group");
  await page.locator("#battery-mode").check();
  await page.reload();
  await expect(page.locator("#wifi-ssid")).toHaveValue("private-remember-ssid");
  await expect(page.locator("#wifi-password")).toHaveValue("private-remember-password");
  await expect(page.locator("#wifi-password")).toHaveAttribute("type", "password");
  await expect(page.locator("#group-name")).toHaveValue("Saved group");
  await expect(page.locator("#battery-mode")).toBeChecked();
  await page.locator("#forget-settings").click();
  await expect(page.locator("#remember-settings")).not.toBeChecked();
  await expect(page.locator("#wifi-ssid")).toHaveValue("private-remember-ssid");
  await page.reload();
  await expect(page.locator("#wifi-ssid")).toHaveValue("");
  await expect(page.locator("#wifi-password")).toHaveValue("");
  expect(requests.filter(r => !r.startsWith("blob:")).join("\n")).not.toContain("private-remember");
});

test("USB logs are passive, bounded, downloadable and release the port", async ({ page }) => {
  await prepare(page);
  await page.evaluate(() => {
    let stream; window.logCalls = [];
    navigator.serial.requestPort = async () => ({
      readable: new ReadableStream({ start(c) { stream = c; window.pushLog = value => stream.enqueue(new TextEncoder().encode(value)); } }),
      open: async options => window.logCalls.push(["open", options]),
      close: async () => window.logCalls.push(["close"]),
      setSignals: () => { throw Error("must not reset"); },
      get writable() { throw Error("must not write"); },
    });
  });
  await page.locator("#wifi-ssid").fill("log-test");
  await page.locator("#prepare").click();
  await page.locator("#logs-panel summary").click();
  await page.locator("#connect-logs").click();
  await expect(page.locator("#logs-status")).toContainText("Reading logs");
  await expect(page.locator("#install-action")).toBeDisabled();
  await page.evaluate(() => window.pushLog("Wi-Fi association failed: AuthFail\nŁączę ponownie\n"));
  await expect(page.locator("#log-output")).toContainText("Łączę ponownie");
  const download = page.waitForEvent("download");
  await page.locator("#save-logs").click();
  expect(await readFile(await (await download).path(), "utf8")).toContain("AuthFail");
  await page.evaluate(() => window.pushLog("z".repeat(70000)));
  await expect.poll(() => page.locator("#log-output").textContent().then(s => s.length)).toBe(65536);
  await page.locator("#disconnect-logs").click();
  await expect(page.locator("#logs-status")).toContainText("USB disconnected");
  await expect(page.locator("#install-action")).toBeEnabled();
  expect(await page.evaluate(() => window.logCalls)).toEqual([["open", { baudRate: 115200, bufferSize: 8192 }], ["close"]]);
  await page.locator("#clear-logs").click();
  await expect(page.locator("#log-output")).toHaveText("");
});

test.describe("Polish browser preference", () => {
  test.use({ locale: "pl-PL" });
  test("starts in Polish and translates validation and connection failures", async ({ page }) => {
    await prepare(page, { releaseFailures: 1 });
    await expect(page.locator("html")).toHaveAttribute("lang", "pl");
    await expect(page.locator("#group-name")).toHaveValue("Poranek");
    await expect(page.locator("#release-state")).toHaveText("Aplikacja niedostępna");
    await expect(page.locator("#status")).toContainText("Nie udało się wczytać");
    await page.locator("#retry-release").click();
    await expect(page.locator("#prepare")).toBeEnabled();
    await page.locator("#wifi-ssid").fill("test-network");
    await page.locator("#wifi-password").fill("short");
    await page.locator("#prepare").click();
    await expect(page.locator("#status")).toContainText("Hasło Wi-Fi musi mieć");
    await page.locator(".masthead [data-language=en]").click();
    await expect(page.locator("#status")).toContainText("8–63 byte Wi-Fi password");
    await expect(page.locator("#group-name")).toHaveValue("Poranek");
  });
});

test("unavailable browser storage keeps editing and language switching usable", async ({ page }) => {
  await page.addInitScript(() => Object.defineProperty(window, "localStorage", { get() { throw new DOMException("blocked", "SecurityError"); } }));
  await prepare(page);
  await page.locator("#group-name").fill("Still editable");
  await page.locator("#wifi-ssid").fill("still-private");
  await expect(page.locator("#storage-status")).toContainText("could not save settings");
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "pl");
  await expect(page.locator("#wifi-ssid")).toHaveValue("still-private");
  await expect(page.locator("#group-name")).toHaveValue("Still editable");
  await expect(page.locator("#storage-status")).toContainText("nie mogła zapisać");
});

test("forget removes credentials even when storage rejects preference writes", async ({ page }) => {
  await prepare(page);
  await page.locator("#wifi-ssid").fill("erase-private-network");
  await page.locator("#wifi-password").fill("erase-private-password");
  await page.evaluate(() => {
    Storage.prototype.setItem = () => { throw new DOMException("full", "QuotaExceededError"); };
  });
  await page.locator("#forget-settings").click();
  expect(await page.evaluate(() => localStorage.getItem("timetable.settings.v1"))).toBeNull();
  await page.reload();
  await expect(page.locator("#wifi-password")).toHaveValue("");
  await expect(page.locator("#wifi-ssid")).toHaveValue("");
});

test("disconnect waits for a pending USB open before enabling installation", async ({ page }) => {
  await prepare(page);
  await page.evaluate(() => {
    window.logCalls = [];
    navigator.serial.requestPort = async () => ({
      open: () => new Promise(resolve => { window.finishUsbOpen = resolve; }),
      close: async () => window.logCalls.push("close"),
    });
  });
  await page.locator("#wifi-ssid").fill("log-test");
  await page.locator("#prepare").click();
  await page.locator("#logs-panel summary").click();
  await page.locator("#connect-logs").click();
  await expect.poll(() => page.evaluate(() => typeof window.finishUsbOpen)).toBe("function");
  await page.locator("#disconnect-logs").click();
  await expect(page.locator("#logs-status")).toHaveText("Closing USB port…");
  await expect(page.locator("#install-action")).toBeDisabled();
  await expect(page.locator("#connect-logs")).toBeDisabled();
  await page.evaluate(() => window.finishUsbOpen());
  await expect(page.locator("#install-action")).toBeEnabled();
  await expect(page.locator("#logs-status")).toContainText("USB disconnected");
  expect(await page.evaluate(() => window.logCalls)).toEqual(["close"]);
});

async function deviceTap(page, x, y, held = false) {
  const canvas = page.locator("#device-canvas");
  await canvas.scrollIntoViewIfNeeded();
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x + x * box.width / 240, box.y + y * box.height / 240);
  await page.mouse.down();
  if (held) await page.waitForTimeout(750);
  await page.mouse.up();
}
test("line and stop pickers stay alphabetically ordered in both page languages", async ({ page }) => {
  await prepare(page);
  await page.locator("#add-pick").click();
  const compare = new Intl.Collator("pl", { numeric: true, sensitivity: "base" }).compare;
  const lines = await page.locator("#route-select option:not([value=''])").allTextContents();
  expect(lines.length).toBeGreaterThan(100);
  expect(lines).toEqual([...lines].sort(compare));
  const route = catalog.routes.findIndex(r => r.line === "4");
  await page.locator("#route-select").selectOption(String(route));
  await page.locator("#stop-search").press("ArrowDown");
  const labels = page.locator("#stop-options .stop-copy > span");
  const stops = await labels.allTextContents();
  expect(stops.length).toBeGreaterThan(20);
  expect(stops).toEqual([...stops].sort(compare));
  await page.locator("#pick-dialog [data-language=pl]").click();
  await expect(labels).toHaveText(stops);
  await page.locator("#pick-dialog [data-language=en]").click();
  await expect(labels).toHaveText(stops);
});
test("real WASM touch edits are provisional, apply preserves Wi-Fi, and reset restores installer settings", async ({ page }, info) => {
  await page.addInitScript(() => {
    const instantiate = WebAssembly.instantiate;
    WebAssembly.instantiate = async (...args) => {
      const result = await instantiate(...args);
      window.previewProbe = result.instance.exports;
      return result;
    };
  });
  await prepare(page);
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-ready", "true");
  await page.locator("#wifi-ssid").fill("private-demo-test");
  await page.locator("#wifi-password").fill("private-demo-password");
  await page.locator("#battery-mode").check();
  await page.locator("#add-pick").click();
  const route = catalog.routes.findIndex(r => r.line === "4");
  await page.locator("#route-select").selectOption({value:String(route)});
  await chooseStop(page, route);
  await page.locator("#walk-minutes").fill("6");
  await page.locator("#save-pick").click();
  await deviceTap(page,120,120,true);
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "picks");
  await deviceTap(page,120,65);
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "walk");
  await deviceTap(page,180,80);
  await deviceTap(page,60,32); // Cancel, without Save.
  await page.locator(".preview-controls summary").click();
  await expect(page.locator("#preview-apply")).toBeDisabled();
  await deviceTap(page,120,65);
  await deviceTap(page,180,80);
  await deviceTap(page,180,200); // Save inside the demo.
  await expect(page.locator("#preview-apply")).toBeEnabled();
  await expect(page.locator(".pick-walk")).toHaveText("6m walk");
  // Device-only settings must preserve the saved demo until explicitly applied.
  await page.locator("#wifi-ssid").fill("new-private-demo-test");
  await page.locator("#wifi-password").fill("new-private-demo-password");
  await page.locator("#battery-mode").uncheck();
  await expect(page.locator("#preview-apply")).toBeEnabled();
  await expect(page.locator("#preview-walk")).toContainText("7m walk");
  await page.locator("#preview-apply").click();
  await expect(page.locator(".pick-walk")).toHaveText("7m walk");
  await expect(page.locator("#wifi-ssid")).toHaveValue("new-private-demo-test");
  await expect(page.locator("#wifi-password")).toHaveValue("new-private-demo-password");
  await expect(page.locator("#battery-mode")).not.toBeChecked();
  const mocked = await page.evaluate(() => {
    const api=window.previewProbe, length=api.preview_snapshot();
    return JSON.parse(new TextDecoder().decode(new Uint8Array(api.memory.buffer,api.preview_output(),length))).config;
  });
  expect(mocked.wifi_ssid).toBe("Browser demo");
  expect(mocked.wifi_password).toBe("");
  expect(mocked.battery).toBeUndefined();
  await deviceTap(page,120,120,true);
  await deviceTap(page,120,65);
  await deviceTap(page,180,80);
  await deviceTap(page,180,200);
  await expect(page.locator("#preview-walk")).toContainText("8m walk");
  await page.locator("#preview-reset").click();
  await expect(page.locator("#preview-walk")).toContainText("7m walk");
  await expect(page.locator("#preview-apply")).toBeDisabled();
  await page.locator("#preview-scenario").selectOption("1");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-clock", "pending");
  await expect(page.locator("#preview-leave")).toHaveText("Saved times / waiting for NTP");
  await page.locator("#preview-scenario").selectOption("2");
  await expect(page.locator("#preview-line")).toHaveText("");
  await page.locator("#preview-scenario").selectOption("0");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-clock", "known");
  await page.locator("#device-canvas").focus();
  await page.keyboard.press("e");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "picks");
  await page.keyboard.press("1");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "walk");
  await expect(page.locator("#device-canvas")).toHaveAttribute("aria-label", "Interactive device demo: Walking time");
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "walk");
  await expect(page.locator("#device-canvas")).toHaveAttribute("aria-label", "Interaktywne demo urządzenia: Czas dojścia");
  await expect(page.locator("#preview-reset")).toHaveText("Zresetuj demo");
  await page.locator("#device-canvas").focus();
  await page.keyboard.press("+");
  await page.keyboard.press("Enter");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-screen", "board");
  await expect(page.locator("#preview-apply")).toBeEnabled();
  await page.screenshot({ path: `/tmp/timetable-wasm-${info.project.name}.png`, fullPage:true });
});
test("a failed WASM download leaves installer usable and retries without losing configuration", async ({ page }) => {
  let failures = 1;
  await page.route("**/device-preview.wasm", route => failures-- > 0 ? route.fulfill({status:503}) : route.continue());
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await expect(page.locator("#preview-retry")).toBeVisible();
  await page.locator("#group-name").fill("Keep my group");
  await page.locator("#preview-retry").click();
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#preview-group")).toHaveText("Keep my group");
  await expect(page.locator("#group-name")).toHaveValue("Keep my group");
});

test("invalid installer drafts suspend stale WASM interactions until valid settings return", async ({ page }) => {
  await page.clock.install();
  let moduleRequests = 0;
  page.on("request", request => { if (request.url().endsWith("device-preview.wasm")) moduleRequests++; });
  await prepare(page);
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-ready", "true");
  await page.locator("#add-pick").click();
  const route = catalog.routes.findIndex(r => r.line === "4");
  await page.locator("#route-select").selectOption({value:String(route)});
  await chooseStop(page, route);
  await page.locator("#save-pick").click();
  const invalid = "ą".repeat(13); // HTML character limit permits it; Rust's 24-byte name does not.
  await page.locator("#group-name").fill(invalid);
  await expect(page.locator("#device-canvas")).toHaveAttribute("aria-disabled", "true");
  await expect(page.locator("#preview-state")).toHaveText("Complete valid group settings to resume the demo.");
  await page.clock.fastForward(2000); // Four refresh intervals must not republish the old snapshot.
  for (const key of ["e", "1", "+", "Enter"]) await page.locator("#device-canvas").dispatchEvent("keydown", {key});
  await expect(page.locator("#preview-apply")).toBeDisabled();
  await expect(page.locator("#preview-state")).toHaveText("Complete valid group settings to resume the demo.");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-ready", "false");
  await expect(page.locator("#group-name")).toHaveValue(invalid);
  await page.locator("#group-name").fill("Valid new group");
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#device-canvas")).toHaveAttribute("aria-disabled", "false");
  await expect(page.locator("#preview-group")).toHaveText("Valid new group");
  await expect(page.locator("#preview-line")).toHaveText("4");
  expect(moduleRequests).toBe(1);
});


test("C6 selection switches firmware, portrait controls and remembered model without mixing chip manifests", async ({page},info)=>{
  await page.addInitScript(config=>{
    if(!localStorage.getItem("timetable.settings.v1")) localStorage.setItem("timetable.settings.v1",JSON.stringify({version:1,active:0,config}));
  },saved);
  await prepare(page);
  await page.route("**/devices/waveshare-c6/latest.json",r=>r.fulfill({json:{...release,chip:"ESP32-C6",board:"Waveshare ESP32-C6-Touch-LCD-1.47",flash_size:8388608}}));
  await page.locator("#device-model").selectOption("waveshare-c6");
  await expect(page.locator("#prepare")).toBeEnabled();
  const canvas=page.locator("#device-canvas");
  await expect(canvas).toHaveAttribute("width","172");await expect(canvas).toHaveAttribute("height","320");
  await expect(canvas).toHaveAttribute("data-ready","true");
  await canvas.focus();await page.keyboard.press("e");await expect(canvas).toHaveAttribute("data-screen","picks");
  await page.keyboard.press("1");await expect(canvas).toHaveAttribute("data-screen","walk");
  await page.keyboard.press("+");await page.keyboard.press("Enter");await expect(canvas).toHaveAttribute("data-screen","board");
  await page.locator("#wifi-ssid").fill("C6-network");await page.locator("#prepare").click();
  const manifest=await page.evaluate(async()=>await(await fetch(document.querySelector("#installer").manifest)).json());
  expect(manifest.builds[0].chipFamily).toBe("ESP32-C6");expect(manifest.builds[0].parts[1].offset).toBe(4128768);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBeTruthy();
  await page.screenshot({path:`/tmp/timetable-c6-${info.project.name}.png`,fullPage:true});
  await canvas.screenshot({path:`/tmp/timetable-c6-panel-${info.project.name}.png`});
  await page.reload();await expect(page.locator("#device-model")).toHaveValue("waveshare-c6");await expect(canvas).toHaveAttribute("width","172");
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#device-model").selectOption("guition-c3");await expect(page.locator("#install-area")).toBeHidden();
  await expect(page.locator("#prepare")).toBeEnabled();await expect(canvas).toHaveAttribute("width","240");
  await page.locator("#prepare").click();
  expect(await page.evaluate(async()=> (await(await fetch(document.querySelector("#installer").manifest)).json()).builds[0].chipFamily)).toBe("ESP32-C3");
});

test("landscape has large touch cards, complete list paging and a remembered orientation-specific install",async({page},info)=>{
 const config=structuredClone(saved),route=catalog.routes.find(r=>r.line==="4"),link=catalog.links.find(([r,,bits])=>catalog.routes[r].id===route.id&&(bits&1)),stop=catalog.stops[link[1]];
 config.groups[0].picks=Array.from({length:4},(_,i)=>({...saved.groups[0].picks[0],route:route.id,line:route.line,stop:stop.id,stop_code:stop.code,stop_name:stop.name,headsign:route.headsigns[0],walk:6+i}));
 await page.addInitScript(config=>{if(!localStorage.getItem("timetable.settings.v1"))localStorage.setItem("timetable.settings.v1",JSON.stringify({version:1,active:0,config}));},config);
 await prepare(page);
 await expect(page.locator("#orientation-control")).toBeHidden();
 const c6={...release,chip:"ESP32-C6",board:"Waveshare ESP32-C6-Touch-LCD-1.47",flash_size:8388608};
 await page.route("**/devices/waveshare-c6/latest.json",r=>r.fulfill({json:{...c6,orientation:"portrait"}}));
 await page.route("**/devices/waveshare-c6-landscape/latest.json",r=>r.fulfill({json:{...c6,orientation:"landscape"}}));
 await page.locator("#device-model").selectOption("waveshare-c6");await expect(page.locator("#prepare")).toBeEnabled();
 await page.locator("#wifi-ssid").fill("landscape-network");await page.locator("#prepare").click();await expect(page.locator("#install-area")).toBeVisible();
 await page.locator("#device-orientation").selectOption("landscape");await expect(page.locator("#install-area")).toBeHidden();
 const canvas=page.locator("#device-canvas");await expect(canvas).toHaveAttribute("width","320");await expect(canvas).toHaveAttribute("height","172");await expect(canvas).toHaveAttribute("data-orientation","landscape");
 await expect(page.locator("#prepare")).toBeEnabled();
 const tap=async(x,y,delay=0)=>{const box=await canvas.boundingBox();await canvas.click({position:{x:x*box.width/320,y:y*box.height/172},delay});};
 await canvas.screenshot({path:`/tmp/timetable-landscape-board-${info.project.name}.png`});
 await tap(160,90,800);await expect(canvas).toHaveAttribute("data-screen","picks");
 await canvas.screenshot({path:`/tmp/timetable-landscape-choices-${info.project.name}.png`});
 await tap(240,104);await expect(canvas).toHaveAttribute("data-screen","walk");
 await tap(80,100);await tap(186,100);await tap(292,100);
 await canvas.screenshot({path:`/tmp/timetable-landscape-walk-${info.project.name}.png`});
 await tap(260,150);await expect(canvas).toHaveAttribute("data-screen","board");
 await page.locator("#preview-apply").click();await expect(page.locator(".pick-walk").nth(3)).toHaveText("10m walk");
 await tap(160,90,800);await tap(240,104);await tap(24,44);await expect(canvas).toHaveAttribute("data-screen","routes");
 await tap(160,56);await expect(canvas).toHaveAttribute("data-screen","stops");
 const first=await canvas.evaluate(c=>c.toDataURL());await tap(286,150);expect(await canvas.evaluate(c=>c.toDataURL())).not.toBe(first);
 await tap(32,150);expect(await canvas.evaluate(c=>c.toDataURL())).toBe(first);
 await canvas.screenshot({path:`/tmp/timetable-landscape-stops-${info.project.name}.png`});
 await page.locator("#preview-reset").click();await expect(canvas).toHaveAttribute("data-screen","board");
 await page.locator(".preview-controls summary").click();
 await page.locator("#preview-scenario").selectOption("1");await expect(canvas).toHaveAttribute("data-clock","pending");await expect(page.locator("#preview-leave")).toHaveText("Saved times / waiting for NTP");
 await page.locator("#prepare").click();const manifest=await page.evaluate(async()=>await(await fetch(document.querySelector("#installer").manifest)).json());expect(manifest.builds[0].chipFamily).toBe("ESP32-C6");
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 expect(await page.evaluate(()=>JSON.parse(localStorage.getItem("timetable.settings.v1")).orientation)).toBe("landscape");
 await page.reload();await expect(canvas).toHaveAttribute("data-ready","true");await expect(page.locator("#device-orientation")).toHaveValue("landscape");await expect(canvas).toHaveAttribute("data-orientation","landscape");
 await page.getByRole("button",{name:"Polski",exact:true}).click();await expect(page.locator("#device-orientation option:checked")).toHaveText("Pozioma");
 await page.locator("#device-orientation").selectOption("portrait");await expect(canvas).toHaveAttribute("width","172");await expect(page.locator("#install-area")).toBeHidden();
});
