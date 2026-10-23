package org.nostdrs.timetable;

import android.content.*;
import android.app.admin.DevicePolicyManager;
import android.util.Base64;
import org.json.JSONObject;

/** Exported only behind DUMP permission (ADB shell has it, ordinary apps do not). */
public final class ProvisionReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        if (!"org.nostdrs.timetable.PROVISION".equals(intent.getAction())) return;
        try {
            String encoded = intent.getStringExtra("settings");
            if (encoded == null || encoded.length() > 24000) throw new Exception("Settings too large");
            byte[] bytes = Base64.decode(encoded, Base64.NO_WRAP);
            if (Native.validate(SettingsStore.buffer(bytes)) != 1) throw new Exception("Invalid tablet settings");
            // JSONObject is used only to pass platform options to their OS APIs;
            // the full document has already passed the shared Rust validator.
            JSONObject settings = new JSONObject(new String(bytes, "UTF-8"));
            SettingsStore.atomic(SettingsStore.config(context), bytes);
            context.getSharedPreferences("platform", 0).edit()
                .putBoolean("kioskPaused",false)
                .putBoolean("boot", settings.getBoolean("launch_on_boot"))
                .putBoolean("camera_motion", settings.getBoolean("camera_motion")).apply();
            DevicePolicyManager policy=(DevicePolicyManager)context.getSystemService(Context.DEVICE_POLICY_SERVICE);
            boolean owner=policy!=null && policy.isDeviceOwnerApp(context.getPackageName());
            setResultCode(1); setResultData("Timetable settings saved; device_owner="+owner);
        } catch (Exception e) { setResultCode(0); setResultData("Timetable configuration failed: " + e.getMessage()); }
    }
}
