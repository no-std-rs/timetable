import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
const saved = JSON.parse(await readFile(new URL("../../display/tests/settings.json", import.meta.url)));
const apk = Buffer.alloc(4096, 42);
const release = { version: 1, package: "org.nostdrs.timetable", min_sdk: 21, abis: ["armeabi-v7a", "arm64-v8a"], file: "timetable.apk", size: apk.length, sha256: createHash("sha256").update(apk).digest("hex"), certificate_sha256: "a".repeat(64), signing: "development" };
async function prepare(page, { sdk = 22, fail = false, pending = false, corrupt = false } = {}) {
  await page.addInitScript(({ saved, sdk, fail, pending }) => {
    localStorage.setItem("timetable.settings.v1", JSON.stringify({ version: 1, active: 0, config: { ...saved, wifi_ssid: "", wifi_password: "" } }));
    Object.defineProperty(navigator, "usb", { value: { addEventListener() {} }, configurable: true });
    window.adbTest = { sdk, fail, pending, calls: [], closed: 0 };
  }, { saved, sdk, fail, pending });
  await page.route("**/android-adb/timetable_web_adb.js", r => r.fulfill({ contentType: "text/javascript", body: `
    export default async function() {}
    export function request() { window.adbTest.pickerActivation = navigator.userActivation.isActive; return Promise.resolve({ async close() { window.adbTest.closed++; window.adbTest.reject?.(new Error('Unplugged')); } }); }
    export class Client {
      static async connect(device, pem) { if (!pem.startsWith('-----BEGIN PRIVATE KEY-----')) throw new Error('missing key'); return new Client(); }
      async inspect() { return JSON.stringify({ model:'Fire 7', sdk:window.adbTest.sdk, abi:'armeabi-v7a\\n' }); }
      async provision(apk, sha, settings, home, owner, progress) {
        window.adbTest.calls.push({ apk:apk.length, sha, settings:JSON.parse(new TextDecoder().decode(settings)), home, owner }); progress(100);
        if(window.adbTest.fail)throw new Error('INSTALL_FAILED_UPDATE_INCOMPATIBLE');
        if(window.adbTest.pending)await new Promise((resolve,reject)=>window.adbTest.reject=reject);
        return JSON.stringify({ installed:true, configured:true, home:home?'choose_on_tablet':'unchanged' });
      }
      async disconnect() {
        window.adbTest.disconnects=(window.adbTest.disconnects||0)+1;
        window.adbTest.activeDisconnects=(window.adbTest.activeDisconnects||0)+1;
        if(window.adbTest.deferDisconnect)await new Promise(resolve=>(window.adbTest.finishDisconnects??=[]).push(resolve));
        window.adbTest.activeDisconnects--;
      }
      free() { window.adbTest.frees=(window.adbTest.frees||0)+1; if(window.adbTest.activeDisconnects)window.adbTest.earlyFree=true; }
    }
  ` }));
  await page.route("**/android/latest.json", r => r.fulfill({ json: release }));
  await page.route("**/android/timetable.apk", r => r.fulfill({ body: corrupt ? Buffer.alloc(1) : apk, contentType: "application/vnd.android.package-archive" }));
  await page.route("**/latest.json", r => r.request().url().includes("/android/") ? r.fallback() : r.fulfill({ status: 404 }));
  await page.goto("/");
  await expect(page.locator("#catalog-status")).toContainText("lines");
  await page.locator("#android-panel > summary").click();
  await expect(page.locator("#android-connect")).toBeEnabled();
  await page.locator("#android-connect").click();
  await expect(page.locator("#android-status")).toContainText(sdk < 21 ? "needs Android" : "Fire 7");
}
test("Android provisioning shares groups without Wi-Fi and keeps privileged options opt-in", async ({ page }) => {
  await prepare(page);
  await expect(page.locator("#android-home")).not.toBeChecked();
  await expect(page.locator("#android-motion")).not.toBeChecked();
  await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("ready on your tablet");
  const result = await page.evaluate(() => window.adbTest);
  expect(result.pickerActivation).toBe(true);
  expect(result.calls[0].settings.groups).toEqual(saved.groups);
  expect(result.calls[0].settings.keep_awake).toBe(true);
  expect(result.calls[0].settings.camera_motion).toBe(false);
  expect(result.calls[0].home).toBe(false); expect(result.calls[0].owner).toBe(false);
  expect(result.calls[0].settings.wifi_ssid).toBeUndefined();
  await page.locator("#android-home").check(); await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("Choose Timetable as Home");
  expect(await page.evaluate(() => window.adbTest.calls[1].home)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
test("old Android is rejected before transfer", async ({ page }) => {
  await prepare(page, { sdk: 19 }); await expect(page.locator("#android-install")).toBeDisabled();
  expect(await page.evaluate(() => window.adbTest.calls)).toHaveLength(0);
});
test("bad downloads and Android update refusals are visible", async ({ page }) => {
  await prepare(page, { corrupt: true }); await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("size mismatch");
  expect(await page.evaluate(() => window.adbTest.calls)).toHaveLength(0);
});
test("stopping USB interrupts a pending install and permits reconnecting", async ({ page }) => {
  await prepare(page, { pending: true }); await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("100%");
  await page.locator("#android-disconnect").click();
  await expect(page.locator("#android-connect")).toBeEnabled();
  await expect(page.locator("#android-install")).toBeDisabled();
  expect(await page.evaluate(() => window.adbTest.closed)).toBe(1);
});
test("a signing mismatch never triggers destructive recovery", async ({ page }) => {
  await prepare(page, { fail: true }); await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("INSTALL_FAILED_UPDATE_INCOMPATIBLE");
  expect(await page.evaluate(() => window.adbTest.calls)).toHaveLength(1);
});

test("repeated disconnects share cleanup and free the client after it finishes", async ({ page }) => {
  await prepare(page);
  await page.evaluate(() => {
    window.adbTest.deferDisconnect = true;
    document.querySelector("#android-disconnect").click();
    document.querySelector("#android-disconnect").click();
  });
  try {
    await expect(page.locator("#android-disconnect")).toBeDisabled();
    await expect.poll(() => page.evaluate(() => window.adbTest.disconnects)).toBe(1);
    await expect(page.locator("#android-connect")).toBeDisabled();
    expect(await page.evaluate(() => window.adbTest.frees || 0)).toBe(0);
  } finally { await page.evaluate(() => window.adbTest.finishDisconnects?.forEach(resolve => resolve())); }
  await expect(page.locator("#android-connect")).toBeEnabled();
  const result = await page.evaluate(() => window.adbTest);
  expect(result.closed).toBe(1); expect(result.disconnects).toBe(1); expect(result.frees).toBe(1);
  expect(result.earlyFree).toBeFalsy();
});
