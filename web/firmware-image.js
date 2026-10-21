import { releaseDevice } from "./devices.js";
import { crc32 } from "./config.js";
import { SLOT_BYTES } from "./feed-bundle.js";
const magic=new TextEncoder().encode("TTBND001");
export async function sha256(bytes) {return [...new Uint8Array(await crypto.subtle.digest("SHA-256",bytes))].map(b=>b.toString(16).padStart(2,"0")).join("");}
export function appLayout(bytes, chipId = 5) {
  const start=0x10000,v=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength);
  if(bytes.length<start+24||bytes[start]!==0xe9||bytes[start+1]<1||bytes[start+1]>16||v.getUint16(start+12,true)!==chipId||![0,1].includes(bytes[start+23]))throw new Error("Invalid display firmware image.");
  let pos=start+24;const segments=[];
  for(let i=0;i<bytes[start+1];i++){
    if(pos+8>bytes.length)throw new Error("Invalid display firmware segment.");
    const address=v.getUint32(pos,true),size=v.getUint32(pos+4,true);pos+=8;
    if(pos+size>bytes.length||pos+size>0x300000)throw new Error("Invalid display firmware segment.");
    segments.push({start:pos,end:pos+size,address});pos+=size;
  }
  const checksum=Math.ceil((pos-start+1)/16)*16+start-1,hash=bytes[start+23]===1;
  if(checksum+1+(hash?32:0)!==bytes.length)throw new Error("Unsupported signed or padded display image.");
  return {start,segments,checksum,hash};
}
export async function customizeFirmware(template,release,bundle) {
  if(template.length!==release.size||await sha256(template)!==release.sha256)throw new Error("The firmware checksum didn’t match. Retry the download before installing.");
  const slot=release.catalog_slot;
  if(!slot||slot.format!=="TTBND001"||slot.capacity!==SLOT_BYTES||!Number.isSafeInteger(slot.offset)||bundle.length>SLOT_BYTES)throw new Error("This firmware does not support selecting another network. Retry the firmware download.");
  const board=releaseDevice(release);
  const app=appLayout(template,board.chipId);
  if(!app.segments.some(s=>(board.chipId===5 ? s.address>=0x3c000000&&s.address<0x3d000000 : s.address>=0x42000000&&s.address<0x42800000)&&slot.offset>=s.start&&slot.offset+SLOT_BYTES<=s.end))throw new Error("Invalid catalogue location in firmware.");
  const v=new DataView(bundle.buffer,bundle.byteOffset,bundle.byteLength);
  if(bundle.length<20||!magic.every((b,i)=>bundle[i]===b)||v.getUint32(8,true)!==bundle.length-16||v.getUint32(12,true)!==crc32(bundle.subarray(16)))throw new Error("Invalid selected network catalogue.");
  if(!magic.every((b,i)=>template[slot.offset+i]===b))throw new Error("Invalid catalogue location in firmware.");
  const bytes=template.slice();bytes.fill(255,slot.offset,slot.offset+SLOT_BYTES);bytes.set(bundle,slot.offset);
  let checksum=0xef;for(const s of app.segments)for(let i=s.start;i<s.end;i++)checksum^=bytes[i];bytes[app.checksum]=checksum;
  if(app.hash)bytes.set(new Uint8Array(await crypto.subtle.digest("SHA-256",bytes.subarray(app.start,app.checksum+1))),app.checksum+1);
  return {bytes,sha256:await sha256(bytes)};
}
