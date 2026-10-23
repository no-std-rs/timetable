import test from "node:test";
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { HostIdentity, ADB_KEY_STORAGE } from "../android-key.js";
function storage() { const values = new Map(); return { getItem: k => values.get(k) ?? null, setItem: (k,v) => values.set(k,v), removeItem: k => values.delete(k) }; }
test("ADB identity is shared by concurrent connects, reconnects and page reloads", async () => {
  const store = storage(), identity = new HostIdentity({ storage: () => store, crypto: webcrypto });
  const [a,b] = await Promise.all([identity.get(),identity.get()]);
  assert.equal(a,b); assert.equal(a.persistent,true); assert.equal(store.getItem(ADB_KEY_STORAGE),a.pem);
  assert.equal((await identity.get()).pem,a.pem);
  const reload = await new HostIdentity({ storage: () => store, crypto: webcrypto }).get();
  assert.equal(reload.pem,a.pem);
  identity.forget(); assert.equal(store.getItem(ADB_KEY_STORAGE),null);
  assert.notEqual((await identity.get()).pem,a.pem);
});
test("corrupt saved keys fail visibly instead of silently rotating authorization", async () => {
  const store = storage(); store.setItem(ADB_KEY_STORAGE,"broken credential");
  const identity = new HostIdentity({ storage: () => store, crypto: webcrypto });
  await assert.rejects(identity.get(),/saved ADB key is invalid/);
  assert.equal(store.getItem(ADB_KEY_STORAGE),"broken credential");
  identity.forget(); assert.equal((await identity.get()).persistent,true);
});
test("blocked storage keeps one session identity and reports it as temporary", async () => {
  const identity = new HostIdentity({ storage: () => { throw new Error("blocked"); }, crypto: webcrypto });
  const first = await identity.get(); assert.equal(first.persistent,false);
  assert.equal((await identity.get()).pem,first.pem);
  assert.throws(() => identity.forget(),/Could not remove/);
});
