//! Compact chrome for narrow screens (phones and other windows under [`MOBILE_WIDTH`]):
//! a short header, a horizontally scrolling tool bar and a bottom sheet for the panels.
//!
//! Desktop panels assume a wide window (toolbar column + 250-520 pt dock next to the
//! canvas). On a 360 pt wide phone they leave no room for the document, so below
//! [`MOBILE_WIDTH`] the app shows this chrome instead. Every button here drives the same
//! commands, tools and panel bodies as the desktop layout: nothing is a mock and nothing
//! hides a feature, it is just packed for touch.

use egui::{Align2, RichText, vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::Tool;
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

/// Content width below which the mobile chrome replaces the desktop panels. Phones
/// (360-430 pt wide) get the compact layout; 800 pt and up keeps the desktop one (the
/// Help-menu search test covers a narrow 800 pt window, so the breakpoint stays below it).
pub const MOBILE_WIDTH: f32 = 720.0;

/// Space kept above the mobile header so it clears the Android status bar, which
/// draws over the app's window. egui doesn't report the real inset, so this is the
/// standard status-bar height until the shell reads it from the OS; desktop is unaffected.
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
}

/// A two-finger gesture over the canvas: pan and pinch it, never paint with it. Returns the
/// gesture when at least two fingers are down with their centre on `rect`.
pub fn two_finger(ctx: &egui::Context, rect: egui::Rect) -> Option<egui::MultiTouchInfo> {
    ctx.input(|i| i.multi_touch()).filter(|m| m.num_touches >= 2 && rect.contains(m.center_pos))
}

/// Compact header: undo, the document title, save and an overflow menu. Everything else
/// lives one tap away in the command palette or the overflow menu, so no menu bar has to
/// fit in 360 pt.
pub fn header(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_header")
        .exact_size(52.0 + SAFE_TOP)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 8, right: 8, top: SAFE_TOP as i8, bottom: 0 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let undo_on = crate::menus::is_enabled(app, "edit.undo");
                let undo = ui.add_enabled_ui(undo_on, |ui| icons::button_with_icon_size(ui, "undo-2", 44.0, 20.0, false, tl!("Undo")));
                if undo.inner.clicked() {
                    let _ = crate::menus::invoke(app, ui.ctx(), "edit.undo", json!({}));
                }
                let title =
                    app.session.active().map(|st| format!("{}{}", st.doc.name, if st.is_dirty() { " *" } else { "" })).unwrap_or_else(|| "PhotoCraft".into());
                // Whatever the right-hand buttons take, the title gets the rest, truncated.
                let avail = (ui.available_width() - 64.0 - 44.0 - 12.0).max(48.0);
                ui.add_sized(vec2(avail, 44.0), egui::Label::new(RichText::new(title).color(t.text).size(13.0)).truncate());
                let save_on = crate::menus::is_enabled(app, "file.save");
                if ui.add_enabled_ui(save_on, |ui| widgets::secondary_button(ui, tl!("Save"), 64.0)).inner.clicked() {
                    let _ = crate::menus::invoke(app, ui.ctx(), "file.save", json!({}));
                }
                let more = icons::button(ui, "ellipsis", 44.0, false, tl!("Options"));
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(220.0);
                    menu_item(ui, app, tl!("New…"), "file.new", json!({}));
                    // Same as the Home-screen button: a build with no picker explains
                    // instead of flashing "cancelled".
                    if ui.add_enabled(crate::menus::is_enabled(app, "file.open"), egui::Button::new(tl!("Open…"))).clicked() {
                        app.open_interactive();
                        ui.close();
                    }
                    menu_item(ui, app, tl!("Save As…"), "file.saveAs", json!({}));
                    menu_item(ui, app, tl!("Redo"), "edit.redo", json!({}));
                    menu_item(ui, app, tl!("Fit on Screen"), "view.fitOnScreen", json!({}));
                    if ui.button(tl!("Search commands, tools and panels…")).clicked() {
                        app.ui.palette_open = true;
                        ui.close();
                    }
                });
            });
        });
}

fn menu_item(ui: &mut egui::Ui, app: &mut PhotocraftApp, label: &str, id: &str, params: serde_json::Value) {
    if ui.add_enabled(crate::menus::is_enabled(app, id), egui::Button::new(label)).clicked() {
        if let Err(e) = crate::menus::invoke(app, ui.ctx(), id, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        ui.close();
    }
}

/// The active tool's options, horizontally scrollable: the same controls as the desktop
/// options bar, so every tool keeps its settings on a phone.
pub fn options_strip(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_options").exact_size(48.0).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(8, 0))).show(ui, |ui| {
        egui::ScrollArea::horizontal().id_salt("mobile-options-scroll").auto_shrink([false, false]).show(ui, |ui| {
            crate::panels::options_content(app, ui);
        });
    });
}

/// Favourite tools, touch-sized and scrollable, with the panel shortcuts beside them.
/// The active tool is always in the row (prepended when it is not a favourite), so the
/// selection never silently points at a tool the user cannot see.
pub fn tool_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("mobile_tools")
        .exact_size(68.0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 12 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if icons::button_with_icon_size(ui, "grid-2x2", 44.0, 20.0, app.ui.mobile.tools_open, tl!("Tools")).clicked() {
                    app.ui.mobile.tools_open = !app.ui.mobile.tools_open;
                }
                // Fixed buttons after the scrolled row: grid, three panel shortcuts.
                let rest = (ui.available_width() - 3.0 * 48.0 - 4.0).max(64.0);
                egui::ScrollArea::horizontal().id_salt("mobile-tools-scroll").auto_shrink([false, false]).max_width(rest).show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let mut favs = favourites().to_vec();
                        if !favs.contains(&app.ui.tool) {
                            favs.insert(0, app.ui.tool);
                        }
                        for tool in favs {
                            let tip = tl!(tool.label()).to_string();
                            if icons::button_with_icon_size(ui, icons::tool_icon(tool), 44.0, 22.0, app.ui.tool == tool, &tip).clicked() {
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
                    let on = app.ui.mobile.panel == Some(group);
                    if icons::button_with_icon_size(ui, icon, 44.0, 20.0, on, name).clicked() {
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

/// The full tool grid over the canvas. Same tools as the desktop toolbar, 44 pt targets.
pub fn tool_sheet(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.mobile.tools_open {
        return;
    }
    let screen = ctx.content_rect();
    egui::Window::new(tl!("Tools"))
        .id(egui::Id::new("mobile-tools"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -84.0))
        .collapsible(false)
        .resizable(false)
        .constrain(true)
        .show(ctx, |ui| {
            ui.set_min_width((screen.width() - 48.0).clamp(280.0, 420.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Tools")).size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "x", 32.0, false, tl!("Close")).clicked() {
                        app.ui.mobile.tools_open = false;
                    }
                });
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("mobile-tools-grid").auto_shrink([false, false]).max_height((screen.height() * 0.5).clamp(200.0, 420.0)).show(
                ui,
                |ui| {
                    egui::Grid::new("mobile-tool-grid").num_columns(6).spacing(vec2(6.0, 6.0)).show(ui, |ui| {
                        for (i, tool) in Tool::ALL.iter().enumerate() {
                            let tip = tl!(tool.label()).to_string();
                            if icons::button_with_icon_size(ui, icons::tool_icon(*tool), 44.0, 22.0, app.ui.tool == *tool, &tip).clicked() {
                                app.ui.tool = *tool;
                                app.ui.mobile.tools_open = false;
                            }
                            if i % 6 == 5 {
                                ui.end_row();
                            }
                        }
                    });
                },
            );
        });
}

/// The bottom sheet: one dock group at a time, with its own height cap and scrolling, so
/// the canvas takes the room back as soon as it closes. The bodies are the desktop dock's
/// ([`crate::panels::dock_body`]), sharing its state and commands.
pub fn sheet(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(group) = app.ui.mobile.panel else { return };
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let h = (screen.height() * 0.45).clamp(260.0, 420.0);
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("mobile_sheet").exact_size(h).frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin::symmetric(8, 6))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
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
            for (g, label) in groups {
                if widgets::pill_tab(ui, label, group == g).clicked() {
                    app.ui.mobile.panel = if group == g { None } else { Some(g) };
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, "x", 32.0, false, tl!("Close")).clicked() {
                    app.ui.mobile.panel = None;
                }
            });
        });
        // The group's own tabs (Layers | Channels | Paths, …), as in the desktop dock.
        let pro = Tokens::get(ui.ctx()).pro;
        let tabs = group.tabs(pro);
        let sel = tab_index(&app.ui.dock_tabs, group).min(tabs.len().saturating_sub(1));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, tab) in tabs.iter().enumerate() {
                if widgets::pill_tab(ui, tab, sel == i).clicked() {
                    *tab_index_mut(&mut app.ui.dock_tabs, group) = i;
                }
            }
        });
        let tab = tab_index(&app.ui.dock_tabs, group).min(tabs.len().saturating_sub(1));
        if group.scrolls_itself(tab) {
            crate::panels::dock_body(app, ui, group, tab);
        } else {
            egui::ScrollArea::vertical().id_salt(("mobile-sheet", group.key())).auto_shrink([false, false]).show(ui, |ui| {
                crate::panels::dock_body(app, ui, group, tab);
            });
        }
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

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
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
        let chrome = |ui: &mut egui::Ui, app: &mut PhotocraftApp| {
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
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(6);
        assert!(is_mobile(&h.ctx));
        let r = h.state().last_canvas_rect;
        assert!(r.width() >= 340.0, "full width with no side panels: {r:?}");
        assert!(r.height() > 500.0, "canvas dominates with the sheet closed: {r:?}");
        // Open the sheet and the tool grid: everything still renders, canvas stays usable.
        h.state_mut().ui.mobile.panel = Some(crate::dock::Group::Layers);
        h.state_mut().ui.mobile.tools_open = true;
        h.run_steps(6);
        assert!(h.query_all_by_label("Save").next().is_some(), "the header keeps Save visible");
        assert!(h.query_all_by_label("Tools").next().is_some(), "the tool grid opens");
        assert_eq!(h.state().ui.mobile.panel, Some(crate::dock::Group::Layers));
        let r = h.state().last_canvas_rect;
        assert!(r.width() >= 340.0, "the sheet never squeezes the width: {r:?}");
        assert!(r.height() > 200.0, "the canvas stays usable with the sheet open: {r:?}");
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
                |ui, app: &mut PhotocraftApp| {
                    if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    sheet(app, ui);
                },
                a,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.run_steps(3);
            a = h.into_state();
            a.ui.mobile.panel = None;
            assert_eq!(a.ui.tool, Tool::Brush, "the tool survives {panel:?}");
            assert_eq!(a.session.active().unwrap().revision, rev, "the document survives {panel:?}");
            assert_eq!(a.session.journal.len(), journal, "no command ran for {panel:?}");
        }
    }

    /// The narrow start screen drops the promo: no drop hint, no Discord, no links.
    #[test]
    fn start_screen_has_no_promo_on_phones() {
        use egui_kittest::kittest::Queryable;
        for (w, promo) in [(360.0, false), (1440.0, true)] {
            let a = app();
            assert!(a.session.documents().is_empty());
            let mut h = Harness::builder().with_size(vec2(w, 800.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    crate::canvas::document_area(app, ui);
                },
                a,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.run_steps(6);
            assert_eq!(h.query_all_by_label_contains("Discord").next().is_some(), promo, "{w} pt promo");
            assert_eq!(h.query_all_by_label_contains("GitHub").next().is_some(), promo, "{w} pt links");
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
}
