import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { csvRows } from "../csv-stream.js";
import { FeedLoader } from "../feed-loader.js";
import { directoryIndex } from "../feed-directory.js";
import { catalogBytes,profileBytes,bundleBytes,zoneBytes } from "../feed-bundle.js";
import { customizeFirmware } from "../firmware-image.js";
import { catalogIndex } from "../catalog-search.js";
import { archives,directory,template } from "./world-fixtures.mjs";
import { saveDraft,loadFeedDraft,forgetDraft,SETTINGS_KEY } from "../settings-store.js";
const now=Date.UTC(2026,9,21,12),encoder=new TextEncoder();
test("header-only optional frequency tables are empty but actual templates remain unsupported",async()=>{
  for(const file of ["frequencies-empty-stored.zip","frequencies-empty-deflated.zip","frequencies-zero.zip"]){
    const bytes=await readFile(new URL(`./fixtures/${file}`,import.meta.url));
    const c=await new FeedLoader().load(bytes,directory.feeds[0]);assert.equal(c.routes.length,1);
  }
  const bytes=await readFile(new URL("./fixtures/frequencies-populated.zip",import.meta.url));
  await assert.rejects(new FeedLoader().load(bytes,directory.feeds[0]),/Frequency-based/);
});
function chunks(text){const bytes=encoder.encode(text);return new ReadableStream({start(c){for(const byte of bytes)c.enqueue(new Uint8Array([byte]));c.close();}});}
test("global directory includes nonnumeric IDs, regional feeds and unavailable sources without inventing cities",()=>{
  const data=structuredClone(directory);data.feeds=data.feeds.slice(0,2);data.feeds.push({...data.feeds[0],id:"jbda-town-rail",country:"JP",city:"",region:"",mirror:"https://files.mobilitydatabase.org/jbda-town-rail/latest.zip"},{...data.feeds[0],id:"../bad",country:"",city:"",region:"",mirror:"https://private.invalid/"},{...data.feeds[0],id:"old",status:"inactive"});
  const index=directoryIndex(data);assert.ok(index.countries().includes(""));assert.ok(index.cities("JP").includes(""));
  assert.equal(index.search("JP","","jbda")[0].id,"jbda-town-rail");
  assert.equal(index.available(index.search("JP")[0]),true);assert.equal(index.available(data.feeds[3]),false);assert.equal(index.available(data.feeds[4]),false);
  assert.deepEqual(index.search("US",null,"metro new").map(f=>f.id),["demo-ny","old"]);
});
test("streaming CSV keeps BOM, quotes, UTF-8 and embedded newlines across one-byte chunks",async()=>{
  const rows=[];for await(const row of csvRows(chunks('\uFEFFid,name\r\nS1,"Łąkowa, ""north""\nplatform"\r\nS2,東京\r\n')))rows.push(row);
  assert.deepEqual(rows,[{id:"S1",name:'Łąkowa, "north"\nplatform'},{id:"S2",name:"東京"}]);
  await assert.rejects(async()=>{for await(const _ of csvRows(chunks('a,b\n1,"missing'))){}},/Incomplete/);
});
test("two actual ZIPs reuse identities without sharing places, zones, memberships or interleaved branch paths",async()=>{
  for(const feed of directory.feeds){
    const loader=new FeedLoader(),c=await loader.load(archives.get(feed.id),feed),index=catalogIndex(c);
    assert.equal(c.timezone,({"demo-ny":"America/New_York","demo-kolkata":"Asia/Kolkata","demo-jp":"Asia/Tokyo"})[feed.id]);
    assert.equal(c.routes[0].id,"R");assert.equal(c.stops.find(s=>s.id==="S1").name,({"demo-ny":"Broadway","demo-kolkata":"Park Street","demo-jp":"東京駅"})[feed.id]);
    assert.equal(index.stopGroups().length,3);
    const patterns=await loader.paths(0);assert.equal(patterns.length,3);
    const branch=patterns.find(p=>p.headsign==="North branch");assert.deepEqual(branch.calls.map(([s])=>c.stops[s].id),["S1","S3","S4"]);assert.equal(branch.calls.at(-1)[1],1);
    const bytes=catalogBytes(c);assert.equal(new TextDecoder().decode(bytes.subarray(32,33)),"R");
  }
});
test("ZIP corruption is rejected before publishing a catalogue",async()=>{
  const bytes=Uint8Array.from(archives.get("demo-ny")),offset=Buffer.from(bytes).indexOf(Buffer.from("Metro New York"));assert.ok(offset>0);bytes[offset]^=1;
  await assert.rejects(()=>new FeedLoader().load(bytes,directory.feeds[0]),/checksum/);
});
test("IANA profiles retain fractional offsets and both northern and southern DST transitions",()=>{
  const offset=(zone,seconds)=>{const bytes=zoneBytes(zone,now),v=new DataView(bytes.buffer);let result=v.getInt32(16,true);for(let n=20;n<bytes.length;n+=12)if(Number(v.getBigInt64(n,true))<=seconds)result=v.getInt32(n+8,true);return result;};
  assert.equal(offset("Asia/Kolkata",Date.UTC(2026,9,21)/1000),19800);
  assert.equal(offset("America/New_York",Date.UTC(2026,2,8,6,59)/1000),-18000);assert.equal(offset("America/New_York",Date.UTC(2026,2,8,7)/1000),-14400);
  assert.equal(offset("Australia/Lord_Howe",Date.UTC(2026,3,4,14,59)/1000),39600);assert.equal(offset("Australia/Lord_Howe",Date.UTC(2026,3,4,15)/1000),37800);
});
test("customized factory image changes only the declared ROM slot and valid ESP footer",async()=>{
  const feed=directory.feeds[1],c=await new FeedLoader().load(archives.get(feed.id),feed),bundle=bundleBytes(catalogBytes(c),profileBytes(feed,c.timezone,now));
  const result=await customizeFirmware(template.bytes,template.release,bundle);
  assert.deepEqual(result.bytes.subarray(0,template.slot),template.bytes.subarray(0,template.slot));
  assert.deepEqual(result.bytes.subarray(template.slot,template.slot+bundle.length),bundle);
  assert.deepEqual(result.bytes.subarray(template.slot+1048576,template.checksum),template.bytes.subarray(template.slot+1048576,template.checksum));
  const dataStart=template.start+32,dataLength=new DataView(result.bytes.buffer).getUint32(template.start+28,true);
  const xor=result.bytes.subarray(dataStart,dataStart+dataLength).reduce((sum,b)=>sum^b,0xef);assert.equal(result.bytes[template.checksum],xor);
  assert.deepEqual(result.bytes.subarray(template.checksum+1),new Uint8Array(createHash("sha256").update(result.bytes.subarray(template.start,template.checksum+1)).digest()));
  assert.equal(result.sha256,createHash("sha256").update(result.bytes).digest("hex"));
  await assert.rejects(()=>customizeFirmware(template.bytes,{...template.release,sha256:"0".repeat(64)},bundle),/checksum/);
  await assert.rejects(()=>customizeFirmware(template.bytes,{...template.release,catalog_slot:{...template.release.catalog_slot,offset:template.slot+1}},bundle),/location/);
});
test("actual WASM swaps checked feed catalogues atomically and reports the selected timezone",async()=>{
  const module=new WebAssembly.Module(await readFile(new URL("../dist/device-preview.wasm",import.meta.url))),api=new WebAssembly.Instance(module,{}).exports;
  const fixture=JSON.parse(await readFile(new URL("../../display/tests/settings.json",import.meta.url))),json=encoder.encode(JSON.stringify(fixture));
  const snapshot=()=>JSON.parse(new TextDecoder().decode(new Uint8Array(api.memory.buffer,api.preview_output(),api.preview_snapshot())));
  const configure=()=>{new Uint8Array(api.memory.buffer,api.preview_input(),json.length).set(json);assert.equal(api.preview_configure(json.length,BigInt(now/1000),0),1);};
  for(const feed of directory.feeds){
    const c=await new FeedLoader().load(archives.get(feed.id),feed),bytes=bundleBytes(catalogBytes(c),profileBytes(feed,c.timezone,now));
    new Uint8Array(api.memory.buffer,api.preview_catalog_input(),bytes.length).set(bytes);assert.equal(api.preview_catalog_commit(bytes.length),1);configure();
    assert.equal(snapshot().feed_id,feed.id);assert.equal(snapshot().timezone,c.timezone);
    const before=snapshot();bytes[12]^=1;new Uint8Array(api.memory.buffer,api.preview_catalog_input(),bytes.length).set(bytes);assert.equal(api.preview_catalog_commit(bytes.length),0);assert.deepEqual(snapshot(),before);
  }
});
test("remembered network drafts stay separate and Forget erases every credential copy even at quota",()=>{
  const values=new Map(),storage={get length(){return values.size;},key:i=>[...values.keys()][i]??null,getItem:k=>values.get(k)??null,setItem:(k,v)=>values.set(k,v),removeItem:k=>values.delete(k)};
  const config=JSON.parse(requireFixture());
  for(const feed of directory.feeds){const c=structuredClone(config);c.groups[0].name=feed.city;saveDraft(storage,{version:1,feed_id:feed.id,active:0,config:c});}
  assert.equal(loadFeedDraft(storage,"demo-ny").config.groups[0].name,"New York");assert.equal(loadFeedDraft(storage,"demo-kolkata").config.groups[0].name,"Kolkata");
  storage.setItem=()=>{throw new Error("QuotaExceededError");};assert.throws(()=>forgetDraft(storage));assert.ok(![...values.keys()].some(k=>k===SETTINGS_KEY||k.startsWith(`${SETTINGS_KEY}.`)));
});
const fixtureText=await readFile(new URL("../../display/tests/settings.json",import.meta.url),"utf8");function requireFixture(){return fixtureText;}

test("long human line labels remain bounded while unsupported service identities fail before installation",async()=>{
  const bytes=await readFile(new URL("./fixtures/long-line.zip",import.meta.url));
  const c=await new FeedLoader().load(bytes,directory.feeds[0]);
  assert.equal(c.routes[0].line,"ABCDE...");assert.ok(catalogBytes(c).length);
  const invalid=await readFile(new URL("./fixtures/bad-service.zip",import.meta.url));
  await assert.rejects(()=>new FeedLoader().load(invalid,directory.feeds[0]),/Service IDs/);
});
