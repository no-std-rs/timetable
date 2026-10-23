import java.nio.*;
import java.nio.file.*;
import org.nostdrs.timetable.Native;

/** Real JNI calls on a host JVM, with the same Rust implementation as Android. */
public final class NativeSmoke {
    static ByteBuffer direct(byte[] bytes){ByteBuffer b=ByteBuffer.allocateDirect(bytes.length).order(ByteOrder.LITTLE_ENDIAN);b.put(bytes).rewind();return b;}
    static void check(boolean value,String text){if(!value)throw new AssertionError(text);}
    public static void main(String[] args)throws Exception{
        ByteBuffer settings=direct(Files.readAllBytes(Paths.get(args[0])));
        ByteBuffer zip=direct(Files.readAllBytes(Paths.get(args[1])));
        ByteBuffer snapshot=ByteBuffer.allocateDirect(16384).order(ByteOrder.LITTLE_ENDIAN);
        ByteBuffer state=ByteBuffer.allocateDirect(64),frame=ByteBuffer.allocateDirect(32768).order(ByteOrder.LITTLE_ENDIAN);
        check(Native.validate(settings)==1,"valid settings");
        check(Native.validate(ByteBuffer.wrap(new byte[]{1,2}))==0,"non-direct settings rejected");
        check(Native.query(settings,zip,snapshot,1792216800L,1792213200L)>0,"real GTFS query");
        check(Native.render(settings,snapshot,state,frame,1024,600,1792216800L,1000,70,true,false,0)>24,"render JNI");
        check(frame.getInt(12)==1,"keep-awake flags");
        check(Native.render(settings,snapshot,state,state,1024,600,1792216800L,1000,70,true,false,0)==-1,"overlapping direct buffers rejected");
        check(Native.render(settings,snapshot,state,frame.asReadOnlyBuffer(),1024,600,1792216800L,1000,70,true,false,0)==-1,"read-only output rejected");
        check(Native.render(settings,snapshot,state,frame,1024,600,Long.MAX_VALUE,1000,70,true,false,0)==-1,"invalid time rejected");
        check(Native.tap(settings,state,1024,600,800,110,1792216800L,2000)==0,"group tap");
        check(Native.render(settings,snapshot,state,frame,600,1024,1792216800L,2000,70,true,false,0)>24,"portrait JNI");
        check(frame.getInt(16)==1,"Rust group selection retained");
        System.out.println("JNI smoke test passed: query, both layouts, group selection, buffer validation");
    }
}
