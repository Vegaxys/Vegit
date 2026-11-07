//! Foundations (design doc, phase 2): the universal Undo / Redo built on `safety.rs`, branch
//! drag and drop, and picking several commits in the graph.
//!
//! A child module of `app`, so it uses the worker plumbing. Every recorded action lands on
//! the tab's Undo stack with the commands that take it back; Undo runs those (recorded in
//! turn, which gives Redo). Server state is never touched: once a moved branch has been
//! pushed, its Undo is refused with a sentence rather than rewriting the server.

use std::path::PathBuf;

use eframe::egui;

use super::{Action, DropMenu, OpKind, Role, Task, ToastKind, VegitApp};
use crate::git::MergeMode;
use crate::safety::{ResetMode, Step, UndoEntry, git_step};

/// Most entries kept per stack.
const MAX_UNDO: usize = 50;

impl VegitApp {
    /// Puts a recorded action on the right stack.
    pub(super) fn record(&mut self, tab_path: &PathBuf, mut entry: UndoEntry, role: Role) {
        if !matches!(role, Role::Undone(_)) {
            self.next_undo_id += 1;
            entry.id = self.next_undo_id;
        }
        let Some(t) = self.tab_by_path(tab_path) else { return };
        match role {
            Role::New => {
                t.undo.push(entry);
                t.redo.clear();
            }
            Role::Undone(orig) => {
                // Keep the original's name and mode; Redo runs what the undo recorded.
                if let Some(o) = orig {
                    entry.what = o.what;
                    entry.mode = o.mode;
                    entry.forward = o.forward;
                }
                t.redo.push(entry);
            }
            Role::Redone => t.undo.push(entry),
        }
        if t.undo.len() > MAX_UNDO {
            t.undo.remove(0);
        }
    }

    /// After a push: actions that moved the pushed branch can't be undone without a force push.
    pub(super) fn mark_pushed(&mut self, tab_path: &PathBuf) {
        let Some(t) = self.tab_by_path(tab_path) else { return };
        let Some(branch) = t.snap.head_branch.clone() else { return };
        for e in t.undo.iter_mut().filter(|e| e.branches.contains(&branch)) {
            e.pushed = true;
        }
    }

    /// Undoes the last action (`id`: only if it is still that one, for a toast's Undo).
    pub(super) fn undo(&mut self, ctx: &egui::Context, id: Option<u64>) {
        let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
        let Some(tab) = self.tab() else { return };
        match tab.undo.last() {
            Some(e) if id.is_some_and(|i| i != e.id) => {
                self.toast(ctx, "Other actions came after this one: the toolbar's Undo takes them back one by one.", ToastKind::Info);
            }
            Some(e) if e.pushed => {
                self.toast(ctx, "Already on the server: undoing would need a force push.", ToastKind::Info);
            }
            Some(_) => {
                let Some(e) = tab.undo.pop() else { return };
                let ok = Some(format!("Undid {}", e.what));
                let (steps, what) = (e.steps.clone(), Some((e.what.clone(), e.mode)));
                self.send(Task::Steps { repo: repo.clone(), label: "Undo".into(), ok_text: ok, steps, what, kind: OpKind::Plain, role: Role::Undone(Some(Box::new(e))) });
                self.send(Task::Reload { repo });
            }
            // Nothing recorded yet (a fresh start): the last commit can still be undone.
            None => {
                let Some(r) = tab.snap.current_ref() else { return };
                if r.upstream.is_some() && r.ahead == 0 {
                    self.toast(ctx, "The last commit is already on the server: undoing would need a force push.", ToastKind::Info);
                    return;
                }
                let what = Some(("commit".to_string(), ResetMode::Soft));
                let steps = vec![git_step(&["reset", "--soft", "HEAD~1"])];
                let ok = Some("Undid the last commit: its changes are staged again".to_string());
                self.send(Task::Steps { repo: repo.clone(), label: "Undo".into(), ok_text: ok, steps, what, kind: OpKind::Plain, role: Role::Undone(None) });
                self.send(Task::Reload { repo });
            }
        }
    }

    pub(super) fn redo(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
        let Some(e) = self.tab().and_then(|t| t.redo.pop()) else { return };
        // Undoing moved refs: redo moves them again. Undoing only restored files: run it again.
        let steps = if e.steps.is_empty() { e.forward.clone() } else { e.steps.clone() };
        let ok = Some(format!("Redid {}", e.what));
        self.send(Task::Steps { repo: repo.clone(), label: "Redo".into(), ok_text: ok, steps, what: Some((e.what, e.mode)), kind: OpKind::Plain, role: Role::Redone });
        self.send(Task::Reload { repo });
    }

    pub(super) fn handle_safety(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::OpenDropMenu { source, target } => {
                let pos = ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default();
                self.drop_menu = Some(DropMenu { source, target, pos });
            }
            Action::CloseDropMenu => self.drop_menu = None,
            Action::MergeInto { source, target, mode } => {
                let current = self.tab().and_then(|t| t.snap.head_branch.clone());
                if current.as_deref() == Some(target.as_str()) {
                    return self.handle_quick(ctx, Action::Merge { source, mode });
                }
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                // Switch to the target, then merge: one action, one Undo (it switches back).
                let mut steps = vec![git_step(&["switch", &target])];
                steps.push(Step::Git(crate::git::merge_args(mode, &source)));
                let (ok, kind) = match mode {
                    MergeMode::Squash => (None, OpKind::Squash { source: source.clone(), target: target.clone() }),
                    _ => (Some(format!("Brought {source} into {target}")), OpKind::Plain),
                };
                let what = (mode != MergeMode::Squash).then(|| ("merge".to_string(), ResetMode::Keep));
                self.send(Task::Steps { repo: repo.clone(), label: "Merge".into(), ok_text: ok, steps, what, kind, role: Role::New });
                self.send(Task::Reload { repo });
            }
            Action::PickCommit { sha, toggle, range } => {
                let Some(t) = self.tab() else { return };
                // Start from the commit already selected, so Ctrl+click adds a second one.
                if t.picked_commits.is_empty() {
                    if let super::Selection::Commit(s) = &t.selection {
                        if *s != sha {
                            t.picked_commits.push(s.clone());
                        }
                    }
                }
                if range {
                    let anchor = t.picked_commits.first().cloned().or_else(|| match &t.selection {
                        super::Selection::Commit(s) => Some(s.clone()),
                        _ => None,
                    });
                    let (Some(a), Some(b)) = (anchor.and_then(|a| t.commit_index(&a)), t.commit_index(&sha)) else { return };
                    let (lo, hi) = (a.min(b), a.max(b));
                    let first = t.snap.commits[a].sha.clone();
                    t.picked_commits = std::iter::once(first.clone())
                        .chain(t.snap.commits[lo..=hi].iter().map(|c| c.sha.clone()).filter(|s| *s != first))
                        .collect();
                } else if toggle {
                    if let Some(i) = t.picked_commits.iter().position(|s| *s == sha) {
                        t.picked_commits.remove(i);
                    } else {
                        t.picked_commits.push(sha);
                    }
                }
                if t.picked_commits.len() < 2 {
                    // One left: it becomes the plain selection again.
                    let single = t.picked_commits.pop();
                    if let Some(s) = single {
                        self.handle_one(ctx, Action::Select(super::Selection::Commit(s)));
                    }
                }
            }
            Action::ClearPicked => {
                if let Some(t) = self.tab() {
                    t.picked_commits.clear();
                }
            }
            other => self.handle_big(ctx, other),
        }
    }
}
