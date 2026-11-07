//! The "big pieces" (design doc, phase 3) behind the buttons: editing history, adding to an
//! earlier commit, resolving conflicts block by block, worktrees and Git LFS.
//!
//! A child module of `app`. Every local change goes through the safety net (`Task::Steps` or
//! the recorded rewrite task), so each has its Undo; server actions (LFS locks) run on their
//! own threads. The git commands are built and tested in `rewrite.rs`, `conflict.rs`,
//! `worktree.rs` and `lfs.rs`.

use std::path::{Path, PathBuf};

use eframe::egui;

use super::{Action, Dialog, JobDone, OpKind, Role, Task, ToastKind, VegitApp};
use crate::conflict::{self, Segment};
use crate::git::{self, FileState};
use crate::lfs;
use crate::rewrite::{self, Change, HistoryCommit, PlanItem};
use crate::safety::{ResetMode, Step, git_step};
use crate::worktree;

/// "Edit history from here": the plan being prepared in the detail panel.
#[derive(Clone, Debug)]
pub struct HistoryEdit {
    pub from: String,
    pub history: Vec<HistoryCommit>,
    /// Oldest first, as git applies it; the panel shows it newest first.
    pub plan: Vec<PlanItem>,
    /// Commits of the range already on the server (doc guard rail).
    pub pushed: usize,
    pub upstream: Option<String>,
}

/// A conflicted file shown as cards.
#[derive(Clone, Debug)]
pub struct ConflictView {
    pub path: String,
    /// `None` while the file is being read.
    pub segs: Option<Vec<Segment>>,
    /// Conflicts the file had when first opened, for "2 of 5 left".
    pub total: usize,
    /// Branch names for (what git calls ours, theirs).
    pub names: (String, String),
    /// Block being edited by hand, and its text.
    pub editing: Option<(usize, String)>,
    pub show_base: bool,
}

/// The "Open in a separate folder" window.
#[derive(Clone, Debug)]
pub struct WorktreeForm {
    pub branch: String,
    pub remote: bool,
    /// Create `branch` from this commit or branch.
    pub new_from: Option<String>,
    pub path: String,
    pub open_tab: bool,
    pub unity: bool,
}

/// What a recorded history edit is, before its plan is known (built on the worker).
#[derive(Clone, Debug)]
pub enum RewriteKind {
    Single { sha: String, change: Change, message: String },
    Move { sha: String, up: bool },
    Plan(Vec<PlanItem>, Vec<HistoryCommit>),
}

/// Builds the steps of a history edit (worker thread): reads the range, checks the plan.
pub(super) fn rewrite_steps(repo: &Path, kind: RewriteKind) -> Result<Vec<Step>, String> {
    let (history, plan) = match kind {
        RewriteKind::Single { sha, change, message } => rewrite::single_plan(repo, &sha, change, &message).map_err(|e| e.0)?,
        RewriteKind::Move { sha, up } => rewrite::move_plan(repo, &sha, up).map_err(|e| e.0)?,
        RewriteKind::Plan(plan, history) => (history, plan),
    };
    rewrite::check(&plan, &history)?;
    rewrite::plan_steps(repo, &plan, &rewrite::vegit_editor).map_err(|e| format!("Could not prepare the history edit: {e}"))
}

/// Reads a conflicted file and names its sides (worker thread).
pub(super) fn read_conflict(repo: &Path, path: &str, pending: Option<git::PendingOp>) -> Result<(Vec<Segment>, (String, String)), String> {
    let text = std::fs::read(repo.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let segs = conflict::parse(&String::from_utf8_lossy(&text));
    let first = conflict::blocks(&segs).first().map(|b| (b.ours_label.clone(), b.theirs_label.clone())).unwrap_or_default();
    Ok((segs, conflict::side_names(repo, pending, &first.0, &first.1)))
}

impl VegitApp {
    pub(super) fn handle_big(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::RewriteCommit { sha, change, message } => {
                let what = match change {
                    Change::Reword => "edit message",
                    Change::Drop => "remove commit",
                    Change::Edit => "stop to change files",
                    _ => "combine commits",
                };
                let ok = match change {
                    Change::Reword => "Message changed".to_string(),
                    Change::Drop => format!("Removed {}", git::short(&sha)),
                    Change::Edit => "Stopped on the commit: change files, then Continue".to_string(),
                    _ => "Commits combined".to_string(),
                };
                self.send_rewrite(ctx, RewriteKind::Single { sha, change, message }, what, ok);
            }
            Action::MoveCommit { sha, up } => {
                let ok = if up { "Moved the commit up" } else { "Moved the commit down" };
                self.send_rewrite(ctx, RewriteKind::Move { sha, up }, "move commit", ok.to_string());
            }
            Action::AskCommitMessage { sha, change } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let message = if change == Change::SquashNew {
                    // A new message for two commits: start from both.
                    let parent = rewrite::parent_of(&repo, &sha).unwrap_or_default();
                    format!("{}\n\n{}", git::full_message(&repo, &parent).trim(), git::full_message(&repo, &sha).trim())
                } else {
                    git::full_message(&repo, &sha).trim().to_string()
                };
                self.dialog = Some(Dialog::CommitMessage { sha, change, message });
            }
            Action::OpenHistoryEdit(from) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.send(Task::History { repo, from });
            }
            Action::HistoryApply => {
                let Some(t) = self.tab() else { return };
                let Some(h) = t.history_edit.take() else { return };
                let n = rewrite::change_count(&h.plan, &h.history);
                self.send_rewrite(ctx, RewriteKind::Plan(h.plan, h.history), "edit history", format!("History edited ({n} change{})", if n == 1 { "" } else { "s" }));
            }
            Action::HistoryCancel => {
                if let Some(t) = self.tab() {
                    t.history_edit = None;
                }
            }
            Action::OpenFixup => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let candidates = rewrite::fixup_candidates(&repo);
                if candidates.is_empty() {
                    self.toast(ctx, "No earlier commit to add to: every commit here is already on the server.", ToastKind::Info);
                    return;
                }
                self.dialog = Some(Dialog::Fixup { candidates, choice: 0 });
            }
            Action::FixupInto(target) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let steps = rewrite::fixup_steps(&repo, &target);
                let ok = Some(format!("Added to {}", git::short(&target)));
                self.send(Task::Steps { repo: repo.clone(), label: "Add to an earlier commit".into(), ok_text: ok, steps, what: Some(("add to an earlier commit".into(), ResetMode::Keep)), kind: OpKind::Plain, role: Role::New });
                self.send(Task::Reload { repo });
            }
            Action::OpenConflict(path) => self.open_conflict(path),
            Action::CloseConflict => {
                if let Some(t) = self.tab() {
                    t.conflict = None;
                    t.diff = None;
                }
            }
            Action::ResolveBlock { index, choice } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let Some(t) = self.tab() else { return };
                let Some(cv) = t.conflict.as_mut() else { return };
                let Some(segs) = &cv.segs else { return };
                let text = conflict::resolve(segs, index, &choice);
                let left = conflict::count(&conflict::parse(&text));
                let path = cv.path.clone();
                cv.editing = None;
                let mut steps = vec![Step::Backup(vec![path.clone()]), Step::Write(path.clone(), text)];
                // The last block: the file is resolved, mark it so (git add).
                if left == 0 {
                    steps.push(git_step(&["add", "--", &path]));
                }
                let ok = Some(if left == 0 { format!("{path} resolved") } else { format!("{left} conflict{} left in {path}", if left == 1 { "" } else { "s" }) });
                self.send(Task::Steps { repo: repo.clone(), label: "Resolve".into(), ok_text: ok, steps, what: Some(("conflict choice".into(), ResetMode::Keep)), kind: OpKind::Plain, role: Role::New });
                if left == 0 {
                    if let Some(t) = self.tab() {
                        t.conflict = None;
                        t.diff = None;
                    }
                } else {
                    self.open_conflict(path);
                }
                self.send(Task::Reload { repo });
            }
            Action::RecreateConflict(path) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let steps = vec![Step::Backup(vec![path.clone()]), git_step(&["checkout", "--conflict=zdiff3", "--", &path])];
                self.send(Task::Steps { repo, label: "Show the original".into(), ok_text: None, steps, what: Some(("show the original".into(), ResetMode::Keep)), kind: OpKind::Plain, role: Role::New });
                self.open_conflict(path);
            }
            Action::OpenMergeTool(path) => {
                let Some(repo) = self.active_path() else { return };
                self.spawn_steps(ctx, repo, "Merge tool".into(), vec![vec!["mergetool".into(), "--no-prompt".into(), "--".into(), path]], None, OpKind::Plain);
            }
            Action::OpenWorktreeForm { branch, remote, new_from } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let path = worktree::suggest_path(&repo, &branch).display().to_string();
                let unity = repo.join("ProjectSettings").is_dir();
                self.dialog = Some(Dialog::Worktree(WorktreeForm { branch, remote, new_from, path, open_tab: true, unity }));
            }
            Action::CreateWorktree { path, branch, remote, new_from, open_tab } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let args = worktree::add_args(Path::new(&path), &branch, remote, new_from.as_deref());
                self.busy.push("Worktree".into());
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                // Checking out a big project can take a while: off the worker.
                std::thread::spawn(move || {
                    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    let result = git::run(&repo, &refs).map(|_| ()).map_err(|e| e.0);
                    let _ = tx.send(JobDone::WorktreeAdded { repo, path: PathBuf::from(path), result, open_tab });
                    ctx2.request_repaint();
                });
            }
            Action::OpenWorktree(path) => self.open_worktree(ctx, &path),
            Action::RemoveWorktree(path) => {
                let files = worktree::changed_files(&path);
                if files.is_empty() {
                    self.handle_big(ctx, Action::RemoveWorktreeNow { path, stash: false, force: false });
                } else {
                    self.dialog = Some(Dialog::RemoveWorktree { path, files });
                }
            }
            Action::RemoveWorktreeNow { path, stash, force } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let mut steps = Vec::new();
                if stash {
                    let p = path.display().to_string();
                    steps.push(git_step(&["-C", &p, "stash", "push", "-u", "-m", "Saved by Vegit before removing this folder"]));
                }
                steps.push(Step::Git(worktree::remove_args(&path, force)));
                let ok = Some(format!("Removed the folder {}", path.display()));
                self.send(Task::Steps { repo: repo.clone(), label: "Remove folder".into(), ok_text: ok, steps, what: None, kind: OpKind::Plain, role: Role::New });
                // Its tab, if open, has nothing left to show.
                if let Some(i) = self.tabs.iter().position(|t| t.path.as_deref() == Some(path.as_path())) {
                    self.handle_one(ctx, Action::CloseTab(i));
                }
                self.send(Task::Reload { repo });
            }
            Action::OpenLfsPreset => {
                let patterns = lfs::UNITY_PATTERNS.join("\n");
                let lockable = lfs::UNITY_LOCKABLE.join("\n");
                self.dialog = Some(Dialog::LfsPreset { patterns, lockable });
            }
            Action::LfsTrack { patterns, lockable } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let mut steps: Vec<Step> = patterns.iter().map(|p| Step::Git(lfs::track_args(p, false))).collect();
                steps.extend(lockable.iter().map(|p| Step::Git(lfs::track_args(p, true))));
                steps.push(git_step(&["add", "--", ".gitattributes"]));
                let n = patterns.len() + lockable.len();
                let ok = Some(format!("LFS now stores {n} kind{} of files (.gitattributes is staged)", if n == 1 { "" } else { "s" }));
                self.send(Task::Steps { repo: repo.clone(), label: "LFS".into(), ok_text: ok, steps, what: None, kind: OpKind::Plain, role: Role::New });
                self.send(Task::Reload { repo });
            }
            Action::LfsLock(path) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.spawn_steps(ctx, repo, "Lock".into(), vec![lfs::lock_args(&path)], Some(format!("{path} is locked: others see it's yours")), OpKind::Locks);
            }
            Action::LfsUnlock { path, force } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.spawn_steps(ctx, repo, "Unlock".into(), vec![lfs::unlock_args(&path, force)], Some(format!("{path} is unlocked")), OpKind::Locks);
            }
            Action::RefreshLocks => {
                if let Some(repo) = self.active_path() {
                    self.spawn_locks(ctx, repo);
                }
            }
            _ => {}
        }
    }

    fn send_rewrite(&mut self, ctx: &egui::Context, kind: RewriteKind, what: &str, ok: String) {
        let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
        self.send(Task::Rewrite { repo: repo.clone(), kind, what: what.to_string(), ok_text: ok });
        self.send(Task::Reload { repo });
    }

    pub(super) fn open_conflict(&mut self, path: String) {
        let Some(repo) = self.active_path() else { return };
        let Some(t) = self.tab() else { return };
        let pending = t.snap.pending;
        let (total, names) = match &t.conflict {
            Some(cv) if cv.path == path => (cv.total, cv.names.clone()),
            _ => (0, Default::default()),
        };
        let keep_base = t.conflict.as_ref().is_some_and(|c| c.show_base);
        t.conflict = Some(ConflictView { path: path.clone(), segs: None, total, names, editing: None, show_base: keep_base });
        self.send(Task::Conflict { repo, path, pending });
    }

    /// Opens a worktree's folder as a tab right after its repository's tab.
    fn open_worktree(&mut self, ctx: &egui::Context, path: &Path) {
        let main = self.tab().and_then(|t| t.snap.worktrees.iter().find(|w| w.main).map(|w| w.path.clone()));
        if self.tabs.iter().any(|t| t.path.as_deref() == Some(path)) {
            if let Err(e) = self.open_repo(path, true) {
                self.toast(ctx, e, ToastKind::Error);
            }
            return;
        }
        if let Err(e) = self.open_repo(path, true) {
            self.toast(ctx, e, ToastKind::Error);
            return;
        }
        let new = self.tabs.len() - 1;
        let same = |a: &Path, b: &Path| a.to_string_lossy().replace('\\', "/").eq_ignore_ascii_case(&b.to_string_lossy().replace('\\', "/"));
        if let Some(main) = main {
            if let Some(i) = self.tabs.iter().position(|t| t.path.as_deref().is_some_and(|p| same(p, &main))) {
                let tab = self.tabs.remove(new);
                let at = (i + 1).min(self.tabs.len());
                self.tabs.insert(at, tab);
                self.active = at;
                self.sync_open_tabs();
            }
        }
    }

    pub(super) fn on_worktree_added(&mut self, ctx: &egui::Context, repo: PathBuf, path: PathBuf, result: Result<(), String>, open_tab: bool) {
        self.busy.retain(|b| b != "Worktree");
        match result {
            Ok(()) => {
                self.toast(ctx, format!("The branch is open in {}", path.display()), ToastKind::Success);
                self.send(Task::Reload { repo });
                if open_tab {
                    self.open_worktree(ctx, &path);
                }
            }
            Err(e) => self.toast(ctx, format!("Could not open it in a separate folder: {e}"), ToastKind::Error),
        }
    }

    /// Reads the LFS locks on a network thread.
    pub(super) fn spawn_locks(&mut self, ctx: &egui::Context, repo: PathBuf) {
        let tx = self.jobs_tx.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let result = lfs::locks(&repo).map_err(|e| e.0);
            let _ = tx.send(JobDone::Locks { tab_path: repo, result });
            ctx2.request_repaint();
        });
    }

    /// After a push: my locked files with no change left were just pushed: offer to unlock.
    pub(super) fn offer_unlock(&mut self, ctx: &egui::Context, tab_path: &PathBuf) {
        let Some(t) = self.tab_by_path(tab_path) else { return };
        let Some(Ok(locks)) = &t.lfs_locks else { return };
        let changed: std::collections::HashSet<&str> = t.snap.status.unstaged.iter().chain(&t.snap.status.staged).map(|f| f.path.as_str()).collect();
        let done: Vec<String> = locks.iter().filter(|l| l.mine && !changed.contains(l.path.as_str())).map(|l| l.path.clone()).collect();
        if let [one] = done.as_slice() {
            self.toast_offer(ctx, format!("{one} is pushed and still locked by you."), ToastKind::Info, "Unlock it", Action::LfsUnlock { path: one.clone(), force: false });
        }
    }

    /// Staging a very big file that LFS doesn't store: suggest storing its kind with LFS.
    pub(super) fn suggest_lfs(&mut self, ctx: &egui::Context, files: &[git::FileChange]) {
        let Some(repo) = self.active_path() else { return };
        let Some(t) = self.tab() else { return };
        let tracked = t.snap.lfs.tracked.clone();
        let big = files.iter().filter(|f| f.state != FileState::Deleted && !tracked.contains(&f.path)).find_map(|f| {
            let size = std::fs::metadata(repo.join(&f.path)).ok()?.len();
            (size > lfs::SUGGEST_BYTES).then(|| (f.path.clone(), size))
        });
        if let Some((path, size)) = big {
            let pattern = lfs::pattern_for(&path);
            let text = format!(
                "{path} weighs {}. GitHub refuses files over {} MB. Store {pattern} files with LFS?",
                lfs::human_size(size),
                lfs::HOST_LIMIT_MB
            );
            self.toast_offer(ctx, text, ToastKind::Info, "Use LFS", Action::LfsTrack { patterns: vec![pattern], lockable: Vec::new() });
        }
    }
}

/// Branches open in another folder, with that folder (for the graph and the double-click).
pub fn other_folders(tab: &super::RepoTab) -> Vec<(String, PathBuf)> {
    let here = tab.path.as_ref().map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase());
    tab.snap
        .worktrees
        .iter()
        // This folder (the main one for the sample, which has no path) is not "another".
        .filter(|w| if here.is_none() { !w.main } else { Some(w.path.to_string_lossy().replace('\\', "/").to_lowercase()) != here })
        .filter_map(|w| w.branch.clone().map(|b| (b, w.path.clone())))
        .collect()
}
