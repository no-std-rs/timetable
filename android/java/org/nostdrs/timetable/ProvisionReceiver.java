package org.nostdrs.timetable;

import android.content.*;
import android.app.admin.DevicePolicyManager;
import android.util.Base64;
import org.json.JSONObject;

/** Exported only behind DUMP permission (ADB shell has it, ordinary apps do not). */
public final class ProvisionReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        String action=intent.getAction();
        if("org.nostdrs.timetable.WIFI".equals(action)) {
            final PendingResult pending=goAsync();final Context app=context.getApplicationContext();final String payload=intent.getStringExtra("payload");
            new Thread(new Runnable(){public void run(){
                try {
                    if(payload==null || payload.length()>2048)throw new Exception("Invalid Wi-Fi settings");
                    JSONObject report=DeviceTools.wifi(app,new JSONObject(new String(Base64.decode(payload,Base64.NO_WRAP),"UTF-8")));
                    pending.setResultCode(1);pending.setResultData(encoded(report));
                } catch(Exception e){pending.setResultCode(0);pending.setResultData("Android refused Wi-Fi setup; use system settings or device-owner mode");}
                finally {pending.finish();}
            }},"timetable-wifi").start();
            return;
        }
        if("org.nostdrs.timetable.APPS".equals(action)) {
            try {setResultCode(1);setResultData(encoded(DeviceTools.apps(context,intent.getStringExtra("package"))));}
            catch(Exception e){setResultCode(0);setResultData("Could not read installed applications");}
            return;
        }
        if (!"org.nostdrs.timetable.PROVISION".equals(action)) return;
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
                .remove("rotation") // A fresh USB configuration supplies the new default.
                .putBoolean("kioskPaused",false)
                .putBoolean("boot", settings.getBoolean("launch_on_boot"))
                .putBoolean("camera_motion", settings.getBoolean("camera_motion")).apply();
            DevicePolicyManager policy=(DevicePolicyManager)context.getSystemService(Context.DEVICE_POLICY_SERVICE);
            boolean owner=policy!=null && policy.isDeviceOwnerApp(context.getPackageName());
            setResultCode(1); setResultData("Timetable settings saved; device_owner="+owner);
        } catch (Exception e) { setResultCode(0); setResultData("Timetable configuration failed: " + e.getMessage()); }
    }
    static String encoded(JSONObject report) throws Exception {
        byte[] bytes=report.toString().getBytes("UTF-8");
        if(bytes.length>45000)throw new Exception("Diagnostic report is too large");
        return "TT_REPORT:"+Base64.encodeToString(bytes,Base64.NO_WRAP);
    }
}
