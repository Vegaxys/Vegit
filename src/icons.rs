//! Small line icons drawn with the painter, so they follow the theme colours
//! (emoji glyphs from the font come in fixed colours and some are missing).

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Undo,
    Redo,
    Fetch,
    Pull,
    Push,
    Branch,
    Stash,
    Pop,
    Terminal,
    Folder,
    Refresh,
    Gear,
}

pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = Stroke::new(1.8, color);
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    let at = |x: f32, y: f32| pos2(c.x + x * r, c.y + y * r);
    match icon {
        Icon::Undo => {
            arc(p, at(0.1, 0.1), r * 0.6, 200.0, 470.0, s);
            arrow_head(p, at(-0.5, -0.1), vec2(-0.2, 1.0), r, s);
        }
        Icon::Redo => {
            // Mirror image of Undo.
            arc(p, at(-0.1, 0.1), r * 0.6, -290.0, -20.0, s);
            arrow_head(p, at(0.5, -0.1), vec2(0.2, 1.0), r, s);
        }
        Icon::Fetch | Icon::Refresh => {
            arc(p, c, r * 0.7, -60.0, 240.0, s);
            arrow_head(p, at(0.35, -0.6), vec2(1.0, 0.5), r, s);
        }
        Icon::Pull => {
            p.line_segment([at(0.0, -0.8), at(0.0, 0.4)], s);
            p.line_segment([at(-0.45, -0.05), at(0.0, 0.4)], s);
            p.line_segment([at(0.45, -0.05), at(0.0, 0.4)], s);
            p.line_segment([at(-0.7, 0.8), at(0.7, 0.8)], s);
        }
        Icon::Push => {
            p.line_segment([at(0.0, 0.5), at(0.0, -0.7)], s);
            p.line_segment([at(-0.45, -0.25), at(0.0, -0.7)], s);
            p.line_segment([at(0.45, -0.25), at(0.0, -0.7)], s);
            p.line_segment([at(-0.7, 0.8), at(0.7, 0.8)], s);
        }
        Icon::Branch => {
            p.line_segment([at(-0.4, -0.6), at(-0.4, 0.6)], s);
            p.add(Shape::CubicBezier(eframe::epaint::CubicBezierShape::from_points_stroke(
                [at(0.45, -0.4), at(0.45, 0.1), at(-0.4, 0.0), at(-0.4, 0.45)],
                false,
                Color32::TRANSPARENT,
                s,
            )));
            for q in [at(-0.4, -0.65), at(-0.4, 0.65), at(0.45, -0.5)] {
                p.circle_filled(q, r * 0.2, color);
            }
        }
        Icon::Stash | Icon::Pop => {
            // Tray with an arrow going in (stash) or out (pop).
            p.line_segment([at(-0.8, 0.2), at(-0.8, 0.8)], s);
            p.line_segment([at(-0.8, 0.8), at(0.8, 0.8)], s);
            p.line_segment([at(0.8, 0.8), at(0.8, 0.2)], s);
            if icon == Icon::Stash {
                p.line_segment([at(0.0, -0.8), at(0.0, 0.4)], s);
                p.line_segment([at(-0.35, 0.05), at(0.0, 0.4)], s);
                p.line_segment([at(0.35, 0.05), at(0.0, 0.4)], s);
            } else {
                p.line_segment([at(0.0, 0.4), at(0.0, -0.8)], s);
                p.line_segment([at(-0.35, -0.45), at(0.0, -0.8)], s);
                p.line_segment([at(0.35, -0.45), at(0.0, -0.8)], s);
            }
        }
        Icon::Terminal => {
            p.line_segment([at(-0.7, -0.5), at(-0.2, 0.0)], s);
            p.line_segment([at(-0.2, 0.0), at(-0.7, 0.5)], s);
            p.line_segment([at(0.0, 0.6), at(0.7, 0.6)], s);
        }
        Icon::Gear => {
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * r * 0.55, c + d * r * 0.9], Stroke::new(2.2, color));
            }
            p.circle_stroke(c, r * 0.55, s);
            p.circle_stroke(c, r * 0.2, s);
        }
        Icon::Folder => {
            let pts = vec![at(-0.8, -0.6), at(-0.2, -0.6), at(0.0, -0.4), at(0.8, -0.4), at(0.8, 0.7), at(-0.8, 0.7), at(-0.8, -0.6)];
            p.add(Shape::line(pts, s));
        }
    }
}

fn arc(p: &egui::Painter, c: Pos2, radius: f32, from_deg: f32, to_deg: f32, s: Stroke) {
    let steps = 20;
    let pts: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = (from_deg + (to_deg - from_deg) * i as f32 / steps as f32).to_radians();
            pos2(c.x + radius * a.cos(), c.y + radius * a.sin())
        })
        .collect();
    p.add(Shape::line(pts, s));
}

fn arrow_head(p: &egui::Painter, tip: Pos2, dir: egui::Vec2, r: f32, s: Stroke) {
    let d = dir.normalized();
    let n = vec2(-d.y, d.x);
    let len = r * 0.4;
    p.line_segment([tip, tip - d * len + n * len * 0.7], s);
    p.line_segment([tip, tip - d * len - n * len * 0.7], s);
}

/// A toolbar button with the icon above its label, like a classic git client toolbar.
pub fn tool_button(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool, text: Color32, hover: Color32) -> egui::Response {
    let size = vec2(52.0, 44.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(rect, egui::CornerRadius::same(12), hover);
    }
    let color = if enabled { text } else { text.gamma_multiply(0.35) };
    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 14.0), vec2(16.0, 16.0));
    paint(p, icon_rect, icon, color);
    p.text(pos2(rect.center().x, rect.bottom() - 9.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.5), color);
    resp
}

/// Small square icon button.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, text: Color32, fill: Color32, hover: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::same(10), if resp.hovered() { hover } else { fill });
    paint(p, rect.shrink(9.0), icon, text);
    resp
}
