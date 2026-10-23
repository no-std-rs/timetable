package org.nostdrs.timetable;

import java.io.*;
import java.net.*;
import java.nio.file.*;
import java.util.*;

/** Real disk cache with deterministic HTTP peers; no Android SDK or live feeds. */
public final class FeedCacheSmoke {
    static final String ORIGIN="https://files.mobilitydatabase.org/test/latest.zip";
    static final String PINNED="https://files.mobilitydatabase.org/test/pinned.zip";
    static void require(boolean value){if(!value)throw new AssertionError();}
    static final class Response extends HttpURLConnection {
        final int code;final byte[] bytes;final Map<String,String> headers=new HashMap<String,String>();
        boolean read,closed;
        Response(int code,String body) throws Exception{super(new URL(ORIGIN));this.code=code;bytes=body.getBytes("UTF-8");headers.put("Content-Length",Integer.toString(bytes.length));}
        Response header(String name,String value){headers.put(name,value);return this;}
        public int getResponseCode(){return code;}
        public String getHeaderField(String name){return headers.get(name);}
        public InputStream getInputStream(){read=true;if(code==304)throw new AssertionError("304 has no body");return new ByteArrayInputStream(bytes);}
        public void connect(){} public boolean usingProxy(){return false;} public void disconnect(){closed=true;}
    }
    static final class Transport implements FeedCache.Transport {
        final ArrayDeque<Response> queue=new ArrayDeque<Response>();final List<String> urls=new ArrayList<String>();
        boolean cancelled;
        Transport(Response... responses){queue.addAll(Arrays.asList(responses));}
        public HttpURLConnection open(URL url){urls.add(url.toString());return queue.remove();}
        public void check() throws IOException{if(cancelled)throw new IOException("Cancelled");}
    }
    static final FeedCache.Validate VALIDATE=new FeedCache.Validate(){public void check(File file,long checked) throws Exception{
        require(checked>0);if(!new String(Files.readAllBytes(file.toPath()),"UTF-8").startsWith("valid:"))throw new IOException("Invalid ZIP fixture");
    }};
    static void fails(File file,File metadata,FeedCache.State state,Transport transport,FeedCache.Validate validate) throws Exception {
        String before=FeedCache.digest(file),sidecar=FeedCache.digest(metadata);
        try{FeedCache.refresh(file,metadata,ORIGIN,state,transport,validate);throw new AssertionError("Expected failure");}catch(IOException expected){}
        require(before.equals(FeedCache.digest(file)));require(sidecar.equals(FeedCache.digest(metadata)));
        for(Response response:transport.queue)require(!response.read);
    }
    public static void main(String[] args) throws Exception {
        Path dir=Files.createTempDirectory("timetable-http-test");File archive=dir.resolve("feed.zip").toFile(),metadata=dir.resolve("feed.http").toFile();
        try {
            require(FeedCache.state(archive,metadata,ORIGIN)==null);
            Response first=new Response(200,"valid:first").header("ETag","\"v1\"").header("Last-Modified","Wed, 21 Oct 2015 07:28:00 GMT");
            FeedCache.refresh(archive,metadata,ORIGIN,null,new Transport(first),VALIDATE);require(first.read && first.closed);
            FeedCache.State state=FeedCache.state(archive,metadata,ORIGIN);require(state!=null && state.checkedAt()>0);
            long modified=archive.lastModified();String digest=FeedCache.digest(archive);
            Response unchanged=new Response(304,"").header("Content-Length","9000000");
            FeedCache.refresh(archive,metadata,ORIGIN,state,new Transport(unchanged),VALIDATE);
            require("\"v1\"".equals(unchanged.getRequestProperty("If-None-Match")));require(unchanged.getRequestProperty("If-Modified-Since")==null);
            require(!unchanged.read && unchanged.closed && archive.lastModified()==modified && digest.equals(FeedCache.digest(archive)));
            fails(archive,metadata,state,new Transport(new Response(304,"").header("ETag","\"wrong\"")),VALIDATE);
            fails(archive,metadata,state,new Transport(new Response(200,"invalid")),VALIDATE);
            Response shortBody=new Response(200,"valid:short").header("Content-Length","100");
            fails(archive,metadata,state,new Transport(shortBody),VALIDATE);
            final Transport cancel=new Transport(new Response(200,"valid:cancelled"));
            fails(archive,metadata,state,cancel,new FeedCache.Validate(){public void check(File file,long checked) throws Exception{VALIDATE.check(file,checked);cancel.cancelled=true;}});
            // URL-specific validators follow the old representation, never a redirect's new target.
            Response redirect=new Response(302,"").header("Location",PINNED),pinned=new Response(200,"valid:second").header("Last-Modified","Wed, 21 Oct 2015 07:28:00 GMT");
            FeedCache.refresh(archive,metadata,ORIGIN,state,new Transport(redirect,pinned),VALIDATE);
            require(pinned.getRequestProperty("If-None-Match")==null);
            state=FeedCache.state(archive,metadata,ORIGIN);
            Response redirectAgain=new Response(302,"").header("Location",PINNED),notModified=new Response(304,"");
            FeedCache.refresh(archive,metadata,ORIGIN,state,new Transport(redirectAgain,notModified),VALIDATE);
            require(redirectAgain.getRequestProperty("If-Modified-Since")==null);
            require("Wed, 21 Oct 2015 07:28:00 GMT".equals(notModified.getRequestProperty("If-Modified-Since")));
            // A replaced/corrupt ZIP cannot borrow the sidecar's successful check or validators.
            Files.write(archive.toPath(),"valid:unrelated".getBytes("UTF-8"));require(FeedCache.state(archive,metadata,ORIGIN)==null);
            fails(archive,metadata,null,new Transport(new Response(304,"")),VALIDATE);
            require(FeedCache.state(archive,metadata,ORIGIN+"other")==null);
            require(FeedCache.safe("bad\r\nInjected: yes")==null);
            for(File file:dir.toFile().listFiles())require(!file.getName().endsWith(".download"));
            System.out.println("Full ZIP cache: 200/304, redirects, validation, cancellation, corruption and recovery passed");
        } finally {for(File file:dir.toFile().listFiles())file.delete();dir.toFile().delete();}
    }
}
