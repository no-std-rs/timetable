import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { bundleBytes, catalogBytes, SLOT_BYTES } from "../feed-bundle.js";
export const directory=JSON.parse(await readFile(new URL("./fixtures/feeds.json",import.meta.url)));
export const archives=new Map(await Promise.all(directory.feeds.map(async f=>[f.id,await readFile(new URL(`./fixtures/${f.id}.zip`,import.meta.url))])));
export function factoryImage(bundle, chipId = 5) {
  const start=0x10000,slot=start+24+8+16,dataLength=16+SLOT_BYTES+16,end=start+24+8+dataLength,checksum=Math.ceil((end-start+1)/16)*16+start-1;
  const bytes=new Uint8Array(checksum+33),v=new DataView(bytes.buffer);
  bytes[0]=0xe9;bytes[2]=2;bytes[3]=chipId===13?0x30:0x2f;v.setUint16(12,chipId,true);
  bytes[start]=0xe9;bytes[start+1]=1;bytes[start+23]=1;v.setUint16(start+12,chipId,true);
  v.setUint32(start+24,chipId===13?0x42000020:0x3c000020,true);v.setUint32(start+28,dataLength,true);
  bytes.fill(0x5a,start+32,end);bytes.fill(255,slot,slot+SLOT_BYTES);bytes.set(bundle,slot);
  let xor=0xef;for(let i=start+32;i<end;i++)xor^=bytes[i];bytes[checksum]=xor;
  bytes.set(createHash("sha256").update(bytes.subarray(start,checksum+1)).digest(),checksum+1);
  const release={chip:chipId===13?"ESP32-C6":"ESP32-C3",board:chipId===13?"Waveshare ESP32-C6-Touch-LCD-1.47":"GUITION ESP32-2424S012-V1.0",flash_size:chipId===13?8388608:4194304,config_offset:4128768,config_version:2,revision:"b".repeat(40),sha256:createHash("sha256").update(bytes).digest("hex"),size:bytes.length,catalog_slot:{offset:slot,capacity:SLOT_BYTES,format:"TTBND001"}};
  return {bytes,release,checksum,start,slot};
}
const defaultCatalog=JSON.parse(await readFile(new URL("../../catalog/catalog.json",import.meta.url)));
export const template=factoryImage(bundleBytes(catalogBytes(defaultCatalog),new Uint8Array()));
