// CSV records across arbitrary chunks, quoted newlines/commas and UTF-8 boundaries.
export async function* csvRows(stream, { maxRecord = 65536, maxFields = 256 } = {}) {
  let row = [], field = "", quoted = false, afterQuote = false, cr = false, size = 0, header;
  const finish = () => {
    row.push(field); field = ""; size = 0;
    if(row.length>maxFields)throw new Error("A feed CSV row exceeds the display’s field limit.");
    const result = row; row = [];
    if (!header) { header = result.map((name, i) => i ? name : name.replace(/^\uFEFF/, "")); return null; }
    if (result.length === 1 && result[0] === "") return null;
    if (result.length !== header.length) throw new Error("Invalid GTFS CSV row.");
    return Object.fromEntries(header.map((name, i) => [name, result[i]]));
  };
  for await (const chunk of stream.pipeThrough(new TextDecoderStream("utf-8", { fatal: true }))) {
    for (const char of chunk) {
      const code=char.codePointAt(0);size+=code<128?1:code<2048?2:code<65536?3:4;
      if (size > maxRecord) throw new Error("A feed CSV record exceeds the supported limit.");
      if (cr) { cr = false; if (char === "\n") continue; }
      if (quoted) {
        if (afterQuote) {
          if (char === '"') { field += char; afterQuote = false; continue; }
          quoted = false; afterQuote = false;
          if (![",", "\r", "\n"].includes(char)) throw new Error("Invalid GTFS CSV quoting.");
        } else { if (char === '"') afterQuote = true; else field += char; continue; }
      }
      if (char === '"') { if (field) throw new Error("Invalid GTFS CSV quoting."); quoted = true; }
      else if (char === ",") { row.push(field); field = ""; if(row.length>=maxFields)throw new Error("A feed CSV row exceeds the display’s field limit."); }
      else if (char === "\r" || char === "\n") { const result = finish(); cr = char === "\r"; if (result) yield result; }
      else field += char;
    }
  }
  if (quoted && !afterQuote) throw new Error("Incomplete GTFS CSV record.");
  if (field || row.length || afterQuote) { const result = finish(); if (result) yield result; }
}
