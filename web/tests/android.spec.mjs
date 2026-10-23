import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
const saved = JSON.parse(await readFile(new URL("../../display/tests/settings.json", import.meta.url)));
const apk = Buffer.alloc(4096, 42);
const release = { version: 1, package: "org.nostdrs.timetable", min_sdk: 21, abis: ["armeabi-v7a", "arm64-v8a"], file: "timetable.apk", size: apk.length, sha256: createHash("sha256").update(apk).digest("hex"), certificate_sha256: "a".repeat(64), signing: "development" };
async function prepare(page, { sdk = 22, fail = false, pending = false, corrupt = false, charging = false } = {}) {
  await page.addInitScript(({ saved, sdk, fail, pending, charging }) => {
    localStorage.setItem("timetable.settings.v1", JSON.stringify({ version: 1, active: 0, config: { ...saved, wifi_ssid: "", wifi_password: "" } }));
    Object.defineProperty(navigator, "usb", { value: { addEventListener() {} }, configurable: true });
    window.adbTest = { sdk, fail, pending, charging, calls: [], keys: [], wifi: [], limits: [], appChanges: [], closed: 0, apps: [
      { package: 'com.example.background', label: 'Background app', enabled: true, system: false, blocked: false, reason: '' },
      { package: 'com.example.other', label: 'Other app', enabled: true, system: false, blocked: false, reason: '' },
      { package: 'com.vendor.store', label: 'Vendor store', enabled: true, system: true, blocked: false, reason: '' },
      { package: 'com.vendor.home', label: 'Vendor Home', enabled: true, system: true, blocked: true, reason: 'Home' },
    ] };
  }, { saved, sdk, fail, pending, charging });
  await page.route("**/android-adb/timetable_web_adb.js*", r => r.fulfill({ contentType: "text/javascript", body: `
    export default async function() {}
    export function request() { window.adbTest.pickerActivation = navigator.userActivation.isActive; return Promise.resolve({ async close() { window.adbTest.closed++; window.adbTest.reject?.(new Error('Unplugged')); } }); }
    export class Client {
      static async connect(device, pem) { if (!pem.startsWith('-----BEGIN PRIVATE KEY-----')) throw new Error('missing key'); window.adbTest.keys.push([...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(pem)))].join(',')); return new Client(); }
      async inspect() { return JSON.stringify({ model:'Fire 7', sdk:window.adbTest.sdk, abi:'armeabi-v7a\\n' }); }
      async provision(apk, sha, settings, home, owner, progress) {
        window.adbTest.calls.push({ apk:apk.length, sha, settings:JSON.parse(new TextDecoder().decode(settings)), home, owner }); progress(100);
        if(window.adbTest.fail)throw new Error('INSTALL_FAILED_UPDATE_INCOMPATIBLE');
        if(window.adbTest.pending)await new Promise((resolve,reject)=>window.adbTest.reject=reject);
        return JSON.stringify({ installed:true, configured:true, home:home?'choose_on_tablet':'unchanged' });
      }
      async configure_wifi(ssid,password) { window.adbTest.wifi.push({ssid,password}); return 'Wi-Fi saved and connection requested.'; }
      async battery_report() { return JSON.stringify({ battery:'level: 79', power:'mWakefulness=Awake', identity:'uid=2000(shell)', charging:'Battery 79', limits:window.adbTest.charging?[{path:'/sys/class/power_supply/battery/charge_control_end_threshold',value:100,writable:true}]:[] }); }
      async set_charge_limit(path,value) { window.adbTest.limits.push({path,value}); return 78; }
      async apps() { return JSON.stringify({user:0,apps:window.adbTest.apps}); }
      async set_app_enabled(name,enabled) {
        const app=window.adbTest.apps.find(a=>a.package===name); if(!app || !enabled && app.blocked)throw new Error('Protected application');
        if(window.adbTest.failApp===name)throw new Error('Android refused this app');
        app.enabled=enabled;window.adbTest.appChanges.push({name,enabled});
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

test("real WebUSB WASM connects through repeated terminal closes during inspect", async ({ page }) => {
  const { installFakeADB } = await import("./fake-adb-usb.mjs");
  await page.addInitScript(installFakeADB);
  await page.route("**/android/latest.json", r => r.fulfill({ json: release }));
  await page.goto("/");
  await page.locator("#android-panel > summary").click();
  await expect(page.locator("#android-connect")).toBeEnabled();
  await page.locator("#android-connect").click();
  await expect(page.locator("#android-status")).toContainText("Connected: Fire 7");
  const result = await page.evaluate(() => window.usbPeer);
  expect(result.services).toHaveLength(3); expect(result.duplicates).toBe(3);
  await page.locator("#android-disconnect").click();
  await expect(page.locator("#android-connect")).toBeEnabled();
});

test("ADB host identity survives disconnect and reload until explicitly forgotten", async ({ page }) => {
  await prepare(page);
  const first=await page.evaluate(()=>window.adbTest.keys[0]);
  await page.locator("#android-disconnect").click(); await expect(page.locator("#android-connect")).toBeEnabled();
  await page.locator("#android-connect").click();await expect(page.locator("#android-status")).toContainText("Fire 7");
  expect(await page.evaluate(()=>window.adbTest.keys[1])).toBe(first);
  await page.reload(); await page.locator("#android-panel > summary").click(); await expect(page.locator("#android-connect")).toBeEnabled();
  await page.locator("#android-connect").click(); await expect(page.locator("#android-status")).toContainText("Fire 7");
  expect(await page.evaluate(()=>window.adbTest.keys[0])).toBe(first);
  await page.locator("#android-disconnect").click();await expect(page.locator("#android-forget-key")).toBeEnabled();
  await page.locator("#android-forget-key").click();await page.locator("#android-connect").click();await expect(page.locator("#android-status")).toContainText("Fire 7");
  expect(await page.evaluate(()=>window.adbTest.keys[1])).not.toBe(first);
});

test("Wi-Fi is explicitly opted in and credentials travel only through ADB", async ({ page }) => {
  const requests=[];page.on('request',r=>requests.push(r.url()+(r.postData()??'')));
  await prepare(page);await expect(page.locator('#android-wifi')).not.toBeChecked();
  await page.locator('#wifi-ssid').fill("House's Wi-Fi");await page.locator('#wifi-password').fill('secret-56789');
  await page.locator('#android-wifi').check();await page.locator('#android-install').click();
  await expect(page.locator('#android-wifi-status')).toContainText('Check the connection on the tablet');
  const result=await page.evaluate(()=>window.adbTest);expect(result.wifi).toEqual([{ssid:"House's Wi-Fi",password:'secret-56789'}]);
  expect(result.calls[0].settings.wifi_password).toBeUndefined();expect(requests.join(' ')).not.toContain('secret-56789');
});

test("battery tools distinguish real capability and show actual rounded readback", async ({ page }) => {
  await prepare(page,{charging:true});await page.locator('#android-tools > summary').click();await page.locator('#android-battery').click();
  await expect(page.locator('#android-battery-output')).toContainText('level: 79');await expect(page.locator('#android-charge-controls')).toBeVisible();
  await page.locator('#android-charge-apply').click();await expect(page.locator('#android-charge-status')).toContainText('actual threshold of 78%');
  expect(await page.evaluate(()=>window.adbTest.limits[0].value)).toBe(80);
  await page.locator('#android-disconnect').click();await expect(page.locator('#android-charge-controls')).toBeHidden();
});

test("unsupported battery caps stay unavailable and app selection is reversible and guarded", async ({ page }) => {
  await prepare(page);await page.locator('#android-tools > summary').click();await page.locator('#android-battery').click();
  await expect(page.locator('#android-charge-status')).toContainText('No writable standard');await expect(page.locator('#android-charge-controls')).toBeHidden();
  await page.locator('#android-apps').click();const apps=page.locator('#android-app-list');
  await expect(apps.locator('input:checked')).toHaveCount(0);await expect(apps).not.toContainText('Vendor store');
  await apps.locator('[data-package="com.example.background"]').check();await page.locator('#android-disable-apps').click();
  await expect(page.locator('#android-status')).toContainText('Updated 1 apps');
  await apps.locator('[data-package="com.example.background"]').check();await page.locator('#android-enable-apps').click();await expect(page.locator('#android-status')).toContainText('Updated 1 apps');
  await page.locator('#android-show-system').check();await expect(apps.locator('[data-package="com.vendor.home"]')).toBeDisabled();
  expect(await page.evaluate(()=>window.adbTest.appChanges)).toEqual([{name:'com.example.background',enabled:false},{name:'com.example.background',enabled:true}]);
  await page.evaluate(()=>window.adbTest.failApp='com.example.other');
  await apps.locator('[data-package="com.example.background"]').check();await apps.locator('[data-package="com.example.other"]').check();
  await page.locator('#android-disable-apps').click();await expect(page.locator('#android-status')).toContainText('Updated 1 apps before stopping: Android refused');
});
