import { HostIdentity } from "./android-key.js";
import { validateConfig } from "./config.js";
import { validFeedId } from "./feed-directory.js";
import { zoneBytes } from "./feed-bundle.js";
import { t } from "./i18n.js";

// One groups editor feeds both targets. Android uses its existing OS network.
export function tabletSettings(source, options, now = Date.now()) {
  if (!source.timezone || !validFeedId(source.feed_id)) throw new Error("Choose a transport network first.");
  const { groups, automatic } = validateConfig({ wifi_ssid: "Android", wifi_password: "", groups: structuredClone(source.groups), automatic: source.automatic });
  if (!groups.some(g => g.picks.length)) throw new Error("Add a line and stop first.");
  if (!Number.isInteger(options.idle_seconds) || options.idle_seconds < 30 || options.idle_seconds > 3600) throw new Error("Idle time must be between 30 and 3600 seconds.");
  for (const key of ["keep_awake", "camera_motion", "launch_on_boot"]) if (typeof options[key] !== "boolean") throw new Error("Invalid tablet options.");
  if (options.camera_motion && !options.keep_awake) throw new Error("Keep the screen on to use motion dimming.");
  return { version: 1, feed_id: source.feed_id, zone: [...zoneBytes(source.timezone, now)], automatic, groups, ...options };
}
export function tabletWifi(source) {
  validateConfig({ wifi_ssid: source.wifi_ssid, wifi_password: source.wifi_password, automatic: source.automatic, groups: source.groups });
  if (/\p{Cc}/u.test(source.wifi_ssid + source.wifi_password)) throw new Error("Check the Wi-Fi name and password");
  return { ssid: source.wifi_ssid, password: source.wifi_password };
}
export function checkAndroidRelease(value) {
  if (value?.version !== 1 || value.package !== "org.nostdrs.timetable" || value.min_sdk !== 21 || value.file !== "timetable.apk" || !Number.isInteger(value.size) || value.size < 1024 || value.size > 32 * 1024 * 1024 || !/^[a-f0-9]{64}$/.test(value.sha256) || !/^[a-f0-9]{64}$/.test(value.certificate_sha256) || !Array.isArray(value.abis) || !["armeabi-v7a", "arm64-v8a"].every(abi => value.abis.includes(abi)) || !["development", "provided"].includes(value.signing)) throw new Error("Invalid Android download manifest.");
  return value;
}
export function supportsTablet(caps) { return Number.isInteger(caps?.sdk) && caps.sdk >= 21 && ["armeabi-v7a", "arm64-v8a"].includes(String(caps.abi).trim()); }
export class AndroidInstaller {
  constructor(panel, source) {
    this.panel = panel; this.source = source; this.busy = false; this.client = null; this.raw = null; this.cancelled = false; this.stopping = false; this.closing = null;
    this.identity = new HostIdentity(); this.appSelection = new Set(); this.appReport = null; this.diagnostics = null; this.ownerReport = null; this.dangerous = false;
    this.keyStatus = "The ADB host key is saved only in this browser and is never uploaded.";
    this.$ = id => panel.querySelector(`#android-${id}`);
    this.message("Open this section to prepare Android setup.");
    panel.addEventListener("toggle", () => { if (panel.open) void this.load(); });
    this.$("connect").addEventListener("click", () => void this.connect());
    this.$("disconnect").addEventListener("click", () => void this.stop());
    this.$("forget-key").addEventListener("click", () => this.forgetKey());
    this.$("dangerous").checked = false;
    this.$("dangerous").addEventListener("change", () => {
      this.dangerous = this.$("dangerous").checked;
      if (!this.dangerous) this.$("root").checked = false;
      this.controls();
    });
    this.$("policy").addEventListener("click", () => void this.readOwnership());
    this.$("take-owner").addEventListener("click", () => void this.takeOwnership());
    this.$("run-command").addEventListener("click", () => void this.runCommand());
    this.$("command").addEventListener("input", () => this.controls());
    this.$("battery").addEventListener("click", () => void this.readBattery());
    this.$("save-report").addEventListener("click", () => this.saveReport());
    this.$("apps").addEventListener("click", () => void this.readApps());
    this.$("disable-apps").addEventListener("click", () => void this.changeApps(false));
    this.$("enable-apps").addEventListener("click", () => void this.changeApps(true));
    this.$("show-system").addEventListener("change", () => { this.appSelection.clear(); this.renderApps(); });
    this.$("charge-apply").addEventListener("click", () => void this.changeLimit());
    this.$("install").addEventListener("click", () => void this.install());
    document.addEventListener("languagechange", () => { this.message(this.messageKey, this.messageValues, this.messageError); this.setKeyStatus(this.keyStatus); this.renderApps(); });
    navigator.usb?.addEventListener("disconnect", event => { if (event.device === this.raw) void this.stop(); });
  }
  message(key, values = {}, error = false) {
    this.messageKey = key; this.messageValues = values; this.messageError = error;
    this.$("status").textContent = t(key, values); this.$("status").classList.toggle("error", error);
  }
  controls() {
    const closing = this.stopping || !!this.closing;
    this.$("connect").disabled = !this.module || this.busy || !!this.client || closing;
    this.$("install").disabled = this.busy || closing || !this.client || !supportsTablet(this.caps);
    this.$("disconnect").disabled = !this.raw || closing;
    this.$("forget-key").disabled = this.busy || closing || !!this.client;
    for (const id of ["battery", "apps"]) this.$(id).disabled = this.busy || closing || !this.client;
    this.$("policy").disabled = this.busy || closing || !this.client;
    const commandsLocked = !this.dangerous || this.busy || closing || !this.client;
    this.$("danger-controls").hidden = !this.dangerous;
    this.$("existing-owner").disabled = commandsLocked || !this.ownerReport;
    this.$("take-owner").disabled = commandsLocked || !this.ownerReport;
    this.$("command").disabled = commandsLocked;
    this.$("run-command").disabled = commandsLocked || !this.$("command").value.trim();
    this.$("save-report").disabled = !this.diagnostics;
    this.$("disable-apps").disabled = this.busy || closing || !this.client || !this.appReport?.apps.some(a => this.appSelection.has(a.package) && a.enabled && !a.blocked);
    this.$("enable-apps").disabled = this.busy || closing || !this.client || !this.appReport?.apps.some(a => this.appSelection.has(a.package) && !a.enabled);
    this.$("charge-apply").disabled = this.busy || closing || !this.client || !this.diagnostics?.limits?.some(a => a.writable);
    this.$("charge-node").disabled = this.busy || closing || !this.client;
    for (const input of this.panel.querySelectorAll("input")) input.disabled = this.busy || closing || input.dataset.protected === "true";
    this.$("dangerous").disabled = this.busy || closing || !this.client;
    this.$("root").disabled = commandsLocked;
  }
  async load() {
    if (this.module || this.loading) return;
    if (!navigator.usb || !window.isSecureContext) { this.message("Android USB setup needs desktop Chrome or Edge over HTTPS or localhost."); return; }
    this.loading = true; this.message("Preparing Android USB setup…");
    try {
      const module = await import("./android-adb/timetable_web_adb.js?v=3"); await module.default({ module_or_path: new URL("./android-adb/timetable_web_adb_bg.wasm?v=3", import.meta.url) });
      const response = await fetch("./android/latest.json"); if (!response.ok) throw new Error("Android build is unavailable.");
      this.release = checkAndroidRelease(await response.json()); this.module = module;
      this.$("download").href = `./android/${this.release.file}`; this.$("download").hidden = false;
      this.$("signing").hidden = this.release.signing !== "development";
      this.message("Enable USB debugging, connect the tablet, then approve the computer on its screen.");
    } catch (e) { this.message("Could not prepare Android setup: {reason}", { reason: String(e.message ?? e) }, true); }
    finally { this.loading = false; this.controls(); }
  }
  async connect() {
    if (!this.module || this.busy || this.client || this.stopping || this.closing) return;
    // Browser-restored form values must not opt a new USB connection into commands.
    this.dangerous = false; this.$("dangerous").checked = false; this.$("root").checked = false;
    // Call the picker before the first await to retain the click's activation.
    let selection; try { selection = this.module.request(); } catch(e) { this.message(String(e.message ?? e), {}, true); return; }
    this.busy = true; this.cancelled = false; this.controls(); this.message("Approve USB debugging on the tablet…");
    try {
      this.raw = await selection; this.controls();
      const { pem, persistent } = await this.identity.get(); if (this.cancelled) return;
      this.setKeyStatus(persistent ? "ADB key saved in this browser. Reconnects use the same computer identity." : "Browser storage is unavailable. The ADB key lasts only for this page session.");
      this.client = await this.module.Client.connect(this.raw, pem);
      if (this.cancelled) return;
      this.caps = JSON.parse(await this.client.inspect());
      this.message(supportsTablet(this.caps) ? "Connected: {model} · Android API {sdk}" : "This tablet needs Android 5.0 or newer and an ARM processor.", { model: this.caps.model.trim(), sdk: this.caps.sdk });
    } catch(e) { if (!this.cancelled) this.message("USB connection failed: {reason}", { reason: String(e.message ?? e) }, true); this.cancelled = true; }
    finally { this.busy = false; if (this.cancelled) await this.close(); this.controls(); }
  }
  forgetKey() {
    if (this.busy || this.client || this.stopping || this.closing) return;
    try {
      this.identity.forget();
      this.setKeyStatus("ADB key forgotten. The tablet will ask to authorize this browser again.");
    } catch (e) { this.message(e.message, {}, true); }
  }
  async install() {
    if (this.busy || this.stopping || this.closing || !this.client || !supportsTablet(this.caps)) return;
    let settings, home, owner, wifi;
    try {
      const source = this.source();
      if (this.$("wifi").checked) wifi = tabletWifi(source);
      settings = tabletSettings(source, { keep_awake: this.$("awake").checked, camera_motion: this.$("motion").checked, idle_seconds: Number(this.$("idle").value), launch_on_boot: this.$("boot").checked });
      home = this.$("home").checked; owner = this.$("owner").checked;
    } catch(e) { this.message(e.message, {}, true); return; }
    this.busy = true; this.cancelled = false; this.controls();
    try {
      this.message("Downloading the Android app…");
      this.download = new AbortController();
      const response = await fetch(`./android/${this.release.file}`, { signal: this.download.signal });
      if (!response.ok) throw new Error("Android download failed.");
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (bytes.length !== this.release.size) throw new Error("Android download size mismatch.");
      if (this.cancelled) return;
      const result = JSON.parse(await this.client.provision(bytes, this.release.sha256, new TextEncoder().encode(JSON.stringify(settings)), home, owner, percent => this.message("Installing on the tablet… {percent}%", { percent })));
      if (this.cancelled) return;
      if (wifi) {
        this.message("Configuring tablet Wi-Fi…");
        await this.client.configure_wifi(wifi.ssid, wifi.password);
      }
      if (this.cancelled) return;
      this.$("wifi-status").textContent = wifi ? t("Wi-Fi settings sent. Check the connection on the tablet; saving a network does not confirm internet access.") : "";
      if (result.owner_error) {
        this.$("adb").open = true;
        this.$("command-output").hidden = false;
        this.$("command-output").textContent = `${result.owner_error}\n\n${result.owner_report ?? ""}`;
        this.message("App installed and configured. Device ownership is blocked: {reason}. Inspect the current owner in Advanced ADB controls.", { reason: result.owner_error }, true);
      } else this.message(result.home === "choose_on_tablet" ? "Installed and configured. Choose Timetable as Home on the tablet if its firmware allows it." : "Installed and configured. The timetable is ready on your tablet.");
    } catch(e) { if (!this.cancelled) this.message("Setup stopped: {reason}", { reason: String(e.message ?? e) }, true); }
    finally { this.download = null; this.busy = false; if (this.cancelled) await this.close(); this.controls(); }
  }
  setKeyStatus(key) { this.keyStatus = key; this.$("key-status").textContent = t(key); }
  async deviceTask(work) {
    if (this.busy || this.stopping || this.closing || !this.client) return;
    this.busy = true; this.cancelled = false; this.controls();
    try { await work(); }
    catch (e) { if (!this.cancelled) this.message("Device action failed: {reason}", { reason: String(e.message ?? e) }, true); }
    finally { this.busy = false; if (this.cancelled) await this.close(); this.controls(); }
  }
  async readOwnership() {
    await this.deviceTask(async () => {
      this.ownerReport = null;
      const report = await this.client.ownership_report();
      if (this.cancelled) return;
      this.ownerReport = JSON.parse(report);
      this.$("command-output").hidden = false;
      this.$("command-output").textContent = this.ownerReport.report;
      const owners = this.ownerReport.owners.filter(owner => owner.user === 0);
      this.$("existing-owner").replaceChildren(...(owners.length ? owners : [{component:"",kind:t("No owner on main user")}]).map(owner => {
        const option = document.createElement("option"); option.value = owner.component;
        option.textContent = owner.component ? `${owner.kind} · ${owner.component} · user 0` : owner.kind;
        return option;
      }));
      this.message("Device ownership report ready.");
    });
  }
  async takeOwnership() {
    if (!this.dangerous || !this.ownerReport) return;
    const previous = this.$("existing-owner").value, root = this.$("root").checked;
    await this.deviceTask(async () => {
      // Clear the snapshot even on refusal; a partial takeover may have removed the old owner.
      this.ownerReport = null;
      this.$("command-output").hidden = false;
      this.$("command-output").textContent = "";
      try {
        const output = await this.client.take_ownership(previous, root);
        if (this.cancelled) return;
        this.$("command-output").textContent = output;
        this.message("Timetable is now the device owner.");
      } catch (e) {
        if (!this.cancelled) this.$("command-output").textContent = String(e.message ?? e);
        throw e;
      }
    });
  }
  async runCommand() {
    if (!this.dangerous) return;
    const command = this.$("command").value, root = this.$("root").checked;
    if (!command.trim() || command.length > 4096 || command.includes("\0")) return;
    await this.deviceTask(async () => {
      this.ownerReport = null;
      this.$("command-output").hidden = false;
      this.$("command-output").textContent = "";
      this.message("Running ADB command…");
      try {
        const output = await this.client.shell(command, root);
        if (this.cancelled) return;
        this.$("command-output").textContent = output;
        this.message("ADB command completed.");
      } catch (e) {
        if (!this.cancelled) this.$("command-output").textContent = String(e.message ?? e);
        throw e;
      }
    });
  }
  async readBattery() {
    await this.deviceTask(async () => {
      this.message("Reading battery diagnostics…");
      const report = await this.client.battery_report(); if (this.cancelled) return;
      this.diagnostics = JSON.parse(report);
      this.showBattery(); this.message("Battery diagnostics ready. Nothing was simulated.");
    });
  }
  showBattery() {
    const data = this.diagnostics;
    this.$("battery-output").hidden = false;
    this.$("battery-output").textContent = `${data.identity}\n\n${t("Android battery status")}\n${data.battery}\n\n${t("Power and wake locks")}\n${data.power}\n\n${t("Hardware charging nodes")}\n${data.charging}`;
    const limits = data.limits.filter(a => a.writable);
    this.$("charge-controls").hidden = !limits.length;
    this.$("charge-node").replaceChildren(...limits.map(limit => { const option = document.createElement("option"); option.value = limit.path; option.textContent = `${limit.path} · ${limit.value}%`; return option; }));
    this.$("charge-status").textContent = t(limits.length ? "A real writable charging threshold is available. Android will report the actual value after setting it." : "No writable standard charging threshold was found. Vendor support, root access or an external controller may be needed.");
  }
  async changeLimit() {
    const path = this.$("charge-node").value, value = Number(this.$("charge-percent").value);
    if (!Number.isInteger(value) || value < 50 || value > 100) { this.message("Choose a charging threshold between 50% and 100%.", {}, true); return; }
    await this.deviceTask(async () => {
      const actual = await this.client.set_charge_limit(path, value); if (this.cancelled) return;
      this.$("charge-status").textContent = t("Requested {requested}%; Android reports an actual threshold of {actual}%.", { requested: value, actual });
      this.message("Charging threshold checked on the tablet.");
    });
  }
  saveReport() {
    if (!this.diagnostics) return;
    const blob = new Blob([JSON.stringify({ model: this.caps?.model, sdk: this.caps?.sdk, recorded_at: new Date().toISOString(), diagnostics: this.diagnostics }, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob), link = document.createElement("a"); link.href = url; link.download = "timetable-battery-diagnostics.json"; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  async readApps() {
    await this.deviceTask(async () => {
      this.message("Reading installed apps…");
      const report = await this.client.apps(); if (this.cancelled) return;
      this.appReport = JSON.parse(report); this.appSelection.clear(); this.renderApps();
      this.message("Choose individual apps to disable or re-enable.");
    });
  }
  renderApps() {
    const list = this.$("app-list"); list.replaceChildren();
    for (const app of this.appReport?.apps ?? []) {
      if (app.system && !this.$("show-system").checked) continue;
      const row = document.createElement("label"), box = document.createElement("input"), text = document.createElement("span"), detail = document.createElement("small");
      box.type = "checkbox"; box.dataset.package = app.package; box.dataset.protected = String(app.blocked && app.enabled); box.disabled = app.blocked && app.enabled || this.busy; box.checked = this.appSelection.has(app.package);
      box.addEventListener("change", () => { if (box.checked) this.appSelection.add(app.package); else this.appSelection.delete(app.package); this.controls(); });
      text.textContent = app.label; detail.textContent = `${app.package} · ${t(app.enabled ? "Enabled" : "Disabled")}${app.blocked ? ` · ${t("Protected")}` : ""}`;
      text.append(detail); row.append(box, text); list.append(row);
    }
    this.controls();
  }
  async changeApps(enabled) {
    const chosen = this.appReport?.apps.filter(app => this.appSelection.has(app.package) && app.enabled !== enabled && (enabled || !app.blocked)) ?? [];
    if (!chosen.length) return;
    await this.deviceTask(async () => {
      let changed = 0;
      try {
        for (const app of chosen) {
          if (this.cancelled) return;
          await this.client.set_app_enabled(app.package, enabled); if (this.cancelled) return; app.enabled = enabled; changed++;
        }
        const report = await this.client.apps(); if (this.cancelled) return;
        this.appReport = JSON.parse(report); this.appSelection.clear();
        this.message("Updated {count} apps. Their data was kept.", { count: changed });
      } catch (e) { throw new Error(t("Updated {count} apps before stopping: {reason}", { count: changed, reason: String(e.message ?? e) })); }
      finally { this.renderApps(); }
    });
  }
  async stop() {
    if (this.stopping) return;
    this.stopping = true; this.cancelled = true; this.download?.abort(); this.controls();
    try {
      // Closing USB interrupts an outstanding ADB read without borrowing Client twice.
      try { await this.raw?.close(); } catch { /* Already unplugged. */ }
      if (!this.busy) await this.close();
      this.message("Android USB disconnected. An installed app and its saved data are kept.");
    } finally { this.stopping = false; this.controls(); }
  }
  close() {
    if (this.closing) return this.closing;
    const client = this.client;
    this.client = null; this.raw = null; this.caps = null; this.appReport = null; this.appSelection.clear(); this.diagnostics = null;
    this.dangerous = false; this.$("dangerous").checked = false;
    this.ownerReport = null; this.$("existing-owner").replaceChildren();
    this.$("command-output").hidden = true; this.$("command-output").textContent = ""; this.$("root").checked = false;
    this.$("battery-output").hidden = true; this.$("battery-output").textContent = ""; this.$("charge-controls").hidden = true; this.$("charge-status").textContent = ""; this.renderApps();
    // Cancellation and the operation's finally block share one cleanup. Keep
    // the WASM client alive until its asynchronous disconnect releases its borrow.
    this.closing = (async () => {
      try { await client?.disconnect(); } catch { /* Unplugged or cancelled. */ }
      finally { client?.free(); }
    })().finally(() => { this.closing = null; this.controls(); });
    this.controls(); return this.closing;
  }
}
