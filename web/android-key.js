// ADB host identity is a local credential, independent of timetable drafts.
export const ADB_KEY_STORAGE = "timetable.adb.host-key.v1";
const algorithm = { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" };
function decode(pem) {
  if (typeof pem !== "string" || pem.length > 4096 || !/^-----BEGIN PRIVATE KEY-----\r?\n[A-Za-z0-9+/=\r\n]+-----END PRIVATE KEY-----\r?\n?$/.test(pem)) throw new Error("Invalid key");
  return Uint8Array.from(atob(pem.replace(/-----[^\n]+-----/g, "").replace(/\s/g, "")), c => c.charCodeAt(0));
}
async function validate(pem, crypto) {
  const key = await crypto.subtle.importKey("pkcs8", decode(pem), algorithm, false, ["sign"]);
  if (key.algorithm.modulusLength !== 2048) throw new Error("Invalid key size");
}
async function generate(crypto) {
  const key = await crypto.subtle.generateKey({ ...algorithm, modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]) }, true, ["sign", "verify"]);
  const bytes = new Uint8Array(await crypto.subtle.exportKey("pkcs8", key.privateKey));
  const base64 = btoa(String.fromCharCode(...bytes));
  return `-----BEGIN PRIVATE KEY-----\n${base64.match(/.{1,64}/g).join("\n")}\n-----END PRIVATE KEY-----\n`;
}
export class HostIdentity {
  constructor({ storage = () => globalThis.localStorage, crypto = globalThis.crypto } = {}) {
    this.storage = storage; this.crypto = crypto; this.cached = null; this.pending = null;
  }
  get() {
    if (this.cached) return Promise.resolve(this.cached);
    if (this.pending) return this.pending;
    this.pending = this.load().then(value => this.cached = value).finally(() => { this.pending = null; });
    return this.pending;
  }
  async load() {
    let stored;
    try { stored = this.storage()?.getItem(ADB_KEY_STORAGE); } catch { /* Session identity still works with blocked storage. */ }
    if (stored != null) {
      try { await validate(stored, this.crypto); }
      catch { throw new Error("The saved ADB key is invalid. Forget it before connecting again."); }
      return { pem: stored, persistent: true };
    }
    const pem = await generate(this.crypto);
    let persistent = false;
    try { this.storage().setItem(ADB_KEY_STORAGE, pem); persistent = true; } catch { /* Disclose session-only authorization. */ }
    return { pem, persistent };
  }
  forget() {
    if (this.pending) throw new Error("Wait for the USB operation to finish.");
    try { this.storage().removeItem(ADB_KEY_STORAGE); }
    catch { throw new Error("Could not remove the saved ADB key from this browser."); }
    this.cached = null;
  }
}
