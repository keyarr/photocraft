//! Compact chrome for narrow screens (phones and other windows under [`MOBILE_WIDTH`]):
//! a short header, a horizontally scrolling tool bar and a bottom sheet for the panels.
//!
//! Desktop panels assume a wide window (toolbar column + 250-520 pt dock next to the
//! canvas). On a 360 pt wide phone they leave no room for the document, so below
//! [`MOBILE_WIDTH`] the app shows this chrome instead. Every button here drives the same
//! commands, tools and panel bodies as the desktop layout: nothing is a mock and nothing
//! hides a feature, it is just packed for touch.

use egui::{Align2, Rect, RichText, Sense, pos2, vec2};
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
    /// Filter text for the tool list (cleared whenever the sheet opens or a tool is picked).
    pub tools_filter: String,
}

/// A two-finger gesture over the canvas: pan and pinch it, never paint with it. Returns the
/// gesture when at least two fingers are down with their centre on `rect`.
pub fn two_finger(ctx: &egui::Context, rect: egui::Rect) -> Option<egui::MultiTouchInfo> {
    ctx.input(|i| i.multi_touch()).filter(|m| m.num_touches >= 2 && rect.contains(m.center_pos))
}

/// Compact header: undo, the document title, export and an overflow menu. Everything else
/// lives one tap away in the command palette or the overflow menu, so no menu bar has to
/// fit in 360 pt. With no document open only the title and the overflow menu stay:
/// undo and export need an editable document.
pub fn header(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_header")
        .exact_size(52.0 + SAFE_TOP)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 8, right: 8, top: SAFE_TOP as i8, bottom: 0 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let has_doc = app.session.active().is_some();
                if has_doc {
                    let undo_on = crate::menus::is_enabled(app, "edit.undo");
                    let undo = ui.add_enabled_ui(undo_on, |ui| icons::button_with_icon_size(ui, "undo-2", 48.0, 22.0, false, tl!("Undo")));
                    if undo.inner.clicked() {
                        let _ = crate::menus::invoke(app, ui.ctx(), "edit.undo", json!({}));
                    }
                }
                let title =
                    app.session.active().map(|st| format!("{}{}", st.doc.name, if st.is_dirty() { " *" } else { "" })).unwrap_or_else(|| "PhotoCraft".into());
                // Whatever the right-hand buttons take, the title gets the rest, truncated.
                let right_reserve = if has_doc { 64.0 } else { 0.0 } + 48.0 + 12.0;
                let avail = (ui.available_width() - right_reserve).max(48.0);
                ui.add_sized(vec2(avail, 48.0), egui::Label::new(RichText::new(title).color(t.text).size(13.0)).truncate());
                if has_doc {
                    // No save picker on phones: saving happens automatically
                    // (autosave + on exit), so the header offers Export instead.
                    let export_on = crate::menus::is_enabled(app, "file.export.exportAs");
                    if ui.add_enabled_ui(export_on, |ui| widgets::secondary_button(ui, tl!("Export"), 64.0)).inner.clicked() {
                        crate::mobile_export::open(app);
                    }
                }
                let more = icons::button(ui, "ellipsis", 48.0, false, tl!("Options"));
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(280.0);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    menu_item(ui, app, tl!("New…"), "file.new", json!({}));
                    // Same as the Home-screen button: a build with no picker explains
                    // instead of flashing "cancelled".
                    if ui
                        .add_enabled(crate::menus::is_enabled(app, "file.open"), egui::Button::new(tl!("Open…")).min_size(vec2(280.0, 44.0)))
                        .clicked()
                    {
                        app.open_interactive();
                        ui.close();
                    }
                    menu_item(ui, app, tl!("Save As…"), "file.saveAs", json!({}));
                    menu_item(ui, app, tl!("Redo"), "edit.redo", json!({}));
                    menu_item(ui, app, tl!("Fit on Screen"), "view.fitOnScreen", json!({}));
                    if ui.add_sized(vec2(280.0, 44.0), egui::Button::new(tl!("Search commands, tools and panels…"))).clicked() {
                        app.ui.palette_open = true;
                        ui.close();
                    }
                });
            });
        });
}

fn menu_item(ui: &mut egui::Ui, app: &mut PhotocraftApp, label: &str, id: &str, params: serde_json::Value) {
    if ui.add_enabled(crate::menus::is_enabled(app, id), egui::Button::new(label).min_size(vec2(280.0, 44.0))).clicked() {
        if let Err(e) = crate::menus::invoke(app, ui.ctx(), id, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        ui.close();
    }
}

/// The active tool's options, horizontally scrollable: the same controls
/// as the desktop options bar, so every tool keeps its settings on a phone.
/// The tool itself is already marked in the bottom bar, so no icon or name here.
pub fn options_strip(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_options").exact_size(48.0).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(8, 0))).show(ui, |ui| {
        let rest = ui.available_width().max(80.0);
        egui::ScrollArea::horizontal().id_salt("mobile-options-scroll").auto_shrink([false, false]).max_width(rest).show(ui, |ui| {
            crate::panels::options_content(app, ui);
            // Right padding inside the scroll: the last field (Opacity/Flow) ended
            // exactly at the panel edge and read as clipped.
            ui.add_space(12.0);
        });
    });
}

/// Favourite tools, touch-sized and scrollable, with the panel shortcuts beside them.
/// The active tool is always in the row (prepended when it is not a favourite), so the
/// selection never silently points at a tool the user cannot see. Hidden with no
/// document open: the start screen has no editing context.
pub fn tool_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
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
                if icons::button_with_icon_size(ui, "grid-2x2", 48.0, 22.0, app.ui.mobile.tools_open, tl!("Tools")).clicked() {
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
                            favs.insert(0, app.ui.tool);
                        }
                        let held_id = egui::Id::new("mobile-fav-held");
                        if ui.input(|i| i.pointer.any_pressed()) {
                            ui.data_mut(|d| d.remove::<Tool>(held_id));
                        }
                        for tool in favs {
                            let tip = tl!(tool.label()).to_string();
                            let resp = icons::button_with_icon_size(ui, icons::tool_icon(tool), 48.0, 24.0, app.ui.tool == tool, &tip);
                            // Touch has no hover: a press-and-hold names the tool without selecting it.
                            let held_for = resp.is_pointer_button_down_on().then(|| ui.input(|i| i.pointer.press_start_time().map(|t0| i.time - t0))).flatten();
                            if let Some(seconds) = held_for
                                && seconds < 0.35
                            {
                                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(0.35 - seconds));
                            }
                            let long_press = held_for.is_some_and(|seconds| seconds >= 0.35);
                            if long_press {
                                ui.data_mut(|d| d.insert_temp(held_id, tool));
                                ui.ctx().request_repaint();
                            }
                            if resp.clicked() && ui.data(|d| d.get_temp::<Tool>(held_id)) != Some(tool) {
                                app.ui.tool = tool;
                            }
                            if ui.data(|d| d.get_temp::<Tool>(held_id)) == Some(tool) {
                                egui::Area::new(egui::Id::new(("mobile-fav-tip", tool.label())))
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
                        }
                    });
                });
                for (icon, name, group) in [
                    ("palette", tl!("Color"), crate::dock::Group::Color),
                    ("sliders-horizontal", tl!("Properties"), crate::dock::Group::Properties),
                    ("layers", tl!("Layers"), crate::dock::Group::Layers),
                ] {
                    // Panel shortcuts use the quiet rail style on purpose: the accent fill
                    // is reserved for the active canvas tool above, so brush + open panel
                    // never read as two selected tools.
                    let on = app.ui.mobile.panel == Some(group);
                    if icons::rail_button(ui, icon, 48.0, on, name).clicked() {
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

/// The full tool list over the canvas: every tool with its name, in desktop-toolbar
/// groups, with a filter. The old 6-column icon grid was unreadable on touch (no hover
/// ever shows the tooltip), so each card names its tool. No new `tl!` literals:
/// everything here reuses strings the catalogs already translate.
pub fn tool_sheet(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.mobile.tools_open || app.session.active().is_none() {
        return;
    }
    let screen = ctx.content_rect();
    let list_w = (screen.width() - 48.0).clamp(280.0, 420.0);
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
            // Custom header: title left, X on the same row right. The default title
            // bar plus a second row with a lone X wasted a full row of height.
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
                vec2(avail, 48.0),
                egui::TextEdit::singleline(&mut app.ui.mobile.tools_filter).hint_text(tl!("Search commands, tools and panels…")).desired_width(f32::INFINITY).margin(vec2(12.0, 12.0)),
            );
            ui.add_space(8.0);
            // Fixed width from the window size: deriving it from available_width
            // inside the grid fed the grid's own sizing back into itself and the
            // window blew up past the screen (only its bottom showed on-device).
            let card_w = ((list_w - 32.0 - 8.0) / 2.0).clamp(48.0, 200.0);
            egui::ScrollArea::vertical().id_salt("mobile-tools-list").auto_shrink([false, false]).max_height((screen.height() * 0.5).clamp(200.0, 420.0)).show(
                ui,
                |ui| {
                    // Top/bottom breathing room: first/last cards were clipped by the
                    // scroll viewport edge.
                    ui.add_space(16.0);
                    let q = app.ui.mobile.tools_filter.trim().to_lowercase();
                    let sections = crate::panels::visible_sections(&app.session.prefs().toolbar.hidden);
                    let tools: Vec<Tool> = sections
                        .iter()
                        .flat_map(|section| section.iter().flat_map(|(_, slot)| slot.iter().copied()))
                        .filter(|tool| q.is_empty() || tl!(tool.label()).to_lowercase().contains(&q))
                        .collect();
                    if tools.is_empty() {
                        ui.label(RichText::new(tl!("No matching commands")).color(Tokens::get(ui.ctx()).text_faint));
                        return;
                    }
                    egui::Grid::new("mobile-tools-grid").num_columns(2).spacing([8.0, 8.0]).show(ui, |ui| {
                        for (i, tool) in tools.iter().enumerate() {
                            tool_card(app, ui, *tool, card_w);
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                        if tools.len() % 2 == 1 {
                            ui.end_row();
                        }
                    });
                    ui.add_space(16.0);
                },
            );
        });
}

/// One touch card: 24px icon over the short name ("Mover", not "Ferramenta Mover").
/// 48x68pt minimum, two columns show 8-10 tools without immediate scrolling.
/// `w` is fixed by the caller: measuring it per cell breaks the grid's sizing.
fn tool_card(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool, w: f32) {
    let t = Tokens::get(ui.ctx());
    let full = tl!(tool.label()).to_string();
    let short = Tool::short_display(tl!(tool.label())).to_string();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 68.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, full.clone()));
    let tint = icons::button_chrome(ui, r, app.ui.tool == tool, resp.hovered());
    icons::paint(ui, Rect::from_center_size(pos2(r.center().x, r.top() + 24.0), vec2(32.0, 32.0)), icons::tool_icon(tool), 24.0, tint);
    let fg = if app.ui.tool == tool { t.accent_text } else { t.text };
    let mut job = egui::text::LayoutJob::simple_singleline(short, egui::FontId::proportional(12.5), fg);
    job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 12.0).max(1.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.top() + 44.0), g, fg);
    if resp.on_hover_text(full).clicked() {
        app.ui.tool = tool;
        app.ui.mobile.tools_open = false;
        app.ui.mobile.tools_filter.clear();
    }
}

/// The bottom sheet: one dock group at a time, with its own height cap and scrolling, so
/// the canvas takes the room back as soon as it closes. The bodies are the desktop dock's
/// ([`crate::panels::dock_body`]), sharing its state and commands.
pub fn sheet(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.session.active().is_none() {
        return;
    }
    let Some(group) = app.ui.mobile.panel else { return };
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let h = (screen.height() * 0.45).clamp(260.0, 420.0);
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("mobile_sheet").exact_size(h).frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin { left: 12, right: 12, top: 6, bottom: 6 })).show(ui, |outer| {
        // The contents live in a child pinned to the sheet's rect, clipped at its bottom.
        // A bottom panel whose content runs past `exact_size` keeps its bottom edge but
        // reports its top as `overflowed bottom - size`, so the canvas grew down by the
        // overflow and painted over the group pills (on-device: ppp 2.57, pt-BR labels).
        let body = outer.max_rect();
        let clip = outer.clip_rect();
        let mut inner = outer.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        inner.set_clip_rect(Rect::from_min_max(pos2(clip.min.x, body.min.y.max(clip.min.y)), pos2(clip.max.x, body.max.y.min(clip.max.y))));
        let ui = &mut inner;
        // Fixed 40pt header row: the 24pt pills and the close button share one center
        // line. A bare `horizontal` top-aligns them (X sat below the baseline).
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
            // The pills plus Close don't fit in 360 pt: scroll them instead of
            // clipping Navigator and Close off-screen. Shrunk to the content: an
            // unshrunk horizontal scroll takes the whole panel height and pushes
            // the tabs and the body out from under the canvas.
            let rest = (row_w - 40.0 - ui.spacing().item_spacing.x).max(64.0);
            egui::ScrollArea::horizontal()
                .id_salt("mobile-sheet-groups")
                .auto_shrink([true, true])
                .max_width(rest)
                // No scrollbar: when the pills overflow it cut through their middles.
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (g, label) in groups {
                        if widgets::pill_tab(ui, label, group == g).clicked() {
                            app.ui.mobile.panel = if group == g { None } else { Some(g) };
                        }
                    }
                });
            });
            if icons::button(ui, "x", 32.0, false, tl!("Close")).clicked() {
                app.ui.mobile.panel = None;
            }
        });
        // The group's own tabs (Layers | Channels | Paths, …), as in the desktop dock.
        // This segmented control is the only "Properties" label kept: no extra header below it.
        // The Color group's own "Color" tab duplicates the group pill above it, so the
        // sheet only offers Swatches | Gradients | Patterns (its content stays the body).
        let pro = Tokens::get(ui.ctx()).pro;
        let tabs = group.tabs(pro);
        let sel = tab_index(&app.ui.dock_tabs, group).min(tabs.len().saturating_sub(1));
        // Same as above: one row, so the body keeps the rest of the sheet.
        // Panel tabs scroll instead of clipping ("Navegad" cutoff on 360pt).
        // Shrunk to the content, like the group row above.
        egui::ScrollArea::horizontal()
            .id_salt(("mobile-sheet-tabs", group.key()))
            .auto_shrink([true, true])
            // Same as the group pills above: no scrollbar through the tabs.
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (i, tab) in tabs.iter().enumerate() {
                        if group == crate::dock::Group::Color && *tab == "Color" {
                            continue;
                        }
                        if widgets::pill_tab(ui, tab, sel == i).clicked() {
                            *tab_index_mut(&mut app.ui.dock_tabs, group) = i;
                        }
                    }
                });
            });
        let tab = tab_index(&app.ui.dock_tabs, group).min(tabs.len().saturating_sub(1));
        if group.scrolls_itself(tab) {
            crate::panels::dock_body(app, ui, group, tab);
        } else {
            egui::ScrollArea::vertical().id_salt(("mobile-sheet", group.key())).auto_shrink([false, false]).show(ui, |ui| {
                // Top padding: the first size fields started at the scroll origin and
                // read as cut in half by the tab row above.
                ui.add_space(8.0);
                crate::panels::dock_body(app, ui, group, tab);
                ui.add_space(8.0);
            });
        }
        // Report exactly the sheet's rect, never the overflow.
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
        let chrome = |ui: &mut egui::Ui, app: &mut PhotocraftApp| {
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
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
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

    /// The home screen carries no promo at any width: no Discord, no links.
    #[test]
    fn start_screen_has_no_promo_on_phones() {
        use egui_kittest::kittest::Queryable;
        for w in [360.0, 1440.0] {
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
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let ctx = ui.ctx().clone();
                tool_sheet(app, &ctx);
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        for probe in ["Brush Tool", "Clone Stamp Tool", "Horizontal Type Tool", "Zoom Tool"] {
            assert!(h.query_all_by_label_contains(probe).next().is_some(), "the sheet names {probe}");
        }
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
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let ctx = ui.ctx().clone();
                tool_sheet(app, &ctx);
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
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
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let ctx = ui.ctx().clone();
                tool_sheet(app, &ctx);
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
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
            |ui, app: &mut PhotocraftApp| {
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
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(8);
        for probe in ["Add an adjustment", "Brightness/Contrast", "Levels"] {
            let n = h.query_all_by_label_contains(probe).next().unwrap_or_else(|| panic!("the adjustments body renders {probe}"));
            assert!(n.rect().min.y >= 0.0 && n.rect().max.y <= 800.0, "{probe} is inside the window: {:?}", n.rect());
        }
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
            |ui, app: &mut PhotocraftApp| {
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
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
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
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                options_strip(app, ui);
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(6);
        assert!(h.query_all_by_label_contains("Clone Stamp Tool").next().is_none(), "the strip no longer repeats the active tool");
        assert!(h.query_all_by_label_contains("Aligned").next().is_some(), "the Clone Stamp options stay");
    }

    /// With no document open the strip draws nothing: there is no tool context.
    #[test]
    fn options_strip_hides_without_a_document() {
        use egui_kittest::kittest::Queryable;
        let a = app();
        assert!(a.session.active().is_none());
        let mut h = Harness::builder().with_size(vec2(360.0, 800.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                options_strip(app, ui);
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
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
            let chrome = |ui: &mut egui::Ui, app: &mut PhotocraftApp| {
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
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
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
}
