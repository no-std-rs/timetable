// Local SVG geometry only: feed coordinates, no tiles or routing service.
export function projectDots(points,width=480,height=200,padding=28){
  if(!points.length)return [];
  const origin=points[0].point,cos=Math.cos(origin.lat*Math.PI/180);
  const raw=points.map(item=>({...item,x:((item.point.lon-origin.lon+540)%360-180)*cos,y:origin.lat-item.point.lat}));
  const xs=raw.map(p=>p.x),ys=raw.map(p=>p.y),left=Math.min(...xs),right=Math.max(...xs),top=Math.min(...ys),bottom=Math.max(...ys);
  const scale=Math.min((width-2*padding)/Math.max(right-left,1e-7),(height-2*padding)/Math.max(bottom-top,1e-7));
  return raw.map(item=>({...item,x:width/2+(item.x-(left+right)/2)*scale,y:height/2+(item.y-(top+bottom)/2)*scale}));
}
const svg=(tag,attributes={})=>{const element=document.createElementNS("http://www.w3.org/2000/svg",tag);for(const[key,value]of Object.entries(attributes))element.setAttribute(key,value);return element;};
export function renderStopDots(map,points,{hint,activate}){
  map.replaceChildren();const dots=projectDots(points);
  if(dots.length>=2){const a=dots[0],b=dots[1];map.append(svg("line",{x1:a.x,y1:a.y,x2:b.x,y2:b.y,class:"stop-dot-link"}));}
  for(const dot of dots){
    const group=svg("g",{transform:`translate(${dot.x} ${dot.y})`,class:"stop-dot"});
    const title=svg("title");title.textContent=dot.label;group.append(title);
    if(dot.anchor){
      group.append(svg("circle",{r:11,class:"stop-dot-anchor"}));
      const text=svg("text",{y:4,"text-anchor":"middle",class:"stop-dot-number"});text.textContent=dot.anchor;group.append(text);
    }else{
      group.setAttribute("role","button");group.setAttribute("tabindex",dot.disabled?-1:0);group.setAttribute("aria-label",dot.label);group.setAttribute("aria-pressed",String(dot.selected));group.setAttribute("aria-disabled",String(dot.disabled));
      group.append(svg("circle",{r:20,class:"stop-dot-hit"}),svg("circle",{r:dot.selected?8:6,class:"stop-dot-symbol"}));
      const choose=()=>{if(!dot.disabled)activate(dot);};
      group.addEventListener("click",choose);group.addEventListener("keydown",event=>{if(event.key==="Enter"||event.key===" "){event.preventDefault();choose();}});
      for(const event of["mouseenter","focus"])group.addEventListener(event,()=>{hint.textContent=dot.label;});
    }
    map.append(group);
  }
}
