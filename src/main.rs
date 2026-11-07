//! Vegit: a friendly desktop git client.
//!
//! Usage: `vegit [repository-folder]` or `vegit --screenshot <file.png>` (draws about
//! 120 frames of the sample project, saves the window as a PNG and exits).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod access;
mod app;
mod avatars;
mod backdrop;
mod demo;
mod diff;
mod filelist;
mod forge;
mod git;
mod graph;
mod icons;
mod preview;
#[cfg(test)]
mod tests;
mod theme;
mod ui;
mod ui_detail;
mod ui_graph;
mod ui_quick;
mod ui_side;
mod util;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let mut screenshot: Option<PathBuf> = None;
    let mut open: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            screenshot = args.next().map(PathBuf::from);
        } else if let Some(p) = a.strip_prefix("--screenshot=") {
            screenshot = Some(PathBuf::from(p));
        } else if !a.starts_with("--") {
            open = Some(PathBuf::from(a));
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Vegit")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(icons::app_icon(256))
            // The theme draws the whole window, title bar included.
            .with_decorations(false),
        persist_window: screenshot.is_none(),
        ..Default::default()
    };

    eframe::run_native(
        "Vegit",
        options,
        Box::new(move |cc| {
            let mut app = app::VegitApp::new(cc, screenshot);
            if let Some(p) = open {
                if let Err(e) = app.open_repo(&p, true) {
                    eprintln!("{e}");
                }
            }
            Ok(Box::new(app))
        }),
    )
}
