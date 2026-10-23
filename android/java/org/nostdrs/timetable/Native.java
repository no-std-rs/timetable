package org.nostdrs.timetable;

import java.nio.ByteBuffer;

/** Platform-free JNI surface; the host JVM exercises it in CI. */
public final class Native {
    static { System.loadLibrary("timetable_android"); }
    private Native() {}
    public static native int validate(ByteBuffer settings);
    public static native int render(ByteBuffer settings, ByteBuffer snapshot, ByteBuffer state,
        ByteBuffer frame, int width, int height, long now, long monotonic,
        int battery, boolean plugged, boolean motionAvailable, int status);
    public static native int tap(ByteBuffer settings, ByteBuffer state, int width, int height,
        int x, int y, long now, long monotonic);
    public static native int query(ByteBuffer settings, ByteBuffer archive, ByteBuffer output, long now, long checkedAt);
    public static native int motion(ByteBuffer state, ByteBuffer luma, int width, int height);
    public static native int activity(ByteBuffer state, long monotonic);
}
