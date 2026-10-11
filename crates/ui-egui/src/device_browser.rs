//! On-device file browser for File › Open (Android).
//!
//! DocumentsUI results can't reach a NativeActivity without Java in the build,
//! and this stack has none, so phones get a small egui browser over the shared
//! storage instead. It only opens when no `file_dialog` service exists (see
//! `open_interactive`): desktop and web keep their own dialogs.
//!
//! No JNI either: access is probed by listing a folder (denied without the
//! all-files grant), and the modal walks the user to Settings by hand.
//! Unreadable folders degrade to text, never a panic.

use std::path::{Path, PathBuf};

use crate::PhotocraftApp;

/// Shared-storage candidates, first readable wins as the start folder.
const ROOTS: &[&str] = &[
    "/sdcard/Download",
    "/sdcard/DCIM",
    "/sdcard/Pictures",
    "/sdcard/Documents",
    "/sdcard",
    "/storage/emulated/0/Download",
    "/storage/emulated/0",
];

/// One folder's worth of rows; the rest hides behind a note rather than a
/// stall on giant folders.
const MAX_ENTRIES: usize = 2000;

/// Transient browser state (never persisted: `#[serde(skip)]` on `UiState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Browser {
    dir: PathBuf,
    error: Option<String>,
    /// All-files verdict, probed on open and on demand (`None` = probe now).
    access: Option<bool>,
}

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: Option<u64>,
}

struct Listing {
    dirs: Vec<Entry>,
    files: Vec<Entry>,
    truncated: bool,
}

/// All-files access is held when a probe file can be created (and removed) in
/// Download. Listing alone is no proof: denied apps still enumerate folders on
/// some builds, so a listing-only probe shows an empty browser instead of the
/// gate below. Probed on open and on demand, never every frame. Off-device
/// there is no shared storage, so the probe fails and the gate shows (which
/// also keeps the browser host-testable through `access: Some(true)`).
fn has_access() -> bool {
    let probe = Path::new("/sdcard/Download").join(".photocraft-access-probe");
    if std::fs::write(&probe, b"1").is_err() {
        return false;
    }
    let _ = std::fs::remove_file(&probe);
    true
}

fn start_dir() -> PathBuf {
    ROOTS.iter().map(PathBuf::from).find(|d| std::fs::read_dir(d).is_ok()).unwrap_or_else(|| PathBuf::from("/sdcard/Download"))
}

/// File › Open without a system picker: show the browser modal.
pub fn open(app: &mut PhotocraftApp) {
    app.ui.device_browser = Some(Browser { dir: start_dir(), error: None, access: None });
}

/// What File › Open shows: exactly what `import` opens
/// (`photocraft_io::is_openable_file`), so the browser never hides a format
/// the importer handles.
fn is_supported(name: &str) -> bool {
    photocraft_io::is_openable_file(name)
}

fn list(dir: &Path) -> Result<Listing, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut truncated = false;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if e.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if !is_dir && !is_supported(&name) {
            continue;
        }
        if dirs.len() + files.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let size = if is_dir { None } else { e.metadata().map(|m| m.len()).ok() };
        let entry = Entry { name, path: e.path(), is_dir, size };
        if is_dir { dirs.push(entry); } else { files.push(entry); }
    }
    let by_name = |a: &Entry, b: &Entry| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    dirs.sort_by(by_name);
    files.sort_by(by_name);
    Ok(Listing { dirs, files, truncated })
}

fn size_text(size: Option<u64>) -> String {
    match size {
        None => "—".into(),
        Some(n) if n < 1024 => format!("{n} B"),
        Some(n) if n < 1024 * 1024 => format!("{} KB", n / 1024),
        Some(n) => format!("{} MB", n / (1024 * 1024)),
    }
}

/// Draw the modal when File › Open asked for it; a no-op otherwise.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut b) = app.ui.device_browser.take() else { return };
    if b.access.is_none() {
        b.access = Some(has_access());
    }
    if b.access != Some(true) {
        let gate = gate(ctx);
        if gate.recheck {
            b.access = None;
        }
        if !gate.closed {
            app.ui.device_browser = Some(b);
        }
        return;
    }
    let mut picked: Option<String> = None;
    let mut closed = false;
    egui::Modal::new(egui::Id::new("device-browser")).show(ctx, |ui| {
        ui.set_min_width(280.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Open")).size(14.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::icons::button(ui, "x", 32.0, false, tl!("Close")).clicked() {
                    closed = true;
                }
            });
        });
        ui.horizontal(|ui| {
            let up = b.dir.parent().filter(|p| !p.as_os_str().is_empty()).is_some();
            let mut go_up = false;
            ui.add_enabled_ui(up, |ui| {
                go_up = crate::widgets::secondary_button(ui, tl!("Up"), 64.0).clicked();
            });
            if go_up
                && let Some(parent) = b.dir.parent().map(Path::to_path_buf)
            {
                b.dir = parent;
                b.error = None;
            }
            ui.label(egui::RichText::new(b.dir.display().to_string()).small().weak());
        });
        if let Some(err) = b.error.as_deref() {
            ui.label(err);
        }
        match list(&b.dir) {
            Err(e) => b.error = Some(e),
            Ok(l) => {
                b.error = None;
                if l.dirs.is_empty() && l.files.is_empty() {
                    ui.label(egui::RichText::new(tl!("Empty folder")).weak());
                }
                egui::ScrollArea::vertical().id_salt("device-browser-list").auto_shrink([false, false]).max_height(320.0).show(ui, |ui| {
                    for e in l.dirs.iter().chain(l.files.iter()) {
                        let label = if e.is_dir { format!("{}/", e.name) } else { format!("{}  {}", e.name, size_text(e.size)) };
                        if ui.add_sized(egui::vec2(ui.available_width(), 36.0), egui::Button::new(label)).clicked() {
                            if e.is_dir {
                                b.dir = e.path.clone();
                            } else {
                                picked = Some(e.path.to_string_lossy().into_owned());
                            }
                        }
                    }
                });
                if l.truncated {
                    let n = MAX_ENTRIES.to_string();
                    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("First {n} entries shown"), &[("n", &n)])).small().weak());
                }
            }
        }
    });
    if let Some(path) = picked {
        app.open_paths(&[path]);
    } else if !closed {
        app.ui.device_browser = Some(b);
    }
}

/// No grant yet: the one manual step, plus a recheck for coming back from
/// Settings.
fn gate(ctx: &egui::Context) -> Gate {
    let mut out = Gate::default();
    egui::Modal::new(egui::Id::new("device-browser-gate")).show(ctx, |ui| {
        ui.set_min_width(280.0);
        ui.label(egui::RichText::new(tl!("Open")).size(14.0).strong());
        ui.label(tl!("PhotoCraft needs the \"All files access\" permission to open files from this device."));
        ui.label(tl!("Open Settings > Apps > PhotoCraft and turn on \"All files access\", then come back here."));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if crate::widgets::secondary_button(ui, tl!("Check again"), 128.0).clicked() {
                out.recheck = true;
            }
            if crate::widgets::secondary_button(ui, tl!("Close"), 96.0).clicked() {
                out.closed = true;
            }
        });
    });
    out
}

#[derive(Default)]
struct Gate {
    closed: bool,
    recheck: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        dir: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("photocraft-browser-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("sub")).unwrap();
            std::fs::create_dir_all(dir.join(".hidden")).unwrap();
            std::fs::write(dir.join("a.png"), b"x").unwrap();
            std::fs::write(dir.join("b.PSD"), b"x").unwrap();
            std::fs::write(dir.join("c.txt"), b"x").unwrap();
            std::fs::write(dir.join(".dot.png"), b"x").unwrap();
            Self { dir }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn listing_shows_supported_files_and_dirs_only() {
        let t = TempDir::new();
        let l = list(&t.dir).unwrap();
        assert_eq!(l.dirs.iter().map(|e| e.name.clone()).collect::<Vec<_>>(), ["sub"]);
        assert_eq!(l.files.iter().map(|e| e.name.clone()).collect::<Vec<_>>(), ["a.png", "b.PSD"]);
        assert!(!l.truncated);
    }

    #[test]
    fn unreadable_dir_is_an_error_not_a_panic() {
        assert!(list(Path::new("/no/such/dir-anywhere")).is_err());
        assert!(list(Path::new("")).is_err());
    }

    #[test]
    fn ext_match_is_case_insensitive() {
        assert!(is_supported("a.PNG"));
        assert!(is_supported("a.psd"));
        assert!(is_supported("a.tif"));
        assert!(is_supported("a.dng"));
        assert!(is_supported("a.svg"));
        assert!(!is_supported("a.txt"));
        assert!(!is_supported("noext"));
    }

    /// No shared storage here: the write probe fails, so the gate shows.
    #[test]
    fn access_probe_fails_without_shared_storage() {
        assert!(!has_access());
    }

    #[test]
    fn browser_body_renders_a_folder() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let t = TempDir::new();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.device_browser = Some(Browser { dir: t.dir.clone(), error: None, access: Some(true) });
        let mut h = Harness::builder().with_size(egui::vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                show(app, ui.ctx());
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        assert!(h.query_all_by_label_contains("a.png").next().is_some(), "file row renders");
        assert!(h.query_all_by_label_contains("sub/").next().is_some(), "dir row renders");
        assert!(h.state().ui.device_browser.is_some(), "modal stays open");
    }
}
