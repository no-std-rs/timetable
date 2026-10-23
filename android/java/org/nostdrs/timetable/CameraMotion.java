package org.nostdrs.timetable;

import android.graphics.ImageFormat;
import android.hardware.Camera;
import android.os.SystemClock;
import android.view.*;
import android.widget.FrameLayout;
import java.nio.ByteBuffer;
import java.util.List;

/** Optional local camera input. Rust samples luma and makes the motion decision. */
@SuppressWarnings("deprecation")
final class CameraMotion implements SurfaceHolder.Callback,Camera.PreviewCallback {
    final MainActivity app;
    final SurfaceView surface;
    final ByteBuffer history=ByteBuffer.allocateDirect(800);
    Camera camera;
    ByteBuffer luma;
    int width,height;
    long sampled;
    boolean active,available;
    CameraMotion(MainActivity app,FrameLayout parent){
        this.app=app;surface=new SurfaceView(app);surface.getHolder().addCallback(this);
        // A real, tiny preview surface keeps API21 camera drivers producing
        // frames without depending on a WebView or a GPU inference framework.
        parent.addView(surface,new FrameLayout.LayoutParams(2,2,Gravity.BOTTOM|Gravity.RIGHT));
    }
    public void surfaceCreated(SurfaceHolder holder){available=true;app.startCamera();}
    public void surfaceChanged(SurfaceHolder holder,int format,int w,int h){}
    public void surfaceDestroyed(SurfaceHolder holder){available=false;stop();}
    void start(){
        if(camera!=null || !available)return;
        try {
            int id=-1;Camera.CameraInfo info=new Camera.CameraInfo();
            for(int i=0;i<Camera.getNumberOfCameras();i++){Camera.getCameraInfo(i,info);if(info.facing==Camera.CameraInfo.CAMERA_FACING_FRONT){id=i;break;}}
            if(id<0)return;
            camera=Camera.open(id);Camera.Parameters p=camera.getParameters();
            Camera.Size chosen=null;
            for(Camera.Size size:p.getSupportedPreviewSizes())if(size.width>=32 && size.height>=24 && (chosen==null || size.width*size.height<chosen.width*chosen.height))chosen=size;
            if(chosen==null || chosen.width>4096 || chosen.height>4096)throw new Exception("Unsupported camera size");
            width=chosen.width;height=chosen.height;p.setPreviewSize(width,height);p.setPreviewFormat(ImageFormat.NV21);camera.setParameters(p);
            luma=ByteBuffer.allocateDirect(width*height);camera.setPreviewDisplay(surface.getHolder());
            camera.setPreviewCallbackWithBuffer(this);camera.addCallbackBuffer(new byte[width*height*3/2]);
            camera.startPreview();active=true;
        }catch(Exception e){stop();}
    }
    void stop(){active=false;if(camera!=null){camera.setPreviewCallbackWithBuffer(null);camera.release();camera=null;}luma=null;}
    public void onPreviewFrame(byte[] data,Camera source){
        try {
            long now=SystemClock.elapsedRealtime();
            if(luma!=null && now-sampled>=1000 && data.length>=width*height){
                sampled=now;luma.clear();luma.put(data,0,width*height);luma.rewind();
                if(Native.motion(history,luma,width,height)==1)Native.activity(app.state,now);
            }
        }finally{if(camera==source)source.addCallbackBuffer(data);}
    }
}
