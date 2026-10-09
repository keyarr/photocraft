//! PhotoCraft for Android: eframe shell, touch-first, CPU canvas first.
//!
//! Same `PhotocraftApp` + engine as desktop/web, only the `Services` differ:
//! files arrive as bytes through the inbox (Storage Access Framework hands the
//! Java side a descriptor, the JNI bridge pushes the bytes here), exactly like
//! the web shell's async inbox (`apps/photocraft-web/src/web.rs`). No clipboard,
//! no control server, no `open`/reveal yet: every one of those is `None`, which
//! the app treats as "cancelled"/unavailable, never a crash
//! (`ui-egui/src/file_dialog.rs:92`). No file dialog on-device until the JNI half
//! lands either; on a host run (`cargo run -p photocraft-android`) rfd dialogs
//! are wired so the dev loop can open and save.
//!
//! GPU is opt-in (`PHOTOCRAFT_ANDROID_GPU=1`); default is the CPU compositor.
//! The JNI half (SAF intents, `getFilesDir`, ClipboardManager) is still open:
//! its seam is `shared_inbox`/`push_bytes` + `data_dir` below, both safe Rust
//! with no new unsafe.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;
use std::sync::OnceLock;

use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{Inbox, PhotocraftApp, Services};

/// Android entry point (`android_main`); compiled in on-device only.
#[cfg(target_os = "android")]
pub mod android;

/// Mobile open set: PNG/JPEG for photos, PSD/PSB to round-trip desktop work,
/// `.pcraft` native. The full desktop list stays in `apps/photocraft`; SAF
/// mime filters derive from this when the JNI half lands.
const OPEN_EXTS: &[&str] = &["pcraft", "psd", "psb", "png", "jpg", "jpeg", "webp"];

/// Where private files live: set by the Java side from `Context.getFilesDir`
/// before `android_main` runs (`PHOTOCRAFT_ANDROID_DATA_DIR` on a host run).
/// `None` means prefs don't persist and recovery is off (same as web).
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("PHOTOCRAFT_ANDROID_DATA_DIR").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
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

/// File dialogs for a host run (`cargo run -p photocraft-android`); `None`
/// on-device, where SAF intents land through `push_bytes` instead.
#[cfg(not(target_os = "android"))]
fn host_file_dialog() -> Option<photocraft_ui_egui::FileDialogFn> {
    use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest};
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
                let dialog = dialog.add_filter("All Formats", OPEN_EXTS);
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
fn host_file_dialog() -> Option<photocraft_ui_egui::FileDialogFn> {
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
    let prefs_file = dir.map(|d| d.join("preferences.json"));
    let load_path = prefs_file.clone();
    let save_path = prefs_file;
    let services = Services {
        import: Some(Box::new(|name: &str, bytes: &[u8]| photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let mut opts = photocraft_io::ExportOptions::default();
            if let Some(q) = settings.jpeg_quality {
                opts.encode.jpeg_quality = q;
            }
            opts.encode.webp_lossless = settings.webp_lossless;
            if let Some(q) = settings.webp_quality {
                opts.encode.webp_quality = q;
            }
            opts.tiff_layers = settings.tiff_layers;
            opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
            photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        // No SAF dialog on-device until the JNI half lands (`push_bytes` is the
        // seam); a host run gets rfd dialogs so the dev loop can open and save.
        file_dialog: host_file_dialog(),
        write: Some(Box::new(|path: &str, bytes: &[u8]| photocraft_format::atomic_write(std::path::Path::new(path), bytes).map_err(|e| e.to_string()))),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        inbox: Some(inbox.clone()),
        // ClipboardManager via JNI later; None pastes from the internal clipboard.
        clipboard_set_image: None,
        clipboard_get_image: None,
        load_prefs: Some(Box::new(move || load_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()))),
        save_prefs: Some(Box::new(move |text: &str| {
            let path = save_path.as_ref().ok_or("no app data directory")?;
            photocraft_format::atomic_write(path, text.as_bytes()).map_err(|e| e.to_string())
        })),
        // No control TCP server, no screen picker, no open_url on a phone.
        ..Default::default()
    };
    (services, inbox)
}

/// Build the app with CPU canvas default; GPU only when asked for.
pub fn new_app() -> (PhotocraftApp, Inbox) {
    let (services, inbox) = android_services();
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.background_jobs = std::env::var_os("PHOTOCRAFT_INLINE_JOBS").is_none();
    (app, inbox)
}

fn maybe_enable_gpu(app: &mut PhotocraftApp, cc: &eframe::CreationContext) {
    if std::env::var_os("PHOTOCRAFT_ANDROID_GPU").is_none() {
        return;
    }
    if let Some(rs) = cc.wgpu_render_state.clone() {
        app.set_wgpu(rs);
    }
}

/// Run on host (`main.rs`) or from `android_main` (`src/android.rs`).
pub fn run(options: eframe::NativeOptions) -> eframe::Result {
    log::info!("photocraft-android: open set: {}", OPEN_EXTS.join(","));
    eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
            let (mut app, _inbox) = new_app();
            maybe_enable_gpu(&mut app, cc);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    use std::sync::{Arc, Mutex};

    fn red_png(w: u32, h: u32) -> Vec<u8> {
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
        photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
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
    fn prefs_without_a_data_dir_fail_gracefully() {
        let (mut services, _) = android_services_in(None);
        let back: Option<String> = services.load_prefs.as_mut().and_then(|load| load());
        assert!(back.is_none());
        let mut save = services.save_prefs.take().unwrap();
        assert!(save("{}").is_err());
    }

    #[test]
    fn prefs_roundtrip_with_a_data_dir() {
        let dir = std::env::temp_dir().join(format!("photocraft-android-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (mut services, _) = android_services_in(Some(dir.clone()));
        services.save_prefs.take().unwrap()("{\"theme\":\"pro\"}").unwrap();
        let back: Option<String> = services.load_prefs.as_mut().and_then(|load| load());
        assert_eq!(back.as_deref(), Some("{\"theme\":\"pro\"}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
