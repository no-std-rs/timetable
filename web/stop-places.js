// Stop identity retains accents and full feed names; the search is wider.
// This normalization also runs allocation-free in the Rust journey planner.
const placeWords = value => value.split(/[\t-\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+/u).filter(Boolean).join(" ");
export const placeName = value => Array.from(placeWords(value),c=>c.toLowerCase()).join("");
const indexes=new WeakMap();
export function stopPlaces(catalog) {
  if(catalog&&indexes.has(catalog))return indexes.get(catalog);
  const groups=[],byName=new Map(),byId=new Map();
  for(const stop of [...catalog?.stops??[],...catalog?.via_stops??[]]){
    if((stop.location_type??0)!==0)continue;
    const key=placeName(stop.place_name??stop.name);
    let group=byName.get(key);
    if(!group){group={key,name:stop.name,stops:[]};byName.set(key,group);groups.push(group);}
    group.stops.push(stop);byId.set(stop.id,group);
  }
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
        if(!best||walk<best.walk||walk===best.walk&&metres<best.metres)best={place,walk,extra,metres,via:anchor.origin.stop};
      }
    }
    if(best)suggestions.push(best);
  }
  return suggestions.sort((a,b)=>a.walk-b.walk||a.metres-b.metres||a.place.name.localeCompare(b.place.name));
}
