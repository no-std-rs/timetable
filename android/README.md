# Timetable for Android tablets

A native Android host for the existing Rust timetable. The `tablet` crate reuses
`display::App`, groups, scheduling, walking estimates, `feed::Builder`, timezone
transitions, and the `gtfs` ZIP/CSV parser. Rust lays out native text and shapes
for portrait and landscape, with more departures on larger screens. Android
provides Canvas, lifecycle, HTTPS, file mapping and optional camera input.
There is no WebView, AndroidX, Google Play Services, native allocator, or Rust UI
framework. Java is the platform adapter; the browser's ADB client is Rust too.

The baseline is **Android 5.0/API21, ARMv7 or ARM64**. This includes the OS/CPU of
[2015 and 2017 Fire 7 tablets](https://www.developer.amazon.com/docs/device-specs/ft-device-specifications-fire-models.html).
Their Android 5.1-based Fire OS has no proximity or ambient-light sensor, so the
optional motion input uses the front camera. These are compatibility targets,
not physically tested devices. Original Android 4.x Fire tablets are unsupported.
The app targets API28 for direct sideloading, not Play Store publication.

## Build without an SDK/NDK installation

On Linux x86_64, install Rust and a JDK17 or newer, then:

```sh
rustup target add armv7-linux-androideabi aarch64-linux-android
python3 android/build.py
```

Outputs are `android/dist/timetable.apk` and its SHA256/signing manifest. The
builder downloads under 70MiB of checksum-pinned tools: an AOSP `android.jar`,
Google AAPT2, D8 and APK signer. Their immutable versions, URLs and SHA256 values
are in [`tools.json`](tools.json). No Gradle, sdkmanager, full SDK or NDK is
installed. Pass `--java-home /path/to/jdk` or set `JAVA_HOME` if needed. Tools are
cached in `android/.cache`; `--cache` selects another directory.

Rust's bundled linker builds the JNI library against a tiny **link-only** libc
stub. That stub is never packaged. The actual library imports only the checked
Bionic C functions and calls JNI through Android's function table. The builder
rejects unexpected imports, sets 16KiB ELF segment alignment, and verifies both
v1 and v2 APK signatures for API21+. This deliberately small ABI boundary avoids
an NDK sysroot; adding new native platform APIs requires revisiting that choice.

A measured rebuild with tools/dependencies already cached took **11 seconds**
and produced a **283KB APK including both ABIs** on the development host. Cold
CI also needs downloads and Rust compilation; these are not cold-build timings.
The Android CI job uses the runner's existing JDK, tests JNI in a real host JVM,
and uploads the APK before the website is assembled from that same run.

## Signing and updates

Without `--keystore`, the builder creates a development PKCS12 key in the tools
cache. Keep it to update a development installation. CI caches this development
key with its tools; cache eviction can change it, and it is **not a release key**.
The site explicitly labels these builds. Android refuses an update signed by a
different certificate; provisioning reports that refusal and never uninstalls
the app or deletes its data to work around it.

For a stable installation, keep a private PKCS12 keystore with alias `timetable`:

```sh
export TIMETABLE_KEY_PASSWORD='your-keystore-password'
python3 android/build.py --keystore /private/path/timetable.p12
```

Keep the keystore outside the checkout and CI caches. The deployment workflow
currently publishes development builds; production signing needs a private
keystore supplied to the builder. Increment `versionCode`/`versionName` in
`AndroidManifest.xml` when publishing a versioned release. Neither APKs nor
private keys belong in source control.

## Provision from the website

Build the browser transport with `wasm-bindgen-cli` 0.2.129 installed:

```sh
rustup target add wasm32-unknown-unknown --toolchain 1.95.0
cargo +1.95.0 install wasm-bindgen-cli --version 0.2.129 --locked
cd web
npm ci
npm run build
npm run test:browser
```

The Android panel uses the site's existing network and groups. It does not need
or transmit the ESP32 Wi-Fi fields: connect the tablet to Wi-Fi in Android's
settings. In desktop Chrome/Edge over HTTPS or localhost, enable USB debugging,
connect, approve the computer on the tablet, and choose Install and configure.
Linux may require the usual Android USB/udev permissions; on Windows a competing
ADB driver may prevent WebUSB from claiming the interface. Stop other ADB clients
before connecting. The browser generates an ephemeral ADB key locally; it is
never uploaded or persisted, so a later page session may need authorization again.

`provisioner` pins the WebUSB implementation from
[adb_client PR222](https://github.com/cocool97/adb_client/pull/222) to commit
`66ce40c6588f1f103d7db0f9209d7471a0309dd5`. It checks tablet API/ABI, verifies the
APK SHA256, sends it using ADB sync, installs with `pm install -r`, and sends
Rust-validated settings to a receiver protected by `android.permission.DUMP`
(held by the ADB shell). It checks installation/configuration responses before
launching. Stop closes USB and interrupts pending work; an already installed app
is retained. Shell arguments are quoted and legacy shell exit codes are checked.

## Power, Home and kiosk behavior

- **Keep awake:** Android's window flag keeps the display on while the app is
  visible. Turning it off restores the system's usual timeout.
- **Motion dimming:** opt-in camera permission, foreground only. Rust compares a
  small luma grid about once per second, with two-sample debounce. Images remain
  in RAM and are never saved/uploaded. This is movement detection, not a person
  classifier. The screen dims; it does not enter deep sleep. Camera failure or
  refusal leaves normal brightness and a working timetable.
- **Home:** opt-in ADB selection where supported, otherwise opens Home settings
  for the user to choose. Some Fire OS versions block third-party launchers.
  No Amazon launcher is disabled or removed. Optional launch-at-boot remains
  subject to Android's background activity restrictions.
- **Device owner:** separate explicit opt-in for a dedicated tablet. Android may
  require no accounts or existing owner, and vendor firmware may refuse. Setup
  never factory-resets a tablet. An accepted owner allows persistent Home and
  lock-task mode. The app's visible **••• → Exit kiosk and open settings** stops
  lock task and clears its persistent Home preference. Reprovision to re-enable.
- **80% charging:** unsupported through ordinary Android app/ADB controls. A
  real cap needs a vendor feature, privileged/root control, or external charging
  hardware. The UI states this limitation; it does not fake a cap using Android's
  battery simulation commands. No root or sysfs writes are performed.

## Cached data and network limits

A matching CRC-checked departure snapshot is drawn before any networking. A
background worker queries a memory-mapped cached ZIP, then downloads an update
when the ZIP is three hours old (or Refresh is tapped). Failed downloads/parses
keep the previously validated data; a provisional ZIP is never published. The
same full-archive structure validation and selected-table CRC checks as the
shared Rust parser apply. Parsing still uses the embedded explicit capacities;
large selections/unsupported GTFS fail rather than silently dropping departures.
The footer shows the last successful source check, not the time an old ZIP was
queried again. Cached departures cover six hours; this is not an offline route
planner. Files live in the app's private storage.

HTTPS uses Android's certificate/hostname verification, TLS1.2, and two additional
public CA roots for older trust stores: [Amazon Root CA1](https://www.amazontrust.com/repository/AmazonRootCA1.pem)
and [ISRG Root X1](https://letsencrypt.org/certs/isrgrootx1.pem), bundled verbatim in
`assets/trust-anchors.pem`. Requests/redirects are restricted to
`files.mobilitydatabase.org`, with at most five redirects, a 512MiB ZIP limit,
finite socket timeouts and exact Content-Length checks when present.

## Offline checks and design previews

```sh
cargo test -p timetable-tablet --locked
cargo check -p timetable-tablet --lib --locked --target riscv32imac-unknown-none-elf
sh tools/test-android-jni.sh
cargo test --manifest-path android/provisioner/Cargo.toml --locked
cargo run -p timetable-tablet --example preview
```

JNI smoke tests load the actual native implementation into a host JVM, query the
synthetic GTFS fixture, render both orientations, select a group, and reject
aliased/non-direct buffers. Browser tests mock the USB boundary and check settings,
opt-ins, failures and stopping. The SVG example uses the same Rust layout with
sample departures and writes `/tmp/timetable-tablet-*.svg`. These checks do not
replace a physical Fire OS test of Home policies, camera behavior, TLS and startup.
