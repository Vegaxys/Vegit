//! The "quick wins" behind the buttons: remotes, tags on the server, upstream branches,
//! deleting a branch that isn't merged (with Undo), merge options and the git identity.
//!
//! A child module of `app`, so it uses the same worker and job plumbing; the git commands
//! themselves are built in `git.rs` (and tested there). Local work goes to the worker,
//! anything touching a server runs on its own thread.

use std::path::PathBuf;

use eframe::egui;

use super::{Action, Dialog, JobDone, OpKind, RepoTab, Selection, Task, ToastKind, VegitApp, dirs_home};
use crate::git::{self, MergeMode, RefKind};

/// Where the access check of the "Add remote" window stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteCheck {
    Idle,
    Checking,
    /// The server answered; the number of branches it has.
    Reachable(usize),
    /// Why it didn't, in plain words.
    Failed(String),
}

/// The "Add remote" window: paste an address, Vegit checks it and proposes a name.
#[derive(Clone, Debug)]
pub struct RemoteForm {
    pub url: String,
    pub name: String,
    /// The person typed a name: stop proposing one.
    pub name_edited: bool,
    /// Remotes already there, as (name, url), to propose a free and meaningful name.
    pub existing: Vec<(String, String)>,
    pub check: RemoteCheck,
    /// The address the check (running or done) is about.
    pub checked_url: String,
    /// When the address last changed: the check waits for the typing to stop.
    pub edited_at: f64,
}

/// The "Push…" window: where to push the current branch, and the tags that go with it.
#[derive(Clone, Debug)]
pub struct PushForm {
    pub branch: String,
    pub remotes: Vec<String>,
    pub remote: usize,
    /// Name of the branch on the server (the same name by default).
    pub remote_branch: String,
    /// Tags on the pushed commits; those missing on the chosen remote are offered.
    pub reachable: Vec<String>,
    pub push_tags: bool,
    pub force: bool,
    /// The branch already tracks a remote branch.
    pub has_upstream: bool,
}

impl PushForm {
    pub fn remote_name(&self) -> &str {
        self.remotes.get(self.remote).map_or("origin", String::as_str)
    }
}

impl RepoTab {
    /// The remote tags and pushes go to by default: origin, else the first one.
    pub fn default_remote(&self) -> Option<String> {
        self.snap.remotes.iter().find(|r| *r == "origin").or(self.snap.remotes.first()).cloned()
    }

    /// The tag is not on the default remote. Only known after a fetch (`ls-remote --tags`).
    pub fn is_local_only_tag(&self, tag: &str) -> bool {
        let Some(remote) = self.default_remote() else { return false };
        self.remote_tags.get(&remote).is_some_and(|on_server| !on_server.contains(tag))
    }

    /// Tags among `tags` that `remote` doesn't have; `None` while that isn't known yet.
    pub fn new_tags_for(&self, remote: &str, tags: &[String]) -> Option<Vec<String>> {
        let on_server = self.remote_tags.get(remote)?;
        Some(tags.iter().filter(|t| !on_server.contains(*t)).cloned().collect())
    }

    /// Remote branches a local branch could track, the one with the same name first.
    pub fn track_options(&self, local: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .snap
            .refs
            .iter()
            .filter(|r| r.kind == RefKind::Remote && !r.name.ends_with("/HEAD"))
            .map(|r| r.name.clone())
            .collect();
        v.sort_by_key(|n| (n.split_once('/').map(|(_, b)| b) != Some(local), n.clone()));
        v
    }
}

impl VegitApp {
    /// Shows a toast with a button (an Undo, "Push it to origin"); it stays a bit longer.
    pub fn toast_offer(&mut self, ctx: &egui::Context, text: impl Into<String>, kind: ToastKind, label: impl Into<String>, action: Action) {
        self.toast(ctx, text, kind);
        if let Some(t) = self.toasts.last_mut() {
            t.action = Some((label.into(), action));
            t.until += 4.0;
        }
    }

    fn no_remote(&mut self, ctx: &egui::Context) {
        self.toast_offer(ctx, "This repository has no remote yet: nowhere to push.", ToastKind::Info, "Add remote…", Action::OpenAddRemote);
    }

    /// Runs several git commands one after the other on a network thread; stops at the first
    /// failure. `label` is what the busy indicator shows.
    pub(super) fn spawn_steps(&mut self, ctx: &egui::Context, dir: PathBuf, label: String, steps: Vec<Vec<String>>, ok_text: Option<String>, kind: OpKind) {
        self.busy.push(label.clone());
        let tx = self.jobs_tx.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let mut result = Ok(String::new());
            for args in &steps {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                result = git::run(&dir, &refs).map_err(|e| e.0);
                if result.is_err() {
                    break;
                }
            }
            let _ = tx.send(JobDone::Op { tab_path: dir, label, ok_text, result, kind, from_worker: false, journal: None });
            ctx2.request_repaint();
        });
    }

    /// Asks the server which tags it has (nothing is downloaded), for the "local" badges.
    pub(super) fn spawn_remote_tags(&mut self, ctx: &egui::Context, dir: PathBuf, remote: String) {
        let tx = self.jobs_tx.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let result = git::remote_tags(&dir, &remote).map_err(|e| e.0);
            let _ = tx.send(JobDone::RemoteTags { tab_path: dir, remote, result });
            ctx2.request_repaint();
        });
    }

    /// After a successful operation: the follow-ups that depend on what it was.
    pub(super) fn after_op(&mut self, ctx: &egui::Context, tab_path: &PathBuf, kind: &OpKind) {
        match kind {
            OpKind::RemoteTags(remote) => self.spawn_remote_tags(ctx, tab_path.clone(), remote.clone()),
            OpKind::Squashed(summary, body) => {
                if let Some(t) = self.tab_by_path(tab_path) {
                    t.summary = summary.clone();
                    t.description = body.clone();
                    t.amend = false;
                    t.selection = Selection::Wip;
                    t.detail = None;
                    t.diff = None;
                }
                self.toast(ctx, "Its changes are staged as one new commit: check the message, then Commit.", ToastKind::Success);
            }
            _ => {}
        }
    }

    pub(super) fn on_remote_checked(&mut self, url: String, result: Result<usize, String>) {
        if let Some(Dialog::AddRemote(form)) = self.dialog.as_mut() {
            if form.checked_url == url {
                form.check = match result {
                    Ok(n) => RemoteCheck::Reachable(n),
                    Err(e) => RemoteCheck::Failed(e),
                };
            }
        }
    }

    pub(super) fn on_branch_checked(&mut self, ctx: &egui::Context, name: String, result: Result<(String, Vec<(String, String)>), String>) {
        match result {
            Ok((sha, commits)) if commits.is_empty() => self.force_delete(ctx, name, sha),
            Ok((sha, commits)) => self.dialog = Some(Dialog::Unmerged { branch: name, sha, commits }),
            Err(e) => self.toast(ctx, format!("Could not read {name}: {e}"), ToastKind::Error),
        }
    }

    /// Deletes a branch without asking: the safety net records its tip, so the toast's Undo
    /// recreates it.
    fn force_delete(&mut self, ctx: &egui::Context, name: String, _sha: String) {
        self.queue_git(ctx, Some(format!("Deleted {name}")), vec!["branch".into(), "-D".into(), name], None, OpKind::Plain);
    }

    fn open_push_dialog(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tab() else { return };
        let Some(branch) = tab.snap.head_branch.clone() else {
            self.toast(ctx, "Switch to a branch before pushing (HEAD is detached).", ToastKind::Info);
            return;
        };
        if tab.snap.remotes.is_empty() {
            self.no_remote(ctx);
            return;
        }
        let remotes = tab.snap.remotes.clone();
        let upstream = tab.snap.current_ref().and_then(|r| r.upstream.clone());
        let tracked = upstream.as_deref().and_then(|u| {
            let i = remotes.iter().position(|r| u.starts_with(&format!("{r}/")))?;
            Some((i, u[remotes[i].len() + 1..].to_string()))
        });
        let (remote, remote_branch) = tracked.unwrap_or_else(|| (remotes.iter().position(|r| r == "origin").unwrap_or(0), branch.clone()));
        let reachable = match &tab.path {
            Some(p) => git::tags_reachable(p, &branch).unwrap_or_default(),
            None => tab.snap.refs.iter().filter(|r| r.kind == RefKind::Tag).map(|r| r.name.clone()).collect(),
        };
        let unknown = !tab.remote_tags.contains_key(&remotes[remote]);
        let first = remotes[remote].clone();
        self.dialog = Some(Dialog::Push(PushForm {
            branch,
            remotes,
            remote,
            remote_branch,
            reachable,
            push_tags: true,
            force: false,
            has_upstream: upstream.is_some(),
        }));
        if unknown {
            self.handle_quick(ctx, Action::CheckRemoteTags(first));
        }
    }

    /// Toolbar Push stays one click; it only asks where to push when the branch has no
    /// remote branch yet and there are several remotes to choose from.
    pub(super) fn push(&mut self, ctx: &egui::Context, force: bool) {
        let Some(tab) = self.tab() else { return };
        let Some(branch) = tab.snap.head_branch.clone() else {
            self.toast(ctx, "Switch to a branch before pushing (HEAD is detached).", ToastKind::Info);
            return;
        };
        let has_upstream = tab.snap.current_ref().is_some_and(|r| r.upstream.is_some());
        let remotes = tab.snap.remotes.clone();
        let mut args = vec!["push".to_string()];
        if force {
            args.push("--force-with-lease".into());
        }
        if !has_upstream {
            match remotes.len() {
                0 => return self.no_remote(ctx),
                1 => args.extend(["-u".into(), remotes[0].clone(), branch]),
                _ => return self.open_push_dialog(ctx),
            }
        }
        self.git_background(ctx, "Push".into(), args);
    }

    pub(super) fn handle_quick(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::OpenAddRemote => {
                let Some(tab) = self.tab() else { return };
                let existing = tab.snap.remote_urls.clone();
                self.dialog = Some(Dialog::AddRemote(RemoteForm {
                    url: String::new(),
                    name: git::suggest_remote_name(&existing, ""),
                    name_edited: false,
                    existing,
                    check: RemoteCheck::Idle,
                    checked_url: String::new(),
                    edited_at: 0.0,
                }));
            }
            Action::CheckRemote(url) => {
                let dir = self.active_path().unwrap_or_else(dirs_home);
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let result = git::remote_check(&dir, &url);
                    let _ = tx.send(JobDone::RemoteChecked { url, result });
                    ctx2.request_repaint();
                });
            }
            Action::AddRemote { name, url } => {
                let Some(dir) = self.active_path() else { return self.sample_refusal(ctx) };
                let steps = vec![git::remote_add_args(&name, &url), vec!["fetch".into(), "--prune".into(), name.clone()]];
                self.spawn_steps(ctx, dir, "Fetch".into(), steps, Some(format!("Remote {name} added and fetched")), OpKind::Plain);
            }
            Action::FetchRemote(name) => self.git_background(ctx, "Fetch".into(), vec!["fetch".into(), "--prune".into(), name]),
            Action::OpenPushDialog => self.open_push_dialog(ctx),
            Action::PushTo { remote, branch, remote_branch, force, tags, set_upstream } => {
                let Some(dir) = self.active_path() else { return self.sample_refusal(ctx) };
                let mut push = vec!["push".to_string()];
                if force {
                    push.push("--force-with-lease".into());
                }
                if set_upstream {
                    push.push("-u".into());
                }
                push.push(remote.clone());
                push.push(if remote_branch == branch { branch.clone() } else { format!("{branch}:{remote_branch}") });
                let mut steps = vec![push];
                if !tags.is_empty() {
                    steps.push(git::push_tags_args(&remote, &tags));
                }
                let ok = match tags.len() {
                    0 => format!("Pushed {branch} to {remote}"),
                    1 => format!("Pushed {branch} and tag {} to {remote}", tags[0]),
                    n => format!("Pushed {branch} and {n} tags to {remote}"),
                };
                self.spawn_steps(ctx, dir, "Push".into(), steps, Some(ok), OpKind::RemoteTags(remote));
            }
            Action::PushTags { remote, tags } => {
                let Some(dir) = self.active_path() else { return self.sample_refusal(ctx) };
                let ok = if tags.len() == 1 { format!("Tag {} pushed to {remote}", tags[0]) } else { format!("{} tags pushed to {remote}", tags.len()) };
                self.spawn_steps(ctx, dir, "Push tags".into(), vec![git::push_tags_args(&remote, &tags)], Some(ok), OpKind::RemoteTags(remote));
            }
            Action::CheckRemoteTags(remote) => {
                if let Some(dir) = self.active_path() {
                    self.spawn_remote_tags(ctx, dir, remote);
                }
            }
            Action::CreateTag { name, args } => {
                let remote = self.tab().and_then(|t| t.default_remote());
                let kind = match remote {
                    Some(r) => OpKind::Offer(format!("Push it to {r}"), Box::new(Action::PushTags { remote: r, tags: vec![name.clone()] })),
                    None => OpKind::Plain,
                };
                self.queue_git(ctx, Some(format!("Tagged {name}")), args, None, kind);
            }
            Action::DeleteTag { tag, remote } => {
                let Some(dir) = self.active_path() else { return self.sample_refusal(ctx) };
                self.queue_git(ctx, Some(format!("Deleted tag {tag}")), vec!["tag".into(), "-d".into(), tag.clone()], None, OpKind::Plain);
                if let Some(r) = remote {
                    let ok = Some(format!("Tag {tag} deleted on {r}"));
                    self.spawn_steps(ctx, dir, "Delete tag on the server".into(), vec![git::delete_remote_tag_args(&r, &tag)], ok, OpKind::RemoteTags(r));
                }
            }
            Action::DeleteBranch(name) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.send(Task::BranchCheck { repo, name });
            }
            Action::ForceDeleteBranch { name, sha } => self.force_delete(ctx, name, sha),
            Action::Merge { source, mode } => {
                let target = self.tab().and_then(|t| t.snap.head_branch.clone()).unwrap_or_else(|| "HEAD".into());
                let args = git::merge_args(mode, &source);
                let (ok, kind) = match mode {
                    MergeMode::Squash => (None, OpKind::Squash { source: source.clone(), target }),
                    _ => (Some(format!("Brought {source} into {target}")), OpKind::Plain),
                };
                self.queue_git(ctx, ok, args, None, kind);
            }
            Action::SetIdentity { global, name, email } => {
                let scope = if global { "all projects" } else { "this project" };
                self.queue_git(ctx, None, git::set_identity_args(global, "user.name", &name), None, OpKind::Plain);
                let ok = Some(format!("Commits are now signed {} <{}> ({scope}).", name.trim(), email.trim()));
                self.queue_git(ctx, ok, git::set_identity_args(global, "user.email", &email), None, OpKind::Plain);
            }
            other => self.handle_safety(ctx, other),
        }
    }
}

/// Git's refusals that deserve a plainer sentence than its own.
pub fn explain_failure(e: &str) -> Option<String> {
    if e.contains("Not possible to fast-forward") || e.contains("Diverging branches can't be fast-forwarded") {
        return Some("Nothing was changed: both branches have new commits, so this needs a merge commit. Use « Bring … into … » instead.".into());
    }
    if e.contains("has no upstream branch") {
        return Some("This branch has no remote branch yet. Use Push… (right-click Push) to choose where it goes.".into());
    }
    None
}

/// Commits of a branch that no other branch has, for the delete check (worker thread).
pub(super) fn branch_check(repo: &std::path::Path, name: &str) -> Result<(String, Vec<(String, String)>), String> {
    let sha = git::branch_sha(repo, name).map_err(|e| e.0)?;
    let commits = git::unique_commits(repo, name).map_err(|e| e.0)?;
    Ok((sha, commits))
}
