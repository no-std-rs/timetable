package org.nostdrs.timetable;

import android.app.admin.DevicePolicyManager;
import android.content.*;
import android.content.pm.*;
import android.net.wifi.*;
import android.os.Build;
import android.view.inputmethod.*;
import org.json.*;
import java.io.IOException;
import java.util.*;

/** Permission-protected reports for the browser's explicit ADB maintenance actions. */
final class DeviceTools {
    static JSONObject apps(Context context,String selected) throws Exception {
        PackageManager pm=context.getPackageManager();
        Set<String> protectedPackages=new HashSet<String>();
        protectedPackages.add(context.getPackageName());
        Intent home=new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME);
        for(ResolveInfo info:pm.queryIntentActivities(home,0))if(info.activityInfo!=null)protectedPackages.add(info.activityInfo.packageName);
        ResolveInfo settings=pm.resolveActivity(new Intent(android.provider.Settings.ACTION_SETTINGS),0);
        if(settings!=null && settings.activityInfo!=null)protectedPackages.add(settings.activityInfo.packageName);
        InputMethodManager input=(InputMethodManager)context.getSystemService(Context.INPUT_METHOD_SERVICE);
        if(input!=null)for(InputMethodInfo info:input.getInputMethodList())protectedPackages.add(info.getPackageName());
        DevicePolicyManager policy=(DevicePolicyManager)context.getSystemService(Context.DEVICE_POLICY_SERVICE);
        List<ComponentName> admins=policy==null?null:policy.getActiveAdmins();
        if(admins!=null)for(ComponentName admin:admins)protectedPackages.add(admin.getPackageName());
        List<ApplicationInfo> installed=selected==null?pm.getInstalledApplications(PackageManager.GET_DISABLED_COMPONENTS):Collections.singletonList(pm.getApplicationInfo(selected,PackageManager.GET_DISABLED_COMPONENTS));
        if(installed.size()>512)throw new IOException("Too many applications for this diagnostic report");
        JSONArray rows=new JSONArray();
        for(ApplicationInfo app:installed){
            String name=app.packageName;
            boolean core=name.equals("android") || name.startsWith("com.android.") || app.uid%100000<10000 || (app.flags&ApplicationInfo.FLAG_PERSISTENT)!=0;
            boolean blocked=core || protectedPackages.contains(name);
            rows.put(new JSONObject().put("package",name).put("label",pm.getApplicationLabel(app).toString())
                .put("enabled",app.enabled).put("system",(app.flags&ApplicationInfo.FLAG_SYSTEM)!=0)
                .put("blocked",blocked).put("reason",core?"Android core service":blocked?"Home, settings, keyboard, administrator or Timetable":""));
        }
        return new JSONObject().put("user",context.getApplicationInfo().uid/100000).put("apps",rows);
    }
    static JSONObject wifi(Context context,JSONObject value) throws Exception {
        String ssid=value.getString("ssid"),password=value.getString("password");
        int n=ssid.getBytes("UTF-8").length,p=password.getBytes("UTF-8").length;
        if(n<1 || n>32 || controls(ssid) || controls(password) || !(p==0 || p>=8 && p<=63 || password.matches("[a-fA-F0-9]{64}")))throw new IOException("Invalid Wi-Fi settings");
        DevicePolicyManager policy=(DevicePolicyManager)context.getSystemService(Context.DEVICE_POLICY_SERVICE);
        if(Build.VERSION.SDK_INT>=29 && context.getApplicationInfo().targetSdkVersion>=29 && (policy==null || !policy.isDeviceOwnerApp(context.getPackageName())))throw new IOException("This Android version requires system Wi-Fi setup or device-owner permission");
        WifiManager wifi=(WifiManager)context.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
        if(wifi==null)throw new IOException("Wi-Fi is unavailable");
        WifiConfiguration config=new WifiConfiguration();config.SSID=quote(ssid);
        if(password.isEmpty())config.allowedKeyManagement.set(WifiConfiguration.KeyMgmt.NONE);
        else {config.allowedKeyManagement.set(WifiConfiguration.KeyMgmt.WPA_PSK);config.preSharedKey=password.matches("[a-fA-F0-9]{64}")?password:quote(password);}
        if(!wifi.isWifiEnabled() && !wifi.setWifiEnabled(true))throw new IOException("Android did not enable Wi-Fi");
        // Enabling the radio is asynchronous. Wait off the main thread, within
        // the foreground broadcast budget, before asking the driver to save it.
        long deadline=android.os.SystemClock.elapsedRealtime()+5000;
        while(!wifi.isWifiEnabled() && android.os.SystemClock.elapsedRealtime()<deadline)Thread.sleep(100);
        if(!wifi.isWifiEnabled())throw new IOException("Android did not finish enabling Wi-Fi");
        int id=wifi.addNetwork(config);
        if(id<0)throw new IOException("Android refused the Wi-Fi configuration");
        if(Build.VERSION.SDK_INT<26 && !wifi.saveConfiguration())throw new IOException("Android did not save the Wi-Fi configuration");
        if(!wifi.enableNetwork(id,true))throw new IOException("Wi-Fi was saved but Android did not select it");
        return new JSONObject().put("saved",true).put("connection_requested",true);
    }
    static boolean controls(String s){for(int i=0;i<s.length();i++)if(Character.isISOControl(s.charAt(i)))return true;return false;}
    static String quote(String s){return "\""+s.replace("\\","\\\\").replace("\"","\\\"")+"\"";}
}
