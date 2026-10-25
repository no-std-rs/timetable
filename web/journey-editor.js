import { renderStopDots } from "./stop-dot-map.js";
import { searchText } from "./catalog-search.js";
import { stopPlaces, walkableStops, deviceStop, validateJourneyPlaces } from "./stop-places.js";
import { validateJourney } from "./config.js";
import { t } from "./i18n.js";
const node=(tag,text,className)=>{const n=document.createElement(tag);if(text!==undefined)n.textContent=text;if(className)n.className=className;return n;};
export class JourneyEditor {
  constructor({group,catalog,changed,preview}) {
    Object.assign(this,{group,catalog,changed,preview});this.generation=0;
    this.$=id=>document.getElementById(id);
    this.$("group-mode").addEventListener("change",()=>{
      if(this.$("group-mode").value==="journey")this.group().journey={origins:[],targets:[],transfer_minutes:3,max_transfers:1};
      else delete this.group().journey;
      this.render();this.changed();
    });
    for(const [id,key]of[["journey-slack","transfer_minutes"],["journey-depth","max_transfers"]])this.$(id).addEventListener("input",()=>{this.group().journey[key]=Number(this.$(id).value);this.invalidate();this.changed();});
    for(const kind of["origin","target"])this.$(`journey-${kind}-search`).addEventListener("input",()=>this.matches(kind));
    for(const id of["journey-quick-minutes"])this.$(id).addEventListener("input",()=>this.quickAdd());
    this.$("journey-preview").addEventListener("click",()=>void this.calculate());
  }
  invalidate(){this.generation++;this.$("journey-preview-status").textContent="";this.$("journey-preview-result").replaceChildren();this.$("journey-preview").disabled=false;}
  render(){
    this.invalidate();const j=this.group().journey;
    this.$("group-mode").value=j?"journey":"departures";
    this.$("journey-editor").hidden=!j;this.$("picks").hidden=!!j;
    this.$("add-pick").hidden=!!j||this.group().picks.length>=4;
    if(!j)return;
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
        input.addEventListener("input",()=>{s.walk=Number(input.value);this.invalidate();this.quickAdd();this.changed();});label.append(input);
        const remove=node("button",t("Remove"),"text-button");remove.type="button";remove.setAttribute("aria-label",`${t("Remove")}: ${name}`);
        remove.addEventListener("click",()=>{j[`${kind}s`].splice(i,1);this.render();this.changed();});row.append(info,label,remove);list.append(row);
      });
      this.$(`journey-${kind}-search`).disabled=j[`${kind}s`].length>=4||!this.catalog();
      this.$(`journey-${kind}-search`).value="";this.$(`journey-${kind}-matches`).replaceChildren();
    }
    this.quickAdd();
  }
  quickAdd(){
    const j=this.group().journey,places=stopPlaces(this.catalog());
    const point=origin=>places.byId.get(origin?.stop)?.stops.find(s=>Number.isFinite(s.lat)&&Number.isFinite(s.lon));
    const ready=j&&point(j.origins[0])&&point(j.origins[1]);
    this.$("journey-quick-add").hidden=!j||!point(j.origins[0]);
    const list=this.$("journey-quick-matches"),map=this.$("journey-quick-map");list.replaceChildren();map.replaceChildren();
    this.$("journey-quick-legend").textContent="";
    this.$("journey-quick-hint").textContent=t(ready?"Tap a dot to add or remove a starting stop.":"Choose two starting stops to see stops between them.");
    if(!ready){this.$("journey-quick-count").textContent="";return;}
    const suggestions=walkableStops(places,j.origins.slice(0,2),{mode:"between",extraMinutes:Number(this.$("journey-quick-minutes").value)});
    this.$("journey-quick-count").textContent=suggestions.length?t("{count} intermediate stops",{count:suggestions.length}):t("No more stops within this walking range.");
    this.$("journey-quick-legend").textContent=j.origins.slice(0,2).map((s,i)=>`${i+1} · ${places.byId.get(s.stop).name}`).join(" → ");
    const dots=j.origins.slice(0,2).map((s,i)=>({anchor:i+1,point:point(s),label:places.byId.get(s.stop).name}));
    const toggle=suggestion=>{
      const selected=j.origins.findIndex(s=>places.byId.get(s.stop)===suggestion.place);
      if(selected>=2)j.origins.splice(selected,1);
      else if(selected<0&&j.origins.length<4){
        const stop=deviceStop(suggestion.place);
        if(!stop)return;
        j.origins.push({stop:stop.id,stop_name:suggestion.place.name,stop_code:stop.code,walk:suggestion.walk});
      }
      this.render();this.changed();
    };
    for(const suggestion of suggestions){
      const stop=deviceStop(suggestion.place);
      const selected=j.origins.some(s=>places.byId.get(s.stop)===suggestion.place);
      const label=selected?t("Remove {stop}",{stop:suggestion.place.name}):t("Add {stop} · about {minutes} min walk",{stop:suggestion.place.name,minutes:suggestion.walk});
      const disabled=!selected&&(!stop||j.origins.length>=4);
      const button=node("button",label,"text-button");button.type="button";button.disabled=disabled;
      button.addEventListener("click",()=>toggle(suggestion));list.append(button);
      dots.push({point:suggestion.point,label,selected,disabled,suggestion});
    }
    renderStopDots(map,dots,{hint:this.$("journey-quick-hint"),activate:dot=>toggle(dot.suggestion)});
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
    this.invalidate();const generation=this.generation;
    const button=this.$("journey-preview"),status=this.$("journey-preview-status");
    try{
      const journey=structuredClone(validateJourneyPlaces(validateJourney(this.group().journey),stopPlaces(this.catalog())));button.disabled=true;
      status.textContent=t("Checking direct journeys…");
      const update=result=>{if(generation!==this.generation)return;this.result(result);};
      const result=await this.preview(journey,Number(this.$("journey-preview-depth").value),update);update(result);
    }catch(error){if(generation===this.generation)status.textContent=t(error.message);}
    finally{if(generation===this.generation)button.disabled=false;}
  }
  result(result){
    this.$("journey-preview-status").textContent=t(result.complete?"Search complete: checked up to {depth} transfers.":"Best found so far: checked up to {depth} transfers.",{depth:result.depth});
    const list=this.$("journey-preview-result");list.replaceChildren();
    if(!result.paths.length){list.append(node("p",t(result.complete?"No reachable journey within the search window.":"No journey found at this depth yet.")));return;}
    const format=at=>new Intl.DateTimeFormat(undefined,{timeZone:result.timezone,hour:"2-digit",minute:"2-digit"}).format(new Date(at*1000));
    for(const path of result.paths){
      const card=node("article",undefined,"journey-result");card.append(node("strong",`${t("Arrive")} ${format(path.arrival)} · ${Math.ceil((path.arrival-result.now)/60)} min`));
      card.append(node("p",`${path.walk} ${t("min walk")} · ${path.origin.name} → ${path.target.name}`));
      const legs=node("ol");path.legs.forEach((leg,i)=>{const wait=i?`${Math.floor((leg.departure-path.legs[i-1].arrival)/60)} ${t("min transfer")} · `:"";legs.append(node("li",`${wait}${leg.line} · ${format(leg.departure)}–${format(leg.arrival)} · ${leg.from.name}${leg.from.code?` · ${leg.from.code}`:""} → ${leg.to.name}${leg.to.code?` · ${leg.to.code}`:""}`));});card.append(legs);list.append(card);
    }
  }
}
