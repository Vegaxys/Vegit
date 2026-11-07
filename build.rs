//! Gives the Windows executable its icon (Explorer, taskbar pin, shortcuts).
//!
//! `assets/brand/app-icon.svg` is rendered at the usual icon sizes, packed into an .ico
//! in `OUT_DIR` and embedded as a resource, so the .exe always matches the Figma asset.
//! The window icon at run time comes from the same SVG (see `icons::app_icon`).

use std::path::PathBuf;

const SVG: &str = "assets/brand/app-icon.svg";
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed={SVG}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let data = std::fs::read(SVG).unwrap_or_else(|e| panic!("can't read {SVG}: {e}"));
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).unwrap_or_else(|e| panic!("can't parse {SVG}: {e}"));
    let pngs: Vec<Vec<u8>> = SIZES
        .iter()
        .map(|&px| {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px).expect("icon size is never zero");
            let scale = px as f32 / tree.size().width();
            resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
            pixmap.encode_png().expect("PNG encoding of an in-memory pixmap")
        })
        .collect();

    // ICO with PNG images: a 6-byte header, one 16-byte entry per size, then the PNGs.
    let mut ico = vec![0, 0, 1, 0];
    ico.extend((SIZES.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * SIZES.len();
    for (&px, png) in SIZES.iter().zip(&pngs) {
        let side = if px >= 256 { 0 } else { px as u8 }; // 0 means 256 in the ICO format
        ico.extend([side, side, 0, 0]);
        ico.extend(1u16.to_le_bytes()); // colour planes
        ico.extend(32u16.to_le_bytes()); // bits per pixel
        ico.extend((png.len() as u32).to_le_bytes());
        ico.extend((offset as u32).to_le_bytes());
        offset += png.len();
    }
    for png in &pngs {
        ico.extend(png);
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("vegit.ico");
    std::fs::write(&out, ico).unwrap_or_else(|e| panic!("can't write {}: {e}", out.display()));

    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon(out.to_str().expect("OUT_DIR is valid UTF-8"))
        .compile()
        .expect("can't embed the icon (is the Windows SDK's rc.exe installed?)");
}
