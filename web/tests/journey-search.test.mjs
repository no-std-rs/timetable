import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { FeedLoader } from "../feed-loader.js";
import { searchJourneys } from "../journey-search.js";
import { stopPlaces, coalesceJourneyStops } from "../stop-places.js";
import { validateJourney } from "../config.js";
const NOW=1792216800;
const config=()=>({origins:[{stop:"A",stop_name:"Maków Polnych",stop_code:"A01",walk:6},{stop:"B",stop_name:"Further stop",stop_code:"B01",walk:12}],targets:[{stop:"Z",stop_name:"Bolechowo-Os./Szkoła",stop_code:"Z01",walk:0}],transfer_minutes:3,max_transfers:1});
async function loader(name="journey.zip"){
  const loader=new FeedLoader();await loader.load(new Uint8Array(await readFile(new URL(`../../display/tests/fixtures/${name}`,import.meta.url))),{id:"mdb-982",provider:"Synthetic transit"});return loader;
}
test("browser stages agree with the Rust fixture for direct, one-transfer and deeper journeys",async()=>{
  const feed=await loader();const progress=[];
  const result=await searchJourneys(feed,config(),{now:NOW,maxTransfers:4,onProgress:r=>progress.push(structuredClone(r))});
  assert.deepEqual(progress.map(p=>p.depth),[0,1,2,3,4]);
  assert.deepEqual(progress.map(p=>p.complete),[false,false,false,false,true]);
  assert.deepEqual(progress.slice(0,3).map(p=>p.paths[0].arrival-NOW),[35*60,30*60,29*60]);
  assert.equal(result.paths[0].legs.length,3);assert.equal(result.paths[0].legs[0].line,"911");
  assert.equal(result.paths[0].target.name,"Bolechowo-Os./Szkoła");
});
test("walking, transfer minimums, feed exclusions, shuffled calls and multiple targets stay consistent",async()=>{
  for(const [file,minutes]of[["journey-direct.zip",28],["journey-forbidden.zip",35],["journey-minimum.zip",35],["journey-timed.zip",30],["journey-shuffled.zip",30],["journey-station.zip",30],["journey-station-forbidden.zip",35]]){
    const result=await searchJourneys(await loader(file),config(),{now:NOW});assert.equal(result.paths[0].arrival-NOW,minutes*60,file);
  }
  assert.deepEqual((await searchJourneys(await loader("journey-no-service.zip"),config(),{now:NOW})).paths,[]);
  const c=config();c.targets.push({stop:"W",stop_name:"Other target",stop_code:"W01",walk:0});
  assert.equal((await searchJourneys(await loader(),c,{now:NOW})).paths[0].arrival-NOW,27*60);
  c.targets=c.targets.slice(0,1);c.transfer_minutes=2;
  assert.equal((await searchJourneys(await loader(),c,{now:NOW})).paths[0].arrival-NOW,25*60);
});
test("device settings reject unsupported depth and ambiguous endpoint lists",()=>{
  const c=config();assert.equal(validateJourney(c),c);
  for(const transfer_minutes of[-1,31,NaN])assert.throws(()=>validateJourney({...c,transfer_minutes}));
  assert.throws(()=>validateJourney({...c,max_transfers:3}));
  assert.throws(()=>validateJourney({...c,origins:[]}));
  assert.throws(()=>validateJourney({...c,origins:[c.origins[0],c.origins[0]]}));
});

test("dwell calls spanning either window boundary remain usable",async()=>{
  const result=await searchJourneys(await loader("journey-boundaries.zip"),config(),{now:NOW,maxTransfers:0});
  assert.equal(result.paths[0].legs[0].departure,NOW+7*60);
  assert.equal(result.paths[0].arrival,NOW+359*60);
});
test("unsupported rules fail only for reachable transfers, and conditional boarding is explicit",async()=>{
  assert.equal((await searchJourneys(await loader("journey-conditional-through.zip"),config(),{now:NOW,maxTransfers:0})).paths[0].arrival,NOW+35*60);
  assert.equal((await searchJourneys(await loader("journey-remote-rule.zip"),config(),{now:NOW})).paths[0].arrival,NOW+30*60);
  for(const file of["journey-inseat-rule.zip","journey-conditional.zip","journey-conditional-alight.zip"]){
    await assert.rejects(searchJourneys(await loader(file),config(),{now:NOW}),/does not yet support/);
  }
});

test("a timed-transfer guarantee does not waive the configured user margin",async()=>{
  const c=config();c.transfer_minutes=2;
  assert.equal((await searchJourneys(await loader("journey-timed.zip"),c,{now:NOW})).paths[0].arrival,NOW+25*60);
});

test("scoped rules apply to matching trips and override less-specific rules",async()=>{
  for(const file of["journey-scoped-rule.zip","journey-scope-unrelated.zip","journey-scope-precedence.zip"]){
    const result=await searchJourneys(await loader(file),config(),{now:NOW});
    assert.equal(result.paths[0].arrival,NOW+30*60,file);
  }
  const c=config();c.transfer_minutes=2;
  const trips=await searchJourneys(await loader("journey-scope-trip.zip"),c,{now:NOW});
  assert.equal(trips.paths[0].arrival,NOW+30*60,"trip override permits legal but route prohibition still excludes tight");
  assert.equal(trips.paths[0].legs[1].trip,JSON.stringify(["legal",1]));
});
test("a later arriving service remains available when the earlier trip cannot transfer",async()=>{
  const result=await searchJourneys(await loader("journey-scope-alternative.zip"),config(),{now:NOW});
  assert.equal(result.paths[0].arrival,NOW+30*60);
  assert.equal(result.paths[0].legs[0].line,"Alt");
});

test("a scoped cross-stop link never creates a transfer for unrelated services",async()=>{
  for(const [name,minutes]of[["matching",30],["unrelated",35]]){
    const result=await searchJourneys(await loader(`journey-scope-link-${name}.zip`),config(),{now:NOW});
    assert.equal(result.paths[0].arrival,NOW+minutes*60,name);
  }
});

test("one named endpoint searches every platform and retains distinct routes to that place",async()=>{
  for(const name of ["journey-places.zip","journey-places-shuffled.zip"]){
    const feed=await loader(name),c=config();c.origins.length=1;
    const result=await searchJourneys(feed,c,{now:NOW});
    assert.equal(result.paths.length,2);
    assert.ok(result.paths.every(path=>["Z","Z2"].includes(path.target.id)));
    assert.equal(result.paths[0].arrival,NOW+22*60);
    assert.equal(result.paths[0].legs[0].from.id,"A2");
    assert.equal(result.paths[0].legs[0].to.id,"Z2");
    c.origins[0].walk=10;
    assert.ok((await searchJourneys(feed,c,{now:NOW})).paths[0].arrival>NOW+22*60);
    c.origins[0].stop="U";
    assert.deepEqual((await searchJourneys(feed,c,{now:NOW})).paths,[]);
  }
});

test("all transfer routes survive a shared final ride and include ordered non-boarding stops",async()=>{
  const feed=await loader("journey-alternatives.zip"),c=config();c.origins.length=1;
  assert.deepEqual((await searchJourneys(feed,c,{now:NOW,maxTransfers:0})).paths,[],"the endpoints have no direct connection");
  const result=await searchJourneys(feed,c,{now:NOW});
  assert.equal(result.until,NOW+6*3600);
  assert.deepEqual(result.paths.map(path=>path.legs.map(leg=>leg.line)),[["911","Transfer"],["911","Transfer"],["Alt","Transfer"]]);
  assert.deepEqual(result.paths.map(path=>path.legs[0].stops.map(call=>call.stop.id)),[["A","P","X"],["A","Q","X"],["A","X"]]);
  for(const path of result.paths){
    assert.equal(path.arrival,NOW+30*60);
    assert.deepEqual(path.legs[1].stops.map(call=>call.stop.id),["X","V","Z"]);
    assert.equal(path.legs[1].stops[1].arrival,NOW+25*60);
    assert.ok(path.legs[1].departure-path.legs[0].arrival>=3*60);
  }
  const rows=feed.rows.bind(feed);
  // Input ordering must not select a single winning prefix by accident.
  feed.rows=async function*(name){const data=await Array.fromAsync(rows(name));yield* name==="stop_times.txt"?data.reverse():data;};
  const reversed=await searchJourneys(feed,c,{now:NOW});
  assert.deepEqual(new Set(reversed.paths.map(p=>JSON.stringify(p.legs.map(l=>l.stops.map(c=>c.stop.id))))),new Set(result.paths.map(p=>JSON.stringify(p.legs.map(l=>l.stops.map(c=>c.stop.id))))));
});

test("direct and transfer alternatives remain together, with repeated services grouped by ordered route",async()=>{
  const result=await searchJourneys(await loader(),config(),{now:NOW,maxTransfers:2});
  assert.deepEqual(result.paths.map(path=>path.legs.map(leg=>leg.line)),[["911","Middle","Final"],["911","Transfer"],["Direct"],["Direct"]]);
  assert.deepEqual(result.paths.slice(2).map(path=>path.legs[0].stops.map(c=>c.stop.id)),[["B","W","Z"],["B","Z"]]);
  assert.equal(result.paths.filter(path=>path.legs.at(-1).line==="Transfer").length,1,"later identical services are one route alternative");
});
test("untimed calls distinguish route alternatives without inventing boarding or arrival times",async()=>{
  const feed=await loader("journey-untimed-alternatives.zip"),c=config();c.origins.length=1;
  const result=await searchJourneys(feed,c,{now:NOW});
  assert.equal(result.paths.length,3,"patterns differing only at untimed stops remain distinct");
  const viaPark=result.paths.find(p=>p.legs[0].stops.some(c=>c.stop.id==="P"));
  const park=viaPark.legs[0].stops[1],through=viaPark.legs[1].stops[1];
  assert.equal(park.arrival,null);assert.equal(park.departure,null);
  assert.equal(park.stop.code,"");assert.equal(park.stop.platform_code,"番二三四五六七八九十");
  assert.equal(through.arrival,null);assert.equal(through.departure,NOW+25*60);
  c.origins[0].stop="P";
  assert.deepEqual((await searchJourneys(feed,c,{now:NOW})).paths,[],"a blank time cannot be a boarding time");
  const rows=feed.rows.bind(feed);
  feed.rows=async function*(name){for await(const row of rows(name))yield name==="stop_times.txt"&&row.stop_id==="V"?{...row,drop_off_type:"0"}:row;};
  c.origins[0].stop="A";c.targets[0].stop="V";
  assert.deepEqual((await searchJourneys(feed,c,{now:NOW})).paths,[],"a partial time cannot supply an arrival");
});
test("place grouping excludes stations, preserves full names and coalesces old platform selections",async()=>{
  const feed=await loader("journey-places.zip"),places=stopPlaces(feed.catalog);
  assert.deepEqual(places.byId.get("A").stops.map(s=>s.id).sort(),["A","A2"]);
  assert.equal(places.byId.has("AP"),false);
  assert.notEqual(places.byId.get("A"),places.byId.get("U"));
  const c=config();c.origins.push({stop:"A2",walk:4});
  c.targets.push({stop:"Z2",walk:2});
  assert.equal(coalesceJourneyStops(c,places),true);
  assert.deepEqual(c.origins.map(s=>[s.stop,s.walk]),[["A",4],["B",12]]);
  assert.deepEqual(c.targets.map(s=>[s.stop,s.walk]),[["Z",0]]);
  assert.equal(coalesceJourneyStops(c,places),false);
  const long="A".repeat(64);
  const distinct=stopPlaces({stops:[{id:"1",name:long,place_name:long+" north"},{id:"2",name:long,place_name:long+" south"}]});
  assert.equal(distinct.groups.length,2,"display-label truncation never merges different full feed names");
  const terminal=stopPlaces({stops:[{id:"A",name:"Start",lat:52.4,lon:16.9}],via_stops:[{id:"Z",name:"Alight only",lat:52.4,lon:16.901}],links:[[0,0,1]]});
  assert.equal(terminal.byId.has("Z"),true,"drop-off-only platforms remain target choices");
  assert.equal(terminal.boarding.has("Z"),false);
  const {walkableStops}=await import("../stop-places.js");
  assert.deepEqual(walkableStops(terminal,[{stop:"A",walk:6}]),[],"drop-off-only stops are not suggested for boarding");
});

test("walkable suggestions include all nearby places and the corridor between chosen starts",async()=>{
  const {walkableStops}=await import("../stop-places.js");
  const feed=await loader("journey-places.zip"),places=stopPlaces(feed.catalog),c=config();
  const nearby=walkableStops(places,c.origins);
  assert.deepEqual(nearby.map(s=>[s.place.stops[0].id,s.walk]),[["Y",11],["W",15]]);
  const between=walkableStops(places,c.origins,{mode:"between",extraMinutes:20});
  assert.deepEqual(between.map(s=>s.place.stops[0].id),["Y","W"]);
  assert.ok(!between.some(s=>s.place.stops.some(p=>p.id==="X")),"off-corridor stops are excluded");
  assert.deepEqual(walkableStops(places,c.origins.slice(0,1),{mode:"between"}),[]);
  assert.deepEqual(walkableStops(places,c.origins,{extraMinutes:0}),[]);
  assert.deepEqual(walkableStops(places,[{stop:"U",walk:0}]),[],"missing coordinates never become zero coordinates");
  const many=stopPlaces({stops:Array.from({length:32},(_,i)=>({id:String(i),name:String(i),lat:52.4,lon:16.9+i*.00001}))});
  assert.equal(walkableStops(many,[{stop:"0",walk:6}]).length,31,"all walkable places remain available, beyond a search's 20-result limit");
});

test("same-name interchange platforms connect without a parent station and retain explicit transfer rules",async()=>{
  for(const depth of[1,2])for(const[name,minutes]of[["journey-place-transfer.zip",depth===1?30:29],["journey-place-transfer-forbidden.zip",depth===1?35:29],["journey-place-transfer-minimum.zip",depth===1?35:29]]){
    const result=await searchJourneys(await loader(name),config(),{now:NOW,maxTransfers:depth});
    assert.equal(result.paths[0].arrival,NOW+minutes*60,name);
  }
});

test("deeper exploration finds a middle ride whose platforms do not appear on either endpoint trip",async()=>{
  const feed=await loader("journey-place-middle.zip");
  for(const depth of[1,2]){
    const result=await searchJourneys(feed,config(),{now:NOW,maxTransfers:depth});
    assert.equal(result.paths[0].arrival,NOW+(depth===1?35:29)*60);
  }
});

test("homonymous places stay separate by distance and declared station, with exact-ID fallback without coordinates",async()=>{
  for(const[name,minutes]of[["journey-places-far.zip",30],["journey-places-unlocated.zip",30],["journey-places-parents.zip",30],["journey-places-shared-parent.zip",22],["journey-place-transfer-far.zip",35]]){
    const feed=await loader(name),places=stopPlaces(feed.catalog);
    if(name.includes("shared-parent"))assert.equal(places.byId.get("A"),places.byId.get("A2"));
    else if(name.startsWith("journey-places"))assert.notEqual(places.byId.get("A"),places.byId.get("A2"));
    assert.equal((await searchJourneys(feed,config(),{now:NOW})).paths[0].arrival,NOW+minutes*60,name);
  }
});
test("dot positions preserve geographic order and remain finite for colocated stops and the dateline",async()=>{
  const {projectDots}=await import("../stop-dot-map.js");
  const points=[{point:{lat:52.4,lon:16.9}},{point:{lat:52.4,lon:16.92}},{point:{lat:52.4,lon:16.91}}];
  const dots=projectDots(points);assert.ok(dots[0].x<dots[2].x&&dots[2].x<dots[1].x);
  for(const p of projectDots([points[0],points[0]]))assert.ok(Number.isFinite(p.x)&&Number.isFinite(p.y));
  for(const p of projectDots([{point:{lat:0,lon:179.999}},{point:{lat:0,lon:-179.999}}]))assert.ok(p.x>=28&&p.x<=452&&p.y>=28&&p.y<=172);
});

test("overlapping halos keep one global partition for selected peers and intermediate changes",async()=>{
  for(const file of["journey-places-overlap.zip","journey-places-overlap-shuffled.zip"]){
    const feed=await loader(file),places=stopPlaces(feed.catalog);
    assert.equal(places.byId.get("A"),places.byId.get("A2"));
    assert.notEqual(places.byId.get("A3"),places.byId.get("A2"));
    const c=config();c.origins[0].stop="A3";
    assert.equal((await searchJourneys(feed,c,{now:NOW})).paths[0].arrival,NOW+35*60);
    c.origins[0].stop="A2";
    assert.equal((await searchJourneys(feed,c,{now:NOW})).paths[0].arrival,NOW+22*60);
  }
  assert.equal((await searchJourneys(await loader("journey-place-middle-overlap.zip"),config(),{now:NOW,maxTransfers:2})).paths[0].arrival,NOW+35*60);
  const unicode=stopPlaces({stops:[{id:"𐀀",name:"Same",lat:52.4,lon:16.9},{id:"",name:"Same",lat:52.4,lon:16.9001}]});
  assert.equal(unicode.byId.get("𐀀").stops[0].id,"","ID order follows Unicode scalar/UTF-8 ordering on Rust and JavaScript");
});

test("raw labels retain glyph inputs beyond bounded human display fields",async()=>{
  const feed=await loader("journey-places-glyphs.zip"),peer=feed.catalog.stops.find(s=>s.id==="A2"),interchange=feed.catalog.stops.find(s=>s.id==="X");
  assert.equal(peer.code,"");assert.equal(peer.platform_code,"番二三四五六七八九十");
  assert.ok(!interchange.name.includes("界"));assert.ok(interchange.place_name.includes("界"));
});


test("device eligibility covers every peer in the native name/station cohort",async()=>{
  const {deviceStop,validateJourneyPlaces}=await import("../stop-places.js");
  for(const peer of ["x".repeat(33),"界".repeat(11)])for(const distant of [false,true]){
    const places=stopPlaces({stops:[
      {id:"A",name:"Start",lat:52.4,lon:16.9},
      {id:peer,name:"Start",lat:distant?53.4:52.4,lon:16.9},
      {id:"Z",name:"Target",lat:52.5,lon:17},
    ]});
    assert.equal(deviceStop(places.byId.get("A")),undefined);
    assert.equal(deviceStop(places.byId.get("Z")).id,"Z");
    assert.throws(()=>validateJourneyPlaces({origins:[{stop:"A"}],targets:[{stop:"Z"}]},places),/32-byte/);
  }
  const parent=stopPlaces({stops:[{id:"A",name:"Platform",parent:"P".repeat(33)}]});
  assert.equal(deviceStop(parent.byId.get("A")),undefined);
});

test("glyph inputs match native bounded labels without hidden suffixes",async()=>{
  const {deviceLabel,journeyGlyphText}=await import("../journey-labels.js");
  assert.equal(deviceLabel(" \tA\u0000B\u0085界語 ",6),"AB 界");
  assert.equal(deviceLabel("A".repeat(61)+"界語",64),"A".repeat(61)+"界");
  assert.equal(deviceLabel("番二三四五六七八九十",16),"番二三四五");
  assert.equal(deviceLabel("A \u0000 B",4),"A  B");
  assert.deepEqual(journeyGlyphText({stops:[],routes:[{line:"display",route_name:"一二三"}]}),["·","一二"]);
  const feed=await loader("journey-places-glyphs.zip"),text=journeyGlyphText(feed.catalog).join("");
  for(const c of ["·","番","二","三","四","五","界"])assert.ok(text.includes(c),c);
  for(const c of ["一","耀","語","六"])assert.ok(!text.includes(c),c);
  assert.ok(new Set(feed.catalog.stops.flatMap(s=>[...(s.place_name+s.platform_code)])).size>4096);
});
