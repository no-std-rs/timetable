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
        return JSON.stringify({ installed:true, configured:true, home:home?'choose_on_tablet':'unchanged', owner_error:window.adbTest.ownerError, owner_report:'Profile Owner (User 0): com.owner/.Admin' });
      }
      async shell(command,root) {
        (window.adbTest.shells??=[]).push({command,root});
        if(window.adbTest.shellPending)await new Promise((resolve,reject)=>window.adbTest.reject=reject);
        if(window.adbTest.shellError)throw new Error(window.adbTest.shellError);
        return window.adbTest.shellOutput??'uid=2000(shell)';
      }
      async ownership_report() { return JSON.stringify({ can_remove_owner:window.adbTest.sdk>=24, report:'Profile Owner (User 0): admin=ComponentInfo{com.owner/.Admin}', owners:[{kind:'profile',user:0,component:'com.owner/.Admin'},{kind:'profile',user:10,component:'com.work/.Admin'}] }); }
      async take_ownership(previous,root) {
        (window.adbTest.takeovers??=[]).push({previous,root});
        if(window.adbTest.ownerError)throw new Error(window.adbTest.ownerError);
        return 'Device Owner: org.nostdrs.timetable/.AdminReceiver';
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
  await page.locator("#android-display-options > summary").click();
  await page.locator("#android-home").check(); await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("Choose Timetable as Home");
  expect(await page.evaluate(() => window.adbTest.calls[1].home)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
test("unused idle input cannot block setup; motion checks its timer and keep-awake dependency", async ({ page }) => {
  await prepare(page);
  await expect(page.locator("#android-idle")).toBeHidden();
  await page.locator("#android-idle").evaluate(input => { input.value = ""; });
  await page.locator("#android-rotation").selectOption("reverse_landscape");
  await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("ready on your tablet");
  expect(await page.evaluate(() => window.adbTest.calls[0].settings)).toMatchObject({ idle_seconds: 120, camera_motion: false, rotation: "reverse_landscape" });
  await page.locator("#android-display-options > summary").click();
  await page.locator("#android-motion").check();
  await expect(page.locator("#android-idle")).toBeVisible();
  await page.locator("#android-idle").fill("29");
  await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("between 30 and 3600");
  expect(await page.evaluate(() => window.adbTest.calls)).toHaveLength(1);
  await page.locator("#android-idle").fill("30");
  await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("ready on your tablet");
  expect(await page.evaluate(() => window.adbTest.calls[1].settings)).toMatchObject({ idle_seconds: 30, camera_motion: true });
  await page.locator("#android-awake").uncheck();
  await expect(page.locator("#android-motion")).toBeDisabled();
  await expect(page.locator("#android-idle")).toBeHidden();
  await page.locator("#android-install").click();
  await expect(page.locator("#android-status")).toContainText("ready on your tablet");
  expect(await page.evaluate(() => window.adbTest.calls[2].settings)).toMatchObject({ keep_awake: false, camera_motion: false, idle_seconds: 120 });
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
  await page.locator('#android-network-options > summary').click();
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


test("ADB commands require a click, root is explicit, and output is plain text", async ({page}) => {
  await prepare(page);
  await page.locator('#android-adb > summary').click();
  await page.locator('#android-dangerous').check();
  await expect(page.locator('#android-root')).not.toBeChecked();
  expect(await page.evaluate(()=>window.adbTest.shells??[])).toHaveLength(0);
  await page.locator('#android-command').fill('id');
  await page.evaluate(()=>window.adbTest.shellOutput='<img src=x onerror=alert(1)>');
  await page.locator('#android-run-command').click();
  await expect(page.locator('#android-command-output')).toHaveText('<img src=x onerror=alert(1)>');
  await expect(page.locator('#android-command-output img')).toHaveCount(0);
  await page.locator('#android-root').check();
  await page.locator('#android-command').fill('dpm list-owners');
  await page.locator('#android-run-command').click();
  await expect.poll(()=>page.evaluate(()=>window.adbTest.shells.length)).toBe(2);
  expect(await page.evaluate(()=>window.adbTest.shells)).toEqual([{command:'id',root:false},{command:'dpm list-owners',root:true}]);
  await page.locator('#android-disconnect').click();
  await expect(page.locator('#android-root')).not.toBeChecked();
  await expect(page.locator('#android-command-output')).toBeHidden();
});
test("owner refusal keeps installation and exposes the conflicting profile owner", async ({page}) => {
  await prepare(page, {sdk:24});
  await page.evaluate(()=>window.adbTest.ownerError='User already has a profile owner: com.owner/.Admin');
  await page.locator('#android-install').click();
  await expect(page.locator('#android-status')).toContainText('App installed and configured.');
  await expect(page.locator('#android-status')).toContainText('ownership is blocked');
  await expect(page.locator('#android-adb')).toHaveAttribute('open','');
  await expect(page.locator('#android-command-output')).toContainText('com.owner/.Admin');
  expect(await page.evaluate(()=>window.adbTest.takeovers??[])).toHaveLength(0);
  await page.locator('#android-policy').click();
  await expect(page.locator('#android-existing-owner option')).toHaveCount(1);
  await expect(page.locator('#android-existing-owner')).toHaveValue('com.owner/.Admin');
  await expect(page.locator('#android-take-owner')).toBeDisabled();
  await page.locator('#android-dangerous').check();
  await page.locator('#android-take-owner').click();
  await expect(page.locator('#android-status')).toContainText('User already has a profile owner');
  await expect(page.locator('#android-take-owner')).toBeDisabled();
  expect(await page.evaluate(()=>window.adbTest.takeovers)).toEqual([{previous:'com.owner/.Admin',root:false}]);
});
test("stopping a pending shell command closes USB and permits reconnecting", async ({page}) => {
  await prepare(page);
  await page.locator('#android-adb > summary').click();
  await page.locator('#android-dangerous').check();
  await page.evaluate(()=>window.adbTest.shellPending=true);
  await page.locator('#android-command').fill('sleep 120');
  await page.locator('#android-run-command').click();
  await expect(page.locator('#android-status')).toContainText('Running ADB command');
  await expect(page.locator('#android-run-command')).toBeDisabled();
  await page.locator('#android-disconnect').click();
  await expect(page.locator('#android-connect')).toBeEnabled();
  await expect(page.locator('#android-run-command')).toBeDisabled();
});


async function realADB(page, sdk=22) {
  await page.addInitScript(sdk=>window.fakeAdbSdk=sdk,sdk);
  const { installFakeADB } = await import('./fake-adb-usb.mjs');
  await page.addInitScript(installFakeADB);
  await page.route('**/android/latest.json', r=>r.fulfill({json:release}));
  await page.goto('/');
  await page.locator('#android-panel > summary').click();
  await expect(page.locator('#android-connect')).toBeEnabled();
  await page.locator('#android-connect').click();
  await expect(page.locator('#android-status')).toContainText('Fire 7');
  await page.locator('#android-adb > summary').click();
  await page.locator('#android-dangerous').check();
}
test('real WASM executes explicit shell and su commands and reports nonzero exit', async ({page})=>{
  await realADB(page);
  await page.evaluate(()=>window.usbPeer.replies=[{match:'(\nid\n)',body:'uid=2000(shell)'},{match:'su -c',body:'su: permission denied',status:1}]);
  await page.locator('#android-command').fill('id');await page.locator('#android-run-command').click();
  await expect(page.locator('#android-command-output')).toHaveText('uid=2000(shell)');
  await page.locator('#android-root').check();await page.locator('#android-run-command').click();
  await expect(page.locator('#android-command-output')).toContainText('Device command failed (1): su: permission denied');
  const services=await page.evaluate(()=>window.usbPeer.services);
  expect(services.filter(s=>s.includes('su -c'))).toHaveLength(1);
  expect(await page.evaluate(()=>window.usbPeer.replies)).toHaveLength(0);
});
test('real WASM refuses a changed owner and verifies every step of replacement', async ({page})=>{
  await realADB(page,24);
  const profile='Profile Owner (User 0):\n  admin=ComponentInfo{com.owner/.Admin}\n';
  const empty='Device Policy Manager state:\n';
  const final='Device Owner:\n  admin=ComponentInfo{org.nostdrs.timetable/.AdminReceiver}\n  User ID: 0\n';
  await page.evaluate(profile=>window.usbPeer.replies=[{match:'dumpsys device_policy',body:profile},{match:'dumpsys device_policy',body:profile.replace('com.owner/','com.changed/')}],profile);
  await page.locator('#android-policy').click();await expect(page.locator('#android-existing-owner')).toHaveValue('com.owner/.Admin');
  await page.locator('#android-take-owner').click();
  await expect(page.locator('#android-status')).toContainText('selected owner changed');
  expect(await page.evaluate(()=>window.usbPeer.services.some(s=>s.includes('dpm remove')))).toBe(false);
  await page.evaluate(({profile,empty,final})=>window.usbPeer.replies=[
    {match:'dumpsys device_policy',body:profile},
    {match:'dumpsys device_policy',body:profile},
    {match:'dpm set-active-admin --user 0 org.nostdrs.timetable/.AdminReceiver',body:'Success: Active admin set to component org.nostdrs.timetable/.AdminReceiver'},
    {match:'dpm remove-active-admin --user 0',body:'Success: Admin removed'},
    {match:'dumpsys device_policy',body:empty},
    {match:'dumpsys device_policy',body:empty},
    {match:'dpm set-device-owner',body:'Success: Device owner set'},
    {match:'dumpsys device_policy',body:final},
    {match:'am start -W',body:'Status: ok'}
  ],{profile,empty,final});
  await page.locator('#android-policy').click();await expect(page.locator('#android-take-owner')).toBeEnabled();
  await page.locator('#android-take-owner').click();
  await expect(page.locator('#android-status')).toHaveText('Timetable is now the device owner.');
  await expect(page.locator('#android-command-output')).toContainText('org.nostdrs.timetable/.AdminReceiver');
  expect(await page.evaluate(()=>window.usbPeer.replies)).toHaveLength(0);
});
test('real WASM stops when Android refuses production administrator removal',async({page})=>{
  await realADB(page,24);
  await page.evaluate(()=>{
    const body='Profile Owner (User 0):\n admin=ComponentInfo{com.owner/.Admin}\n';
    window.usbPeer.replies=[{match:'dumpsys device_policy',body},{match:'dumpsys device_policy',body},{match:'dpm set-active-admin',body:'Success: Active admin set to component org.nostdrs.timetable/.AdminReceiver'},{match:'dpm remove-active-admin',body:'Attempt to remove non-test admin',status:1}];
  });
  await page.locator('#android-policy').click();await expect(page.locator('#android-take-owner')).toBeEnabled();
  await page.locator('#android-take-owner').click();
  await expect(page.locator('#android-status')).toContainText('Attempt to remove non-test admin');
  expect(await page.evaluate(()=>window.usbPeer.services.some(s=>s.includes('dpm set-device-owner')))).toBe(false);
  expect(await page.evaluate(()=>window.usbPeer.replies)).toHaveLength(0);
});


test('dangerous installer commands require a fresh session opt-in while ownership inspection stays available',async({page})=>{
  await prepare(page);
  await page.locator('#android-adb > summary').click();
  await expect(page.locator('#android-dangerous')).not.toBeChecked();
  await expect(page.locator('#android-danger-controls')).toBeHidden();
  await page.locator('#android-policy').click();
  await expect(page.locator('#android-command-output')).toContainText('Profile Owner');
  await expect(page.locator('#android-take-owner')).toBeDisabled();
  await page.evaluate(()=>{
    document.querySelector('#android-command').value='id';
    document.querySelector('#android-run-command').dispatchEvent(new MouseEvent('click'));
    document.querySelector('#android-take-owner').dispatchEvent(new MouseEvent('click'));
  });
  expect(await page.evaluate(()=>window.adbTest.shells??[])).toHaveLength(0);
  expect(await page.evaluate(()=>window.adbTest.takeovers??[])).toHaveLength(0);
  await page.locator('#android-dangerous').check();
  await expect(page.locator('#android-danger-controls')).toBeVisible();
  await page.locator('#android-root').check();
  await page.locator('#android-dangerous').uncheck();
  await expect(page.locator('#android-root')).not.toBeChecked();
  await expect(page.locator('#android-run-command')).toBeDisabled();
  await page.locator('#android-dangerous').check();
  await page.locator('#android-disconnect').click();
  await expect(page.locator('#android-connect')).toBeEnabled();
  await expect(page.locator('#android-dangerous')).not.toBeChecked();
  // Simulate form values restored by the browser before the next connection.
  await page.evaluate(()=>{
    document.querySelector('#android-dangerous').checked=true;
    document.querySelector('#android-root').checked=true;
  });
  await page.locator('#android-connect').click();
  await expect(page.locator('#android-status')).toContainText('Fire 7');
  await expect(page.locator('#android-dangerous')).not.toBeChecked();
  await expect(page.locator('#android-root')).not.toBeChecked();
  await expect(page.locator('#android-danger-controls')).toBeHidden();
  await expect(page.locator('#android-run-command')).toBeDisabled();
});


for(const sdk of [21,22,23])test(`SDK ${sdk} keeps owner removal unavailable even with root selected`,async({page})=>{
  await realADB(page,sdk);
  await page.evaluate(()=>window.usbPeer.replies=[{match:'dumpsys device_policy',body:'Profile Owner (User 0):\n admin=ComponentInfo{com.owner/.Admin}\n'}]);
  await page.locator('#android-policy').click();
  await expect(page.locator('#android-owner-support')).toContainText('requires Android 7.0');
  await page.locator('#android-root').check();
  await expect(page.locator('#android-take-owner')).toBeDisabled();
  await page.evaluate(()=>document.querySelector('#android-take-owner').dispatchEvent(new MouseEvent('click')));
  expect(await page.evaluate(()=>window.usbPeer.services.some(s=>s.includes('dpm ')))).toBe(false);
  await expect(page.locator('#android-command')).toBeEnabled();
});
for(const status of [0,1])test(`unavailable Timetable admin leaves previous owner intact (exit ${status})`,async({page})=>{
  await realADB(page,24);
  await page.evaluate(status=>{
    const body='Profile Owner (User 0):\n admin=ComponentInfo{com.owner/.Admin}\n';
    window.usbPeer.replies=[{match:'dumpsys device_policy',body},{match:'dumpsys device_policy',body},{match:'dpm set-active-admin --user 0 org.nostdrs.timetable/.AdminReceiver',body:'Error: Unknown admin',status}];
  },status);
  await page.locator('#android-policy').click();await expect(page.locator('#android-take-owner')).toBeEnabled();
  await page.locator('#android-take-owner').click();
  await expect(page.locator('#android-status')).toContainText(status ? 'Install the current Timetable app' : 'previous owner was kept');
  expect(await page.evaluate(()=>window.usbPeer.services.some(s=>s.includes('dpm remove-active-admin')||s.includes('dpm set-device-owner')))).toBe(false);
  expect(await page.evaluate(()=>window.usbPeer.replies)).toHaveLength(0);
});
