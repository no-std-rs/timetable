// One itinerary as a local SVG stop-dot strip: walks, every ride stop and each
// change, wrapped to the card width. No map tiles or routing service.
const WIDTH={walk:58,change:58,stop:28},ROW=54,PAD=14;
export function diagramTokens(path){
  const tokens=[];
  if(path.walk)tokens.push({kind:"walk",minutes:path.walk});
  path.legs.forEach((leg,i)=>{
    if(i)tokens.push({kind:"change",minutes:Math.floor((leg.departure-path.legs[i-1].arrival)/60)});
    tokens.push({kind:"line",line:leg.line,leg:i});
    leg.stops.forEach((call,at)=>{
      const board=at===0,alight=at===leg.stops.length-1;
      tokens.push({kind:"stop",leg:i,at,call,board,alight,request:board?leg.requestBoard:alight?leg.requestAlight:call.pickup>=2||call.dropoff>=2});
    });
  });
  if(path.egress)tokens.push({kind:"walk",minutes:path.egress});
  return tokens;
}
export function layoutTokens(tokens,width){
  let x=PAD,row=0;
  const placed=tokens.map(token=>{
    const w=token.kind==="line"?Math.max(30,14+7*[...token.line].length):WIDTH[token.kind];
    if(x+w>width-PAD&&x>PAD){x=PAD;row++;}
    const item={...token,x,w,row,cx:x+w/2,cy:24+row*ROW};x+=w;return item;
  });
  return {tokens:placed,height:(placed.at(-1)?.row??0)*ROW+48};
}
const svg=(tag,attributes={})=>{const element=document.createElementNS("http://www.w3.org/2000/svg",tag);for(const[key,value]of Object.entries(attributes))element.setAttribute(key,String(value));return element;};
export function renderRoute(map,path,{width,hint,describe,activate}){
  map.replaceChildren();
  const {tokens,height}=layoutTokens(diagramTokens(path),width);
  map.setAttribute("viewBox",`0 0 ${width} ${height}`);map.setAttribute("width",width);map.setAttribute("height",height);
  // Ride lines join the line label and its stops, continuing across wrapped rows.
  for(let i=1;i<tokens.length;i++){
    const a=tokens[i-1],b=tokens[i];
    if(b.kind!=="stop"||!["stop","line"].includes(a.kind)||a.leg!==b.leg)continue;
    if(a.row===b.row)map.append(svg("line",{x1:a.cx,y1:a.cy,x2:b.cx,y2:b.cy,class:"route-ride"}));
    else map.append(svg("line",{x1:a.cx,y1:a.cy,x2:width-4,y2:a.cy,class:"route-ride"}),svg("line",{x1:4,y1:b.cy,x2:b.cx,y2:b.cy,class:"route-ride"}));
  }
  for(const token of tokens){
    if(token.kind==="walk"||token.kind==="change"){
      map.append(svg("line",{x1:token.x+4,y1:token.cy,x2:token.x+token.w-4,y2:token.cy,class:`route-${token.kind}`}));
      const text=svg("text",{x:token.cx,y:token.cy-8,"text-anchor":"middle",class:"route-minutes"});text.textContent=`${token.minutes}′`;map.append(text);
    }else if(token.kind==="line"){
      map.append(svg("rect",{x:token.x+2,y:token.cy-10,width:token.w-4,height:20,rx:10,class:"route-line"}));
      const text=svg("text",{x:token.cx,y:token.cy+4,"text-anchor":"middle",class:"route-line-text"});text.textContent=token.line;map.append(text);
    }else{
      const {label,selected,disabled}=describe(token);
      const group=svg("g",{transform:`translate(${token.cx} ${token.cy})`,class:"route-stop-dot",role:"button",tabindex:disabled?-1:0,"aria-label":label,"aria-pressed":selected,"aria-disabled":disabled});
      const title=svg("title");title.textContent=label;group.append(title,svg("circle",{r:14,class:"route-dot-hit"}));
      if(token.request)group.append(svg("circle",{r:10,class:"route-dot-request"}));
      group.append(svg("circle",{r:token.board||token.alight?7:4.5,class:`route-dot${token.board||token.alight?" endpoint":""}${selected?" selected":""}`}));
      const choose=()=>{if(!disabled)activate(token);};
      group.addEventListener("click",choose);
      group.addEventListener("keydown",event=>{if(event.key==="Enter"||event.key===" "){event.preventDefault();choose();}});
      for(const event of["mouseenter","focus"])group.addEventListener(event,()=>{hint.textContent=label;});
      map.append(group);
    }
  }
}
