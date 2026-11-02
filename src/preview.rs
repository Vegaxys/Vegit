//! "Render" view for images: the file before and after the change, decoded on the worker.
//!
//! Raster formats go through the `image` crate, SVG through `resvg`. Both sides are
//! scaled down to [`MAX_SIDE`] so a 8K texture from a Unity project can't eat the GPU.

use std::path::Path;

use eframe::egui::ColorImage;

use crate::git::{self, DiffSource, FileChange, FileState};

const MAX_SIDE: u32 = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Raster,
    Svg,
}

/// Files the diff view can show as pictures.
pub fn kind_of(path: &str) -> Option<Kind> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tga" | "ico" => Some(Kind::Raster),
        "svg" => Some(Kind::Svg),
        _ => None,
    }
}

pub struct Side {
    pub image: ColorImage,
    /// Original pixel size, before any scaling for display.
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

#[derive(Default)]
pub struct Pair {
    pub before: Option<Result<Side, String>>,
    pub after: Option<Result<Side, String>>,
}

/// Loads both sides of an image change. `None` on a side means the file doesn't exist there.
pub fn load_pair(repo: &Path, source: &DiffSource, file: &FileChange) -> Pair {
    let kind = kind_of(&file.path).unwrap_or(Kind::Raster);
    let old_path = file.old_path.clone().unwrap_or_else(|| file.path.clone());
    let show = |spec: String| git::run_bytes(repo, &["show", &spec]);
    let (before, after): (Option<Vec<u8>>, Option<Vec<u8>>) = match source {
        DiffSource::Unstaged => {
            let after = (file.state != FileState::Deleted).then(|| std::fs::read(repo.join(&file.path)).ok()).flatten();
            let before = if file.state == FileState::Untracked {
                None
            } else {
                show(format!(":{}", file.path)).or_else(|| show(format!("HEAD:{}", file.path)))
            };
            (before, after)
        }
        DiffSource::Staged => {
            let after = (file.state != FileState::Deleted).then(|| show(format!(":{}", file.path))).flatten();
            let before = (file.state != FileState::Added).then(|| show(format!("HEAD:{old_path}"))).flatten();
            (before, after)
        }
        DiffSource::Commit { sha, parent } => {
            let after = (file.state != FileState::Deleted).then(|| show(format!("{sha}:{}", file.path))).flatten();
            let before = match parent {
                Some(p) if file.state != FileState::Added => show(format!("{p}:{old_path}")),
                _ => None,
            };
            (before, after)
        }
    };
    Pair {
        before: before.map(|b| decode(&b, kind)),
        after: after.map(|b| decode(&b, kind)),
    }
}

pub fn decode(bytes: &[u8], kind: Kind) -> Result<Side, String> {
    match kind {
        Kind::Raster => {
            let img = image::load_from_memory(bytes).map_err(|e| format!("Can't read this image: {e}"))?;
            let (width, height) = (img.width(), img.height());
            let img = if width.max(height) > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            Ok(Side { image: ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), width, height, bytes: bytes.len() })
        }
        Kind::Svg => {
            let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).map_err(|e| format!("Can't read this SVG: {e}"))?;
            let size = tree.size();
            let (w, h) = (size.width().max(1.0), size.height().max(1.0));
            // Draw small icons bigger so they stay crisp, big drawings smaller.
            let scale = (1024.0 / w.max(h)).clamp(0.05, 8.0);
            let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
            let mut pixmap = resvg::tiny_skia::Pixmap::new(pw.max(1), ph.max(1)).ok_or("SVG is too large")?;
            resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
            let image = ColorImage::from_rgba_premultiplied([pw as usize, ph as usize], pixmap.data());
            Ok(Side { image, width: w as u32, height: h as u32, bytes: bytes.len() })
        }
    }
}
