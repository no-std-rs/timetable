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
export function checkAndroidRelease(value) {
  if (value?.version !== 1 || value.package !== "org.nostdrs.timetable" || value.min_sdk !== 21 || value.file !== "timetable.apk" || !Number.isInteger(value.size) || value.size < 1024 || value.size > 32 * 1024 * 1024 || !/^[a-f0-9]{64}$/.test(value.sha256) || !/^[a-f0-9]{64}$/.test(value.certificate_sha256) || !Array.isArray(value.abis) || !["armeabi-v7a", "arm64-v8a"].every(abi => value.abis.includes(abi)) || !["development", "provided"].includes(value.signing)) throw new Error("Invalid Android download manifest.");
  return value;
}
export function supportsTablet(caps) { return Number.isInteger(caps?.sdk) && caps.sdk >= 21 && ["armeabi-v7a", "arm64-v8a"].includes(String(caps.abi).trim()); }
async function identity() {
  const key = await crypto.subtle.generateKey({ name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" }, true, ["sign", "verify"]);
  const bytes = new Uint8Array(await crypto.subtle.exportKey("pkcs8", key.privateKey));
  const base64 = btoa(String.fromCharCode(...bytes));
  return `-----BEGIN PRIVATE KEY-----\n${base64.match(/.{1,64}/g).join("\n")}\n-----END PRIVATE KEY-----\n`;
}
export class AndroidInstaller {
  constructor(panel, source) {
    this.panel = panel; this.source = source; this.busy = false; this.client = null; this.raw = null; this.cancelled = false;
    this.$ = id => panel.querySelector(`#android-${id}`);
    this.message("Open this section to prepare Android setup.");
    panel.addEventListener("toggle", () => { if (panel.open) void this.load(); });
    this.$("connect").addEventListener("click", () => void this.connect());
    this.$("disconnect").addEventListener("click", () => void this.stop());
    this.$("install").addEventListener("click", () => void this.install());
    document.addEventListener("languagechange", () => this.message(this.messageKey, this.messageValues, this.messageError));
    navigator.usb?.addEventListener("disconnect", event => { if (event.device === this.raw) void this.stop(); });
  }
  message(key, values = {}, error = false) {
    this.messageKey = key; this.messageValues = values; this.messageError = error;
    this.$("status").textContent = t(key, values); this.$("status").classList.toggle("error", error);
  }
  controls() {
    this.$("connect").disabled = !this.module || this.busy || !!this.client;
    this.$("install").disabled = this.busy || !this.client || !supportsTablet(this.caps);
    this.$("disconnect").disabled = !this.raw;
    for (const input of this.panel.querySelectorAll("input")) input.disabled = this.busy;
  }
  async load() {
    if (this.module || this.loading) return;
    if (!navigator.usb || !window.isSecureContext) { this.message("Android USB setup needs desktop Chrome or Edge over HTTPS or localhost."); return; }
    this.loading = true; this.message("Preparing Android USB setup…");
    try {
      const module = await import("./android-adb/timetable_web_adb.js"); await module.default();
      const response = await fetch("./android/latest.json"); if (!response.ok) throw new Error("Android build is unavailable.");
      this.release = checkAndroidRelease(await response.json()); this.module = module;
      this.$("download").href = `./android/${this.release.file}`; this.$("download").hidden = false;
      this.$("signing").hidden = this.release.signing !== "development";
      this.message("Enable USB debugging, connect the tablet, then approve the computer on its screen.");
    } catch (e) { this.message("Could not prepare Android setup: {reason}", { reason: String(e.message ?? e) }, true); }
    finally { this.loading = false; this.controls(); }
  }
  async connect() {
    if (!this.module || this.busy || this.client) return;
    // Call the picker before the first await to retain the click's activation.
    let selection; try { selection = this.module.request(); } catch(e) { this.message(String(e.message ?? e), {}, true); return; }
    this.busy = true; this.cancelled = false; this.controls(); this.message("Approve USB debugging on the tablet…");
    try {
      this.raw = await selection; this.controls();
      const pem = await identity(); if (this.cancelled) return;
      this.client = await this.module.Client.connect(this.raw, pem);
      if (this.cancelled) return;
      this.caps = JSON.parse(await this.client.inspect());
      this.message(supportsTablet(this.caps) ? "Connected: {model} · Android API {sdk}" : "This tablet needs Android 5.0 or newer and an ARM processor.", { model: this.caps.model.trim(), sdk: this.caps.sdk });
    } catch(e) { if (!this.cancelled) this.message("USB connection failed: {reason}", { reason: String(e.message ?? e) }, true); this.cancelled = true; }
    finally { this.busy = false; if (this.cancelled) await this.close(); this.controls(); }
  }
  async install() {
    if (this.busy || !this.client || !supportsTablet(this.caps)) return;
    let settings, home, owner;
    try {
      settings = tabletSettings(this.source(), { keep_awake: this.$("awake").checked, camera_motion: this.$("motion").checked, idle_seconds: Number(this.$("idle").value), launch_on_boot: this.$("boot").checked });
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
      this.message(result.home === "choose_on_tablet" ? "Installed and configured. Choose Timetable as Home on the tablet if its firmware allows it." : "Installed and configured. The timetable is ready on your tablet.");
    } catch(e) { if (!this.cancelled) this.message("Setup stopped: {reason}", { reason: String(e.message ?? e) }, true); }
    finally { this.download = null; this.busy = false; if (this.cancelled) await this.close(); this.controls(); }
  }
  async stop() {
    this.cancelled = true; this.download?.abort();
    // Closing USB interrupts an outstanding ADB read without borrowing Client twice.
    try { await this.raw?.close(); } catch { /* Already unplugged. */ }
    if (!this.busy) await this.close();
    this.message("Android USB disconnected. An installed app and its saved data are kept."); this.controls();
  }
  async close() {
    try { await this.client?.disconnect(); } catch { /* Unplugged or cancelled. */ }
    this.client?.free(); this.client = null; this.raw = null; this.caps = null;
  }
}
