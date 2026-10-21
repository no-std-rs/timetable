// Browser-only GTFS ZIP reader. Decompression and CSV parsing stay streamed.
const u16 = (b,n) => new DataView(b.buffer,b.byteOffset,b.byteLength).getUint16(n,true);
const u32 = (b,n) => new DataView(b.buffer,b.byteOffset,b.byteLength).getUint32(n,true);
const table = Uint32Array.from({length:256},(_,i)=>{for(let n=0;n<8;n++)i=i>>>1^(0xedb88320&-(i&1));return i>>>0;});
export class ZipArchive {
  constructor(bytes) {
    this.bytes=bytes;this.members=new Map();
    let end=-1;
    for(let n=bytes.length-22;n>=Math.max(0,bytes.length-65557);n--)if(u32(bytes,n)===0x06054b50&&n+22+u16(bytes,n+20)===bytes.length){end=n;break;}
    if(end<0||u16(bytes,end+4)||u16(bytes,end+6)||u16(bytes,end+8)!==u16(bytes,end+10)||u16(bytes,end+10)===65535)throw new Error("Unsupported timetable ZIP archive.");
    let pos=u32(bytes,end+16);const directoryEnd=pos+u32(bytes,end+12);
    if(directoryEnd!==end)throw new Error("Invalid timetable ZIP directory.");
    for(let i=0;i<u16(bytes,end+10);i++){
      if(pos+46>directoryEnd||u32(bytes,pos)!==0x02014b50)throw new Error("Invalid timetable ZIP directory.");
      const length=u16(bytes,pos+28), extra=u16(bytes,pos+30), comment=u16(bytes,pos+32);
      const next=pos+46+length+extra+comment;
      if(next>directoryEnd||u16(bytes,pos+34))throw new Error("Unsupported timetable ZIP archive.");
      const name=new TextDecoder(u16(bytes,pos+8)&0x800?"utf-8":"windows-1252",{fatal:true}).decode(bytes.subarray(pos+46,pos+46+length));
      const member={name,flags:u16(bytes,pos+8),method:u16(bytes,pos+10),crc:u32(bytes,pos+16),compressed:u32(bytes,pos+20),size:u32(bytes,pos+24),offset:u32(bytes,pos+42),directory:u32(bytes,end+16)};
      if(member.size===0xffffffff||member.compressed===0xffffffff||member.offset===0xffffffff)throw new Error("Unsupported timetable ZIP archive.");
      if(this.members.has(name))throw new Error("Duplicate timetable ZIP file.");
      this.members.set(name,member);pos=next;
    }
    if(pos!==directoryEnd)throw new Error("Invalid timetable ZIP directory.");
  }
  stream(name) {
    const m=this.members.get(name),b=this.bytes;
    if(!m)throw new Error(`Missing GTFS table: ${name}`);
    if(m.flags&~0x080e||![0,8].includes(m.method))throw new Error("Unsupported timetable ZIP archive.");
    const n=m.offset;
    if(n+30>m.directory||u32(b,n)!==0x04034b50||u16(b,n+6)!==m.flags||u16(b,n+8)!==m.method)throw new Error("Invalid timetable ZIP entry.");
    const start=n+30+u16(b,n+26)+u16(b,n+28),end=start+m.compressed;
    if(end>m.directory||new TextDecoder().decode(b.subarray(n+30,n+30+u16(b,n+26)))!==name)throw new Error("Invalid timetable ZIP entry.");
    let stream=new ReadableStream({start(c){c.enqueue(b.subarray(start,end));c.close();}});
    if(m.method===8)stream=stream.pipeThrough(new DecompressionStream("deflate-raw"));
    let count=0,crc=0xffffffff;
    return stream.pipeThrough(new TransformStream({
      transform(chunk,c){count+=chunk.length;if(count>m.size||count>2**31)throw new Error("Timetable table exceeds its size limit.");for(const byte of chunk)crc=crc>>>8^table[(crc^byte)&255];c.enqueue(chunk);},
      flush(){if(count!==m.size||((crc^0xffffffff)>>>0)!==m.crc)throw new Error("Timetable ZIP checksum mismatch.");},
    }));
  }
}
