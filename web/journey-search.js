// Optional exploration on the user's computer. Device routing is independent:
// both firmware and Android run the bounded Rust planner on their own GTFS data.
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
  for(const stop of[...settings.origins,...settings.targets])if(!stops.has(stop.stop))throw new Error("A journey stop changed. Reload the network and select it again.");
  const calls=new Map();let count=0;
  for await(const row of loader.rows("stop_times.txt")){
    const trip=loader.trips.get(row.trip_id);if(!trip)fail();
    const mask=services.get(trip.service_id)?.active??0;if(!mask)continue;
    const arrival=time(row.arrival_time),departure=time(row.departure_time);
    if(arrival===null||departure===null)continue;if(arrival>departure)fail();
    const sequence=uint(row.stop_sequence),pickup=uint(row.pickup_type||"0",3),dropoff=uint(row.drop_off_type||"0",3);
    const stop=stops.get(row.stop_id);if(!stop)fail();
    for(let day=0;day<3;day++){
      if(!(mask&(1<<day)))continue;
      const a=days[day].epoch+arrival,d=days[day].epoch+departure;
      if(d<now||a>now+HORIZON)continue;
      const key=JSON.stringify([row.trip_id,day]);let list=calls.get(key);
      if(!list){list=[];calls.set(key,list);}
      list.push({sequence,arrival:a,departure:d,pickup:d>now+HORIZON?1:pickup,dropoff:a<now?1:dropoff,stop,trip:key,line:loader.catalog.routes[trip.route].line});
      if(++count>300000)throw new Error("Browser journey search reached its memory budget. Choose a smaller transport feed.");
    }
  }
  for(const list of calls.values()){
    list.sort((a,b)=>a.sequence-b.sequence);
    if(list.some((c,i)=>i&&(c.sequence===list[i-1].sequence||c.arrival<list[i-1].departure)))fail();
  }
  const rules=new Map();
  if(loader.zip.members.has("transfers.txt"))for await(const row of loader.rows("transfers.txt")){
    const scoped=[row.from_route_id,row.to_route_id,row.from_trip_id,row.to_trip_id].some(Boolean);
    const type=uint(row.transfer_type||"0",5);
    const error=scoped?"Journey routing does not yet support route-specific transfers.":type>3?"Journey routing does not yet support in-seat transfer rules.":null;
    const seconds=error?0:type===2?uint(row.min_transfer_time):uint(row.min_transfer_time||"0");
    const key=JSON.stringify([row.from_stop_id,row.to_stop_id]),old=rules.get(key);
    if(old&&!old.error&&!error)fail();
    // Unsupported scoped/in-seat rules matter only if a reachable label tries
    // to use their stop pair. Keep their identity without disabling remote trips.
    const rule={from:row.from_stop_id,to:row.to_stop_id,seconds,forbidden:!error&&type===3,error:error||old?.error};
    if(rule.error)rule.forbidden=false;
    rules.set(key,rule);
  }
  const children=new Map();
  for(const stop of stops.values())if(stop.parent){
    if(!stops.has(stop.parent))fail();
    const list=children.get(stop.parent)??[];list.push(stop.id);children.set(stop.parent,list);
  }
  const applicable=(from,to)=>{
    const keys=[[from,to],[from,stops.get(to)?.parent],[stops.get(from)?.parent,to],[stops.get(from)?.parent,stops.get(to)?.parent]];
    let chosen=null,error=null;
    for(const[a,b]of keys)if(a&&b){const rule=rules.get(JSON.stringify([a,b]));if(rule){chosen??=rule;error??=rule.error;}}
    return error?{...chosen,error,forbidden:false}:chosen;
  };
  const links=new Map();
  const connect=(from,to)=>{
    const rule=applicable(from,to);if(rule?.forbidden)return;
    let incoming=links.get(to);if(!incoming){incoming=new Map();links.set(to,incoming);}
    incoming.set(from,{seconds:rule?.error?0:Math.max(settings.transfer_minutes*60,rule?.seconds??0),error:rule?.error});
  };
  for(const siblings of children.values())for(const from of siblings)for(const to of siblings)if(from!==to)connect(from,to);
  for(const rule of rules.values()){
    const from=children.get(rule.from)??[rule.from],to=children.get(rule.to)??[rule.to];
    for(const a of from)for(const b of to)if(a!==b)connect(a,b);
  }
  let previous=new Map();
  for(const origin of settings.origins)previous.set(origin.stop,{arrival:now+origin.walk*60,walk:origin.walk,origin:stops.get(origin.stop),legs:[]});
  const best=new Map();
  const publish=(depth,complete)=>({depth,complete,now,timezone:loader.timezone,paths:[...best.values()].sort(compare)});
  for(let depth=0;depth<=maxTransfers;depth++){
    const next=new Map();
    for(const list of calls.values()){
      let boarded=null;
      for(const call of list){
        if(boarded&&call.dropoff!==1&&call.sequence>boarded.call.sequence){
          const label={...boarded.label,conditional:boarded.label.conditional||boarded.call.pickup>=2||call.dropoff>=2,arrival:call.arrival,legs:[...boarded.label.legs,{trip:call.trip,line:call.line,from:boarded.call.stop,to:call.stop,departure:boarded.call.departure,arrival:call.arrival}]};
          if(!next.has(call.stop.id)||compare(label,next.get(call.stop.id))<0)next.set(call.stop.id,label);
        }
        if(call.pickup===1)continue;
        const options=[];const same=previous.get(call.stop.id);
        const sameRule=applicable(call.stop.id,call.stop.id);
        if(same&&(!depth||!sameRule?.forbidden))options.push({label:same,seconds:depth&&!sameRule?.error?Math.max(settings.transfer_minutes*60,sameRule?.seconds??0):0,error:depth?sameRule?.error:null});
        if(depth)for(const[from,link]of links.get(call.stop.id)??[]){
          const label=previous.get(from);if(label)options.push({label,...link});
        }
        for(const {label,seconds,error}of options){
          if(label.arrival+seconds>call.departure||label.legs.at(-1)?.trip===call.trip)continue;
          if(error)throw new Error(error);
          // Every previous label has exactly `depth` rides in this round.
          if(!boarded||label.walk<boarded.label.walk)boarded={call,label};
        }
      }
    }
    for(const target of settings.targets){
      const label=next.get(target.stop);if(!label)continue;
      const candidate={...label,arrival:label.arrival+target.walk*60,target:stops.get(target.stop),egress:target.walk};
      if(candidate.arrival<=now+HORIZON&&candidate.conditional)throw new Error("Journey routing does not yet support conditional boarding or alighting.");
      if(candidate.arrival<=now+HORIZON&&(!best.has(target.stop)||compare(candidate,best.get(target.stop))<0))best.set(target.stop,candidate);
    }
    previous=next;onProgress(publish(depth,depth===maxTransfers));
    // Let the optional exploration UI paint its completed-depth result.
    await new Promise(resolve=>setTimeout(resolve,0));
  }
  return publish(maxTransfers,true);
}
