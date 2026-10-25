// Optional exploration on the user's computer. Device routing is independent:
// both firmware and Android run the bounded Rust planner on their own GTFS data.
import { stopPlaces } from "./stop-places.js";
import { validateJourney } from "./config.js";
const HORIZON=6*3600;
const fail=()=>{throw new Error("The timetable contains unsupported journey data.");};
function dateParts(zone,now){
  const formatter=new Intl.DateTimeFormat("en-CA",{timeZone:zone,year:"numeric",month:"2-digit",day:"2-digit"});
  const parts=Object.fromEntries(formatter.formatToParts(new Date(now*1000)).map(p=>[p.type,p.value]));
  const day=Date.UTC(Number(parts.year),Number(parts.month)-1,Number(parts.day));
  const offsets=new Intl.DateTimeFormat("en-GB",{timeZone:zone,timeZoneName:"longOffset"});
  const offset=at=>{
    const text=offsets.formatToParts(new Date(at*1000)).find(p=>p.type==="timeZoneName").value;
    if(text==="GMT")return 0;
    const m=/^GMT([+-])(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(text);if(!m)fail();
    return(m[1]==="-"?-1:1)*(Number(m[2])*3600+Number(m[3])*60+Number(m[4]??0));
  };
  return[-1,0,1].map(n=>{
    const d=new Date(day+n*86400000),noon=d.getTime()/1000+12*3600;
    const first=noon-offset(noon),epoch=noon-offset(first)-12*3600;
    return {date:d.toISOString().slice(0,10).replaceAll("-",""),weekday:(d.getUTCDay()+6)%7,epoch};
  });
}
const uint=(value,maximum=Number.MAX_SAFE_INTEGER)=>{
  if(!/^\d+$/.test(value??""))fail();const n=Number(value);if(!Number.isSafeInteger(n)||n>maximum)fail();return n;
};
const time=value=>{
  if(value===undefined||value==="")return null;
  const parts=value.split(":");if(parts.length!==3)fail();
  return uint(parts[0])*3600+uint(parts[1],59)*60+uint(parts[2],59);
};
function compare(a,b){return a.arrival-b.arrival||a.legs.length-b.legs.length||a.walk-b.walk;}

export async function searchJourneys(loader,settings,{now=Math.floor(Date.now()/1000),maxTransfers=settings.max_transfers,onProgress=()=>{}}={}){
  validateJourney(settings);
  if(!Number.isInteger(maxTransfers)||maxTransfers<0||maxTransfers>4||!Number.isSafeInteger(now))throw new Error("Invalid journey search depth or time.");
  const days=dateParts(loader.timezone,now),services=new Map();
  const service=id=>{if(!id)fail();if(!services.has(id))services.set(id,{active:0,removed:0});return services.get(id);};
  if(loader.zip.members.has("calendar.txt"))for await(const row of loader.rows("calendar.txt")){
    if(!/^\d{8}$/.test(row.start_date??"")||!/^\d{8}$/.test(row.end_date??"")||row.start_date>row.end_date)fail();
    const flags=["monday","tuesday","wednesday","thursday","friday","saturday","sunday"].map(day=>uint(row[day],1));
    days.forEach((day,i)=>{if(day.date>=row.start_date&&day.date<=row.end_date&&flags[day.weekday])service(row.service_id).active|=1<<i;});
  }
  if(loader.zip.members.has("calendar_dates.txt"))for await(const row of loader.rows("calendar_dates.txt")){
    const type=uint(row.exception_type,2);if(type<1)fail();
    days.forEach((day,i)=>{if(day.date===row.date){const s=service(row.service_id);if(type===1)s.active|=1<<i;else{s.active&=~(1<<i);s.removed|=1<<i;}}});
  }
  const stops=new Map(loader.catalog.stops.map(s=>[s.id,s]));
  const places=stopPlaces(loader.catalog);
  for(const stop of[...settings.origins,...settings.targets])if(!places.byId.has(stop.stop))throw new Error("A journey stop changed. Reload the network and select it again.");
  const calls=new Map(),identities=new Map(),untimed=new Map();let count=0,untimedCount=0;
  for await(const row of loader.rows("stop_times.txt")){
    const trip=loader.trips.get(row.trip_id);if(!trip)fail();
    const mask=services.get(trip.service_id)?.active??0;if(!mask)continue;
    const arrival=time(row.arrival_time),departure=time(row.departure_time);
    if(arrival!==null&&departure!==null&&arrival>departure)fail();
    const sequence=uint(row.stop_sequence),pickup=uint(row.pickup_type||"0",3),dropoff=uint(row.drop_off_type||"0",3);
    const stop=stops.get(row.stop_id);if(!stop)fail();
    if(arrival===null||departure===null){
      const list=untimed.get(row.trip_id)??[];
      list.push({sequence,arrival,departure,pickup,dropoff,stop,line:loader.catalog.routes[trip.route].line});untimed.set(row.trip_id,list);
      if(++untimedCount>300000)throw new Error("Browser journey search reached its memory budget. Choose a smaller transport feed.");
      continue;
    }
    for(let day=0;day<3;day++){
      if(!(mask&(1<<day)))continue;
      const a=days[day].epoch+arrival,d=days[day].epoch+departure;
      if(d<now||a>now+HORIZON)continue;
      const key=JSON.stringify([row.trip_id,day]);let list=calls.get(key);
      if(!list){list=[];calls.set(key,list);identities.set(key,{trip:row.trip_id,route:loader.catalog.routes[trip.route].id,day});}
      list.push({sequence,arrival:a,departure:d,pickup:d>now+HORIZON?1:pickup,dropoff:a<now?1:dropoff,stop,trip:key,line:loader.catalog.routes[trip.route].line});
      if(++count>300000)throw new Error("Browser journey search reached its memory budget. Choose a smaller transport feed.");
    }
  }
  for(const [key,list]of calls){
    const identity=identities.get(key),day=identity.day;
    // Untimed calls describe the route but cannot supply boarding/arrival
    // times. Attach them only to trip instances with calls in our window.
    for(const call of untimed.get(identity.trip)??[]){
      list.push({...call,trip:key,arrival:call.arrival===null?null:days[day].epoch+call.arrival,departure:call.departure===null?null:days[day].epoch+call.departure});
      if(++count>300000)throw new Error("Browser journey search reached its memory budget. Choose a smaller transport feed.");
    }
    list.sort((a,b)=>a.sequence-b.sequence);
    let lastTime=null;
    for(const [i,call]of list.entries()){
      const first=call.arrival??call.departure;
      if(i&&call.sequence===list[i-1].sequence||first!==null&&lastTime!==null&&first<lastTime)fail();
      lastTime=call.departure??call.arrival??lastTime;
    }
  }
  const scopeFields=["from_route_id","to_route_id","from_trip_id","to_trip_id"];
  const rules=new Map(),allRules=[];
  const specificity=r=>r.from_trip_id&&r.to_trip_id?6:
    r.from_trip_id&&r.to_route_id||r.from_route_id&&r.to_trip_id?5:
    r.from_trip_id||r.to_trip_id?4:r.from_route_id&&r.to_route_id?3:
    r.from_route_id||r.to_route_id?2:1;
  if(loader.zip.members.has("transfers.txt"))for await(const row of loader.rows("transfers.txt")){
    const type=uint(row.transfer_type||"0",5);
    const error=type>3?"Journey routing does not yet support in-seat transfer rules.":null;
    if(!row.from_stop_id||!row.to_stop_id){if(error)throw new Error(error);fail();}
    const seconds=type===2?uint(row.min_transfer_time):uint(row.min_transfer_time||"0");
    const scopes=Object.fromEntries(scopeFields.map(field=>[field,row[field]||""]));
    const key=JSON.stringify([row.from_stop_id,row.to_stop_id]),signature=JSON.stringify(scopes);
    const list=rules.get(key)??[];
    if(list.some(rule=>rule.signature===signature))fail();
    const rule={from:row.from_stop_id,to:row.to_stop_id,seconds,forbidden:type===3,error,signature,...scopes};
    list.push(rule);rules.set(key,list);allRules.push(rule);
  }
  const children=new Map();
  for(const stop of stops.values())if(stop.parent){
    if(!stops.has(stop.parent))fail();
    const list=children.get(stop.parent)??[];list.push(stop.id);children.set(stop.parent,list);
  }
  const applicable=(from,to,arriving,departing)=>{
    const a=identities.get(arriving),b=identities.get(departing);
    const keys=[[from,to],[from,stops.get(to)?.parent],[stops.get(from)?.parent,to],[stops.get(from)?.parent,stops.get(to)?.parent]];
    let chosen=null,score=-1;
    for(const[x,y]of keys)if(x&&y)for(const rule of rules.get(JSON.stringify([x,y]))??[]){
      if(rule.from_trip_id&&rule.from_trip_id!==a?.trip||rule.to_trip_id&&rule.to_trip_id!==b?.trip||
         rule.from_route_id&&rule.from_route_id!==a?.route||rule.to_route_id&&rule.to_route_id!==b?.route)continue;
      const rank=specificity(rule)*4+Number(rule.from===from)+Number(rule.to===to);
      if(rank===score&&chosen!==rule)fail();
      if(rank>score){chosen=rule;score=rank;}
    }
    return chosen;
  };
  const links=new Map();
  const connect=(from,to)=>{
    let incoming=links.get(to);if(!incoming){incoming=new Set();links.set(to,incoming);}
    incoming.add(from);
  };
  for(const place of places.groups)for(const from of place.stops)for(const to of place.stops)if(from!==to)connect(from.id,to.id);
  for(const siblings of children.values())for(const from of siblings)for(const to of siblings)if(from!==to)connect(from,to);
  for(const rule of allRules){
    const from=children.get(rule.from)??[rule.from],to=children.get(rule.to)??[rule.to];
    for(const a of from)for(const b of to)if(a!==b)connect(a,b);
  }
  const routeSegments=legs=>legs.map(leg=>[leg.route,leg.stops.map(call=>call.stop.id)]);
  // Keep every route prefix and arriving trip (for scoped transfer rules).
  // Earlier services of an identical prefix can share their onward ride.
  const offer=(map,stop,label)=>{
    let list=map.get(stop);if(!list){list=new Map();map.set(stop,list);}
    const key=JSON.stringify([label.origin.id,label.walk,routeSegments(label.legs),label.legs.at(-1)?.trip,label.error,!!label.conditional]),old=list.get(key);
    if(!old||compare(label,old)<0)list.set(key,label);
    return old?0:1;
  };
  const budget=()=>{throw new Error("Browser journey search reached its memory budget. Choose a smaller transport feed.");};
  let previous=new Map();
  for(const origin of settings.origins)for(const stop of places.byId.get(origin.stop).stops)offer(previous,stop.id,{arrival:now+origin.walk*60,walk:origin.walk,origin:stop,legs:[]});
  const best=new Map();
  // Repeated services of the same ordered route are one choice: show its
  // earliest feasible itinerary, while retaining every distinct stop pattern.
  const routeKey=path=>JSON.stringify([places.byId.get(path.origin.id).key,places.byId.get(path.target.id).key,routeSegments(path.legs)]);
  const publish=(depth,complete)=>({depth,complete,now,until:now+HORIZON,timezone:loader.timezone,paths:[...best.values()].sort(compare)});
  for(let depth=0;depth<=maxTransfers;depth++){
    const next=new Map();let labels=0;
    for(const list of calls.values()){
      const boardings=[];
      for(const [at,call] of list.entries()){
        if(call.arrival===null||call.departure===null)continue;
        if(call.dropoff!==1)for(const boarded of boardings){
          const leg={trip:call.trip,route:identities.get(call.trip).route,line:call.line,from:boarded.call.stop,to:call.stop,departure:boarded.call.departure,arrival:call.arrival,stops:list.slice(boarded.at,at+1).map(c=>({stop:c.stop,arrival:c.arrival,departure:c.departure}))};
          const label={...boarded.label,error:boarded.error,conditional:boarded.label.conditional||boarded.call.pickup>=2||call.dropoff>=2,arrival:call.arrival,legs:[...boarded.label.legs,leg]};
          labels+=offer(next,call.stop.id,label);if(labels>300000)budget();
        }
        if(call.pickup===1)continue;
        const options=[...(previous.get(call.stop.id)?.values()??[])].map(label=>({label,from:call.stop.id}));
        if(depth)for(const from of links.get(call.stop.id)??[])for(const label of previous.get(from)?.values()??[])options.push({label,from});
        for(const {label,from}of options){
          if(label.legs.at(-1)?.trip===call.trip)continue;
          const rule=depth?applicable(from,call.stop.id,label.legs.at(-1)?.trip,call.trip):null;
          if(rule?.forbidden)continue;
          const place=places.byId.get(from);
          const samePlace=from===call.stop.id||place&&place===places.byId.get(call.stop.id)||stops.get(from)?.parent&&stops.get(from).parent===call.stop.parent;
          if(depth&&!rule&&!samePlace)continue;
          // Timed guarantees do not waive the user's chosen minimum margin.
          const seconds=depth?Math.max(settings.transfer_minutes*60,rule?.seconds??0):0;
          if(label.arrival+seconds>call.departure)continue;
          boardings.push({at,call,label,error:label.error||rule?.error});
          if(boardings.length>300000)budget();
        }
      }
    }
    for(const target of settings.targets)for(const stop of places.byId.get(target.stop).stops)for(const label of next.get(stop.id)?.values()??[]){
      const candidate={...label,arrival:label.arrival+target.walk*60,target:stop,egress:target.walk};
      if(candidate.arrival<=now+HORIZON&&candidate.error)throw new Error(candidate.error);
      if(candidate.arrival<=now+HORIZON&&candidate.conditional)throw new Error("Journey routing does not yet support conditional boarding or alighting.");
      const key=routeKey(candidate);
      if(candidate.arrival<=now+HORIZON&&(!best.has(key)||compare(candidate,best.get(key))<0))best.set(key,candidate);
      if(best.size>300000)budget();
    }
    previous=next;onProgress(publish(depth,depth===maxTransfers));
    await new Promise(resolve=>setTimeout(resolve,0));
  }
  return publish(maxTransfers,true);
}
