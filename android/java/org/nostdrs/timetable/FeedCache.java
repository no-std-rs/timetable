package org.nostdrs.timetable;

import java.io.*;
import java.net.*;
import java.security.*;
import java.util.Properties;

/** Full ZIP cache. Validators belong to a URL and the exact validated bytes. */
final class FeedCache {
    interface Transport { HttpURLConnection open(URL url) throws Exception; void check() throws IOException; }
    interface Validate { void check(File file,long checkedAt) throws Exception; }
    static final long LIMIT=512L*1024*1024;
    static final class State {
        final Properties values;
        State(Properties values){this.values=values;}
        long checkedAt(){try{return Long.parseLong(values.getProperty("checked","0"));}catch(NumberFormatException e){return 0;}}
    }
    static State state(File archive,File metadata,String origin) {
        Properties p=new Properties();
        try {
            if(!archive.isFile() || metadata.length()>8192) return null;
            FileInputStream in=new FileInputStream(metadata);try{p.load(in);}finally{in.close();}
            if(!origin.equals(p.getProperty("origin")) || !digest(archive).equals(p.getProperty("sha256")))return null;
            return new State(p);
        } catch(Exception ignored){return null;}
    }
    static long refresh(File archive,File metadata,String origin,State old,Transport transport,Validate validate) throws Exception {
        URL url=new URL(origin); File temporary=null;
        try {
            for(int redirects=0;redirects<=5;redirects++) {
                transport.check();
                // A redirect may select a new representation, never send another URL's validators.
                HttpURLConnection request=transport.open(url);
                try {
                    request.setInstanceFollowRedirects(false);request.setConnectTimeout(15000);request.setReadTimeout(30000);
                    request.setRequestProperty("Accept-Encoding","identity");
                    boolean conditional=false;
                    if(old!=null && url.toString().equals(old.values.getProperty("url"))) {
                        String etag=safe(old.values.getProperty("etag")),modified=safe(old.values.getProperty("modified"));
                        if(etag!=null){request.setRequestProperty("If-None-Match",etag);conditional=true;}
                        else if(modified!=null){request.setRequestProperty("If-Modified-Since",modified);conditional=true;}
                    }
                    int code=request.getResponseCode();long checked=System.currentTimeMillis()/1000;
                    if(code==304) {
                        if(!conditional)throw new IOException("Unexpected HTTP 304 without a validated cache");
                        String responseTag=request.getHeaderField("ETag"),storedTag=old.values.getProperty("etag");
                        if(responseTag!=null && storedTag!=null && !responseTag.replaceFirst("^W/","").equals(storedTag.replaceFirst("^W/","")))throw new IOException("Mismatched HTTP validator");
                        // 304 has no response body, even with the representation's Content-Length.
                        transport.check();validate.check(archive,checked);transport.check();
                        Properties p=new Properties();p.putAll(old.values);p.setProperty("checked",Long.toString(checked));
                        save(metadata,p);return checked;
                    }
                    if(code==301 || code==302 || code==303 || code==307 || code==308) {
                        String location=request.getHeaderField("Location");
                        if(location==null)throw new IOException("Invalid timetable redirect");
                        url=new URL(url,location);continue;
                    }
                    if(code!=200)throw new IOException("Timetable server returned HTTP "+code);
                    String encoding=request.getHeaderField("Content-Encoding");
                    if(encoding!=null && !"identity".equalsIgnoreCase(encoding))throw new IOException("Unsupported response encoding");
                    String length=request.getHeaderField("Content-Length");long expected=length==null?-1:Long.parseLong(length);
                    if(expected>LIMIT || expected< -1)throw new IOException("Timetable ZIP too large");
                    temporary=File.createTempFile("timetable-",".download",archive.getParentFile());
                    InputStream in=request.getInputStream();FileOutputStream out=new FileOutputStream(temporary);long total=0;
                    try {byte[] bytes=new byte[32768];int n;
                        while((n=in.read(bytes))!=-1){transport.check();total+=n;if(total>LIMIT)throw new IOException("Timetable ZIP too large");out.write(bytes,0,n);}
                        if(expected>=0 && total!=expected)throw new IOException("Truncated timetable download");
                        out.getFD().sync();
                    } finally {try{in.close();}finally{out.close();}}
                    validate.check(temporary,checked);transport.check();
                    Properties p=new Properties();p.setProperty("origin",origin);p.setProperty("url",url.toString());
                    p.setProperty("sha256",digest(temporary));p.setProperty("checked",Long.toString(checked));
                    String etag=safe(request.getHeaderField("ETag")),modified=safe(request.getHeaderField("Last-Modified"));
                    if(etag!=null)p.setProperty("etag",etag);if(modified!=null)p.setProperty("modified",modified);
                    transport.check();
                    if(!temporary.renameTo(archive))throw new IOException("Cannot save timetable ZIP");temporary=null;
                    // If power fails here, the old sidecar fails its SHA check next boot.
                    save(metadata,p);return checked;
                } finally {request.disconnect();}
            }
            throw new IOException("Too many timetable redirects");
        } finally {if(temporary!=null)temporary.delete();}
    }
    static String safe(String value){return value!=null && !value.isEmpty() && value.length()<=1024 && value.matches("[\\x20-\\x7e]+")?value:null;}
    static String digest(File file) throws Exception {
        MessageDigest hash=MessageDigest.getInstance("SHA-256");FileInputStream in=new FileInputStream(file);
        try{byte[] bytes=new byte[32768];int n;while((n=in.read(bytes))!=-1)hash.update(bytes,0,n);}finally{in.close();}
        StringBuilder text=new StringBuilder();for(byte b:hash.digest())text.append(String.format(java.util.Locale.ROOT,"%02x",b));return text.toString();
    }
    static void save(File file,Properties values) throws IOException {
        File temporary=File.createTempFile("http-",".metadata",file.getParentFile());
        try{FileOutputStream out=new FileOutputStream(temporary);try{values.store(out,null);out.getFD().sync();}finally{out.close();}
            if(!temporary.renameTo(file))throw new IOException("Cannot save HTTP cache metadata");
        }finally{temporary.delete();}
    }
}
