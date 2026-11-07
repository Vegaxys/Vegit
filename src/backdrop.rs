//! Halftone background behind the bento tiles.
//!
//! A regular grid of cells covers the window. Each cell samples a gradient field at its
//! centre and draws a graphic (dot, ring, square, diamond, line, cross…) whose size follows
//! that value, which is what a halftone print does. The linear and radial gradients span
//! the whole window (left to right, top to bottom, corner to corner, centre to corners);
//! only the noise repeats, every [`PERIOD`] cells. The pattern is still: it is drawn
//! again only when something else repaints the window, so it costs no CPU when idle.
//! Colour: the theme's accent at low strength over the canvas.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

/// The noise repeats every this many cells: the tile of the pattern.
const PERIOD: f32 = 32.0;

/// The graphic drawn in each halftone cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Graphic {
    /// A graphic and a gradient picked at random at each launch.
    #[default]
    Random,
    Off,
    Dot,
    Ring,
    Square,
    Diamond,
    Line,
    Cross,
    Plus,
    Triangle,
}

impl Graphic {
    pub const KINDS: [Graphic; 8] = [Self::Dot, Self::Ring, Self::Square, Self::Diamond, Self::Line, Self::Cross, Self::Plus, Self::Triangle];
    pub const ALL: [Graphic; 10] = [
        Self::Random,
        Self::Off,
        Self::Dot,
        Self::Ring,
        Self::Square,
        Self::Diamond,
        Self::Line,
        Self::Cross,
        Self::Plus,
        Self::Triangle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Random => "Random at each launch",
            Self::Off => "None",
            Self::Dot => "Dots",
            Self::Ring => "Circles",
            Self::Square => "Squares",
            Self::Diamond => "Diamonds",
            Self::Line => "Lines",
            Self::Cross => "Crosses (×)",
            Self::Plus => "Plus signs (+)",
            Self::Triangle => "Triangles",
        }
    }
}

/// How the halftone samples its size: the shape of the underlying gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Gradient {
    Horizontal,
    Vertical,
    #[default]
    Diagonal,
    Radial,
    Noise,
}

impl Gradient {
    pub const ALL: [Gradient; 5] = [Self::Horizontal, Self::Vertical, Self::Diagonal, Self::Radial, Self::Noise];

    pub fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "Horizontal gradient",
            Self::Vertical => "Vertical gradient",
            Self::Diagonal => "Diagonal gradient",
            Self::Radial => "Radial gradient",
            Self::Noise => "Noise",
        }
    }
}

/// What to draw, after "Random" was resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub graphic: Graphic,
    pub gradient: Gradient,
    /// 0..1, how visible.
    pub strength: f32,
    /// Size of a cell, in points.
    pub cell: f32,
    /// Seed of the noise gradient.
    pub seed: u64,
}

/// A graphic, a gradient and a noise seed for this launch.
pub fn launch_pick() -> (Graphic, Gradient, u64) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
    let g = Graphic::KINDS[(hash(t, 1) % Graphic::KINDS.len() as u64) as usize];
    let d = Gradient::ALL[(hash(t, 2) % Gradient::ALL.len() as u64) as usize];
    (g, d, hash(t, 3))
}

fn hash(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise on a lattice that wraps every `n` points, so it tiles over one period.
fn periodic_noise(u: f32, v: f32, n: i64, seed: u64) -> f32 {
    let (x, y) = (u * n as f32, v * n as f32);
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (smooth(x - x0 as f32), smooth(y - y0 as f32));
    let at = |i: i64, j: i64| (hash(i.rem_euclid(n) as u64 ^ (seed << 16), j.rem_euclid(n) as u64) % 10_000) as f32 / 10_000.0;
    let a = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * fx;
    let b = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * fx;
    a + (b - a) * fy
}

/// The gradient's value (0..1) at `at`, a point of the window `area`. Horizontal goes from
/// the left edge (0) to the right edge (1), vertical from top to bottom, diagonal from the
/// top-left corner to the bottom-right one, radial from the centre (1) to the corners (0).
/// Noise tiles every `period` points instead.
fn field(g: Gradient, at: Pos2, area: Rect, period: f32, seed: u64) -> f32 {
    let (x, y) = (at.x - area.left(), at.y - area.top());
    let (w, h) = (area.width().max(1.0), area.height().max(1.0));
    let t = match g {
        Gradient::Horizontal => x / w,
        Gradient::Vertical => y / h,
        // Position along the corner-to-corner line, whatever the window's proportions.
        Gradient::Diagonal => (x * w + y * h) / (w * w + h * h),
        Gradient::Radial => 1.0 - (at - area.center()).length() / (area.size().length() / 2.0),
        Gradient::Noise => {
            let (u, v) = (x / period, y / period);
            let n = periodic_noise(u, v, 4, seed) * 0.65 + periodic_noise(u, v, 8, seed ^ 0xABCD) * 0.35;
            // Stretch the middle range so both empty and full areas show up.
            (n - 0.5) * 1.8 + 0.5
        }
    };
    t.clamp(0.0, 1.0) // the last row and column of cells can overhang the window
}

/// Paints the canvas and the halftone over `rect`.
pub fn paint(p: &egui::Painter, rect: Rect, look: &Look, canvas: Color32, accent: Color32) {
    p.rect_filled(rect, CornerRadius::ZERO, canvas);
    if matches!(look.graphic, Graphic::Off | Graphic::Random) || look.strength <= 0.0 {
        return;
    }
    let ink = accent.gamma_multiply((0.04 + 0.26 * look.strength).clamp(0.0, 1.0));
    let cell = look.cell.clamp(8.0, 48.0);
    let period = cell * PERIOD;
    let p = p.with_clip_rect(rect);
    let cols = (rect.width() / cell).ceil() as i64 + 1;
    let rows = (rect.height() / cell).ceil() as i64 + 1;
    let half = cell / 2.0;
    for j in 0..rows {
        for i in 0..cols {
            let c = pos2(rect.left() + i as f32 * cell + half, rect.top() + j as f32 * cell + half);
            let amount = field(look.gradient, c, rect, period, look.seed);
            if amount < 0.06 {
                continue;
            }
            draw(&p, look.graphic, c, cell, amount, ink);
        }
    }
}

fn draw(p: &egui::Painter, g: Graphic, c: Pos2, cell: f32, a: f32, ink: Color32) {
    let half = cell / 2.0;
    match g {
        Graphic::Dot => {
            p.circle_filled(c, half * 0.92 * a, ink);
        }
        Graphic::Ring => {
            p.circle_stroke(c, half * 0.85 * a, Stroke::new((1.0 + a * 1.5).min(half * 0.5), ink));
        }
        Graphic::Square => {
            let s = cell * 0.9 * a;
            p.rect_filled(Rect::from_center_size(c, vec2(s, s)), CornerRadius::ZERO, ink);
        }
        Graphic::Diamond => {
            let r = half * 0.95 * a;
            p.add(Shape::convex_polygon(vec![c - vec2(r, 0.0), c - vec2(0.0, r), c + vec2(r, 0.0), c + vec2(0.0, r)], ink, Stroke::NONE));
        }
        Graphic::Line => {
            // Horizontal bars whose thickness follows the field, like engraved halftone.
            let h = cell * 0.85 * a;
            p.rect_filled(Rect::from_center_size(c, vec2(cell + 0.5, h)), CornerRadius::ZERO, ink);
        }
        Graphic::Cross => {
            let d = half * 0.8 * a;
            let s = Stroke::new(1.0 + a * 2.0, ink);
            p.line_segment([c - vec2(d, d), c + vec2(d, d)], s);
            p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], s);
        }
        Graphic::Plus => {
            let d = half * 0.9 * a;
            let s = Stroke::new(1.0 + a * 2.2, ink);
            p.line_segment([c - vec2(d, 0.0), c + vec2(d, 0.0)], s);
            p.line_segment([c - vec2(0.0, d), c + vec2(0.0, d)], s);
        }
        Graphic::Triangle => {
            let r = half * 0.95 * a;
            p.add(Shape::convex_polygon(vec![c + vec2(0.0, -r), c + vec2(r * 0.87, r * 0.5), c + vec2(-r * 0.87, r * 0.5)], ink, Stroke::NONE));
        }
        Graphic::Random | Graphic::Off => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 1600.0, y: 900.0 } };
    const PERIOD_PX: f32 = 16.0 * PERIOD;

    fn at(g: Gradient, x: f32, y: f32) -> f32 {
        field(g, pos2(x, y), AREA, PERIOD_PX, 42)
    }

    /// The linear gradients run once across the whole window, from one edge (or corner)
    /// to the opposite one, instead of repeating in tiles.
    #[test]
    fn linear_gradients_span_the_window() {
        assert_eq!(at(Gradient::Horizontal, 0.0, 450.0), 0.0);
        assert_eq!(at(Gradient::Horizontal, 1600.0, 450.0), 1.0);
        assert!((at(Gradient::Horizontal, 800.0, 0.0) - 0.5).abs() < 1e-4);
        assert_eq!(at(Gradient::Vertical, 800.0, 0.0), 0.0);
        assert_eq!(at(Gradient::Vertical, 800.0, 900.0), 1.0);
        assert!((at(Gradient::Vertical, 0.0, 450.0) - 0.5).abs() < 1e-4);
        assert_eq!(at(Gradient::Diagonal, 0.0, 0.0), 0.0);
        assert_eq!(at(Gradient::Diagonal, 1600.0, 900.0), 1.0);
        assert!((at(Gradient::Diagonal, 800.0, 450.0) - 0.5).abs() < 1e-4);
    }

    /// Diagonal bands cross the corner-to-corner line at a right angle, so the two other
    /// corners get the value of their projection on that line.
    #[test]
    fn diagonal_bands_cross_the_corner_to_corner_line() {
        let (w2, h2) = (1600.0f32 * 1600.0, 900.0f32 * 900.0);
        assert!((at(Gradient::Diagonal, 1600.0, 0.0) - w2 / (w2 + h2)).abs() < 1e-4);
        assert!((at(Gradient::Diagonal, 0.0, 900.0) - h2 / (w2 + h2)).abs() < 1e-4);
    }

    #[test]
    fn radial_is_full_in_the_centre_and_empty_in_the_corners() {
        assert_eq!(at(Gradient::Radial, 800.0, 450.0), 1.0);
        assert!(at(Gradient::Radial, 0.0, 0.0) < 1e-4);
        assert!(at(Gradient::Radial, 1600.0, 900.0) < 1e-4);
    }

    /// The noise still tiles: the same value one period apart.
    #[test]
    fn noise_repeats_after_one_period() {
        for k in 0..20 {
            let (x, y) = (k as f32 * 37.0, k as f32 * 19.0);
            let a = at(Gradient::Noise, x, y);
            assert!((a - at(Gradient::Noise, x + PERIOD_PX, y)).abs() < 1e-3, "not periodic in x");
            assert!((a - at(Gradient::Noise, x, y + PERIOD_PX)).abs() < 1e-3, "not periodic in y");
        }
    }

    #[test]
    fn field_stays_between_zero_and_one() {
        for g in Gradient::ALL {
            for k in 0..200 {
                // Includes points past the window edge, like the overhanging last cells.
                let x = at(g, k as f32 * 9.1 - 20.0, k as f32 * 5.3 - 20.0);
                assert!((0.0..=1.0).contains(&x));
            }
        }
    }
}
