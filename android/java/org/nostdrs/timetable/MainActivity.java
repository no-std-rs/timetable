package org.nostdrs.timetable;

import android.Manifest;
import android.app.*;
import android.app.admin.DevicePolicyManager;
import android.content.*;
import android.content.pm.PackageManager;
import android.content.pm.ActivityInfo;
import android.graphics.*;
import android.os.*;
import android.text.TextUtils;
import android.view.*;
import android.widget.*;
import org.json.JSONObject;
import java.io.*;
import java.net.*;
import java.nio.*;
import java.util.Arrays;
import java.util.concurrent.*;
import javax.net.ssl.HttpsURLConnection;

/** Android lifecycle, Canvas, storage and HTTPS. Timetable/UI/presence logic is Rust. */
public final class MainActivity extends Activity {
    final Handler handler = new Handler(Looper.getMainLooper());
    final ExecutorService worker = Executors.newSingleThreadExecutor();
    final ByteBuffer state = ByteBuffer.allocateDirect(64).order(ByteOrder.LITTLE_ENDIAN);
    volatile ByteBuffer settings;
    ByteBuffer snapshot;
    byte[] settingsBytes;
    JSONObject options;
    BoardView board;
    CameraMotion camera;
    volatile boolean destroyed, busy;
    volatile HttpsURLConnection connection;
    int status, battery = -1;
    boolean plugged, resumed, cameraAsked;
    long nextRefresh;
    String problem;
    final Runnable tick = new Runnable() { public void run() {
        if (!resumed) return;
        board.invalidate();
        if (settings != null && !busy && SystemClock.elapsedRealtime() >= nextRefresh) update(false);
        handler.postDelayed(this, 1000);
    }};
    final BroadcastReceiver batteryReceiver = new BroadcastReceiver() {
        public void onReceive(Context context, Intent intent) {
            int level=intent.getIntExtra(BatteryManager.EXTRA_LEVEL,-1), scale=intent.getIntExtra(BatteryManager.EXTRA_SCALE,100);
            battery=level>=0 && scale>0 ? level*100/scale : -1;
            plugged=intent.getIntExtra(BatteryManager.EXTRA_PLUGGED,0)!=0;
            if (board != null) board.invalidate();
        }
    };
    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().getDecorView().setSystemUiVisibility(5894);
        android.widget.FrameLayout container = new android.widget.FrameLayout(this);
        board = new BoardView(); container.addView(board);
        camera = new CameraMotion(this, container);
        setContentView(container);
        registerReceiver(batteryReceiver,new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
    }
    @Override public void onResume() {
        super.onResume(); resumed=true;
        loadSettings(); applyRotation(); Native.activity(state,SystemClock.elapsedRealtime()); applyDeviceOwner(); startCamera();
        handler.removeCallbacks(tick); handler.post(tick);
    }
    @Override public void onPause() {
        resumed=false; handler.removeCallbacks(tick); camera.stop(); super.onPause();
    }
    @Override public void onDestroy() {
        destroyed=true; handler.removeCallbacksAndMessages(null); camera.stop();
        HttpsURLConnection current=connection; if (current!=null) current.disconnect();
        worker.shutdownNow(); unregisterReceiver(batteryReceiver); super.onDestroy();
    }
    void loadSettings() {
        try {
            byte[] bytes=SettingsStore.read(SettingsStore.config(this),16384);
            if (Arrays.equals(bytes,settingsBytes)) return;
            ByteBuffer candidate=SettingsStore.buffer(bytes);
            if (Native.validate(candidate)!=1) throw new IOException("Invalid settings");
            options=new JSONObject(new String(bytes,"UTF-8")); settings=candidate; settingsBytes=bytes;
            for(int i=0;i<state.capacity();i++)state.put(i,(byte)0);
            snapshot=null;
            try { snapshot=SettingsStore.buffer(SettingsStore.read(snapshotFile(settings),16384)); }
            catch(IOException ignored) { }
            problem=null; nextRefresh=0;
        } catch(Exception e) { problem="Configure this tablet over USB from the Timetable website."; }
    }
    void applyDeviceOwner() {
        if (settings==null || getSharedPreferences("platform",0).getBoolean("kioskPaused",false)) return;
        DevicePolicyManager policy=(DevicePolicyManager)getSystemService(DEVICE_POLICY_SERVICE);
        if (policy==null || !policy.isDeviceOwnerApp(getPackageName())) return;
        try {
            ComponentName admin=new ComponentName(this,AdminReceiver.class);
            IntentFilter home=new IntentFilter(Intent.ACTION_MAIN);
            home.addCategory(Intent.CATEGORY_HOME); home.addCategory(Intent.CATEGORY_DEFAULT);
            policy.addPersistentPreferredActivity(admin,home,new ComponentName(this,MainActivity.class));
            policy.setLockTaskPackages(admin,new String[]{getPackageName()});
            if (policy.isLockTaskPermitted(getPackageName()) && !((KeyguardManager)getSystemService(KEYGUARD_SERVICE)).isKeyguardLocked()) startLockTask();
        } catch(RuntimeException e) { Toast.makeText(this,"Device-owner controls are unavailable on this firmware.",Toast.LENGTH_LONG).show(); }
    }
    static final String[] ROTATIONS={"system","portrait","landscape","reverse_portrait","reverse_landscape"};
    static final String[] ROTATION_LABELS={"Follow Android settings","Portrait","Landscape","Portrait upside down","Landscape upside down"};
    static final int[] ORIENTATIONS={ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED,ActivityInfo.SCREEN_ORIENTATION_PORTRAIT,
        ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE,ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT,ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE};
    int rotationIndex() {
        String value=options==null?"system":options.optString("rotation","system");
        for(int i=0;i<ROTATIONS.length;i++)if(ROTATIONS[i].equals(value))return i;
        return 0;
    }
    void applyRotation() { setRequestedOrientation(ORIENTATIONS[rotationIndex()]); }
    void showSettings() {
        new AlertDialog.Builder(this).setTitle("Screen rotation")
            .setSingleChoiceItems(ROTATION_LABELS,rotationIndex(),new DialogInterface.OnClickListener(){
                public void onClick(DialogInterface dialog,int which){
                    try {
                        if(settingsBytes==null)throw new IOException("Configure this tablet over USB first.");
                        JSONObject changed=new JSONObject(new String(settingsBytes,"UTF-8"));
                        changed.put("rotation",ROTATIONS[which]);
                        byte[] bytes=changed.toString().getBytes("UTF-8");
                        if(Native.validate(SettingsStore.buffer(bytes))!=1)throw new IOException("Invalid settings");
                        SettingsStore.atomic(SettingsStore.config(MainActivity.this),bytes);
                        loadSettings();applyRotation();board.invalidate();dialog.dismiss();
                    }catch(Exception e){Toast.makeText(MainActivity.this,e.getMessage(),Toast.LENGTH_LONG).show();}
                }
            }).setNegativeButton("Close",null)
            .setNeutralButton("Android settings",new DialogInterface.OnClickListener(){public void onClick(DialogInterface dialog,int which){
                try{startActivity(new Intent(android.provider.Settings.ACTION_SETTINGS));}
                catch(RuntimeException e){Toast.makeText(MainActivity.this,"Android settings are unavailable.",Toast.LENGTH_LONG).show();}
            }})
            .setPositiveButton("Device info",new DialogInterface.OnClickListener(){public void onClick(DialogInterface dialog,int which){performAction("Device capabilities");}}).show();
    }
    void startCamera() {
        if (settings==null || !resumed || !options.optBoolean("camera_motion")) { camera.stop(); return; }
        if (Build.VERSION.SDK_INT>=23 && checkSelfPermission(Manifest.permission.CAMERA)!=PackageManager.PERMISSION_GRANTED) {
            if (!cameraAsked) { cameraAsked=true; requestPermissions(new String[]{Manifest.permission.CAMERA},31); }
            return;
        }
        camera.start();
    }
    @Override public void onRequestPermissionsResult(int request,String[] permissions,int[] results) {
        super.onRequestPermissionsResult(request,permissions,results);
        if(request==31 && results.length>0 && results[0]==PackageManager.PERMISSION_GRANTED)startCamera();
    }
    void update(final boolean force) {
        if (settings==null || busy) return;
        final ByteBuffer querySettings=settings;
        final String feed=options.optString("feed_id");
        busy=true; status=1; board.invalidate();
        nextRefresh=SystemClock.elapsedRealtime()+3*3600*1000L;
        worker.execute(new Runnable() { public void run() {
            final File archive=new File(getFilesDir(),feed+".zip"), metadata=new File(getFilesDir(),feed+".http");
            final String origin="https://files.mobilitydatabase.org/"+feed+"/latest.zip";
            try {
                // Cached data is queried and shown before opening any socket.
                FeedCache.State cachedState=FeedCache.state(archive,metadata,origin);
                boolean cached=false;
                if(archive.exists())try{saveAndPublish(querySettings,query(querySettings,archive,cachedState==null?archive.lastModified()/1000:cachedState.checkedAt()));cached=true;}catch(IOException invalidCache){ /* Fetch a replacement; keep any validated departure snapshot. */ }
                if(!force && cached && cachedState!=null && System.currentTimeMillis()/1000-cachedState.checkedAt()>=0 && System.currentTimeMillis()/1000-cachedState.checkedAt()<3*3600) {
                    finish(querySettings,null);return;
                }
                long checked=FeedCache.refresh(archive,metadata,origin,cached?cachedState:null,new FeedCache.Transport(){
                    public void check() throws IOException {if(destroyed || Thread.currentThread().isInterrupted())throw new IOException("Cancelled");}
                    public HttpURLConnection open(URL url) throws Exception {
                        check();
                        if(!"https".equals(url.getProtocol()) || !"files.mobilitydatabase.org".equals(url.getHost()) || url.getPort()!=-1 || url.getUserInfo()!=null)throw new IOException("Unsupported timetable redirect");
                        HttpsURLConnection request=(HttpsURLConnection)url.openConnection();connection=request;
                        request.setSSLSocketFactory(TransportTLS.factory(MainActivity.this));return request;
                    }
                },new FeedCache.Validate(){public void check(File file,long checkedAt) throws Exception {query(querySettings,file,checkedAt);}});
                connection=null;
                saveAndPublish(querySettings,query(querySettings,archive,checked));
                finish(querySettings,null);
            } catch(final Exception e) { connection=null; finish(querySettings,e.getMessage()); }
        }});
    }
    ByteBuffer query(ByteBuffer config,File archive,long checkedAt) throws IOException {
        ByteBuffer result=ByteBuffer.allocateDirect(16384).order(ByteOrder.LITTLE_ENDIAN);
        int n=Native.query(config,SettingsStore.mapped(archive),result,System.currentTimeMillis()/1000,checkedAt);
        if(n<=0)throw new IOException("Timetable data is unsupported or exceeds the current device limits.");
        result.limit(n);return result;
    }
    File snapshotFile(ByteBuffer config) throws IOException {
        try {
            java.security.MessageDigest digest=java.security.MessageDigest.getInstance("SHA-256");
            ByteBuffer source=config.duplicate(); source.position(0); digest.update(source);
            StringBuilder name=new StringBuilder("snapshot-");
            for(byte b:digest.digest())name.append(String.format(java.util.Locale.ROOT,"%02x",b));
            return new File(getFilesDir(),name.append(".bin").toString());
        } catch(java.security.NoSuchAlgorithmException e) { throw new IOException(e); }
    }
    void saveAndPublish(final ByteBuffer config,final ByteBuffer result) throws IOException {
        byte[] bytes=new byte[result.limit()];result.position(0);result.get(bytes);result.position(0);
        SettingsStore.atomic(snapshotFile(config),bytes);
        handler.post(new Runnable(){public void run(){if(!destroyed && settings==config){snapshot=result;board.invalidate();}}});
    }
    void finish(final ByteBuffer config,final String error) { handler.post(new Runnable(){public void run(){
        if(destroyed)return;busy=false;
        if(settings!=config){nextRefresh=0;board.invalidate();return;}
        status=error==null?0:2;problem=error;
        if(error!=null)nextRefresh=SystemClock.elapsedRealtime()+5*60*1000;
        board.invalidate();
    }}); }
    @Override public boolean onCreateOptionsMenu(Menu menu) {
        return true;
    }
    @Override public boolean onPrepareOptionsMenu(Menu menu) {
        menu.clear();
        for(String action:actions())menu.add(action);return true;
    }
    boolean isPinned() { return ((ActivityManager)getSystemService(ACTIVITY_SERVICE)).isInLockTaskMode(); }
    String[] actions() {
        return new String[]{"Refresh timetable","Settings","Choose Home app",isPinned()?"Exit kiosk":"Pin app"};
    }
    void showActions() {
        final String[] actions=actions();
        new AlertDialog.Builder(this).setItems(actions,new DialogInterface.OnClickListener(){
            public void onClick(DialogInterface dialog,int which){performAction(actions[which]);}
        }).show();
    }
    @Override public boolean onOptionsItemSelected(MenuItem item) {performAction(item.getTitle().toString());return true;}
    void performAction(String title) {
        if(title.equals("Refresh timetable"))update(true);
        else if(title.equals("Settings"))showSettings();
        else if(title.equals("Pin app")) {
            try{startLockTask();}catch(RuntimeException e){Toast.makeText(this,"Screen pinning is unavailable on this tablet.",Toast.LENGTH_LONG).show();}
        }
        else if(title.equals("Choose Home app")) {
            try {startActivity(new Intent("android.settings.HOME_SETTINGS"));}catch(RuntimeException e){startActivity(new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME));}
        } else if(title.equals("Exit kiosk")) {
            getSharedPreferences("platform",0).edit().putBoolean("kioskPaused",true).apply();
            try{stopLockTask();}catch(RuntimeException ignored){}
            DevicePolicyManager policy=(DevicePolicyManager)getSystemService(DEVICE_POLICY_SERVICE);
            if(policy!=null && policy.isDeviceOwnerApp(getPackageName()))try{
                ComponentName admin=new ComponentName(this,AdminReceiver.class);
                policy.setLockTaskPackages(admin,new String[0]);
                policy.clearPackagePersistentPreferredActivities(admin,getPackageName());
            }catch(RuntimeException ignored){}
            startActivity(new Intent(android.provider.Settings.ACTION_SETTINGS));
        }
        else if(title.equals("Device capabilities"))new AlertDialog.Builder(this).setTitle("This tablet")
            .setMessage("Android "+Build.VERSION.RELEASE+" (API "+Build.VERSION.SDK_INT+")\nCamera motion: "+(camera.active?"running locally":"off or unavailable")+
                "\n80% charging cap: unavailable through standard Android APIs. A supported vendor/root integration or external power controller is required."+
                (problem==null?"":"\n\n"+problem)).setPositiveButton("OK",null).show();
    }
    final class BoardView extends View {
        final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
        final ByteBuffer frame=ByteBuffer.allocateDirect(32768).order(ByteOrder.LITTLE_ENDIAN);
        BoardView(){super(MainActivity.this);setFocusable(true);setContentDescription("Timetable departure board");}
        @Override protected void onDraw(Canvas canvas) {
            float density=getResources().getDisplayMetrics().density;
            int width=(int)(getWidth()/density),height=(int)(getHeight()/density);
            canvas.save();canvas.scale(density,density);
            if(settings==null) {
                canvas.drawColor(0xfff5f1e8);paint.setColor(0xff252b28);paint.setTextSize(28);canvas.drawText("Timetable",24,54,paint);
                paint.setTextSize(15);canvas.drawText("Configure this tablet over USB.",24,90,paint);
                canvas.drawText("no-std-rs.github.io/timetable",24,116,paint);canvas.restore();return;
            }
            int n=Native.render(settings,snapshot,state,frame,width,height,System.currentTimeMillis()/1000,SystemClock.elapsedRealtime(),battery,plugged,camera.active,status);
            if(n<24){
                canvas.drawColor(0xfff5f1e8);paint.setColor(0xff252b28);paint.setTextSize(24);
                canvas.drawText("Timetable",24,54,paint);paint.setTextSize(14);
                canvas.drawText("Check the tablet’s date and time.",24,90,paint);
                canvas.drawText("Use ••• to open system settings.",24,116,paint);
                canvas.drawText("•••",width-48,27,paint);canvas.restore();return;
            }
            frame.position(12);int flags=frame.getInt();
            boolean awake=(flags&1)!=0;
            boolean wasAwake=(getWindow().getAttributes().flags&WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)!=0;
            // Updating window attributes on every draw can cause a redraw loop.
            if(awake!=wasAwake){
                if(awake)getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                else getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            }
            WindowManager.LayoutParams window=getWindow().getAttributes();float brightness=(flags&2)!=0?0.02f:-1f;
            if(window.screenBrightness!=brightness){window.screenBrightness=brightness;getWindow().setAttributes(window);}
            frame.position(24);
            try {while(frame.position()<n) {
                int opcode=frame.getInt();int x=frame.getInt(),y=frame.getInt();
                if(opcode==1){int w=frame.getInt(),h=frame.getInt(),r=frame.getInt();paint.setColor(frame.getInt());canvas.drawRoundRect(new RectF(x,y,x+w,y+h),r,r,paint);}
                else if(opcode==2){int size=frame.getInt(),w=frame.getInt(),color=frame.getInt(),style=frame.getInt(),len=frame.getInt();
                    if(len<0 || len>4096 || frame.position()+len>n)break;
                    byte[] bytes=new byte[len];frame.get(bytes);frame.position((frame.position()+3)&~3);
                    String value=new String(bytes,"UTF-8");paint.setColor(color);paint.setTextSize(size);
                    paint.setTypeface(Typeface.create((style&3)==1?"serif":(style&3)==2?"monospace":"sans-serif",(style&4)!=0?Typeface.BOLD:Typeface.NORMAL));
                    android.text.TextPaint tp=new android.text.TextPaint(paint);
                    CharSequence clipped=TextUtils.ellipsize(value,tp,w,TextUtils.TruncateAt.END);
                    canvas.drawText(clipped.toString(),x,y,paint);
                } else break;
            }} catch(Exception ignored){} canvas.restore();
        }
        @Override public boolean onTouchEvent(MotionEvent event) {
            if(event.getAction()==MotionEvent.ACTION_UP && settings!=null){
                float d=getResources().getDisplayMetrics().density;
                int action=Native.tap(settings,state,(int)(getWidth()/d),(int)(getHeight()/d),(int)(event.getX()/d),(int)(event.getY()/d),System.currentTimeMillis()/1000,SystemClock.elapsedRealtime());
                if(action==1)update(true);else if(action==2)showActions();invalidate();performClick();
            } return true;
        }
        @Override public boolean performClick(){super.performClick();return true;}
    }
}
