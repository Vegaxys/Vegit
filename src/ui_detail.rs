//! Right panel (staging area and commit composer, or a commit's details) and the diff view.
//!
//! Every list here is virtualised with `show_rows`: only the visible rows are laid out,
//! so a commit touching 20 000 files or a 200 000-line scene diff scrolls smoothly.
//! File lists can be shown as full paths or as a folder tree (see [`crate::filelist`]).

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::app::{Action, Dialog, FileViewKind, HunkMode, ImageState, ListId, RepoTab, Selection, ShownImage};
use crate::diff::{DiffMode, DiffRow, LineKind, ParsedDiff, SplitRow};
use crate::filelist::{self, Entry, INDENT};
use crate::git::{self, DiffSource, FileChange, FileState};
use crate::theme::Palette;
use crate::util::{FILE_ROW_H, abs_time, accent_button, avatar, card, file_row, rel_time, section_title, segmented};

pub fn show(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, tree: bool, sort_by_status: bool, acts: &mut Vec<Action>) {
    if tab.loading {
        loading(ui, "Reading the repository…");
        return;
    }
    match tab.selection.clone() {
        Selection::Wip => wip(ui, tab, pal, tree, sort_by_status, acts),
        Selection::Commit(sha) => commit(ui, tab, &sha, pal, tree, acts),
    }
}

pub fn loading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Spinner::new().size(22.0));
        ui.label(RichText::new(text).weak());
    });
}

/// "Path / Tree" switch shown above file lists.
fn view_switch(ui: &mut egui::Ui, pal: &Palette, tree: bool, acts: &mut Vec<Action>) {
    let mut v = tree;
    if segmented(ui, pal, &mut v, &[(false, "Path"), (true, "Tree")]) {
        acts.push(Action::SetTreeView(v));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Unstaged,
    Staged,
}

impl Side {
    fn list(self) -> ListId {
        match self {
            Side::Unstaged => ListId::Unstaged,
            Side::Staged => ListId::Staged,
        }
    }
}

const SECTION_HEAD_H: f32 = 30.0;

fn wip(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, tree: bool, sort_by_status: bool, acts: &mut Vec<Action>) {
    // Composer pinned to the bottom so the file lists can take the rest. No box around it:
    // a thin line is enough to separate it from the lists.
    egui::Panel::bottom("composer")
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 6, bottom: 0 }))
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| composer(ui, tab, pal, acts));

    egui::Panel::top("wip_top")
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 2 }))
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| {
            let n = tab.snap.status.unstaged.len() + tab.snap.status.staged.len();
            let row_w = ui.available_width();
            // Row 1: discard everything on the left, the title centred.
            let (row, _) = ui.allocate_exact_size(vec2(row_w, 30.0), Sense::hover());
            let trash = Rect::from_min_size(pos2(row.left(), row.center().y - 13.0), vec2(26.0, 26.0));
            let can_discard = !tab.snap.status.unstaged.is_empty();
            let tr = ui.interact(trash, ui.scope_id().with("discard_all"), if can_discard { Sense::click() } else { Sense::hover() });
            {
                let p = ui.painter();
                let col = if can_discard { pal.removed } else { pal.muted.gamma_multiply(0.5) };
                if tr.hovered() && can_discard {
                    p.rect_filled(trash, CornerRadius::same(5), pal.removed.gamma_multiply(0.18));
                }
                p.rect_stroke(trash, CornerRadius::same(5), Stroke::new(1.0, col), egui::StrokeKind::Inside);
                crate::icons::paint(p, trash.shrink(6.0), crate::icons::Icon::Trash, col);
            }
            if tr.on_hover_text("Discard all unstaged changes").clicked() && can_discard {
                acts.push(Action::Dialog(Dialog::Confirm {
                    title: "Discard all changes?".into(),
                    body: format!("Every unstaged change ({} file(s)) will be lost, including new files. This can't be undone.", tab.snap.status.unstaged.len()),
                    confirm: "Discard all".into(),
                    then: Box::new(Action::DiscardMany(tab.snap.status.unstaged.clone())),
                }));
            }
            // Title: "15 file changes on [branch]", centred, the branch pill cut with "…".
            let font = egui::FontId::proportional(14.5);
            let lead = format!("{n} file change{} on ", if n == 1 { "" } else { "s" });
            let p = ui.painter();
            let lead_g = p.layout_no_wrap(lead, font.clone(), pal.text);
            let branch = tab.snap.head_branch.clone().unwrap_or_else(|| "detached HEAD".into());
            let max_pill = (row_w - 2.0 * 34.0 - lead_g.size().x - 16.0).max(40.0);
            let pill_g = p.layout(branch.clone(), egui::FontId::proportional(13.0), Color32::WHITE, f32::INFINITY);
            let pill_text_w = pill_g.size().x.min(max_pill);
            let total = lead_g.size().x + pill_text_w + 14.0;
            let x0 = row.center().x - total / 2.0;
            let lead_w = lead_g.size().x;
            p.galley(pos2(x0, row.center().y - lead_g.size().y / 2.0), lead_g, pal.text);
            let pill = Rect::from_min_size(pos2(x0 + lead_w, row.center().y - 10.0), vec2(pill_text_w + 14.0, 20.0));
            p.rect_filled(pill, CornerRadius::same(5), pal.accent);
            let mut job = egui::text::LayoutJob::simple_singleline(branch.clone(), egui::FontId::proportional(13.0), pal.accent_text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(pill_text_w);
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(pill.left() + 7.0, pill.center().y - g.size().y / 2.0), g, pal.accent_text);
            ui.interact(pill, ui.scope_id().with("branch_pill"), Sense::hover()).on_hover_text(branch);

            // Row 2: sort order on the left, Path / Tree centred.
            let (row2, _) = ui.allocate_exact_size(vec2(row_w, 30.0), Sense::hover());
            let sort_r = Rect::from_min_size(pos2(row2.left(), row2.center().y - 12.0), vec2(40.0, 24.0));
            let sr = ui.interact(sort_r, ui.scope_id().with("sort_files"), Sense::click());
            {
                let p = ui.painter();
                if sr.hovered() {
                    p.rect_filled(sort_r, CornerRadius::same(5), pal.hover);
                }
                let label = if sort_by_status { "± ↓" } else { "A-Z" };
                p.text(sort_r.center(), Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), pal.text2());
            }
            let tip = if sort_by_status { "Sorted by kind of change (click: by name)" } else { "Sorted by name (click: by kind of change)" };
            if sr.on_hover_text(tip).clicked() {
                acts.push(Action::SetSortByStatus(!sort_by_status));
            }
            let switch_w = 130.0;
            let sw = Rect::from_center_size(row2.center(), vec2(switch_w, 26.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(sw).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| view_switch(ui, pal, tree, acts));
        });

    // Two lists, each with its own scrollbar, split by a line that can be dragged. Each list
    // folds to its header; until the line is dragged, the list with more files gets more room.
    const DIVIDER_H: f32 = 12.0;
    let total_h = ui.available_height();
    let avail = (total_h - DIVIDER_H).max(SECTION_HEAD_H * 2.0);
    let (nu, ns) = (tab.snap.status.unstaged.len().max(1) as f32, tab.snap.status.staged.len().max(1) as f32);
    let share = tab.wip_split.unwrap_or_else(|| (nu / (nu + ns)).clamp(0.35, 0.7));
    let min_list = SECTION_HEAD_H + 2.0 * FILE_ROW_H;
    // A folded list keeps its header plus a little air before the next block.
    let head = SECTION_HEAD_H + 8.0;
    let (top_h, bottom_h) = match tab.folded {
        [true, true] => (head, head),
        [true, false] => (head, avail - head),
        [false, true] => (avail - head, head),
        [false, false] => {
            let t = (avail * share).clamp(min_list.min(avail / 2.0), (avail - min_list).max(avail / 2.0)).floor();
            (t, avail - t)
        }
    };
    let w = ui.available_width();

    ui.allocate_ui(vec2(w, top_h), |ui| file_section(ui, tab, pal, tree, sort_by_status, Side::Unstaged, top_h, acts));

    let (div, resp) = ui.allocate_exact_size(vec2(w, DIVIDER_H), Sense::click_and_drag());
    let can_drag = tab.folded == [false, false];
    let active = can_drag && (resp.hovered() || resp.dragged());
    if can_drag && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let p = ui.painter();
    p.rect_filled(Rect::from_center_size(div.center(), vec2(w, if active { 2.0 } else { 1.0 })), CornerRadius::ZERO, if active { pal.accent } else { pal.border });
    // A small grip in the middle says the line can be moved.
    p.rect_filled(Rect::from_center_size(div.center(), vec2(36.0, 4.0)), CornerRadius::same(2), if active { pal.accent } else { pal.card });
    if can_drag && resp.dragged() {
        let new_top = top_h + resp.drag_delta().y;
        tab.wip_split = Some((new_top / avail).clamp(0.1, 0.9));
    }
    if resp.double_clicked() {
        tab.wip_split = None;
    }
    let resp = if can_drag { resp.on_hover_text("Drag to share the room between the two lists; double-click to reset") } else { resp };
    let _ = resp;

    ui.allocate_ui(vec2(w, bottom_h), |ui| file_section(ui, tab, pal, tree, sort_by_status, Side::Staged, bottom_h, acts));
}

/// One of the two lists: a fixed header (title, count, stage/unstage all), then its own
/// scrolling list below it, so the scrollbar never covers the header button.
fn file_section(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, tree: bool, sort_by_status: bool, side: Side, height: f32, acts: &mut Vec<Action>) {
    ui.set_min_height(height);
    let list = side.list();
    let files: &[FileChange] = match side {
        Side::Unstaged => &tab.snap.status.unstaged,
        Side::Staged => &tab.snap.status.staged,
    };
    ui.allocate_ui_with_layout(vec2(ui.available_width(), SECTION_HEAD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let (title, button, action, tone) = match side {
            Side::Unstaged => ("Unstaged Files", "Stage All Changes", Action::StageAll, pal.added),
            Side::Staged => ("Staged Files", "Unstage All Changes", Action::UnstageAll, pal.removed),
        };
        // Outlined in green (stage) or red (unstage), like the reference.
        let section_button = |text: &str| {
            egui::Button::new(RichText::new(text).size(12.0).color(tone))
                .fill(tone.gamma_multiply(0.12))
                .stroke(Stroke::new(1.0, tone.gamma_multiply(0.8)))
                .corner_radius(CornerRadius::same(4))
        };
        // Chevron and title fold the list.
        let folded = tab.folded[list as usize];
        let (cr, cresp) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::click());
        let c = cr.center();
        let s = Stroke::new(1.6, pal.text2());
        if folded {
            ui.painter().line_segment([c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0)], s);
            ui.painter().line_segment([c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)], s);
        } else {
            ui.painter().line_segment([c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0)], s);
            ui.painter().line_segment([c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)], s);
        }
        let tresp = ui.add(egui::Label::new(RichText::new(format!("{title} ({})", files.len())).size(14.0).color(pal.text)).sense(Sense::click()));
        if cresp.clicked() || tresp.clicked() {
            tab.folded[list as usize] = !folded;
        }
        if folded {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!files.is_empty(), section_button(button)).clicked() {
                    acts.push(action.clone());
                }
            });
            return;
        }
        if tree && !files.is_empty() {
            let collapsed_any = !tab.collapsed[list as usize].is_empty();
            let text = if collapsed_any { "Expand all" } else { "Collapse all" };
            if ui.add(egui::Button::new(RichText::new(text).size(11.5).color(pal.text2())).frame(false)).clicked() {
                acts.push(Action::CollapseAll(list, !collapsed_any));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!files.is_empty(), section_button(button)).clicked() {
                acts.push(action);
            }
        });
    });
    if tab.folded[list as usize] {
        return;
    }
    // Thin line under the header, then the list.
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, CornerRadius::ZERO, pal.border);
    ui.add_space(4.0);
    if files.is_empty() {
        let text = match side {
            Side::Unstaged => "Nothing to stage: the folder matches the last commit.",
            Side::Staged => "Staged files go into the next commit.",
        };
        ui.label(RichText::new(text).color(pal.muted).size(12.0));
        return;
    }

    let mut entries = filelist::flatten(files, tree, &tab.collapsed[list as usize]);
    if sort_by_status && !tree {
        let rank = |st: FileState| match st {
            FileState::Conflicted => 0,
            FileState::Modified | FileState::TypeChange => 1,
            FileState::Renamed => 2,
            FileState::Added | FileState::Untracked => 3,
            FileState::Deleted => 4,
        };
        entries.sort_by_cached_key(|e| match e {
            Entry::File { idx, .. } => (rank(files[*idx].state), files[*idx].path.to_lowercase()),
            Entry::Folder { path, .. } => (9, path.to_lowercase()),
        });
    }
    let source = match side {
        Side::Unstaged => DiffSource::Unstaged,
        Side::Staged => DiffSource::Staged,
    };
    let open = tab.diff.as_ref().map(|d| (d.source.clone(), d.file.path.clone()));
    let picked_here = tab.picked_list == list;
    let mut click: Option<(usize, egui::Modifiers)> = None;
    let id = match side {
        Side::Unstaged => "unstaged_list",
        Side::Staged => "staged_list",
    };
    // show_rows sizes rows with the current item spacing: zero it first, or rows drift and the list ends early.
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_rows(ui, FILE_ROW_H, entries.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (pos, e) in entries.iter().enumerate().skip(range.start).take(range.len()) {
            match e {
                Entry::Folder { path, name, depth, collapsed } => {
                    let label = if side == Side::Staged { "Unstage" } else { "Stage" };
                    let (resp, button) = filelist::folder_row(ui, name, *depth, *collapsed, pal, Some(label));
                    if button {
                        acts.push(if side == Side::Staged { Action::UnstagePath(path.to_string()) } else { Action::StagePath(path.to_string()) });
                    } else if resp.clicked() {
                        acts.push(Action::ToggleFolder(list, path.to_string()));
                    }
                    let p = path.to_string();
                    resp.context_menu(|ui| {
                        crate::util::menu_pad(ui);
                        if ui.button(if side == Side::Staged { "Unstage folder" } else { "Stage folder" }).clicked() {
                            acts.push(if side == Side::Staged { Action::UnstagePath(p.clone()) } else { Action::StagePath(p.clone()) });
                            ui.close();
                        }
                        if ui.button("Show in folder").clicked() {
                            acts.push(Action::OpenInExplorer(Some(p.clone())));
                            ui.close();
                        }
                    });
                }
                Entry::File { idx, depth } => {
                    let f = &files[*idx];
                    let is_open = open.as_ref() == Some(&(source.clone(), f.path.clone()));
                    let selected = is_open || (picked_here && tab.picked.contains(&f.path));
                    let label = match (side, f.state) {
                        (Side::Staged, _) => "Unstage",
                        (_, FileState::Conflicted) => "Resolved",
                        _ => "Stage",
                    };
                    let (resp, button) = file_row(ui, f, pal, selected, Some(label), indent(tree, *depth));
                    // The button works on the whole selection when the row is part of it.
                    let group: Vec<FileChange> = if picked_here && tab.picked.len() > 1 && tab.picked.contains(&f.path) {
                        files.iter().filter(|x| tab.picked.contains(&x.path)).cloned().collect()
                    } else {
                        vec![f.clone()]
                    };
                    if button {
                        acts.push(if side == Side::Staged { Action::UnstageMany(group.clone()) } else { Action::StageMany(group.clone()) });
                    } else if resp.clicked() {
                        click = Some((pos, ui.input(|i| i.modifiers)));
                    }
                    file_menu(&resp, &group, Some(side), pal, acts);
                }
            }
        }
    });

    // Selection: plain click picks one file and shows it, Ctrl+click adds or removes,
    // Shift+click picks the range from the last plain click.
    if let Some((pos, mods)) = click {
        let Entry::File { idx, .. } = entries[pos] else { return };
        let f = files[idx].clone();
        if tab.picked_list != list {
            tab.picked.clear();
            tab.picked_list = list;
        }
        if mods.shift {
            let anchor_pos = tab
                .anchor
                .as_ref()
                .and_then(|a| entries.iter().position(|e| matches!(e, Entry::File { idx, .. } if &files[*idx].path == a)))
                .unwrap_or(pos);
            let (a, b) = (anchor_pos.min(pos), anchor_pos.max(pos));
            tab.picked.clear();
            for e in &entries[a..=b] {
                if let Entry::File { idx, .. } = e {
                    tab.picked.insert(files[*idx].path.clone());
                }
            }
        } else if mods.command {
            if !tab.picked.remove(&f.path) {
                tab.picked.insert(f.path.clone());
            }
            tab.anchor = Some(f.path.clone());
        } else {
            tab.picked.clear();
            tab.picked.insert(f.path.clone());
            tab.anchor = Some(f.path.clone());
            acts.push(Action::OpenDiff(source, f));
        }
    }
}

fn indent(tree: bool, depth: usize) -> f32 {
    if tree { depth as f32 * INDENT + 18.0 } else { 0.0 }
}

/// Right-click menu of one file or of the picked group. `side` is `None` for a past commit.
fn file_menu(resp: &egui::Response, group: &[FileChange], side: Option<Side>, pal: &Palette, acts: &mut Vec<Action>) {
    let files = group.to_vec();
    let one = files.len() == 1;
    let what = if one { "file".to_string() } else { format!("{} files", files.len()) };
    resp.context_menu(|ui| {
        crate::util::menu_pad(ui);
        match side {
            Some(Side::Unstaged) => {
                if ui.button(format!("Stage {what}")).clicked() {
                    acts.push(Action::StageMany(files.clone()));
                    ui.close();
                }
            }
            Some(Side::Staged) => {
                if ui.button(format!("Unstage {what}")).clicked() {
                    acts.push(Action::UnstageMany(files.clone()));
                    ui.close();
                }
            }
            None => {}
        }
        if one {
            let f = &files[0];
            if f.state == FileState::Conflicted {
                ui.separator();
                if ui.button("Keep my version (ours)").clicked() {
                    acts.push(Action::ResolveConflict(f.path.clone(), true));
                    ui.close();
                }
                if ui.button("Keep their version (theirs)").clicked() {
                    acts.push(Action::ResolveConflict(f.path.clone(), false));
                    ui.close();
                }
                if ui.button("Mark as resolved").clicked() {
                    acts.push(Action::Stage(f.clone()));
                    ui.close();
                }
                ui.separator();
            }
            if f.state != FileState::Untracked {
                if ui.button("File history").clicked() {
                    acts.push(Action::FileHistory(f.path.clone()));
                    ui.close();
                }
                if ui.button("Blame").clicked() {
                    acts.push(Action::FileBlame(f.path.clone()));
                    ui.close();
                }
            }
            if ui.button("Open file").clicked() {
                acts.push(Action::OpenFile(f.path.clone()));
                ui.close();
            }
            if ui.button("Show in folder").clicked() {
                acts.push(Action::OpenInExplorer(Some(f.path.clone())));
                ui.close();
            }
            if ui.button("Copy path").clicked() {
                acts.push(Action::Copy(f.path.clone()));
                ui.close();
            }
        } else if ui.button("Copy paths").clicked() {
            acts.push(Action::Copy(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join("\n")));
            ui.close();
        }
        if side == Some(Side::Unstaged) {
            ui.separator();
            if ui.button(RichText::new(format!("Discard changes ({what})")).color(pal.removed)).clicked() {
                let body = if one {
                    format!("Your changes to {} will be lost. This can't be undone.", files[0].path)
                } else {
                    format!("Your changes to {} files will be lost. This can't be undone.", files.len())
                };
                acts.push(Action::Dialog(Dialog::Confirm {
                    title: "Discard changes?".into(),
                    body,
                    confirm: "Discard".into(),
                    then: Box::new(Action::DiscardMany(files.clone())),
                }));
                ui.close();
            }
        }
    });
}

fn composer(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    // Tab row: "Commit" (active) and a stash shortcut, over a line.
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let commit_tab = Rect::from_min_size(bar.min, vec2(104.0, bar.height()));
    {
        let p = ui.painter();
        p.rect_filled(commit_tab, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, pal.card);
        let c = pos2(commit_tab.left() + 18.0, commit_tab.center().y);
        p.circle_stroke(c, 4.0, Stroke::new(1.6, pal.text));
        p.line_segment([c - vec2(9.0, 0.0), c - vec2(4.0, 0.0)], Stroke::new(1.6, pal.text));
        p.line_segment([c + vec2(4.0, 0.0), c + vec2(9.0, 0.0)], Stroke::new(1.6, pal.text));
        p.text(pos2(commit_tab.left() + 34.0, commit_tab.center().y), Align2::LEFT_CENTER, "Commit", egui::FontId::proportional(14.0), pal.text);
        p.rect_filled(Rect::from_min_size(pos2(bar.left(), bar.bottom() - 1.0), vec2(bar.width(), 1.0)), CornerRadius::ZERO, pal.border);
    }
    let stash_r = Rect::from_min_size(pos2(commit_tab.right() + 6.0, bar.top() + 3.0), vec2(30.0, bar.height() - 6.0));
    let sr = ui.interact(stash_r, ui.scope_id().with("composer_stash"), Sense::click());
    if sr.hovered() {
        ui.painter().rect_filled(stash_r, CornerRadius::same(5), pal.hover);
    }
    crate::icons::paint(ui.painter(), stash_r.shrink2(vec2(7.0, 6.0)), crate::icons::Icon::Stash, pal.text2());
    if sr.on_hover_text("Stash these changes").clicked() && !tab.snap.status.is_clean() {
        acts.push(Action::Dialog(Dialog::Stash { message: String::new(), include_untracked: true }));
    }
    ui.add_space(6.0);
    ui.checkbox(&mut tab.amend, RichText::new("Amend previous commit").color(pal.text2()));

    // Summary and description share one box, like a single message field.
    egui::Frame::new()
        .fill(pal.canvas)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let n = tab.summary.chars().count();
                ui.add(
                    egui::TextEdit::singleline(&mut tab.summary)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(15.0))
                        .hint_text("Commit summary")
                        .desired_width(ui.available_width() - 36.0),
                );
                let color = if n > 72 { pal.warn } else { pal.muted };
                ui.label(RichText::new((72i64 - n as i64).to_string()).size(12.0).color(color)).on_hover_text("Characters left before the usual 72");
            });
            ui.add(
                egui::TextEdit::multiline(&mut tab.description)
                    .frame(egui::Frame::NONE)
                    .hint_text("Description")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
        });
    ui.add_space(6.0);

    // Split button: Commit on the left (main action), Stage all on the right. Both halves
    // are placed in exact rectangles so neither is clipped and the outer corners stay round.
    let n = tab.snap.status.staged.len();
    let unstaged = tab.snap.status.unstaged.len();
    let has_msg = !tab.summary.trim().is_empty();
    let h = 36.0;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let gap = 3.0;
    let left_w = ((row.width() - gap) * 0.62).floor();
    let left_r = Rect::from_min_size(row.min, vec2(left_w, h));
    let right_r = Rect::from_min_max(pos2(left_r.right() + gap, row.top()), row.max);

    let label = if tab.amend {
        "Amend commit".to_string()
    } else if n == 0 {
        "Commit (nothing staged)".to_string()
    } else {
        format!("Commit {n} file{}", if n == 1 { "" } else { "s" })
    };
    let enabled = tab.amend || (n > 0 && has_msg);
    let commit = egui::Button::new(RichText::new(label).color(pal.accent_text).strong())
        .fill(pal.accent)
        .corner_radius(CornerRadius { nw: 6, sw: 6, ne: 2, se: 2 });
    let resp = ui.put(left_r, |ui: &mut egui::Ui| ui.add_enabled(enabled, commit));
    if resp.on_hover_text("Commit the staged files (Ctrl+Enter)").clicked() {
        acts.push(Action::Commit);
    }
    if enabled && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
        acts.push(Action::Commit);
    }

    let stage = egui::Button::new(RichText::new(format!("Stage all ({unstaged})")).color(pal.text))
        .fill(pal.card)
        .corner_radius(CornerRadius { nw: 2, sw: 2, ne: 6, se: 6 });
    let resp = ui.put(right_r, |ui: &mut egui::Ui| ui.add_enabled(unstaged > 0, stage));
    if resp.on_hover_text("Stage every unstaged change").clicked() {
        acts.push(Action::StageAll);
    }
}

fn commit(ui: &mut egui::Ui, tab: &mut RepoTab, sha: &str, pal: &Palette, tree: bool, acts: &mut Vec<Action>) {
    let Some(c) = tab.commit(sha).cloned() else {
        ui.label("This commit is no longer in the history.");
        return;
    };
    let Some(detail) = tab.detail.as_ref().filter(|d| d.sha == sha) else {
        loading(ui, "Loading commit…");
        return;
    };
    egui::Panel::top("commit_card")
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 6 }))
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| {
            card(pal).show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Click "commit 1a2b3c4" to copy its full SHA or its short id.
                let title = ui
                    .add(egui::Label::new(RichText::new(format!("commit {}  ▾", git::short(sha))).monospace().color(pal.muted)).sense(Sense::click()))
                    .on_hover_text("Click to copy");
                egui::Popup::menu(&title).show(|ui| {
                    crate::util::menu_pad(ui);
                    if ui.button(format!("Copy SHA  ({}…)", &sha[..sha.len().min(12)])).clicked() {
                        acts.push(Action::Copy(sha.to_string()));
                        ui.close();
                    }
                    if ui.button(format!("Copy id  ({})", git::short(sha))).clicked() {
                        acts.push(Action::Copy(git::short(sha).to_string()));
                        ui.close();
                    }
                });
                let mut lines = detail.message.lines();
                let title = lines.next().unwrap_or_default();
                ui.label(RichText::new(title).size(17.0).strong());
                let body: String = lines.collect::<Vec<_>>().join("\n");
                if !body.trim().is_empty() {
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                        ui.label(RichText::new(body.trim()).color(pal.text));
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                    avatar(ui.painter(), r.center(), 15.0, &c.author, &c.email);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&c.author).strong());
                        ui.label(RichText::new(abs_time(c.time)).size(11.5).color(pal.muted));
                    });
                });
                if !detail.committer.is_empty() && !detail.committer.starts_with(&c.author) {
                    ui.label(RichText::new(format!("Committed by {}", detail.committer)).size(11.5).color(pal.muted));
                }
                if !c.parents.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(if c.parents.len() > 1 { "Parents" } else { "Parent" }).size(11.5).color(pal.muted));
                        for p in &c.parents {
                            if ui.link(RichText::new(git::short(p)).monospace()).clicked() {
                                acts.push(Action::SelectRef(p.clone()));
                            }
                        }
                    });
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                section_title(ui, &format!("CHANGED FILES  ·  {}", detail.files.len()), pal);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| view_switch(ui, pal, tree, acts));
            });
            if detail.files.is_empty() {
                ui.label(RichText::new("No file changes (for example a merge without conflicts).").color(pal.muted).size(12.0));
            }
        });

    let source = DiffSource::Commit { sha: sha.to_string(), parent: c.parents.first().cloned() };
    let open = tab.diff.as_ref().map(|d| (d.source.clone(), d.file.path.clone()));
    let files = &detail.files;
    let entries = filelist::flatten(files, tree, &tab.collapsed[ListId::Commit as usize]);
    // show_rows sizes rows with the current item spacing: zero it first, or rows drift and the list ends early.
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, FILE_ROW_H, entries.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for e in &entries[range] {
            match e {
                Entry::Folder { path, name, depth, collapsed } => {
                    let (resp, _) = filelist::folder_row(ui, name, *depth, *collapsed, pal, None);
                    if resp.clicked() {
                        acts.push(Action::ToggleFolder(ListId::Commit, path.to_string()));
                    }
                }
                Entry::File { idx, depth } => {
                    let f = &files[*idx];
                    let selected = open.as_ref() == Some(&(source.clone(), f.path.clone()));
                    let (resp, _) = file_row(ui, f, pal, selected, None, indent(tree, *depth));
                    if resp.clicked() {
                        acts.push(Action::OpenDiff(source.clone(), f.clone()));
                    }
                    file_menu(&resp, std::slice::from_ref(f), None, pal, acts);
                }
            }
        }
    });
}

const DIFF_ROW_H: f32 = 21.0;

/// Full diff of one file, replacing the graph in the middle of the window. A toolbar
/// switches between hunk, inline and split views, jumps between changes and opens the
/// file's history or blame. Pictures also get a "Code / Render" switch.
/// `embedded` is set inside the file history view, which has its own header.
pub fn diff_view(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, mode: DiffMode, embedded: bool, acts: &mut Vec<Action>) {
    let jump = tab.diff.as_mut().and_then(|d| d.jump.take());
    let Some(dv) = &tab.diff else { return };
    let source_label = match &dv.source {
        DiffSource::Unstaged => "Unstaged changes".to_string(),
        DiffSource::Staged => "Staged changes".to_string(),
        DiffSource::Commit { sha, .. } => format!("In commit {}", git::short(sha)),
    };
    ui.horizontal(|ui| {
        if !embedded {
            if ui.button("← Back").on_hover_text("Back to the history (Esc)").clicked() {
                acts.push(Action::CloseDiff);
            }
            ui.add_space(4.0);
        }
        ui.label(RichText::new(&dv.file.path).strong().size(15.0));
        ui.label(RichText::new(source_label).color(pal.muted));
        if let Some(p) = &dv.parsed {
            if !p.binary {
                ui.label(RichText::new(format!("+{}", p.added)).color(pal.added).strong());
                ui.label(RichText::new(format!("−{}", p.removed)).color(pal.removed).strong());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match dv.source {
            DiffSource::Unstaged => {
                if ui.add(accent_button("Stage file", pal)).clicked() {
                    acts.push(Action::Stage(dv.file.clone()));
                }
            }
            DiffSource::Staged => {
                if ui.button("Unstage file").clicked() {
                    acts.push(Action::Unstage(dv.file.clone()));
                }
            }
            DiffSource::Commit { .. } => {}
        });
    });

    // Toolbar: history tools, previous/next change, the three views, code/render.
    ui.horizontal(|ui| {
        let tracked = dv.file.state != FileState::Untracked;
        if !embedded && tracked {
            if group_button(ui, pal, "Blame", false).on_hover_text("Who last changed each line").clicked() {
                acts.push(Action::FileBlame(dv.file.path.clone()));
            }
            if group_button(ui, pal, "History", false).on_hover_text("Every commit that changed this file").clicked() {
                acts.push(Action::FileHistory(dv.file.path.clone()));
            }
            ui.add_space(8.0);
        }
        let showing_code = !(dv.render && dv.images.is_some());
        if showing_code {
            if group_button(ui, pal, "↑", false).on_hover_text("Previous change").clicked() {
                acts.push(Action::Jump(false));
            }
            if group_button(ui, pal, "↓", false).on_hover_text("Next change").clicked() {
                acts.push(Action::Jump(true));
            }
            ui.add_space(8.0);
            let mut m = mode;
            if segmented(ui, pal, &mut m, &[(DiffMode::Hunk, "Hunk"), (DiffMode::Inline, "Inline"), (DiffMode::Split, "Split")]) {
                acts.push(Action::SetDiffMode(m));
            }
        }
        // Code / Render always sits at the right end of the toolbar.
        if dv.images.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut render = dv.render;
                if segmented(ui, pal, &mut render, &[(false, "</> Code"), (true, "Render")]) {
                    acts.push(Action::SetRender(render));
                }
            });
        }
    });
    if !embedded && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        acts.push(Action::CloseDiff);
    }
    ui.add_space(6.0);

    if dv.render {
        if let Some(images) = &dv.images {
            image_view(ui, images, &dv.file, pal);
            return;
        }
    }

    let Some(parsed) = &dv.parsed else {
        loading(ui, "Computing the differences… (big files such as Unity scenes can take a moment; the window stays usable)");
        return;
    };
    if parsed.truncated {
        ui.label(
            RichText::new(format!(
                "This diff is very large; only the first {} MB are shown. Stage or discard still apply to the whole file.",
                git::MAX_DIFF_BYTES / (1024 * 1024)
            ))
            .color(pal.warn),
        );
    }
    if parsed.binary {
        let hint = if dv.images.is_some() { " Switch to Render to see the picture." } else { "" };
        ui.label(RichText::new(format!("This is a binary file; there is no text to compare.{hint}")).color(pal.muted));
        return;
    }
    if parsed.hunks.is_empty() {
        ui.label(RichText::new("No differences to show (the file may be empty or only its mode changed).").color(pal.muted));
        return;
    }

    let source = dv.source.clone();
    let is_untracked = dv.file.state == FileState::Untracked;
    // Hunk buttons need a normal hunk diff; inline/split diffs hold the whole file in one hunk.
    let hunk_tools = mode == DiffMode::Hunk && !dv.whole;
    let content_w = 130.0 + parsed.max_chars as f32 * 7.6;
    let mut scroll = egui::ScrollArea::both().auto_shrink([false, false]).id_salt(("diff", mode as u8));
    if let Some(row) = jump {
        scroll = scroll.vertical_scroll_offset(row as f32 * DIFF_ROW_H);
    }
    // show_rows sizes rows with the current item spacing: zero it first, or rows drift and the list ends early.
    ui.spacing_mut().item_spacing.y = 0.0;
    let top = match mode {
        DiffMode::Split => {
            scroll = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("diff_split");
            if let Some(row) = jump {
                scroll = scroll.vertical_scroll_offset(row as f32 * DIFF_ROW_H);
            }
            scroll
                .show_rows(ui, DIFF_ROW_H, parsed.split_rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let start = range.start;
                    let w = ui.available_width();
                    for r in &parsed.split_rows[range] {
                        let (rect, _) = ui.allocate_exact_size(vec2(w, DIFF_ROW_H), Sense::hover());
                        match *r {
                            SplitRow::Gap => {}
                            SplitRow::Header(hi) => {
                                if !dv.whole {
                                    ui.painter().rect_filled(rect, CornerRadius::same(8), pal.card);
                                    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, &parsed.hunks[hi].header, FontId::monospace(11.5), pal.muted);
                                }
                            }
                            SplitRow::Pair(l, r) => {
                                let half = (rect.width() - 6.0) / 2.0;
                                let lr = Rect::from_min_size(rect.min, vec2(half, rect.height()));
                                let rr = Rect::from_min_size(rect.min + vec2(half + 6.0, 0.0), vec2(half, rect.height()));
                                split_cell(ui, lr, l.map(|(h, i)| &parsed.hunks[h].lines[i]), true, pal);
                                split_cell(ui, rr, r.map(|(h, i)| &parsed.hunks[h].lines[i]), false, pal);
                            }
                        }
                    }
                    start
                })
                .inner
        }
        _ => {
            let rows = if mode == DiffMode::Inline { &parsed.inline_rows } else { &parsed.rows };
            scroll
                .show_rows(ui, DIFF_ROW_H, rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let start = range.start;
                    let w = content_w.max(ui.available_width());
                    for r in &rows[range] {
                        let (rect, _) = ui.allocate_exact_size(vec2(w, DIFF_ROW_H), Sense::hover());
                        match *r {
                            DiffRow::Gap => {}
                            DiffRow::Header(hi) => {
                                if hunk_tools {
                                    hunk_header(ui, rect, parsed, hi, &source, is_untracked, pal, acts);
                                } else {
                                    ui.painter().rect_filled(rect, CornerRadius::same(8), pal.card);
                                    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, &parsed.hunks[hi].header, FontId::monospace(11.5), pal.muted);
                                }
                            }
                            DiffRow::Line(hi, li) => diff_line(ui.painter(), rect, &parsed.hunks[hi].lines[li], pal),
                        }
                    }
                    start
                })
                .inner
        }
    };
    if let Some(d) = tab.diff.as_mut() {
        d.top_row = top;
    }
}

/// One side of the split view: line number and text, red on the left for removed lines,
/// green on the right for added ones, an empty hatch where the other side has no line.
fn split_cell(ui: &egui::Ui, r: Rect, line: Option<&crate::diff::DiffLine>, left: bool, pal: &Palette) {
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    let font = FontId::monospace(12.5);
    let Some(l) = line else {
        p.rect_filled(r, CornerRadius::ZERO, pal.row_alt);
        return;
    };
    let (bg, no) = match (l.kind, left) {
        (LineKind::Removed, _) => (Some(pal.removed_bg), l.old_no),
        (LineKind::Added, _) => (Some(pal.added_bg), l.new_no),
        (_, true) => (None, l.old_no),
        (_, false) => (None, l.new_no),
    };
    if let Some(bg) = bg {
        p.rect_filled(r, CornerRadius::ZERO, bg);
    }
    let y = r.center().y;
    p.text(pos2(r.left() + 44.0, y), Align2::RIGHT_CENTER, no.map(|n| n.to_string()).unwrap_or_default(), font.clone(), pal.muted);
    let fg = if l.kind == LineKind::Note { pal.muted } else { pal.text };
    p.text(pos2(r.left() + 56.0, y), Align2::LEFT_CENTER, &l.text, font, fg);
}

/// A compact bordered button like the ones grouped in the diff toolbar.
fn group_button(ui: &mut egui::Ui, pal: &Palette, text: &str, on: bool) -> egui::Response {
    let b = egui::Button::new(RichText::new(text).size(12.0).color(if on { pal.text } else { pal.text2() }))
        .fill(if on { pal.card } else { pal.canvas })
        .corner_radius(CornerRadius::same(8))
        .min_size(vec2(0.0, 24.0));
    ui.add(b)
}

const HISTORY_ROW_H: f32 = 46.0;
const BLAME_ROW_H: f32 = 20.0;

/// File history or blame, shown in the middle tile instead of the graph.
pub fn file_view(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, mode: DiffMode, acts: &mut Vec<Action>) {
    let Some(fv) = &tab.file_view else { return };
    let path = fv.path.clone();
    let kind = fv.kind;
    ui.horizontal(|ui| {
        if ui.button("← Back").on_hover_text("Back to the history (Esc)").clicked() {
            acts.push(Action::CloseFileView);
        }
        ui.add_space(4.0);
        ui.label(RichText::new(&path).strong().size(15.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut k = kind;
            if segmented(ui, pal, &mut k, &[(FileViewKind::Blame, "Blame"), (FileViewKind::History, "History")]) {
                acts.push(match k {
                    FileViewKind::Blame => Action::FileBlame(path.clone()),
                    FileViewKind::History => Action::FileHistory(path.clone()),
                });
            }
        });
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        acts.push(Action::CloseFileView);
    }
    ui.add_space(6.0);

    match kind {
        FileViewKind::History => {
            let entries = match &fv.history {
                None => return loading(ui, "Reading the file's history…"),
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(pal.warn));
                    return;
                }
                Some(Ok(v)) => v,
            };
            if entries.is_empty() {
                ui.label(RichText::new("No commit changed this file yet.").color(pal.muted));
                return;
            }
            let selected = fv.selected;
            egui::Panel::left("history_list")
                .default_size(320.0)
                .size_range(220.0..=520.0)
                .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 10, top: 0, bottom: 0 }))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("{} commits", entries.len())).color(pal.muted).size(12.0));
                    // show_rows sizes rows with the current item spacing: zero it first, or rows drift and the list ends early.
                    ui.spacing_mut().item_spacing.y = 0.0;
                    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("history_rows").show_rows(ui, HISTORY_ROW_H, entries.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for i in range {
                            let e = &entries[i];
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), HISTORY_ROW_H), Sense::click());
                            let p = ui.painter_at(rect);
                            if selected == Some(i) {
                                p.rect_filled(rect.shrink(1.0), CornerRadius::same(10), pal.selected);
                            } else if resp.hovered() {
                                p.rect_filled(rect.shrink(1.0), CornerRadius::same(10), pal.hover);
                            }
                            avatar(&p, pos2(rect.left() + 18.0, rect.center().y), 11.0, &e.commit.author, &e.commit.email);
                            p.text(pos2(rect.left() + 38.0, rect.top() + 14.0), Align2::LEFT_CENTER, &e.commit.summary, FontId::proportional(13.0), pal.text);
                            let meta = format!("{} · {} · {}", e.commit.author, rel_time(e.commit.time), git::short(&e.commit.sha));
                            p.text(pos2(rect.left() + 38.0, rect.bottom() - 13.0), Align2::LEFT_CENTER, meta, FontId::proportional(11.5), pal.muted);
                            if resp.clicked() {
                                acts.push(Action::PickHistory(i));
                            }
                        }
                    });
                });
            if tab.diff.is_some() {
                diff_view(ui, tab, pal, mode, true, acts);
            } else {
                ui.label(RichText::new("Pick a commit on the left to see what it changed in this file.").color(pal.muted));
            }
        }
        FileViewKind::Blame => {
            let lines = match &fv.blame {
                None => return loading(ui, "Reading who changed each line…"),
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(pal.warn));
                    return;
                }
                Some(Ok(v)) => v,
            };
            // show_rows sizes rows with the current item spacing: zero it first, or rows drift and the list ends early.
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("blame_rows").show_rows(ui, BLAME_ROW_H, lines.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let w = ui.available_width();
                let gutter = 300.0;
                for i in range {
                    let l = &lines[i];
                    let (rect, _) = ui.allocate_exact_size(vec2(w, BLAME_ROW_H), Sense::hover());
                    // Alternate shading per commit block so runs of lines read as one change.
                    let p = ui.painter_at(rect);
                    if l.block % 2 == 0 {
                        p.rect_filled(Rect::from_min_size(rect.min, vec2(gutter, rect.height())), CornerRadius::ZERO, pal.row_alt);
                    }
                    if l.starts_block {
                        let gr = Rect::from_min_size(rect.min, vec2(gutter - 8.0, rect.height()));
                        let resp = ui.interact(gr, ui.scope_id().with(("blame", i)), Sense::click());
                        let col = pal.lane((l.sha.as_bytes()[0] as usize) % 8);
                        p.text(pos2(rect.left() + 6.0, rect.center().y), Align2::LEFT_CENTER, git::short(&l.sha), FontId::monospace(11.5), col);
                        let who = format!("{} · {}", l.author, rel_time(l.time));
                        p.with_clip_rect(gr).text(pos2(rect.left() + 70.0, rect.center().y), Align2::LEFT_CENTER, who, FontId::proportional(12.0), pal.text2());
                        if resp.on_hover_text(format!("{}\nClick to show this commit", l.summary)).clicked() {
                            acts.push(Action::CloseFileView);
                            acts.push(Action::SelectRef(l.sha.clone()));
                        }
                    }
                    let font = FontId::monospace(12.5);
                    p.text(pos2(rect.left() + gutter + 40.0, rect.center().y), Align2::RIGHT_CENTER, (i + 1).to_string(), font.clone(), pal.muted);
                    p.text(pos2(rect.left() + gutter + 52.0, rect.center().y), Align2::LEFT_CENTER, &l.text, font, pal.text);
                }
            });
        }
    }
}

/// Before / after pictures side by side, each fitted into its half on a checkerboard so
/// transparent pixels are visible.
fn image_view(ui: &mut egui::Ui, images: &ImageState, file: &FileChange, pal: &Palette) {
    let ImageState::Ready { before, after } = images else {
        loading(ui, "Loading the picture…");
        return;
    };
    let mut sides: Vec<(&str, &Result<ShownImage, String>, Color32)> = Vec::new();
    if let Some(b) = before {
        sides.push(("Before", b, pal.removed));
    }
    if let Some(a) = after {
        let label = if before.is_some() { "After" } else { "Added" };
        sides.push((label, a, pal.added));
    }
    if sides.is_empty() {
        let text = if file.state == FileState::Deleted { "This picture was deleted." } else { "No picture to show." };
        ui.label(RichText::new(text).color(pal.muted));
        return;
    }
    if before.is_some() && after.is_none() {
        sides[0].0 = "Deleted";
    }
    let avail = ui.available_size();
    let n = sides.len() as f32;
    let gap = 12.0;
    let col_w = (avail.x - gap * (n - 1.0)) / n;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (label, side, color) in sides {
            let (rect, _) = ui.allocate_exact_size(vec2(col_w, avail.y), Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, CornerRadius::same(8), pal.card);
            p.text(rect.left_top() + vec2(14.0, 16.0), Align2::LEFT_CENTER, label, FontId::proportional(13.0), color);
            let area = Rect::from_min_max(rect.left_top() + vec2(14.0, 34.0), rect.right_bottom() - vec2(14.0, 34.0));
            match side {
                Ok(img) => {
                    let tex = img.texture.size_vec2();
                    let scale = (area.width() / tex.x).min(area.height() / tex.y).min(4.0);
                    let shown = Rect::from_center_size(area.center(), tex * scale);
                    checkerboard(&p, shown, pal);
                    p.image(img.texture.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    p.rect_stroke(shown, CornerRadius::ZERO, Stroke::new(1.0, pal.border), egui::StrokeKind::Outside);
                    let info = format!("{} × {} px  ·  {}", img.width, img.height, human_size(img.bytes));
                    p.text(pos2(rect.center().x, rect.bottom() - 17.0), Align2::CENTER_CENTER, info, FontId::proportional(12.0), pal.muted);
                }
                Err(e) => {
                    p.text(area.center(), Align2::CENTER_CENTER, e, FontId::proportional(13.0), pal.warn);
                }
            }
        }
    });
}

fn checkerboard(p: &egui::Painter, r: Rect, pal: &Palette) {
    let size = 10.0;
    p.rect_filled(r, CornerRadius::ZERO, pal.panel);
    let (cols, rows) = ((r.width() / size).ceil() as usize, (r.height() / size).ceil() as usize);
    for y in 0..rows {
        for x in (y % 2..cols).step_by(2) {
            let cell = Rect::from_min_size(r.min + vec2(x as f32 * size, y as f32 * size), vec2(size, size)).intersect(r);
            p.rect_filled(cell, CornerRadius::ZERO, pal.hover);
        }
    }
}

fn human_size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

#[allow(clippy::too_many_arguments)]
fn hunk_header(ui: &mut egui::Ui, rect: Rect, parsed: &ParsedDiff, hi: usize, source: &DiffSource, is_untracked: bool, pal: &Palette, acts: &mut Vec<Action>) {
    let visible = rect.intersect(ui.clip_rect());
    ui.painter().rect_filled(rect, CornerRadius::same(8), pal.card);
    ui.painter().text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        &parsed.hunks[hi].header,
        FontId::monospace(11.5),
        pal.muted,
    );
    // Buttons stay at the right edge of the visible area even when scrolled sideways.
    let mut x = visible.right() - 8.0;
    let mut button = |ui: &mut egui::Ui, text: RichText, w: f32| {
        let r = Rect::from_min_size(pos2(x - w, rect.top() + 1.0), vec2(w, rect.height() - 2.0));
        x -= w + 6.0;
        ui.put(r, egui::Button::new(text.size(11.5)).corner_radius(CornerRadius::same(8))).clicked()
    };
    match source {
        DiffSource::Unstaged if !is_untracked => {
            if button(ui, RichText::new("Stage hunk"), 86.0) {
                acts.push(Action::Hunk { index: hi, mode: HunkMode::Stage });
            }
            if button(ui, RichText::new("Discard hunk").color(pal.removed), 92.0) {
                acts.push(Action::Dialog(Dialog::Confirm {
                    title: "Discard this hunk?".into(),
                    body: "These lines will go back to how they were. This can't be undone.".into(),
                    confirm: "Discard".into(),
                    then: Box::new(Action::Hunk { index: hi, mode: HunkMode::Discard }),
                }));
            }
        }
        DiffSource::Staged => {
            if button(ui, RichText::new("Unstage hunk"), 96.0) {
                acts.push(Action::Hunk { index: hi, mode: HunkMode::Unstage });
            }
        }
        _ => {}
    }
}

fn diff_line(p: &egui::Painter, rect: Rect, l: &crate::diff::DiffLine, pal: &Palette) {
    let font = FontId::monospace(12.5);
    let (bg, sign, sign_color) = match l.kind {
        LineKind::Added => (Some(pal.added_bg), "+", pal.added),
        LineKind::Removed => (Some(pal.removed_bg), "−", pal.removed),
        LineKind::Context | LineKind::Note => (None, " ", pal.muted),
    };
    if let Some(bg) = bg {
        p.rect_filled(rect, CornerRadius::ZERO, bg);
    }
    let fg = if l.kind == LineKind::Note { pal.muted } else { pal.text };
    let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
    let y = rect.center().y;
    p.text(pos2(rect.left() + 46.0, y), Align2::RIGHT_CENTER, num(l.old_no), font.clone(), pal.muted);
    p.text(pos2(rect.left() + 92.0, y), Align2::RIGHT_CENTER, num(l.new_no), font.clone(), pal.muted);
    p.text(pos2(rect.left() + 104.0, y), Align2::LEFT_CENTER, sign, font.clone(), sign_color);
    p.text(pos2(rect.left() + 120.0, y), Align2::LEFT_CENTER, &l.text, font, fg);
}

