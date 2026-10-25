// Stop identity retains accents and full feed names; the search is wider.
// This normalization also runs allocation-free in the Rust journey planner.
const placeWords = value => value.split(/[\t-\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+/u).filter(Boolean).join(" ");
export const placeName = value => Array.from(placeWords(value),c=>c.toLowerCase()).join("");
const indexes=new WeakMap();
export function stopPlaces(catalog) {
  if(catalog&&indexes.has(catalog))return indexes.get(catalog);
  const groups=[],byName=new Map(),byId=new Map();
  const stops=[...catalog?.stops??[],...catalog?.via_stops??[]];
  const stations=new Map(stops.map(s=>[s.id,s]));
  for(const stop of [...stops].sort((a,b)=>idOrder(a.id,b.id))){
    if((stop.location_type??0)!==0)continue;
    const name=placeName(stop.place_name??stop.name);
    const identity=stop.parent?`parent:${stop.parent}`:position(stop)?`name:${name}`:`id:${stop.id}`;
    const choices=byName.get(identity)??[];
    let group=choices.find(g=>stop.parent||!position(stop)||nearPlace(g.stops[0],stop));
    if(!group){group={key:`place:${stop.parent||stop.id}`,name:stations.get(stop.parent)?.name||stop.name,stops:[]};choices.push(group);byName.set(identity,choices);groups.push(group);}
    group.stops.push(stop);byId.set(stop.id,group);
  }
  // Native partitioning indexes the entire name/station cohort, including
  // distant homonyms, before deciding which platforms share a physical place.
  for(const choices of byName.values()){
    const supported=choices.every(g=>g.stops.every(s=>fitsId(s.id)&&(!s.parent||fitsId(s.parent))));
    for(const group of choices)group.device_supported=supported;
  }
  const compare=new Intl.Collator(catalog?.language||"pl",{numeric:true,sensitivity:"base"}).compare;
  groups.sort((a,b)=>compare(a.name,b.name));
  const boarding=catalog?.links?new Set(catalog.links.map(([,i])=>catalog.stops[i]?.id)):new Set(byId.keys());
  const index={groups,byId,boarding};
  if(catalog)indexes.set(catalog,index);
  return index;
}

// Older drafts may contain several platforms of one place. One walk now applies
// to the place; keep the shortest configured walk instead of hiding a fast option.
export function coalesceJourneyStops(journey,places){
  let changed=false;
  for(const kind of ["origins","targets"]){
    const seen=new Map(),merged=[];
    for(const stop of journey[kind]){
      const key=places.byId.get(stop.stop)??stop.stop;
      const previous=seen.get(key);
      if(previous){previous.walk=Math.min(previous.walk,stop.walk);changed=true;}
      else{seen.set(key,stop);merged.push(stop);}
    }
    if(merged.length!==journey[kind].length)journey[kind]=merged;
  }
  return changed;
}

const radians=value=>value*Math.PI/180;
const longitude=(a,b)=>(b-a+540)%360-180;
const position=stop=>Number.isFinite(stop.lat)&&Number.isFinite(stop.lon)&&Math.abs(stop.lat)<=90&&Math.abs(stop.lon)<=180;
export function distanceMetres(a,b){
  const dlat=radians(b.lat-a.lat),dlon=radians(longitude(a.lon,b.lon));
  const h=Math.sin(dlat/2)**2+Math.cos(radians(a.lat))*Math.cos(radians(b.lat))*Math.sin(dlon/2)**2;
  return 6371000*2*Math.asin(Math.sqrt(Math.min(1,Math.max(0,h))));
}
function between(point,a,b){
  const project=stop=>[radians(longitude(a.lon,stop.lon))*Math.cos(radians((a.lat+b.lat)/2))*6371000,radians(stop.lat-a.lat)*6371000];
  const [x,y]=project(point),[dx,dy]=project(b),length=dx*dx+dy*dy;
  if(length<1)return false;
  const t=(x*dx+y*dy)/length;
  return t>=0&&t<=1&&Math.hypot(x-t*dx,y-t*dy)<=150;
}
// Estimates follow a known walk to a chosen start, then a straight line at
// 75 m/min. They are editable access walks, never inferred transfer links.
export function walkableStops(places,origins,{mode="nearby",extraMinutes=10}={}){
  if(!Number.isFinite(extraMinutes)||extraMinutes<0||extraMinutes>60)return [];
  const anchors=origins.slice(0,mode==="between"?2:1).map(origin=>({origin,stops:places.byId.get(origin.stop)?.stops.filter(position)??[]}));
  if(!anchors[0]?.stops.length||mode==="between"&&(anchors.length<2||!anchors[1].stops.length))return [];
  const chosen=new Set(origins.map(s=>places.byId.get(s.stop)));
  const suggestions=[];
  for(const place of places.groups){
    if(chosen.has(place)||!place.stops.some(s=>places.boarding.has(s.id)))continue;
    let best=null;
    for(const stop of place.stops.filter(position)){
      if(mode==="between"&&!anchors[0].stops.some(a=>anchors[1].stops.some(b=>between(stop,a,b))))continue;
      for(const anchor of anchors)for(const from of anchor.stops){
        const metres=distanceMetres(from,stop),extra=Math.ceil(metres/75),walk=anchor.origin.walk+extra;
        if(extra>extraMinutes||walk>60)continue;
        if(!best||walk<best.walk||walk===best.walk&&metres<best.metres)best={place,walk,extra,metres,via:anchor.origin.stop,point:stop};
      }
    }
    if(best)suggestions.push(best);
  }
  return suggestions.sort((a,b)=>a.walk-b.walk||a.metres-b.metres||a.place.name.localeCompare(b.place.name));
}

function idOrder(a,b){
  let i=0,j=0;
  while(i<a.length&&j<b.length){const x=a.codePointAt(i),y=b.codePointAt(j);if(x!==y)return x-y;i+=x>65535?2:1;j+=y>65535?2:1;}
  return a.length-b.length;
}
function nearPlace(a,b){
  const la=Math.trunc(a.lat*1e6),lb=Math.trunc(b.lat*1e6),lo=Math.trunc(a.lon*1e6),lp=Math.trunc(b.lon*1e6);
  const radians=(la+lb)*Math.PI/360000000,x2=radians*radians;
  const cos=1+x2*(-.5+x2*(1/24+x2*(-1/720+x2*(1/40320-x2/3628800))));
  let delta=Math.abs(lo-lp);if(delta>180000000)delta=360000000-delta;
  const dx=delta*(111195/1e6)*cos,dy=(la-lb)*(111195/1e6);
  return dx*dx+dy*dy<=125*125;
}

const fitsId=value=>new TextEncoder().encode(value).length<=32;
export const deviceStop=place=>place?.device_supported?place.stops[0]:undefined;
export function validateJourneyPlaces(journey,places){
  for(const endpoint of [...journey.origins,...journey.targets]){
    const place=places.byId.get(endpoint.stop);
    if(place&&!place.device_supported)throw new Error("This stop's platform IDs exceed the device's 32-byte limit. Choose another stop.");
  }
  return journey;
}
