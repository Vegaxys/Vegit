//! Application state, the action queue and the git operations behind every button.
//!
//! UI code never runs git directly: it pushes [`Action`]s while drawing, and
//! [`VegitApp::handle`] turns them into [`Task`]s for one worker thread. The window thread
//! never waits on git, so a huge diff (a Unity scene, a big asset import) can take its time
//! without freezing anything. Tasks run in order, which keeps "stage, then reload" correct.
//! Network operations (fetch, pull, push, clone) get their own threads so local work
//! is never stuck behind a slow server.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::access;
use crate::demo;
use crate::forge::{self, Forge, PullRequest, RemoteRepo};
use crate::diff::{DiffMode, ParsedDiff};
use crate::git::{self, Commit, DiffSource, FileChange, FileState, Snapshot};
use crate::graph::{self, GraphLayout};
use crate::preview;
use crate::theme::{self, Appearance, LaneMode, Palette, ThemeChoice};

pub const SETTINGS_KEY: &str = "vegit_settings";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Colour preset the rest of the appearance starts from.
    pub theme: ThemeChoice,
    pub recent: Vec<String>,
    pub open_tabs: Vec<String>,
    /// File lists as a folder tree instead of full paths.
    pub tree_view: bool,
    /// Hunk, inline or split view of changes.
    pub diff_mode: DiffMode,
    /// Interface colour replacing the preset's accent.
    pub accent: Option<[u8; 3]>,
    /// Font files replacing the preset's fonts (`None`: the preset's).
    pub ui_font: Option<String>,
    pub code_font: Option<String>,
    pub text_size: f32,
    pub lane_mode: LaneMode,
    pub lane_color: [u8; 3],
    pub lane_seed: u64,
    /// Keep the checked-out branch on the first column of the graph.
    pub pin_head: bool,
    pub show_descriptions: bool,
    /// Fetch profile pictures (Gravatar / GitHub) for commit authors.
    pub show_avatars: bool,
    /// Widths of the graph columns: branch/tag, graph, message, author, date, id.
    pub columns: [f32; 6],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            recent: Vec::new(),
            open_tabs: Vec::new(),
            tree_view: false,
            diff_mode: DiffMode::default(),
            accent: None,
            ui_font: None,
            code_font: None,
            text_size: 13.5,
            lane_mode: LaneMode::default(),
            lane_color: [0x2d, 0x8c, 0xff],
            lane_seed: 7,
            pin_head: true,
            show_descriptions: true,
            show_avatars: true,
            columns: [150.0, 100.0, 220.0, 130.0, 90.0, 70.0],
        }
    }
}

impl Settings {
    pub fn appearance(&self) -> Appearance {
        Appearance {
            theme: self.theme,
            accent: self.accent,
            ui_font: self.ui_font.clone(),
            code_font: self.code_font.clone(),
            text_size: self.text_size,
            lanes: self.lane_mode,
            lane_color: self.lane_color,
            seed: self.lane_seed,
        }
    }
}

/// Which file list a folder belongs to, so collapsing one list leaves the others alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListId {
    Unstaged,
    Staged,
    Commit,
}

/// One side of an image change, ready to draw.
pub struct ShownImage {
    pub texture: egui::TextureHandle,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

pub enum ImageState {
    Loading,
    Ready { before: Option<Result<ShownImage, String>>, after: Option<Result<ShownImage, String>> },
}

/// Pull requests of the repository's hosting service.
pub enum PrState {
    NotLoaded,
    Loading,
    Ready(RemoteRepo, Vec<PullRequest>),
    /// No supported service, not signed in, or the service failed; the text says which.
    Unavailable(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileViewKind {
    History,
    Blame,
}

/// The middle tile showing one file's history or blame instead of the graph.
pub struct FileView {
    pub path: String,
    pub kind: FileViewKind,
    pub history: Option<Result<Vec<git::HistoryEntry>, String>>,
    pub selected: Option<usize>,
    pub blame: Option<Result<Vec<git::BlameLine>, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Wip,
    Commit(String),
}

pub struct CommitDetail {
    pub sha: String,
    pub message: String,
    pub committer: String,
    pub files: Vec<FileChange>,
}

pub struct DiffView {
    pub source: DiffSource,
    pub file: FileChange,
    /// `None` while the worker is still producing it.
    pub parsed: Option<ParsedDiff>,
    /// Matches the request that is allowed to fill this view; older answers are dropped.
    pub token: u64,
    /// Picture files: show the rendered images instead of the code.
    pub render: bool,
    /// `None` for files that aren't pictures.
    pub images: Option<ImageState>,
    /// The diff carries the whole file (inline and split views) rather than hunks.
    pub whole: bool,
    /// Row to scroll to on the next frame (previous/next change buttons).
    pub jump: Option<usize>,
    /// First visible row, remembered from the last frame.
    pub top_row: usize,
}

/// One open repository (a tab).
pub struct RepoTab {
    /// `None` for the built-in sample.
    pub path: Option<PathBuf>,
    pub name: String,
    pub snap: Snapshot,
    pub graph: GraphLayout,
    pub refs_by_sha: HashMap<String, Vec<usize>>,
    /// True until the first snapshot arrives from the worker.
    pub loading: bool,
    pub selection: Selection,
    pub detail: Option<CommitDetail>,
    pub diff: Option<DiffView>,
    pub summary: String,
    pub description: String,
    pub amend: bool,
    pub side_filter: String,
    pub search: String,
    /// Row to bring into view on the next frame.
    pub scroll_to: Option<usize>,
    pub last_poll: f64,
    pub polling: bool,
    pub fingerprint: String,
    /// After a commit, show the new commit once the reload lands.
    select_head_after_reload: bool,
    pub collapsed: [HashSet<String>; 3],
    /// Commit removed by Undo and the commit it left HEAD on, so Redo can bring it back.
    pub redo: Option<(String, String)>,
    pub file_view: Option<FileView>,
    /// Files picked in the working-changes lists (Ctrl/Shift+click), all in one list.
    pub picked: HashSet<String>,
    pub picked_list: ListId,
    /// Last plainly clicked file, start of a Shift+click range.
    pub anchor: Option<String>,
    pub prs: PrState,
    /// Unstaged / staged lists folded to their header.
    pub folded: [bool; 2],
    /// Share of the height given to the unstaged list, once the divider was dragged.
    pub wip_split: Option<f32>,
    /// Only this author's commits stay bright in the graph.
    pub author_filter: Option<String>,
    /// Horizontal scroll of the graph rows, so the header can follow it.
    pub graph_hscroll: f32,
}

impl RepoTab {
    pub fn new(path: Option<PathBuf>) -> Self {
        let name = match &path {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
            None => "sample-project".to_string(),
        };
        Self {
            path,
            name,
            snap: Snapshot::default(),
            graph: GraphLayout::default(),
            refs_by_sha: HashMap::new(),
            loading: true,
            selection: Selection::Wip,
            detail: None,
            diff: None,
            summary: String::new(),
            description: String::new(),
            amend: false,
            side_filter: String::new(),
            search: String::new(),
            scroll_to: None,
            last_poll: 0.0,
            polling: false,
            fingerprint: String::new(),
            select_head_after_reload: false,
            collapsed: Default::default(),
            redo: None,
            file_view: None,
            picked: HashSet::new(),
            picked_list: ListId::Unstaged,
            anchor: None,
            prs: PrState::NotLoaded,
            folded: [false, false],
            wip_split: None,
            author_filter: None,
            graph_hscroll: 0.0,
        }
    }

    pub fn is_sample(&self) -> bool {
        self.path.is_none()
    }

    fn set_snapshot(&mut self, snap: Snapshot, graph: GraphLayout) {
        let first = self.loading;
        self.loading = false;
        self.graph = graph;
        self.refs_by_sha = snap.refs_by_sha();
        self.snap = snap;
        if let Selection::Commit(sha) = &self.selection {
            if !self.snap.commits.iter().any(|c| &c.sha == sha) {
                self.selection = Selection::Wip;
                self.detail = None;
            }
        }
        if first && self.snap.status.is_clean() {
            if let Some(h) = self.snap.head_sha.clone() {
                self.selection = Selection::Commit(h);
            }
        }
    }

    pub fn commit_index(&self, sha: &str) -> Option<usize> {
        self.snap.commits.iter().position(|c| c.sha == sha)
    }

    pub fn commit(&self, sha: &str) -> Option<&Commit> {
        self.snap.commits.iter().find(|c| c.sha == sha)
    }

    /// The repository on its hosting service (origin first), for web links.
    pub fn web_repo(&self) -> Option<RemoteRepo> {
        let mut urls: Vec<&(String, String)> = self.snap.remote_urls.iter().collect();
        urls.sort_by_key(|(n, _)| n != "origin");
        urls.iter().find_map(|(n, u)| forge::detect(n, u))
    }

    /// The graph shows a "work in progress" row on top when there are local changes.
    pub fn has_wip_row(&self) -> bool {
        !self.snap.status.is_clean() || self.snap.head_sha.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkMode {
    Stage,
    Unstage,
    Discard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullMode {
    FastForwardIfPossible,
    FastForwardOnly,
    Rebase,
}

#[derive(Clone, Debug)]
pub enum Dialog {
    NewBranch { at: Option<String>, name: String, checkout: bool },
    RenameBranch { old: String, name: String },
    NewTag { at: String, name: String, message: String },
    Stash { message: String, include_untracked: bool },
    Confirm { title: String, body: String, confirm: String, then: Box<Action> },
    Access(AccessForm),
    NewPullRequest(PrForm),
    Settings,
}

/// The "Create pull request" window: from a repository and branch into another.
#[derive(Clone, Debug)]
pub struct PrForm {
    /// Remotes on a supported service; `from` and `to` index into it.
    pub remotes: Vec<RemoteRepo>,
    pub forge: Forge,
    pub from: usize,
    pub from_branch: String,
    pub to: usize,
    pub to_branch: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
    /// Branches known on each remote, as (remote, branch).
    pub branches: Vec<(String, String)>,
    pub current: Option<String>,
}

impl PrForm {
    pub fn branches_of(&self, remote: usize) -> Vec<&str> {
        let Some(r) = self.remotes.get(remote) else { return Vec::new() };
        self.branches.iter().filter(|(rem, _)| *rem == r.remote).map(|(_, b)| b.as_str()).collect()
    }

    /// The source branch exists on the source remote, so the service can see it.
    pub fn from_is_pushed(&self) -> bool {
        self.branches_of(self.from).contains(&self.from_branch.as_str())
    }

    /// Picks sensible remotes and branches for the chosen service.
    pub fn select_forge(&mut self, forge: Forge) {
        self.forge = forge;
        let of_forge: Vec<usize> = (0..self.remotes.len()).filter(|&i| self.remotes[i].forge == forge).collect();
        let Some(&first) = of_forge.first() else { return };
        self.from = of_forge.iter().copied().find(|&i| self.remotes[i].remote == "origin").unwrap_or(first);
        // Fork workflow: propose the original ("upstream") as the destination.
        self.to = of_forge.iter().copied().find(|&i| self.remotes[i].remote == "upstream").unwrap_or(self.from);
        self.from_branch = self.current.clone().unwrap_or_default();
        self.to_branch = self.default_target();
    }

    pub fn default_target(&self) -> String {
        let names = self.branches_of(self.to);
        // The usual default branch, unless that is the branch being proposed.
        let source = Some(self.from_branch.as_str()).filter(|b| !b.is_empty()).or(self.current.as_deref());
        ["main", "master", "develop"]
            .iter()
            .find(|n| names.contains(n) && source != Some(**n))
            .map(|n| n.to_string())
            .or_else(|| names.iter().find(|n| Some(n.to_string()) != self.current).map(|n| n.to_string()))
            .unwrap_or_else(|| "main".into())
    }
}

#[derive(Clone, Debug, Default)]
pub struct AccessForm {
    pub host: String,
    pub user: String,
    pub token: String,
    pub test_url: String,
    /// Credential helper and SSH key are read once when the window opens, not every frame.
    pub checked: bool,
    pub helper: Option<String>,
    pub ssh_key: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub enum Action {
    Select(Selection),
    SelectRef(String),
    OpenDiff(DiffSource, FileChange),
    CloseDiff,
    Git { label: String, args: Vec<String>, background: bool },
    Stage(FileChange),
    Unstage(FileChange),
    StageAll,
    UnstageAll,
    Hunk { index: usize, mode: HunkMode },
    Commit,
    Dialog(Dialog),
    CheckoutLocal(String),
    CheckoutRemote(String),
    Pull(PullMode),
    Push { force: bool },
    Fetch,
    UndoLastCommit,
    OpenRepo(PathBuf),
    OpenSample,
    InitRepo(PathBuf),
    Clone { url: String, dest: PathBuf },
    CloseTab(usize),
    SwitchTab(usize),
    ShowHome,
    Refresh,
    Copy(String),
    OpenTerminal,
    OpenInExplorer(Option<String>),
    OpenFile(String),
    SetTheme(ThemeChoice),
    SaveToken { host: String, user: String, token: String },
    TestAccess(String),
    CreateSshKey,
    UseCredentialManager,
    ToggleFolder(ListId, String),
    StagePath(String),
    UnstagePath(String),
    SetRender(bool),
    SetTreeView(bool),
    Redo,
    FileHistory(String),
    FileBlame(String),
    PickHistory(usize),
    CloseFileView,
    StageMany(Vec<FileChange>),
    UnstageMany(Vec<FileChange>),
    DiscardMany(Vec<FileChange>),
    SetDiffMode(DiffMode),
    Jump(bool),
    ResolveConflict(String, bool),
    CollapseAll(ListId, bool),
    LoadPullRequests { interactive: bool },
    CheckoutPullRequest(usize),
    OpenUrl(String),
    /// Opens the pull request window, optionally starting from a given branch.
    OpenPullRequestForm(Option<String>),
    CreatePullRequest(PrForm),
    OpenSubmodule(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub until: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpKind {
    Plain,
    Commit,
}

/// Work for the worker thread, processed strictly in order.
enum Task {
    Run { repo: PathBuf, label: String, ok_text: Option<String>, args: Vec<String>, input: Option<String>, kind: OpKind },
    Delete { repo: PathBuf, rel: String },
    Reload { repo: PathBuf },
    Diff { repo: PathBuf, source: DiffSource, file: FileChange, token: u64, whole: bool },
    Detail { repo: PathBuf, commit: Commit },
    Images { repo: PathBuf, source: DiffSource, file: FileChange, token: u64 },
    FileLog { repo: PathBuf, path: String },
    Blame { repo: PathBuf, path: String },
}

/// Result of work done off the window thread.
enum JobDone {
    Op { tab_path: PathBuf, label: String, ok_text: Option<String>, result: Result<String, String>, kind: OpKind, from_worker: bool },
    Snapshot { tab_path: PathBuf, result: Result<(Snapshot, GraphLayout), String> },
    Diff { tab_path: PathBuf, token: u64, parsed: ParsedDiff },
    Detail { tab_path: PathBuf, detail: CommitDetail },
    Images { tab_path: PathBuf, token: u64, pair: preview::Pair },
    FileLog { tab_path: PathBuf, path: String, result: Result<Vec<git::HistoryEntry>, String> },
    Blame { tab_path: PathBuf, path: String, result: Result<Vec<git::BlameLine>, String> },
    Cloned { dest: PathBuf, result: Result<String, String> },
    Prs { tab_path: PathBuf, result: Result<(RemoteRepo, Vec<PullRequest>), String> },
    PrCreated { tab_path: PathBuf, result: Result<String, String> },
    Poll { tab_path: PathBuf, status: Option<git::WorkStatus>, fingerprint: String },
}

pub struct HomeForm {
    pub open_path: String,
    pub clone_url: String,
    pub clone_dest: String,
    pub init_path: String,
}

pub struct ScreenshotJob {
    pub path: PathBuf,
    pub frame: u32,
    pub requested: bool,
    pub done: Arc<AtomicBool>,
}

pub struct VegitApp {
    pub tabs: Vec<RepoTab>,
    pub active: usize,
    pub show_home: bool,
    pub settings: Settings,
    pub toasts: Vec<Toast>,
    pub dialog: Option<Dialog>,
    pub home: HomeForm,
    /// Labels of running network operations.
    pub busy: Vec<String>,
    /// Tasks sent to the worker and not answered yet.
    pub pending: usize,
    worker: Sender<Task>,
    jobs_tx: Sender<JobDone>,
    jobs_rx: Receiver<JobDone>,
    pub actions: Vec<Action>,
    pub screenshot: Option<ScreenshotJob>,
    applied_theme: Option<Appearance>,
    next_token: u64,
}

impl VegitApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> Self {
        let settings: Settings = if screenshot.is_some() {
            Settings::default()
        } else {
            cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default()
        };
        let (tx, rx) = mpsc::channel();
        let worker = spawn_worker(tx.clone(), cc.egui_ctx.clone());
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            show_home: false,
            settings,
            toasts: Vec::new(),
            dialog: None,
            home: HomeForm {
                open_path: String::new(),
                clone_url: String::new(),
                clone_dest: dirs_home().join("Projects").display().to_string(),
                init_path: String::new(),
            },
            busy: Vec::new(),
            pending: 0,
            worker,
            jobs_tx: tx,
            jobs_rx: rx,
            actions: Vec::new(),
            screenshot: screenshot.map(|path| ScreenshotJob {
                path,
                frame: 0,
                requested: false,
                done: Arc::new(AtomicBool::new(false)),
            }),
            applied_theme: None,
            next_token: 0,
        };

        if app.screenshot.is_some() {
            app.open_sample();
            // Preview switches for checking other screens in screenshot mode.
            if let Ok(v) = std::env::var("VEGIT_THEME") {
                if let Some(t) = ThemeChoice::ALL.iter().find(|t| format!("{t:?}").eq_ignore_ascii_case(&v)) {
                    app.settings.theme = *t;
                }
            }
            if let Ok(which) = std::env::var("VEGIT_PREVIEW_DIFF") {
                let files = &app.tabs[0].snap.status.unstaged;
                let pick = files.iter().find(|f| f.path.ends_with(&which)).or(files.first()).cloned();
                if let Some(f) = pick {
                    app.actions.push(Action::OpenDiff(DiffSource::Unstaged, f));
                }
            }
            if let Ok(m) = std::env::var("VEGIT_DIFF_MODE") {
                app.settings.diff_mode = match m.as_str() {
                    "split" => DiffMode::Split,
                    "inline" => DiffMode::Inline,
                    _ => DiffMode::Hunk,
                };
            }
            if std::env::var_os("VEGIT_PREVIEW_HISTORY").is_some() {
                app.actions.push(Action::FileHistory("src/search/panel.rs".into()));
            }
            if std::env::var_os("VEGIT_PREVIEW_PICK").is_some() {
                let t = &mut app.tabs[0];
                t.picked = t.snap.status.unstaged.iter().take(2).map(|f| f.path.clone()).collect();
            }
            if std::env::var_os("VEGIT_PREVIEW_PR").is_some() {
                app.actions.push(Action::OpenPullRequestForm(None));
            }
            if std::env::var_os("VEGIT_FOLD_STAGED").is_some() {
                app.tabs[0].folded[1] = true;
            }
            if std::env::var_os("VEGIT_PREVIEW_SETTINGS").is_some() {
                app.actions.push(Action::Dialog(Dialog::Settings));
            }
            if std::env::var_os("VEGIT_PREVIEW_COMMIT").is_some() {
                if let Some(c) = app.tabs[0].snap.commits.get(1).map(|c| c.sha.clone()) {
                    app.actions.push(Action::Select(Selection::Commit(c)));
                }
            }
            if std::env::var_os("VEGIT_TREE").is_some() {
                app.settings.tree_view = true;
            }
            if std::env::var_os("VEGIT_PREVIEW_ACCESS").is_some() {
                app.actions.push(Action::Dialog(Dialog::Access(AccessForm { host: "github.com".into(), ..Default::default() })));
            }
        } else {
            for p in app.settings.open_tabs.clone() {
                let _ = app.open_repo(Path::new(&p), false);
            }
            app.active = 0;
            if app.tabs.is_empty() {
                app.show_home = true;
            }
        }
        app
    }

    pub fn palette(&self) -> Palette {
        Palette::build(&self.settings.appearance())
    }

    pub fn tab(&mut self) -> Option<&mut RepoTab> {
        if self.show_home {
            return None;
        }
        self.tabs.get_mut(self.active)
    }

    fn tab_by_path(&mut self, path: &Path) -> Option<&mut RepoTab> {
        self.tabs.iter_mut().find(|t| t.path.as_deref() == Some(path))
    }

    /// Re-applies fonts and colours when an appearance setting changed, and keeps the
    /// global switches (graph pinning, avatars) in step with the settings.
    pub fn ensure_theme(&mut self, ctx: &egui::Context) {
        let a = self.settings.appearance();
        if self.applied_theme.as_ref() != Some(&a) {
            theme::apply(ctx, &a);
            self.applied_theme = Some(a);
        }
        crate::avatars::ENABLED.store(self.settings.show_avatars, std::sync::atomic::Ordering::Relaxed);
        let pin = self.settings.pin_head;
        if graph::PIN_HEAD.swap(pin, std::sync::atomic::Ordering::Relaxed) != pin {
            for t in &mut self.tabs {
                t.graph = graph::layout_for(&t.snap.commits, t.snap.head_sha.as_deref());
            }
        }
    }

    pub fn toast(&mut self, ctx: &egui::Context, text: impl Into<String>, kind: ToastKind) {
        let secs = if kind == ToastKind::Error { 10.0 } else { 4.0 };
        self.toasts.push(Toast { text: text.into(), kind, until: ctx.input(|i| i.time) + secs });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn remember_recent(&mut self, path: &Path) {
        let s = path.display().to_string();
        self.settings.recent.retain(|r| r != &s);
        self.settings.recent.insert(0, s);
        self.settings.recent.truncate(12);
    }

    fn send(&mut self, task: Task) {
        if self.worker.send(task).is_ok() {
            self.pending += 1;
        }
    }

    pub fn open_sample(&mut self) {
        if let Some(i) = self.tabs.iter().position(|t| t.is_sample()) {
            self.active = i;
        } else {
            let snap = demo::snapshot();
            let layout = graph::layout_for(&snap.commits, snap.head_sha.as_deref());
            let mut tab = RepoTab::new(None);
            tab.set_snapshot(snap, layout);
            tab.prs = demo::pull_requests();
            tab.selection = Selection::Wip;
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
        self.show_home = false;
    }

    /// Adds a tab right away and lets the worker read the repository, so even a huge
    /// repository opens without blocking the window.
    pub fn open_repo(&mut self, path: &Path, remember: bool) -> Result<(), String> {
        let root = git::repo_root(path).map_err(|e| format!("Not a git repository: {e}"))?;
        if let Some(i) = self.tabs.iter().position(|t| t.path.as_deref() == Some(root.as_path())) {
            self.active = i;
            self.show_home = false;
            return Ok(());
        }
        if remember {
            self.remember_recent(&root);
        }
        self.tabs.push(RepoTab::new(Some(root.clone())));
        self.active = self.tabs.len() - 1;
        self.show_home = false;
        self.sync_open_tabs();
        self.send(Task::Reload { repo: root });
        Ok(())
    }

    fn sync_open_tabs(&mut self) {
        self.settings.open_tabs = self
            .tabs
            .iter()
            .filter_map(|t| t.path.as_ref().map(|p| p.display().to_string()))
            .collect();
    }

    fn active_path(&mut self) -> Option<PathBuf> {
        self.tab().and_then(|t| t.path.clone())
    }

    fn reload_active(&mut self) {
        if let Some(repo) = self.active_path() {
            self.send(Task::Reload { repo });
        }
    }

    fn sample_refusal(&mut self, ctx: &egui::Context) {
        self.toast(ctx, "This is the sample project. Open one of your own repositories to make changes.", ToastKind::Info);
    }

    /// Queues a git command for the active repository, followed by a reload.
    fn queue_git(&mut self, ctx: &egui::Context, ok_text: Option<String>, args: Vec<String>, input: Option<String>, kind: OpKind) {
        let Some(repo) = self.active_path() else {
            self.sample_refusal(ctx);
            return;
        };
        let label = format!("git {}", args.first().map_or("", String::as_str));
        self.send(Task::Run { repo: repo.clone(), label, ok_text, args, input, kind });
        self.send(Task::Reload { repo });
    }

    fn queue(&mut self, ctx: &egui::Context, ok_text: &str, args: &[&str]) {
        let ok = (!ok_text.is_empty()).then(|| ok_text.to_string());
        self.queue_git(ctx, ok, args.iter().map(|s| s.to_string()).collect(), None, OpKind::Plain);
    }

    /// Runs git on its own thread (network operations) and reloads when done.
    fn git_background(&mut self, ctx: &egui::Context, label: String, args: Vec<String>) {
        let Some(path) = self.active_path() else {
            self.toast(ctx, "The sample project has no remote. Open one of your own repositories.", ToastKind::Info);
            return;
        };
        self.spawn_network(ctx, path, label, args);
    }

    fn spawn_network(&mut self, ctx: &egui::Context, dir: PathBuf, label: String, args: Vec<String>) {
        self.busy.push(label.clone());
        let tx = self.jobs_tx.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = git::run(&dir, &refs).map_err(|e| e.0);
            let ok_text = Some(format!("{label}: done"));
            let _ = tx.send(JobDone::Op { tab_path: dir, label, ok_text, result, kind: OpKind::Plain, from_worker: false });
            ctx2.request_repaint();
        });
    }

    /// Picks up finished jobs and polls the working tree every few seconds.
    pub fn pump_jobs(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.jobs_rx.try_recv() {
            match done {
                JobDone::Op { tab_path, label, ok_text, result, kind, from_worker } => {
                    if from_worker {
                        self.pending = self.pending.saturating_sub(1);
                    } else {
                        self.busy.retain(|b| b != &label);
                    }
                    match result {
                        Ok(_) => {
                            if let Some(t) = ok_text {
                                self.toast(ctx, t, ToastKind::Success);
                            }
                            if kind == OpKind::Commit {
                                if let Some(t) = self.tab_by_path(&tab_path) {
                                    t.summary.clear();
                                    t.description.clear();
                                    t.amend = false;
                                    t.select_head_after_reload = true;
                                }
                            }
                        }
                        Err(e) => {
                            let mut text = format!("{label} failed: {e}");
                            if access::looks_like_auth_error(&e) {
                                text.push_str("\n\nPrivate repository? Open Access (key button, top right) to sign in.");
                            }
                            self.toast(ctx, text, ToastKind::Error);
                        }
                    }
                    if !from_worker && self.tabs.iter().any(|t| t.path.as_ref() == Some(&tab_path)) {
                        if label == "Fetch" || label == "Push" {
                            self.spawn_prs(ctx, tab_path.clone(), false);
                        }
                        self.send(Task::Reload { repo: tab_path });
                    }
                }
                JobDone::Snapshot { tab_path, result } => {
                    self.pending = self.pending.saturating_sub(1);
                    match result {
                        Ok((snap, layout)) => self.apply_snapshot(ctx, &tab_path, snap, layout),
                        Err(e) => {
                            if let Some(t) = self.tab_by_path(&tab_path) {
                                t.loading = false;
                            }
                            self.toast(ctx, format!("Could not read the repository: {e}"), ToastKind::Error);
                        }
                    }
                }
                JobDone::Diff { tab_path, token, parsed } => {
                    self.pending = self.pending.saturating_sub(1);
                    if let Some(t) = self.tab_by_path(&tab_path) {
                        if let Some(dv) = t.diff.as_mut().filter(|d| d.token == token) {
                            dv.parsed = Some(parsed);
                        }
                    }
                }
                JobDone::Images { tab_path, token, pair } => {
                    self.pending = self.pending.saturating_sub(1);
                    let upload = |side: Option<Result<preview::Side, String>>, name: &str| {
                        side.map(|r| {
                            r.map(|s| ShownImage {
                                texture: ctx.load_texture(name, s.image, egui::TextureOptions::LINEAR),
                                width: s.width,
                                height: s.height,
                                bytes: s.bytes,
                            })
                        })
                    };
                    let before = upload(pair.before, "preview-before");
                    let after = upload(pair.after, "preview-after");
                    if let Some(t) = self.tab_by_path(&tab_path) {
                        if let Some(dv) = t.diff.as_mut().filter(|d| d.token == token) {
                            dv.images = Some(ImageState::Ready { before, after });
                        }
                    }
                }
                JobDone::FileLog { tab_path, path, result } => {
                    self.pending = self.pending.saturating_sub(1);
                    let first = result.as_ref().ok().filter(|v| !v.is_empty()).map(|_| 0usize);
                    if let Some(t) = self.tab_by_path(&tab_path) {
                        if let Some(fv) = t.file_view.as_mut().filter(|f| f.path == path) {
                            fv.history = Some(result);
                        }
                    }
                    // Show the newest change of the file right away.
                    if let Some(i) = first {
                        let active = self.tabs.get(self.active).and_then(|t| t.path.clone()) == Some(tab_path);
                        if active {
                            self.handle_one(ctx, Action::PickHistory(i));
                        }
                    }
                }
                JobDone::Blame { tab_path, path, result } => {
                    self.pending = self.pending.saturating_sub(1);
                    if let Some(t) = self.tab_by_path(&tab_path) {
                        if let Some(fv) = t.file_view.as_mut().filter(|f| f.path == path) {
                            fv.blame = Some(result);
                        }
                    }
                }
                JobDone::Detail { tab_path, detail } => {
                    self.pending = self.pending.saturating_sub(1);
                    if let Some(t) = self.tab_by_path(&tab_path) {
                        if t.selection == Selection::Commit(detail.sha.clone()) {
                            t.detail = Some(detail);
                        }
                    }
                }
                JobDone::Prs { tab_path, result } => {
                    self.busy.retain(|b| b != "Pull requests");
                    if let Some(t) = self.tab_by_path(&tab_path) {
                        t.prs = match result {
                            Ok((repo, prs)) => PrState::Ready(repo, prs),
                            Err(e) => PrState::Unavailable(e),
                        };
                    }
                }
                JobDone::PrCreated { tab_path, result } => {
                    self.busy.retain(|b| b != "Creating pull request");
                    match result {
                        Ok(url) => {
                            self.toast(ctx, "Pull request created; it is open in your browser.", ToastKind::Success);
                            ctx.open_url(egui::OpenUrl::new_tab(url));
                            self.spawn_prs(ctx, tab_path, false);
                        }
                        Err(e) => self.toast(ctx, format!("Could not create the pull request: {e}"), ToastKind::Error),
                    }
                }
                JobDone::Cloned { dest, result } => {
                    self.busy.retain(|b| b != "Clone");
                    match result {
                        Ok(_) => {
                            self.toast(ctx, "Repository cloned", ToastKind::Success);
                            if let Err(e) = self.open_repo(&dest, true) {
                                self.toast(ctx, e, ToastKind::Error);
                            }
                        }
                        Err(e) => {
                            let mut text = format!("Clone failed: {e}");
                            if access::looks_like_auth_error(&e) {
                                text.push_str("\n\nPrivate repository? Open Access (key button, top right) to sign in, then try again.");
                            }
                            self.toast(ctx, text, ToastKind::Error);
                        }
                    }
                }
                JobDone::Poll { tab_path, status, fingerprint } => {
                    let pending = self.pending;
                    let Some(t) = self.tab_by_path(&tab_path) else { continue };
                    t.polling = false;
                    let changed_refs = !t.fingerprint.is_empty() && t.fingerprint != fingerprint;
                    t.fingerprint = fingerprint;
                    if changed_refs {
                        self.send(Task::Reload { repo: tab_path });
                    } else if let Some(st) = status {
                        // A poll that started before a queued stage/commit would show stale files.
                        if pending == 0 {
                            t.snap.status = st;
                        }
                    }
                }
            }
        }

        let now = ctx.input(|i| i.time);
        let idle = self.busy.is_empty() && self.pending == 0;
        let tx = self.jobs_tx.clone();
        if let Some(tab) = self.tab() {
            if let Some(path) = tab.path.clone() {
                if !tab.polling && !tab.loading && idle && now - tab.last_poll > 3.0 {
                    tab.polling = true;
                    tab.last_poll = now;
                    let ctx2 = ctx.clone();
                    std::thread::spawn(move || {
                        let status = git::load_status(&path).ok();
                        let fingerprint = [
                            git::run(&path, &["for-each-ref", "--format=%(refname)%(objectname)"]).unwrap_or_default(),
                            git::run(&path, &["rev-parse", "-q", "--verify", "HEAD"]).unwrap_or_default(),
                            git::run(&path, &["symbolic-ref", "-q", "HEAD"]).unwrap_or_default(),
                            git::run(&path, &["stash", "list", "--format=%H"]).unwrap_or_default(),
                        ]
                        .concat();
                        let _ = tx.send(JobDone::Poll { tab_path: path, status, fingerprint });
                        ctx2.request_repaint();
                    });
                }
            }
        }
        // Keep polling even when the user is idle.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    fn apply_snapshot(&mut self, ctx: &egui::Context, path: &Path, snap: Snapshot, layout: GraphLayout) {
        let mut follow_up = Vec::new();
        let Some(t) = self.tab_by_path(path) else { return };
        t.set_snapshot(snap, layout);
        let load_prs = matches!(t.prs, PrState::NotLoaded) && !t.snap.remotes.is_empty();
        if load_prs {
            t.prs = PrState::Loading;
        }
        if std::mem::take(&mut t.select_head_after_reload) {
            if t.has_wip_row() {
                t.selection = Selection::Wip;
            } else if let Some(h) = t.snap.head_sha.clone() {
                follow_up.push(Action::Select(Selection::Commit(h)));
            }
        }
        // The open diff may be stale after staging or editing: ask for it again, or close it
        // when the file left that list.
        let open = t.diff.as_ref().map(|dv| (dv.source.clone(), dv.file.path.clone()));
        if let Some((source, file_path)) = open {
            let still_there = match source {
                DiffSource::Unstaged => t.snap.status.unstaged.iter().find(|f| f.path == file_path).cloned(),
                DiffSource::Staged => t.snap.status.staged.iter().find(|f| f.path == file_path).cloned(),
                DiffSource::Commit { .. } => None,
            };
            match (source, still_there) {
                (DiffSource::Commit { .. }, _) => {}
                (source, Some(file)) => follow_up.push(Action::OpenDiff(source, file)),
                (_, None) => t.diff = None,
            }
        }
        if let Selection::Commit(sha) = t.selection.clone() {
            if t.detail.is_none() {
                follow_up.push(Action::Select(Selection::Commit(sha)));
            }
        }
        if load_prs {
            self.spawn_prs(ctx, path.to_path_buf(), false);
        }
        let was_active = self.tabs.get(self.active).and_then(|t| t.path.as_deref()) == Some(path) && !self.show_home;
        if was_active {
            // Re-requesting a diff keeps the old one on screen until the new one arrives.
            for a in follow_up {
                match a {
                    Action::OpenDiff(source, file) => self.request_diff(ctx, source, file, true),
                    Action::Select(sel) => self.select(sel),
                    _ => {}
                }
            }
        }
    }

    pub fn handle(&mut self, ctx: &egui::Context) {
        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            self.handle_one(ctx, a);
        }
    }

    fn handle_one(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Select(sel) => {
                if let Some(t) = self.tab() {
                    t.diff = None;
                }
                self.select(sel);
            }
            Action::SelectRef(target) => {
                if let Some(tab) = self.tab() {
                    if let Some(i) = tab.commit_index(&target) {
                        tab.scroll_to = Some(i + usize::from(tab.has_wip_row()));
                        tab.diff = None;
                        self.select(Selection::Commit(target));
                    }
                }
            }
            Action::OpenDiff(source, file) => self.request_diff(ctx, source, file, false),
            Action::CloseDiff => {
                if let Some(t) = self.tab() {
                    t.diff = None;
                }
            }
            Action::Git { label, args, background } => {
                if background {
                    self.git_background(ctx, label, args);
                } else {
                    let ok = (!label.is_empty()).then_some(label);
                    self.queue_git(ctx, ok, args, None, OpKind::Plain);
                }
            }
            Action::Stage(f) => {
                if f.state == FileState::Deleted {
                    self.queue(ctx, "", &["rm", "--cached", "-q", "--", &f.path]);
                } else {
                    self.queue(ctx, "", &["add", "--", &f.path]);
                }
            }
            Action::Unstage(f) => {
                let has_head = self.tab().is_some_and(|t| t.snap.head_sha.is_some());
                let mut args: Vec<String> = if has_head {
                    vec!["restore".into(), "--staged".into(), "--".into()]
                } else {
                    vec!["rm".into(), "--cached".into(), "-q".into(), "--".into()]
                };
                args.push(f.path.clone());
                if let Some(o) = &f.old_path {
                    args.push(o.clone());
                }
                self.queue_git(ctx, None, args, None, OpKind::Plain);
            }
            Action::StageAll => self.queue(ctx, "", &["add", "-A"]),
            Action::UnstageAll => {
                let has_head = self.tab().is_some_and(|t| t.snap.head_sha.is_some());
                if has_head {
                    self.queue(ctx, "", &["reset", "-q"]);
                } else {
                    self.queue(ctx, "", &["rm", "-r", "--cached", "-q", "."]);
                }
            }
            Action::Hunk { index, mode } => self.apply_hunk(ctx, index, mode),
            Action::Commit => self.commit(ctx),
            Action::Dialog(d) => self.dialog = Some(d),
            Action::CheckoutLocal(name) => self.queue(ctx, &format!("Switched to {name}"), &["switch", &name]),
            Action::CheckoutRemote(remote_name) => {
                // origin/feature/x -> feature/x, tracking the remote branch.
                let local = remote_name.split_once('/').map(|(_, b)| b.to_string()).unwrap_or(remote_name.clone());
                let exists = self.tab().is_some_and(|t| t.snap.refs.iter().any(|r| r.kind == git::RefKind::Local && r.name == local));
                if exists {
                    self.queue(ctx, &format!("Switched to {local}"), &["switch", &local]);
                } else {
                    self.queue(ctx, &format!("Created {local} tracking {remote_name}"), &["switch", "-c", &local, "--track", &remote_name]);
                }
            }
            Action::Pull(mode) => {
                let flag = match mode {
                    PullMode::FastForwardIfPossible => "--ff",
                    PullMode::FastForwardOnly => "--ff-only",
                    PullMode::Rebase => "--rebase",
                };
                self.git_background(ctx, "Pull".into(), vec!["pull".into(), flag.into(), "--no-edit".into()]);
            }
            Action::Push { force } => self.push(ctx, force),
            Action::Fetch => self.git_background(ctx, "Fetch".into(), vec!["fetch".into(), "--all".into(), "--prune".into()]),
            Action::UndoLastCommit => {
                let redo = self.tab().and_then(|t| {
                    let head = t.snap.head_sha.clone()?;
                    let parent = t.commit(&head)?.parents.first()?.clone();
                    Some((head, parent))
                });
                if let Some(t) = self.tab() {
                    t.redo = redo;
                }
                self.queue(ctx, "Last commit undone; its changes are staged again (Redo brings it back)", &["reset", "--soft", "HEAD~1"]);
            }
            Action::Redo => {
                let Some((sha, _)) = self.tab().and_then(|t| t.redo.take()) else { return };
                self.queue(ctx, "Commit restored", &["reset", "--soft", &sha]);
            }
            Action::ToggleFolder(list, path) => {
                if let Some(t) = self.tab() {
                    let set = &mut t.collapsed[list as usize];
                    if !set.remove(&path) {
                        set.insert(path);
                    }
                }
            }
            Action::StagePath(dir) => self.queue(ctx, "", &["add", "-A", "--", &dir]),
            Action::UnstagePath(dir) => {
                let has_head = self.tab().is_some_and(|t| t.snap.head_sha.is_some());
                if has_head {
                    self.queue(ctx, "", &["restore", "--staged", "--", &dir]);
                } else {
                    self.queue(ctx, "", &["rm", "-r", "--cached", "-q", "--", &dir]);
                }
            }
            Action::SetRender(on) => {
                if let Some(dv) = self.tab().and_then(|t| t.diff.as_mut()) {
                    dv.render = on;
                }
            }
            Action::SetTreeView(on) => self.settings.tree_view = on,
            Action::FileHistory(path) | Action::FileBlame(path) if self.tab().is_some_and(|t| t.is_sample()) => {
                // The sample has no files on disk: show a made-up history.
                let Some(tab) = self.tab() else { return };
                let entries: Vec<git::HistoryEntry> = tab
                    .snap
                    .commits
                    .iter()
                    .step_by(4)
                    .map(|c| git::HistoryEntry { commit: c.clone(), path: path.clone() })
                    .collect();
                tab.diff = None;
                tab.file_view = Some(FileView { path, kind: FileViewKind::History, history: Some(Ok(entries)), selected: None, blame: None });
                self.handle_one(ctx, Action::PickHistory(0));
            }
            Action::FileHistory(path) => {
                let Some(repo) = self.active_path() else { return };
                if let Some(tab) = self.tab() {
                    tab.diff = None;
                    tab.file_view = Some(FileView { path: path.clone(), kind: FileViewKind::History, history: None, selected: None, blame: None });
                }
                self.send(Task::FileLog { repo, path });
            }
            Action::FileBlame(path) => {
                let Some(repo) = self.active_path() else { return };
                if let Some(tab) = self.tab() {
                    tab.diff = None;
                    tab.file_view = Some(FileView { path: path.clone(), kind: FileViewKind::Blame, history: None, selected: None, blame: None });
                }
                self.send(Task::Blame { repo, path });
            }
            Action::PickHistory(i) => {
                let Some(tab) = self.tab() else { return };
                let Some(fv) = tab.file_view.as_mut() else { return };
                let Some(Ok(entries)) = &fv.history else { return };
                let Some(e) = entries.get(i).cloned() else { return };
                fv.selected = Some(i);
                let source = DiffSource::Commit { sha: e.commit.sha.clone(), parent: e.commit.parents.first().cloned() };
                let file = FileChange { path: e.path, old_path: None, state: FileState::Modified };
                self.request_diff(ctx, source, file, false);
            }
            Action::CloseFileView => {
                if let Some(t) = self.tab() {
                    t.file_view = None;
                    t.diff = None;
                }
            }
            Action::StageMany(files) => {
                for chunk in files.chunks(100) {
                    let mut args: Vec<String> = vec!["add".into(), "-A".into(), "--".into()];
                    args.extend(chunk.iter().map(|f| f.path.clone()));
                    self.queue_git(ctx, None, args, None, OpKind::Plain);
                }
                if let Some(t) = self.tab() {
                    t.picked.clear();
                }
            }
            Action::UnstageMany(files) => {
                let has_head = self.tab().is_some_and(|t| t.snap.head_sha.is_some());
                for chunk in files.chunks(100) {
                    let mut args: Vec<String> = if has_head {
                        vec!["restore".into(), "--staged".into(), "--".into()]
                    } else {
                        vec!["rm".into(), "--cached".into(), "-q".into(), "--".into()]
                    };
                    for f in chunk {
                        args.push(f.path.clone());
                        if let Some(o) = &f.old_path {
                            args.push(o.clone());
                        }
                    }
                    self.queue_git(ctx, None, args, None, OpKind::Plain);
                }
                if let Some(t) = self.tab() {
                    t.picked.clear();
                }
            }
            Action::DiscardMany(files) => {
                let Some(repo) = self.active_path() else {
                    self.sample_refusal(ctx);
                    return;
                };
                let (untracked, tracked): (Vec<_>, Vec<_>) = files.into_iter().partition(|f| f.state == FileState::Untracked);
                for f in untracked {
                    self.send(Task::Delete { repo: repo.clone(), rel: f.path });
                }
                for chunk in tracked.chunks(100) {
                    let mut args: Vec<String> = vec!["restore".into(), "--worktree".into(), "--".into()];
                    args.extend(chunk.iter().map(|f| f.path.clone()));
                    self.send(Task::Run { repo: repo.clone(), label: "git restore".into(), ok_text: None, args, input: None, kind: OpKind::Plain });
                }
                self.send(Task::Reload { repo });
                if let Some(t) = self.tab() {
                    t.picked.clear();
                    t.diff = None;
                }
            }
            Action::SetDiffMode(mode) => {
                self.settings.diff_mode = mode;
                let reopen = self.tab().and_then(|t| t.diff.as_ref()).filter(|d| d.whole != (mode != DiffMode::Hunk)).map(|d| (d.source.clone(), d.file.clone()));
                if let Some((source, file)) = reopen {
                    self.request_diff(ctx, source, file, true);
                }
            }
            Action::Jump(next) => {
                let mode = self.settings.diff_mode;
                let Some(dv) = self.tab().and_then(|t| t.diff.as_mut()) else { return };
                let Some(parsed) = &dv.parsed else { return };
                let starts = parsed.starts(mode);
                // A little slack so the change just jumped to counts as "current".
                let top = dv.top_row + 2;
                let target = if next {
                    starts.iter().copied().find(|&r| r > top)
                } else {
                    starts.iter().copied().rev().find(|&r| r + 1 < top)
                };
                if let Some(r) = target {
                    dv.jump = Some(r.saturating_sub(2));
                }
            }
            Action::ResolveConflict(path, ours) => {
                let side = if ours { "--ours" } else { "--theirs" };
                let Some(repo) = self.active_path() else { return };
                self.send(Task::Run { repo: repo.clone(), label: "git checkout".into(), ok_text: None, args: vec!["checkout".into(), side.into(), "--".into(), path.clone()], input: None, kind: OpKind::Plain });
                let text = format!("{path}: kept {} version", if ours { "your" } else { "their" });
                self.send(Task::Run { repo: repo.clone(), label: "git add".into(), ok_text: Some(text), args: vec!["add".into(), "--".into(), path], input: None, kind: OpKind::Plain });
                self.send(Task::Reload { repo });
            }
            Action::LoadPullRequests { interactive } => {
                let Some(p) = self.active_path() else { return };
                if interactive {
                    self.busy.push("Pull requests".into());
                }
                if let Some(t) = self.tab() {
                    t.prs = PrState::Loading;
                }
                self.spawn_prs(ctx, p, interactive);
            }
            Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Action::CheckoutPullRequest(i) => {
                let cmds = match self.tab().map(|t| &t.prs) {
                    Some(PrState::Ready(repo, prs)) => prs.get(i).map(|pr| (forge::checkout_commands(repo, pr), pr.number)),
                    _ => None,
                };
                let Some((cmds, number)) = cmds else { return };
                let n = cmds.len();
                for (k, args) in cmds.into_iter().enumerate() {
                    let ok = (k + 1 == n).then(|| format!("Checked out pull request #{number}"));
                    self.queue_git(ctx, ok, args, None, OpKind::Plain);
                }
            }
            Action::OpenPullRequestForm(from_branch) => {
                let Some(tab) = self.tab() else { return };
                let remotes = match (&tab.path, &tab.prs) {
                    (Some(p), _) => forge::remotes_of(p),
                    (None, PrState::Ready(r, _)) => vec![r.clone()],
                    _ => Vec::new(),
                };
                if remotes.is_empty() {
                    self.toast(ctx, "No remote of this repository is on GitHub, GitLab, Bitbucket or Azure DevOps.", ToastKind::Info);
                    return;
                }
                let branches: Vec<(String, String)> = tab
                    .snap
                    .refs
                    .iter()
                    .filter(|r| r.kind == git::RefKind::Remote)
                    .filter_map(|r| r.name.split_once('/').map(|(rem, b)| (rem.to_string(), b.to_string())))
                    .collect();
                let current = tab.snap.head_branch.clone();
                let title = tab.snap.head_sha.as_ref().and_then(|h| tab.commit(h)).map(|c| c.summary.clone()).unwrap_or_default();
                let forge_kind = remotes[0].forge;
                let mut form = PrForm {
                    remotes,
                    forge: forge_kind,
                    from: 0,
                    from_branch: String::new(),
                    to: 0,
                    to_branch: String::new(),
                    title,
                    body: String::new(),
                    draft: false,
                    branches,
                    current,
                };
                form.select_forge(forge_kind);
                if let Some(b) = from_branch {
                    form.from_branch = b;
                    form.to_branch = form.default_target();
                }
                self.dialog = Some(Dialog::NewPullRequest(form));
            }
            Action::CreatePullRequest(form) => {
                let Some(path) = self.active_path() else {
                    self.sample_refusal(ctx);
                    return;
                };
                let (Some(from), Some(to)) = (form.remotes.get(form.from).cloned(), form.remotes.get(form.to).cloned()) else { return };
                self.busy.push("Creating pull request".into());
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let result = forge::create(&from, &form.from_branch, &to, &form.to_branch, form.title.trim(), form.body.trim(), form.draft);
                    let _ = tx.send(JobDone::PrCreated { tab_path: path, result });
                    ctx2.request_repaint();
                });
            }
            Action::CollapseAll(list, collapse) => {
                let tree = self.settings.tree_view;
                let Some(t) = self.tab() else { return };
                let files: Vec<String> = match list {
                    ListId::Unstaged => t.snap.status.unstaged.iter().map(|f| f.path.clone()).collect(),
                    ListId::Staged => t.snap.status.staged.iter().map(|f| f.path.clone()).collect(),
                    ListId::Commit => t.detail.as_ref().map(|d| d.files.iter().map(|f| f.path.clone()).collect()).unwrap_or_default(),
                };
                let set = &mut t.collapsed[list as usize];
                set.clear();
                if collapse && tree {
                    for f in files {
                        let mut end = 0;
                        while let Some(i) = f[end..].find('/') {
                            end += i;
                            set.insert(f[..end].to_string());
                            end += 1;
                        }
                    }
                }
            }
            Action::OpenSubmodule(rel) => {
                if let Some(p) = self.active_path() {
                    if let Err(e) = self.open_repo(&p.join(&rel), true) {
                        self.toast(ctx, format!("{e}\nThe submodule may not be checked out yet (git submodule update --init)."), ToastKind::Error);
                    }
                }
            }
            Action::OpenRepo(p) => {
                if let Err(e) = self.open_repo(&p, true) {
                    self.toast(ctx, e, ToastKind::Error);
                }
            }
            Action::OpenSample => self.open_sample(),
            Action::InitRepo(p) => {
                if let Err(e) = std::fs::create_dir_all(&p) {
                    self.toast(ctx, format!("Could not create the folder: {e}"), ToastKind::Error);
                    return;
                }
                match git::run(&p, &["init"]) {
                    Ok(_) => match self.open_repo(&p, true) {
                        Ok(()) => self.toast(ctx, "New repository created", ToastKind::Success),
                        Err(e) => self.toast(ctx, e, ToastKind::Error),
                    },
                    Err(e) => self.toast(ctx, e.0, ToastKind::Error),
                }
            }
            Action::Clone { url, dest } => {
                let parent = dest.parent().map(Path::to_path_buf).unwrap_or_else(dirs_home);
                if let Err(e) = std::fs::create_dir_all(&parent) {
                    self.toast(ctx, format!("Could not create the folder: {e}"), ToastKind::Error);
                    return;
                }
                self.busy.push("Clone".into());
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let d = dest.display().to_string();
                    let result = git::run(&parent, &["clone", &url, &d]).map_err(|e| e.0);
                    let _ = tx.send(JobDone::Cloned { dest, result });
                    ctx2.request_repaint();
                });
            }
            Action::CloseTab(i) => {
                if i < self.tabs.len() {
                    self.tabs.remove(i);
                    if self.active >= self.tabs.len() {
                        self.active = self.tabs.len().saturating_sub(1);
                    }
                    if self.tabs.is_empty() {
                        self.show_home = true;
                    }
                    self.sync_open_tabs();
                }
            }
            Action::SwitchTab(i) => {
                self.active = i.min(self.tabs.len().saturating_sub(1));
                self.show_home = false;
                self.reload_active();
            }
            Action::ShowHome => self.show_home = true,
            Action::Refresh => {
                self.reload_active();
                if let Some(p) = self.active_path() {
                    self.spawn_prs(ctx, p, false);
                }
            }
            Action::Copy(text) => {
                ctx.copy_text(text);
                self.toast(ctx, "Copied to clipboard", ToastKind::Info);
            }
            Action::OpenTerminal => {
                if let Some(p) = self.active_path() {
                    open_terminal(&p);
                }
            }
            Action::OpenInExplorer(rel) => {
                if let Some(p) = self.active_path() {
                    let target = rel.map(|r| p.join(r)).unwrap_or(p);
                    reveal(&target);
                }
            }
            Action::OpenFile(rel) => {
                if let Some(p) = self.active_path() {
                    open_with_default(&p.join(rel));
                }
            }
            Action::SetTheme(t) => self.settings.theme = t,
            Action::SaveToken { host, user, token } => {
                let input = access::credential_input(&host, &user, &token);
                self.send(Task::Run {
                    repo: dirs_home(),
                    label: "Saving the sign-in".into(),
                    ok_text: Some(format!("Signed in to {host} as {user}. Private repositories there now work.")),
                    args: vec!["credential".into(), "approve".into()],
                    input: Some(input),
                    kind: OpKind::Plain,
                });
            }
            Action::TestAccess(url) => {
                let label = format!("Access test for {url}");
                self.busy.push(label.clone());
                let tx = self.jobs_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let home = dirs_home();
                    let result = git::run(&home, &["ls-remote", "--heads", &url]).map_err(|e| e.0);
                    let ok_text = Some(format!("Access works: you can clone {url}"));
                    let _ = tx.send(JobDone::Op { tab_path: home, label, ok_text, result, kind: OpKind::Plain, from_worker: false });
                    ctx2.request_repaint();
                });
            }
            Action::CreateSshKey => match access::create_ssh_key() {
                Ok(path) => self.toast(ctx, format!("SSH key created ({path}). Copy it and add it to your account."), ToastKind::Success),
                Err(e) => self.toast(ctx, e, ToastKind::Error),
            },
            Action::UseCredentialManager => match git::run(&dirs_home(), &["config", "--global", "credential.helper", "manager"]) {
                Ok(_) => self.toast(ctx, "Sign-ins will now be remembered by Git Credential Manager.", ToastKind::Success),
                Err(e) => self.toast(ctx, e.0, ToastKind::Error),
            },
        }
    }

    /// Loads the open pull requests on a network thread.
    fn spawn_prs(&mut self, ctx: &egui::Context, path: PathBuf, interactive: bool) {
        let tx = self.jobs_tx.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let result = forge::list(&path, interactive);
            let _ = tx.send(JobDone::Prs { tab_path: path, result });
            ctx2.request_repaint();
        });
    }

    fn request_diff(&mut self, ctx: &egui::Context, source: DiffSource, file: FileChange, keep_old: bool) {
        self.next_token += 1;
        let token = self.next_token;
        let is_picture = preview::kind_of(&file.path).is_some();
        let whole = self.settings.diff_mode != DiffMode::Hunk;
        let Some(tab) = self.tab() else { return };
        // Re-opening the same file keeps the code/render choice the person made.
        let render = tab.diff.as_ref().filter(|d| d.file.path == file.path).map_or(is_picture, |d| d.render);
        let Some(repo) = tab.path.clone() else {
            let parsed = ParsedDiff::parse(&demo::diff_for(&file.path));
            let images = is_picture.then(|| {
                let after = preview::decode(demo::SAMPLE_SVG.as_bytes(), preview::Kind::Svg).map(|s| ShownImage {
                    texture: ctx.load_texture("preview-after", s.image, egui::TextureOptions::LINEAR),
                    width: s.width,
                    height: s.height,
                    bytes: s.bytes,
                });
                ImageState::Ready { before: None, after: Some(after) }
            });
            tab.diff = Some(DiffView { source, file, parsed: Some(parsed), token, render, images, whole, jump: None, top_row: 0 });
            return;
        };
        let old = if keep_old { tab.diff.take().and_then(|d| d.parsed) } else { None };
        let images = is_picture.then_some(ImageState::Loading);
        tab.diff = Some(DiffView { source: source.clone(), file: file.clone(), parsed: old, token, render, images, whole, jump: None, top_row: 0 });
        if is_picture {
            self.send(Task::Images { repo: repo.clone(), source: source.clone(), file: file.clone(), token });
        }
        self.send(Task::Diff { repo, source, file, token, whole });
    }

    fn select(&mut self, sel: Selection) {
        let Some(tab) = self.tab() else { return };
        tab.selection = sel.clone();
        let Selection::Commit(sha) = sel else {
            tab.detail = None;
            return;
        };
        let Some(commit) = tab.commit(&sha).cloned() else { return };
        if tab.detail.as_ref().is_some_and(|d| d.sha == sha) {
            return;
        }
        tab.detail = None;
        match tab.path.clone() {
            Some(repo) => self.send(Task::Detail { repo, commit }),
            None => {
                tab.detail = Some(CommitDetail {
                    message: format!(
                        "{}\n\nA sample commit in the built-in demo project. Open one of your own repositories to see real history.",
                        commit.summary
                    ),
                    committer: format!("{} <{}>", commit.author, commit.email),
                    files: demo::files_for(&sha),
                    sha,
                });
            }
        }
    }

    fn apply_hunk(&mut self, ctx: &egui::Context, index: usize, mode: HunkMode) {
        let Some(tab) = self.tab() else { return };
        if tab.is_sample() {
            self.sample_refusal(ctx);
            return;
        }
        let Some(dv) = &tab.diff else { return };
        // A brand-new file can only be staged as a whole.
        if dv.file.state == FileState::Untracked && mode == HunkMode::Stage {
            let f = dv.file.clone();
            self.handle_one(ctx, Action::Stage(f));
            return;
        }
        let Some(patch) = dv.parsed.as_ref().and_then(|p| p.hunk_patch(index)) else { return };
        let args: &[&str] = match mode {
            HunkMode::Stage => &["apply", "--cached", "--whitespace=nowarn", "-"],
            HunkMode::Unstage => &["apply", "--cached", "-R", "--whitespace=nowarn", "-"],
            HunkMode::Discard => &["apply", "-R", "--whitespace=nowarn", "-"],
        };
        let args = args.iter().map(|s| s.to_string()).collect();
        self.queue_git(ctx, None, args, Some(patch), OpKind::Plain);
    }

    fn commit(&mut self, ctx: &egui::Context) {
        self.commit_inner(ctx, false);
    }

    /// `staging_first`: an `add -A` was just queued, so an empty staging area is fine.
    fn commit_inner(&mut self, ctx: &egui::Context, staging_first: bool) {
        let Some(tab) = self.tab() else { return };
        let summary = tab.summary.trim().to_string();
        let description = tab.description.trim().to_string();
        let amend = tab.amend;
        let nothing_staged = tab.snap.status.staged.is_empty();
        if summary.is_empty() && !amend {
            self.toast(ctx, "Write a short summary first.", ToastKind::Info);
            return;
        }
        if nothing_staged && !amend && !staging_first {
            self.toast(ctx, "Nothing is staged. Stage some files first.", ToastKind::Info);
            return;
        }
        let mut args: Vec<String> = vec!["commit".into()];
        if amend {
            args.push("--amend".into());
            if summary.is_empty() {
                args.push("--no-edit".into());
            }
        }
        if !summary.is_empty() {
            args.extend(["-m".into(), summary]);
        }
        if !description.is_empty() {
            args.extend(["-m".into(), description]);
        }
        let label = if amend { "Commit amended" } else { "Committed" };
        self.queue_git(ctx, Some(label.into()), args, None, OpKind::Commit);
    }

    fn push(&mut self, ctx: &egui::Context, force: bool) {
        let Some(tab) = self.tab() else { return };
        let Some(branch) = tab.snap.head_branch.clone() else {
            self.toast(ctx, "Switch to a branch before pushing (HEAD is detached).", ToastKind::Info);
            return;
        };
        let has_upstream = tab.snap.current_ref().is_some_and(|r| r.upstream.is_some());
        let remote = tab.snap.remotes.first().cloned();
        let mut args = vec!["push".to_string()];
        if force {
            args.push("--force-with-lease".into());
        }
        if !has_upstream {
            let Some(remote) = remote else {
                self.toast(ctx, "This repository has no remote to push to.", ToastKind::Info);
                return;
            };
            args.extend(["-u".into(), remote, branch]);
        }
        self.git_background(ctx, "Push".into(), args);
    }

    pub fn save_settings(&mut self, storage: &mut dyn eframe::Storage) {
        self.sync_open_tabs();
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }
}

fn spawn_worker(out: Sender<JobDone>, ctx: egui::Context) -> Sender<Task> {
    let (tx, rx) = mpsc::channel::<Task>();
    std::thread::Builder::new()
        .name("git-worker".into())
        .spawn(move || {
            for task in rx {
                let done = run_task(task);
                if out.send(done).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        })
        .expect("could not start the git worker thread");
    tx
}

fn run_task(task: Task) -> JobDone {
    match task {
        Task::Run { repo, label, ok_text, args, input, kind } => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = match input {
                Some(i) => git::run_with_input(&repo, &refs, &i),
                None => git::run(&repo, &refs),
            }
            .map_err(|e| e.0);
            JobDone::Op { tab_path: repo, label, ok_text, result, kind, from_worker: true }
        }
        Task::Delete { repo, rel } => {
            let result = std::fs::remove_file(repo.join(&rel)).map(|_| String::new()).map_err(|e| format!("{rel}: {e}"));
            JobDone::Op { tab_path: repo, label: "Delete".into(), ok_text: None, result, kind: OpKind::Plain, from_worker: true }
        }
        Task::Reload { repo } => {
            let result = git::load_snapshot(&repo)
                .map(|snap| {
                    let layout = graph::layout_for(&snap.commits, snap.head_sha.as_deref());
                    (snap, layout)
                })
                .map_err(|e| e.0);
            JobDone::Snapshot { tab_path: repo, result }
        }
        Task::Diff { repo, source, file, token, whole } => {
            let (text, truncated) = git::file_diff_ctx(&repo, &source, &file, whole);
            let mut parsed = ParsedDiff::parse(&text);
            parsed.truncated = truncated;
            JobDone::Diff { tab_path: repo, token, parsed }
        }
        Task::Images { repo, source, file, token } => {
            let pair = preview::load_pair(&repo, &source, &file);
            JobDone::Images { tab_path: repo, token, pair }
        }
        Task::FileLog { repo, path } => {
            let result = git::file_log(&repo, &path).map_err(|e| e.0);
            JobDone::FileLog { tab_path: repo, path, result }
        }
        Task::Blame { repo, path } => {
            let result = git::blame(&repo, &path).map_err(|e| e.0);
            JobDone::Blame { tab_path: repo, path, result }
        }
        Task::Detail { repo, commit } => {
            let detail = CommitDetail {
                message: git::full_message(&repo, &commit.sha),
                committer: git::committer_line(&repo, &commit.sha),
                files: git::commit_files(&repo, &commit).unwrap_or_default(),
                sha: commit.sha,
            };
            JobDone::Detail { tab_path: repo, detail }
        }
    }
}

pub fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn open_terminal(dir: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", "cmd", "/K"]).current_dir(dir).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("x-terminal-emulator").current_dir(dir).spawn();
    }
}

fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        if path.is_file() {
            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
        } else {
            let _ = std::process::Command::new("explorer").arg(path).spawn();
        }
    }
    #[cfg(not(windows))]
    {
        let dir = if path.is_file() { path.parent().unwrap_or(path) } else { path };
        open_with_default(dir);
    }
}

fn open_with_default(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

/// Marks the screenshot as taken once the PNG is on disk.
pub fn save_screenshot(image: &egui::ColorImage, path: &Path, done: &AtomicBool) {
    let [w, h] = image.size;
    let mut buf = Vec::with_capacity(w * h * 4);
    for px in &image.pixels {
        buf.extend_from_slice(&px.to_array());
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match image::RgbaImage::from_raw(w as u32, h as u32, buf) {
        Some(img) => {
            if let Err(e) = img.save(path) {
                eprintln!("Could not save screenshot: {e}");
            }
        }
        None => eprintln!("Screenshot buffer had an unexpected size"),
    }
    done.store(true, Ordering::SeqCst);
}
