package org.nostdrs.timetable;

import android.content.Context;
import java.io.*;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.FileChannel;

/** File lifetime and atomic replacement only; Rust validates all app data. */
final class SettingsStore {
    static final int MAX_SETTINGS = 16384;
    static ByteBuffer buffer(byte[] bytes) {
        ByteBuffer b = ByteBuffer.allocateDirect(bytes.length).order(ByteOrder.LITTLE_ENDIAN);
        b.put(bytes).rewind(); return b;
    }
    static byte[] read(File file, int maximum) throws IOException {
        if (file.length() > maximum || file.length() <= 0) throw new IOException("Invalid file size");
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        FileInputStream in = new FileInputStream(file);
        try { byte[] bytes = new byte[4096]; int n;
            while ((n = in.read(bytes)) != -1) {
                if (out.size() + n > maximum) throw new IOException("File too large");
                out.write(bytes, 0, n);
            }
        } finally { in.close(); }
        return out.toByteArray();
    }
    static void atomic(File file, byte[] bytes) throws IOException {
        File temporary = File.createTempFile(file.getName(), ".new", file.getParentFile());
        try {
            FileOutputStream out = new FileOutputStream(temporary);
            try { out.write(bytes); out.getFD().sync(); } finally { out.close(); }
            if (!temporary.renameTo(file)) throw new IOException("Cannot save settings");
        } finally { temporary.delete(); }
    }
    static ByteBuffer mapped(File file) throws IOException {
        long length = file.length();
        if (length <= 0 || length > 512L * 1024 * 1024) throw new IOException("Timetable ZIP too large");
        FileInputStream in = new FileInputStream(file);
        try { return in.getChannel().map(FileChannel.MapMode.READ_ONLY, 0, length); }
        finally { in.close(); }
    }
    static File config(Context context) { return new File(context.getFilesDir(), "settings.json"); }
}
