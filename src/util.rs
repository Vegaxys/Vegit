//! Small drawing and formatting helpers shared by the panels.

use chrono::{Local, TimeZone};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Sense, pos2, vec2};

use crate::git::{FileChange, FileState};
use crate::icons::{self, Icon};
use crate::theme::Palette;

/// Round avatar: the author's profile picture when it is available, otherwise initials
/// on a colour that stays the same for each e-mail address.
pub fn avatar(p: &egui::Painter, c: Pos2, r: f32, name: &str, email: &str) {
    if let Some(tex) = crate::avatars::texture(p.ctx(), email) {
        let rect = egui::Rect::from_center_size(c, vec2(r * 2.0, r * 2.0));
        let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        // A rounded rectangle as wide as it is round is a circle: the picture gets clipped to it.
        p.add(egui::epaint::RectShape::filled(rect, CornerRadius::same(r.round() as u8), Color32::WHITE).with_texture(tex, uv));
        return;
    }
    let hash = email.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    let hue = (hash % 360) as f32 / 360.0;
    let fill: Color32 = egui::ecolor::Hsva::new(hue, 0.45, 0.78, 1.0).into();
    p.circle_filled(c, r, fill);
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    p.text(c, Align2::CENTER_CENTER, initials, FontId::proportional(r * 0.95), Color32::from_rgb(0x22, 0x1e, 0x2a));
}

pub fn rel_time(t: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let d = (now - t).max(0);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        86_400..=2_591_999 => {
            let n = d / 86_400;
            if n == 1 { "yesterday".into() } else { format!("{n} days ago") }
        }
        _ => Local.timestamp_opt(t, 0).single().map(|dt| dt.format("%d %b %Y").to_string()).unwrap_or_default(),
    }
}

pub fn abs_time(t: i64) -> String {
    Local
        .timestamp_opt(t, 0)
        .single()
        .map(|dt| dt.format("%A %d %B %Y, %H:%M").to_string())
        .unwrap_or_default()
}

pub fn state_color(state: FileState, pal: &Palette) -> Color32 {
    match state {
        FileState::Added | FileState::Untracked => pal.added,
        FileState::Deleted => pal.removed,
        FileState::Conflicted => pal.warn,
        FileState::Renamed | FileState::TypeChange => pal.lane(5),
        FileState::Modified => pal.lane(3),
    }
}

/// Height of a file line; fixed so long lists can be virtualised.
pub const FILE_ROW_H: f32 = 28.0;

/// Small coloured mark for a file's change: pencil (modified), plus (new), minus
/// (deleted), arrow (renamed), exclamation (conflict).
pub fn status_icon(p: &egui::Painter, c: Pos2, state: FileState, color: Color32) {
    let icon = match state {
        FileState::Added | FileState::Untracked => Icon::StatusAdded,
        FileState::Deleted => Icon::StatusDeleted,
        FileState::Renamed | FileState::TypeChange => Icon::StatusRenamed,
        FileState::Conflicted => Icon::StatusConflict,
        FileState::Modified => Icon::StatusModified,
    };
    icons::paint(p, egui::Rect::from_center_size(c, vec2(15.0, 15.0)), icon, color);
}

/// An icon followed by a text, hoverable as one piece (for a tooltip).
pub fn icon_label(ui: &mut egui::Ui, icon: Icon, side: f32, text: egui::RichText, color: Color32) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add(icons::image(ui.ctx(), icon, side, color));
        ui.label(text);
    })
    .response
}

/// Single-line search box with a magnifier drawn inside, before the text.
pub fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32, pal: &Palette) -> egui::Response {
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .margin(egui::Margin { left: 28, right: 6, top: 4, bottom: 4 })
            .desired_width(width),
    );
    let r = resp.rect;
    icons::paint(ui.painter(), egui::Rect::from_center_size(pos2(r.left() + 15.0, r.center().y), vec2(15.0, 15.0)), Icon::Search, pal.muted);
    resp
}

/// A clickable file line: change mark, then the path with its folder in a quieter colour
/// and the file name in full colour. In tree view `indent` shifts it under its folder and
/// only the name is shown.
pub fn file_row(ui: &mut egui::Ui, f: &FileChange, pal: &Palette, selected: bool, button: Option<&str>, indent: f32) -> (egui::Response, bool) {
    let (full, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
    let p = ui.painter_at(full);
    let rect = full.with_min_x(full.left() + indent);
    // Pointer anywhere on the row, including over its button (which takes the hover itself).
    let hovered = ui.rect_contains_pointer(full);
    if selected {
        p.rect_filled(rect.shrink2(vec2(0.0, 1.0)), CornerRadius::same(5), pal.selected);
    } else if hovered {
        p.rect_filled(rect.shrink2(vec2(0.0, 1.0)), CornerRadius::same(5), pal.hover);
    }
    status_icon(&p, pos2(rect.left() + 14.0, rect.center().y), f.state, state_color(f.state, pal));

    let (dir, name) = match f.path.rfind('/') {
        Some(i) => (&f.path[..=i], &f.path[i + 1..]),
        None => ("", f.path.as_str()),
    };
    let font = FontId::proportional(13.0);
    let mut x = rect.left() + 30.0;
    if !dir.is_empty() && indent == 0.0 {
        let dir_g = p.layout_no_wrap(dir.to_string(), font.clone(), pal.muted);
        let w = dir_g.size().x;
        p.galley(pos2(x, rect.center().y - dir_g.size().y / 2.0), dir_g, pal.muted);
        x += w;
    }
    let name_g = p.layout_no_wrap(name.to_string(), font, pal.text);
    p.galley(pos2(x, rect.center().y - name_g.size().y / 2.0), name_g, pal.text);

    let clicked_button = match button {
        Some(label) if hovered => hover_pill(ui, full, label, pal, resp.id.with("pill")),
        _ => false,
    };
    let resp = resp.on_hover_text(f.old_path.as_ref().map_or(f.path.clone(), |o| format!("{o} → {}", f.path)));
    (resp, clicked_button)
}

/// Small action pill drawn inside the right end of a hovered row. It stays within the
/// row's height and away from the scrollbar, so hovering never makes anything bigger.
pub fn hover_pill(ui: &mut egui::Ui, row: egui::Rect, label: &str, pal: &Palette, id: egui::Id) -> bool {
    let font = FontId::proportional(11.5);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), pal.text).size().x + 18.0;
    let r = egui::Rect::from_min_size(pos2(row.right() - w - 14.0, row.center().y - 10.0), vec2(w, 20.0));
    let resp = ui.interact(r, id, Sense::click());
    let fill = if resp.hovered() { pal.accent } else { pal.accent.gamma_multiply(0.3) };
    let fg = if resp.hovered() { pal.accent_text } else { pal.text };
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(10), fill);
    p.text(r.center(), Align2::CENTER_CENTER, label, font, fg);
    resp.clicked()
}

/// A rounded block inside a tile (commit composer, commit summary, home cards).
pub fn card(pal: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.card)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
}

/// Gap between bento tiles, in points.
pub const GAP: i8 = 4;

/// One bento tile: a big rounded block standing on the canvas.
pub fn tile(pal: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.panel)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(8))
}

/// Draws a tile that fills all the space it is given.
pub fn fill_tile<R>(ui: &mut egui::Ui, pal: &Palette, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    tile(pal)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            add(ui)
        })
        .inner
}

/// A filled accent button (main action of a panel).
pub fn accent_button(text: impl Into<String>, pal: &Palette) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.into()).color(pal.accent_text).strong())
        .fill(pal.accent)
        .corner_radius(CornerRadius::same(8))
}

/// Two or more choices shown as one pill with the active part filled.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, pal: &Palette, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::new().fill(pal.canvas).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // Inside a right-aligned row items are laid out from the right: reverse them so
        // the choices still read left to right.
        let reversed = ui.layout().prefer_right_to_left();
        ui.horizontal(|ui| {
            let ordered: Vec<&(T, &str)> = if reversed { options.iter().rev().collect() } else { options.iter().collect() };
            for (v, label) in ordered {
                let on = *value == *v;
                let text = egui::RichText::new(*label).size(12.0).color(if on { pal.text } else { pal.muted });
                let b = egui::Button::new(text)
                    .fill(if on { pal.card } else { Color32::TRANSPARENT })
                    .corner_radius(CornerRadius::same(8))
                    .min_size(vec2(0.0, 22.0));
                if ui.add(b).clicked() && !on {
                    *value = *v;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// Gives menu entries room on the left: egui's menu style squeezes button padding to 2 px,
/// so the hover highlight touched the text. Call it first in every menu.
pub fn menu_pad(ui: &mut egui::Ui) {
    ui.spacing_mut().button_padding = vec2(12.0, 5.0);
    ui.spacing_mut().item_spacing.y = 2.0;
}

pub fn section_title(ui: &mut egui::Ui, text: &str, pal: &Palette) {
    ui.label(egui::RichText::new(text).size(11.5).color(pal.muted).strong());
}
