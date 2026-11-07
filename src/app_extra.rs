//! Phases 4 and 5 of the design doc behind the buttons: staging lines, comparing, searching
//! the content, the full history, advanced stash, copying several commits, bisect,
//! `rebase --onto`, submodules, clone options, cleanup, sparse-checkout, patches, signatures,
//! maintenance and commit options.
//!
//! A child module of `app`. Local changes go through the safety net (`Task::Steps` or a
//! recorded `Task::Run`), so they can be undone; slow or network work runs on its own thread.
//! The git side is in `extras.rs`, tested in `tests.rs`.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui;

use super::{Action, Dialog, HunkMode, JobDone, OpKind, Role, Task, ToastKind, VegitApp};
use crate::extras::{self, BisectOutcome};
use crate::git::{self, FileChange, FileState};
use crate::lfs::human_size;
use crate::safety::{ResetMode, Step};

/// How the graph's search field searches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchMode {
    #[default]
    Message,
    Author,
    /// Commits adding or removing the text (`log -S`).
    ChangedText,
    /// Commits touching a matching file path.
    FilePath,
}

impl SearchMode {
    pub const ALL: [SearchMode; 4] = [SearchMode::Message, SearchMode::Author, SearchMode::ChangedText, SearchMode::FilePath];

    pub fn label(self) -> &'static str {
        match self {
            SearchMode::Message => "Message",
            SearchMode::Author => "Author",
            SearchMode::ChangedText => "Changed text",
            SearchMode::FilePath => "File path",
        }
    }

    /// Searched by git in the background (Enter starts it), not in the loaded commits.
    pub fn in_content(self) -> bool {
        matches!(self, SearchMode::ChangedText | SearchMode::FilePath)
    }
}

/// Two commits or two branches compared in the detail panel.
#[derive(Clone, Debug, Default)]
pub struct CompareView {
    /// "a1b2c3 → d4e5f6" or "main ↔ feature".
    pub title: String,
    pub a_name: String,
    pub b_name: String,
    /// The diff is taken from `from` to `to` (for branches: from the fork point).
    pub from: String,
    pub to: String,
    pub files: Option<Vec<FileChange>>,
    /// Branches only: commits only on a, only on b.
    pub only_a: Vec<(String, String)>,
    pub only_b: Vec<(String, String)>,
    pub branches: bool,
    /// 0: commits, 1: files.
    pub page: usize,
}

/// Lines picked in the diff gutter: (hunk, lines of its body).
pub type LinePick = (usize, Vec<usize>);

impl VegitApp {
    pub(super) fn handle_extra(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::PickLine { hunk, line } => {
                let Some(t) = self.tab() else { return };
                if hunk == usize::MAX {
                    t.line_pick = None;
                    return;
                }
                // Lines are picked within one hunk at a time.
                let pick = t.line_pick.get_or_insert((hunk, Vec::new()));
                if pick.0 != hunk {
                    *pick = (hunk, Vec::new());
                }
                match pick.1.iter().position(|l| *l == line) {
                    Some(i) => {
                        pick.1.remove(i);
                    }
                    None => pick.1.push(line),
                }
                if pick.1.is_empty() {
                    t.line_pick = None;
                }
            }
            Action::StageLines { mode } => self.stage_lines(ctx, mode),
            Action::CompareBranches { a, b } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                if let Some(t) = self.tab() {
                    t.compare = Some(CompareView { title: format!("{a} ↔ {b}"), a_name: a.clone(), b_name: b.clone(), branches: true, ..Default::default() });
                    t.diff = None;
                }
                self.send(Task::Compare { repo, a, b, branches: true });
            }
            Action::CloseCompare => {
                if let Some(t) = self.tab() {
                    t.compare = None;
                    t.diff = None;
                }
            }
            Action::ContentSearch => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let Some(t) = self.tab() else { return };
                let (query, mode) = (t.search.trim().to_string(), t.search_mode);
                if query.is_empty() || !mode.in_content() {
                    t.search_hits = None;
                    return;
                }
                t.search_hits = Some((query.clone(), mode, None));
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                // `log -S` can take a while on a big repository: its own thread.
                std::thread::spawn(move || {
                    let result = match mode {
                        SearchMode::ChangedText => extras::search_changed_text(&repo, &query),
                        _ => extras::search_file_path(&repo, &query),
                    }
                    .map_err(|e| e.0);
                    let _ = tx.send(JobDone::Search { tab_path: repo, query, result });
                    ctx2.request_repaint();
                });
            }
            Action::LoadMoreHistory => {
                let Some(repo) = self.active_path() else { return };
                git::set_commit_limit(&repo, git::commit_limit(&repo) + git::MAX_COMMITS);
                self.send(Task::Reload { repo });
            }
            Action::GoToCommit(sha) => {
                let Some(repo) = self.active_path() else { return };
                match extras::position_of(&repo, &sha) {
                    Some(pos) => {
                        git::set_commit_limit(&repo, pos + 200);
                        if let Some(t) = self.tab() {
                            t.select_after = Some(sha);
                        }
                        self.send(Task::Reload { repo });
                    }
                    None => self.toast(ctx, "This commit isn't in any branch or tag of this repository.", ToastKind::Info),
                }
            }
            Action::StashFiles(paths) => {
                let n = paths.len();
                let args = extras::stash_files_args(&paths, &format!("{n} file{} put aside", if n == 1 { "" } else { "s" }));
                self.queue_git(ctx, Some(format!("Stashed {n} file{}", if n == 1 { "" } else { "s" })), args, None, OpKind::Plain);
            }
            Action::CopyCommits { shas, mention, no_commit } => {
                let Some(t) = self.tab() else { return };
                // Oldest first, the order they were made in.
                let mut ordered: Vec<(usize, String)> = shas.iter().filter_map(|s| t.commit_index(s).map(|i| (i, s.clone()))).collect();
                ordered.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
                let ordered: Vec<String> = ordered.into_iter().map(|(_, s)| s).collect();
                let target = t.snap.head_branch.clone().unwrap_or_else(|| "HEAD".into());
                let n = ordered.len();
                t.picked_commits.clear();
                let ok = if no_commit { format!("The changes of {n} commits are staged") } else { format!("Copied {n} commits onto {target}") };
                self.queue_git(ctx, Some(ok), extras::cherry_pick_args(&ordered, mention, no_commit), None, OpKind::Plain);
            }
            Action::StartBisect(good) => {
                let ok = Some("Testing a commit halfway: does the problem happen here? Answer in the banner.".to_string());
                self.queue_git(ctx, ok, vec!["bisect".into(), "start".into(), "HEAD".into(), good], None, OpKind::Bisect);
            }
            Action::BisectAnswer(word) => {
                self.queue_git(ctx, None, vec!["bisect".into(), word], None, OpKind::Bisect);
            }
            Action::BisectStop => self.queue_git(ctx, Some("Back where you started".into()), vec!["bisect".into(), "reset".into()], None, OpKind::Plain),
            Action::BisectRun(cmd) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.spawn_steps(ctx, repo, "Bisect".into(), vec![vec!["bisect".into(), "run".into(), "sh".into(), "-c".into(), cmd]], None, OpKind::Bisect);
            }
            Action::OpenOnto(branch) => {
                let Some(t) = self.tab() else { return };
                let mut options: Vec<String> = t.snap.refs.iter().filter(|r| r.kind != git::RefKind::Tag && r.name != branch).map(|r| r.name.clone()).collect();
                options.sort();
                let new_base = options.iter().position(|o| o == "main" || o == "master").unwrap_or(0);
                let old_base = options.iter().position(|o| t.snap.refs.iter().any(|r| &r.name == o && r.kind == git::RefKind::Local) && o != &options[new_base]).unwrap_or(0);
                self.dialog = Some(Dialog::Onto { branch, options, new_base, old_base });
            }
            Action::MoveOnto { branch, new_base, old_base } => {
                let ok = Some(format!("Moved the commits of {branch} onto {new_base}"));
                self.queue_git(ctx, ok, extras::onto_args(&new_base, &old_base, &branch), None, OpKind::Plain);
            }
            Action::AddSubmodule { url, path } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let ok = Some(format!("Submodule {path} added (stage and commit to keep it)"));
                self.spawn_steps(ctx, repo, "Submodule".into(), vec![extras::submodule_add_args(&url, &path)], ok, OpKind::Plain);
            }
            Action::SubmoduleUpdate(path) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.spawn_steps(ctx, repo, "Submodule".into(), vec![extras::submodule_update_args(&path)], Some(format!("{path} updated to its latest commit")), OpKind::Plain);
            }
            Action::SubmoduleSync(path) => self.queue_git(ctx, Some(format!("URL of {path} synced")), extras::submodule_sync_args(&path), None, OpKind::Plain),
            Action::SubmoduleRemove(path) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let risks = extras::submodule_risks(&repo, &path);
                self.dialog = Some(Dialog::RemoveSubmodule { path, risks });
            }
            Action::SubmoduleRemoveNow { path, keep_git_data } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let steps = extras::submodule_remove_steps(&repo, &path, keep_git_data);
                self.send(Task::Steps { repo: repo.clone(), label: "Remove submodule".into(), ok_text: Some(format!("Submodule {path} removed (commit to keep it so)")), steps, what: None, kind: OpKind::Plain, role: Role::New });
                self.send(Task::Reload { repo });
            }
            Action::ListHeads(url) => {
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let result = extras::remote_heads(&super::dirs_home(), &url).map_err(|e| e.0);
                    let _ = tx.send(JobDone::Heads { url, result });
                    ctx2.request_repaint();
                });
            }
            Action::LoadFullHistory => self.git_background(ctx, "Fetch".into(), vec!["fetch".into(), "--unshallow".into()]),
            Action::CleanNew => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let Some(t) = self.tab() else { return };
                let untracked: Vec<String> = t.snap.status.unstaged.iter().filter(|f| f.state == FileState::Untracked).map(|f| f.path.clone()).collect();
                let n = untracked.len();
                let steps = extras::clean_steps(&untracked);
                let ok = Some(format!("Deleted {n} new file{}", if n == 1 { "" } else { "s" }));
                self.send(Task::Steps { repo: repo.clone(), label: "Clean".into(), ok_text: ok, steps, what: Some((format!("delete new files ({n})"), ResetMode::Keep)), kind: OpKind::Plain, role: Role::New });
                self.send(Task::Reload { repo });
            }
            Action::OpenCleanIgnored => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                // Measuring Library/ can take a moment.
                std::thread::spawn(move || {
                    let (files, size) = extras::ignored(&repo);
                    let _ = tx.send(JobDone::Ignored { files, size });
                    ctx2.request_repaint();
                });
            }
            Action::CleanIgnored => self.queue_git(ctx, Some("Ignored files deleted".into()), vec!["clean".into(), "-f".into(), "-d".into(), "-X".into(), "-q".into()], None, OpKind::Plain),
            Action::OpenSparse => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let kept = extras::sparse_folders(&repo);
                let folders = extras::folders(&repo).into_iter().map(|f| {
                    let on = kept.as_ref().is_none_or(|k| k.iter().any(|x| f == *x || f.starts_with(&format!("{x}/")) || x.starts_with(&format!("{f}/"))));
                    (f, on)
                });
                self.dialog = Some(Dialog::Sparse { folders: folders.collect() });
            }
            Action::SetSparse(folders) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let (args, ok) = match folders {
                    Some(f) => (extras::sparse_set_args(&f), format!("Only {} folder{} downloaded", f.len(), if f.len() == 1 { "" } else { "s" })),
                    None => (vec!["sparse-checkout".into(), "disable".into()], "Every folder is downloaded again".to_string()),
                };
                self.spawn_steps(ctx, repo, "Sparse".into(), vec![args], Some(ok), OpKind::Plain);
            }
            Action::SavePatch(shas) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let Some(t) = self.tab() else { return };
                let mut ordered: Vec<(usize, String)> = shas.iter().filter_map(|s| t.commit_index(s).map(|i| (i, s.clone()))).collect();
                ordered.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
                let ordered: Vec<String> = ordered.into_iter().map(|(_, s)| s).collect();
                let name = format!("{}.patch", ordered.last().map(|s| git::short(s)).unwrap_or("changes"));
                let Some(file) = rfd::FileDialog::new().set_title("Save the patch file").set_file_name(&name).save_file() else { return };
                match extras::format_patch(&repo, &ordered).map_err(|e| e.0).and_then(|p| std::fs::write(&file, p).map_err(|e| e.to_string())) {
                    Ok(()) => self.toast(ctx, format!("Saved {}", file.display()), ToastKind::Success),
                    Err(e) => self.toast(ctx, format!("Could not save the patch: {e}"), ToastKind::Error),
                }
            }
            Action::OpenPatch(file) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let preview = extras::patch_preview(&repo, &file);
                self.dialog = Some(Dialog::ApplyPatch { file, preview });
            }
            Action::ApplyPatch(file) => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let steps = vec![Step::Git(extras::patch_apply_args(&file))];
                let ok = Some(format!("Applied {}", file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                self.send(Task::Steps { repo: repo.clone(), label: "Patch".into(), ok_text: ok, steps, what: Some(("apply patch".into(), ResetMode::Keep)), kind: OpKind::Plain, role: Role::New });
                self.send(Task::Reload { repo });
            }
            Action::SpeedUp => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.busy.push("Speeding up".into());
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let before = extras::repo_size(&repo);
                    let result = git::run(&repo, &["gc", "-q"]).map_err(|e| e.0).map(|_| (before, extras::repo_size(&repo)));
                    let _ = tx.send(JobDone::Maintenance { result: result.map(|(b, a)| format!("Repository tidied: {} → {}", human_size(b), human_size(a))) });
                    ctx2.request_repaint();
                });
            }
            Action::KeepFast => self.queue_git(ctx, Some("Git will now tidy this repository in the background, every hour".into()), vec!["maintenance".into(), "start".into()], None, OpKind::Plain),
            Action::CheckRepo => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                self.busy.push("Speeding up".into());
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let result = extras::check(&repo).map(|p| {
                        if p.is_empty() {
                            "No problems found: every object of the repository is intact.".to_string()
                        } else {
                            format!("{} problem{} found:\n{}", p.len(), if p.len() == 1 { "" } else { "s" }, p.iter().take(6).cloned().collect::<Vec<_>>().join("\n"))
                        }
                    });
                    let _ = tx.send(JobDone::Maintenance { result });
                    ctx2.request_repaint();
                });
            }
            Action::GrepAt { sha, query } => {
                let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let result = extras::grep_at(&repo, &sha, &query).map_err(|e| e.0);
                    let _ = tx.send(JobDone::Grep { sha, query, result });
                    ctx2.request_repaint();
                });
            }
            Action::SetCommitOptions(opts) => {
                if let Some(t) = self.tab() {
                    t.commit_opts = opts;
                }
            }
            _ => {}
        }
    }

    /// Stages, unstages or discards the lines picked in the diff's gutter.
    fn stage_lines(&mut self, ctx: &egui::Context, mode: HunkMode) {
        let Some(repo) = self.active_path() else { return self.sample_refusal(ctx) };
        let Some(t) = self.tab() else { return };
        let Some((hunk, lines)) = t.line_pick.take() else { return };
        let Some(dv) = &t.diff else { return };
        let Some(patch) = dv.parsed.as_ref().and_then(|p| p.hunk_patch(hunk)) else { return };
        let reverse = mode != HunkMode::Stage;
        let Some(partial) = extras::line_patch(&patch, &lines, reverse) else { return };
        let path = dv.file.path.clone();
        let n = lines.len();
        let (args, steps_first, what, ok): (Vec<&str>, Option<Step>, Option<(String, ResetMode)>, String) = match mode {
            HunkMode::Stage => (vec!["apply", "--cached", "--whitespace=nowarn", "-"], None, None, String::new()),
            HunkMode::Unstage => (vec!["apply", "--cached", "-R", "--whitespace=nowarn", "-"], None, None, String::new()),
            HunkMode::Discard => (
                vec!["apply", "-R", "--whitespace=nowarn", "-"],
                Some(Step::Backup(vec![path])),
                Some(("discard lines".into(), ResetMode::Keep)),
                format!("{n} line{} discarded", if n == 1 { "" } else { "s" }),
            ),
        };
        let mut steps: Vec<Step> = steps_first.into_iter().collect();
        steps.push(Step::Input(args.iter().map(|s| s.to_string()).collect(), partial));
        let ok = (!ok.is_empty()).then_some(ok);
        self.send(Task::Steps { repo: repo.clone(), label: "Lines".into(), ok_text: ok, steps, what, kind: OpKind::Plain, role: Role::New });
        self.send(Task::Reload { repo });
    }

    /// Two commits picked: compare them (older → newer).
    pub(super) fn compare_picked(&mut self) {
        let Some(repo) = self.active_path() else { return };
        let Some(t) = self.tab() else { return };
        if t.picked_commits.len() != 2 {
            if t.compare.as_ref().is_some_and(|c| !c.branches) {
                t.compare = None;
            }
            return;
        }
        let mut two: Vec<(usize, String)> = t.picked_commits.iter().filter_map(|s| t.commit_index(s).map(|i| (i, s.clone()))).collect();
        two.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
        let [(_, older), (_, newer)] = [two[0].clone(), two[1].clone()];
        t.compare = Some(CompareView {
            title: format!("Comparing {} → {}", git::short(&older), git::short(&newer)),
            a_name: git::short(&older).into(),
            b_name: git::short(&newer).into(),
            from: older.clone(),
            to: newer.clone(),
            page: 1,
            ..Default::default()
        });
        self.send(Task::Compare { repo, a: older, b: newer, branches: false });
    }

    /// After a bisect answer: the next commit to test, or the first broken one.
    pub(super) fn after_bisect(&mut self, ctx: &egui::Context, out: &str) {
        if let BisectOutcome::Found(sha) = extras::parse_bisect(out) {
            let summary = self.tab().and_then(|t| t.commit(&sha).map(|c| c.summary.clone())).unwrap_or_default();
            self.toast(ctx, format!("The first broken commit is {} {summary}", git::short(&sha)), ToastKind::Success);
            if let Some(t) = self.tab() {
                t.select_after = Some(sha);
            }
            self.queue_git(ctx, None, vec!["bisect".into(), "reset".into()], None, OpKind::Plain);
        }
    }

    pub(super) fn on_compare(&mut self, tab_path: &PathBuf, result: Result<super::CompareResult, String>) {
        let Some(t) = self.tab_by_path(tab_path) else { return };
        let Some(cv) = t.compare.as_mut() else { return };
        match result {
            Ok(r) => {
                cv.from = r.from;
                cv.to = r.to;
                cv.files = Some(r.files);
                cv.only_a = r.only_a;
                cv.only_b = r.only_b;
            }
            Err(e) => {
                cv.files = Some(Vec::new());
                cv.title = format!("{} · {e}", cv.title);
            }
        }
    }

    /// Signatures of recent commits, read in the background after each reload.
    pub(super) fn spawn_signatures(&mut self, ctx: &egui::Context, repo: PathBuf) {
        let tx = self.jobs_tx.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let sigs = extras::signatures(&repo, 300);
            let _ = tx.send(JobDone::Signatures { tab_path: repo, sigs });
            ctx2.request_repaint();
        });
    }

    /// Files dropped on the window: a `.patch` / `.diff` opens its preview.
    pub fn dropped_files(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = files.into_iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("patch") || e.eq_ignore_ascii_case("diff"))) {
            self.handle_extra(ctx, Action::OpenPatch(p));
        }
    }

    /// The commit options of the active repository, applied to a commit's arguments.
    pub(super) fn commit_extras(&mut self, description: String) -> (Vec<String>, String) {
        match self.tab() {
            Some(t) => (t.commit_opts.args(), t.commit_opts.description(&description)),
            None => (Vec::new(), description),
        }
    }
}

/// Work for the worker: what the compare panel shows.
pub(super) fn compare(repo: &std::path::Path, a: &str, b: &str, branches: bool) -> Result<super::CompareResult, String> {
    if branches {
        let base = extras::merge_base(repo, a, b).ok_or("These branches share no history.")?;
        let to = extras::rev(repo, b).ok_or_else(|| format!("{b} not found"))?;
        Ok(super::CompareResult {
            files: extras::compare_files(repo, &base, &to).map_err(|e| e.0)?,
            only_a: extras::only_on(repo, b, a),
            only_b: extras::only_on(repo, a, b),
            from: base,
            to,
        })
    } else {
        Ok(super::CompareResult { files: extras::compare_files(repo, a, b).map_err(|e| e.0)?, only_a: Vec::new(), only_b: Vec::new(), from: a.into(), to: b.into() })
    }
}

/// The hits of a content search, for the graph (`None`: not a content search, or running).
pub fn content_hits(tab: &super::RepoTab) -> Option<&HashSet<String>> {
    match &tab.search_hits {
        Some((q, m, Some(Ok(hits)))) if *q == tab.search.trim() && *m == tab.search_mode => Some(hits),
        _ => None,
    }
}
