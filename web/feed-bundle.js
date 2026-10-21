import { crc32 } from "./config.js";
export const SLOT_BYTES=1024*1024;
const encoder=new TextEncoder();
const builtIn = new Set(" Ą˘Ł¤ĽŚ§¨ŠŞŤŹ­ŽŻ°ą˛ł´ľśˇ¸šşťź˝žżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙");
export const needsGlyph = c => c.codePointAt(0) > 127 && !builtIn.has(c) && !/\p{Cc}/u.test(c);

export function label(text,capacity) {
  text=String(text??"").replaceAll("\0","").replace(/\p{White_Space}+/gu," ").replace(/^ +| +$/g, "");
  if(encoder.encode(text).length<=capacity)return text;
  let result="";for(const c of text){if(encoder.encode(result+c).length>capacity-3)break;result+=c;}return result+"...";
}
function field(bytes,offset,length,text) {
  const value=encoder.encode(text);if(value.length>length||value.includes(0))throw new Error("A feed identity exceeds the display’s supported limit.");bytes.set(value,offset);
}
// Intl supplies the installed IANA database; transition rules are frozen into ROM.
export function zoneBytes(timezone, now=Date.now()) {
  const format=new Intl.DateTimeFormat("en-GB",{timeZone:timezone,timeZoneName:"longOffset"});
  const offset=seconds=>{
    const value=format.formatToParts(new Date(seconds*1000)).find(p=>p.type==="timeZoneName").value;
    if(value==="GMT")return 0;
    const m=/^GMT([+-])(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(value);if(!m)throw new Error("Unsupported feed timezone.");
    return (m[1]==="-"?-1:1)*(Number(m[2])*3600+Number(m[3])*60+Number(m[4]??0));
  };
  const year=new Date(now).getUTCFullYear(),start=Date.UTC(year-2,0,1)/1000,end=Date.UTC(year+11,0,1)/1000;
  const transitions=[];let previous=offset(start);
  for(let at=start+86400;at<end;at+=86400){
    const next=offset(at);if(next===previous)continue;
    let lo=at-86400,hi=at;while(hi-lo>1){const mid=Math.floor((lo+hi)/2);if(offset(mid)===previous)lo=mid;else hi=mid;}
    transitions.push([hi,next]);previous=next;
  }
  if(transitions.length>32)throw new Error("This timezone exceeds the display’s transition limit.");
  const bytes=new Uint8Array(20+12*transitions.length),v=new DataView(bytes.buffer);
  v.setBigInt64(0,BigInt(start),true);v.setBigInt64(8,BigInt(end),true);v.setInt32(16,offset(start),true);
  transitions.forEach(([at,o],i)=>{v.setBigInt64(20+12*i,BigInt(at),true);v.setInt32(28+12*i,o,true);});return bytes;
}
export function glyphBytes(texts) {
  const chars=[...new Set(texts.flatMap(text=>[...String(text||"")]).filter(needsGlyph))].sort((a,b)=>a.codePointAt(0)-b.codePointAt(0));
  if(!chars.length)return new Uint8Array();
  if(chars.length>4096)throw new Error("These labels exceed the display’s glyph limit. Select fewer lines.");
  const canvas=typeof OffscreenCanvas!=="undefined"?new OffscreenCanvas(10,10):Object.assign(document.createElement("canvas"),{width:10,height:10});
  const context=canvas.getContext("2d",{willReadFrequently:true}),bytes=new Uint8Array(10+24*chars.length),v=new DataView(bytes.buffer);
  bytes.set(encoder.encode("TTGLY001"));v.setUint16(8,chars.length,true);
  context.font="10px sans-serif";context.textAlign="center";context.textBaseline="middle";context.fillStyle="black";
  chars.forEach((char,i)=>{
    context.clearRect(0,0,10,10);context.fillText(char,5,5,10);const rgba=context.getImageData(0,0,10,10).data;
    v.setUint32(10+24*i,char.codePointAt(0),true);
    for(let row=0;row<10;row++){let bits=0;for(let col=0;col<10;col++)if(rgba[(row*10+col)*4+3]>=96)bits|=1<<col;v.setUint16(14+24*i+row*2,bits,true);}
  });return bytes;
}
export function profileBytes(feed,timezone,now,glyphs=new Uint8Array()) {
  const zone=zoneBytes(timezone,now),bytes=new Uint8Array(250+zone.length+glyphs.length);
  bytes.set(encoder.encode("TTPRF001"));field(bytes,8,80,feed.id);field(bytes,88,64,timezone);field(bytes,152,96,label(feed.provider||feed.name||feed.id,96));new DataView(bytes.buffer).setUint16(248,zone.length,true);bytes.set(zone,250);bytes.set(glyphs,250+zone.length);return bytes;
}
export function catalogBytes(catalog,selectedRoutes=null) {
  const routes=catalog.routes.map((r,i)=>[r,i]).filter(([,i])=>!selectedRoutes||selectedRoutes.has(i));
  const allowed=new Set(routes.map(([,i])=>i)),links=catalog.links.filter(([r])=>allowed.has(r));
  const stops=[...new Set(links.map(([,s])=>s))].sort((a,b)=>a-b);
  if(routes.length>65535||stops.length>65535)throw new Error("This catalogue exceeds the display’s index capacity. Select fewer lines.");
  const ri=new Map(routes.map(([,r],i)=>[r,i])),si=new Map(stops.map((s,i)=>[s,i]));
  const size=32+168*routes.length+112*stops.length+6*links.length;
  if(size>SLOT_BYTES-2048)throw new Error("This catalogue is too large for the display. Select fewer lines.");
  const bytes=new Uint8Array(size),v=new DataView(bytes.buffer);
  bytes.set(encoder.encode("TTCAT001"));field(bytes,8,8,catalog.date);v.setUint16(16,routes.length,true);v.setUint16(18,stops.length,true);v.setUint32(20,links.length,true);
  let pos=32;
  for(const [r]of routes){if(r.device_error)throw new Error(r.device_error);field(bytes,pos,32,r.id);field(bytes,pos+32,8,r.line);field(bytes,pos+40,64,label(r.headsigns[0],64));field(bytes,pos+104,64,label(r.headsigns[1],64));pos+=168;}
  for(const i of stops){const s=catalog.stops[i];field(bytes,pos,32,s.id);field(bytes,pos+32,64,label(s.name,64));field(bytes,pos+96,16,s.code);pos+=112;}
  for(const [r,s,bits]of links){v.setUint16(pos,ri.get(r),true);v.setUint16(pos+2,si.get(s),true);bytes[pos+4]=bits;pos+=6;}
  v.setUint32(24,crc32(bytes.subarray(32)),true);return bytes;
}
export function bundleBytes(catalog,profile) {
  const bytes=new Uint8Array(20+catalog.length+profile.length),v=new DataView(bytes.buffer);
  if(bytes.length>SLOT_BYTES)throw new Error("This catalogue is too large for the display. Select fewer lines.");
  bytes.set(encoder.encode("TTBND001"));v.setUint32(8,bytes.length-16,true);v.setUint32(16,catalog.length,true);bytes.set(catalog,20);bytes.set(profile,20+catalog.length);v.setUint32(12,crc32(bytes.subarray(16)),true);return bytes;
}
