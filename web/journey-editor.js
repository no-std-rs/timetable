import { renderRoute } from "./route-diagram.js";
import { searchText } from "./catalog-search.js";
import { stopPlaces, nearestEndpoint, deviceStop, validateJourneyPlaces } from "./stop-places.js";
import { validateJourney } from "./config.js";
import { t } from "./i18n.js";
const node=(tag,text,className)=>{const n=document.createElement(tag);if(text!==undefined)n.textContent=text;if(className)n.className=className;return n;};
const SHOWN=8;
export class JourneyEditor {
  constructor({group,catalog,changed,preview,ready=()=>true}) {
    Object.assign(this,{group,catalog,changed,preview,ready});this.generation=0;
    this.$=id=>document.getElementById(id);
    this.$("group-mode").addEventListener("change",()=>{
      if(this.$("group-mode").value==="journey")this.group().journey={origins:[],targets:[],transfer_minutes:3,max_transfers:1};
      else delete this.group().journey;
      this.render();this.changed();
    });
    for(const [id,key]of[["journey-slack","transfer_minutes"],["journey-depth","max_transfers"]])this.$(id).addEventListener("input",()=>{this.group().journey[key]=Number(this.$(id).value);this.invalidate();this.changed();});
    for(const kind of["origin","target"])this.$(`journey-${kind}-search`).addEventListener("input",()=>this.matches(kind));
    this.$("journey-quick-minutes").addEventListener("input",()=>this.invalidate());
    this.$("journey-preview-depth").addEventListener("change",()=>this.invalidate());
    this.$("journey-preview").addEventListener("click",()=>void this.calculate());
    this.$("journey-routes-more").addEventListener("click",()=>{this.expanded=true;this.result(this.last);});
  }
  // Any edit invalidates shown routes. Once the network is loaded in this
  // browser, routes refresh automatically; the first search is a button.
  invalidate(){
    this.generation++;clearTimeout(this.timer);this.last=null;this.expanded=false;
    this.$("journey-preview-status").textContent="";this.$("journey-preview-result").replaceChildren();this.$("journey-routes-more").hidden=true;this.$("journey-route-hint").textContent="";
    const j=this.group().journey,shown=!!(j&&j.origins.length&&j.targets.length&&this.catalog());
    this.$("journey-routes").hidden=!shown;this.$("journey-preview").disabled=false;
    if(!shown)return;
    if(this.ready()){const generation=this.generation;this.timer=setTimeout(()=>{if(generation===this.generation)void this.calculate();},300);}
    else this.$("journey-preview-status").textContent=t("Find routes downloads this network's timetable once and searches it in your browser.");
  }
  render(){
    const j=this.group().journey;
    this.$("group-mode").value=j?"journey":"departures";
    this.$("journey-editor").hidden=!j;this.$("picks").hidden=!!j;
    this.$("add-pick").hidden=!!j||this.group().picks.length>=4;
    if(j){
      this.$("journey-slack").value=j.transfer_minutes;this.$("journey-depth").value=j.max_transfers;
      const places=stopPlaces(this.catalog());
      for(const kind of["origin","target"]){
        const list=this.$(`journey-${kind}s`);list.replaceChildren();
        j[`${kind}s`].forEach((s,i)=>{
          const live=this.catalog()?.stops.find(stop=>stop.id===s.stop);
          const place=places.byId.get(s.stop);
          const name=place?.name||s.stop_name||live?.name||s.stop;
          const code=place?.stops.length>1?t("{count} platforms",{count:place.stops.length}):s.stop_code||live?.code||s.stop;
          const row=node("div",undefined,"journey-stop");const info=node("span");info.append(node("strong",name),node("small",code));
          const label=node("label",t(kind==="origin"?"Walk there (min)":"Walk after arrival (min)"));
          const input=node("input");Object.assign(input,{type:"number",min:"0",max:"60",value:s.walk});input.setAttribute("aria-label",`${t("Walking time")}: ${name}`);
          input.addEventListener("input",()=>{s.walk=Number(input.value);this.invalidate();this.changed();});label.append(input);
          const remove=node("button",t("Remove"),"text-button");remove.type="button";remove.setAttribute("aria-label",`${t("Remove")}: ${name}`);
          remove.addEventListener("click",()=>{j[`${kind}s`].splice(i,1);this.render();this.changed();});row.append(info,label,remove);list.append(row);
        });
        this.$(`journey-${kind}-search`).disabled=j[`${kind}s`].length>=4||!this.catalog();
        this.$(`journey-${kind}-search`).value="";this.$(`journey-${kind}-matches`).replaceChildren();
      }
    }
    this.invalidate();
  }
  matches(kind){
    const list=this.$(`journey-${kind}-matches`);list.replaceChildren();
    const query=searchText(this.$(`journey-${kind}-search`).value).trim();if(!query)return;
    const terms=query.split(/\s+/),selected=this.group().journey[`${kind}s`];let count=0;
    const places=stopPlaces(this.catalog());
    for(const place of places.groups){
      if(kind==="origin"&&!place.stops.some(s=>places.boarding.has(s.id)))continue;
      if(selected.some(s=>places.byId.get(s.stop)===place)||!place.stops.some(stop=>terms.every(term=>searchText(`${place.name} ${stop.place_name??stop.name} ${stop.platform_code??stop.code} ${stop.id}`).includes(term))))continue;
      const stop=deviceStop(place);
      const detail=place.stops.length>1?t("{count} platforms",{count:place.stops.length}):place.stops[0].code||place.stops[0].id;
      const button=node("button",`${place.name} · ${detail}`,"text-button");button.type="button";
      button.disabled=!stop;
      if(!stop)button.title=t("This stop's platform IDs exceed the device's 32-byte limit. Choose another stop.");
      button.addEventListener("click",()=>{selected.push({stop:stop.id,stop_name:place.name,stop_code:stop.code,walk:kind==="origin"?6:0});this.render();this.changed();});list.append(button);
      if(++count===20)break;
    }
    if(!count)list.append(node("p",t("No matching stops."),"fine-print"));
  }
  async calculate(){
    clearTimeout(this.timer);this.generation++;const generation=this.generation;
    const button=this.$("journey-preview"),status=this.$("journey-preview-status");
    try{
      const journey=structuredClone(validateJourneyPlaces(validateJourney(this.group().journey),stopPlaces(this.catalog())));button.disabled=true;
      status.textContent=t("Checking direct journeys…");
      const update=result=>{if(generation!==this.generation)return;this.result(result);};
      const result=await this.preview(journey,Number(this.$("journey-preview-depth").value),update,Number(this.$("journey-quick-minutes").value)||0);update(result);
    }catch(error){if(generation===this.generation&&error.name!=="AbortError")status.textContent=t(error.message);}
    finally{if(generation===this.generation)button.disabled=false;}
  }
  // A tapped stop joins the nearer side (by estimated walk) or leaves it.
  plan(place,fallback){
    const j=this.group().journey,places=stopPlaces(this.catalog());
    const near={origins:nearestEndpoint(places,place,j.origins),targets:nearestEndpoint(places,place,j.targets)};
    const kind=near.origins&&near.targets?(near.origins.metres<=near.targets.metres?"origins":"targets"):near.origins?"origins":near.targets?"targets":fallback;
    if(kind==="origins"&&!place.stops.some(s=>places.boarding.has(s.id)))return {error:"No boarding at this stop"};
    if(j[kind].length>=4)return {error:kind==="origins"?"Starting stops are full (4)":"Target stops are full (4)"};
    const stop=deviceStop(place);if(!stop)return {error:"This stop's platform IDs exceed the device's 32-byte limit. Choose another stop."};
    return {kind,entry:{stop:stop.id,stop_name:place.name,stop_code:stop.code,walk:Math.min(60,near[kind]?.walk??j[kind][0]?.walk??0)}};
  }
  describe(token,format){
    const j=this.group().journey,places=stopPlaces(this.catalog()),call=token.call,place=places.byId.get(call.stop.id);
    const time=token.board?call.departure:call.arrival??call.departure,code=call.stop.platform_code||call.stop.code;
    const base=`${call.stop.name}${code?` · ${code}`:""} · ${time===null?t("Time not provided"):format(time)}${token.request?` · ${t("request stop")}`:""}`;
    if(!place)return {label:`${base} · ${t("Not a boarding stop")}`,selected:false,disabled:true};
    for(const kind of["origins","targets"]){
      if(!j[kind].some(s=>places.byId.get(s.stop)===place))continue;
      const last=j[kind].length<2;
      return {label:`${base} · ${t(kind==="origins"?last?"Your only starting stop":"Your starting stop · tap to remove":last?"Your only target stop":"Your target stop · tap to remove")}`,selected:true,disabled:last};
    }
    const plan=this.plan(place,token.leg===0?"origins":"targets");
    if(plan.error)return {label:`${base} · ${t(plan.error)}`,selected:false,disabled:true};
    return {label:`${base} · ${t(plan.kind==="origins"?"Add as a starting stop · about {minutes} min walk":"Add as a target · about {minutes} min walk after arrival",{minutes:plan.entry.walk})}`,selected:false,disabled:false};
  }
  toggle(token){
    const j=this.group().journey,places=stopPlaces(this.catalog()),place=places.byId.get(token.call.stop.id);if(!place)return;
    for(const kind of["origins","targets"]){
      const at=j[kind].findIndex(s=>places.byId.get(s.stop)===place);
      if(at<0)continue;
      if(j[kind].length>1){j[kind].splice(at,1);this.render();this.changed();}
      return;
    }
    const plan=this.plan(place,token.leg===0?"origins":"targets");if(plan.error)return;
    j[plan.kind].push(plan.entry);this.render();this.changed();
  }
  // Routes from estimated nearby stops name what the device still needs.
  missing(path){
    const j=this.group().journey,places=stopPlaces(this.catalog()),entries=[];
    for(const [kind,end]of[["origins",path.start],["targets",path.end]]){
      if(end.configured)continue;
      const place=places.byId.get(end.stop),stop=deviceStop(place);
      entries.push({kind,name:end.name,walk:end.walk,entry:stop&&{stop:stop.id,stop_name:place.name,stop_code:stop.code,walk:end.walk}});
    }
    const full=["origins","targets"].some(kind=>j[kind].length+entries.filter(e=>e.kind===kind).length>4);
    return {entries,possible:!full&&entries.every(e=>e.entry)};
  }
  result(result){
    this.last=result;
    this.$("journey-preview-status").textContent=t(result.complete?"Search complete: checked up to {depth} transfers.":"Best found so far: checked up to {depth} transfers.",{depth:result.depth});
    const list=this.$("journey-preview-result");list.replaceChildren();this.$("journey-routes-more").hidden=true;
    if(!result.paths.length){list.append(node("p",t(result.complete?"No reachable journey within the search window.":"No journey found at this depth yet.")));return;}
    const format=at=>new Intl.DateTimeFormat(undefined,{timeZone:result.timezone,hour:"2-digit",minute:"2-digit"}).format(new Date(at*1000));
    list.append(node("p",t("{count} route alternatives · {from}–{until} · up to {depth} transfers",{count:result.paths.length,from:format(result.now),until:format(result.until),depth:result.depth}),"fine-print"));
    const width=Math.max(260,list.clientWidth-34||560);
    for(const path of this.expanded?result.paths:result.paths.slice(0,SHOWN)){
      const card=node("article",undefined,"journey-result journey-route"),leave=path.legs[0].departure-path.walk*60;
      const head=node("p",undefined,"journey-route-head");
      head.append(node("strong",`${t("Arrive")} ${format(path.arrival)} · ${Math.ceil((path.arrival-result.now)/60)} min`),node("span",t("Leave {time}",{time:format(leave)})));
      card.append(head);
      const steps=[];
      if(path.walk)steps.push(t("{minutes} min walk to {stop}",{minutes:path.walk,stop:path.start.name}));
      path.legs.forEach((leg,i)=>{
        if(i)steps.push(t("{minutes} min change",{minutes:Math.floor((leg.departure-path.legs[i-1].arrival)/60)}));
        const request=flag=>flag?` (${t("request stop")})`:"";
        steps.push(`${leg.line} ${format(leg.departure)} ${leg.from.name}${request(leg.requestBoard)} → ${format(leg.arrival)} ${leg.to.name}${request(leg.requestAlight)}`);
      });
      if(path.egress)steps.push(t("then {minutes} min walk",{minutes:path.egress}));
      card.append(node("p",steps.join(" · "),"journey-route-summary"));
      const {entries,possible}=this.missing(path),state=node("p",undefined,"journey-route-state");
      if(!entries.length)state.append(node("span",t("Uses your stops"),"journey-route-badge"));
      else{
        state.append(node("span",t("Add {stops} to use this route",{stops:entries.map(e=>t(e.kind==="origins"?"{stop} (start, {minutes} min walk)":"{stop} (target, {minutes} min walk after)",{stop:e.name,minutes:e.walk})).join(", ")})));
        const add=node("button",t("Add these stops"),"text-button route-action");add.type="button";add.disabled=!possible;
        if(!possible)add.title=t("Remove a stop first: devices allow four starting and four target stops.");
        add.addEventListener("click",()=>{const j=this.group().journey;for(const e of entries)j[e.kind].push(e.entry);this.render();this.changed();});
        state.append(add);
      }
      card.append(state);
      const map=document.createElementNS("http://www.w3.org/2000/svg","svg");map.setAttribute("class","journey-route-map");map.setAttribute("role","group");
      map.setAttribute("aria-label",t("Every stop on this route"));
      renderRoute(map,path,{width,hint:this.$("journey-route-hint"),describe:token=>this.describe(token,format),activate:token=>this.toggle(token)});
      card.append(map);
      const details=node("details");details.append(node("summary",t("Stop list")));
      const legs=node("ol");
      path.legs.forEach((leg,i)=>{
        const wait=i?`${Math.floor((leg.departure-path.legs[i-1].arrival)/60)} ${t("min transfer")} · `:"";
        const ride=node("li");
        ride.append(node("strong",`${wait}${leg.line} · ${format(leg.departure)}–${format(leg.arrival)}`));
        const calls=node("ol",undefined,"journey-calls");
        for(const [at,call]of leg.stops.entries()){
          const action=at===0?t("Board here"):at===leg.stops.length-1?t("Alight here"):t("Pass through");
          const time=at===0?call.departure:call.arrival??call.departure,code=call.stop.platform_code||call.stop.code;
          const request=(at===0?leg.requestBoard:at===leg.stops.length-1?leg.requestAlight:call.pickup>=2||call.dropoff>=2)?` · ${t("request stop")}`:"";
          calls.append(node("li",`${time===null?t("Time not provided"):format(time)} · ${call.stop.name}${code?` · ${code}`:""} · ${action}${request}`));
        }
        ride.append(calls);legs.append(ride);
      });
      details.append(legs);card.append(details);list.append(card);
    }
    if(!this.expanded&&result.paths.length>SHOWN){const more=this.$("journey-routes-more");more.hidden=false;more.textContent=t("Show {count} more routes",{count:result.paths.length-SHOWN});}
  }
}
