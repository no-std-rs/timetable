import java.nio.*;
import java.nio.file.*;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import org.nostdrs.timetable.Native;

/** Same native planner and saved profile used by Android, exercised in a JVM. */
public final class JourneySmoke {
    static ByteBuffer direct(byte[] bytes){ByteBuffer b=ByteBuffer.allocateDirect(bytes.length).order(ByteOrder.LITTLE_ENDIAN);b.put(bytes).rewind();return b;}
    static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    static String text(ByteBuffer frame){byte[] bytes=new byte[frame.capacity()];frame.position(0);frame.get(bytes);frame.position(0);return new String(bytes,StandardCharsets.UTF_8);}
    public static void main(String[] args)throws Exception{
        long now=1792216800L;
        ByteBuffer settings=direct(Files.readAllBytes(Paths.get(args[0])));
        byte[] archive=Files.readAllBytes(Paths.get(args[1]));
        ByteBuffer snapshot=ByteBuffer.allocateDirect(16384),state=ByteBuffer.allocateDirect(64),frame=ByteBuffer.allocateDirect(32768);
        check(Native.validate(settings)==1,"journey settings accepted");
        check(Native.query(settings,direct(archive),snapshot,now,now)>0,"on-device journey query");
        check(Native.render(settings,snapshot,state,frame,1024,600,now,1000,70,true,false,0)>24,"journey rendering");
        String initial=text(frame);
        check(initial.contains("Arrive 08:30"),"whole connection ETA is shown");
        check(initial.contains("911")&&initial.contains("Transfer"),"both bus legs shown");
        check(initial.contains("3 min transfer"),"actual change time shown");
        check(Native.render(settings,snapshot,state,frame,600,1024,now+240,2000,70,true,false,0)>24,"saved journey profile rerenders locally");
        check(text(frame).contains("Arrive 09:00"),"next best connection replaces an unreachable first bus");
        byte[] saved=new byte[snapshot.capacity()];snapshot.position(0);snapshot.get(saved);snapshot.position(0);
        archive[archive.length-1]^=1;
        check(Native.query(settings,direct(archive),snapshot,now,now)<0,"invalid archive rejected");
        byte[] after=new byte[snapshot.capacity()];snapshot.position(0);snapshot.get(after);snapshot.position(0);
        check(Arrays.equals(saved,after),"failed query preserves the saved journey profile");
        System.out.println("Journey JNI smoke passed: ETA, both legs, transfer slack, local next-best selection, atomic failure");
    }
}
