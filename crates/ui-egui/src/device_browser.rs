//! On-device file browser for File › Open (Android).
//!
//! DocumentsUI results can't reach a NativeActivity without Java in the build,
//! and this stack has none, so phones get a small egui browser over the shared
//! storage instead. It only opens when no `file_dialog` service exists (see
//! `open_interactive`): desktop and web keep their own dialogs. Folders and
//! photos share one grid: folders get an icon, photos a thumbnail.
//!
//! The access verdict comes from `Services::storage_has_access` when the shell
//! provides it (Android's JNI bridge: media/all-files grants); otherwise a
//! write probe stands in (host tests). The gate's buttons fire
//! `Services::storage_request` (runtime dialog) and
//! `Services::storage_settings` (the all-files screen, which
//! `MANAGE_EXTERNAL_STORAGE` needs by OS design: it never shows a dialog).
//! Unreadable folders degrade to text, never a panic.

use std::path::{Path, PathBuf};

use crate::PhotocraftApp;

/// Shared-storage candidates, first readable wins as the start folder.
const ROOTS: &[&str] =
    &["/sdcard/Download", "/sdcard/DCIM", "/sdcard/Pictures", "/sdcard/Documents", "/sdcard", "/storage/emulated/0/Download", "/storage/emulated/0"];

/// One folder's worth of rows; the rest hides behind a note rather than a
/// stall on giant folders.
const MAX_ENTRIES: usize = 2000;

/// Photo thumbnail: longest side in px. Twice the 88 pt cell, so it stays
/// sharp on 2x phone displays (the GPU downscales on draw).
const THUMB_SIDE: u32 = 192;
/// Cap on cached thumbnails: at 192 px each is ~150 KB, so this bounds the
/// grid's GPU use (~20 MB). Older folders re-decode on revisit.
const MAX_CACHED_THUMBS: usize = 150;
/// Files bigger than this get a placeholder icon instead of a thumbnail: the
/// file still opens on tap, it just isn't decoded for the grid.
const MAX_THUMB_BYTES: u64 = 32 * 1024 * 1024;
/// How many missing thumbnails one frame decodes: a folder of RAWs must not
/// stall the scroll, the rest resolve over the next frames.
const THUMB_BUDGET: usize = 4;

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
    /// Cache fingerprint (size + mtime): a thumbnail is reused only while both match.
    fingerprint: u64,
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
        let fingerprint = e.metadata().ok().map(|m| fingerprint_of(&m, size)).unwrap_or(0);
        let entry = Entry { name, path: e.path(), is_dir, size, fingerprint };
        if is_dir {
            dirs.push(entry);
        } else {
            files.push(entry);
        }
    }
    let by_name = |a: &Entry, b: &Entry| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    dirs.sort_by(by_name);
    files.sort_by(by_name);
    Ok(Listing { dirs, files, truncated })
}

fn fingerprint_of(meta: &std::fs::Metadata, size: Option<u64>) -> u64 {
    let (secs, nanos) =
        meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| (d.as_secs(), d.subsec_nanos())).unwrap_or((0, 0));
    secs.wrapping_mul(31).wrapping_add(u64::from(nanos)).wrapping_mul(31).wrapping_add(size.unwrap_or(0))
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
    // With shell hooks the verdict is live: recheck while the gate is up so
    // granting in the dialog or Settings opens the browser with no extra tap.
    // The write probe stays one-shot (no per-frame file writes).
    if b.access.is_none() || (b.access != Some(true) && app.services.storage_has_access.is_some()) {
        b.access = Some(match app.services.storage_has_access.as_mut() {
            Some(has) => has(),
            None => has_access(),
        });
    }
    if b.access != Some(true) {
        let gate = gate(ctx, &mut app.services);
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
    let mut budget = THUMB_BUDGET;
    let mut pending = false;
    egui::Modal::new(egui::Id::new("device-browser")).show(ctx, |ui| {
        ui.set_min_width(320.0);
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
            if go_up && let Some(parent) = b.dir.parent().map(Path::to_path_buf) {
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
                egui::ScrollArea::vertical().id_salt("device-browser-list").auto_shrink([false, false]).max_height(400.0).show(ui, |ui| {
                    let cols = ((ui.available_width() / 100.0).floor() as usize).clamp(2, 4);
                    egui::Grid::new("device-browser-grid").spacing([4.0, 8.0]).show(ui, |ui| {
                        for (i, e) in l.dirs.iter().chain(l.files.iter()).enumerate() {
                            let tex = if e.is_dir { None } else { thumb(ui, e, &mut budget, &mut pending) };
                            if cell(ui, e, tex).clicked() {
                                if e.is_dir {
                                    b.dir = e.path.clone();
                                } else {
                                    picked = Some(e.path.to_string_lossy().into_owned());
                                }
                            }
                            if i % cols == cols - 1 {
                                ui.end_row();
                            }
                        }
                    });
                });
                if pending {
                    ui.ctx().request_repaint();
                }
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

/// Which files get a decoded thumbnail: flat photos through the codecs,
/// `.pcraft` through its embedded preview. PSD/PSB, raws and SVGs keep the
/// placeholder icon: those need the full importer, not a quick grid thumb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThumbKind {
    Pcraft,
    Flat,
    None,
}

fn thumb_kind(name: &str) -> ThumbKind {
    if name.rsplit(['/', '\\']).next().and_then(|n| n.rsplit_once('.')).is_some_and(|(base, ext)| !base.is_empty() && ext.eq_ignore_ascii_case(photocraft_format::EXTENSION)) {
        ThumbKind::Pcraft
    } else if photocraft_codecs::from_extension(name).is_some_and(|f| f.caps().read) {
        ThumbKind::Flat
    } else {
        ThumbKind::None
    }
}

fn thumb_key(path: &Path, fingerprint: u64) -> egui::Id {
    egui::Id::new(("device-browser-thumb", path.to_string_lossy().into_owned(), fingerprint))
}

fn thumb_failed_id() -> egui::Id {
    egui::Id::new("device-browser-thumb-failed")
}

fn thumb_order_id() -> egui::Id {
    egui::Id::new("device-browser-thumb-order")
}

/// Decode the grid thumbnail for one file. `None` degrades to the placeholder
/// icon, never an error: unreadable, oversized and unsupported files still
/// open on tap.
fn decode_thumb(path: &Path, name: &str, size: Option<u64>) -> Option<egui::ColorImage> {
    if size.is_some_and(|n| n == 0 || n > MAX_THUMB_BYTES) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() as u64 > MAX_THUMB_BYTES {
        return None;
    }
    match thumb_kind(name) {
        ThumbKind::None => None,
        ThumbKind::Pcraft => {
            let png = photocraft_format::read_thumbnail(&bytes).ok()??;
            let img = photocraft_codecs::decode(&png).ok()?;
            Some(shrink(&img.to_rgba8(), img.width(), img.height()))
        }
        ThumbKind::Flat => {
            let limits =
                photocraft_codecs::Limits { max_width: 12_000, max_height: 12_000, max_pixels: 64 * 1024 * 1024, max_alloc: 256 * 1024 * 1024 };
            let img = photocraft_codecs::decode_with(&bytes, &photocraft_codecs::DecodeOptions { limits, keep_orientation: false }).ok()?;
            Some(shrink(&img.to_rgba8(), img.width(), img.height()))
        }
    }
}

/// Nearest-neighbour shrink of RGBA8 bytes to `THUMB_SIDE` on the long side.
fn shrink(px: &[u8], w: u32, h: u32) -> egui::ColorImage {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let side = THUMB_SIDE as usize;
    let k = w.max(h).div_ceil(side).max(1);
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = Vec::with_capacity(ow * oh * 4);
    for y in 0..oh {
        for x in 0..ow {
            let o = (y * k * w + x * k) * 4;
            match px.get(o..o + 4) {
                Some(p) => out.extend_from_slice(p),
                None => out.extend_from_slice(&[0, 0, 0, 0]),
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([ow, oh], &out)
}

/// The cached thumbnail, decoding at most `*budget` missing ones this frame.
/// `pending` stays true while a file still waits for its turn, so the grid
/// repaints until every thumb resolved. Anything without a thumbnail (folders,
/// PSDs, failures) is `None`: the cell draws its icon instead.
fn thumb(ui: &egui::Ui, e: &Entry, budget: &mut usize, pending: &mut bool) -> Option<egui::TextureHandle> {
    if e.is_dir || thumb_kind(&e.name) == ThumbKind::None || e.size.is_some_and(|n| n == 0 || n > MAX_THUMB_BYTES) {
        return None;
    }
    let key = thumb_key(&e.path, e.fingerprint);
    if let Some(tex) = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(tex);
    }
    if ui.ctx().data(|d| d.get_temp::<std::collections::HashSet<egui::Id>>(thumb_failed_id()).is_some_and(|s| s.contains(&key))) {
        return None;
    }
    if *budget == 0 {
        *pending = true;
        return None;
    }
    *budget -= 1;
    match decode_thumb(&e.path, &e.name, e.size) {
        Some(img) => {
            let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
            let tex = ui.ctx().load_texture(format!("device-thumb-{}-{}", e.fingerprint, e.name), img, options);
            ui.ctx().data_mut(|d| {
                d.insert_temp(key, tex.clone());
                let evicted = {
                    let order = d.get_temp_mut_or_default::<Vec<egui::Id>>(thumb_order_id());
                    if !order.contains(&key) {
                        order.push(key);
                    }
                    let cut = order.len().saturating_sub(MAX_CACHED_THUMBS);
                    order.drain(..cut).collect::<Vec<_>>()
                };
                for old in evicted {
                    d.remove::<egui::TextureHandle>(old);
                }
            });
            Some(tex)
        }
        None => {
            ui.ctx().data_mut(|d| {
                let set = d.get_temp_mut_or_default::<std::collections::HashSet<egui::Id>>(thumb_failed_id());
                if set.len() > 512 {
                    set.clear();
                }
                set.insert(key);
            });
            None
        }
    }
}

/// Shorten a file name to `n` chars for the grid cell (`painter::text` never wraps).
fn truncate(name: &str, n: usize) -> String {
    if name.chars().count() <= n {
        return name.to_string();
    }
    let cut = name.char_indices().take(n).last().map_or(0, |(i, _)| i);
    format!("{}…", &name[..cut])
}

/// One grid cell: a folder icon or the photo's thumbnail, with the name and
/// size underneath. The accessible label keeps the old list text (folders with
/// their trailing slash), so tapping a photo still opens it.
fn cell(ui: &mut egui::Ui, e: &Entry, tex: Option<egui::TextureHandle>) -> egui::Response {
    const S: f32 = 88.0;
    let t = crate::theme::Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(egui::vec2(S + 8.0, S + 30.0), egui::Sense::click());
    let label = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    let box_r = egui::Rect::from_min_size(r.min + egui::vec2(4.0, 0.0), egui::vec2(S, S));
    ui.painter().rect_filled(box_r, 6.0, t.field);
    if e.is_dir {
        crate::icons::paint(ui, box_r, "folder", 30.0, t.icon);
    } else if let Some(tex) = tex {
        let (w, h) = (tex.size()[0] as f32, tex.size()[1] as f32);
        let k = (S / w.max(h).max(1.0)).min(1.0);
        let fitted = egui::Rect::from_center_size(box_r.center(), egui::vec2(w * k, h * k));
        ui.painter().image(tex.id(), fitted, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    } else {
        crate::icons::paint(ui, box_r, "image", 28.0, t.text_faint);
    }
    ui.painter().text(
        box_r.left_center() + egui::vec2(0.0, S / 2.0 + 10.0),
        egui::Align2::LEFT_CENTER,
        truncate(&e.name, 14),
        egui::FontId::proportional(12.0),
        t.text,
    );
    if !e.is_dir {
        ui.painter().text(
            box_r.left_center() + egui::vec2(0.0, S / 2.0 + 24.0),
            egui::Align2::LEFT_CENTER,
            size_text(e.size),
            egui::FontId::proportional(10.5),
            t.text_faint,
        );
    }
    resp
}

/// No grant yet: fire the runtime dialog and/or jump to the Settings screen
/// when the shell offers them, or dismiss with the corner X. The verdict is
/// rechecked every frame while the gate is up, so there is no Check again
/// button. Without the hooks (host tests) it stays the manual-steps text with
/// its Check/Close row.
fn gate(ctx: &egui::Context, services: &mut crate::Services) -> Gate {
    let mut out = Gate::default();
    let assisted = services.storage_request.is_some() || services.storage_settings.is_some();
    egui::Modal::new(egui::Id::new("device-browser-gate")).show(ctx, |ui| {
        ui.set_min_width(280.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Open")).size(14.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::icons::button(ui, "x", 32.0, false, tl!("Close")).clicked() {
                    out.closed = true;
                }
            });
        });
        if assisted {
            ui.label(tl!("PhotoCraft needs access to the files on this device to open them. Allow photo access here, or open Settings for every file kind (PSD, .pcraft)."));
        } else {
            ui.label(tl!("PhotoCraft needs the \"All files access\" permission to open files from this device."));
            ui.label(tl!("Open Settings > Apps > PhotoCraft and turn on \"All files access\", then come back here."));
        }
        ui.add_space(8.0);
        if assisted {
            ui.horizontal(|ui| {
                if services.storage_request.is_some()
                    && crate::widgets::secondary_button(ui, tl!("Allow access"), 128.0).clicked()
                    && let Some(request) = services.storage_request.as_mut()
                {
                    request();
                }
                if services.storage_settings.is_some()
                    && crate::widgets::secondary_button(ui, tl!("Open Settings"), 128.0).clicked()
                    && let Some(open) = services.storage_settings.as_mut()
                {
                    open();
                }
            });
        } else {
            ui.horizontal(|ui| {
                if crate::widgets::secondary_button(ui, tl!("Check again"), 128.0).clicked() {
                    out.recheck = true;
                }
                if crate::widgets::secondary_button(ui, tl!("Close"), 96.0).clicked() {
                    out.closed = true;
                }
            });
        }
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
    fn thumbs_decode_small_photos_and_skip_the_rest() {
        let dir = std::env::temp_dir().join(format!("photocraft-thumb-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = |name: &str, w: u32, h: u32| {
            let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
            let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
            std::fs::write(dir.join(name), &bytes).unwrap();
            bytes.len() as u64
        };
        let len = png("red.png", 200, 100);
        std::fs::write(dir.join("bad.png"), b"not an image").unwrap();
        std::fs::write(dir.join("note.txt"), b"hi").unwrap();

        assert_eq!(thumb_kind("red.PNG"), ThumbKind::Flat);
        assert_eq!(thumb_kind("a.pcraft"), ThumbKind::Pcraft);
        assert_eq!(thumb_kind("a.psd"), ThumbKind::None);
        assert_eq!(thumb_kind("a.txt"), ThumbKind::None);
        assert_eq!(thumb_kind("noext"), ThumbKind::None);

        let img = decode_thumb(&dir.join("red.png"), "red.png", Some(len)).expect("small photo decodes");
        assert_eq!(img.size, [100, 50], "long side shrinks to the grid, aspect kept");
        assert!(decode_thumb(&dir.join("bad.png"), "bad.png", Some(12)).is_none(), "corrupt bytes degrade to the icon");
        assert!(decode_thumb(&dir.join("note.txt"), "note.txt", Some(2)).is_none(), "unsupported files get no decode");
        assert!(decode_thumb(&dir.join("red.png"), "red.png", Some(MAX_THUMB_BYTES + 1)).is_none(), "oversized files keep the icon");
        assert!(decode_thumb(&dir.join("red.png"), "red.png", Some(0)).is_none(), "empty files keep the icon");
        assert!(decode_thumb(&dir.join("missing.png"), "missing.png", Some(10)).is_none(), "unreadable files never panic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn photo_grid_caches_thumbs_and_dirs_keep_their_icon() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let dir = std::env::temp_dir().join(format!("photocraft-grid-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for (name, px) in [("red.png", [255u8, 0, 0, 255]), ("green.png", [0u8, 255, 0, 255])] {
            let img = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgba, px.repeat(64)).unwrap();
            let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.device_browser = Some(Browser { dir: dir.clone(), error: None, access: Some(true) });
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
        assert!(h.query_all_by_label_contains("red.png").next().is_some(), "photo cell renders");
        assert!(h.query_all_by_label_contains("green.png").next().is_some(), "photo cell renders");
        assert!(h.query_all_by_label_contains("sub/").next().is_some(), "dir cell renders");
        for name in ["red.png", "green.png"] {
            let p = dir.join(name);
            let m = std::fs::metadata(&p).unwrap();
            let key = thumb_key(&p, fingerprint_of(&m, Some(m.len())));
            assert!(h.ctx.data(|d| d.get_temp::<egui::TextureHandle>(key).is_some()), "{name} thumb cached");
        }
        assert!(h.state().ui.device_browser.is_some(), "modal stays open");
        let _ = std::fs::remove_dir_all(&dir);
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

    fn hooked_services() -> crate::Services {
        crate::Services {
            storage_has_access: Some(Box::new(|| false)),
            storage_request: Some(Box::new(|| {})),
            storage_settings: Some(Box::new(|| {})),
            ..Default::default()
        }
    }

    fn gate_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                show(app, ui.ctx());
            },
            app,
        )
    }

    /// The verdict comes from the shell hook, not the probe: off-device the
    /// probe always fails, so file rows prove the hook was consulted.
    #[test]
    fn browser_uses_the_shell_verdict_when_present() {
        use egui_kittest::kittest::Queryable;
        let t = TempDir::new();
        let services = crate::Services { storage_has_access: Some(Box::new(|| true)), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.ui.device_browser = Some(Browser { dir: t.dir.clone(), error: None, access: None });
        let mut h = gate_harness(app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        assert!(h.query_all_by_label_contains("a.png").next().is_some(), "hook verdict opens the browser");
    }

    /// A denied hook keeps the gate with only the request/Settings buttons.
    #[test]
    fn gate_offers_request_and_settings_buttons() {
        use egui_kittest::kittest::Queryable;
        let t = TempDir::new();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), hooked_services());
        app.ui.device_browser = Some(Browser { dir: t.dir.clone(), error: None, access: None });
        let mut h = gate_harness(app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        assert!(h.query_all_by_label_contains("Allow access").next().is_some(), "request button renders");
        assert!(h.query_all_by_label_contains("Open Settings").next().is_some(), "settings button renders");
        assert!(h.query_all_by_label_contains("Check again").next().is_none(), "no recheck button");
        assert!(h.state().ui.device_browser.is_some(), "gate stays open");
    }
}
