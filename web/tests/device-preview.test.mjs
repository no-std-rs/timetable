import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
const root = new URL("../../", import.meta.url);
const bytes = await readFile(new URL("../dist/device-preview.wasm", import.meta.url));
const module = new WebAssembly.Module(bytes);
const fixture = JSON.parse(await readFile(new URL("../../display/tests/settings.json", import.meta.url)));
const now = 1792301400n;
function load() {
  const api = new WebAssembly.Instance(module, {}).exports;
  const configure = value => {
    const json = new TextEncoder().encode(JSON.stringify(value));
    new Uint8Array(api.memory.buffer, api.preview_input(), json.length).set(json);
    return api.preview_configure(json.length, now, 0);
  };
  const snapshot = () => JSON.parse(new TextDecoder().decode(new Uint8Array(api.memory.buffer, api.preview_output(), api.preview_snapshot())));
  assert.equal(configure(fixture), 1);
  return { api, configure, snapshot };
}
test("actual WASM pixels match native Rust for clock/network states and saved/cancelled device edits", () => {
  const reference = JSON.parse(execFileSync("cargo", ["run", "-q", "-p", "timetable-web-preview", "--locked", "--release", "--example", "reference"], { cwd: root, encoding: "utf8" }));
  const {api, snapshot} = load();
  for(const expected of reference) {
    const step=expected.step;
    if(step>=1 && step<=5) api.preview_mode(step,now);
    if(step===6) { api.preview_mode(0,now); api.preview_gesture(120,120,0,0,1); }
    if(step===7) api.preview_gesture(120,65,0,0,0);
    if(step===8) api.preview_gesture(180,80,0,0,0);
    if(step===9) assert.equal(api.preview_gesture(180,200,0,0,0),2);
    if(step===10) api.preview_gesture(120,212,0,0,0);
    if(step===11) api.preview_gesture(120,120,0,0,0);
    if(step===12) api.preview_gesture(120,24,0,0,0);
    if(step===13) api.preview_gesture(120,65,0,0,0);
    if(step===14) api.preview_gesture(120,120,0,0,1);
    if(step===15) api.preview_gesture(120,100,0,0,0);
    if(step===16 || step===17) api.preview_gesture(120,70,0,0,0);
    if(step===18) api.preview_gesture(60,45,0,0,0);
    if(step>=19 && step<=21) api.preview_gesture(45,24,0,0,0);
    assert.equal(api.preview_render(now,step%2),1);
    const rgba = new Uint8Array(api.memory.buffer,api.preview_frame(),240*240*4);
    let hash = 0xcbf29ce484222325n;
    for(const byte of rgba) hash=BigInt.asUintN(64,(hash^BigInt(byte))*0x100000001b3n);
    assert.equal(hash.toString(16).padStart(16,"0"),expected.hash,`step ${step}`);
    assert.equal(snapshot().screen,expected.screen);
  }
  assert.equal(snapshot().config.groups[0].picks[0].walk,7);
});
test("WASM ABI rejects malformed updates atomically and uses fixed private memory without host imports", () => {
  assert.deepEqual(WebAssembly.Module.imports(module),[]);
  const {api,configure,snapshot}=load(), original=snapshot();
  assert.equal(api.memory.buffer.byteLength,4194304);
  assert.throws(()=>api.memory.grow(1),RangeError);
  assert.equal(api.preview_configure(api.preview_capacity()+1,now,0),0);
  assert.equal(configure({...fixture,groups:[]}),0);
  assert.deepEqual(snapshot(),original);
  assert.equal(api.preview_mode(999,now),0);
  assert.equal(api.preview_mode(0,2n**63n-1n),0);
  assert.equal(api.preview_gesture(100,999,0,0,1),0);
  assert.deepEqual(snapshot(),original);
  for (const invalid of [2n**63n-1n, -(2n**63n), 253402300800n]) {
    assert.doesNotThrow(()=>api.preview_refresh(invalid));
    assert.deepEqual(snapshot(),original);
  }
  api.preview_mode(1,now);
  api.preview_render(now+3600n,0);
  assert.equal(snapshot().clock_known,false);
  assert.equal(snapshot().next.chance,null);
});

test("escaped names roundtrip exactly and maximal settings plus a draft fit bounded output", () => {
  const {api,configure,snapshot}=load();
  for(const name of ['Home "north"', 'Home\\north', '"'.repeat(24)]) {
    const value=structuredClone(fixture); value.groups[0].name=name;
    assert.equal(configure(value),1); assert.equal(snapshot().config.groups[0].name,name);
  }
  const pick={route:'R'.repeat(32),line:'L'.repeat(8),stop:'S'.repeat(32),stop_name:'"'.repeat(48)+'N'.repeat(16),stop_code:'C'.repeat(16),headsign:'"'.repeat(48)+'H'.repeat(16),direction:1,walk:60,spread:15,margin:10};
  const config={wifi_ssid:'Browser demo',wifi_password:'',automatic:false,groups:Array.from({length:4},()=>({name:'G'.repeat(24),days:127,start:1439,end:1438,picks:Array.from({length:4},()=>({...pick}))}))};
  assert.ok(new TextEncoder().encode(JSON.stringify(config)).length<=api.preview_capacity());
  assert.equal(configure(config),1);
  api.preview_gesture(120,120,0,0,1);api.preview_gesture(120,65,0,0,0);
  const state=snapshot();assert.equal(state.screen,5);
  assert.deepEqual(state.config,config);assert.deepEqual(state.draft,pick);
});


test("accessible schedule status follows the Rust manual override and Resume auto action", () => {
  const {api,configure,snapshot}=load();
  const config=structuredClone(fixture);config.groups[0].days=127;config.groups[0].start=0;config.groups[0].end=0;
  assert.equal(configure(config),1);assert.equal(snapshot().manual,true);
  api.preview_gesture(120,24,0,0,0);api.preview_gesture(120,212,0,0,0);api.preview_render(now,0);
  assert.equal(snapshot().manual,false);
  for(const group of config.groups)group.days=0;
  assert.equal(configure(config),1);
  api.preview_gesture(120,24,0,0,0);api.preview_gesture(120,212,0,0,0);api.preview_render(now,0);
  assert.equal(snapshot().manual,true);
});
