//! Touch export sheet: format, quality, scale, transparency, metadata and file
//! name for phones, which have no save picker. Same encode path as Export As
//! ([`crate::export_dialog`]), just packed for touch and written through the
//! shell's gallery hook when it has one, else into its export dir. On builds
//! with a picker (a narrow desktop window) confirming delegates to Export As,
//! so the name field there only seeds it.

use egui::vec2;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;
use crate::NivetaApp;

/// Transient export-sheet state (never persisted).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileExport {
    pub open: bool,
    pub format: String,
    pub quality: f32,
    pub lossless: bool,
    pub scale: f32,
    pub transparency: bool,
    pub metadata: String,
    pub filename: String,
}

fn stem_of(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 => base[..i].to_string(),
        _ => base.to_string(),
    }
}

/// Open the sheet for the active document, seeded from its name and the
/// export prefs. With no document there is nothing to export.
pub fn open(app: &mut NivetaApp) {
    let Some(st) = app.session.active() else {
        // Engine-style errors stay English-only (like "no document" below).
        app.ui.status = "No document to export".into();
        app.ui.status_error = true;
        return;
    };
    app.ui.mobile_export = MobileExport {
        open: true,
        format: "png".into(),
        quality: app.session.prefs().export.jpeg_quality as f32,
        lossless: false,
        scale: 100.0,
        transparency: true,
        metadata: "none".into(),
        filename: stem_of(&st.doc.name),
    };
}

fn fmt(e: &MobileExport) -> &str {
    if e.format.is_empty() { "png" } else { &e.format }
}

/// Where a direct write lands: the shell's export dir, else shared storage
/// when it takes a folder. `None` means nowhere writable was found.
fn base_dir(app: &NivetaApp) -> Option<String> {
    if let Some(b) = app.services.export_base_dir.clone() {
        return Some(b);
    }
    for root in ["/sdcard/DCIM/Niveta", "/storage/emulated/0/DCIM/Niveta"] {
        if std::fs::create_dir_all(root).is_ok() {
            return Some(root.into());
        }
    }
    None
}

fn fields(e: &MobileExport) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("format".into(), json!(fmt(e)));
    f.insert("quality".into(), json!(e.quality.round()));
    f.insert("lossless".into(), json!(e.lossless));
    f.insert("transparency".into(), json!(e.transparency));
    f.insert("scale".into(), json!(e.scale));
    f.insert("metadata".into(), json!(if e.metadata.is_empty() { "none" } else { &e.metadata }));
    f
}

fn fail(app: &mut NivetaApp, e: String) {
    log::warn!("mobile export failed: {e}");
    app.ui.status = e;
    app.ui.status_error = true;
}

/// Encode with the sheet's settings and write the file. With a picker the
/// Export As flow asks for the path; without one the file lands in `base_dir`.
pub fn confirm(app: &mut NivetaApp) {
    let f = fields(&app.ui.mobile_export);
    if app.services.file_dialog.is_some() {
        match crate::export_dialog::confirm(app, &f) {
            Ok(_) => app.ui.mobile_export.open = false,
            Err(e) => fail(app, e),
        }
        return;
    }
    let Some(st) = app.session.active() else {
        fail(app, "No document to export".into());
        return;
    };
    let ext = fmt(&app.ui.mobile_export).to_string();
    let mut name = app.ui.mobile_export.filename.trim().replace(['/', '\\'], "_");
    if name.is_empty() {
        name = stem_of(&st.doc.name);
    }
    if let Some((b, _)) = name.rsplit_once('.')
        && !b.is_empty()
    {
        name = b.to_string();
    }
    if name.is_empty() {
        name = "export".into();
    }
    let file_name = format!("{name}.{ext}");
    // A gallery save owns the location; the path is only the encoder's
    // extension hint there.
    let target = if app.services.save_to_gallery.is_some() {
        file_name.clone()
    } else {
        let Some(base) = base_dir(app) else {
            // Temporary build-gap notice, English-only like the engine's errors.
            fail(app, "Export needs storage access: grant it and try again".into());
            return;
        };
        if base.ends_with('/') { format!("{base}{file_name}") } else { format!("{base}/{file_name}") }
    };
    let out = match crate::export_dialog::export_document(&st.doc, &f, None) {
        Ok(d) => d,
        Err(e) => {
            fail(app, e);
            return;
        }
    };
    let settings = crate::export_dialog::settings(&f);
    let export = match app.services.export.as_ref() {
        Some(e) => e,
        None => {
            fail(app, "no exporter configured".into());
            return;
        }
    };
    let (bytes, warnings) = match export(&out, &target, &settings) {
        Ok(r) => r,
        Err(e) => {
            fail(app, e);
            return;
        }
    };
    let saved = app.services.save_to_gallery.as_mut().map(|save| save(&file_name, &bytes));
    let path = match saved {
        Some(Ok(path)) => path,
        Some(Err(e)) => {
            fail(app, e);
            return;
        }
        None => {
            if let Some(parent) = std::path::Path::new(&target).parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                fail(app, format!("Couldn't export {target}"));
                return;
            }
            match app.services.write.as_mut() {
                Some(write) => {
                    if let Err(e) = write(&target, &bytes) {
                        fail(app, format!("Couldn't export {target}: {e}"));
                        return;
                    }
                }
                None => {
                    fail(app, "no writer configured".into());
                    return;
                }
            }
            target
        }
    };
    app.ui.status = format!("Exported {path} ({})", crate::sizing::human_bytes(bytes.len() as f64));
    log::info!("mobile export: {}", app.ui.status);
    app.ui.status_error = false;
    crate::notices::io_warnings(app, &format!("Exported {}", crate::file_open::display_name(&path)), &warnings);
    app.ui.mobile_export.open = false;
}

/// Draw the sheet when open: format pills, the settings the format needs,
/// file name, destination and the Export button. Touch-sized throughout.
pub fn show(app: &mut NivetaApp, ctx: &egui::Context) {
    if !app.ui.mobile_export.open {
        return;
    }
    if app.session.active().is_none() {
        app.ui.mobile_export.open = false;
        return;
    }
    let (bw, bh) = app.session.active().map(|s| (f64::from(s.doc.size.width), f64::from(s.doc.size.height))).unwrap_or((1.0, 1.0));
    // Destination preview, computed before the sheet borrows the state mutably
    // (the sheet runs every frame, so a name typed this frame shows next).
    let dest = base_dir(app)
        .map(|b| {
            let e = &app.ui.mobile_export;
            format!("{b}/{}.{}", e.filename.trim(), fmt(e))
        })
        .unwrap_or_default();
    let mut do_export = false;
    let mut close = false;
    egui::Modal::new(egui::Id::new("mobile-export")).show(ctx, |ui| {
        ui.set_min_width(300.0);
        ui.set_max_width(360.0);
        let t = Tokens::get(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Export")).size(15.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::icons::button(ui, "x", 44.0, false, tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        let e = &mut app.ui.mobile_export;
        ui.label(egui::RichText::new(tl!("Format")).color(t.text_dim));
        let rest = (ui.available_width() - 8.0).max(120.0);
        egui::ScrollArea::horizontal().id_salt("mobile-export-formats").auto_shrink([true, true]).max_width(rest).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (key, label) in crate::export_dialog::FORMATS {
                    if crate::widgets::pill_tab(ui, label, fmt(e) == key).clicked() {
                        e.format = key.into();
                    }
                }
            });
        });
        ui.add_space(8.0);
        let cur = fmt(e).to_string();
        if cur == "webp" {
            crate::widgets::checkbox(ui, &mut e.lossless, tl!("Lossless"));
        }
        if cur == "jpg" || (cur == "webp" && !e.lossless) {
            crate::widgets::slider_row(ui, tl!("Quality"), &mut e.quality, 1.0..=100.0, "%", None);
            e.quality = e.quality.clamp(1.0, 100.0);
        }
        if cur != "jpg" {
            crate::widgets::checkbox(ui, &mut e.transparency, tl!("Transparency"));
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Metadata")).color(t.text_dim));
            let mut m = e.metadata.clone();
            let opts: Vec<(String, &str)> = vec![("none".into(), tl!("None")), ("all".into(), tl!("All"))];
            if crate::widgets::dropdown(ui, "mobile-export-metadata", &mut m, &opts, 130.0) {
                e.metadata = m;
            }
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tl!("Image Size")).color(t.text_dim));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Scale")).color(t.text_dim));
            crate::widgets::value_field(ui, &mut e.scale, 1.0..=1000.0, "%", 70.0);
            e.scale = e.scale.clamp(1.0, 1000.0);
        });
        let (w, h) = crate::export_dialog::scaled_size(bw, bh, f64::from(e.scale) / 100.0);
        ui.label(egui::RichText::new(format!("{w} × {h} px")).color(t.text_dim).size(11.5));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tl!("Name")).color(t.text_dim));
        let avail = ui.available_width().max(200.0);
        ui.add_sized(vec2(avail, 48.0), egui::TextEdit::singleline(&mut e.filename).desired_width(f32::INFINITY).margin(vec2(12.0, 12.0)));
        if !dest.is_empty() {
            ui.label(egui::RichText::new(dest.clone()).color(t.text_dim).size(11.5));
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add_sized(vec2(avail, 52.0), egui::Button::new(tl!("Export"))).clicked() {
                do_export = true;
            }
        });
    });
    if close {
        app.ui.mobile_export.open = false;
    } else if do_export {
        confirm(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> NivetaApp {
        NivetaApp::new(niveta_engine::Session::new(), crate::Services::default())
    }

    #[test]
    fn opening_seeds_the_name_from_the_document() {
        let mut a = app();
        a.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        assert!(!a.ui.mobile_export.open);
        open(&mut a);
        assert!(a.ui.mobile_export.open);
        assert!(!a.ui.mobile_export.filename.is_empty());
        assert_eq!(a.ui.mobile_export.format, "png");
    }

    #[test]
    fn opening_without_a_document_explains() {
        let mut a = app();
        open(&mut a);
        assert!(!a.ui.mobile_export.open);
        assert!(a.ui.status_error);
    }

    #[test]
    fn confirm_writes_the_chosen_format_into_the_base_dir() {
        let dir = std::env::temp_dir().join(format!("niveta-mobile-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = app();
        a.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        a.services.export =
            Some(Box::new(|_: &niveta_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"fake-png".to_vec(), Vec::new()))));
        a.services.write = Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        a.services.export_base_dir = Some(dir.to_string_lossy().into_owned());
        open(&mut a);
        a.ui.mobile_export.format = "jpg".into();
        a.ui.mobile_export.quality = 70.0;
        a.ui.mobile_export.filename = "shot".into();
        confirm(&mut a);
        assert!(!a.ui.mobile_export.open);
        assert!(!a.ui.status_error, "{}", a.ui.status);
        assert_eq!(std::fs::read(dir.join("shot.jpg")).unwrap(), b"fake-png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_prefers_the_gallery_hook_over_a_path() {
        let saved = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let sink = saved.clone();
        let mut a = app();
        a.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        a.services.export = Some(Box::new(|_: &niveta_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"fake-jpeg".to_vec(), Vec::new()))));
        a.services.export_base_dir = Some("DCIM/Niveta".into());
        a.services.save_to_gallery = Some(Box::new(move |name: &str, bytes: &[u8]| {
            sink.lock().unwrap_or_else(|e| e.into_inner()).push((name.to_string(), bytes.to_vec()));
            Ok(format!("DCIM/Niveta/{name}"))
        }));
        a.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("the path writer should not run".into())));
        open(&mut a);
        a.ui.mobile_export.filename = "shot".into();
        confirm(&mut a);
        assert!(!a.ui.status_error, "{}", a.ui.status);
        assert!(a.ui.status.contains("DCIM/Niveta/shot.png"), "{}", a.ui.status);
        let saved = saved.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].0, "shot.png");
        assert_eq!(saved[0].1, b"fake-jpeg");
    }

    #[test]
    fn sheet_settings_match_export_as() {
        let mut e = MobileExport { format: "webp".into(), quality: 70.0, ..MobileExport::default() };
        let s = crate::export_dialog::settings(&fields(&e));
        assert!(!s.webp_lossless);
        assert_eq!(s.webp_quality, Some(70));
        assert_eq!(s.jpeg_quality, None);
        e.format = "jpg".into();
        let s = crate::export_dialog::settings(&fields(&e));
        assert_eq!(s.jpeg_quality, Some(70));
    }

    /// Without a picker Save As has nowhere to ask: it opens this sheet
    /// instead of answering "cancelled".
    #[test]
    fn save_as_without_a_picker_opens_the_export_sheet() {
        let mut a = app();
        a.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(a.services.file_dialog.is_none());
        crate::menus::invoke(&mut a, &egui::Context::default(), "file.saveAs", json!({})).unwrap();
        assert!(a.ui.mobile_export.open);
        assert!(!a.ui.mobile_export.filename.is_empty());
    }
}
