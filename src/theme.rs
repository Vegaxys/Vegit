//! Colour themes. Every theme uses the same "bento" structure: a dark (or deep) canvas
//! with separate rounded tiles on it; only the colours change.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Theme};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    Midnight,
    Dracula,
    Monokai,
    Nord,
    Dusk,
    Paper,
    SolarizedLight,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 7] = [
        Self::Midnight,
        Self::Dracula,
        Self::Monokai,
        Self::Nord,
        Self::Dusk,
        Self::Paper,
        Self::SolarizedLight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Midnight => "Midnight",
            Self::Dracula => "Dracula",
            Self::Monokai => "Monokai",
            Self::Nord => "Nord",
            Self::Dusk => "Dusk",
            Self::Paper => "Paper (light)",
            Self::SolarizedLight => "Solarized (light)",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// Background between the tiles.
    pub canvas: Color32,
    /// Tile background.
    pub panel: Color32,
    /// Inputs, pills and headers inside a tile.
    pub card: Color32,
    pub row_alt: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub added: Color32,
    pub added_bg: Color32,
    pub removed: Color32,
    pub removed_bg: Color32,
    pub warn: Color32,
    pub lanes: [Color32; 8],
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const fn lanes(v: [u32; 8]) -> [Color32; 8] {
    [hex(v[0]), hex(v[1]), hex(v[2]), hex(v[3]), hex(v[4]), hex(v[5]), hex(v[6]), hex(v[7])]
}

impl Palette {
    pub fn of(choice: ThemeChoice) -> Self {
        match choice {
            ThemeChoice::Midnight => Self {
                dark: true,
                canvas: hex(0x09090b),
                panel: hex(0x161618),
                card: hex(0x222225),
                row_alt: hex(0x1a1a1d),
                hover: hex(0x26262b),
                selected: hex(0x1c3150),
                border: hex(0x26262a),
                text: hex(0xf2f2f4),
                muted: hex(0x8b8b94),
                accent: hex(0x2d8cff),
                accent_text: Color32::WHITE,
                added: hex(0x3ecf8e),
                added_bg: hex(0x12301f),
                removed: hex(0xff5c6c),
                removed_bg: hex(0x3a1519),
                warn: hex(0xff9a3c),
                lanes: lanes([0x2d8cff, 0xff8a3d, 0x9b7bff, 0x3ecf8e, 0xff6fb5, 0xffd23f, 0x3fd0e0, 0xff5c6c]),
            },
            ThemeChoice::Dracula => Self {
                dark: true,
                canvas: hex(0x1b1c25),
                panel: hex(0x282a36),
                card: hex(0x343746),
                row_alt: hex(0x2c2e3b),
                hover: hex(0x3a3d4e),
                selected: hex(0x44475a),
                border: hex(0x363949),
                text: hex(0xf8f8f2),
                muted: hex(0x8b93c4),
                accent: hex(0xbd93f9),
                accent_text: hex(0x282a36),
                added: hex(0x50fa7b),
                added_bg: hex(0x24402f),
                removed: hex(0xff5555),
                removed_bg: hex(0x4a2530),
                warn: hex(0xffb86c),
                lanes: lanes([0xbd93f9, 0xff79c6, 0x8be9fd, 0x50fa7b, 0xffb86c, 0xf1fa8c, 0xff5555, 0x6272a4]),
            },
            ThemeChoice::Monokai => Self {
                dark: true,
                canvas: hex(0x1a1b16),
                panel: hex(0x272822),
                card: hex(0x34352d),
                row_alt: hex(0x2b2c25),
                hover: hex(0x3a3b32),
                selected: hex(0x49483e),
                border: hex(0x3a3a30),
                text: hex(0xf8f8f2),
                muted: hex(0x9a957c),
                accent: hex(0xa6e22e),
                accent_text: hex(0x272822),
                added: hex(0xa6e22e),
                added_bg: hex(0x2f3a1c),
                removed: hex(0xf92672),
                removed_bg: hex(0x4a1f2c),
                warn: hex(0xfd971f),
                lanes: lanes([0x66d9ef, 0xf92672, 0xa6e22e, 0xfd971f, 0xae81ff, 0xe6db74, 0xfd5ff0, 0x75715e]),
            },
            ThemeChoice::Nord => Self {
                dark: true,
                canvas: hex(0x21252e),
                panel: hex(0x2e3440),
                card: hex(0x3b4252),
                row_alt: hex(0x323846),
                hover: hex(0x3b4252),
                selected: hex(0x434c5e),
                border: hex(0x3b4252),
                text: hex(0xeceff4),
                muted: hex(0x9aa3b5),
                accent: hex(0x88c0d0),
                accent_text: hex(0x2e3440),
                added: hex(0xa3be8c),
                added_bg: hex(0x34402f),
                removed: hex(0xbf616a),
                removed_bg: hex(0x45303a),
                warn: hex(0xebcb8b),
                lanes: lanes([0x88c0d0, 0xbf616a, 0xa3be8c, 0xebcb8b, 0xb48ead, 0x81a1c1, 0xd08770, 0x8fbcbb]),
            },
            ThemeChoice::Dusk => Self {
                dark: true,
                canvas: hex(0x110f17),
                panel: hex(0x1d1b26),
                card: hex(0x2a2636),
                row_alt: hex(0x221f2c),
                hover: hex(0x2f2a42),
                selected: hex(0x3b335c),
                border: hex(0x2c283a),
                text: hex(0xece8f3),
                muted: hex(0x9b94ab),
                accent: hex(0xf29e6b),
                accent_text: hex(0x1d1b26),
                added: hex(0x7fd19a),
                added_bg: hex(0x1f3a2c),
                removed: hex(0xf28b96),
                removed_bg: hex(0x47232b),
                warn: hex(0xf0c05a),
                lanes: lanes([0xf29e6b, 0x9d8cf2, 0x5fd0c0, 0xf0c05a, 0xec7fc0, 0x6fb3f2, 0xa8d86a, 0xd6a284]),
            },
            ThemeChoice::Paper => Self {
                dark: false,
                canvas: hex(0xe8e1d5),
                panel: hex(0xfaf7f1),
                card: hex(0xf0eae0),
                row_alt: hex(0xf5f0e8),
                hover: hex(0xe9e1f3),
                selected: hex(0xdcd2f5),
                border: hex(0xddd3c4),
                text: hex(0x2b2733),
                muted: hex(0x857d8c),
                accent: hex(0x5b4bc4),
                accent_text: Color32::WHITE,
                added: hex(0x2f855a),
                added_bg: hex(0xdff2e3),
                removed: hex(0xc03e4e),
                removed_bg: hex(0xf8e0e0),
                warn: hex(0xc77a12),
                lanes: lanes([0x5b4bc4, 0xe06c5a, 0x2a9d8f, 0xd99a1e, 0xc24f9b, 0x3d84c6, 0x7aa63a, 0x9a6a4f]),
            },
            ThemeChoice::SolarizedLight => Self {
                dark: false,
                canvas: hex(0xe6dfc8),
                panel: hex(0xfdf6e3),
                card: hex(0xf3ecd6),
                row_alt: hex(0xf8f1dc),
                hover: hex(0xeee8d5),
                selected: hex(0xe2dac0),
                border: hex(0xe0d8c0),
                text: hex(0x3b4d53),
                muted: hex(0x839496),
                accent: hex(0x268bd2),
                accent_text: Color32::WHITE,
                added: hex(0x859900),
                added_bg: hex(0xeef0d0),
                removed: hex(0xdc322f),
                removed_bg: hex(0xf8dcd6),
                warn: hex(0xcb4b16),
                lanes: lanes([0x268bd2, 0xd33682, 0x859900, 0xb58900, 0x6c71c4, 0x2aa198, 0xcb4b16, 0xdc322f]),
            },
        }
    }

    pub fn lane(&self, i: usize) -> Color32 {
        self.lanes[i % self.lanes.len()]
    }

    /// Second level of the text hierarchy: authors, descriptions, list items.
    /// Titles and messages use `text`, dates/ids/folders use `muted`.
    pub fn text2(&self) -> Color32 {
        mix(self.text, self.panel, 0.32)
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Interface and code fonts of a theme, as Windows font files; the first one found is used
/// and egui's built-in fonts stay behind them for missing glyphs (emoji, symbols).
pub struct FontChoice {
    pub ui: &'static [&'static str],
    pub code: &'static [&'static str],
    pub label: &'static str,
}

impl ThemeChoice {
    pub fn fonts(self) -> FontChoice {
        match self {
            Self::Midnight => FontChoice { ui: &["segoeui.ttf"], code: &["CascadiaMono.ttf", "consola.ttf"], label: "Segoe UI · Cascadia Mono" },
            Self::Dracula => FontChoice { ui: &["segoeui.ttf"], code: &["CascadiaCode.ttf", "consola.ttf"], label: "Segoe UI · Cascadia Code" },
            Self::Monokai => FontChoice { ui: &["trebuc.ttf", "segoeui.ttf"], code: &["consola.ttf"], label: "Trebuchet · Consolas" },
            Self::Nord => FontChoice { ui: &["calibri.ttf", "segoeui.ttf"], code: &["CascadiaMono.ttf", "consola.ttf"], label: "Calibri · Cascadia Mono" },
            Self::Dusk => FontChoice { ui: &["trebuc.ttf", "segoeui.ttf"], code: &["consola.ttf"], label: "Trebuchet · Consolas" },
            Self::Paper => FontChoice { ui: &["georgia.ttf", "segoeui.ttf"], code: &["consola.ttf"], label: "Georgia · Consolas" },
            Self::SolarizedLight => FontChoice { ui: &["segoeui.ttf"], code: &["lucon.ttf", "consola.ttf"], label: "Segoe UI · Lucida Console" },
        }
    }
}

fn fonts_dir() -> std::path::PathBuf {
    std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts")
}

/// Reads a font: a bare file name is looked up in the system fonts folder, a full path
/// (picked with "Other font file…") is read as is.
fn load_font(candidates: &[&str]) -> Option<(String, Vec<u8>)> {
    candidates.iter().find_map(|f| {
        let path = std::path::Path::new(f);
        let full = if path.is_absolute() { path.to_path_buf() } else { fonts_dir().join(f) };
        std::fs::read(full).ok().map(|b| (f.to_string(), b))
    })
}

/// Fonts offered in the settings, as (name, file). Only those present are listed.
pub const FONT_CHOICES: &[(&str, &str)] = &[
    ("Segoe UI", "segoeui.ttf"),
    ("Calibri", "calibri.ttf"),
    ("Candara", "Candara.ttf"),
    ("Corbel", "corbel.ttf"),
    ("Tahoma", "tahoma.ttf"),
    ("Trebuchet MS", "trebuc.ttf"),
    ("Verdana", "verdana.ttf"),
    ("Arial", "arial.ttf"),
    ("Georgia", "georgia.ttf"),
    ("Cambria", "cambria.ttc"),
    ("Cascadia Code", "CascadiaCode.ttf"),
    ("Cascadia Mono", "CascadiaMono.ttf"),
    ("Consolas", "consola.ttf"),
    ("Lucida Console", "lucon.ttf"),
    ("Courier New", "cour.ttf"),
];

pub fn installed_fonts() -> Vec<(&'static str, &'static str)> {
    let dir = fonts_dir();
    FONT_CHOICES.iter().copied().filter(|(_, f)| dir.join(f).exists()).collect()
}

/// How branch lanes are coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LaneMode {
    /// The preset's own colours.
    #[default]
    Theme,
    /// A smooth gradient starting at the interface colour.
    Gradient,
    Rainbow,
    /// Random colours; "Shuffle" picks new ones.
    Random,
    /// Every branch in one colour.
    Unicolor,
    /// Lighter and darker shades of the interface colour.
    FromUi,
}

impl LaneMode {
    pub const ALL: [LaneMode; 6] = [Self::Theme, Self::Gradient, Self::Rainbow, Self::Random, Self::Unicolor, Self::FromUi];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme colours",
            Self::Gradient => "Gradient",
            Self::Rainbow => "Rainbow",
            Self::Random => "Random",
            Self::Unicolor => "Single colour",
            Self::FromUi => "From the interface colour",
        }
    }
}

/// Everything that shapes the look, taken from the settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub theme: ThemeChoice,
    pub accent: Option<[u8; 3]>,
    pub ui_font: Option<String>,
    pub code_font: Option<String>,
    pub text_size: f32,
    pub lanes: LaneMode,
    pub lane_color: [u8; 3],
    pub seed: u64,
}

impl Palette {
    /// The preset's palette with the person's interface colour and branch colours applied.
    pub fn build(a: &Appearance) -> Self {
        let mut p = Palette::of(a.theme);
        if let Some([r, g, b]) = a.accent {
            let c = Color32::from_rgb(r, g, b);
            p.accent = c;
            p.selected = mix(c, p.panel, 0.72);
            let lum = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            p.accent_text = if lum > 150.0 { Color32::from_rgb(0x15, 0x15, 0x18) } else { Color32::WHITE };
        }
        let (s, v) = if p.dark { (0.62, 0.95) } else { (0.7, 0.72) };
        let hsv = |h: f32, s: f32, v: f32| -> Color32 { egui::ecolor::Hsva::new(h.rem_euclid(1.0), s, v, 1.0).into() };
        let accent_h = egui::ecolor::Hsva::from(p.accent).h;
        match a.lanes {
            LaneMode::Theme => {}
            LaneMode::Gradient => {
                for i in 0..8 {
                    p.lanes[i] = hsv(accent_h + i as f32 * 0.055, s, v);
                }
            }
            LaneMode::Rainbow => {
                for i in 0..8 {
                    p.lanes[i] = hsv(i as f32 / 8.0, s, v);
                }
            }
            LaneMode::Random => {
                let mut x = a.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
                for i in 0..8 {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    p.lanes[i] = hsv((x % 1000) as f32 / 1000.0, s, v);
                }
            }
            LaneMode::Unicolor => {
                let [r, g, b] = a.lane_color;
                p.lanes = [Color32::from_rgb(r, g, b); 8];
            }
            LaneMode::FromUi => {
                let base = egui::ecolor::Hsva::from(p.accent);
                for i in 0..8 {
                    let t = i as f32 / 7.0;
                    p.lanes[i] = hsv(base.h, (base.s * (1.0 - 0.5 * t)).clamp(0.2, 1.0), (0.55 + 0.45 * (1.0 - (t - 0.5).abs() * 2.0)).clamp(0.4, 1.0));
                }
            }
        }
        p
    }
}

/// Installs the fonts (the person's choice, or the preset's) in front of egui's defaults.
pub fn apply_fonts(ctx: &egui::Context, a: &Appearance) {
    let fc = a.theme.fonts();
    let mut fonts = egui::FontDefinitions::default();
    let ui_override = a.ui_font.as_deref().map(|f| vec![f]);
    let code_override = a.code_font.as_deref().map(|f| vec![f]);
    let ui_list: Vec<&str> = ui_override.unwrap_or_else(|| fc.ui.to_vec());
    let code_list: Vec<&str> = code_override.unwrap_or_else(|| fc.code.to_vec());
    for (family, candidates) in [(egui::FontFamily::Proportional, ui_list), (egui::FontFamily::Monospace, code_list)] {
        if let Some((name, bytes)) = load_font(&candidates) {
            fonts.font_data.insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name);
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, a: &Appearance) {
    apply_fonts(ctx, a);
    let p = Palette::build(a);
    let size = a.text_size.clamp(11.0, 18.0);
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::same(10);
    v.panel_fill = p.canvas;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(18);
    v.menu_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = p.card;
    v.faint_bg_color = p.row_alt;
    v.code_bg_color = p.card;
    // Inputs sink into the canvas colour so they stay visible on tiles and on cards alike.
    v.text_edit_bg_color = Some(p.canvas);
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.removed;
    v.selection.bg_fill = p.selected;
    v.selection.stroke = Stroke::new(1.0, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    // Checkboxes and slider rails use bg_fill, buttons use weak_bg_fill: a lighter
    // bg_fill keeps check boxes visible even when they sit on a card.
    w.inactive.bg_fill = mix(p.card, p.text, 0.14);
    w.inactive.weak_bg_fill = p.card;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.active.bg_fill = p.selected;
    w.active.weak_bg_fill = p.selected;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = r;
    w.open.bg_fill = p.selected;
    w.open.weak_bg_fill = p.selected;
    w.open.corner_radius = r;
    // Hovering or pressing must never make an element bigger (it shifted rows around).
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.expansion = 0.0;
    }

    for theme in [Theme::Light, Theme::Dark] {
        ctx.set_visuals_of(theme, v.clone());
        ctx.style_mut_of(theme, |s| {
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            // Interface text behaves like a desktop app's: it can't be selected by dragging.
            s.interaction.selectable_labels = false;
            s.spacing.button_padding = egui::vec2(12.0, 6.0);
            s.spacing.interact_size.y = 28.0;
            s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(size));
            s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(size));
            s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(size - 2.5));
            s.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(size - 1.0));
            s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(size + 8.5));
        });
    }
}
