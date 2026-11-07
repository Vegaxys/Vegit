//! Path view and tree view of a file list.
//!
//! Git lists files sorted by path, so the tree can be built in one pass without sorting
//! or allocating: folder rows borrow slices of the file paths. The result is a flat list
//! of fixed-height rows, which keeps huge lists virtualised.

use std::collections::HashSet;

use eframe::egui::{self, Align2, CornerRadius, FontId, Sense, pos2, vec2};

use crate::git::FileChange;
use crate::icons::{self, Icon};
use crate::theme::Palette;
use crate::util::FILE_ROW_H;

pub const INDENT: f32 = 16.0;

pub enum Entry<'a> {
    Folder { path: &'a str, name: &'a str, depth: usize, collapsed: bool },
    File { idx: usize, depth: usize },
}

pub fn flatten<'a>(files: &'a [FileChange], tree: bool, collapsed: &HashSet<String>) -> Vec<Entry<'a>> {
    if !tree {
        return (0..files.len()).map(|idx| Entry::File { idx, depth: 0 }).collect();
    }
    let mut out = Vec::with_capacity(files.len() + 16);
    let mut stack: Vec<&str> = Vec::new();
    // Depth of the collapsed folder hiding what follows, if any.
    let mut hidden_below: Option<usize> = None;
    for (idx, f) in files.iter().enumerate() {
        let dir = f.path.rfind('/').map_or("", |i| &f.path[..i]);
        while let Some(top) = stack.last() {
            let inside = dir == *top || (dir.starts_with(top) && dir.as_bytes().get(top.len()) == Some(&b'/'));
            if inside {
                break;
            }
            stack.pop();
        }
        if hidden_below.is_some_and(|d| stack.len() <= d) {
            hidden_below = None;
        }
        let mut start = stack.last().map_or(0, |s| s.len() + 1);
        while start < dir.len() {
            let end = dir[start..].find('/').map_or(dir.len(), |e| start + e);
            let path = &dir[..end];
            let depth = stack.len();
            let is_collapsed = collapsed.contains(path);
            if hidden_below.is_none() {
                out.push(Entry::Folder { path, name: &dir[start..end], depth, collapsed: is_collapsed });
                if is_collapsed {
                    hidden_below = Some(depth);
                }
            }
            stack.push(path);
            start = end + 1;
        }
        if hidden_below.is_none() {
            out.push(Entry::File { idx, depth: stack.len() });
        }
    }
    out
}

/// A folder line of the tree: chevron, folder shape, name. Returns the row response and
/// whether its hover button was clicked.
pub fn folder_row(ui: &mut egui::Ui, name: &str, depth: usize, collapsed: bool, pal: &Palette, button: Option<&str>) -> (egui::Response, bool) {
    let (full, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
    let p = ui.painter_at(full);
    let hovered = ui.rect_contains_pointer(full);
    if hovered {
        p.rect_filled(full.shrink2(vec2(0.0, 1.0)), CornerRadius::same(9), pal.hover);
    }
    let x = full.left() + depth as f32 * INDENT;
    let cy = full.center().y;
    let chevron = if collapsed { Icon::ChevronRight } else { Icon::ChevronDown };
    icons::paint(&p, egui::Rect::from_center_size(pos2(x + 8.0, cy), vec2(13.0, 13.0)), chevron, pal.muted);
    icons::paint(&p, egui::Rect::from_center_size(pos2(x + 26.0, cy), vec2(17.0, 17.0)), Icon::Folder, pal.accent);
    p.text(pos2(x + 40.0, cy), Align2::LEFT_CENTER, name, FontId::proportional(13.0), pal.text2());

    let clicked_button = match button {
        Some(label) if hovered => crate::util::hover_pill(ui, full, label, pal, resp.id.with("pill")),
        _ => false,
    };
    (resp, clicked_button)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileState;

    fn files(paths: &[&str]) -> Vec<FileChange> {
        paths.iter().map(|p| FileChange { path: p.to_string(), old_path: None, state: FileState::Modified }).collect()
    }

    fn describe(entries: &[Entry<'_>], files: &[FileChange]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Folder { name, depth, .. } => format!("{}{name}/", "  ".repeat(*depth)),
                Entry::File { idx, depth } => format!("{}{}", "  ".repeat(*depth), files[*idx].path.rsplit('/').next().unwrap()),
            })
            .collect()
    }

    #[test]
    fn tree_groups_files_under_their_folders() {
        let f = files(&["Assets/Scenes/Main.unity", "Assets/Scenes/Menu.unity", "Assets/hero.png", "README.md"]);
        let rows = describe(&flatten(&f, true, &HashSet::new()), &f);
        assert_eq!(rows, ["Assets/", "  Scenes/", "    Main.unity", "    Menu.unity", "  hero.png", "README.md"]);
    }

    #[test]
    fn collapsed_folder_hides_its_content_only() {
        let f = files(&["Assets/Scenes/Main.unity", "Assets/hero.png", "README.md"]);
        let collapsed: HashSet<String> = ["Assets/Scenes".to_string()].into();
        let rows = describe(&flatten(&f, true, &collapsed), &f);
        assert_eq!(rows, ["Assets/", "  Scenes/", "  hero.png", "README.md"]);
    }
}
