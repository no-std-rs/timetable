import java.io.*;
import java.security.*;
import java.security.cert.*;
import java.util.*;
import com.android.apksig.*;

/** Use Google's APK signer/verifier directly; no Gradle daemon or SDK install. */
public final class Sign {
    public static void main(String[] args) throws Exception {
        char[] password=System.getenv("TIMETABLE_KEY_PASSWORD").toCharArray();
        KeyStore store=KeyStore.getInstance("PKCS12");
        try(InputStream in=new FileInputStream(args[0])){store.load(in,password);}
        PrivateKey key=(PrivateKey)store.getKey("timetable",password);
        X509Certificate cert=(X509Certificate)store.getCertificate("timetable");
        ApkSigner.SignerConfig signer=new ApkSigner.SignerConfig.Builder("timetable",key,Collections.singletonList(cert)).build();
        new ApkSigner.Builder(Collections.singletonList(signer)).setInputApk(new File(args[1]))
            .setOutputApk(new File(args[2])).setMinSdkVersion(21)
            .setV1SigningEnabled(true).setV2SigningEnabled(true).setV3SigningEnabled(false)
            .setV4SigningEnabled(false).setLibraryPageAlignmentBytes(16384).build().sign();
        ApkVerifier.Result result=new ApkVerifier.Builder(new File(args[2])).setMinCheckedPlatformVersion(21).build().verify();
        if(!result.isVerified() || !result.isVerifiedUsingV1Scheme() || !result.isVerifiedUsingV2Scheme())
            throw new SecurityException("APK verification failed: "+result.getErrors());
        StringBuilder digest=new StringBuilder();
        for(byte b:MessageDigest.getInstance("SHA-256").digest(cert.getEncoded()))digest.append(String.format("%02x",b));
        System.out.println(digest);
    }
}
