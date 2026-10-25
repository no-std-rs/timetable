// Match Rust journey::bounded_label: UTF-8 capacity, collapsed whitespace,
// control characters removed, no ellipsis or partial Unicode scalar.
export function deviceLabel(value,capacity){
  let text="",bytes=0;
  for(const word of String(value??"").split(/\p{White_Space}+/u).filter(Boolean)){
    if(text){if(bytes===capacity)break;text+=" ";bytes++;}
    for(const c of word){
      if(/\p{Cc}/u.test(c))continue;
      const size=new TextEncoder().encode(c).length;
      if(bytes+size>capacity)return text;
      text+=c;bytes+=size;
    }
  }
  return text;
}
export const journeyGlyphText=catalog=>[
  "·",
  ...[...catalog.stops,...catalog.via_stops??[]].flatMap(s=>[
    deviceLabel(s.place_name??s.name,64),deviceLabel(s.platform_code??s.code,16),
  ]),
  ...catalog.routes.map(r=>deviceLabel(r.route_name??r.line,8)),
];
