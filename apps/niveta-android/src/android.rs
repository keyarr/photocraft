//! Android entry point: the one isolated `unsafe` item in this crate (see the
//! `[lints.rust]` note in `Cargo.toml`).
//!
//! `android-activity` declares `extern "Rust" { fn android_main(..) }`, so our
//! definition must keep the literal symbol name or the `.so` won't link.
//!
//! The storage permission bridge lives here too. Asking Android for permissions
//! (`checkSelfPermission`, `requestPermissions`) and opening the Settings
//! screens are JNI calls on the `NativeActivity` object, and turning its raw
//! pointers into `jni` handles is `unsafe`. Confining those two conversions to
//! this file keeps the crate's `unsafe` surface where it already is: every
//! public function below is safe Rust that degrades to a default (no access,
//! no-op) when JNI fails, never a panic.
//!
//! Why JNI at all: `MANAGE_EXTERNAL_STORAGE` never shows a runtime dialog by
//! OS design, it is granted only in Settings, and this stack has no Java to
//! fire intents with. The bridge fires the runtime dialog for photo access and
//! the Settings screen for all-files access; the file browser
//! (`ui-egui/src/device_browser.rs`) only sees the three hooks below through
//! `Services`, so the UI crate stays JNI-free.

#![allow(unsafe_code)]

use std::sync::{Mutex, OnceLock};

use winit::platform::android::activity::AndroidApp;

/// Runtime permissions the browser needs. Photos are covered by the media
/// permission on new devices and the storage permission on old ones; anything
/// else in shared storage (a PSD or `.pcraft` sitting in Download is not
/// "media") needs all-files access from Settings.
const READ_MEDIA_IMAGES: &str = "android.permission.READ_MEDIA_IMAGES";
const READ_EXTERNAL_STORAGE: &str = "android.permission.READ_EXTERNAL_STORAGE";

/// Settings screens, first hit wins (a missing handler throws, which is
/// routine on old or vendor ROMs, and the next one is tried).
const MANAGE_APP_ACTION: &str = "android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION";
const MANAGE_ALL_ACTION: &str = "android.settings.MANAGE_ALL_FILES_ACCESS_PERMISSION";
const APP_DETAILS_ACTION: &str = "android.settings.APPLICATION_DETAILS_SETTINGS";

/// The live activity's VM plus a global ref to the activity object, refreshed
/// on every `android_main`: the system may destroy and recreate the activity
/// (rotation, multi-window) and calls through a dead object only throw.
/// `None` until the first `android_main` runs.
struct Bridge {
    vm: jni::JavaVM,
    activity: jni::refs::Global<jni::objects::JObject<'static>>,
}

static BRIDGE: OnceLock<Mutex<Option<Bridge>>> = OnceLock::new();

/// Remember the running activity for the UI thread's permission buttons.
/// Called from `android_main` before the app moves into eframe.
pub(crate) fn remember(app: &AndroidApp) {
    // SAFETY: both pointers come from android-activity's live `AndroidApp`.
    // `from_raw` only wraps the process-wide VM singleton (which outlives us),
    // and the activity pointer is the unowned global ref android-activity
    // documents, borrowed here just long enough to mint our own `Global`,
    // which carries the lifetime from then on.
    let bridged: jni::errors::Result<Bridge> = unsafe {
        let vm = jni::JavaVM::from_raw(app.vm_as_ptr().cast());
        let raw = app.activity_as_ptr() as jni::sys::jobject;
        vm.attach_current_thread(|env| {
            let activity = jni::objects::JObject::from_raw(env, raw);
            Ok(Bridge { vm: vm.clone(), activity: env.new_global_ref(activity)? })
        })
    };
    match bridged {
        Ok(bridge) => {
            let slot = BRIDGE.get_or_init(|| Mutex::new(None));
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(bridge);
        }
        Err(e) => log::warn!("niveta-android: storage bridge unavailable: {e:?}"),
    }
}

/// Run `f` with the activity object; `None` when there is no activity yet or
/// any JNI call fails. A failed call can leave a Java exception pending on
/// this thread, so it is cleared before returning: the next attempt (e.g. the
/// next Settings fallback) must start clean.
fn jni_call<T>(f: impl FnOnce(&mut jni::Env, &jni::objects::JObject) -> jni::errors::Result<T>) -> Option<T> {
    let guard = BRIDGE.get()?.lock().unwrap_or_else(|e| e.into_inner());
    let bridge = guard.as_ref()?;
    let activity = bridge.activity.as_obj();
    match bridge.vm.attach_current_thread(|env| f(env, activity)) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("niveta-android: JNI call failed: {e:?}");
            let _ = bridge.vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok::<(), jni::errors::Error>(())
            });
            None
        }
    }
}

fn check_self(perm: &str) -> bool {
    jni_call(|env, activity| {
        let perm = jni::objects::JString::from_str(env, perm)?;
        let r = env.call_method(activity, jni::jni_str!("checkSelfPermission"), jni::jni_sig!("(Ljava/lang/String;)I"), &[(&perm).into()])?;
        Ok(r.i().is_ok_and(|v| v == 0))
    })
    .unwrap_or(false)
}

/// All-files access (API 30+; below that the method is missing and this is
/// `false`, where the storage permissions above are the whole story).
fn all_files() -> bool {
    jni_call(|env, _| {
        let r = env.call_static_method(jni::jni_str!("android/os/Environment"), jni::jni_str!("isExternalStorageManager"), jni::jni_sig!("()Z"), &[])?;
        Ok(r.z().unwrap_or(false))
    })
    .unwrap_or(false)
}

/// Whether the on-device browser can read shared storage: photo access or
/// all-files. The browser's write probe (`device_browser::has_access`) stays
/// as the off-device fallback.
pub fn storage_has_access() -> bool {
    check_self(READ_MEDIA_IMAGES) || check_self(READ_EXTERNAL_STORAGE) || all_files()
}

/// Fire the runtime dialog for photo access and return. The glue forwards no
/// permission callback, so the answer surfaces as a later
/// `storage_has_access` verdict and the gate rechecks on demand.
pub fn storage_request() {
    let done = jni_call(|env, activity| {
        let first = jni::objects::JString::from_str(env, READ_MEDIA_IMAGES)?;
        let second = jni::objects::JString::from_str(env, READ_EXTERNAL_STORAGE)?;
        let arr = jni::objects::JObjectArray::<jni::objects::JString>::new(env, 2, &first)?;
        arr.set_element(env, 1, &second)?;
        env.call_method(activity, jni::jni_str!("requestPermissions"), jni::jni_sig!("([Ljava/lang/String;I)V"), &[(&arr).into(), 1i32.into()])?;
        Ok(())
    });
    if done.is_none() {
        log::warn!("niveta-android: storage request failed");
    }
}

fn package_name(env: &mut jni::Env, activity: &jni::objects::JObject) -> jni::errors::Result<String> {
    let obj = env.call_method(activity, jni::jni_str!("getPackageName"), jni::jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
    let s = env.as_cast::<jni::objects::JString>(&obj)?;
    Ok(s.to_string())
}

fn start_activity(env: &mut jni::Env, activity: &jni::objects::JObject, action: &str, with_package: bool) -> jni::errors::Result<()> {
    let act = jni::objects::JString::from_str(env, action)?;
    let intent = if with_package {
        let pkg = package_name(env, activity)?;
        let uri_str = jni::objects::JString::from_str(env, format!("package:{pkg}"))?;
        let uri = env
            .call_static_method(
                jni::jni_str!("android/net/Uri"),
                jni::jni_str!("parse"),
                jni::jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[(&uri_str).into()],
            )?
            .l()?;
        env.new_object(jni::jni_str!("android/content/Intent"), jni::jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"), &[(&act).into(), (&uri).into()])?
    } else {
        env.new_object(jni::jni_str!("android/content/Intent"), jni::jni_sig!("(Ljava/lang/String;)V"), &[(&act).into()])?
    };
    env.call_method(activity, jni::jni_str!("startActivity"), jni::jni_sig!("(Landroid/content/Intent;)V"), &[(&intent).into()])?;
    Ok(())
}

/// Open the all-files screen for this app, falling back to the all-apps list
/// and then the app details. The user grants there and taps Check again back
/// in the browser.
pub fn storage_settings() {
    let opened = jni_call(|env, activity| {
        // A failed intent leaves a Java exception pending; clear it so the
        // next attempt runs (a missing Settings activity is routine).
        for (action, with_package) in [(MANAGE_APP_ACTION, true), (MANAGE_ALL_ACTION, false), (APP_DETAILS_ACTION, true)] {
            env.exception_clear();
            if start_activity(env, activity, action, with_package).is_ok() {
                return Ok(true);
            }
        }
        Ok(false)
    })
    .unwrap_or(false);
    if !opened {
        log::warn!("niveta-android: could not open storage settings");
    }
}

/// Save one exported image through MediaStore (`DCIM/Niveta`), so the gallery
/// indexes it at once and no all-files grant is needed. `name` includes the
/// extension; returns the visible path. Android 10+ only: the columns used
/// here (`relative_path`, `is_pending`) arrived with scoped storage.
pub fn save_to_gallery(name: &str, bytes: &[u8]) -> Result<String, String> {
    let sdk = jni_call(|env, _| {
        env.get_static_field(jni::jni_str!("android/os/Build$VERSION"), jni::jni_str!("SDK_INT"), jni::jni_sig!("I"))?.i()
    });
    if !sdk.is_some_and(|v| v >= 29) {
        return Err("Gallery export needs Android 10 or newer".into());
    }
    match jni_call(|env, activity| media_store_save(env, activity, name, bytes)) {
        Some(path) => Ok(path),
        None => Err("Couldn't save to the gallery".into()),
    }
}

/// Insert the row pending, stream the bytes, then flip `is_pending` off. A
/// failed write deletes the row, so a broken export never leaves a hidden
/// half-file in the gallery.
fn media_store_save(env: &mut jni::Env, activity: &jni::objects::JObject, name: &str, bytes: &[u8]) -> jni::errors::Result<String> {
    let resolver = env.call_method(activity, jni::jni_str!("getContentResolver"), jni::jni_sig!("()Landroid/content/ContentResolver;"), &[])?.l()?;
    let text = jni::objects::JString::from_str(env, "content://media/external/images/media")?;
    let collection = env.call_static_method(jni::jni_str!("android/net/Uri"), jni::jni_str!("parse"), jni::jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[(&text).into()])?.l()?;
    let values = media_values(env, &[("_display_name", name), ("mime_type", mime_for(name)), ("relative_path", "DCIM/Niveta")])?;
    put_integer(env, &values, "is_pending", 1)?;
    let row = env.call_method(&resolver, jni::jni_str!("insert"), jni::jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;"), &[(&collection).into(), (&values).into()])?.l()?;
    if row.is_null() {
        return Err(jni::errors::Error::NullPtr("MediaStore insert returned null"));
    }

    let stream = env.call_method(&resolver, jni::jni_str!("openOutputStream"), jni::jni_sig!("(Landroid/net/Uri;)Ljava/io/OutputStream;"), &[(&row).into()])?.l()?;
    if stream.is_null() {
        delete_row(env, &resolver, &row);
        return Err(jni::errors::Error::NullPtr("MediaStore openOutputStream returned null"));
    }
    let written = write_stream(env, &stream, bytes).and_then(|()| env.call_method(&stream, jni::jni_str!("close"), jni::jni_sig!("()V"), &[]).map(|_| ()));
    if let Err(e) = written {
        // The failed call may have left an exception pending: clear it before
        // the close and delete, then leave it clear for the next JNI user.
        env.exception_clear();
        let _ = env.call_method(&stream, jni::jni_str!("close"), jni::jni_sig!("()V"), &[]);
        env.exception_clear();
        delete_row(env, &resolver, &row);
        return Err(e);
    }

    let done = media_values(env, &[])?;
    put_integer(env, &done, "is_pending", 0)?;
    let null = jni::objects::JObject::null();
    let updated = env.call_method(
        &resolver,
        jni::jni_str!("update"),
        jni::jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I"),
        &[(&row).into(), (&done).into(), (&null).into(), (&null).into()],
    )?;
    if updated.i().unwrap_or(0) <= 0 {
        delete_row(env, &resolver, &row);
        return Err(jni::errors::Error::NullPtr("MediaStore did not publish the image"));
    }
    Ok(format!("DCIM/Niveta/{name}"))
}

/// A fresh `ContentValues` with the string fields already put.
fn media_values<'local>(env: &mut jni::Env<'local>, fields: &[(&str, &str)]) -> jni::errors::Result<jni::objects::JObject<'local>> {
    let values = env.new_object(jni::jni_str!("android/content/ContentValues"), jni::jni_sig!("()V"), &[])?;
    for &(key, value) in fields {
        let key = jni::objects::JString::from_str(env, key)?;
        let value = jni::objects::JString::from_str(env, value)?;
        env.call_method(&values, jni::jni_str!("put"), jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"), &[(&key).into(), (&value).into()])?;
    }
    Ok(values)
}

fn put_integer(env: &mut jni::Env, values: &jni::objects::JObject, key: &str, value: i32) -> jni::errors::Result<()> {
    let key = jni::objects::JString::from_str(env, key)?;
    let boxed = env.call_static_method(jni::jni_str!("java/lang/Integer"), jni::jni_str!("valueOf"), jni::jni_sig!("(I)Ljava/lang/Integer;"), &[value.into()])?.l()?;
    env.call_method(values, jni::jni_str!("put"), jni::jni_sig!("(Ljava/lang/String;Ljava/lang/Integer;)V"), &[(&key).into(), (&boxed).into()])?;
    Ok(())
}

/// Write in 1 MiB chunks: a single array for a 24 MP PNG would double the
/// export's memory in the Java heap for no reason. The local ref is dropped
/// per chunk, or a large export overflows the JNI local reference table.
fn write_stream(env: &mut jni::Env, stream: &jni::objects::JObject, bytes: &[u8]) -> jni::errors::Result<()> {
    for chunk in bytes.chunks(1 << 20) {
        let array = env.byte_array_from_slice(chunk)?;
        env.call_method(stream, jni::jni_str!("write"), jni::jni_sig!("([B)V"), &[(&array).into()])?;
        env.delete_local_ref(array);
    }
    Ok(())
}

/// Best effort: the caller already has the real error, and the pending row is
/// invisible either way.
fn delete_row(env: &mut jni::Env, resolver: &jni::objects::JObject, row: &jni::objects::JObject) {
    let null = jni::objects::JObject::null();
    let _ = env.call_method(resolver, jni::jni_str!("delete"), jni::jni_sig!("(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I"), &[row.into(), (&null).into(), (&null).into()]);
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("tif" | "tiff") => "image/tiff",
        Some("tga") => "image/x-tga",
        _ => "application/octet-stream",
    }
}

// SAFETY: `#[unsafe(no_mangle)]` only fixes the symbol name so the
// android-activity glue (C, in that crate) resolves `android_main`; it does
// not make the body unsafe. The body calls safe Rust only (logger init,
// bridge init and `crate::run`), takes no raw pointers, and returns `()` on
// `Destroy` via winit's normal shutdown path.
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("niveta-android: activity start");
    remember(&app);
    match app.internal_data_path() {
        Some(dir) => {
            log::info!("niveta-android: data dir {}", dir.display());
            crate::set_data_dir(dir);
        }
        None => log::warn!("niveta-android: no internal data path; preferences and recovery stay off"),
    }
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    if let Err(e) = crate::run(options) {
        log::error!("niveta-android: run_native failed: {e}");
    }
}
