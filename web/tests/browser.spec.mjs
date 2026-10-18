import { test, expect } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
const catalog = JSON.parse(await readFile(new URL("../../catalog/catalog.json", import.meta.url), "utf8"));
const firmware = Buffer.alloc(1e5, 233);
const release = { chip: "ESP32-C3", board: "GUITION ESP32-2424S012-V1.0", flash_size: 4194304, config_offset: 4128768, config_version: 1, revision: "a".repeat(40), sha256: createHash("sha256").update(firmware).digest("hex"), size: firmware.length };
async function prepare(page, { corrupt = false } = {}) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "serial", { value: { getPorts: async () => [] }, configurable: true });
  });
  await page.route("**/latest.json", (r) => r.fulfill({ json: release }));
  await page.route("**/timetable-display.bin", (r) => r.fulfill({ body: corrupt ? Buffer.alloc(1e5) : firmware, contentType: "application/octet-stream" }));
  await page.goto("/");
}
test("groups, directions and walking times update a clean responsive preview", async ({ page }, info) => {
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#add-pick").click();
  await expect(page.locator("#pick-dialog")).toBeVisible();
  const route = catalog.routes.findIndex((r) => r.line === "4");
  await page.locator("#route-select").selectOption({ value: String(route) });
  await page.locator("#direction-select").selectOption("0");
  await expect(page.locator("#route-select")).toHaveValue(String(route));
  expect(await page.locator("#route-select option:checked").textContent()).toBe("4");
  await page.locator("#walk-minutes").fill("9");
  await page.getByRole("button", { name: "Save choice" }).click();
  await expect(page.locator("#preview-line")).toHaveText("4");
  await expect(page.locator("#preview-leave")).toHaveText("Leave in 2 min");
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
});
test("history restoration preserves group controls and USB settings while clearing credentials", async ({ page }) => {
  await prepare(page);
  await expect(page.locator("#prepare")).toBeEnabled();
  await page.locator("#group-name").fill("Commute");
  await page.locator("#scheduled").check();
  await page.locator("#start-time").fill("09:15");
  await page.locator("#end-time").fill("12:30");
  await page.locator("#wifi-ssid").fill("before-history-ssid");
  await page.locator("#wifi-password").fill("before-history-password");
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
