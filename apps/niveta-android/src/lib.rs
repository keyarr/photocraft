//! Niveta for Android: eframe shell, touch-first, CPU canvas first.
//!
//! Same `NivetaApp` + engine as desktop/web, only the `Services` differ:
//! files arrive as bytes through the inbox (Storage Access Framework hands the
//! Java side a descriptor, the JNI bridge pushes the bytes here), exactly like
//! the web shell's async inbox (`apps/niveta-web/src/web.rs`). No clipboard,
//! no control server, no `open`/reveal yet: every one of those is `None`, which
//! the app treats as "cancelled"/unavailable, never a crash
//! (`ui-egui/src/file_dialog.rs:92`). No system picker on-device (its results
//! can't reach a NativeActivity without Java in the build): File › Open gets
//! the on-device browser (`ui-egui/src/device_browser.rs`) once all-files
//! access is granted; on a host run (`cargo run -p niveta-android`) rfd
//! dialogs are wired so the dev loop can open and save.
//!
//! GPU canvas is opt-in (Preferences › Performance › Rendering Mode: GPU, or
//! `NIVETA_ANDROID_GPU=1` on a host run); default is the CPU compositor.
//! The JNI half (ClipboardManager) is still open:
//! its seam is `shared_inbox`/`push_bytes` + `data_dir` below, both safe Rust
//! with no new unsafe.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;
use std::sync::OnceLock;

use niveta_doc::Document;
use niveta_engine::Session;
use niveta_ui_egui::{Inbox, NivetaApp, Services};

/// Android entry point (`android_main`); compiled in on-device only.
#[cfg(target_os = "android")]
pub mod android;

// Mobile open set lives with the importer (`niveta_io::open_extensions`):
// PNG/JPEG photos, TIFF, PSD/PSB, `.pcraft`, SVG, camera raws and the rest
// `import` opens. The full list stays in one place so the browser
// (`ui-egui/src/device_browser.rs`) and the host dialog below never drift.

/// Private files dir, remembered from `android_main` (`internal_data_path`;
/// `NIVETA_ANDROID_DATA_DIR` on a host run). Same pattern as
/// [`SHARED_INBOX`]: `run` never returns a handle for it, so `android_main`
/// stores it here before the app starts.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Remember where private files live. Called once from `android_main` before
/// the app starts; later calls (activity recreation) keep the first value.
pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

/// Where private files live.
/// `None` means prefs don't persist and recovery is off (same as web).
pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR
        .get()
        .cloned()
        .or_else(|| std::env::var_os("NIVETA_ANDROID_DATA_DIR").map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty())
}

/// Queue bytes from SAF (or a host test) for the app to open next frame.
/// The JNI `onActivityResult` calls this; no Java file paths cross into Rust.
pub fn push_inbox(inbox: &Inbox, name: String, bytes: Vec<u8>) {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
}

/// The inbox the running app drains. `run` never returns its handle, so the JNI
/// bridge can't take it as a parameter: it pushes through here instead
/// (`push_bytes`), and every `Services` built here shares it.
static SHARED_INBOX: OnceLock<Inbox> = OnceLock::new();

/// The running app's inbox (created on first use).
pub fn shared_inbox() -> Inbox {
    SHARED_INBOX.get_or_init(Inbox::default).clone()
}

/// Queue SAF bytes for the running app; `false` when over [`MAX_INBOX_BYTES`]
/// (caller drops and warns). Room-checked, so JNI never grows memory unbounded.
pub fn push_bytes(name: String, bytes: Vec<u8>) -> bool {
    let inbox = shared_inbox();
    if !inbox_has_room(&inbox, bytes.len()) {
        return false;
    }
    push_inbox(&inbox, name, bytes);
    true
}

/// Cap an input-sized queue before it becomes a memory problem on a phone.
const MAX_INBOX_BYTES: usize = 256 * 1024 * 1024;

/// `false` when the next push would exceed the cap (caller drops and warns).
pub fn inbox_has_room(inbox: &Inbox, incoming: usize) -> bool {
    let held = inbox.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(_, b)| b.len()).sum::<usize>();
    held.checked_add(incoming).is_some_and(|n| n <= MAX_INBOX_BYTES)
}

/// File dialogs for a host run (`cargo run -p niveta-android`); `None`
/// on-device, where SAF intents land through `push_bytes` instead.
#[cfg(not(target_os = "android"))]
fn host_file_dialog() -> Option<niveta_ui_egui::FileDialogFn> {
    use niveta_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest};
    Some(Box::new(|request: FileDialogRequest, parent: Option<&eframe::Frame>, reply: FileDialogReply| {
        let mut dialog = rfd::AsyncFileDialog::new();
        if let Some(parent) = parent {
            dialog = dialog.set_parent(parent);
        }
        let answer = match request {
            FileDialogRequest::Open { multiple, initial_dir } => {
                if let Some(dir) = initial_dir {
                    dialog = dialog.set_directory(dir);
                }
                let dialog = dialog.add_filter("All Formats", &niveta_io::open_extensions());
                if multiple {
                    let picked = dialog.pick_files();
                    Box::pin(async move {
                        picked.await.map(|files| FileDialogAnswer::Paths(files.iter().map(|f| f.path().to_string_lossy().into_owned()).collect()))
                    }) as std::pin::Pin<Box<dyn std::future::Future<Output = Option<FileDialogAnswer>> + Send>>
                } else {
                    let picked = dialog.pick_file();
                    Box::pin(async move { picked.await.map(|file| FileDialogAnswer::Paths(vec![file.path().to_string_lossy().into_owned()])) })
                        as std::pin::Pin<Box<dyn std::future::Future<Output = Option<FileDialogAnswer>> + Send>>
                }
            }
            FileDialogRequest::Save { suggested } => {
                if let Some(name) = std::path::Path::new(&suggested).file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                if let Some(dir) = std::path::Path::new(&suggested).parent().filter(|p| !p.as_os_str().is_empty()) {
                    dialog = dialog.set_directory(dir);
                }
                let picked = dialog.save_file();
                Box::pin(async move { picked.await.map(|file| FileDialogAnswer::SaveTo(file.path().to_string_lossy().into_owned())) })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = Option<FileDialogAnswer>> + Send>>
            }
        };
        // Without the thread the reply is dropped unanswered, which the app takes as Cancel.
        if let Err(e) = std::thread::Builder::new().name("file dialog".into()).spawn(move || reply.send(block_on(answer))) {
            log::error!("couldn't wait for the file dialog: {e}");
        }
    }))
}

/// No dialogs on-device yet (SAF comes through `push_bytes`).
#[cfg(target_os = "android")]
fn host_file_dialog() -> Option<niveta_ui_egui::FileDialogFn> {
    None
}

/// Runs `future` to completion on this thread, sleeping until it is woken.
#[cfg(not(target_os = "android"))]
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::sync::Arc;
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park();
    }
}

/// Platform services for Android. Every closure returns `Err`, never panics,
/// on any input (bad bytes, unknown extension, missing data dir).
pub fn android_services() -> (Services, Inbox) {
    android_services_in(data_dir())
}

/// Same, with the data dir injected (tests stay off the process environment).
fn android_services_in(dir: Option<PathBuf>) -> (Services, Inbox) {
    let inbox = shared_inbox();
    let prefs_file = dir.clone().map(|d| d.join("preferences.json"));
    // No data dir means no persistence and no recovery (same as web): leave the
    // services unset so a change stays in memory without a save warning.
    let load_prefs = prefs_file.clone().map(|path| {
        let f: niveta_ui_egui::LoadTextFn = Box::new(move || std::fs::read_to_string(&path).ok());
        f
    });
    let save_prefs = prefs_file.map(|path| {
        let f: niveta_ui_egui::SaveTextFn =
            Box::new(move |text: &str| niveta_format::atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string()));
        f
    });
    let mut services = Services {
        import: Some(Box::new(|name: &str, bytes: &[u8]| niveta_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &niveta_ui_egui::ExportSettings| {
            let mut opts = niveta_io::ExportOptions::default();
            if let Some(q) = settings.jpeg_quality {
                opts.encode.jpeg_quality = q;
            }
            opts.encode.webp_lossless = settings.webp_lossless;
            if let Some(q) = settings.webp_quality {
                opts.encode.webp_quality = q;
            }
            opts.tiff_layers = settings.tiff_layers;
            opts.xmp = if settings.xmp_all { niveta_io::XmpEmbed::All } else { niveta_io::XmpEmbed::None };
            niveta_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        // No system picker on-device (File › Open gets the on-device browser);
        // a host run gets rfd dialogs so the dev loop can open and save.
        file_dialog: host_file_dialog(),
        write: Some(Box::new(|path: &str, bytes: &[u8]| niveta_format::atomic_write(std::path::Path::new(path), bytes).map_err(|e| e.to_string()))),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = niveta_codecs::Image::from_u8(w, h, niveta_codecs::ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            niveta_codecs::encode(&img, niveta_codecs::Format::Png, &niveta_codecs::EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        inbox: Some(inbox.clone()),
        // ClipboardManager via JNI later; None pastes from the internal clipboard.
        clipboard_set_image: None,
        clipboard_get_image: None,
        load_prefs,
        save_prefs,
        // No control TCP server, no screen picker, no open_url on a phone.
        ..Default::default()
    };
    // Storage permission hooks for the on-device browser's gate: the runtime
    // dialog plus the Settings screen, through the JNI bridge (`android.rs`).
    // Host builds keep `None` and the probe fallback (their picker exists).
    #[cfg(target_os = "android")]
    {
        services.storage_has_access = Some(Box::new(crate::android::storage_has_access));
        services.storage_request = Some(Box::new(crate::android::storage_request));
        services.storage_settings = Some(Box::new(crate::android::storage_settings));
        // Exports go through MediaStore: no all-files grant, and the gallery
        // indexes them at once. `export_base_dir` is only the sheet's label.
        services.save_to_gallery = Some(Box::new(crate::android::save_to_gallery));
        services.export_base_dir = Some("DCIM/Niveta".into());
    }
    if let Some(d) = dir {
        install_recovery(&mut services, &d);
    }
    (services, inbox)
}

/// Crash-recovery autosave wired to the private recovery dir, plus the
/// synchronous exit snapshot and the picker-free export dir. Without a data
/// dir everything stays off (same as web): changes live in memory only.
fn install_recovery(services: &mut Services, dir: &std::path::Path) {
    use std::sync::{Arc, Mutex};
    let store = Arc::new(Mutex::new(niveta_format::RecoveryStore::new(dir.join("recovery"))));
    {
        let store = store.clone();
        services.autosave = Some(Box::new(move |doc, rev, path| {
            store.lock().unwrap_or_else(|e| e.into_inner()).autosave_checked(doc, rev, path.map(str::to_string))
        }));
    }
    {
        let store = store.clone();
        services.autosave_results = Some(Box::new(move || store.lock().unwrap_or_else(|e| e.into_inner()).take_completed()));
    }
    {
        let store = store.clone();
        services.discard_autosave = Some(Box::new(move |id| {
            let _ = store.lock().unwrap_or_else(|e| e.into_inner()).discard(id);
        }));
    }
    {
        let store = store.clone();
        services.recover = Some(Box::new(move || {
            store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .recover()
                .into_iter()
                .map(|(e, doc)| niveta_ui_egui::Recovered { key: e.info.key, path: e.info.original_path, doc })
                .collect()
        }));
    }
    {
        let store = store.clone();
        services.adopt_autosave = Some(Box::new(move |id, key| {
            store.lock().unwrap_or_else(|e| e.into_inner()).adopt(id, key);
        }));
    }
    {
        let store = store.clone();
        services.autosave_blocking = Some(Box::new(move |doc, rev, path| {
            store.lock().unwrap_or_else(|e| e.into_inner()).save_blocking(doc, rev, path.map(str::to_string))
        }));
    }
    // The shell may have already named a MediaStore folder (`save_to_gallery`);
    // only a build without that hook probes a filesystem path.
    if services.export_base_dir.is_none() {
        services.export_base_dir = Some(export_base_dir(dir));
    }
}

/// Host fallback for picker-free exports: shared DCIM/Niveta when it takes a
/// file (needs the storage grant), else the private exports dir (always
/// writable). On-device the gallery hook owns the location. The path is shown
/// on the sheet, so nothing is a surprise.
fn export_base_dir(dir: &std::path::Path) -> String {
    for root in ["/sdcard/DCIM/Niveta", "/storage/emulated/0/DCIM/Niveta"] {
        let probe = std::path::Path::new(root).join(".niveta-write-probe");
        if std::fs::create_dir_all(root).is_ok() && std::fs::write(&probe, b"1").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return root.to_string();
        }
    }
    let private = dir.join("exports");
    let _ = std::fs::create_dir_all(&private);
    private.to_string_lossy().into_owned()
}

/// Build the app with CPU canvas default; GPU only when asked for.
pub fn new_app() -> (NivetaApp, Inbox) {
    let (services, inbox) = android_services();
    let mut app = NivetaApp::new(Session::new(), services);
    app.background_jobs = std::env::var_os("NIVETA_INLINE_JOBS").is_none();
    (app, inbox)
}

/// Whether the Android shell should use the wgpu canvas: `auto` tries the GPU first (the canvas
/// falls back to the CPU renderer on init failure or a device fault), `gpu` forces it, `cpu` opts
/// out. The host dev-loop override wins.
fn android_wants_gpu(app: &NivetaApp) -> bool {
    std::env::var_os("NIVETA_ANDROID_GPU").is_some()
        || app.session.prefs().performance.effective_rendering_mode() != niveta_engine::prefs::RenderingMode::Cpu
}

fn maybe_enable_gpu(app: &mut NivetaApp, cc: &eframe::CreationContext) {
    let Some(rs) = cc.wgpu_render_state.clone() else { return };
    // The window always has a wgpu device: record it so Help › System Info shows
    // the real adapter even while the image compositor stays on the CPU.
    app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
    if !android_wants_gpu(app) {
        app.perf.gpu_info.canvas = "cpu".into();
        return;
    }
    app.set_wgpu(rs);
}

/// Run on host (`main.rs`) or from `android_main` (`src/android.rs`).
pub fn run(options: eframe::NativeOptions) -> eframe::Result {
    log::info!("niveta-android: open set: {}", niveta_io::open_extensions().join(","));
    eframe::run_native(
        "Niveta",
        options,
        Box::new(|cc| {
            let (mut app, _inbox) = new_app();
            NivetaApp::setup_context(&cc.egui_ctx, app.ui.theme);
            maybe_enable_gpu(&mut app, cc);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use niveta_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    use std::sync::{Arc, Mutex};

    fn red_png(w: u32, h: u32) -> Vec<u8> {
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
        niveta_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
    }

    #[test]
    fn garbage_bytes_are_an_error_not_a_panic() {
        let (services, _) = android_services();
        let import = services.import.as_ref().unwrap();
        assert!(import("photo.png", b"definitely not an image").is_err());
        assert!(import("photo.png", &[]).is_err());
        let export = services.export.as_ref().unwrap();
        let doc = import("red.png", &red_png(2, 2)).unwrap().0;
        assert!(export(&doc, "out.zzz-no-such-format", &Default::default()).is_err());
    }

    #[test]
    fn inbox_carries_a_file_within_cap() {
        let inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
        let bytes = red_png(2, 2);
        assert!(inbox_has_room(&inbox, bytes.len()));
        push_inbox(&inbox, "red.png".into(), bytes.clone());
        let taken = std::mem::take(&mut *inbox.lock().unwrap_or_else(|e| e.into_inner()));
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].0, "red.png");
        assert_eq!(taken[0].1, bytes);
        assert!(!inbox_has_room(&inbox, MAX_INBOX_BYTES + 1));
    }

    #[test]
    fn services_share_one_inbox_for_the_jni_bridge() {
        // `run` never returns its inbox handle, so JNI pushes through the shared
        // one; every Services built here must drain that same queue.
        let (a, ia) = android_services();
        let (b, ib) = android_services();
        assert!(Arc::ptr_eq(&ia, &ib));
        assert!(Arc::ptr_eq(a.inbox.as_ref().unwrap(), b.inbox.as_ref().unwrap()));
        assert!(push_bytes("red.png".into(), red_png(2, 2)));
        assert!(!inbox_has_room(&shared_inbox(), MAX_INBOX_BYTES + 1));
        let taken = std::mem::take(&mut *shared_inbox().lock().unwrap_or_else(|e| e.into_inner()));
        assert!(taken.iter().any(|(n, _)| n == "red.png"));
    }

    #[test]
    fn prefs_without_a_data_dir_stay_off() {
        let (services, _) = android_services_in(None);
        assert!(services.load_prefs.is_none());
        assert!(services.save_prefs.is_none());
        assert!(services.autosave.is_none());
        assert!(services.autosave_results.is_none());
        assert!(services.discard_autosave.is_none());
        assert!(services.recover.is_none());
        assert!(services.adopt_autosave.is_none());
        assert!(services.autosave_blocking.is_none());
        assert!(services.export_base_dir.is_none());
    }

    #[test]
    fn recovery_roundtrips_through_a_data_dir() {
        let dir = std::env::temp_dir().join(format!("niveta-android-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (mut services, _) = android_services_in(Some(dir.clone()));
        for (name, s) in
            [("autosave", services.autosave.is_some()), ("results", services.autosave_results.is_some()), ("discard", services.discard_autosave.is_some()), ("recover", services.recover.is_some()), ("adopt", services.adopt_autosave.is_some()), ("blocking", services.autosave_blocking.is_some())]
        {
            assert!(s, "{name} is wired with a data dir");
        }
        assert!(services.export_base_dir.is_some());
        let doc = std::sync::Arc::new(services.import.as_ref().unwrap()("red.png", &red_png(4, 4)).unwrap().0);
        let id = doc.id.0;
        services.autosave_blocking.as_mut().unwrap()(&doc, 3, None).unwrap();
        let found = services.recover.as_mut().unwrap()();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].doc, *doc);
        // Adopting the entry keeps snapshots on it: still one entry after more saves.
        services.adopt_autosave.as_mut().unwrap()(id, &found[0].key);
        services.autosave_blocking.as_mut().unwrap()(&doc, 4, None).unwrap();
        assert_eq!(services.recover.as_mut().unwrap()().len(), 1);
        services.discard_autosave.as_mut().unwrap()(id);
        assert!(services.recover.as_mut().unwrap()().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefs_roundtrip_with_a_data_dir() {
        let dir = std::env::temp_dir().join(format!("niveta-android-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (mut services, _) = android_services_in(Some(dir.clone()));
        services.save_prefs.take().unwrap()("{\"theme\":\"pro\"}").unwrap();
        let back: Option<String> = services.load_prefs.as_mut().and_then(|load| load());
        assert_eq!(back.as_deref(), Some("{\"theme\":\"pro\"}"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Host builds have no activity to ask: the hooks stay `None` and the
    /// browser keeps its probe fallback (which the host never reaches: its
    /// picker service exists).
    #[cfg(not(target_os = "android"))]
    #[test]
    fn host_build_has_no_storage_hooks() {
        let (services, _) = android_services();
        assert!(services.storage_has_access.is_none());
        assert!(services.storage_request.is_none());
        assert!(services.storage_settings.is_none());
    }

    #[test]
    fn auto_tries_gpu_and_cpu_is_opt_out() {
        if std::env::var_os("NIVETA_ANDROID_GPU").is_some() {
            return; // host override on: prefs can't keep the canvas off.
        }
        let (mut app, _) = new_app();
        app.session.edit_prefs(|p| {
            p.performance.rendering_mode = None;
            p.performance.use_gpu = true;
        });
        assert!(android_wants_gpu(&app), "auto tries the GPU first");
        app.session.edit_prefs(|p| p.performance.rendering_mode = Some(niveta_engine::prefs::RenderingMode::Gpu));
        assert!(android_wants_gpu(&app));
        app.session.edit_prefs(|p| p.performance.rendering_mode = Some(niveta_engine::prefs::RenderingMode::Cpu));
        assert!(!android_wants_gpu(&app));
    }
}
