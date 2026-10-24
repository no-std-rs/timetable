import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { FeedLoader } from "../feed-loader.js";
import { searchJourneys } from "../journey-search.js";
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
  for(const [file,minutes]of[["journey-direct.zip",28],["journey-forbidden.zip",35],["journey-minimum.zip",35],["journey-shuffled.zip",30],["journey-station.zip",30],["journey-station-forbidden.zip",35]]){
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
