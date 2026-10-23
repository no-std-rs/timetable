package org.nostdrs.timetable;

import android.content.*;

public final class BootReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        if (Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction()) &&
            context.getSharedPreferences("platform", 0).getBoolean("boot", false)) {
            // Modern Android may disallow background launches. Becoming the Home
            // app/device owner is the reliable dedicated-device path when supported.
            Intent open = new Intent(context, MainActivity.class);
            open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            try { context.startActivity(open); } catch (RuntimeException ignored) { }
        }
    }
}
