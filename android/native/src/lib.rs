#![no_std]
//! Thin JNI boundary. Android owns direct buffers; safe Rust owns the app logic.
//! No Android native APIs, allocator, C++ runtime, SDK sysroot or NDK are linked.
#![allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
#[cfg(any(test, feature = "host-test"))]
extern crate std;
use jni_sys::{JNIEnv, jboolean, jclass, jint, jlong, jobject};
use timetable_tablet::{DisplayList, Settings, State, board, feed};

// Bionic exports these stable C symbols on API21+. The tiny link-only stub used
// by the builder is never packaged; Android supplies the real libc at runtime.
#[cfg(target_os = "android")]
#[link(name = "c")]
unsafe extern "C" {
    fn abort() -> !;
}
#[cfg(all(target_os = "android", not(test)))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { abort() }
}

#[derive(Clone, Copy)]
struct Region {
    pointer: *mut u8,
    len: usize,
}
impl Region {
    unsafe fn get(env: *mut JNIEnv, object: jobject, max: usize) -> Option<Self> {
        if env.is_null() || object.is_null() {
            return None;
        }
        // JNI guarantees a valid environment for every entry. These methods do
        // not retain references; the Java argument keeps its direct buffer alive.
        let api = unsafe { &(**env).v1_4 };
        let len = unsafe { (api.GetDirectBufferCapacity)(env, object) };
        let pointer = unsafe { (api.GetDirectBufferAddress)(env, object) }.cast::<u8>();
        let len = usize::try_from(len).ok()?;
        if pointer.is_null() || len == 0 || len > max {
            return None;
        }
        Some(Self { pointer, len })
    }
    unsafe fn writable(env: *mut JNIEnv, object: jobject, max: usize) -> Option<Self> {
        let region = unsafe { Self::get(env, object, max) }?;
        // A read-only mapped buffer can expose an address through JNI, but
        // writing through it would fault even though its capacity is valid.
        let api = unsafe { &(**env).v1_4 };
        let class = unsafe { (api.GetObjectClass)(env, object) };
        if class.is_null() {
            return None;
        }
        let method =
            unsafe { (api.GetMethodID)(env, class, c"isReadOnly".as_ptr(), c"()Z".as_ptr()) };
        unsafe { (api.DeleteLocalRef)(env, class) };
        if method.is_null() {
            return None;
        }
        let read_only = unsafe { (api.CallBooleanMethodA)(env, object, method, core::ptr::null()) };
        (!read_only).then_some(region)
    }
    fn overlaps(self, other: Self) -> bool {
        (self.pointer as usize).abs_diff(other.pointer as usize)
            < if self.pointer as usize <= other.pointer as usize {
                self.len
            } else {
                other.len
            }
    }
    unsafe fn read<'a>(self) -> &'a [u8] {
        unsafe { core::slice::from_raw_parts(self.pointer, self.len) }
    }
    unsafe fn write<'a>(self) -> &'a mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.pointer, self.len) }
    }
}
fn distinct(regions: &[Region]) -> bool {
    regions
        .iter()
        .enumerate()
        .all(|(i, r)| regions[i + 1..].iter().all(|s| !r.overlaps(*s)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_nostdrs_timetable_Native_validate(
    env: *mut JNIEnv,
    _: jclass,
    settings: jobject,
) -> jint {
    unsafe { Region::get(env, settings, timetable_tablet::SETTINGS_BYTES) }
        .and_then(|r| Settings::decode(unsafe { r.read() }).ok())
        .map_or(0, |_| 1)
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_nostdrs_timetable_Native_render(
    env: *mut JNIEnv,
    _: jclass,
    settings: jobject,
    snapshot: jobject,
    state: jobject,
    frame: jobject,
    width: jint,
    height: jint,
    now: jlong,
    monotonic: jlong,
    battery: jint,
    plugged: jboolean,
    motion: jboolean,
    status: jint,
) -> jint {
    let work = || -> Result<usize, &'static str> {
        let settings = unsafe { Region::get(env, settings, timetable_tablet::SETTINGS_BYTES) }
            .ok_or("Settings")?;
        let state = unsafe { Region::writable(env, state, timetable_tablet::STATE_BYTES) }
            .ok_or("State")?;
        let frame = unsafe { Region::writable(env, frame, timetable_tablet::FRAME_BYTES) }
            .ok_or("Frame")?;
        let snapshot = unsafe { Region::get(env, snapshot, timetable_tablet::SNAPSHOT_BYTES) };
        if !(240..=4096).contains(&width)
            || !(240..=4096).contains(&height)
            || !distinct(&[settings, state, frame])
            || snapshot.is_some_and(|r| [settings, state, frame].iter().any(|s| r.overlaps(*s)))
        {
            return Err("Buffers");
        }
        // Regions have been checked for aliasing before creating any references.
        let settings_bytes = unsafe { settings.read() };
        let cfg = Settings::decode(settings_bytes)?;
        if cfg.timezone().local(now).is_none() {
            return Err("Clock is outside timezone coverage");
        }
        let state_bytes = unsafe { state.write() };
        let mut model = State::load(state_bytes);
        let mut app = timetable_display::App::default();
        if let Some(snapshot) = snapshot {
            let _ = timetable_tablet::snapshot::restore(
                settings_bytes,
                unsafe { snapshot.read() },
                &mut app,
            );
        }
        let mut list = DisplayList::new(unsafe { frame.write() })?;
        let flags = board::draw(
            &cfg,
            &mut app,
            &mut model,
            board::Input {
                width,
                height,
                now,
                monotonic_ms: monotonic,
                battery,
                plugged,
                motion_available: motion,
                status: status as u8,
            },
            &mut list,
        )?;
        model.save(state_bytes)?;
        Ok(list.finish(flags, app.group))
    };
    work().map_or(-1, |n| n as jint)
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_nostdrs_timetable_Native_tap(
    env: *mut JNIEnv,
    _: jclass,
    settings: jobject,
    state: jobject,
    width: jint,
    height: jint,
    x: jint,
    y: jint,
    now: jlong,
    monotonic: jlong,
) -> jint {
    let work = || -> Option<u8> {
        let settings = unsafe { Region::get(env, settings, timetable_tablet::SETTINGS_BYTES) }?;
        let state = unsafe { Region::writable(env, state, timetable_tablet::STATE_BYTES) }?;
        if settings.overlaps(state)
            || !(240..=4096).contains(&width)
            || !(240..=4096).contains(&height)
        {
            return None;
        }
        let cfg = Settings::decode(unsafe { settings.read() }).ok()?;
        let data = unsafe { state.write() };
        let mut model = State::load(data);
        let refresh = board::tap(&cfg, &mut model, (width, height), (x, y), now, monotonic);
        model.save(data).ok()?;
        Some(refresh)
    };
    work().map_or(-1, jint::from)
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_nostdrs_timetable_Native_query(
    env: *mut JNIEnv,
    _: jclass,
    settings: jobject,
    archive: jobject,
    output: jobject,
    now: jlong,
    checked_at: jlong,
) -> jint {
    let work = || -> Result<usize, &'static str> {
        let settings = unsafe { Region::get(env, settings, timetable_tablet::SETTINGS_BYTES) }
            .ok_or("Settings")?;
        let archive = unsafe { Region::get(env, archive, 512 * 1024 * 1024) }.ok_or("Archive")?;
        let output = unsafe { Region::writable(env, output, timetable_tablet::SNAPSHOT_BYTES) }
            .ok_or("Output")?;
        if !distinct(&[settings, archive, output]) {
            return Err("Buffers");
        }
        feed::immediate(feed::query(
            unsafe { settings.read() },
            unsafe { archive.read() },
            now,
            checked_at,
            unsafe { output.write() },
        ))?
    };
    work().map_or(-1, |n| n as jint)
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_nostdrs_timetable_Native_motion(
    env: *mut JNIEnv,
    _: jclass,
    state: jobject,
    luma: jobject,
    width: jint,
    height: jint,
) -> jint {
    let work = || -> Option<bool> {
        let state = unsafe { Region::writable(env, state, timetable_tablet::motion::BYTES) }?;
        let frame = unsafe { Region::get(env, luma, 4096 * 4096) }?;
        if state.overlaps(frame) {
            return None;
        }
        timetable_tablet::motion::detect(
            unsafe { state.write() },
            unsafe { frame.read() },
            width.try_into().ok()?,
            height.try_into().ok()?,
        )
        .ok()
    };
    work().map_or(-1, jint::from)
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_nostdrs_timetable_Native_activity(
    env: *mut JNIEnv,
    _: jclass,
    state: jobject,
    monotonic: jlong,
) -> jint {
    let Some(state) = (unsafe { Region::writable(env, state, timetable_tablet::STATE_BYTES) })
    else {
        return -1;
    };
    let bytes = unsafe { state.write() };
    let mut model = State::load(bytes);
    model.active(monotonic);
    model.save(bytes).map_or(-1, |()| 0)
}

// Prebuilt core can retain an unwind-table reference even with panic=abort.
// Unwinding across JNI is forbidden: any such unexpected path terminates.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() -> ! {
    unsafe { abort() }
}
