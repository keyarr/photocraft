//! Compact chrome for narrow screens (phones and other windows under [`MOBILE_WIDTH`]):
//! a short header, a horizontally scrolling tool bar and a bottom sheet for the panels.
//!
//! Desktop panels assume a wide window (toolbar column + 250-520 pt dock next to the
//! canvas). On a 360 pt wide phone they leave no room for the document, so below
//! [`MOBILE_WIDTH`] the app shows this chrome instead. Every button here drives the same
//! commands, tools and panel bodies as the desktop layout: nothing is a mock and nothing
//! hides a feature, it is just packed for touch.

use egui::{Align2, Rect, RichText, Sense, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::Tool;
use crate::theme::Tokens;
use crate::{NivetaApp, icons, widgets};

/// Content width below which the mobile chrome replaces the desktop panels. Phones
/// (360-430 pt wide) get the compact layout; 800 pt and up keeps the desktop one (the
/// Help-menu search test covers a narrow 800 pt window, so the breakpoint stays below it).
pub const MOBILE_WIDTH: f32 = 720.0;

/// Space kept above the mobile header so it clears the Android status bar, which
/// draws over the app's window (Material needs at least 24dp here; 30pt keeps
/// "Home", the title, "Exportar" and "..." off the notifications).
/// egui doesn't report the real inset, so this is the standard status-bar
/// height until the shell reads it from the OS; desktop is unaffected.
#[cfg(target_os = "android")]
const SAFE_TOP: f32 = 30.0;
#[cfg(not(target_os = "android"))]
const SAFE_TOP: f32 = 0.0;

/// Whether the compact mobile chrome is in charge this frame.
pub fn is_mobile(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < MOBILE_WIDTH
}

/// Which dock group the bottom sheet shows (`None` = the canvas has the whole screen).
/// The groups are [`crate::dock::Group`], so the sheet shares the desktop dock's content
/// and tab state instead of copying it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileUi {
    pub panel: Option<crate::dock::Group>,
    /// The full tool grid is open over the canvas.
    pub tools_open: bool,
    /// Filter text for the tool list (cleared whenever the sheet opens or a tool is picked).
    pub tools_filter: String,
}

/// A two-finger gesture over the canvas: pan and pinch it, never paint with it. Returns the
/// gesture when at least two fingers are down with their centre on `rect`.
pub fn two_finger(ctx: &egui::Context, rect: egui::Rect) -> Option<egui::MultiTouchInfo> {
    ctx.input(|i| i.multi_touch()).filter(|m| m.num_touches >= 2 && rect.contains(m.center_pos))
}

/// Which half of the canvas a two-finger tap landed on: left undoes, right redoes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TapSide {
    Undo,
    Redo,
}

/// An in-progress two-finger tap over the canvas: a short two-finger touch with almost
/// no pan or pinch. Anything longer or farther keeps navigating (see [`two_finger`]).
#[derive(Clone, Copy, Debug)]
pub struct TapTrack {
    start: f64,
    center: egui::Pos2,
    moved: f32,
}

/// Longest a two-finger touch may last to still count as a tap, and farthest its
/// centre may wander (points).
const TAP_SECS: f64 = 0.4;
const TAP_TRAVEL: f32 = 24.0;

/// Fold one frame of multi-touch state into the tap tracker. Returns the tap's side when
/// the fingers lift in time and in place: the caller undoes (left) or redoes (right).
/// A pinch, spread or twist is navigation, never a tap, so any real zoom/rotation kills it.
pub fn two_finger_tap(track: &mut Option<TapTrack>, now: f64, rect: egui::Rect, touch: Option<&egui::MultiTouchInfo>) -> Option<TapSide> {
    let Some(m) = touch else {
        let t = track.take()?;
        let dt = now - t.start;
        // Same-frame lifts (dt == 0) still count; `(0.0..TAP_SECS)` would drop them.
        if !dt.is_sign_negative() && dt < TAP_SECS && t.moved < TAP_TRAVEL && rect.contains(t.center) {
            return Some(if t.center.x < rect.center().x { TapSide::Undo } else { TapSide::Redo });
        }
        return None;
    };
    if m.num_touches != 2 {
        // Three fingers (or more) is some other gesture: wait for fresh fingers.
        *track = None;
        return None;
    }
    match track {
        None => {
            // Taps start on the canvas; a gesture that slides in from the chrome never counts.
            if !rect.contains(m.center_pos) {
                return None;
            }
            *track = Some(TapTrack { start: now, center: m.center_pos, moved: 0.0 });
        }
        Some(t) => {
            if (m.zoom_delta - 1.0).abs() > 0.08 || m.rotation_delta.abs() > 0.06 {
                t.moved = TAP_TRAVEL + 1.0;
            } else {
                t.moved = t.moved.max(t.center.distance(m.center_pos));
            }
        }
    }
    None
}

/// Compact header: Home, the document title, export and an overflow menu. Undo and redo
/// live on two-finger taps over the canvas (left half undoes, right half redoes), so they
/// take no header room. Everything else lives one tap away in the command palette or the
/// overflow menu, so no menu bar has to fit in 360 pt. With no document open only the title
/// and the overflow menu stay: Home and export need an editable document.
pub fn header(app: &mut NivetaApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_header")
        .exact_size(52.0 + SAFE_TOP)
        // The top bar is one block: only the options panel below keeps a separator.
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 4, right: 8, top: SAFE_TOP as i8, bottom: 0 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let has_doc = app.session.active().is_some();
                if has_doc {
                    // Same toggle as the desktop options bar's Home button, touch-sized:
                    // going Home snapshots first on picker-less builds, where nothing
                    // else saves before a background kill (see `chrome_ui::home_button`).
                    let n = app.session.documents().len();
                    let on = app.ui.chrome.shows_home(n, app.session.prefs().general.auto_show_home_screen);
                    if icons::button_with_icon_size(ui, "house", 48.0, 22.0, on, tl!("Home")).clicked() {
                        if !on && app.services.file_dialog.is_none() {
                            crate::prefs_ui::autosave_now(app);
                        }
                        app.ui.chrome.home = if on { None } else { Some(n) };
                    }
                }
                let title =
                    app.session.active().map(|st| format!("{}{}", st.doc.name, if st.is_dirty() { " *" } else { "" })).unwrap_or_else(|| "Niveta".into());
                // Whatever the right-hand buttons take, the title gets the rest, truncated.
                let right_reserve = if has_doc { 72.0 } else { 0.0 } + 48.0 + 12.0;
                let avail = (ui.available_width() - right_reserve).max(48.0);
                ui.add_sized(vec2(avail, 48.0), egui::Label::new(RichText::new(title).color(t.text).size(13.0)).truncate());
                if has_doc {
                    // No save picker on phones: saving happens automatically
                    // (autosave + on exit), so the header offers Export instead. Transform
                    // confirm/cancel stay in the floating pill, not duplicated up here.
                    let export_on = crate::menus::is_enabled(app, "file.export.exportAs");
                    if ui.add_enabled_ui(export_on, |ui| widgets::secondary_button(ui, tl!("Export"), 64.0)).inner.clicked() {
                        crate::mobile_export::open(app);
                    }
                }
                let more = icons::button(ui, "ellipsis", 48.0, false, tl!("Options"));
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(MENU_W);
                    // Rows sit close together like the desktop menus': the old 44 + 4 spacing
                    // made the sixteen rows overflow a phone screen.
                    ui.spacing_mut().item_spacing.y = MENU_GAP;
                    menu_item(ui, app, tl!("New…"), "file.new", json!({}));
                    // Same as the Home-screen button: a build with no picker explains
                    // instead of flashing "cancelled".
                    if ui
                        .add_enabled(crate::menus::is_enabled(app, "file.open"), egui::Button::new(tl!("Open…")).min_size(vec2(MENU_W, MENU_ROW_H)))
                        .clicked()
                    {
                        app.open_interactive();
                        ui.close();
                    }
                    menu_item(ui, app, tl!("Save As…"), "file.saveAs", json!({}));
                    menu_item(ui, app, tl!("Redo"), "edit.redo", json!({}));
                    // File › Revert: back to the file the photograph came from, edits undone
                    // as one history step. Dirty documents get the discard prompt first.
                    menu_item(ui, app, tl!("Restore Original"), "file.revert", json!({}));
                    ui.separator();
                    menu_item(ui, app, tl!("Free Transform"), "edit.freeTransform", json!({}));
                    menu_item(ui, app, tl!("Scale"), "edit.transform.scale", json!({}));
                    menu_item(ui, app, tl!("Rotate"), "edit.transform.rotate", json!({}));
                    menu_item(ui, app, tl!("Skew"), "edit.transform.skew", json!({}));
                    menu_item(ui, app, tl!("Distort"), "edit.transform.distort", json!({}));
                    menu_item(ui, app, tl!("Perspective"), "edit.transform.perspective", json!({}));
                    menu_item(ui, app, tl!("Warp"), "edit.transform.warp", json!({}));
                    menu_item(ui, app, tl!("Transform Selection"), "select.transformSelection", json!({}));
                    ui.separator();
                    menu_item(ui, app, tl!("Fit on Screen"), "view.fitOnScreen", json!({}));
                    if ui.add_sized(vec2(MENU_W, MENU_ROW_H), egui::Button::new(tl!("Search commands, tools and panels…"))).clicked() {
                        app.ui.palette_open = true;
                        ui.close();
                    }
                });
            });
        });
}

/// Overflow menu geometry: wide enough for the longest label, rows at 40 + 2 gap (the old 44 + 4
/// pushed the last items off a 360x800 phone) without dropping below the comfortable tap size.
const MENU_W: f32 = 280.0;
const MENU_ROW_H: f32 = 40.0;
const MENU_GAP: f32 = 2.0;

fn menu_item(ui: &mut egui::Ui, app: &mut NivetaApp, label: &str, id: &str, params: serde_json::Value) {
    if ui.add_enabled(crate::menus::is_enabled(app, id), egui::Button::new(label).min_size(vec2(MENU_W, MENU_ROW_H))).clicked() {
        if let Err(e) = crate::menus::invoke(app, ui.ctx(), id, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        ui.close();
    }
}

/// The active tool's options, horizontally scrollable: single-line container with smooth
/// horizontal scrolling and hidden scrollbar so controls are never clipped abruptly.
pub fn options_strip(app: &mut NivetaApp, ui: &mut egui::Ui) {
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_options")
        .exact_size(48.0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(8, 0)))
        .show(ui, |ui| {
            let rest = ui.available_width().max(80.0);
            egui::ScrollArea::horizontal()
                .id_salt("mobile-options-scroll")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .max_width(rest)
                .show(ui, |ui| {
                    // `horizontal` sized the row to its content and pinned it to the strip's
                    // top edge; centered allocates the strip's height and aligns the controls
                    // with the header's, like the desktop bar's own `horizontal_centered`.
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if app.ui.transform.is_some() {
                            crate::transform_tool::mobile_options_bar(app, ui);
                        } else {
                            crate::panels::options_content(app, ui);
                        }
                        ui.add_space(24.0);
                    });
                });
        });
}

/// Floating pill centered at the bottom of the canvas for Transform confirm and cancel actions.
pub fn transform_floating_pill(app: &mut NivetaApp, ctx: &egui::Context) {
    if app.ui.transform.is_none() || !is_mobile(ctx) || app.session.active().is_none() {
        return;
    }
    let screen = ctx.content_rect();
    let offset_y = if app.ui.mobile.panel.is_some() {
        - (screen.height() * 0.45).clamp(260.0, 420.0) - 16.0
    } else {
        -76.0
    };
    egui::Area::new(egui::Id::new("mobile-transform-pill"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, offset_y))
        .show(ctx, |ui| {
            let t = Tokens::get(ui.ctx());
            egui::Frame::popup(ui.style())
                .fill(t.chrome)
                .stroke(egui::Stroke::new(1.0, t.separator))
                .corner_radius(20.0)
                .inner_margin(egui::Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        let warping = app.ui.transform.as_ref().is_some_and(|t| t.warp.is_some());
                        let cancel_tip = if warping { tl!("Cancel warp (Esc)") } else { tl!("Cancel transform (Esc)") };
                        let cancel_btn = icons::button_with_icon_size(ui, "ban", 36.0, 20.0, false, cancel_tip);
                        cancel_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel transform"));
                        if cancel_btn.clicked() {
                            crate::transform_tool::cancel(app);
                        }
                        widgets::vline(ui, 20.0);
                        let commit_tip = if warping {
                            crate::i18n::fmt(tl!("Commit warp ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))])
                        } else {
                            crate::i18n::fmt(tl!("Commit transform ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))])
                        };
                        let commit_btn = icons::button_with_icon_size(ui, "check", 36.0, 20.0, false, &commit_tip);
                        commit_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Commit transform"));
                        if commit_btn.clicked() {
                            crate::transform_tool::commit(app);
                        }
                    });
                });
        });
}

/// Temp-data namespaces of the two long-press cards: the bottom bar's and the tool grid's,
/// which can both be on screen.
const FAV_ROW: &str = "mobile-fav";
const GRID_ROW: &str = "mobile-grid";

/// Hold that turns a touch press on a tool button into its variant card (the desktop
/// toolbar's flyout delay). A tap must never open the card.
const HOLD_SECS: f64 = 0.35;

/// Touch tool button behaviour, shared by the bottom bar and the tool grid: a tap selects
/// the tool (returns `true`); a [`HOLD_SECS`] hold opens the variant card above the button
/// instead and the release that ends it selects nothing. A press away from the card closes
/// it, and moving off the button before the hold is up cancels it (egui drops
/// `is_pointer_button_down_on` past its click distance), so no gesture leaves a stale card.
/// `row` keeps the two rows' cards apart.
fn tool_press(app: &mut NivetaApp, ui: &mut egui::Ui, resp: &egui::Response, tool: Tool, subtools: &[Tool], row: &str) -> bool {
    let held_id = egui::Id::new((row, "held"));
    let card_id = egui::Id::new((row, "card"));
    if ui.input(|i| i.pointer.any_pressed()) {
        ui.data_mut(|d| d.remove::<Tool>(held_id));
        // Touch's click-outside: a press on a card row keeps the card, any other dismisses it.
        if let Some(rect) = ui.data(|d| d.get_temp::<Rect>(card_id.with("rect")))
            && ui.input(|i| i.pointer.press_origin().is_some_and(|p| !rect.contains(p)))
        {
            ui.data_mut(|d| d.remove::<Tool>(card_id));
        }
    }
    let held_for = resp.is_pointer_button_down_on().then(|| ui.input(|i| i.pointer.press_start_time().map(|t0| i.time - t0))).flatten();
    if subtools.len() > 1
        && let Some(seconds) = held_for
        && seconds < HOLD_SECS
    {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(HOLD_SECS - seconds));
    }
    if held_for.is_some_and(|seconds| seconds >= HOLD_SECS) {
        ui.data_mut(|d| d.insert_temp(held_id, tool));
        if subtools.len() > 1 {
            ui.data_mut(|d| d.insert_temp(card_id, tool));
        }
        ui.ctx().request_repaint();
    }
    let held = ui.data(|d| d.get_temp::<Tool>(held_id)) == Some(tool);
    if held && ui.data(|d| d.get_temp::<Tool>(card_id)) != Some(tool) {
        // Touch has no hover: the hold names the tool, in full (the grid shortens it).
        egui::Area::new(egui::Id::new((row, "name", tool.label())))
            .order(egui::Order::Tooltip)
            .fixed_pos(resp.rect.left_top() + vec2(-4.0, -40.0))
            .constrain(true)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(tl!(tool.label())).size(13.0).strong());
                });
            });
    }
    if ui.data(|d| d.get_temp::<Tool>(card_id)) == Some(tool) && subtools.len() > 1 {
        variant_card(app, ui, resp.rect, subtools, card_id);
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.remove::<Tool>(card_id));
    }
    resp.clicked() && !held
}

/// The variant card: every tool of a slot as a 36pt touch row above the button that opened
/// it. Both rows show this card, so a hold behaves the same in the bar and in the grid; the
/// rows carry the short names and take their width from the widest of them, so nothing
/// spills out of the phone.
fn variant_card(app: &mut NivetaApp, ui: &egui::Ui, anchor: Rect, subtools: &[Tool], card_id: egui::Id) {
    let t = Tokens::get(ui.ctx());
    let font = egui::FontId::proportional(12.5);
    let labels: Vec<&str> = subtools.iter().map(|&sub| Tool::short_display(tl!(sub.label()))).collect();
    let label_w = labels.iter().map(|l| ui.painter().layout_no_wrap((*l).to_owned(), font.clone(), t.text).size().x).fold(0.0f32, f32::max);
    let w = (label_w + 46.0).clamp(160.0, (ui.ctx().content_rect().width() - 16.0).max(140.0));
    let h = subtools.len() as f32 * 36.0 + (subtools.len().saturating_sub(1)) as f32 * 3.0 + 12.0;
    let area = egui::Area::new(card_id)
        .order(egui::Order::Tooltip)
        .fixed_pos(anchor.left_top() - vec2(0.0, h + 8.0))
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(6, 6)).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for (&sub, label) in subtools.iter().zip(&labels) {
                        let is_sel = app.ui.tool == sub;
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
                        if is_sel {
                            ui.painter().rect_filled(r, t.radius_sm, t.accent_soft);
                            ui.painter().rect_stroke(r, t.radius_sm, egui::Stroke::new(1.0, t.accent_border), StrokeKind::Inside);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, t.radius_sm, t.hover);
                        }
                        let tint = if is_sel { t.accent_text } else if resp.hovered() { t.text } else { t.icon };
                        icons::paint(ui, Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(20.0, 20.0)), icons::tool_icon(sub), 18.0, tint);
                        let color = if is_sel { t.accent_text } else { t.text };
                        let mut job = egui::text::LayoutJob::simple_singleline((*label).to_owned(), font.clone(), color);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((w - 42.0).max(1.0));
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(r.left() + 36.0, r.center().y - g.size().y / 2.0), g, color);
                        if resp.clicked() {
                            app.ui.tool = sub;
                            ui.data_mut(|d| d.remove::<Tool>(card_id));
                        }
                    }
                });
            });
        });
    ui.data_mut(|d| d.insert_temp(card_id.with("rect"), area.response.rect));
}

/// Favourite tools, touch-sized and scrollable, with the panel shortcuts beside them.
/// The active tool is always in the row (prepended when it is not a favourite), so the
/// selection never silently points at a tool the user cannot see. Hidden with no
/// document open: the start screen has no editing context.
pub fn tool_bar(app: &mut NivetaApp, ui: &mut egui::Ui) {
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("mobile_tools")
        .exact_size(68.0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 12 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // Tools modal toggle button: uses neutral card style when open, never tool-accent selection border.
                let tools_open = app.ui.mobile.tools_open;
                let (tools_rect, tools_resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
                if tools_open {
                    ui.painter().rect_filled(tools_rect, t.radius_sm, t.card);
                    ui.painter().rect_stroke(tools_rect, t.radius_sm, egui::Stroke::new(1.0, t.card_border), StrokeKind::Inside);
                } else if tools_resp.hovered() {
                    ui.painter().rect_filled(tools_rect, t.radius_sm, t.hover);
                }
                let tools_tint = if tools_open || tools_resp.hovered() { t.text } else { t.icon };
                icons::paint(ui, tools_rect, "grid-2x2", 22.0, tools_tint);
                if tools_resp.on_hover_text(tl!("Tools")).clicked() {
                    app.ui.mobile.tools_open = !app.ui.mobile.tools_open;
                    if app.ui.mobile.tools_open {
                        app.ui.mobile.tools_filter.clear();
                    }
                }

                // Fixed buttons after the scrolled row: grid, three panel shortcuts.
                let rest = (ui.available_width() - 3.0 * 48.0 - 4.0).max(64.0);
                egui::ScrollArea::horizontal().id_salt("mobile-tools-scroll").auto_shrink([false, false]).max_width(rest).show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let mut favs = favourites().to_vec();
                        if !favs.contains(&app.ui.tool) {
                            let mut replaced = false;
                            for fav in favs.iter_mut() {
                                if crate::panels::tool_subtools(*fav).contains(&app.ui.tool) {
                                    *fav = app.ui.tool;
                                    replaced = true;
                                    break;
                                }
                            }
                            if !replaced {
                                favs.insert(0, app.ui.tool);
                            }
                        }
                        for tool in favs {
                            let tip = tl!(tool.label()).to_string();
                            let is_active_tool = app.ui.tool == tool;
                            let subtools = crate::panels::tool_subtools(tool);

                            // Exclusive blue accent border ONLY for the active editing tool
                            let resp = icons::button_with_icon_size(ui, icons::tool_icon(tool), 48.0, 24.0, is_active_tool, &tip);
                            if subtools.len() > 1 {
                                let c = resp.rect.right_bottom() - vec2(3.0, 3.0);
                                let triangle = [c - vec2(5.0, 0.0), c, c - vec2(0.0, 5.0)];
                                let tri_tint = if is_active_tool { t.accent_text } else { t.text_faint };
                                ui.painter().add(egui::Shape::convex_polygon(triangle.to_vec(), tri_tint, egui::Stroke::NONE));
                            }

                            if tool_press(app, ui, &resp, tool, subtools, FAV_ROW) {
                                app.ui.tool = tool;
                            }
                        }
                    });
                });
                for (icon, name, group) in [
                    ("palette", tl!("Color"), crate::dock::Group::Color),
                    ("sliders-horizontal", tl!("Properties"), crate::dock::Group::Properties),
                    ("layers", tl!("Layers"), crate::dock::Group::Layers),
                ] {
                    // Panel shortcuts use distinct filled style + bottom indicator dot (never tool blue selection border)
                    let on = app.ui.mobile.panel == Some(group);
                    let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
                    if on {
                        ui.painter().rect_filled(rect, t.radius_sm, t.card);
                        ui.painter().circle_filled(pos2(rect.center().x, rect.bottom() - 5.0), 2.5, t.accent);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
                    }
                    let tint = if on || resp.hovered() { t.text } else { t.text_faint };
                    icons::paint(ui, rect, icon, 24.0, tint);
                    if resp.on_hover_text(name).clicked() {
                        app.ui.mobile.panel = if on { None } else { Some(group) };
                    }
                }
            });
        });
}

/// The tools a phone keeps one tap away; everything else is in the grid behind `grid-2x2`.
fn favourites() -> [Tool; 10] {
    use Tool::*;
    [Move, RectMarquee, Lasso, Brush, Eraser, Gradient, CloneStamp, Type, Hand, Zoom]
}

/// The full tool list over the canvas: high-density 3-4 column grid with subtools grouped
/// by slot, fluid vertical scrolling, and search fixed at the top.
pub fn tool_sheet(app: &mut NivetaApp, ctx: &egui::Context) {
    if !app.ui.mobile.tools_open || app.session.active().is_none() {
        return;
    }
    let screen = ctx.content_rect();
    let list_w = (screen.width() - 48.0).clamp(280.0, 420.0);
    let cols = if list_w >= 340.0 { 4 } else { 3 };
    let gap = 6.0;
    let card_w = ((list_w - 32.0 - (cols as f32 - 1.0) * gap) / cols as f32).clamp(48.0, 160.0);
    egui::Window::new(tl!("Tools"))
        .id(egui::Id::new("mobile-tools"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -84.0))
        .collapsible(false)
        .resizable(false)
        .constrain(true)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(list_w);
            ui.set_max_width(list_w);
            // Custom header: title left, X on the same row right.
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Tools")).size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "x", 40.0, false, tl!("Close")).clicked() {
                        app.ui.mobile.tools_open = false;
                    }
                });
            });
            ui.add_space(4.0);
            let avail = ui.available_width().max(200.0);
            ui.add_sized(
                vec2(avail, 40.0),
                egui::TextEdit::singleline(&mut app.ui.mobile.tools_filter).hint_text(tl!("Search commands, tools and panels…")).desired_width(f32::INFINITY).margin(vec2(10.0, 10.0)),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical().id_salt("mobile-tools-list").auto_shrink([false, false]).max_height((screen.height() * 0.55).clamp(220.0, 480.0)).show(
                ui,
                |ui| {
                    ui.add_space(8.0);
                    let q = app.ui.mobile.tools_filter.trim().to_lowercase();
                    let sections = crate::panels::visible_sections(&app.session.prefs().toolbar.hidden);
                    if !q.is_empty() {
                        let tools: Vec<Tool> = sections
                            .iter()
                            .flat_map(|section| section.iter().flat_map(|(_, slot)| slot.iter().copied()))
                            .filter(|tool| tl!(tool.label()).to_lowercase().contains(&q))
                            .collect();
                        if tools.is_empty() {
                            ui.label(RichText::new(tl!("No matching commands")).color(Tokens::get(ui.ctx()).text_faint));
                            return;
                        }
                        egui::Grid::new("mobile-tools-grid").num_columns(cols).spacing([gap, gap]).show(ui, |ui| {
                            for (i, tool) in tools.iter().enumerate() {
                                tool_card(app, ui, *tool, card_w, crate::panels::tool_subtools(*tool));
                                if (i + 1) % cols == 0 {
                                    ui.end_row();
                                }
                            }
                            if !tools.len().is_multiple_of(cols) {
                                ui.end_row();
                            }
                        });
                    } else {
                        let slots: Vec<Vec<Tool>> = sections
                            .iter()
                            .flat_map(|section| section.iter().map(|(_, slot)| slot.clone()))
                            .collect();
                        egui::Grid::new("mobile-tools-grid").num_columns(cols).spacing([gap, gap]).show(ui, |ui| {
                            for (i, slot) in slots.iter().enumerate() {
                                let active_in_slot = slot.iter().find(|&&t| t == app.ui.tool).copied();
                                let display_tool = active_in_slot.unwrap_or(slot[0]);
                                tool_card(app, ui, display_tool, card_w, slot);
                                if (i + 1) % cols == 0 {
                                    ui.end_row();
                                }
                            }
                            if !slots.len().is_multiple_of(cols) {
                                ui.end_row();
                            }
                        });
                    }
                    ui.add_space(12.0);
                },
            );
        });
}

/// A card's name, wrapped onto at most two rows and centred inside the card, plus the point
/// to paint it at. `halign = Center` centres every row on the galley's origin, so that point
/// is the card's centre line (not a top-left corner).
fn card_label(ctx: &egui::Context, card: Rect, text: &str, color: egui::Color32) -> (std::sync::Arc<egui::Galley>, egui::Pos2) {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), egui::FontId::proportional(11.0), color, (card.width() - 6.0).max(1.0));
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    (ctx.fonts_mut(|f| f.layout_job(job)), pos2(card.center().x, card.top() + 34.0))
}

/// One compact touch card: 20px icon over the short name, which wraps onto a second row
/// instead of ellipsing at once (the hold card lists the slot's variants).
fn tool_card(app: &mut NivetaApp, ui: &mut egui::Ui, tool: Tool, w: f32, subtools: &[Tool]) {
    let t = Tokens::get(ui.ctx());
    let full = tl!(tool.label()).to_string();
    let short = Tool::short_display(tl!(tool.label())).to_string();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, full.clone()));
    let is_sel = app.ui.tool == tool;
    let tint = icons::button_chrome(ui, r, is_sel, resp.hovered());
    icons::paint(ui, Rect::from_center_size(pos2(r.center().x, r.top() + 18.0), vec2(24.0, 24.0)), icons::tool_icon(tool), 20.0, tint);
    if subtools.len() > 1 {
        let c = r.right_bottom() - vec2(3.0, 3.0);
        let triangle = [c - vec2(4.0, 0.0), c, c - vec2(0.0, 4.0)];
        let tri_tint = if is_sel { t.accent_text } else { t.text_faint };
        ui.painter().add(egui::Shape::convex_polygon(triangle.to_vec(), tri_tint, egui::Stroke::NONE));
    }
    let fg = if is_sel { t.accent_text } else { t.text };
    let (label, at) = card_label(ui.ctx(), r, &short, fg);
    ui.painter().galley(at, label, fg);
    let resp = resp.on_hover_text(full);
    if tool_press(app, ui, &resp, tool, subtools, GRID_ROW) {
        app.ui.tool = tool;
        app.ui.mobile.tools_open = false;
        app.ui.mobile.tools_filter.clear();
    }
}

/// The bottom sheet: one dock group at a time, with its own height cap and scrolling, so
/// the canvas takes the room back as soon as it closes. The bodies are the desktop dock's
/// ([`crate::panels::dock_body`]), sharing its state and commands.
pub fn sheet(app: &mut NivetaApp, ui: &mut egui::Ui) {
    if app.session.active().is_none() {
        return;
    }
    let Some(group) = app.ui.mobile.panel else { return };
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let h = (screen.height() * 0.45).clamp(260.0, 420.0);
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("mobile_sheet").exact_size(h).frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin { left: 12, right: 12, top: 6, bottom: 6 })).show(ui, |outer| {
        let body = outer.max_rect();
        let clip = outer.clip_rect();
        let mut inner = outer.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        inner.set_clip_rect(Rect::from_min_max(pos2(clip.min.x, body.min.y.max(clip.min.y)), pos2(clip.max.x, body.max.y.min(clip.max.y))));
        let ui = &mut inner;

        let pro = Tokens::get(ui.ctx()).pro;
        let tabs = group.tabs(pro);
        let has_subtabs = tabs.len() > 1;

        // Unified 40pt header row: horizontally scrolling group pills with safety margin,
        // compact dropdown selector for subtabs, and fixed Close button on the right.
        let row_w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(row_w, 40.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut groups = vec![
                (crate::dock::Group::Color, tl!("Color")),
                (crate::dock::Group::Properties, tl!("Properties")),
                (crate::dock::Group::Layers, tl!("Layers")),
                (crate::dock::Group::History, tl!("History")),
                (crate::dock::Group::Navigator, tl!("Navigator")),
            ];
            if app.ui.panels.character {
                groups.push((crate::dock::Group::Character, tl!("Character")));
            }
            let right_reserve = 32.0 + if has_subtabs { 100.0 } else { 0.0 } + 16.0;
            let rest = (row_w - right_reserve).max(64.0);
            egui::ScrollArea::horizontal()
                .id_salt("mobile-sheet-groups")
                .auto_shrink([true, true])
                .max_width(rest)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (g, label) in groups {
                            if widgets::pill_tab(ui, label, group == g).clicked() {
                                app.ui.mobile.panel = if group == g { None } else { Some(g) };
                            }
                        }
                        ui.add_space(8.0);
                    });
                });

            if has_subtabs {
                let mut sel = tab_index(&app.ui.dock_tabs, group).min(tabs.len().saturating_sub(1));
                let opts: Vec<(usize, &str)> = tabs.iter().enumerate().map(|(i, &tab_name)| (i, tl!(tab_name))).collect();
                // Long tab names (Layer Comps in most languages) must not widen the row past Close.
                let picked = ui.scope(|ui| {
                    ui.set_max_width(92.0);
                    widgets::dropdown(ui, "mobile-sheet-subtab", &mut sel, &opts, 92.0)
                });
                if picked.inner {
                    *tab_index_mut(&mut app.ui.dock_tabs, group) = sel;
                }
            }

            if icons::button(ui, "x", 32.0, false, tl!("Close")).clicked() {
                app.ui.mobile.panel = None;
            }
        });

        let tab = tab_index(&app.ui.dock_tabs, group).min(tabs.len().saturating_sub(1));
        if group.scrolls_itself(tab) {
            crate::panels::dock_body(app, ui, group, tab);
        } else {
            egui::ScrollArea::vertical().id_salt(("mobile-sheet", group.key())).auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(8.0);
                crate::panels::dock_body(app, ui, group, tab);
                ui.add_space(8.0);
            });
        }
        outer.expand_to_include_rect(body);
    });
}

fn tab_index(tabs: &crate::state::DockTabs, group: crate::dock::Group) -> usize {
    match group {
        crate::dock::Group::Color => tabs.color,
        crate::dock::Group::Properties => tabs.properties,
        crate::dock::Group::Character => tabs.character,
        crate::dock::Group::Navigator => tabs.navigator,
        crate::dock::Group::History => tabs.history,
        crate::dock::Group::Layers => tabs.layers,
    }
}

fn tab_index_mut(tabs: &mut crate::state::DockTabs, group: crate::dock::Group) -> &mut usize {
    match group {
        crate::dock::Group::Color => &mut tabs.color,
        crate::dock::Group::Properties => &mut tabs.properties,
        crate::dock::Group::Character => &mut tabs.character,
        crate::dock::Group::Navigator => &mut tabs.navigator,
        crate::dock::Group::History => &mut tabs.history,
        crate::dock::Group::Layers => &mut tabs.layers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use serde_json::json;

    fn app() -> NivetaApp {
        NivetaApp::new(niveta_engine::Session::new(), crate::Services::default())
    }

    /// One frame of the bottom bar driven by raw pointer events: a tap and a hold differ
    /// only in the timing between the press and the release. (The kittest drag/drop helpers
    /// drop the cursor on release, which makes egui see no click at all.)
    fn bar_frame(app: &mut NivetaApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
        let mut out = ctx.run_ui(
            egui::RawInput { time: Some(time), events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 800.0))), ..Default::default() },
            |ui| tool_bar(app, ui),
        );
        out.textures_delta.clear();
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// The clickable rects of one size the last pass laid out, top to bottom then left to
    /// right: how the tests find the bar's buttons and the card's rows.
    fn clickable(ctx: &egui::Context, size: egui::Vec2) -> Vec<Rect> {
        let mut rects: Vec<Rect> = ctx.viewport(|v| {
            v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size && w.sense.senses_click()).map(|w| w.rect).collect()
        });
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
        rects
    }

    /// A phone with a document open and two settled frames of bottom bar behind it.
    fn bar(app: &mut NivetaApp) -> egui::Context {
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let ctx = egui::Context::default();
        NivetaApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        bar_frame(app, &ctx, 0.0, vec![]);
        bar_frame(app, &ctx, 0.1, vec![]);
        ctx
    }

    /// The open card's rows are found by count (their widget rects are layer-local) and
    /// aimed at through the card's own area rect.
    fn card_row_count(ctx: &egui::Context) -> usize {
        clickable(ctx, vec2(160.0, 36.0)).len()
    }

    /// A row of the open card: 6pt margin, then 36pt rows 3pt apart.
    fn card_row(ctx: &egui::Context, index: usize) -> egui::Pos2 {
        let card = ctx.memory(|m| m.area_rect(card_id())).expect("the card is up");
        pos2(card.left() + 46.0, card.top() + 6.0 + 39.0 * index as f32 + 18.0)
    }

    /// A bottom-bar favourite by index (0 = Move, 2 = Lasso, 3 = Brush…): the row starts
    /// after the tools button (8 + 48 + 4), each button is 48 wide with 4 between. Only the
    /// first few are on screen without scrolling, so the tests hold those.
    fn fav_button(ctx: &egui::Context, index: f32) -> egui::Pos2 {
        let at = pos2(8.0 + 48.0 + 4.0 + 52.0 * index + 24.0, 761.0);
        let rect = clickable(ctx, vec2(48.0, 48.0)).into_iter().find(|r| r.contains(at)).unwrap_or_else(|| panic!("the bar drew favourite {index}"));
        assert!(rect.max.x <= 360.0, "favourite {index} is on screen: {rect:?}");
        rect.center()
    }

    fn card_id() -> egui::Id {
        egui::Id::new((FAV_ROW, "card"))
    }

    /// A tap on a bar tool selects it and never opens the variant card: tapping the active
    /// tool (the one with subtools) used to toggle the card open.
    #[test]
    fn bar_tap_selects_and_never_opens_the_variant_card() {
        let mut a = app();
        let ctx = bar(&mut a);
        let lasso = fav_button(&ctx, 2.0);
        for active in [Tool::Move, Tool::Lasso] {
            a.ui.tool = active;
            bar_frame(&mut a, &ctx, 1.0, vec![egui::Event::PointerMoved(lasso), pointer(lasso, true)]);
            bar_frame(&mut a, &ctx, 1.05, vec![pointer(lasso, false)]);
            bar_frame(&mut a, &ctx, 1.1, vec![]);
            assert_eq!(a.ui.tool, Tool::Lasso, "{active:?}: the tap selects the tool");
            assert!(ctx.data(|d| d.get_temp::<Tool>(card_id())).is_none(), "{active:?}: a tap never opens the card");
            assert_eq!(card_row_count(&ctx), 0, "{active:?}: no variant rows after a tap");
        }
    }

    /// Holding a bar tool opens its variant card above the bar: the release that ends the
    /// hold selects nothing, and a row picks the variant.
    #[test]
    fn bar_hold_opens_the_variant_card() {
        let mut a = app();
        let ctx = bar(&mut a);
        a.ui.tool = Tool::Lasso;
        let lasso = fav_button(&ctx, 2.0);
        bar_frame(&mut a, &ctx, 1.0, vec![egui::Event::PointerMoved(lasso), pointer(lasso, true)]);
        bar_frame(&mut a, &ctx, 1.36, vec![]);
        assert_eq!(ctx.data(|d| d.get_temp::<Tool>(card_id())), Some(Tool::Lasso), "the hold opens the card");
        assert_eq!(card_row_count(&ctx), crate::panels::tool_subtools(Tool::Lasso).len(), "every variant gets a row");
        bar_frame(&mut a, &ctx, 1.4, vec![pointer(lasso, false)]);
        bar_frame(&mut a, &ctx, 1.45, vec![]);
        assert_eq!(a.ui.tool, Tool::Lasso, "the release that ends a hold selects nothing");
        assert_eq!(ctx.data(|d| d.get_temp::<Tool>(card_id())), Some(Tool::Lasso), "the card stays up for the tap");
        // The third row is the Magnetic Lasso (Lasso, Polygonal Lasso, Magnetic Lasso).
        let row = card_row(&ctx, 2);
        bar_frame(&mut a, &ctx, 2.0, vec![egui::Event::PointerMoved(row), pointer(row, true)]);
        bar_frame(&mut a, &ctx, 2.05, vec![pointer(row, false)]);
        assert_eq!(a.ui.tool, Tool::MagneticLasso, "the row picks the variant");
        assert!(ctx.data(|d| d.get_temp::<Tool>(card_id())).is_none(), "picking one closes the card");
    }

    /// A press anywhere off the card dismisses it (touch's click-outside), so a stale card
    /// never survives the next gesture.
    #[test]
    fn bar_press_away_dismisses_the_variant_card() {
        let mut a = app();
        let ctx = bar(&mut a);
        a.ui.tool = Tool::Lasso;
        let lasso = fav_button(&ctx, 2.0);
        bar_frame(&mut a, &ctx, 1.0, vec![egui::Event::PointerMoved(lasso), pointer(lasso, true)]);
        bar_frame(&mut a, &ctx, 1.36, vec![]);
        assert_eq!(ctx.data(|d| d.get_temp::<Tool>(card_id())), Some(Tool::Lasso), "the hold opens the card");
        let grid = clickable(&ctx, vec2(48.0, 48.0))[0].center();
        bar_frame(&mut a, &ctx, 1.6, vec![pointer(lasso, false)]);
        bar_frame(&mut a, &ctx, 1.7, vec![egui::Event::PointerMoved(grid), pointer(grid, true)]);
        assert!(ctx.data(|d| d.get_temp::<Tool>(card_id())).is_none(), "pressing away dismisses the card");
        bar_frame(&mut a, &ctx, 1.75, vec![pointer(grid, false)]);
    }

    /// The tool grid's cards behave like the bar's buttons: holding one opens the same
    /// variant card, and the release that ends the hold still selects nothing.
    #[test]
    fn tool_sheet_hold_opens_the_variant_card() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.mobile.tools_open = true;
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let ctx = ui.ctx().clone();
                tool_sheet(app, &ctx);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        let tool = h.state().ui.tool;
        let brush = h.query_all_by_label("Brush Tool").next().expect("the grid shows the brush").rect().center();
        h.drag_at(brush);
        h.run_steps(3);
        let card = egui::Id::new((GRID_ROW, "card"));
        assert_eq!(h.ctx.data(|d| d.get_temp::<Tool>(card)), Some(Tool::Brush), "the hold opens the grid's card");
        h.drop_at(brush);
        h.run_steps(2);
        assert_eq!(h.state().ui.tool, tool, "the release that ends a hold selects nothing");
        assert!(h.state().ui.mobile.tools_open, "the sheet stays up for the variant");
    }

    #[test]
    fn narrow_screens_use_the_mobile_chrome() {
        let ctx = egui::Context::default();
        for (w, h, mobile) in
            [(360.0, 800.0, true), (390.0, 844.0, true), (412.0, 915.0, true), (800.0, 600.0, false), (1024.0, 768.0, false), (1440.0, 900.0, false)]
        {
            let mut out =
                ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(w, h))), ..Default::default() }, |_| {});
            out.textures_delta.clear();
            assert_eq!(is_mobile(&ctx), mobile, "{w}x{h}");
        }
    }

    #[test]
    fn mobile_state_serializes_with_the_rest() {
        let mut s = crate::state::UiState::default();
        assert_eq!(s.mobile.panel, None);
        s.mobile.panel = Some(crate::dock::Group::Layers);
        s.mobile.tools_open = true;
        let back: crate::state::UiState = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back.mobile.panel, Some(crate::dock::Group::Layers));
        assert!(back.mobile.tools_open);
    }

    /// The mobile chrome renders at phone widths and leaves the canvas room: at 360 pt
    /// the canvas keeps the full width (no desktop side panels) with the sheet closed, and
    /// stays usable with the sheet open.
    #[test]
    fn chrome_renders_at_360_points() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let chrome = |ui: &mut egui::Ui, app: &mut NivetaApp| {
            // The themed fonts install after the first frame; skip until they are in.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let ctx = ui.ctx().clone();
            header(app, ui);
            options_strip(app, ui);
            tool_bar(app, ui);
            sheet(app, ui);
            tool_sheet(app, &ctx);
            crate::canvas::document_area(app, ui);
        };
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(chrome, a);
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(60);
        assert!(is_mobile(&h.ctx));
        let r = h.state().last_canvas_rect;
        assert!(r.width() >= 340.0, "full width with no side panels: {r:?}");
        assert!(r.height() > 500.0, "canvas dominates with the sheet closed: {r:?}");
        // Open the sheet and the tool grid: everything still renders, canvas stays usable.
        h.state_mut().ui.mobile.panel = Some(crate::dock::Group::Layers);
        h.state_mut().ui.mobile.tools_open = true;
        h.run_steps(6);
        assert!(h.query_all_by_label("Export").next().is_some(), "the header keeps Export visible");
        assert!(h.query_all_by_label("Tools").next().is_some(), "the tool grid opens");
        assert_eq!(h.state().ui.mobile.panel, Some(crate::dock::Group::Layers));
        let r = h.state().last_canvas_rect;
        assert!(r.width() >= 340.0, "the sheet never squeezes the width: {r:?}");
        assert!(r.height() > 200.0, "the canvas stays usable with the sheet open: {r:?}");
    }

    /// The sheet body renders inside the window: the group and sub-tab rows take
    /// one row each, so the panel body keeps the rest of the sheet. (Both rows
    /// used `horizontal_centered`, which allocates the whole sheet height and
    /// pushed the body off-screen, leaving every tab empty.)
    #[test]
    fn sheet_body_stays_inside_the_window() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        a.ui.mobile.panel = Some(crate::dock::Group::Layers);
        let chrome = |ui: &mut egui::Ui, app: &mut NivetaApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let ctx = ui.ctx().clone();
            header(app, ui);
            options_strip(app, ui);
            tool_bar(app, ui);
            sheet(app, ui);
            tool_sheet(app, &ctx);
            crate::canvas::document_area(app, ui);
        };
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(chrome, a);
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        for probe in ["Opacity", "Background"] {
            let n = h.query_all_by_label_contains(probe).next().unwrap_or_else(|| panic!("the layers body renders {probe}"));
            assert!(n.rect().max.y <= 800.0, "{probe} is inside the window: {:?}", n.rect());
        }
    }

    /// Opening and closing the sheet never touches the editing state.
    #[test]
    fn sheet_keeps_the_edit_state() {
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.tool = Tool::Brush;
        let rev = a.session.active().unwrap().revision;
        let journal = a.session.journal.len();
        for panel in [crate::dock::Group::Color, crate::dock::Group::Properties, crate::dock::Group::Layers] {
            a.ui.mobile.panel = Some(panel);
            let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
                |ui, app: &mut NivetaApp| {
                    if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    sheet(app, ui);
                },
                a,
            );
            NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.run_steps(3);
            a = h.into_state();
            a.ui.mobile.panel = None;
            assert_eq!(a.ui.tool, Tool::Brush, "the tool survives {panel:?}");
            assert_eq!(a.session.active().unwrap().revision, rev, "the document survives {panel:?}");
            assert_eq!(a.session.journal.len(), journal, "no command ran for {panel:?}");
        }
    }

    /// The home screen carries no promo at any width: no Discord, no links.
    #[test]
    fn start_screen_has_no_promo_on_phones() {
        use egui_kittest::kittest::Queryable;
        for w in [360.0, 1440.0] {
            let a = app();
            assert!(a.session.documents().is_empty());
            let mut h = Harness::builder().with_size(vec2(w, 800.0)).build_ui_state(
                |ui, app: &mut NivetaApp| {
                    if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    crate::canvas::document_area(app, ui);
                },
                a,
            );
            NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.run_steps(6);
            assert!(h.query_all_by_label_contains("Discord").next().is_none(), "{w} pt promo");
            assert!(h.query_all_by_label_contains("GitHub").next().is_none(), "{w} pt links");
            assert!(h.query_all_by_label_contains("Your creations").next().is_some(), "{w} pt home");
        }
    }

    /// The tool sheet names every tool: touch has no hover, so the old icon-only grid
    /// left 52 tools undescribed. Each row carries its tool label as its accessible name.
    #[test]
    fn tool_sheet_names_every_tool() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.tool = Tool::Brush;
        a.ui.mobile.tools_open = true;
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let ctx = ui.ctx().clone();
                tool_sheet(app, &ctx);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        for probe in ["Brush Tool", "Clone Stamp Tool", "Horizontal Type Tool", "Zoom Tool"] {
            assert!(h.query_all_by_label_contains(probe).next().is_some(), "the sheet names {probe}");
        }
    }

    /// Card names never leave their card: every tool, in every UI language, wraps onto at
    /// most two rows inside the narrowest card the sheet makes (320pt screen, three columns),
    /// and the paint point centres those rows (`halign = Center` puts the origin in the middle).
    #[test]
    fn tool_card_names_stay_inside_the_card() {
        let ctx = egui::Context::default();
        NivetaApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        // The themed fonts install on the first frame.
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 800.0))), ..Default::default() }, |_| {});
        out.textures_delta.clear();
        let card = egui::Rect::from_min_size(egui::pos2(24.0, 400.0), vec2(78.7, 64.0));
        let tools: Vec<Tool> = crate::panels::visible_sections(&[]).into_iter().flatten().flat_map(|(_, slot)| slot).collect();
        assert!(tools.len() > 30, "the sweep covers the whole toolbox");
        for lang in crate::i18n::Lang::all() {
            crate::i18n::set_current(lang);
            for tool in &tools {
                let short = Tool::short_display(crate::i18n::t(tool.label()));
                let (g, at) = card_label(&ctx, card, short, egui::Color32::WHITE);
                let painted = g.rect.translate(at.to_vec2());
                assert!(card.contains_rect(painted.expand(0.5)), "{}: {short:?} leaves its card: {painted:?}", lang.code());
            }
        }
        crate::i18n::set_current(crate::i18n::Lang::EN);
    }


    /// The sheet filter narrows the rows ("heal" keeps the healing tools, drops the brush).
    #[test]
    fn tool_sheet_filter_narrows_the_rows() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.mobile.tools_open = true;
        a.ui.mobile.tools_filter = "heal".into();
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let ctx = ui.ctx().clone();
                tool_sheet(app, &ctx);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        assert!(h.query_all_by_label_contains("Healing Brush Tool").next().is_some(), "the filter keeps matches");
        assert!(h.query_all_by_label("Brush Tool").next().is_none(), "the filter drops the rest");
        assert!(h.query_all_by_label_contains("No matching commands").next().is_none(), "matches exist, so no empty state");
    }

    /// The tool window stays on screen: a bare right-to-left close row once measured
    /// screen height and blew the window off the top (only its bottom showed on-device).
    #[test]
    fn tool_sheet_stays_inside_the_window() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.mobile.tools_open = true;
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let ctx = ui.ctx().clone();
                tool_sheet(app, &ctx);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        let win = h.query_all_by_label_contains("Tools").next().expect("the tool window renders").rect();
        assert!(win.min.y >= 0.0 && win.max.y <= 800.0 && win.max.x <= 360.0, "the window stays on screen: {win:?}");
        for probe in ["Move Tool", "Rectangular Marquee Tool"] {
            let n = h.query_all_by_label_contains(probe).next().unwrap_or_else(|| panic!("the first row shows {probe}"));
            assert!(n.rect().min.y >= 0.0 && n.rect().max.y <= 800.0, "{probe} is inside the window: {:?}", n.rect());
        }
    }

    /// The Adjustments tab starts at the top of the sheet: unshrunk horizontal pill
    /// scrolls once ate the panel height and pushed the body out under the canvas.
    #[test]
    fn sheet_adjustments_stay_inside_the_window() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.mobile.panel = Some(crate::dock::Group::Properties);
        a.ui.dock_tabs.properties = 1;
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                header(app, ui);
                tool_bar(app, ui);
                sheet(app, ui);
                crate::canvas::document_area(app, ui);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        for probe in ["Add an adjustment", "Brightness/Contrast", "Levels"] {
            let n = h.query_all_by_label_contains(probe).next().unwrap_or_else(|| panic!("the adjustments body renders {probe}"));
            assert!(n.rect().min.y >= 0.0 && n.rect().max.y <= 800.0, "{probe} is inside the window: {:?}", n.rect());
        }
    }

    /// The layer comps tab keeps the sheet header on screen: its long name once widened the
    /// subtab dropdown past the close button and off the phone.
    #[test]
    fn sheet_layer_comps_subtab_stays_inside_the_window() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.mobile.panel = Some(crate::dock::Group::History);
        a.ui.dock_tabs.history = 2;
        let mut h = Harness::builder().with_size(vec2(320.0, 700.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                header(app, ui);
                tool_bar(app, ui);
                sheet(app, ui);
                crate::canvas::document_area(app, ui);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        // "Composição de camadas" is long enough to widen the row; the harness does not sync prefs itself.
        crate::i18n::set_current(crate::i18n::Lang::from_code("pt-br").expect("pt-br"));
        h.run_steps(2);
        let combo = h.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the subtab dropdown renders").rect();
        assert!(combo.min.x >= 0.0 && combo.max.x <= 320.0 - 32.0 - 4.0, "the subtab keeps room for Close: {combo:?}");
    }

    /// The Layers footer stays inside the sheet: the 56pt touch cells once overflowed
    /// the 30pt desktop footer bar onto the bottom nav, and its right-to-left bar hid
    /// buttons off the left edge.
    #[test]
    fn sheet_layers_footer_stays_inside_the_window() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.mobile.panel = Some(crate::dock::Group::Layers);
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                header(app, ui);
                tool_bar(app, ui);
                sheet(app, ui);
                crate::canvas::document_area(app, ui);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        let del = h.query_all_by_label_contains("Delete layer").next().expect("the footer renders Delete").rect();
        assert!(del.max.y <= 800.0, "Delete stays above the bottom nav: {del:?}");
        let link = h.query_all_by_label_contains("Link layers").next().expect("the footer renders Link").rect();
        assert!(link.min.x >= 0.0 && link.max.y <= 800.0, "Link starts on screen: {link:?}");
    }

    /// The options strip hides the active tool's name (the bottom bar already marks it)
    /// but keeps its contextual controls.
    #[test]
    fn options_strip_hides_the_tool_name_but_keeps_its_options() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        a.ui.tool = Tool::CloneStamp;
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                options_strip(app, ui);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        assert!(h.query_all_by_label_contains("Clone Stamp Tool").next().is_none(), "the strip no longer repeats the active tool");
        assert!(h.query_all_by_label_contains("Aligned").next().is_some(), "the Clone Stamp options stay");
        assert!(h.query_all_by_label_contains("Set Source").next().is_some(), "the touch Set Source toggle is there");
    }

    /// With no document open the strip draws nothing: there is no tool context.
    #[test]
    fn options_strip_hides_without_a_document() {
        use egui_kittest::kittest::Queryable;
        let a = app();
        assert!(a.session.active().is_none());
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                options_strip(app, ui);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        assert!(h.query_all_by_label_contains("Aligned").next().is_none(), "no tool options without a document");
    }

    /// The canvas never covers the sheet: at the phone's density (ppp 2.57) the Layers
    /// body ran a few points past the sheet's exact height, the bottom panel then reported
    /// its top that much lower, and the canvas painted over the group pills (SM-S928B).
    #[test]
    fn canvas_stays_above_the_sheet_at_phone_density() {
        for (w, hh, ppp) in [(360.0, 800.0, 1.0), (420.0, 910.0, 2.56875), (412.0, 915.0, 2.625), (360.0, 640.0, 3.0)] {
            let mut a = app();
            a.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            for _ in 0..7 {
                a.run("layer.new.layer", json!({})).unwrap();
            }
            a.ui.mobile.panel = Some(crate::dock::Group::Layers);
            let chrome = |ui: &mut egui::Ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                header(app, ui);
                options_strip(app, ui);
                tool_bar(app, ui);
                sheet(app, ui);
                crate::canvas::document_area(app, ui);
            };
            let mut h = Harness::builder().with_size(vec2(w, hh)).with_pixels_per_point(ppp).build_ui_state(chrome, a);
            NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
            h.run_steps(8);
            let sheet_top = hh - 68.0 - (hh * 0.45_f32).clamp(260.0, 420.0);
            let canvas = h.state().last_canvas_rect;
            assert!(canvas.max.y <= sheet_top + 0.5, "{w}x{hh}@{ppp}: canvas {canvas:?} runs into the sheet at {sheet_top}");
        }
    }

    /// Two fingers over the canvas count as navigation, one (or none) does not.
    #[test]
    fn two_fingers_navigate_single_touch_paints() {
        use egui::{Event, TouchDeviceId, TouchId, TouchPhase};
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 600.0));
        let ctx = egui::Context::default();
        let dev = TouchDeviceId(7);
        let touch = |id: u64, phase: TouchPhase, pos: egui::Pos2| Event::Touch { device_id: dev, id: TouchId(id), phase, pos, force: None };
        let c = rect.center();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 800.0));
        let frame = |time: f64, events: Vec<Event>| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |_| {});
            out.textures_delta.clear();
        };
        // Hover first: a gesture needs a pointer position to start from.
        frame(1.0, vec![Event::PointerMoved(c)]);
        assert_eq!(two_finger(&ctx, rect), None);
        // One finger: still a painting touch, not navigation.
        frame(2.0, vec![touch(1, TouchPhase::Start, c)]);
        assert_eq!(two_finger(&ctx, rect), None, "one finger paints");
        // A second finger lands: from now on the gesture navigates.
        frame(3.0, vec![touch(2, TouchPhase::Start, c + vec2(40.0, 0.0))]);
        let m = two_finger(&ctx, rect).expect("two fingers navigate");
        assert!(m.num_touches >= 2);
        // Outside the canvas (over the tool bar) it is not a canvas gesture.
        let bar = egui::Rect::from_min_size(egui::pos2(0.0, 740.0), vec2(360.0, 60.0));
        assert_eq!(two_finger(&ctx, bar), None);
    }

    fn tap_touch(center: egui::Pos2, n: usize) -> egui::MultiTouchInfo {
        egui::MultiTouchInfo {
            start_time: 0.0,
            start_pos: center,
            center_pos: center,
            num_touches: n,
            zoom_delta: 1.0,
            zoom_delta_2d: vec2(1.0, 1.0),
            rotation_delta: 0.0,
            translation_delta: egui::Vec2::ZERO,
            force: 0.0,
        }
    }

    /// A quick two-finger tap undoes on the left half of the canvas, redoes on the right.
    #[test]
    fn two_finger_tap_picks_undo_left_and_redo_right() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 600.0));
        for (at, want) in [(pos2(90.0, 300.0), TapSide::Undo), (pos2(270.0, 300.0), TapSide::Redo)] {
            let mut track = None;
            assert_eq!(two_finger_tap(&mut track, 1.0, rect, Some(&tap_touch(at, 2))), None);
            assert!(track.is_some(), "fingers down start the track");
            assert_eq!(two_finger_tap(&mut track, 1.2, rect, None), Some(want), "lifting in time fires {want:?}");
            assert!(track.is_none(), "the track resets after firing");
        }
    }

    /// Slow lifts, wandering centres, extra fingers and taps outside the canvas stay navigation.
    #[test]
    fn two_finger_tap_ignores_non_taps() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 600.0));
        // Held too long: a pan, not a tap.
        let mut track = None;
        assert_eq!(two_finger_tap(&mut track, 1.0, rect, Some(&tap_touch(pos2(90.0, 300.0), 2))), None);
        assert_eq!(two_finger_tap(&mut track, 2.0, rect, None), None);
        // Wandered off: a pan, not a tap.
        let mut track = None;
        let mut moved = tap_touch(pos2(190.0, 300.0), 2);
        assert_eq!(two_finger_tap(&mut track, 1.0, rect, Some(&tap_touch(pos2(90.0, 300.0), 2))), None);
        moved.center_pos = pos2(190.0, 300.0);
        assert_eq!(two_finger_tap(&mut track, 1.1, rect, Some(&moved)), None);
        assert_eq!(two_finger_tap(&mut track, 1.2, rect, None), None);
        // Three fingers: some other gesture, and the lift after them fires nothing.
        let mut track = None;
        assert_eq!(two_finger_tap(&mut track, 1.0, rect, Some(&tap_touch(pos2(90.0, 300.0), 2))), None);
        assert_eq!(two_finger_tap(&mut track, 1.1, rect, Some(&tap_touch(pos2(90.0, 300.0), 3))), None);
        assert_eq!(two_finger_tap(&mut track, 1.2, rect, None), None);
        // No fingers ever down: nothing to lift.
        let mut track = None;
        assert_eq!(two_finger_tap(&mut track, 1.2, rect, None), None);
    }

    /// The strip's controls sit on the strip's centre line (the header's 26, plus the header's
    /// 52): the strip's scroll area used to hold a plain `horizontal` row, which sized itself to
    /// its content and pinned the controls to the strip's top edge, chip-first.
    #[test]
    fn options_strip_centers_its_controls() {
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let ctx = egui::Context::default();
        NivetaApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        top_bar_frame(&mut a, &ctx); // fonts and panel state settle on the second frame
        top_bar_frame(&mut a, &ctx);
        let mut controls = 0;
        ctx.viewport(|v| {
            for w in v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()) {
                let r = w.rect;
                let (cy, h) = (r.center().y, r.height());
                // Widgets drawn inside the strip's band, tall enough to be a control.
                if !(54.0..98.0).contains(&cy) || !(12.0..=48.0).contains(&h) {
                    continue;
                }
                assert!((cy - 76.0).abs() <= 1.5, "strip control {r:?} sits off the strip's centre");
                controls += 1;
            }
        });
        assert!(controls >= 5, "the strip draws its tool options, got {controls}");
    }

    /// One frame of the mobile top bar (header + options strip), with the painted shapes.
    fn top_bar_frame(app: &mut NivetaApp, ctx: &egui::Context) -> Vec<egui::epaint::ClippedShape> {
        let out = ctx.run_ui(
            egui::RawInput { time: Some(0.0), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 800.0))), ..Default::default() },
            |ui| {
                header(app, ui);
                options_strip(app, ui);
            },
        );
        let egui::FullOutput { mut textures_delta, shapes, .. } = out;
        textures_delta.clear();
        shapes
    }

    /// The top bar is one block, so it carries a single separator line: the options panel's,
    /// at the bottom of the strip. The header's own line and the content's hand-painted rules
    /// (which landed right under the header and through the options row) used to add dark
    /// lines inside the bar.
    #[test]
    fn mobile_top_bar_paints_one_separator_line_at_its_bottom() {
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let ctx = egui::Context::default();
        NivetaApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        top_bar_frame(&mut a, &ctx); // fonts and panel state settle on the second frame
        let separator = Tokens::get(&ctx).separator;
        let lines: Vec<f32> = top_bar_frame(&mut a, &ctx)
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::LineSegment { points: [p, q], stroke } if stroke.color == separator && (p.y - q.y).abs() < 0.5 => Some(p.y),
                _ => None,
            })
            .collect();
        // Header 52 + options strip 48: the line straddles the strip's edge at 100.
        assert_eq!(lines.len(), 1, "one separator line for the whole top bar, got {lines:?}");
        assert!((98.0..=100.5).contains(&lines[0]), "the line sits at the bottom of the top bar, got {}", lines[0]);
    }

    /// The header carries Home where undo used to be: tapping it parks the documents
    /// on the Home screen, tapping again comes back. Undo/redo moved to two-finger taps.
    #[test]
    fn header_home_button_toggles_the_home_screen() {
        use egui_kittest::Harness;
        let mut a = app();
        a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        assert!(a.ui.chrome.home.is_none());
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                // The themed fonts install after the first frame; skip until they are in.
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                header(app, ui);
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(8);
        // Touch-sized house button at the header's left edge (margin 4 + half of 48).
        let at = egui::pos2(28.0, 26.0);
        h.drag_at(at);
        h.run_steps(1);
        h.drop_at(at);
        h.run_steps(2);
        assert_eq!(h.state().ui.chrome.home, Some(1), "tapping Home parks the document there");
        h.drag_at(at);
        h.run_steps(1);
        h.drop_at(at);
        h.run_steps(2);
        assert!(h.state().ui.chrome.home.is_none(), "tapping Home again comes back");
    }

    /// Restore Original in the overflow menu is File › Revert: the file's pixels come back,
    /// and a document with edits asks first (the prompt's Revert answer continues the command).
    #[test]
    fn overflow_menu_restores_the_original_file() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let dir = std::env::temp_dir().join(format!("niveta-mobile-revert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("original.png");
        let img = niveta_codecs::Image::from_u8(16, 16, niveta_codecs::ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat(256)).unwrap();
        let png = niveta_codecs::encode(&img, niveta_codecs::Format::Png, &niveta_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&path, &png).unwrap();
        let import = |name: &str, bytes: &[u8]| niveta_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string());
        let mut a = NivetaApp::new(niveta_engine::Session::new(), crate::Services { import: Some(Box::new(import)), ..Default::default() });
        a.open_path(&path.to_string_lossy()).unwrap();
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut NivetaApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                header(app, ui);
                crate::discard_ui::show(app, ui.ctx());
            },
            a,
        );
        NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(8);
        let corner = |h: &Harness<'_, NivetaApp>| {
            let st = h.state().session.active().expect("document");
            let img = niveta_compose::thumbnail(&st.doc, 4);
            [img.pixels[0], img.pixels[1], img.pixels[2]]
        };
        let tap = |h: &mut Harness<'_, NivetaApp>, at: egui::Pos2| {
            h.drag_at(at);
            h.run_steps(1);
            h.drop_at(at);
            h.run_steps(3);
        };
        assert_eq!(corner(&h), [255, 0, 0], "the card opens the file itself");
        h.state_mut().run("edit.fill", json!({"color": "#00ff00"})).unwrap();
        h.run_steps(2);
        assert_eq!(corner(&h), [0, 255, 0], "the edit landed");
        // The icon button carries no accesskit label: the ellipsis sits at the header's right.
        tap(&mut h, egui::pos2(332.0, 26.0));
        let item = h.get_by_label("Restore Original").rect().center();
        tap(&mut h, item);
        assert!(h.state().discard.is_some(), "unsaved edits ask before they are thrown away");
        let prompt = h.get_by_label("(R)evert").rect().center();
        tap(&mut h, prompt);
        assert!(h.state().discard.is_none());
        assert_eq!(corner(&h), [255, 0, 0], "the original is back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The histogram/slider adjustment editors (Levels, Threshold, Gradient Map, Color Lookup)
    /// used to hug the left edge of the sheet: their content caps at 300 pt. A wider sheet
    /// (tablet, landscape phone) centres that column instead.
    #[test]
    fn sheet_centres_narrow_adjustment_editors() {
        use egui_kittest::kittest::Queryable;
        for (kind, probe) in [("levels", "Output Levels:"), ("threshold", "Threshold Level"), ("gradientMap", "Preset:"), ("colorLookup", "3D LUT File")] {
            let mut a = app();
            a.run("file.new", json!({"width": 64, "height": 64})).unwrap();
            a.run(&format!("layer.newAdjustmentLayer.{kind}"), json!({})).unwrap();
            a.ui.mobile.panel = Some(crate::dock::Group::Properties);
            a.ui.dock_tabs.properties = 0;
            let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).build_ui_state(
                |ui, app: &mut NivetaApp| {
                    if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    header(app, ui);
                    tool_bar(app, ui);
                    sheet(app, ui);
                    crate::canvas::document_area(app, ui);
                },
                a,
            );
            NivetaApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
            h.run_steps(8);
            // A 300 pt column on a 600 pt screen starts ~150 pt in (the scroll bar shaves a few
            // points off the body's right); the probe sits at its left.
            let content = h.query_all_by_label(probe).last().unwrap_or_else(|| panic!("{kind}: {probe}")).rect();
            assert!((content.min.x - 150.0).abs() <= 6.0, "{kind}: {probe} at {content:?}");
        }
    }
}
