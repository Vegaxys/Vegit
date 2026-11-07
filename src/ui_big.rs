//! Interface of the "big pieces" (design doc, phase 3): history edits (menu actions and the
//! "Edit history" panel), conflict cards, worktrees and Git LFS.

use std::collections::HashMap;

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};

use crate::app::{Action, Dialog, RepoTab};
use crate::conflict::{self, Choice};
use crate::git::{self, FileChange, RefKind};
use crate::lfs;
use crate::rewrite::{self, Change};
use crate::theme::Palette;
use crate::util::accent_button;

/// Width of the dialogs drawn here.
pub fn width(dialog: &Dialog) -> Option<f32> {
    match dialog {
        Dialog::CommitMessage { .. } | Dialog::Fixup { .. } | Dialog::RemoveWorktree { .. } => Some(520.0),
        Dialog::Worktree(_) | Dialog::LfsPreset { .. } => Some(480.0),
        _ => None,
    }
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(20.0).strong());
}

fn danger(text: &str, pal: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(Color32::WHITE)).fill(pal.removed)
}

// ---------------------------------------------------------------------------------------------
// History edits
// ---------------------------------------------------------------------------------------------

/// Commits reachable from HEAD by first parents among the loaded ones: the current branch.
fn current_branch_commits(tab: &RepoTab) -> Vec<String> {
    let by_sha: HashMap<&str, &git::Commit> = tab.snap.commits.iter().map(|c| (c.sha.as_str(), c)).collect();
    let mut out = Vec::new();
    let mut cur = tab.snap.head_sha.clone();
    while let Some(s) = cur {
        let Some(c) = by_sha.get(s.as_str()) else { break };
        out.push(s.clone());
        cur = c.parents.first().cloned();
    }
    out
}

/// The history items of a commit's right-click menu, for commits of the current branch
/// (doc: direct actions, each one click with an Undo, and "Edit history from here…").
pub fn rewrite_menu(ui: &mut egui::Ui, tab: &RepoTab, sha: &str, acts: &mut Vec<Action>) {
    if tab.snap.head_branch.is_none() || tab.snap.pending.is_some() {
        return;
    }
    let chain = current_branch_commits(tab);
    let Some(pos) = chain.iter().position(|s| s == sha) else { return };
    let commit = tab.commit(sha);
    if commit.is_some_and(|c| c.parents.len() > 1) {
        return; // a merge commit: not editable in this version
    }
    let has_below = chain.get(pos + 1).is_some();
    let pushed = tab.snap.current_ref().is_some_and(|r| r.upstream.is_some() && pos >= r.ahead as usize);
    let warn = if pushed { "\nAlready on the server: you will need to force push afterwards." } else { "" };
    ui.separator();
    ui.label(RichText::new("History").weak());
    let tip = |c: Change| format!("{}{warn}", c.git_term());
    if ui.button("Edit message…").on_hover_text(tip(Change::Reword)).clicked() {
        acts.push(Action::AskCommitMessage { sha: sha.to_string(), change: Change::Reword });
        ui.close();
    }
    ui.add_enabled_ui(has_below, |ui| {
        ui.menu_button("Combine with the commit below", |ui| {
            crate::util::menu_pad(ui);
            if ui.button("Keep both messages").on_hover_text(tip(Change::Squash)).clicked() {
                acts.push(Action::RewriteCommit { sha: sha.to_string(), change: Change::Squash, message: String::new() });
                ui.close();
            }
            if ui.button("Keep the message of the one below").on_hover_text(tip(Change::Fixup)).clicked() {
                acts.push(Action::RewriteCommit { sha: sha.to_string(), change: Change::Fixup, message: String::new() });
                ui.close();
            }
            if ui.button("Write a new message…").on_hover_text(tip(Change::SquashNew)).clicked() {
                acts.push(Action::AskCommitMessage { sha: sha.to_string(), change: Change::SquashNew });
                ui.close();
            }
        });
    });
    if ui.button("Remove this commit").on_hover_text(tip(Change::Drop)).clicked() {
        acts.push(Action::RewriteCommit { sha: sha.to_string(), change: Change::Drop, message: String::new() });
        ui.close();
    }
    if pos > 0 && ui.button("Move up").on_hover_text(format!("Swap with the newer commit above (reorder){warn}")).clicked() {
        acts.push(Action::MoveCommit { sha: sha.to_string(), up: true });
        ui.close();
    }
    if has_below && ui.button("Move down").on_hover_text(format!("Swap with the older commit below (reorder){warn}")).clicked() {
        acts.push(Action::MoveCommit { sha: sha.to_string(), up: false });
        ui.close();
    }
    if ui.button("Edit history from here…").on_hover_text("Reorder, combine, remove or reword every commit from here to the latest, then apply once (interactive rebase)").clicked() {
        acts.push(Action::OpenHistoryEdit(sha.to_string()));
        ui.close();
    }
}

/// Payload of a row dragged in the Edit history list.
#[derive(Clone, Copy, Debug)]
struct RowDrag(usize);

/// "Edit history from here": the detail panel becomes the list of commits, newest first.
pub fn history_panel(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let Some(h) = tab.history_edit.as_mut() else { return };
    ui.add_space(6.0);
    ui.label(RichText::new("Edit history").size(18.0).strong());
    ui.label(
        RichText::new(format!("From {} to the latest commit, newest on top. Drag to reorder. Nothing changes before Apply.", git::short(&h.from)))
            .size(12.0)
            .color(pal.muted),
    );
    if h.pushed > 0 {
        let remote = h.upstream.as_deref().and_then(|u| u.split('/').next()).unwrap_or("the server");
        let n = h.pushed;
        egui::Frame::new().fill(pal.warn.gamma_multiply(0.15)).stroke(Stroke::new(1.0, pal.warn)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.label(format!("{n} of these commits {} already on {remote}. You will need to force push.", if n == 1 { "is" } else { "are" }));
        });
    }
    ui.add_space(4.0);
    let mut moved: Option<(usize, usize)> = None;
    let len = h.plan.len();
    let footer_h = 70.0;
    egui::ScrollArea::vertical().max_height((ui.available_height() - footer_h).max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        for i in (0..len).rev() {
            let item = &mut h.plan[i];
            let row = egui::Frame::new().fill(pal.card).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let handle = ui.add(egui::Label::new(RichText::new("≡").size(16.0).color(pal.muted)).sense(egui::Sense::drag())).on_hover_text("Drag to reorder");
                    handle.dnd_set_drag_payload(RowDrag(i));
                    if ui.add_enabled(i + 1 < len, egui::Button::new("↑").small()).on_hover_text("Move up (newer)").clicked() {
                        moved = Some((i, i + 1));
                    }
                    if ui.add_enabled(i > 0, egui::Button::new("↓").small()).on_hover_text("Move down (older)").clicked() {
                        moved = Some((i, i - 1));
                    }
                    ui.label(RichText::new(git::short(&item.sha)).monospace().color(pal.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::ComboBox::from_id_salt(("history_change", &item.sha)).width(170.0).selected_text(short_label(item.change)).show_ui(ui, |ui| {
                            crate::util::menu_pad(ui);
                            for c in Change::ALL {
                                ui.selectable_value(&mut item.change, c, c.label()).on_hover_text(c.git_term());
                            }
                        });
                    });
                });
                // The summary on its own line, so a narrow panel never squeezes it out.
                let text = RichText::new(&item.summary);
                let text = if item.change == Change::Drop { text.strikethrough().color(pal.muted) } else { text.color(pal.text) };
                ui.add(egui::Label::new(text).truncate());
                if item.change.needs_message() {
                    ui.add(egui::TextEdit::multiline(&mut item.message).desired_rows(2).desired_width(f32::INFINITY).hint_text("New message"));
                }
            });
            if let Some(p) = row.response.dnd_release_payload::<RowDrag>() {
                moved = Some((p.0, i));
            }
            if row.response.dnd_hover_payload::<RowDrag>().is_some() {
                ui.painter().rect_stroke(row.response.rect, CornerRadius::same(6), Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
            }
            ui.add_space(3.0);
        }
    });
    if let Some((from, to)) = moved {
        if from != to && to < len {
            let item = h.plan.remove(from);
            h.plan.insert(to, item);
        }
    }
    let check = rewrite::check(&h.plan, &h.history);
    let n = rewrite::change_count(&h.plan, &h.history);
    if let Err(e) = &check {
        ui.label(RichText::new(e).color(pal.removed).size(12.0));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = format!("Apply {n} change{}", if n == 1 { "" } else { "s" });
            if ui.add_enabled(n > 0 && check.is_ok(), accent_button(&label, pal)).on_hover_text("Runs one interactive rebase; Undo puts everything back").clicked() {
                acts.push(Action::HistoryApply);
            }
            if ui.button("Cancel").clicked() {
                acts.push(Action::HistoryCancel);
            }
        });
    });
}

fn short_label(c: Change) -> &'static str {
    match c {
        Change::Keep => "Keep",
        Change::Reword => "Edit message",
        Change::Squash => "Combine (both messages)",
        Change::Fixup => "Combine (keep below)",
        Change::SquashNew => "Combine (new message)",
        Change::Drop => "Remove",
        Change::Edit => "Stop to change files",
    }
}

// ---------------------------------------------------------------------------------------------
// Conflicts
// ---------------------------------------------------------------------------------------------

/// A conflicted file as cards: both versions side by side, named after the branches.
pub fn conflict_view(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let Some(cv) = tab.conflict.as_mut() else { return };
    let path = cv.path.clone();
    ui.horizontal(|ui| {
        if ui.button("← Back").clicked() {
            acts.push(Action::CloseConflict);
        }
        ui.label(RichText::new(&path).strong());
        if let Some(segs) = &cv.segs {
            let left = conflict::count(segs);
            ui.label(RichText::new(format!("{left} of {} conflict{} left", cv.total.max(left), if cv.total == 1 { "" } else { "s" })).color(pal.warn));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Open in my merge tool").on_hover_text("git mergetool: the tool set in git's merge.tool").clicked() {
                acts.push(Action::OpenMergeTool(path.clone()));
            }
            ui.checkbox(&mut cv.show_base, "Show the original").on_hover_text("The version before both changes (zdiff3)");
        });
    });
    ui.add_space(6.0);
    let Some(segs) = cv.segs.clone() else {
        crate::ui_detail::loading(ui, "Reading the conflicts…");
        return;
    };
    let blocks = conflict::blocks(&segs);
    if blocks.is_empty() {
        ui.label("No conflict markers are left in this file.");
        if ui.add(accent_button("Mark as resolved", pal)).clicked() {
            acts.push(Action::Stage(FileChange { path: path.clone(), old_path: None, state: git::FileState::Conflicted }));
            acts.push(Action::CloseConflict);
        }
        return;
    }
    let (ours, theirs) = cv.names.clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (k, b) in blocks.iter().enumerate() {
            egui::Frame::new().fill(pal.card).stroke(Stroke::new(1.0, pal.border)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("Conflict {}", k + 1)).strong());
                ui.columns(2, |cols| {
                    for (col, name, text, color) in [(0, &ours, &b.ours, pal.accent), (1, &theirs, &b.theirs, pal.added)] {
                        let c = &mut cols[col];
                        c.label(RichText::new(name.as_str()).strong().color(color));
                        code_box(c, text, pal, ("c", k, col));
                    }
                });
                if cv.show_base {
                    match &b.base {
                        Some(base) => {
                            ui.label(RichText::new("The original (before both changes)").color(pal.muted));
                            code_box(ui, base, pal, ("b", k, 2));
                        }
                        None => {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new("The original isn't written in this file.").color(pal.muted));
                                if ui.button("Recreate the conflict with it").on_hover_text("git checkout --conflict=zdiff3: your edits in this file are saved first, Undo brings them back").clicked() {
                                    acts.push(Action::RecreateConflict(path.clone()));
                                }
                            });
                        }
                    }
                }
                let editing = cv.editing.as_ref().filter(|(i, _)| *i == k).is_some();
                if editing {
                    if let Some((_, text)) = cv.editing.as_mut() {
                        ui.add(egui::TextEdit::multiline(text).code_editor().desired_rows(6).desired_width(f32::INFINITY));
                    }
                    ui.horizontal(|ui| {
                        if ui.add(accent_button("Use this text", pal)).clicked() {
                            let text = cv.editing.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
                            acts.push(Action::ResolveBlock { index: k, choice: Choice::Text(text) });
                        }
                        if ui.button("Cancel").clicked() {
                            cv.editing = None;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui.button(format!("Keep {ours}")).clicked() {
                            acts.push(Action::ResolveBlock { index: k, choice: Choice::Ours });
                        }
                        if ui.button(format!("Keep {theirs}")).clicked() {
                            acts.push(Action::ResolveBlock { index: k, choice: Choice::Theirs });
                        }
                        if ui.button("Keep both").on_hover_text(format!("{ours}'s lines, then {theirs}'s")).clicked() {
                            acts.push(Action::ResolveBlock { index: k, choice: Choice::Both });
                        }
                        if ui.button("Edit…").clicked() {
                            cv.editing = Some((k, format!("{}{}", b.ours, b.theirs).replace("\r\n", "\n")));
                        }
                    });
                }
            });
            ui.add_space(8.0);
        }
    });
}

fn code_box(ui: &mut egui::Ui, text: &str, pal: &Palette, id: (&str, usize, usize)) {
    egui::Frame::new().fill(pal.canvas).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::both().id_salt(id).max_height(240.0).show(ui, |ui| {
            let shown = if text.is_empty() { "(nothing)".to_string() } else { text.trim_end_matches(['\n', '\r']).replace("\r\n", "\n") };
            ui.add(egui::Label::new(RichText::new(shown).monospace().size(12.5).color(pal.text)).extend());
        });
    });
}

// ---------------------------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------------------------

/// Draws one of this module's dialogs. Returns true when it should close.
pub fn dialog(ui: &mut egui::Ui, dialog: &mut Dialog, tab: Option<&RepoTab>, pal: &Palette, acts: &mut Vec<Action>) -> bool {
    match dialog {
        Dialog::CommitMessage { sha, change, message } => {
            title(ui, if *change == Change::Reword { "Edit message" } else { "Message of the combined commit" });
            ui.label(RichText::new(format!("Commit {}. The first line is the summary.", git::short(sha))).color(pal.muted));
            ui.add(egui::TextEdit::multiline(message).desired_rows(8).desired_width(f32::INFINITY)).request_focus();
            let valid = !message.trim().is_empty();
            if crate::ui::footer(ui, pal, "Save", valid) {
                acts.push(Action::RewriteCommit { sha: sha.clone(), change: *change, message: message.clone() });
                return true;
            }
            false
        }
        Dialog::Fixup { candidates, choice } => {
            title(ui, "Add to an earlier commit");
            ui.label(RichText::new("The staged changes go into the commit you pick, as if they had always been there. Only commits not on the server yet are listed.").color(pal.muted));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for (i, (sha, summary)) in candidates.iter().enumerate() {
                    let text = RichText::new(format!("{}   {summary}", git::short(sha)));
                    if ui.add(egui::Button::selectable(*choice == i, text).min_size(egui::vec2(ui.available_width(), 24.0))).clicked() {
                        *choice = i;
                    }
                }
            });
            if crate::ui::footer(ui, pal, "Add to this commit", !candidates.is_empty()) {
                if let Some((sha, _)) = candidates.get(*choice) {
                    acts.push(Action::FixupInto(sha.clone()));
                }
                return true;
            }
            false
        }
        Dialog::Worktree(form) => {
            title(ui, &format!("Open {} in a separate folder", form.branch));
            ui.label(RichText::new("The branch gets its own folder and tab: you can work on it while this folder stays as it is.").color(pal.muted));
            ui.add_space(4.0);
            ui.label(RichText::new("Folder").color(pal.text2()));
            ui.add(egui::TextEdit::singleline(&mut form.path).desired_width(f32::INFINITY));
            ui.checkbox(&mut form.open_tab, "Open it in a new tab");
            if form.unity {
                ui.label(RichText::new("Unity project: the first time Unity opens the new folder, it re-imports everything into Library/ (this can take a while).").size(12.0).color(pal.warn));
            }
            let valid = !form.path.trim().is_empty() && !std::path::Path::new(form.path.trim()).exists();
            if !form.path.trim().is_empty() && !valid {
                ui.label(RichText::new("This folder already exists: pick another one.").size(12.0).color(pal.warn));
            }
            if crate::ui::footer(ui, pal, "Open", valid) {
                acts.push(Action::CreateWorktree {
                    path: form.path.trim().to_string(),
                    branch: form.branch.clone(),
                    remote: form.remote,
                    new_from: form.new_from.clone(),
                    open_tab: form.open_tab,
                });
                return true;
            }
            false
        }
        Dialog::RemoveWorktree { path, files } => {
            title(ui, "Remove this folder?");
            ui.label(format!("{} has {} file{} with uncommitted changes:", path.display(), files.len(), if files.len() == 1 { "" } else { "s" }));
            egui::Frame::new().fill(pal.canvas).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for f in files.iter().take(10) {
                    ui.label(RichText::new(f).monospace().size(12.0));
                }
                if files.len() > 10 {
                    ui.label(RichText::new(format!("…and {} more", files.len() - 10)).color(pal.muted));
                }
            });
            let mut close = false;
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(danger("Delete anyway", pal)).on_hover_text("git worktree remove --force: the changes are lost").clicked() {
                    acts.push(Action::RemoveWorktreeNow { path: path.clone(), stash: false, force: true });
                    close = true;
                }
                if ui.add(accent_button("Stash them first", pal)).on_hover_text("They go to the Stashes list (shared by every folder), then the folder is removed").clicked() {
                    acts.push(Action::RemoveWorktreeNow { path: path.clone(), stash: true, force: false });
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
            close
        }
        Dialog::LfsPreset { patterns, lockable } => {
            title(ui, "Set up LFS for Unity");
            ui.label(RichText::new("Big files are stored with LFS; scenes and prefabs merge badly, so they can be locked while someone edits them. Change the lists before applying, one pattern per line.").color(pal.muted));
            ui.label(RichText::new("Stored with LFS").color(pal.text2()));
            ui.add(egui::TextEdit::multiline(patterns).code_editor().desired_rows(5).desired_width(f32::INFINITY));
            ui.label(RichText::new("Stored with LFS and lockable").color(pal.text2()));
            ui.add(egui::TextEdit::multiline(lockable).code_editor().desired_rows(2).desired_width(f32::INFINITY));
            let lines = |s: &str| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect::<Vec<_>>();
            let (p, l) = (lines(patterns), lines(lockable));
            if tab.is_some_and(|t| !t.snap.lfs.installed) {
                ui.label(RichText::new("Git LFS is not installed: install it first (git-lfs.com).").color(pal.warn));
            }
            if crate::ui::footer(ui, pal, "Apply", !p.is_empty() || !l.is_empty()) {
                acts.push(Action::LfsTrack { patterns: p, lockable: l });
                return true;
            }
            false
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Sidebar sections and badges
// ---------------------------------------------------------------------------------------------

/// The "Worktrees" section: every folder of this repository.
pub fn worktrees_body(ui: &mut egui::Ui, tab: &RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let here = tab.path.as_ref().map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase());
    if tab.snap.worktrees.len() <= 1 {
        ui.label(RichText::new("Right-click a branch → Open in a separate folder…").color(pal.muted).size(12.0));
    }
    for w in &tab.snap.worktrees {
        let is_here = Some(w.path.to_string_lossy().replace('\\', "/").to_lowercase()) == here;
        let name = w.branch.clone().unwrap_or_else(|| format!("detached at {}", git::short(&w.head)));
        let folder = w.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut text = format!("   {name}  ·  {folder}");
        if w.changes > 0 {
            text.push_str("  ●");
        }
        let color = if is_here { pal.accent } else { pal.text2() };
        let tip = format!(
            "{}\n{}{}",
            w.path.display(),
            if w.changes > 0 { format!("{} uncommitted change(s)\n", w.changes) } else { String::new() },
            if is_here { "This tab" } else { "Click to open it in its tab · right-click to remove" }
        );
        let resp = ui.add(egui::Button::selectable(false, RichText::new(text).color(color)).frame_when_inactive(false).truncate()).on_hover_text(tip);
        if resp.clicked() && !is_here {
            acts.push(Action::OpenWorktree(w.path.clone()));
        }
        if !w.main && !is_here {
            let path = w.path.clone();
            resp.context_menu(|ui| {
                crate::util::menu_pad(ui);
                if ui.button("Open its tab").clicked() {
                    acts.push(Action::OpenWorktree(path.clone()));
                    ui.close();
                }
                if ui.button(RichText::new("Remove folder…").color(pal.removed)).on_hover_text("git worktree remove: the branch stays").clicked() {
                    acts.push(Action::RemoveWorktree(path.clone()));
                    ui.close();
                }
            });
        }
    }
}

/// The "Big files (LFS)" section: locks (mine and others') and the Unity preset.
pub fn lfs_body(ui: &mut egui::Ui, tab: &RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let lfs = &tab.snap.lfs;
    if lfs.unity {
        let preset = egui::Button::new(RichText::new("Set up LFS for Unity…").size(12.0));
        if ui.add(preset).on_hover_text("Big asset files with LFS, scenes and prefabs lockable").clicked() {
            acts.push(Action::OpenLfsPreset);
        }
    }
    if !lfs.used {
        ui.label(RichText::new("This project doesn't use LFS yet.").color(pal.muted).size(12.0));
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Locks").strong().size(12.5));
        if ui.small_button("Refresh").on_hover_text("git lfs locks (also after each fetch)").clicked() {
            acts.push(Action::RefreshLocks);
        }
    });
    match &tab.lfs_locks {
        None => {
            ui.label(RichText::new("Not read yet: Fetch or Refresh.").color(pal.muted).size(12.0));
        }
        Some(Err(e)) => {
            ui.label(RichText::new(format!("Could not read the locks: {e}")).color(pal.muted).size(12.0));
        }
        Some(Ok(locks)) => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            for (head, mine) in [("Mine", true), ("Others", false)] {
                let list: Vec<&lfs::Lock> = locks.iter().filter(|l| l.mine == mine).collect();
                ui.label(RichText::new(format!("{head}  ({})", list.len())).color(pal.text2()).size(12.0));
                for l in list {
                    let who = if mine { String::new() } else { format!("{} · ", l.owner) };
                    let text = RichText::new(format!("   {}  {who}{}", l.path, lfs::since(&l.locked_at, now))).color(pal.text2()).size(12.0);
                    let resp = ui.add(egui::Button::selectable(false, text).frame_when_inactive(false).truncate()).on_hover_text(&l.path);
                    let path = l.path.clone();
                    resp.context_menu(|ui| {
                        crate::util::menu_pad(ui);
                        if mine {
                            if ui.button("Unlock").on_hover_text("git lfs unlock").clicked() {
                                acts.push(Action::LfsUnlock { path: path.clone(), force: false });
                                ui.close();
                            }
                        } else {
                            ui.menu_button("More options", |ui| {
                                crate::util::menu_pad(ui);
                                if ui.button("Force unlock").on_hover_text("git lfs unlock --force: needs admin rights on the server").clicked() {
                                    acts.push(Action::LfsUnlock { path: path.clone(), force: true });
                                    ui.close();
                                }
                            });
                        }
                    });
                }
            }
        }
    }
}

/// Banner when the project uses LFS but LFS is missing (doc §LFS 1).
pub fn lfs_banner(ui: &mut egui::Ui, tab: &RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    if !tab.snap.lfs.used || tab.snap.lfs.installed {
        return;
    }
    egui::Frame::new().fill(pal.warn.gamma_multiply(0.18)).stroke(Stroke::new(1.0, pal.warn)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("This project stores big files with Git LFS, which is not installed. You will only see small pointer files.");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(accent_button("Install Git LFS", pal)).clicked() {
                    acts.push(Action::OpenUrl("https://git-lfs.com".into()));
                }
            });
        });
    });
}

/// LFS and lock badges at the right of a file row (hidden while hovered, where the
/// Stage button goes).
pub fn file_badges(ui: &egui::Ui, row: egui::Rect, f: &FileChange, tab: &RepoTab, locks: &HashMap<String, lfs::Lock>, pal: &Palette) {
    if ui.rect_contains_pointer(row) {
        return;
    }
    let mut x = row.right() - 12.0;
    let p = ui.painter();
    let mut badge = |text: String, color: Color32| {
        let g = p.layout_no_wrap(text, egui::FontId::proportional(10.5), color);
        let r = egui::Rect::from_min_size(egui::pos2(x - g.size().x - 10.0, row.center().y - 8.0), egui::vec2(g.size().x + 10.0, 16.0));
        p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, color.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        p.galley(egui::pos2(r.left() + 5.0, r.center().y - g.size().y / 2.0), g, color);
        x = r.left() - 4.0;
    };
    if let Some(l) = locks.get(&f.path) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        if l.mine {
            badge("Locked by you".into(), pal.accent);
        } else {
            badge(format!("Locked by {} {}", l.owner, lfs::since(&l.locked_at, now)), pal.removed);
        }
    }
    if tab.snap.lfs.tracked.contains(&f.path) {
        let size = tab.path.as_ref().and_then(|r| std::fs::metadata(r.join(&f.path)).ok()).map(|m| lfs::human_size(m.len()));
        badge(size.map_or("LFS".into(), |s| format!("LFS {s}")), pal.muted);
    }
}

/// The locks by path, if they were read.
pub fn locks_map(tab: &RepoTab) -> HashMap<String, lfs::Lock> {
    match &tab.lfs_locks {
        Some(Ok(l)) => lfs::by_path(l),
        _ => HashMap::new(),
    }
}

/// Branches open in another folder: (branch, folder).
pub fn branch_folders(tab: &RepoTab) -> HashMap<String, std::path::PathBuf> {
    crate::app::other_folders(tab).into_iter().collect()
}

/// "Open in a separate folder…" (or "Go to its folder") for a branch menu.
pub fn worktree_item(ui: &mut egui::Ui, tab: &RepoTab, name: &str, kind: RefKind, acts: &mut Vec<Action>) {
    let folders = branch_folders(tab);
    if let Some(p) = folders.get(name) {
        if ui.button("Go to its folder").on_hover_text(format!("{name} is open in {}", p.display())).clicked() {
            acts.push(Action::OpenWorktree(p.clone()));
            ui.close();
        }
        return;
    }
    if tab.snap.head_branch.as_deref() == Some(name) {
        return;
    }
    if ui.button("Open in a separate folder…").on_hover_text("git worktree add: this branch in its own folder and tab").clicked() {
        acts.push(Action::OpenWorktreeForm { branch: name.to_string(), remote: kind == RefKind::Remote, new_from: None });
        ui.close();
    }
}
