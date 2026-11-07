//! The app's icons: SVG files exported from the Figma sheet, built into the executable.
//!
//! Each icon is rasterised by `resvg` at the exact pixel size it is drawn at (so it stays
//! crisp at any zoom), cached as a texture, and drawn as a white mask tinted with the
//! theme colour. Only the alpha of the SVG counts, so its own colour does not matter.
//! The brand set (logo, app icon) keeps its colours instead.
//! Source: Figma file "Vegit", page "Icons" (24 px grid, 2 px round strokes).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureId, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    Settings,
    FolderOpen,
    Refresh,
    Search,
    Key,
    Palette,
    Close,
    WindowMinimize,
    WindowMaximize,
    WindowRestore,
    Computer,
    Cloud,
    PullRequest,
    Tag,
    Box,
    Link,
    Check,
    ChevronRight,
    ChevronDown,
    StatusModified,
    StatusAdded,
    StatusDeleted,
    StatusRenamed,
    StatusConflict,
    Trash,
    Commit,
    Folder,
    ArrowLeft,
    ArrowUp,
    ArrowDown,
    CaretDown,
}

macro_rules! svg {
    ($dir:literal, $name:literal) => {
        ($name, include_bytes!(concat!("../assets/", $dir, "/", $name, ".svg")).as_slice())
    };
}

impl Icon {
    /// File name (without `.svg`) and contents.
    fn source(self) -> (&'static str, &'static [u8]) {
        match self {
            Self::Undo => svg!("icons", "undo"),
            Self::Redo => svg!("icons", "redo"),
            Self::Fetch => svg!("icons", "fetch"),
            Self::Pull => svg!("icons", "pull"),
            Self::Push => svg!("icons", "push"),
            Self::Branch => svg!("icons", "branch"),
            Self::Stash => svg!("icons", "stash"),
            Self::Pop => svg!("icons", "pop"),
            Self::Terminal => svg!("icons", "terminal"),
            Self::Settings => svg!("icons", "settings"),
            Self::FolderOpen => svg!("icons", "folder-open"),
            Self::Refresh => svg!("icons", "refresh"),
            Self::Search => svg!("icons", "search"),
            Self::Key => svg!("icons", "key"),
            Self::Palette => svg!("icons", "palette"),
            Self::Close => svg!("icons", "close"),
            Self::WindowMinimize => svg!("icons", "window-minimize"),
            Self::WindowMaximize => svg!("icons", "window-maximize"),
            Self::WindowRestore => svg!("icons", "window-restore"),
            Self::Computer => svg!("icons", "computer"),
            Self::Cloud => svg!("icons", "cloud"),
            Self::PullRequest => svg!("icons", "pull-request"),
            Self::Tag => svg!("icons", "tag"),
            Self::Box => svg!("icons", "box"),
            Self::Link => svg!("icons", "link"),
            Self::Check => svg!("icons", "check"),
            Self::ChevronRight => svg!("icons", "chevron-right"),
            Self::ChevronDown => svg!("icons", "chevron-down"),
            Self::StatusModified => svg!("icons", "status-modified"),
            Self::StatusAdded => svg!("icons", "status-added"),
            Self::StatusDeleted => svg!("icons", "status-deleted"),
            Self::StatusRenamed => svg!("icons", "status-renamed"),
            Self::StatusConflict => svg!("icons", "status-conflict"),
            Self::Trash => svg!("icons", "trash"),
            Self::Commit => svg!("icons", "commit"),
            Self::Folder => svg!("icons", "folder"),
            Self::ArrowLeft => svg!("icons", "arrow-left"),
            Self::ArrowUp => svg!("icons", "arrow-up"),
            Self::ArrowDown => svg!("icons", "arrow-down"),
            Self::CaretDown => svg!("icons", "caret-down"),
        }
    }
}

const LOGO: (&str, &[u8]) = svg!("brand", "logo");
const APP_ICON: (&str, &[u8]) = svg!("brand", "app-icon");

static CACHE: OnceLock<Mutex<HashMap<(&'static str, u32), TextureHandle>>> = OnceLock::new();

/// Draws `bytes` into a `px`×`px` square. A `mask` keeps only the coverage, as white.
fn rasterize(bytes: &[u8], px: u32, mask: bool) -> ColorImage {
    let px = px.max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px).expect("icon size is never zero");
    // The files are ours and built in: a parse error is a broken asset, drawn as nothing.
    if let Ok(tree) = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()) {
        let size = tree.size();
        let scale = px as f32 / size.width().max(size.height());
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    }
    let size = [px as usize, px as usize];
    if mask {
        let data: Vec<u8> = pixmap.data().chunks_exact(4).flat_map(|p| [p[3]; 4]).collect();
        ColorImage::from_rgba_premultiplied(size, &data)
    } else {
        ColorImage::from_rgba_premultiplied(size, pixmap.data())
    }
}

fn texture(ctx: &egui::Context, (name, bytes): (&'static str, &'static [u8]), px: u32, mask: bool) -> TextureId {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    map.entry((name, px))
        .or_insert_with(|| ctx.load_texture(format!("icon-{name}-{px}"), rasterize(bytes, px, mask), egui::TextureOptions::LINEAR))
        .id()
}

/// Pixel size of a square of `side` points on this screen.
fn pixels(ctx: &egui::Context, side: f32) -> u32 {
    (side * ctx.pixels_per_point()).round().max(1.0) as u32
}

/// Draws `icon` in the largest square that fits in `rect`, centred, tinted with `color`.
pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let square = Rect::from_center_size(rect.center(), vec2(side, side));
    let id = texture(p.ctx(), icon.source(), pixels(p.ctx(), side), true);
    p.image(id, square, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
}

/// The coloured logo, in the largest square that fits in `rect`.
pub fn paint_logo(p: &egui::Painter, rect: Rect) {
    let side = rect.width().min(rect.height());
    let square = Rect::from_center_size(rect.center(), vec2(side, side));
    let id = texture(p.ctx(), LOGO, pixels(p.ctx(), side), false);
    p.image(id, square, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// `icon` as an image widget of `side` points, for buttons and menus built from atoms.
pub fn image(ctx: &egui::Context, icon: Icon, side: f32, color: Color32) -> egui::Image<'static> {
    let id = texture(ctx, icon.source(), pixels(ctx, side), true);
    egui::Image::from_texture((id, vec2(side, side))).tint(color)
}

/// Window and taskbar icon, drawn from the app icon at `px` pixels.
pub fn app_icon(px: u32) -> egui::IconData {
    let img = rasterize(APP_ICON.1, px, false);
    // egui wants straight (not premultiplied) alpha here.
    let rgba = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    egui::IconData { rgba, width: px, height: px }
}

/// A toolbar button with the icon above its label, like a classic git client toolbar.
pub fn tool_button(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool, text: Color32, hover: Color32) -> egui::Response {
    let color = if enabled { text } else { text.gamma_multiply(0.35) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(10.5), color);
    // 52 px like every other button, wider only for longer labels ("Undo rebase").
    let size = vec2((galley.size().x + 14.0).max(52.0), 44.0);
    // Disabled buttons still sense hover, so their tooltip can say why.
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(rect, egui::CornerRadius::same(12), hover);
    }
    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 14.0), vec2(18.0, 18.0));
    paint(p, icon_rect, icon, color);
    p.galley(pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - 9.0 - galley.size().y / 2.0), galley, color);
    resp
}

/// Small square icon button.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, text: Color32, fill: Color32, hover: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::same(10), if resp.hovered() { hover } else { fill });
    paint(p, rect.shrink(7.0), icon, text);
    resp
}

/// Frameless icon button (close crosses): only a soft background on hover.
pub fn ghost_button(ui: &mut egui::Ui, icon: Icon, side: f32, color: Color32, hover: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(side + 8.0, side + 8.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, egui::CornerRadius::same(6), hover);
    }
    paint(p, rect.shrink(4.0), icon, color);
    resp
}
