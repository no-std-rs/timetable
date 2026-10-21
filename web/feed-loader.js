import { ZipArchive } from "./zip-archive.js";
import { csvRows } from "./csv-stream.js";
import { label } from "./feed-bundle.js";
import { mirrorURL } from "./feed-directory.js";
const utf8=new TextEncoder();
const validId=id=>typeof id==="string"&&id!==""&&!/\p{Cc}/u.test(id);
export class FeedLoader {
  rows(name) { return csvRows(this.zip.stream(name),{maxRecord:8192,maxFields:48}); }
  async load(bytes,feed,progress=()=>{}) {
    this.zip=new ZipArchive(bytes);this.feed=feed;this.trips=new Map();this.loadedRoutes=new Set();
    if(this.zip.members.get("frequencies.txt")?.size)throw new Error("Frequency-based timetables are not supported by this display yet.");
    const agencies=[];for await(const a of this.rows("agency.txt"))agencies.push(a);
    const zones=[...new Set(agencies.map(a=>a.agency_timezone).filter(Boolean))];
    if(zones.length!==1)throw new Error("The feed must declare one agency timezone.");
    new Intl.DateTimeFormat("en",{timeZone:zones[0]});this.timezone=zones[0];
    let language=agencies[0]?.agency_lang||"en",compare;
    try{compare=new Intl.Collator(language,{numeric:true,sensitivity:"base"}).compare;}catch{language="en";compare=new Intl.Collator("en",{numeric:true,sensitivity:"base"}).compare;}
    const routes=[];progress("Reading lines…");
    for await(const r of this.rows("routes.txt")){
      if(!validId(r.route_id))throw new Error("Invalid route ID in feed.");
      routes.push({id:r.route_id,line:label(r.route_short_name||r.route_long_name||r.route_id,8),headsigns:["",""]});
      if(routes.length>200000)throw new Error("This feed exceeds the browser catalogue limit.");
    }
    routes.sort((a,b)=>compare(a.line,b.line)||compare(a.id,b.id));
    const routeIds=new Map(routes.map((r,i)=>[r.id,i]));if(routeIds.size!==routes.length)throw new Error("Duplicate route ID in feed.");
    const stops=[];progress("Reading stops…");
    for await(const s of this.rows("stops.txt")){
      if(!validId(s.stop_id))throw new Error("Invalid stop ID in feed.");
      stops.push({id:s.stop_id,name:label(s.stop_name||s.stop_id,64),code:utf8.encode(s.stop_code||"").length<=16?s.stop_code||"":""});
      if(stops.length>2000000)throw new Error("This feed exceeds the browser catalogue limit.");
    }
    stops.sort((a,b)=>compare(a.name,b.name)||compare(a.code,b.code)||compare(a.id,b.id));
    this.stopIds=new Map(stops.map((s,i)=>[s.id,i]));if(this.stopIds.size!==stops.length)throw new Error("Duplicate stop ID in feed.");
    progress("Reading service calendars…");
    for(const table of ["calendar.txt","calendar_dates.txt"])if(this.zip.members.has(table))for await(const row of this.rows(table)){
      if(!validId(row.service_id)||utf8.encode(row.service_id).length>32)throw new Error("Service IDs in this feed exceed the display’s 32-byte limit.");
    }
    if(!this.zip.members.has("calendar.txt")&&!this.zip.members.has("calendar_dates.txt"))throw new Error("The feed has no service calendar.");
    const heads=routes.map(()=>[new Map(),new Map()]);progress("Reading journeys…");
    for await(const t of this.rows("trips.txt")){
      const route=routeIds.get(t.route_id),direction=t.direction_id===""||t.direction_id===undefined?-1:Number(t.direction_id);
      if(route===undefined||!validId(t.trip_id)||![-1,0,1].includes(direction)||this.trips.has(t.trip_id))throw new Error("Invalid journey in feed.");
      const headsign=label(t.trip_headsign||"",64);this.trips.set(t.trip_id,{route,direction,headsign});
      if(utf8.encode(t.trip_id).length>255)routes[route].device_error="Journey IDs on this line exceed the display’s 255-byte limit.";
      if(direction>=0){const map=heads[route][direction];map.set(headsign,(map.get(headsign)||0)+1);}
      if(this.trips.size>3000000)throw new Error("This feed exceeds the browser journey limit.");
    }
    heads.forEach((h,r)=>h.forEach((m,d)=>{routes[r].headsigns[d]=[...m].sort((a,b)=>b[1]-a[1]||compare(a[0],b[0]))[0]?.[0]||"";}));
    const memberships=routes.map(()=>new Map()),characters=routes.map(()=>new Set());
    for(const trip of this.trips.values())for(const c of trip.headsign)characters[trip.route].add(c);
    let count=0,lastOverride="",lastOverrideRoute=-1;progress("Connecting lines and stops…");
    for await(const row of this.rows("stop_times.txt")){
      const trip=this.trips.get(row.trip_id),stop=this.stopIds.get(row.stop_id),pickup=Number(row.pickup_type||0);
      if(!trip||stop===undefined||!Number.isInteger(pickup)||pickup<0||pickup>3)throw new Error("Invalid stop call in feed.");
      if(row.stop_headsign && (row.stop_headsign!==lastOverride||trip.route!==lastOverrideRoute)){lastOverride=row.stop_headsign;lastOverrideRoute=trip.route;for(const c of label(lastOverride,64))characters[trip.route].add(c);}
      if(pickup!==1){const links=memberships[trip.route],bit=trip.direction<0?4:1<<trip.direction;links.set(stop,(links.get(stop)||0)|bit);}
      if(++count%100000===0)progress("Connecting lines and stops…",count);
    }
    const links=memberships.flatMap((m,r)=>[...m].sort((a,b)=>a[0]-b[0]).map(([s,bits])=>[r,s,bits]));
    routes.forEach((r,i)=>{r.characters=[...characters[i]].join("");});
    this.catalog={provider:feed.provider,date:new Date().toISOString().slice(0,10).replaceAll("-",""),source:mirrorURL(feed.id),feed_id:feed.id,timezone:this.timezone,language,routes,stops,links,via_stops:[],patterns:[]};
    return this.catalog;
  }
  async paths(route,progress=()=>{}) {
    if(!Number.isInteger(route)||!this.catalog.routes[route])throw new Error("Invalid route selection.");
    if(this.loadedRoutes.has(route))return this.catalog.patterns;
    const calls=new Map();let count=0;progress("Building route paths…");
    for await(const row of this.rows("stop_times.txt")){
      const trip=this.trips.get(row.trip_id);if(trip?.route!==route)continue;
      const sequence=Number(row.stop_sequence),stop=this.stopIds.get(row.stop_id),pickup=Number(row.pickup_type||0);
      if(!/^\d+$/.test(row.stop_sequence)||!Number.isSafeInteger(sequence)||stop===undefined||!Number.isInteger(pickup)||pickup<0||pickup>3)throw new Error("Invalid route path in feed.");
      const list=calls.get(row.trip_id)||[];list.push([sequence,stop,pickup,label(row.stop_headsign||"",64)]);calls.set(row.trip_id,list);
      if(++count>2000000)throw new Error("This route exceeds the browser path limit.");
    }
    const patterns=new Map();
    for(const [id,list]of calls){
      const trip=this.trips.get(id);list.sort((a,b)=>a[0]-b[0]);
      if(list.some((c,i)=>i&&c[0]===list[i-1][0]))throw new Error("Duplicate stop sequence in route.");
      const ordered=list.map(([,stop,pickup,headsign])=>headsign?[stop,pickup,headsign]:[stop,pickup]);
      const key=JSON.stringify([trip.direction,trip.headsign,ordered]);
      if(patterns.has(key))patterns.get(key).trips++;else patterns.set(key,{route,direction:trip.direction,headsign:trip.headsign,calls:ordered,trips:1});
    }
    this.catalog.patterns.push(...patterns.values());this.loadedRoutes.add(route);return this.catalog.patterns;
  }
}
export async function downloadFeed(feed,progress=()=>{}) {
  const response=await fetch(mirrorURL(feed.id),{credentials:"omit"});if(!response.ok)throw new Error("The selected timetable could not be downloaded.");
  const max=512*1024*1024;if(Number(response.headers.get("Content-Length"))>max)throw new Error("This archive exceeds the browser download limit.");
  const chunks=[];let total=0,last=0;
  for await(const chunk of response.body){total+=chunk.length;if(total>max)throw new Error("This archive exceeds the browser download limit.");chunks.push(chunk);if(performance.now()-last>250){progress("Downloading timetable…",total);last=performance.now();}}
  const bytes=new Uint8Array(total);let pos=0;for(const chunk of chunks){bytes.set(chunk,pos);pos+=chunk.length;}return bytes;
}
