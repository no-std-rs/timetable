import { directoryIndex, DEFAULT_FEED } from "./feed-directory.js";
import { csvRows } from "./csv-stream.js";
import { t, language } from "./i18n.js";
const $=id=>document.getElementById(id);
export class NetworkPicker {
  constructor(onLoad){this.onLoad=onLoad;this.selected=DEFAULT_FEED;
    for(const id of ["network-country","network-city","network-search"])$(id).addEventListener(id==="network-search"?"input":"change",()=>this.render(id));
    $("network-feed").addEventListener("change",()=>this.status());
    $("network-load").addEventListener("click",()=>{const feed=this.index?.feeds.find(f=>f.id===$("network-feed").value);if(feed&&this.index.available(feed))this.onLoad(feed);});
    $("network-refresh-directory").addEventListener("click",()=>this.load(true));
  }
  async load(refresh=false,selected=this.selected){
    this.selected=selected;$("network-refresh-directory").disabled=true;
    try{
      const response=await fetch(refresh?"https://files.mobilitydatabase.org/feeds_v2.csv":"./feeds.json",{credentials:"omit"});
      if(!response.ok)throw new Error("Could not load the network directory.");
      let data;
      if(refresh){
        const feeds=[];for await(const r of csvRows(response.body))if(r.data_type==="gtfs")feeds.push({id:r.id,country:r["location.country_code"],city:r["location.municipality"],region:r["location.subdivision_name"],provider:r.provider,name:r.name,status:r.status,mirror:r["urls.latest"],original:r["urls.direct_download"],authentication:r["urls.authentication_type"],license:r["urls.license"]});
        data={feeds,date:new Date().toISOString().slice(0,10)};
      }else data=await response.json();
      this.data=data;this.index=directoryIndex(data,language());this.render("initial");
      $("network-directory-state").textContent=t("{count} timetable feeds · directory {date}",{count:data.feeds.length,date:data.date});
    }catch{ $("network-directory-state").textContent=t("Could not load the network directory."); }
    finally{$("network-refresh-directory").disabled=false;}
  }
  render(trigger){
    if(!this.data)return;
    this.index=directoryIndex(this.data,language());
    $("network-directory-state").textContent=t("{count} timetable feeds · directory {date}",{count:this.data.feeds.length,date:this.data.date});
    const current=this.index.feeds.find(f=>f.id===this.selected),country=$("network-country"),city=$("network-city"),feed=$("network-feed");
    const countryValue=trigger==="initial"?current?.country??"*":country.value;
    country.replaceChildren(new Option(t("All countries"),"*"),...this.index.countries().map(code=>new Option(this.index.countryName(code)||t("Unspecified country"),code)));
    country.value=countryValue;
    const cityValue=trigger==="initial"?current?this.index.locality(current):"*":trigger==="network-country"?"*":city.value;
    city.replaceChildren(new Option(t("All cities and regions"),"*"),...this.index.cities(country.value==="*"?null:country.value).map(name=>new Option(name||t("Regional / unspecified"),name)));
    city.value=cityValue;if(city.selectedIndex<0)city.value="*";
    const selected=trigger==="initial"?this.selected:feed.value;
    const matches=this.index.search(country.value==="*"?null:country.value,city.value==="*"?null:city.value,$("network-search").value);
    feed.replaceChildren(new Option(t("Choose a transport network"),""),...matches.map(f=>{
      const suffix=this.index.available(f)?"":` · ${t(f.mirror?"Unavailable timetable":"No public ZIP mirror")}`;
      return new Option(`${f.provider}${f.name?` · ${f.name}`:""} · ${f.id}${suffix}`,f.id);
    }));
    feed.value=matches.some(f=>f.id===selected)?selected:"";this.status();
  }
  status(){
    const feed=this.index?.feeds.find(f=>f.id===$("network-feed").value);
    $("network-load").disabled=!feed||!this.index.available(feed);
    $("network-choice-state").textContent=feed
      ?t("{city} · {provider}",{city:this.index.locality(feed)||t("Regional / unspecified"),provider:feed.provider})
        +(!this.index.available(feed)?` · ${t(feed.mirror?"Unavailable timetable":"No public ZIP mirror")}`:"")
      :t("Choose a network, then load its lines and stops.");
    $("network-source").hidden=!feed;$("network-source").href=feed?`https://mobilitydatabase.org/feeds/gtfs/${encodeURIComponent(feed.id)}`:"https://mobilitydatabase.org/";
  }
}
export class FeedClient {
  constructor(progress){this.worker=new Worker(new URL("./feed-worker.js",import.meta.url),{type:"module"});this.jobs=new Map();this.routeJobs=new Map();this.id=0;this.progress=progress;
    this.worker.onmessage=({data})=>{const job=this.jobs.get(data.request);if(!job)return;if(data.progress){this.progress(data.progress,data.count);return;}this.jobs.delete(data.request);if(data.error)job.reject(new Error(data.error));else job.resolve(data);};
    this.worker.onerror=()=>this.close(new Error("The timetable worker stopped. Retry the selected network."));
  }
  request(kind,values){const request=++this.id;return new Promise((resolve,reject)=>{this.jobs.set(request,{resolve,reject});this.worker.postMessage({request,kind,...values});});}
  load(feed){return this.request("load",{feed});}
  paths(route){
    if(!this.routeJobs.has(route))this.routeJobs.set(route,this.request("paths",{route}).finally(()=>this.routeJobs.delete(route)));
    return this.routeJobs.get(route);
  }
  close(error=new DOMException("Network selection changed","AbortError")){this.worker.terminate();for(const job of this.jobs.values())job.reject(error);this.jobs.clear();}
}
