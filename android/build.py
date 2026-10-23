#!/usr/bin/env python3
"""Build a native Rust/Java APK with a JDK and <70MiB of pinned Android tools.

No Gradle, sdkmanager, Android Studio, Android SDK installation or NDK required.
Linux x86_64 host; each native library is linked by Rust's bundled rust-lld.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
ANDROID = ROOT / "android"
TARGETS = {"armeabi-v7a": "armv7-linux-androideabi", "arm64-v8a": "aarch64-linux-android"}

def run(args, **kwargs):
    print("+", " ".join(map(str, args)), flush=True)
    return subprocess.run(list(map(str, args)), cwd=ROOT, check=True, **kwargs)

def tools(cache):
    cache.mkdir(parents=True, exist_ok=True)
    for name, spec in json.loads((ANDROID / "tools.json").read_text()).items():
        output = cache / name
        if output.exists() and hashlib.sha256(output.read_bytes()).hexdigest() == spec["sha256"]:
            continue
        print("Downloading", name, flush=True)
        with urllib.request.urlopen(spec["url"], timeout=120) as response:
            data = response.read()
        if spec.get("base64"):
            data = base64.b64decode(data, validate=False)
        if hashlib.sha256(data).hexdigest() != spec["sha256"]:
            raise RuntimeError(f"Checksum mismatch for {name}")
        temporary = output.with_suffix(".download")
        temporary.write_bytes(data)
        temporary.replace(output)
    with zipfile.ZipFile(cache / "aapt2.jar") as archive:
        (cache / "aapt2").write_bytes(archive.read("aapt2"))
    (cache / "aapt2").chmod(0o755)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=ANDROID / ".cache")
    parser.add_argument("--java-home", type=Path, default=os.environ.get("JAVA_HOME"))
    parser.add_argument("--abi", choices=TARGETS, action="append")
    parser.add_argument("--keystore", type=Path, help="Stable PKCS12 key with alias timetable; password in TIMETABLE_KEY_PASSWORD")
    args = parser.parse_args()
    started = time.monotonic()
    java = lambda name: args.java_home / "bin" / name if args.java_home else shutil.which(name)
    for name in ["java", "javac", "jar", "keytool"]:
        if not java(name):
            parser.error("A JDK17+ is required (set --java-home or JAVA_HOME)")
    tools(args.cache)
    build = ANDROID / "build"
    if build.exists():
        shutil.rmtree(build)
    build.mkdir()
    classes = build / "classes"
    classes.mkdir()
    abis = args.abi or list(TARGETS)
    for abi in abis:
        target = TARGETS[abi]
        link = build / "sysroot" / abi
        link.mkdir(parents=True)
        run(["rustc", ANDROID / "build-tools/libc-stub.rs", "--crate-type", "cdylib", "--target", target,
             "-C", "panic=abort", "-C", "opt-level=z", "-C", "linker=rust-lld", "-C", "link-arg=--soname=libc.so", "-o", link / "libc.so"])
        env = os.environ.copy()
        env["RUSTFLAGS"] = f"-C panic=abort -C linker=rust-lld -C link-arg=-L{link} -C link-arg=-z -C link-arg=max-page-size=16384 -C opt-level=z -C codegen-units=1 -C strip=symbols"
        run(["cargo", "rustc", "-p", "timetable-android", "--lib", "--crate-type", "cdylib", "--release", "--locked", "--target", target], env=env)
        native = ROOT / "target" / target / "release/libtimetable_android.so"
        imports = subprocess.check_output(["readelf", "--dyn-syms", "--wide", str(native)], text=True)
        undefined = {line.split()[-1] for line in imports.splitlines() if " UND " in line and len(line.split()) >= 8}
        if undefined - {"abort", "memcpy", "memmove", "memset", "memcmp", "strlen"}:
            raise RuntimeError(f"Unexpected native imports: {sorted(undefined)}")
        destination = build / "lib" / abi
        destination.mkdir(parents=True)
        shutil.copyfile(native, destination / native.name)
    run([java("javac"), "-source", "8", "-target", "8", "-bootclasspath", args.cache / "android.jar",
         "-d", classes, *sorted((ANDROID / "java").rglob("*.java"))])
    run([java("jar"), "cf", build / "classes.jar", "-C", classes, "."])
    run([java("java"), "-cp", args.cache / "r8.jar", "com.android.tools.r8.D8", "--release", "--min-api", "21",
         "--lib", args.cache / "android.jar", "--output", build, build / "classes.jar"])
    run([args.cache / "aapt2", "compile", "--dir", ANDROID / "resources", "-o", build / "resources.zip"])
    unsigned = build / "unsigned.apk"
    run([args.cache / "aapt2", "link", "-I", args.cache / "android.jar", "--manifest", ANDROID / "AndroidManifest.xml",
         "-A", ANDROID / "assets", "-o", unsigned, build / "resources.zip"])
    with zipfile.ZipFile(unsigned, "a", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        archive.write(build / "classes.dex", "classes.dex")
        for native in sorted((build / "lib").rglob("*.so")):
            archive.write(native, native.relative_to(build))
    keystore = args.keystore or args.cache / "development.p12"
    env = os.environ.copy()
    if not args.keystore:
        env.setdefault("TIMETABLE_KEY_PASSWORD", "android-development")
    if not env.get("TIMETABLE_KEY_PASSWORD"):
        parser.error("Set TIMETABLE_KEY_PASSWORD for the provided signing key")
    if not keystore.exists():
        if args.keystore:
            parser.error("Provided signing key does not exist")
        run([java("keytool"), "-genkeypair", "-keystore", keystore, "-storetype", "PKCS12", "-storepass:env", "TIMETABLE_KEY_PASSWORD",
             "-alias", "timetable", "-keyalg", "RSA", "-keysize", "2048", "-dname", "CN=Timetable development", "-validity", "10000"], env=env)
    run([java("javac"), "-cp", args.cache / "apksig.jar", "-d", build, ANDROID / "build-tools/Sign.java"])
    output = ANDROID / "dist"
    output.mkdir(exist_ok=True)
    apk = output / "timetable.apk"
    command = [java("java"), "-cp", f"{args.cache / 'apksig.jar'}:{build}", "Sign", keystore, unsigned, apk]
    certificate = subprocess.check_output(list(map(str, command)), cwd=ROOT, env=env, text=True).strip()
    if not re.fullmatch("[0-9a-f]{64}", certificate):
        raise RuntimeError("Invalid APK signer output")
    data = apk.read_bytes()
    metadata = {"version": 1, "package": "org.nostdrs.timetable", "min_sdk": 21, "abis": abis,
                "file": "timetable.apk", "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                "certificate_sha256": certificate, "signing": "provided" if args.keystore else "development"}
    (output / "latest.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"Verified v1/v2 APK: {apk} ({len(data):,} bytes), {time.monotonic()-started:.1f}s", flush=True)

if __name__ == "__main__":
    main()
