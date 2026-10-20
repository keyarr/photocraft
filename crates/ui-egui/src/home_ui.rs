//! Home screen for phones: a modern start page with recent edits and device photos.
//!
//! - Top bar: only `Niveta` (semibold) on the left and the settings button
//!   on the right. No logo image, no subtitle, no search, no overflow menu.
//! - `Suas criações` + one-line hint + a blue `+ Nova edição` button that
//!   reuses [`NivetaApp::open_interactive`] (the on-device browser on
//!   Android, the system picker elsewhere).
//! - `Editadas recentemente`: a 2-column grid over the real Open Recent list
//!   (newest first, no fakes). Thumbnails decode lazily with a per-frame
//!   budget and degrade to an icon; the secondary line is the file's
//!   modification time (the last save), or its folder when that is unknown.
//! - `Fotos recentes`: a 3-column square grid over real device photos,
//!   newest arrival first (`max(created, modified)`). The time is only a
//!   sort key here, never shown as an "added" date. Thumbnails use the EXIF preview when
//!   there is one and never decode full-resolution images for the grid.
//!   `Ver todas` sits centered under the grid and reopens the same browser the
//!   Open button uses.
//! - Without photo access the section says so and its button fires the OS
//!   permission prompt directly, instead of opening the browser's gate.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{Color32, Rect, Sense, vec2};

use crate::NivetaApp;
use crate::theme::Tokens;

/// Height of the home top bar (without the Android status inset).
const BAR_H: f32 = 56.0;
/// Status inset on-device; egui does not report the real one.
#[cfg(target_os = "android")]
const SAFE_TOP: f32 = 30.0;
#[cfg(not(target_os = "android"))]
const SAFE_TOP: f32 = 0.0;

/// Recent edits shown (Open Recent is already newest-first).
const RECENT_MAX: usize = 6;
/// Device photos shown on the home grid.
const PHOTOS_MAX: usize = 9;
/// Inline thumbnail decodes per frame; the rest keep their icon until later.
const THUMB_BUDGET: usize = 4;
/// Longest thumbnail side in px (covers 3x phone cells without upscaling).
const THUMB_SIDE: u32 = 512;
/// Files bigger than this keep their icon; they still open on tap.
const MAX_THUMB_BYTES: u64 = 32 * 1024 * 1024;
/// Photo scan cache lifetime in seconds (egui time). Expiry never blocks a
/// frame: the stale list keeps showing while the rescan runs on a later one.
const PHOTOS_TTL: f64 = 15.0;

/// Home top bar: `Niveta` left, settings right. Used on narrow screens
/// instead of [`crate::mobile_ui::header`] when there is nothing to edit.
pub fn header(app: &mut NivetaApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("home_header")
        .exact_size(BAR_H + SAFE_TOP)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 16, right: 12, top: SAFE_TOP as i8, bottom: 0 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(tl!("Niveta")).font(crate::theme::semibold(18.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "settings", 44.0, false, tl!("Preferences")).clicked() {
                        crate::prefs_ui::open_preferences(app, "general");
                    }
                });
            });
        });
}

/// The home page: hero, recent edits (2 cols), recent device photos (3 cols).
/// One vertical scroll; the grids never scroll on their own.
pub fn home_screen(app: &mut NivetaApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let mut open: Option<String> = None;
    let mut open_clean: Option<String> = None;
    let mut new_edit = false;
    let mut see_all = false;
    let mut request_storage = false;
    let storage = storage_access(app);
    egui::ScrollArea::vertical().id_salt("home-scroll").auto_shrink([false, false]).show(ui, |ui| {
        let avail = ui.available_width();
        let content_w = (avail - 32.0).clamp(280.0, 720.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - content_w) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.set_width(content_w);
                ui.add_space(16.0);
                ui.label(egui::RichText::new(tl!("Your creations")).font(crate::theme::semibold(22.0)).color(t.text));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tl!("Pick up where you left off or start a new edit.")).size(13.0).color(t.text_dim));
                ui.add_space(12.0);
                if crate::widgets::primary_button(ui, tl!("+ New edit"), content_w).clicked() {
                    new_edit = true;
                }
                if content_w >= 420.0 {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let msg = crate::canvas::start_screen_drop_hint(app);
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        crate::icons::paint(ui, r, "image", 15.0, t.text_faint);
                        ui.label(egui::RichText::new(msg).size(12.5).color(t.text_faint));
                    });
                }
                ui.add_space(20.0);
                ui.label(egui::RichText::new(tl!("Recently edited")).font(crate::theme::semibold(15.0)).color(t.text));
                ui.add_space(10.0);
                recent_grid(app, ui, content_w, &mut open);
                ui.add_space(20.0);
                ui.label(egui::RichText::new(tl!("Recent photos")).font(crate::theme::semibold(15.0)).color(t.text));
                ui.add_space(10.0);
                let grid_shown = photos_grid(app, ui, content_w, storage, &mut open_clean, &mut see_all, &mut request_storage);
                if grid_shown {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        if crate::widgets::secondary_button(ui, tl!("See all"), 160.0).clicked() {
                            if !storage && app.services.storage_request.is_some() {
                                request_storage = true;
                            } else {
                                see_all = true;
                            }
                        }
                    });
                }
                ui.add_space(24.0);
            });
        });
    });
    if request_storage && let Some(request) = app.services.storage_request.as_mut() {
        request();
    }
    if new_edit || see_all {
        app.open_interactive();
    }
    if let Some(path) = open
        && let Err(e) = app.open_path(&path)
    {
        app.open_failed(&crate::file_open::display_name(&path), &e);
    }
    if let Some(path) = open_clean
        && let Err(e) = app.open_clean_path(&path)
    {
        app.open_failed(&crate::file_open::display_name(&path), &e);
    }
}

struct Recent {
    path: String,
    name: String,
    folder: String,
    modified: Option<SystemTime>,
}

/// Open Recent, newest first; the modification time is the last save when the
/// file is still there, otherwise the folder stands in as secondary text.
fn recents(app: &NivetaApp) -> Vec<Recent> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        return Vec::new();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.ui
            .recent_files
            .iter()
            .take(RECENT_MAX)
            .map(|path| {
                let name = crate::file_open::display_name(path);
                let folder = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                let modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
                Recent { path: path.clone(), name, folder, modified }
            })
            .collect()
    }
}

fn recent_grid(app: &NivetaApp, ui: &mut egui::Ui, content_w: f32, open: &mut Option<String>) {
    let t = Tokens::get(ui.ctx());
    let items = recents(app);
    if items.is_empty() {
        ui.label(egui::RichText::new(tl!("No edits yet.")).size(13.0).color(t.text_dim));
        return;
    }
    let gap = 12.0;
    let cell_w = ((content_w - gap) / 2.0).max(120.0);
    let thumb_h = cell_w * 0.75;
    let card_h = thumb_h + 46.0;
    let mut budget = frame_budget(ui.ctx());
    for row in items.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for item in row {
                if recent_card(app, ui, item, cell_w, thumb_h, card_h, &mut budget).clicked() {
                    *open = Some(item.path.clone());
                }
            }
        });
        ui.add_space(gap);
    }
}

/// One recent-edit card: uniform thumbnail, name, modification age or folder.
fn recent_card(app: &NivetaApp, ui: &mut egui::Ui, item: &Recent, w: f32, thumb_h: f32, h: f32, budget: &mut usize) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, item.name.clone()));
    let resp = resp.on_hover_text(&item.path).on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius, t.hover.gamma_multiply(0.45));
    } else {
        ui.painter().rect_filled(rect, t.radius, t.card);
    }
    let thumb = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(w - 12.0, thumb_h));
    ui.painter().rect_filled(thumb, t.radius_sm, t.field);
    match thumb_for(app, ui, &item.path, budget) {
        Some(tex) => paint_cover(ui, thumb, &tex),
        None => crate::icons::paint(ui, thumb, "image", 26.0, t.text_faint),
    }
    let name_g = crate::tab_strip::elided(ui, &item.name, egui::FontId::proportional(12.5), t.text, w - 24.0);
    ui.painter().galley(rect.min + vec2(12.0, thumb_h + 10.0), name_g, t.text);
    let sub = item.modified.map(relative_time).filter(|s| !s.is_empty()).unwrap_or_else(|| item.folder.clone());
    let sub_g = crate::tab_strip::elided(ui, &truncate(&sub, 32), egui::FontId::proportional(11.0), t.text_faint, w - 24.0);
    ui.painter().galley(rect.min + vec2(12.0, thumb_h + 28.0), sub_g, t.text_faint);
    resp
}

struct Photo {
    path: String,
    modified: SystemTime,
}

fn photos_grid(
    app: &NivetaApp,
    ui: &mut egui::Ui,
    content_w: f32,
    storage: bool,
    open: &mut Option<String>,
    see_all: &mut bool,
    request_storage: &mut bool,
) -> bool {
    let t = Tokens::get(ui.ctx());
    let (photos, pending) = recent_photos(ui.ctx(), storage);
    if photos.is_empty() {
        if pending {
            // Skeleton in the loaded grid's geometry: content swaps in
            // without shifting the page, instead of flashing empty states.
            let gap = 8.0;
            let cell = ((content_w - 2.0 * gap) / 3.0).clamp(80.0, 240.0);
            for _ in 0..3 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for _ in 0..3 {
                        let (rect, _) = ui.allocate_exact_size(vec2(cell, cell), Sense::hover());
                        ui.painter().rect_filled(rect, t.radius_sm, t.field);
                        crate::icons::paint(ui, rect, "image", 24.0, t.text_faint);
                    }
                });
                ui.add_space(gap);
            }
            return false;
        }
        if storage {
            ui.label(egui::RichText::new(tl!("No photos found.")).size(13.0).color(t.text_dim));
        } else {
            ui.label(egui::RichText::new(tl!("Niveta needs access to this device's photos to show them here.")).size(13.0).color(t.text_dim));
        }
        ui.add_space(8.0);
        if crate::widgets::secondary_button(ui, tl!("Choose image"), 170.0).clicked() {
            if !storage && app.services.storage_request.is_some() {
                *request_storage = true;
            } else {
                *see_all = true;
            }
        }
        return false;
    }
    let gap = 8.0;
    let cell = ((content_w - 2.0 * gap) / 3.0).clamp(80.0, 240.0);
    let mut budget = frame_budget(ui.ctx());
    for row in photos.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for p in row {
                let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
                let name = Path::new(&p.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.path.clone());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone()));
                let resp = resp.on_hover_text(&p.path).on_hover_cursor(egui::CursorIcon::PointingHand);
                ui.painter().rect_filled(rect, t.radius_sm, t.field);
                match thumb_for(app, ui, &p.path, &mut budget) {
                    Some(tex) => paint_cover(ui, rect, &tex),
                    None => crate::icons::paint(ui, rect, "image", 24.0, t.text_faint),
                }
                if resp.clicked() {
                    *open = Some(p.path.clone());
                }
            }
        });
        ui.add_space(gap);
    }
    true
}

/// Cached device photo scan (arrival-descending, `PHOTOS_MAX` kept). The cache
/// avoids a filesystem walk every frame; the sort key is the arrival time
/// the OS reports, shown nowhere as an "added" date.
///
/// The walk runs on a background thread: expiry serves the stale list (or a
/// skeleton, on cold start) while the rescan runs, so no frame ever blocks on
/// the filesystem and returning to the app always presents immediately. The
/// `bool` is true while a scan is still outstanding.
fn recent_photos(ctx: &egui::Context, permitted: bool) -> (Vec<Photo>, bool) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (ctx, permitted);
        return (Vec::new(), false);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        type Stored = Vec<(String, u64, u32, u64)>;
        fn to_photos(cached: Stored) -> Vec<Photo> {
            cached
                .into_iter()
                .filter_map(|(path, secs, nanos, _)| {
                    let modified = SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::new(secs, nanos))?;
                    Some(Photo { path, modified })
                })
                .collect()
        }
        fn store(photos: &[Photo]) -> Stored {
            photos
                .iter()
                .map(|p| {
                    let (secs, nanos) =
                        p.modified.duration_since(SystemTime::UNIX_EPOCH).ok().map(|d| (d.as_secs(), d.subsec_nanos())).unwrap_or((0, 0));
                    (p.path.clone(), secs, nanos, 0)
                })
                .collect()
        }
        let id = egui::Id::new("home-recent-photos");
        let job_id = egui::Id::new("home-recent-photos-scan");
        let perm_id = egui::Id::new("home-recent-photos-perm");
        let was_permitted = ctx.data(|d| d.get_temp::<bool>(perm_id));
        if was_permitted != Some(permitted) {
            ctx.data_mut(|d| {
                d.insert_temp(perm_id, permitted);
                d.remove_temp::<(f64, Stored, bool)>(id);
                d.remove_temp::<ScanSlot>(job_id);
            });
        }
        let now = ctx.input(|i| i.time);
        let entry = ctx.data(|d| d.get_temp::<(f64, Stored, bool)>(id));
        if let Some((_, cached, _)) = entry.as_ref().filter(|(at, _, dirty)| now - *at < PHOTOS_TTL && !*dirty) {
            return (to_photos(cached.clone()), false);
        }
        // Harvest a finished background scan: whichever frame notices it
        // publishes the fresh list.
        if let Some(slot) = ctx.data(|d| d.get_temp::<ScanSlot>(job_id)) {
            if let Some(photos) = slot.take() {
                ctx.data_mut(|d| {
                    d.remove_temp::<ScanSlot>(job_id);
                    d.insert_temp(id, (now, store(&photos), false));
                    // Thumbs for the new list resolve over the next frames,
                    // not inside the harvest frame.
                    d.insert_temp(warmup_id(), 0u32);
                });
                return (photos, false);
            }
            // Scan in flight: keep showing whatever we have (possibly stale).
            ctx.request_repaint();
            let stale = entry.map(|(_, cached, _)| to_photos(cached)).unwrap_or_default();
            return (stale, true);
        }
        // (Re)scan needed and none in flight.
        let stale = entry.map(|(_, cached, _)| to_photos(cached)).unwrap_or_default();
        match spawn_scan() {
            Some(slot) => {
                ctx.data_mut(|d| {
                    d.insert_temp(job_id, slot);
                    d.insert_temp(id, (now, store(&stale), true));
                });
                ctx.request_repaint();
                (stale, true)
            }
            None => {
                // Threads unavailable: the old blocking walk, once per expiry.
                let photos = scan_photos();
                ctx.data_mut(|d| d.insert_temp(id, (now, store(&photos), false)));
                (photos, false)
            }
        }
    }
}

/// Handoff for the background photo scan: the worker publishes its list here;
/// frames pick it up without ever blocking on the walk.
#[derive(Clone, Default)]
#[cfg(not(target_arch = "wasm32"))]
struct ScanSlot {
    done: std::sync::Arc<std::sync::Mutex<Option<Vec<Photo>>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ScanSlot {
    fn take(&self) -> Option<Vec<Photo>> {
        self.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}

/// The filesystem walk on a detached worker. `None` when no thread could be
/// spawned (the caller falls back to a blocking scan).
#[cfg(not(target_arch = "wasm32"))]
fn spawn_scan() -> Option<ScanSlot> {
    let slot = ScanSlot::default();
    let out = slot.done.clone();
    let job = std::thread::Builder::new()
        .name("home-photo-scan".into())
        .spawn(move || {
            let photos = std::panic::catch_unwind(std::panic::AssertUnwindSafe(scan_photos)).unwrap_or_default();
            *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(photos);
        });
    if job.is_err() {
        return None;
    }
    Some(slot)
}

fn warmup_id() -> egui::Id {
    egui::Id::new("home-thumb-warmup")
}

/// Inline thumbnail decodes allowed this frame. The first frames after process
/// start present the shell with icons; real thumbs resolve over the next
/// frames so the first present never waits on image decodes.
fn frame_budget(ctx: &egui::Context) -> usize {
    let id = warmup_id();
    let n = ctx.data(|d| d.get_temp::<u32>(id)).unwrap_or(0);
    ctx.data_mut(|d| d.insert_temp(id, n.saturating_add(1)));
    if n < 2 {
        ctx.request_repaint();
        0
    } else {
        THUMB_BUDGET
    }
}

/// Whether the shell's live permission verdict says shared storage is readable;
/// the filesystem probe is only the off-device fallback. Listing is no proof on
/// Android: a denied app still enumerates folders on scoped storage and gets an
/// empty view, so the probe can stay `true` across the grant.
fn storage_access(app: &mut NivetaApp) -> bool {
    match app.services.storage_has_access.as_mut() {
        Some(has) => has(),
        None => device_dirs_readable(),
    }
}

/// Whether any photo folder lists at all. Off-device fallback for
/// [`storage_access`]; never writes, never asks.
fn device_dirs_readable() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        photo_dirs().iter().any(|d| std::fs::read_dir(d).is_ok())
    }
}

fn photo_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for c in [
        "/sdcard/DCIM",
        "/sdcard/DCIM/Camera",
        "/sdcard/Pictures",
        "/sdcard/Download",
        "/storage/emulated/0/DCIM",
        "/storage/emulated/0/DCIM/Camera",
        "/storage/emulated/0/Pictures",
        "/storage/emulated/0/Download",
    ] {
        let p = PathBuf::from(c);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    dirs
}

/// Walk the photo folders (one subfolder deep for camera rolls), keep flat
/// images, sort by arrival on device descending. Arrival is
/// `max(created, modified)`: a download/copy keeps its old mtime but lands
/// with a fresh creation time, which is what "recent" means here. Falls back
/// to mtime where the FS has no birthtime. Same file seen through two roots
/// (`/sdcard` vs `/storage/emulated/0`) counts once. Only metadata is read
/// here, never file contents; paginated loading stays with the full
/// browser, home only previews the head.
fn scan_photos() -> Vec<Photo> {
    let mut out: Vec<(String, SystemTime, u64)> = Vec::new();
    let mut seen_dirs = std::collections::HashSet::new();
    let mut seen_files = std::collections::HashSet::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for d in photo_dirs() {
        if seen_dirs.insert(canonical_key(&d)) {
            roots.push(d);
        }
    }
    // One level into camera rolls (DCIM/Camera, Pictures/Screenshots, …);
    // skips dirs already covered by another root.
    for d in roots.clone() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                if !e.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let p = e.path();
                if seen_dirs.insert(canonical_key(&p)) {
                    roots.push(p);
                }
                if roots.len() >= 24 {
                    break;
                }
            }
        }
    }
    for d in &roots {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !is_flat_photo(&name) {
                continue;
            }
            // Same photo through two overlapping roots: keep the first. Lexical
            // only: a `canonicalize` per file costs a syscall each and made
            // the resume-frame scan take the better part of a second.
            if !seen_files.insert(lexical_key(&e.path())) {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let effective = meta.created().ok().map(|c| c.max(modified)).unwrap_or(modified);
            let len = meta.len();
            if len == 0 || len > 64 * 1024 * 1024 {
                continue;
            }
            out.push((e.path().to_string_lossy().into_owned(), effective, len));
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.1));
    out.truncate(PHOTOS_MAX);
    out.into_iter().map(|(path, modified, _)| Photo { path, modified }).collect()
}

/// Identity of a path for de-dup: resolves symlinks (`/sdcard` points at
/// `/storage/emulated/0` on Android) so overlapping roots collapse. Falls
/// back to a lexical normalization when the path cannot be resolved.
fn canonical_key(p: &Path) -> String {
    if let Ok(c) = p.canonicalize() {
        return c.to_string_lossy().into_owned();
    }
    lexical_key(p)
}

/// Syscall-free half of [`canonical_key`] for hot loops: files under the two
/// spellings of the same mount normalize to one key without touching the FS.
fn lexical_key(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("/storage/emulated/0") {
        return format!("/sdcard{rest}");
    }
    s.into_owned()
}

fn is_flat_photo(name: &str) -> bool {
    niveta_codecs::from_extension(name).is_some_and(|f| f.caps().read)
}

/// "há 5 min", "há 3 h", "há 2 dias": relative age of a modification time.
/// Future or unavailable times degrade to an empty string (the caller shows
/// the folder instead).
fn relative_time(mtime: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(mtime).ok().map(|d| d.as_secs()).unwrap_or(u64::MAX);
    if secs == u64::MAX {
        return String::new();
    }
    if secs < 60 {
        tl!("just now").to_string()
    } else if secs < 3600 {
        crate::i18n::fmt(tl!("{n} min ago"), &[("n", &(secs / 60).to_string())])
    } else if secs < 86400 {
        crate::i18n::fmt(tl!("{n} h ago"), &[("n", &(secs / 3600).to_string())])
    } else if secs / 86400 == 1 {
        tl!("1 day ago").to_string()
    } else {
        crate::i18n::fmt(tl!("{n} days ago"), &[("n", &(secs / 86400).to_string())])
    }
}

fn truncate(name: &str, n: usize) -> String {
    if name.chars().count() <= n {
        return name.to_string();
    }
    let cut = name.char_indices().take(n).last().map_or(0, |(i, _)| i);
    match name.get(..cut) {
        Some(head) => format!("{head}…"),
        None => name.to_string(),
    }
}

fn thumb_id(path: &str, fingerprint: u64) -> egui::Id {
    egui::Id::new(("home-thumb", path.to_owned(), fingerprint))
}

fn thumb_failed_id() -> egui::Id {
    egui::Id::new("home-thumb-failed")
}

/// Cached grid thumbnail for `path`; decodes at most `*budget` misses per
/// frame and repaints while visible thumbs are still pending.
fn thumb_for(app: &NivetaApp, ui: &egui::Ui, path: &str, budget: &mut usize) -> Option<egui::TextureHandle> {
    // A document open with unsaved edits is what the user is working on: the card
    // must show the editor's composite, not the file it was opened from.
    if let Some(st) = app.session.documents().iter().find(|d| d.path.as_deref() == Some(path) && d.is_dirty()) {
        let key = egui::Id::new(("home-edited-thumb", st.doc.id.0));
        // Keyed by document, holding the revision: an edit replaces the texture, so
        // a session's worth of revisions costs one thumbnail, not one each.
        if let Some((rev, tex)) = ui.ctx().data(|d| d.get_temp::<(u64, egui::TextureHandle)>(key))
            && rev == st.revision
        {
            return Some(tex);
        }
        if *budget == 0 {
            ui.ctx().request_repaint();
            return None;
        }
        *budget -= 1;
        let img = niveta_compose::thumbnail(&st.doc, THUMB_SIDE);
        let tex = ui.ctx().load_texture(
            format!("home-edited-{}", st.doc.id.0),
            egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels),
            egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR },
        );
        ui.ctx().data_mut(|d| d.insert_temp(key, (st.revision, tex.clone())));
        return Some(tex);
    }
    let (size, fingerprint) = fingerprint_of(path);
    if size == Some(0) || size.is_some_and(|n| n > MAX_THUMB_BYTES) {
        return None;
    }
    let key = thumb_id(path, fingerprint);
    if let Some(tex) = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(tex);
    }
    if ui.ctx().data(|d| d.get_temp::<std::collections::HashSet<egui::Id>>(thumb_failed_id()).is_some_and(|s| s.contains(&key))) {
        return None;
    }
    if *budget == 0 {
        ui.ctx().request_repaint();
        return None;
    }
    *budget -= 1;
    match decode_thumb(Path::new(path), size) {
        Some(img) => {
            let tex = ui.ctx().load_texture(
                format!("home-thumb-{fingerprint}"),
                img,
                egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR },
            );
            ui.ctx().data_mut(|d| d.insert_temp(key, tex.clone()));
            Some(tex)
        }
        None => {
            ui.ctx().data_mut(|d| {
                let set = d.get_temp_mut_or_default::<std::collections::HashSet<egui::Id>>(thumb_failed_id());
                if set.len() > 256 {
                    set.clear();
                }
                set.insert(key);
            });
            None
        }
    }
}

fn fingerprint_of(path: &str) -> (Option<u64>, u64) {
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len());
    let fp = meta
        .as_ref()
        .map(|m| {
            let (secs, nanos) =
                m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| (d.as_secs(), d.subsec_nanos())).unwrap_or((0, 0));
            secs.wrapping_mul(31).wrapping_add(u64::from(nanos)).wrapping_mul(31).wrapping_add(size.unwrap_or(0))
        })
        .unwrap_or(0);
    (size, fp)
}

/// Quick grid thumbnail: embedded `.pcraft` preview or the EXIF preview when
/// it is big enough, else a bounded full decode, shrunk to [`THUMB_SIDE`].
/// `None` means "keep the icon"; the file still opens on tap.
fn decode_thumb(path: &Path, size: Option<u64>) -> Option<egui::ColorImage> {
    if size.is_some_and(|n| n == 0 || n > MAX_THUMB_BYTES) {
        return None;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() as u64 > MAX_THUMB_BYTES {
        return None;
    }
    if name.rsplit('.').next().is_some_and(|ext| !name.trim_start_matches('.').is_empty() && ext.eq_ignore_ascii_case(niveta_format::EXTENSION)) {
        let png = niveta_format::read_thumbnail(&bytes).ok()??;
        let img = niveta_codecs::decode(&png).ok()?;
        return Some(shrink(&img.to_rgba8(), img.width(), img.height()));
    }
    if !is_flat_photo(&name) {
        return None;
    }
    if let Some(img) = niveta_codecs::exif_thumbnail(&bytes).filter(|img| img.width().max(img.height()) >= THUMB_SIDE) {
        // A 160x120 IFD1 stamp stretched to the grid looks worse than no
        // thumbnail at all; tiny previews fall through to the full decode.
        return Some(shrink(&img.to_rgba8(), img.width(), img.height()));
    }
    let limits = niveta_codecs::Limits { max_width: 12_000, max_height: 12_000, max_pixels: 64 * 1024 * 1024, max_alloc: 256 * 1024 * 1024 };
    let img = niveta_codecs::decode_with(&bytes, &niveta_codecs::DecodeOptions { limits, keep_orientation: false }).ok()?;
    Some(shrink(&img.to_rgba8(), img.width(), img.height()))
}

fn shrink(px: &[u8], w: u32, h: u32) -> egui::ColorImage {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let side = THUMB_SIDE as usize;
    let k = w.max(h).div_ceil(side).max(1);
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    // Area average: every source pixel feeds exactly one output cell, so
    // detail survives the downscale instead of aliasing away.
    let mut sum = vec![0u32; ow * oh * 4];
    let mut count = vec![0u32; ow * oh];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            let cell = (y / k) * ow + (x / k);
            let Some(p) = px.get(o..o + 4) else { continue };
            let s = &mut sum[cell * 4..cell * 4 + 4];
            s[0] += u32::from(p[0]);
            s[1] += u32::from(p[1]);
            s[2] += u32::from(p[2]);
            s[3] += u32::from(p[3]);
            count[cell] += 1;
        }
    }
    let mut out = Vec::with_capacity(ow * oh * 4);
    for (cell, n) in count.iter().enumerate() {
        let n = (*n).max(1);
        let s = &sum[cell * 4..cell * 4 + 4];
        out.extend_from_slice(&[(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8, (s[3] / n) as u8]);
    }
    egui::ColorImage::from_rgba_unmultiplied([ow, oh], &out)
}

/// Paint `tex` over `rect` covering it fully (center-cropped, aspect kept).
fn paint_cover(ui: &egui::Ui, rect: Rect, tex: &egui::TextureHandle) {
    let size = tex.size();
    let (w, h) = (size.first().copied().unwrap_or(1).max(1) as f32, size.get(1).copied().unwrap_or(1).max(1) as f32);
    let (br, tr) = (rect.width() / rect.height().max(1.0), w / h.max(1.0));
    let uv = if (tr - br).abs() < 1e-3 {
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
    } else if tr > br {
        let cut = (1.0 - br / tr) / 2.0;
        Rect::from_min_max(egui::pos2(cut, 0.0), egui::pos2(1.0 - cut, 1.0))
    } else {
        let cut = (1.0 - tr / br) / 2.0;
        Rect::from_min_max(egui::pos2(0.0, cut), egui::pos2(1.0, 1.0 - cut))
    };
    ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_degrades_gracefully() {
        assert!(!relative_time(SystemTime::now()).is_empty());
        assert_eq!(relative_time(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 60)), String::new());
    }

    #[test]
    fn truncate_keeps_short_names() {
        assert_eq!(truncate("cat.png", 14), "cat.png");
        assert!(truncate("a-very-long-file-name.png", 8).ends_with('…'));
    }

    /// One frame of `thumb_for` for `path`: whether the grid got something to paint, and the
    /// image it uploaded (nothing on a cache hit).
    fn thumbnail_frame(ctx: &egui::Context, app: &NivetaApp, path: &str) -> (bool, Option<egui::ColorImage>) {
        let mut budget = 4;
        let mut found = false;
        let egui::FullOutput { mut textures_delta, .. } = ctx.run_ui(Default::default(), |ui| {
            found = thumb_for(app, ui, path, &mut budget).is_some();
        });
        // The frame draws no widgets, so the one texture it may upload is the thumbnail.
        let img = textures_delta.set.values().flatten().next().map(|d| {
            let egui::ImageData::Color(c) = &d.image;
            egui::ColorImage { size: c.size, source_size: c.source_size, pixels: c.pixels.clone() }
        });
        textures_delta.clear();
        (found, img)
    }

    #[test]
    fn missing_files_have_no_thumbnail() {
        let app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear(); // the font atlas uploads once
        assert!(!thumbnail_frame(&ctx, &app, "/no/such/photo-anywhere.png").0);
    }

    /// A document open with unsaved edits shows its own pixels: the card must not keep
    /// showing the file it was opened from (or its icon, when the file moved) after an edit.
    #[test]
    fn open_documents_show_their_edits_on_the_grid() {
        let mut app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        app.run("file.new", serde_json::json!({"width": 32, "height": 32})).unwrap();
        // The grid finds the open document by its path; no such file exists on disk.
        let path = "/no/such/photo-anywhere.png";
        app.session.active_mut().unwrap().path = Some(path.into());
        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let (found, img) = thumbnail_frame(&ctx, &app, path);
        assert!(!found && img.is_none(), "a clean document reads the file, which is missing");
        app.run("edit.fill", serde_json::json!({"color": "#ff0000"})).unwrap();
        let (found, img) = thumbnail_frame(&ctx, &app, path);
        assert!(found, "the edited document renders without the file");
        let img = img.expect("the composite is uploaded");
        let mid = img.pixels[(img.size[1] / 2) * img.size[0] + img.size[0] / 2];
        assert_eq!((mid.r(), mid.g(), mid.b(), mid.a()), (255, 0, 0, 255), "the fill shows on the card");
        // Seen again without an edit: the cached texture, not a fresh render.
        assert!(thumbnail_frame(&ctx, &app, path).1.is_none());
        // Saved (or restored): the file is the thumbnail source again, and it is still missing.
        let st = app.session.active_mut().unwrap();
        st.saved_revision = st.revision;
        let (found, img) = thumbnail_frame(&ctx, &app, path);
        assert!(!found && img.is_none());
    }

    #[test]
    fn home_renders_sections_and_buttons() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                header(app, ui);
                home_screen(app, ui);
            },
            app,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        for probe in ["Niveta", "Your creations", "+ New edit", "Recently edited", "Recent photos"] {
            assert!(h.query_all_by_label_contains(probe).next().is_some(), "home shows {probe}");
        }
        assert!(h.query_all_by_label_contains("See all").next().is_none(), "no photo grid, no See all");
        assert!(h.query_all_by_label_contains("open source").next().is_none(), "no subtitle under the name");
        assert!(h.query_all_by_label_contains("Discord").next().is_none(), "no promo on the home screen");
    }

    #[test]
    fn home_empty_states_invite_a_new_edit() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        assert!(app.ui.recent_files.is_empty());
        let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                home_screen(app, ui);
            },
            app,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        assert!(h.query_all_by_label_contains("No edits yet.").next().is_some(), "recent empty state");
    }

    #[test]
    fn home_lists_real_recent_files_with_thumbnails() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let dir = std::env::temp_dir().join(format!("niveta-home-recent-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut paths = Vec::new();
        for (name, px) in [("red.png", [255u8, 0, 0, 255]), ("green.png", [0u8, 255, 0, 255])] {
            let img = niveta_codecs::Image::from_u8(16, 16, niveta_codecs::ChannelLayout::Rgba, px.repeat(256)).unwrap();
            let bytes = niveta_codecs::encode(&img, niveta_codecs::Format::Png, &niveta_codecs::EncodeOptions::default()).unwrap();
            let p = dir.join(name);
            std::fs::write(&p, &bytes).unwrap();
            paths.push(p.to_string_lossy().into_owned());
        }
        let mut app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        app.ui.recent_files = paths.clone();
        let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                home_screen(app, ui);
            },
            app,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        assert!(h.query_all_by_label_contains("red.png").next().is_some(), "recent card shows the file name");
        assert!(h.query_all_by_label_contains("green.png").next().is_some(), "recent card shows the file name");
        assert!(h.query_all_by_label_contains("No edits yet.").next().is_none(), "no empty state with recents");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlapping_roots_share_one_identity() {
        // `/sdcard` and `/storage/emulated/0` are the same mount on Android;
        // paths nobody can resolve fall back to a lexical normalization.
        let a = Path::new("/storage/emulated/0/__niveta_nope__/x.jpg");
        let b = Path::new("/sdcard/__niveta_nope__/x.jpg");
        assert_eq!(canonical_key(a), canonical_key(b));
        assert_eq!(lexical_key(a), lexical_key(b));
    }

    #[test]
    fn shrink_averages_instead_of_aliasing() {
        // 1px checkerboard: point sampling renders it solid black or white,
        // area averaging renders mid grey everywhere.
        let (w, h) = (1024usize, 1024usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                let o = (y * w + x) * 4;
                px[o] = v;
                px[o + 1] = v;
                px[o + 2] = v;
                px[o + 3] = 255;
            }
        }
        let img = shrink(&px, w as u32, h as u32);
        assert_eq!(img.size, [512, 512]);
        for p in img.pixels.iter().step_by(997) {
            assert!((120..=135).contains(&p.r()), "red averages to grey, got {}", p.r());
            assert!((120..=135).contains(&p.g()), "green averages to grey, got {}", p.g());
            assert!((120..=135).contains(&p.b()), "blue averages to grey, got {}", p.b());
        }
    }

    #[test]
    fn recent_photos_invalidates_cache_on_permission_change() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("home-recent-photos");
        let perm_id = egui::Id::new("home-recent-photos-perm");
        let current_perm = device_dirs_readable();
        let opposing_perm = !current_perm;

        ctx.data_mut(|d| {
            d.insert_temp(perm_id, opposing_perm);
            d.insert_temp(id, (0.0f64, vec![("fake.png".to_string(), 0u64, 0u32, 0u64)], false));
        });

        let (photos, pending) = recent_photos(&ctx, current_perm);
        assert!(pending);
        assert!(photos.is_empty());
        assert_eq!(ctx.data(|d| d.get_temp::<bool>(perm_id)), Some(current_perm));
    }

    #[test]
    fn recent_photos_retains_cache_when_permission_unchanged() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("home-recent-photos");
        let perm_id = egui::Id::new("home-recent-photos-perm");
        let current_perm = device_dirs_readable();

        ctx.data_mut(|d| {
            d.insert_temp(perm_id, current_perm);
            d.insert_temp(id, (0.0f64, vec![("fake.png".to_string(), 0u64, 0u32, 0u64)], false));
        });

        let (photos, pending) = recent_photos(&ctx, current_perm);
        assert!(!pending);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].path, "fake.png");
    }

    #[test]
    fn a_harvested_scan_does_not_read_as_in_flight() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("home-recent-photos");
        let job_id = egui::Id::new("home-recent-photos-scan");
        let perm = device_dirs_readable();
        let slot = ScanSlot::default();
        *slot.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(vec![Photo { path: "fresh.png".to_string(), modified: SystemTime::UNIX_EPOCH }]);
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("home-recent-photos-perm"), perm);
            // Expired cache: the frame takes the harvest path.
            d.insert_temp(id, (-1.0e9f64, Vec::<(String, u64, u32, u64)>::new(), false));
            d.insert_temp(job_id, slot);
        });

        let (photos, pending) = recent_photos(&ctx, perm);
        assert_eq!(photos.len(), 1);
        assert!(!pending);
        assert!(ctx.data(|d| d.get_temp::<ScanSlot>(job_id)).is_none(), "a finished slot must be dropped, or expiry never rescans");
    }

    #[test]
    fn choose_image_asks_the_os_instead_of_the_browser_gate() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let asked = Arc::new(AtomicBool::new(false));
        let mut app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        app.services.storage_has_access = Some(Box::new(|| false));
        let flag = asked.clone();
        app.services.storage_request = Some(Box::new(move || flag.store(true, Ordering::Relaxed)));
        let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                home_screen(app, ui);
            },
            app,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        // Cache an empty, finished scan so the empty state renders on the first frame.
        h.ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("home-recent-photos-perm"), false);
            d.insert_temp(egui::Id::new("home-recent-photos"), (0.0f64, Vec::<(String, u64, u32, u64)>::new(), false));
        });
        h.run_steps(3);
        h.get_by_label("Choose image").click();
        h.run_steps(2);
        assert!(asked.load(Ordering::Relaxed), "the button asks the OS directly");
        assert!(h.query_all_by_label_contains("Allow access").next().is_none(), "no egui permission gate");
        assert!(h.query_all_by_label_contains("See all").next().is_none(), "no grid without permission, no See all");
    }

    #[test]
    fn see_all_appears_below_the_photo_grid_only() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        app.services.storage_has_access = Some(Box::new(|| true));
        let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                home_screen(app, ui);
            },
            app,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("home-recent-photos-perm"), true);
            d.insert_temp(
                egui::Id::new("home-recent-photos"),
                (0.0f64, vec![("/no/such/photo.png".to_string(), 0u64, 0u32, 0u64)], false),
            );
        });
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("See all").next().is_some(), "the grid has its See all");
    }

    #[test]
    fn home_uses_the_shell_permission_verdict() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = NivetaApp::new(niveta_engine::Session::new(), crate::Services::default());
        app.services.storage_has_access = Some(Box::new(|| true));
        let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                home_screen(app, ui);
            },
            app,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        // The host probe says no, the shell hook says yes: the hook owns the
        // verdict, so the cached list survives instead of being rescanned away.
        h.ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("home-recent-photos-perm"), true);
            d.insert_temp(egui::Id::new("home-recent-photos"), (0.0f64, vec![("hooked.png".to_string(), 0u64, 0u32, 0u64)], false));
        });
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("hooked.png").next().is_some(), "the hook's verdict keeps the cached photos");
    }
}
