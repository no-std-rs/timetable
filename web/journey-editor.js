import { searchText } from "./catalog-search.js";
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
    for(const kind of["origin","target"]){
      const list=this.$(`journey-${kind}s`);list.replaceChildren();
      j[`${kind}s`].forEach((s,i)=>{
        const live=this.catalog()?.stops.find(stop=>stop.id===s.stop);
        const name=s.stop_name||live?.name||s.stop,code=s.stop_code||live?.code||s.stop;
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
  matches(kind){
    const list=this.$(`journey-${kind}-matches`);list.replaceChildren();
    const query=searchText(this.$(`journey-${kind}-search`).value).trim();if(!query)return;
    const terms=query.split(/\s+/),selected=this.group().journey[`${kind}s`];let count=0;
    for(const stop of this.catalog()?.stops??[]){
      if(stop.location_type>0||selected.some(s=>s.stop===stop.id)||!terms.every(term=>searchText(`${stop.name} ${stop.code} ${stop.id}`).includes(term)))continue;
      const button=node("button",`${stop.name} · ${stop.code||stop.id}`,"text-button");button.type="button";
      button.disabled=new TextEncoder().encode(stop.id).length>32;
      button.addEventListener("click",()=>{selected.push({stop:stop.id,stop_name:stop.name,stop_code:stop.code,walk:kind==="origin"?6:0});this.render();this.changed();});list.append(button);
      if(++count===20)break;
    }
    if(!count)list.append(node("p",t("No matching stops."),"fine-print"));
  }
  async calculate(){
    this.invalidate();const generation=this.generation;
    const button=this.$("journey-preview"),status=this.$("journey-preview-status");
    try{
      const journey=structuredClone(validateJourney(this.group().journey));button.disabled=true;
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
      const legs=node("ol");path.legs.forEach((leg,i)=>{const wait=i?`${Math.floor((leg.departure-path.legs[i-1].arrival)/60)} ${t("min transfer")} · `:"";legs.append(node("li",`${wait}${leg.line} · ${format(leg.departure)}–${format(leg.arrival)} · ${leg.from.name} → ${leg.to.name}`));});card.append(legs);list.append(card);
    }
  }
}
