import test from "node:test";
import assert from "node:assert/strict";
import { SerialLogs } from "../serial-logs.js";
const turn = () => new Promise(resolve => setImmediate(resolve));
function fixture() {
  let controller; const calls = [];
  const port = { readable: new ReadableStream({ start(c) { controller = c; } }),
    async open(options) { calls.push(["open", options]); }, async close() { calls.push(["close"]); },
    setSignals() { throw Error("logs must not reset the device"); }, get writable() { throw Error("logs must never write"); } };
  return { port, calls, bytes: v => controller.enqueue(v), end: () => controller.close() };
}
test("passive logs decode split UTF-8, retain bounded text and release USB on disconnect", async () => {
  const f = fixture(), logs = new SerialLogs({ requestPort: async () => f.port }, () => {}, 24);
  await logs.connect(); assert.equal(logs.state, "connected");
  const b = new TextEncoder().encode("Łączę Wi-Fi\n"); f.bytes(b.slice(0, 1)); f.bytes(b.slice(1)); await turn();
  assert.equal(logs.text, "Łączę Wi-Fi\n");
  f.bytes(new TextEncoder().encode("a".repeat(40))); await turn(); assert.equal(logs.text, "a".repeat(24));
  await logs.disconnect(); assert.equal(f.port.readable.locked, false);
  assert.deepEqual(f.calls, [["open", { baudRate: 115200, bufferSize: 8192 }], ["close"]]);
  assert.equal(logs.state, "disconnected"); logs.clear(); assert.equal(logs.text, "");
});
test("cancelled chooser, busy port and pending open can be retried without leaked ownership", async () => {
  const cancelled = new SerialLogs({ requestPort: async () => { throw Object.assign(Error(), { name: "NotFoundError" }); } }, () => {});
  await cancelled.connect(); assert.equal(cancelled.state, "idle");
  const busy = new SerialLogs({ requestPort: async () => ({ open: async () => { throw Error("busy"); } }) }, () => {});
  await busy.connect(); assert.equal(busy.state, "error");
  const f = fixture(); let finish;
  f.port.open = async () => new Promise(resolve => { finish = resolve; });
  const logs = new SerialLogs({ requestPort: async () => f.port }, () => {});
  const connecting = logs.connect(); await turn();
  let disconnected = false;
  const closing = logs.disconnect().then(() => { disconnected = true; });
  await turn();
  assert.equal(logs.state, "disconnecting"); assert.equal(disconnected, false);
  await logs.connect(); // Reconnect cannot claim the still-opening port.
  finish(); await Promise.all([connecting, closing]);
  assert.equal(logs.state, "disconnected"); assert.deepEqual(f.calls, [["close"]]); assert.equal(f.port.readable.locked, false);
});
