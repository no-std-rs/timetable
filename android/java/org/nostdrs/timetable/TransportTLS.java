package org.nostdrs.timetable;

import android.content.Context;
import java.io.InputStream;
import java.security.*;
import java.security.cert.*;
import java.util.*;
import javax.net.ssl.*;

/** Keep platform validation, adding narrowly bundled public CA roots for old Fire OS. */
final class TransportTLS {
    private static SSLSocketFactory cached;
    static synchronized SSLSocketFactory factory(Context context) throws Exception {
        if(cached!=null)return cached;
        TrustManagerFactory platform=TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());platform.init((KeyStore)null);
        KeyStore roots=KeyStore.getInstance(KeyStore.getDefaultType());roots.load(null);
        InputStream in=context.getAssets().open("trust-anchors.pem");
        Collection<? extends java.security.cert.Certificate> certificates;
        try{certificates=CertificateFactory.getInstance("X.509").generateCertificates(in);}finally{in.close();}
        int n=0;for(java.security.cert.Certificate cert:certificates)roots.setCertificateEntry("timetable-"+(n++),cert);
        TrustManagerFactory extra=TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());extra.init(roots);
        final X509TrustManager first=manager(platform),second=manager(extra);
        X509TrustManager combined=new X509TrustManager(){
            public X509Certificate[] getAcceptedIssuers(){X509Certificate[] a=first.getAcceptedIssuers(),b=second.getAcceptedIssuers();X509Certificate[] both=Arrays.copyOf(a,a.length+b.length);System.arraycopy(b,0,both,a.length,b.length);return both;}
            public void checkClientTrusted(X509Certificate[] chain,String auth)throws CertificateException{first.checkClientTrusted(chain,auth);}
            public void checkServerTrusted(X509Certificate[] chain,String auth)throws CertificateException{
                try{first.checkServerTrusted(chain,auth);}catch(CertificateException oldStore){second.checkServerTrusted(chain,auth);}
            }
        };
        SSLContext tls=SSLContext.getInstance("TLSv1.2");tls.init(null,new TrustManager[]{combined},null);cached=tls.getSocketFactory();return cached;
    }
    static X509TrustManager manager(TrustManagerFactory factory)throws Exception{
        for(TrustManager value:factory.getTrustManagers())if(value instanceof X509TrustManager)return (X509TrustManager)value;
        throw new CertificateException("No trust manager");
    }
}
