import { FeedLoader, downloadFeed } from "./feed-loader.js";
let loader;
function database() {
  return new Promise((resolve,reject)=>{
    const request=indexedDB.open("timetable.public-feeds",1);
    request.onupgradeneeded=()=>request.result.createObjectStore("feeds",{keyPath:"id"});
    request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
  });
}
async function cached(id) {
  let db;try{
    db=await database();
    const row=await new Promise((resolve,reject)=>{const r=db.transaction("feeds").objectStore("feeds").get(id);r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
    if(!row||row.saved>Date.now()||Date.now()-row.saved>=3*3600000||row.blob.size>512*1024*1024)return null;
    return new Uint8Array(await row.blob.arrayBuffer());
  }catch{return null;}finally{db?.close();}
}
async function remember(id,bytes) {
  let db;try{
    db=await database();
    await new Promise((resolve,reject)=>{const tx=db.transaction("feeds","readwrite");tx.objectStore("feeds").put({id,saved:Date.now(),blob:new Blob([bytes])});tx.oncomplete=resolve;tx.onerror=()=>reject(tx.error);tx.onabort=()=>reject(tx.error);});
  }catch{/* Public-data caching is optional; installation remains available. */}finally{db?.close();}
}
self.onmessage=async({data})=>{
  const {request,kind,feed,route}=data;
  const progress=(text,count)=>self.postMessage({request,progress:text,count});
  try{
    if(kind==="load"){
      loader=new FeedLoader();
      let bytes=await cached(feed.id),hit=!!bytes;
      if(hit)progress("Using downloaded timetable…");else bytes=await downloadFeed(feed,progress);
      const catalog=await loader.load(bytes,feed,progress);
      if(!hit)await remember(feed.id,bytes);
      self.postMessage({request,catalog});
    }else if(kind==="paths")self.postMessage({request,patterns:await loader.paths(route,progress)});
  }catch(error){self.postMessage({request,error:error.message});}
};
