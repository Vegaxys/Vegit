//! Thin layer over the `git` command line.
//!
//! The app shells out to the user's own `git` instead of linking a git library so that
//! credentials, hooks, config and SSH keys behave exactly as they do in a terminal.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How many commits the graph loads; enough for daily work while keeping refreshes fast.
pub const MAX_COMMITS: usize = 3000;

#[derive(Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub summary: String,
    /// First line of the description, shown after the summary in a quieter colour.
    pub body: String,
    /// `stash@{n}` when this row is a stash rather than a real commit of a branch.
    pub stash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug)]
pub struct RefInfo {
    /// Short display name, e.g. `main`, `origin/main`, `v1.0`.
    pub name: String,
    pub kind: RefKind,
    pub target: String,
    /// Upstream short name for local branches (`origin/main`).
    pub upstream: Option<String>,
    /// Commits ahead/behind upstream for local branches.
    pub ahead: u32,
    pub behind: u32,
    /// The upstream branch was deleted on the server (seen after a fetch with prune).
    pub gone: bool,
}

#[derive(Clone, Debug)]
pub struct Stash {
    /// `stash@{0}` style reference used by `git stash apply/pop/drop`.
    pub reference: String,
    pub message: String,
    pub sha: String,
    /// The commit the stash was made on.
    pub base: Option<String>,
    pub time: i64,
    pub author: String,
    pub email: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
    TypeChange,
}

impl FileState {
    fn from_code(c: char) -> Self {
        match c {
            'A' => Self::Added,
            'D' => Self::Deleted,
            'R' | 'C' => Self::Renamed,
            '?' => Self::Untracked,
            'U' => Self::Conflicted,
            'T' => Self::TypeChange,
            _ => Self::Modified,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub state: FileState,
}

#[derive(Clone, Debug, Default)]
pub struct WorkStatus {
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
}

impl WorkStatus {
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty()
    }
}

/// An operation in progress that the user must finish or abort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingOp {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl PendingOp {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}

/// Everything the window shows about one repository, loaded in one go.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub commits: Vec<Commit>,
    pub refs: Vec<RefInfo>,
    pub stashes: Vec<Stash>,
    pub head_sha: Option<String>,
    /// Current branch, `None` when HEAD is detached or the repo has no commit yet.
    pub head_branch: Option<String>,
    pub status: WorkStatus,
    pub pending: Option<PendingOp>,
    pub remotes: Vec<String>,
    /// Submodule folders, relative to the repository root.
    pub submodules: Vec<String>,
    /// (remote name, fetch URL), for links to the hosting service.
    pub remote_urls: Vec<(String, String)>,
    /// Who commits here (user.name / user.email, global and local).
    pub identity: Identity,
    /// What happened lately (the reflog), in plain sentences, newest first.
    pub activity: Vec<crate::safety::Activity>,
}

impl Snapshot {
    /// The summary of a loaded commit (used by the tests).
    #[cfg(test)]
    pub fn commit_summary(&self, sha: &str) -> Option<&str> {
        self.commits.iter().find(|c| c.sha == sha).map(|c| c.summary.as_str())
    }

    /// Refs grouped by the commit they point at, for the labels in the graph.
    pub fn refs_by_sha(&self) -> HashMap<String, Vec<usize>> {
        let mut map: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in self.refs.iter().enumerate() {
            map.entry(r.target.clone()).or_default().push(i);
        }
        map
    }

    pub fn current_ref(&self) -> Option<&RefInfo> {
        let name = self.head_branch.as_ref()?;
        self.refs.iter().find(|r| r.kind == RefKind::Local && &r.name == name)
    }
}

#[derive(Debug)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type GitResult<T> = Result<T, GitError>;

fn base_command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir);
    // Never block on a terminal prompt the user can't see; credential helpers with
    // their own window (Git Credential Manager) still work.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_EDITOR", "true");
    cmd.env("LC_ALL", "C");
    // Background status polls must never grab index.lock and make a stage/commit fail.
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs git and returns stdout; a non-zero exit becomes an error holding stderr.
pub fn run(dir: &Path, args: &[&str]) -> GitResult<String> {
    let out = base_command(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| GitError(format!("Could not start git ({e}). Is git installed and on PATH?")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let mut msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if msg.is_empty() {
            msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        if msg.is_empty() {
            msg = format!("git {} failed", args.first().unwrap_or(&""));
        }
        Err(GitError(msg))
    }
}

/// Like [`run`], with extra environment variables (a temporary index, an author for
/// internal commits, later a sequence editor for interactive rebase).
pub fn run_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> GitResult<String> {
    let mut cmd = base_command(dir);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| GitError(format!("Could not start git ({e}). Is git installed and on PATH?")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(GitError(if msg.is_empty() { format!("git {} failed", args.first().unwrap_or(&"")) } else { msg }))
    }
}

/// Biggest diff kept in memory. Unity scenes and prefabs can produce hundreds of
/// megabytes of YAML diff; past this point the rest is dropped and the view says so.
pub const MAX_DIFF_BYTES: usize = 24 * 1024 * 1024;

/// Like [`run`] but ignores the exit code (git exits 1 on success for `diff --no-index`)
/// and stops reading after [`MAX_DIFF_BYTES`]. Returns the text and whether it was cut.
fn run_lenient(dir: &Path, args: &[&str]) -> (String, bool) {
    use std::io::Read;
    let Ok(mut child) = base_command(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return (String::new(), false);
    };
    let mut buf = Vec::new();
    let mut truncated = false;
    if let Some(out) = child.stdout.take() {
        let _ = out.take(MAX_DIFF_BYTES as u64 + 1).read_to_end(&mut buf);
        if buf.len() > MAX_DIFF_BYTES {
            truncated = true;
            buf.truncate(MAX_DIFF_BYTES);
            // Cut at the last full line so the parser never sees half a line.
            if let Some(nl) = buf.iter().rposition(|&b| b == b'\n') {
                buf.truncate(nl + 1);
            }
            let _ = child.kill();
        }
    }
    let _ = child.wait();
    (String::from_utf8_lossy(&buf).into_owned(), truncated)
}

/// Raw bytes of a successful git command (file contents for image previews).
pub fn run_bytes(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = base_command(dir).args(args).stdin(Stdio::null()).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Runs git with `input` piped to stdin (used to apply single hunks).
pub fn run_with_input(dir: &Path, args: &[&str], input: &str) -> GitResult<String> {
    let mut child = base_command(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| GitError(format!("Could not start git ({e})")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| GitError(e.to_string()))?;
    }
    let out = child.wait_with_output().map_err(|e| GitError(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(GitError(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

/// Returns the top folder of the repository containing `path`.
pub fn repo_root(path: &Path) -> GitResult<PathBuf> {
    let out = run(path, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(out.trim()))
}

pub fn load_snapshot(dir: &Path) -> GitResult<Snapshot> {
    let mut snap = Snapshot::default();
    let git_dir = PathBuf::from(run(dir, &["rev-parse", "--absolute-git-dir"])?.trim());

    snap.head_sha = run(dir, &["rev-parse", "-q", "--verify", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    snap.head_branch = run(dir, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    snap.refs = load_refs(dir)?;
    snap.stashes = load_stashes(dir);
    snap.status = load_status(dir)?;
    snap.remotes = run(dir, &["remote"])
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    if snap.head_sha.is_some() {
        snap.commits = load_commits(dir)?;
        insert_stashes(&mut snap.commits, &snap.stashes);
    }
    snap.remote_urls = snap
        .remotes
        .iter()
        .map(|r| (r.clone(), run(dir, &["remote", "get-url", r]).unwrap_or_default().trim().to_string()))
        .collect();
    // Read .gitmodules directly: `git submodule status` can be slow on big projects.
    if dir.join(".gitmodules").exists() {
        snap.submodules = run(dir, &["config", "--file", ".gitmodules", "--get-regexp", r"\.path$"])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once(' ').map(|(_, p)| p.trim().to_string()))
            .collect();
    }

    snap.identity = identity(dir);
    snap.activity = crate::safety::activity(dir, 60);
    snap.pending = if git_dir.join("MERGE_HEAD").exists() {
        Some(PendingOp::Merge)
    } else if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        Some(PendingOp::Rebase)
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        Some(PendingOp::CherryPick)
    } else if git_dir.join("REVERT_HEAD").exists() {
        Some(PendingOp::Revert)
    } else {
        None
    };
    Ok(snap)
}

fn load_commits(dir: &Path) -> GitResult<Vec<Commit>> {
    let limit = format!("-n{MAX_COMMITS}");
    // Branches, remotes, tags and HEAD, but not refs/stash: stashes live in the sidebar.
    let out = run(
        dir,
        &[
            "log",
            "--branches",
            "--remotes",
            "--tags",
            "HEAD",
            "--date-order",
            &limit,
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%b%x1e",
        ],
    )?;
    let mut commits = Vec::new();
    for rec in out.split('\x1e') {
        let rec = rec.trim_start_matches(['\n', '\r']);
        if rec.is_empty() {
            continue;
        }
        let f: Vec<&str> = rec.split('\x1f').collect();
        if f.len() < 6 {
            continue;
        }
        commits.push(Commit {
            sha: f[0].to_string(),
            parents: f[1].split_whitespace().map(str::to_string).collect(),
            author: f[2].to_string(),
            email: f[3].to_string(),
            time: f[4].trim().parse().unwrap_or(0),
            summary: f[5].trim_end().to_string(),
            body: f.get(6).map(|b| first_line(b)).unwrap_or_default(),
            stash: None,
        });
    }
    Ok(commits)
}

fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    line.chars().take(140).collect()
}

/// One commit of a file's history, with the file's path at that commit (it can be renamed).
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub commit: Commit,
    pub path: String,
}

/// Commits that changed `path`, newest first, following renames.
pub fn file_log(dir: &Path, path: &str) -> GitResult<Vec<HistoryEntry>> {
    let limit = format!("-n{MAX_COMMITS}");
    let out = run(dir, &["log", &limit, "--follow", "--name-only", "--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s", "--", path])?;
    let mut entries = Vec::new();
    for rec in out.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let mut lines = rec.lines();
        let head = lines.next().unwrap_or_default();
        let f: Vec<&str> = head.split('\x1f').collect();
        if f.len() < 6 {
            continue;
        }
        let file = lines.map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or(path).to_string();
        entries.push(HistoryEntry {
            commit: Commit {
                sha: f[0].to_string(),
                parents: f[1].split_whitespace().map(str::to_string).collect(),
                author: f[2].to_string(),
                email: f[3].to_string(),
                time: f[4].trim().parse().unwrap_or(0),
                summary: f[5].trim_end().to_string(),
                body: String::new(),
                stash: None,
            },
            path: file,
        });
    }
    Ok(entries)
}

/// One line of `git blame`.
#[derive(Clone, Debug)]
pub struct BlameLine {
    pub sha: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
    pub text: String,
    /// First line of a run of lines from the same commit: only it shows the commit.
    pub starts_block: bool,
    /// Index of that run, used to shade every other run.
    pub block: usize,
}

/// Who last changed each line of `path` (in the working folder version).
pub fn blame(dir: &Path, path: &str) -> GitResult<Vec<BlameLine>> {
    let out = run(dir, &["blame", "--line-porcelain", "--", path])?;
    let mut lines = Vec::new();
    let (mut sha, mut author, mut time, mut summary) = (String::new(), String::new(), 0i64, String::new());
    let mut prev_sha = String::new();
    let mut block = 0usize;
    for l in out.lines() {
        if let Some(text) = l.strip_prefix('\t') {
            let starts_block = sha != prev_sha;
            if starts_block {
                block += 1;
            }
            prev_sha = sha.clone();
            lines.push(BlameLine {
                sha: sha.clone(),
                author: author.clone(),
                time,
                summary: summary.clone(),
                text: text.replace('\t', "    "),
                starts_block,
                block,
            });
        } else if let Some(a) = l.strip_prefix("author ") {
            author = a.to_string();
        } else if let Some(t) = l.strip_prefix("author-time ") {
            time = t.trim().parse().unwrap_or(0);
        } else if let Some(t) = l.strip_prefix("summary ") {
            summary = t.to_string();
        } else if l.len() >= 40 && l.as_bytes()[..40].iter().all(u8::is_ascii_hexdigit) {
            sha = l[..40].to_string();
        }
    }
    Ok(lines)
}

fn load_refs(dir: &Path) -> GitResult<Vec<RefInfo>> {
    let out = run(
        dir,
        &[
            "for-each-ref",
            "--format=%(refname)%1f%(objectname)%1f%(*objectname)%1f%(upstream:short)%1f%(upstream:track,nobracket)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?;
    let mut refs = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\x1f').collect();
        if f.len() < 5 {
            continue;
        }
        let full = f[0];
        // Annotated tags point at a tag object; the commit is the peeled `*objectname`.
        let target = if f[2].is_empty() { f[1] } else { f[2] }.to_string();
        let (kind, name) = if let Some(n) = full.strip_prefix("refs/heads/") {
            (RefKind::Local, n)
        } else if let Some(n) = full.strip_prefix("refs/remotes/") {
            if n.ends_with("/HEAD") {
                continue;
            }
            (RefKind::Remote, n)
        } else if let Some(n) = full.strip_prefix("refs/tags/") {
            (RefKind::Tag, n)
        } else {
            continue;
        };
        let (ahead, behind) = parse_track(f[4]);
        let gone = f[4].trim() == "gone";
        refs.push(RefInfo {
            name: name.to_string(),
            kind,
            target,
            upstream: Some(f[3].to_string()).filter(|s| !s.is_empty()),
            ahead,
            behind,
            gone,
        });
    }
    Ok(refs)
}

fn parse_track(s: &str) -> (u32, u32) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in s.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

fn load_stashes(dir: &Path) -> Vec<Stash> {
    run(dir, &["stash", "list", "--format=%gd%x1f%H%x1f%s%x1f%P%x1f%at%x1f%an%x1f%ae"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            (f.len() >= 7).then(|| Stash {
                reference: f[0].to_string(),
                sha: f[1].to_string(),
                message: f[2].to_string(),
                base: f[3].split_whitespace().next().map(str::to_string),
                time: f[4].trim().parse().unwrap_or(0),
                author: f[5].to_string(),
                email: f[6].to_string(),
            })
        })
        .collect()
}

/// Adds each stash to the history as a row attached to the commit it was made on.
/// Its other parents (index and untracked snapshots) are left out on purpose: they are
/// git's bookkeeping, not history anyone wants to see.
pub fn insert_stashes(commits: &mut Vec<Commit>, stashes: &[Stash]) {
    for st in stashes {
        let Some(base_idx) = st.base.as_ref().and_then(|b| commits.iter().position(|c| &c.sha == b)) else { continue };
        let by_time = commits.iter().position(|c| c.time <= st.time).unwrap_or(commits.len());
        let at = by_time.min(base_idx);
        commits.insert(
            at,
            Commit {
                sha: st.sha.clone(),
                parents: vec![commits[base_idx].sha.clone()],
                author: st.author.clone(),
                email: st.email.clone(),
                time: st.time,
                summary: st.message.clone(),
                body: String::new(),
                stash: Some(st.reference.clone()),
            },
        );
    }
}

pub fn load_status(dir: &Path) -> GitResult<WorkStatus> {
    let out = run(dir, &["status", "--porcelain=v1", "-z", "-uall"])?;
    let mut status = WorkStatus::default();
    let mut entries = out.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let mut chars = entry.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        let path = entry[3..].to_string();
        // With -z a rename is followed by its original path as a separate entry.
        let old_path = if x == 'R' || x == 'C' {
            entries.next().map(str::to_string)
        } else {
            None
        };
        let conflicted = x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D');
        if conflicted {
            status.unstaged.push(FileChange { path, old_path: None, state: FileState::Conflicted });
            continue;
        }
        if x == '?' {
            status.unstaged.push(FileChange { path, old_path: None, state: FileState::Untracked });
            continue;
        }
        if x != ' ' {
            status.staged.push(FileChange {
                path: path.clone(),
                old_path: old_path.clone(),
                state: FileState::from_code(x),
            });
        }
        if y != ' ' {
            status.unstaged.push(FileChange { path, old_path: None, state: FileState::from_code(y) });
        }
    }
    Ok(status)
}

/// Files changed by a commit, compared with its first parent.
pub fn commit_files(dir: &Path, commit: &Commit) -> GitResult<Vec<FileChange>> {
    let out = match commit.parents.first() {
        Some(p) => run(dir, &["diff", "--name-status", "-z", "-M", p, &commit.sha])?,
        None => run(dir, &["diff-tree", "--root", "-r", "--name-status", "-z", "-M", &commit.sha])?,
    };
    let mut files = Vec::new();
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    // diff-tree --root prints the commit sha first.
    while let Some(code) = it.next() {
        if code.len() >= 40 && code.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let c = code.chars().next().unwrap_or('M');
        let state = FileState::from_code(c);
        if c == 'R' || c == 'C' {
            let old = it.next().unwrap_or_default().to_string();
            let new = it.next().unwrap_or_default().to_string();
            files.push(FileChange { path: new, old_path: Some(old), state });
        } else if let Some(p) = it.next() {
            files.push(FileChange { path: p.to_string(), old_path: None, state });
        }
    }
    Ok(files)
}

pub fn full_message(dir: &Path, sha: &str) -> String {
    run(dir, &["log", "-1", "--format=%B", sha]).unwrap_or_default().trim_end().to_string()
}

pub fn committer_line(dir: &Path, sha: &str) -> String {
    run(dir, &["log", "-1", "--format=%cn <%ce>", sha]).unwrap_or_default().trim().to_string()
}

/// Which side of the working tree a diff is taken from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffSource {
    Unstaged,
    Staged,
    Commit { sha: String, parent: Option<String> },
}

/// The unified diff of one file, and whether it was cut at [`MAX_DIFF_BYTES`]. With `whole_file` the diff carries the entire file as context,
/// which the inline and split views need.
pub fn file_diff_ctx(dir: &Path, source: &DiffSource, file: &FileChange, whole_file: bool) -> (String, bool) {
    let ctx = if whole_file { "-U100000000" } else { "-U3" };
    match source {
        DiffSource::Unstaged if file.state == FileState::Untracked => {
            let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
            run_lenient(dir, &["diff", "--no-color", ctx, "--no-index", "--", null, &file.path])
        }
        DiffSource::Unstaged => run_lenient(dir, &["diff", "--no-color", ctx, "--", &file.path]),
        DiffSource::Staged => run_lenient(dir, &["diff", "--no-color", ctx, "--cached", "-M", "--", &file.path]),
        DiffSource::Commit { sha, parent: Some(p) } => {
            let mut args = vec!["diff", "--no-color", ctx, "-M", p.as_str(), sha.as_str(), "--"];
            if let Some(old) = &file.old_path {
                args.push(old);
            }
            args.push(&file.path);
            run_lenient(dir, &args)
        }
        DiffSource::Commit { sha, parent: None } => {
            run_lenient(dir, &["show", "--no-color", ctx, "--format=", sha, "--", &file.path])
        }
    }
}

/// Short sha for display.
pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

// ---------------------------------------------------------------------------------------------
// Phase 1 "quick wins": remotes, remote tags, upstream, forced branch delete, skip,
// identity and merge options. Builders return argument lists so the UI can queue them on
// the worker like every other command; small queries run git directly.
// ---------------------------------------------------------------------------------------------

/// `git remote add <name> <url>`.
pub fn remote_add_args(name: &str, url: &str) -> Vec<String> {
    vec!["remote".into(), "add".into(), name.into(), url.into()]
}

pub fn remote_rename_args(old: &str, new: &str) -> Vec<String> {
    vec!["remote".into(), "rename".into(), old.into(), new.into()]
}

pub fn remote_set_url_args(name: &str, url: &str) -> Vec<String> {
    vec!["remote".into(), "set-url".into(), name.into(), url.into()]
}

pub fn remote_remove_args(name: &str) -> Vec<String> {
    vec!["remote".into(), "remove".into(), name.into()]
}

/// Checks that `url` is a reachable repository (the same `ls-remote` the clone check uses).
/// Returns how many branches it has, or the error put in plain words.
pub fn remote_check(dir: &Path, url: &str) -> Result<usize, String> {
    run(dir, &["ls-remote", "--heads", url]).map(|out| out.lines().filter(|l| !l.trim().is_empty()).count()).map_err(|e| explain_remote_error(&e.0))
}

/// Turns git's remote errors into a sentence with what to do next.
pub fn explain_remote_error(e: &str) -> String {
    let l = e.to_lowercase();
    if crate::access::looks_like_auth_error(e) || l.contains("could not read from remote repository") {
        "Git could not get in: the address is wrong, or this repository is private and you need to sign in (Access window).".into()
    } else if l.contains("does not appear to be a git repository") || l.contains("not found") || l.contains("no such file") {
        "Nothing answers as a git repository at this address. Check the URL.".into()
    } else if l.contains("could not resolve host") || l.contains("unable to access") || l.contains("timed out") {
        "Could not reach the server. Check the address and your connection.".into()
    } else {
        e.trim().to_string()
    }
}

/// The name to propose for a new remote: `origin` when there is none; `upstream` when the
/// URL is the same repository under another owner (a fork's original); otherwise the
/// owner's name. Never one that is already taken.
pub fn suggest_remote_name(existing: &[(String, String)], url: &str) -> String {
    let taken = |n: &str| existing.iter().any(|(e, _)| e == n);
    if existing.is_empty() {
        return "origin".into();
    }
    let new = crate::forge::detect("new", url);
    let base = existing.iter().find(|(n, _)| n == "origin").or(existing.first()).and_then(|(n, u)| crate::forge::detect(n, u));
    let mut candidate = match (&new, &base) {
        (Some(n), Some(b)) => {
            let (n_owner, n_repo) = n.path.rsplit_once('/').unwrap_or(("", &n.path));
            let (b_owner, b_repo) = b.path.rsplit_once('/').unwrap_or(("", &b.path));
            if n_repo.eq_ignore_ascii_case(b_repo) && !n_owner.eq_ignore_ascii_case(b_owner) && !taken("upstream") {
                "upstream".to_string()
            } else {
                n_owner.rsplit('/').next().unwrap_or("remote").to_lowercase()
            }
        }
        (Some(n), None) => n.path.split('/').next().unwrap_or("remote").to_lowercase(),
        _ => "remote".into(),
    };
    if candidate.is_empty() {
        candidate = "remote".into();
    }
    if !taken(&candidate) {
        return candidate;
    }
    (2..).map(|k| format!("{candidate}{k}")).find(|n| !taken(n)).unwrap_or(candidate)
}

/// Tag names that exist on `remote` (one `ls-remote`, nothing downloaded).
pub fn remote_tags(dir: &Path, remote: &str) -> GitResult<std::collections::HashSet<String>> {
    let out = run(dir, &["ls-remote", "--tags", "--refs", remote])?;
    Ok(out.lines().filter_map(|l| l.split_whitespace().nth(1)?.strip_prefix("refs/tags/").map(str::to_string)).collect())
}

/// Pushes tags one by one with explicit refspecs: unlike `--follow-tags` this also pushes
/// lightweight tags.
pub fn push_tags_args(remote: &str, tags: &[String]) -> Vec<String> {
    let mut args = vec!["push".to_string(), remote.to_string()];
    args.extend(tags.iter().map(|t| format!("refs/tags/{t}")));
    args
}

pub fn delete_remote_tag_args(remote: &str, tag: &str) -> Vec<String> {
    vec!["push".into(), remote.into(), "--delete".into(), format!("refs/tags/{tag}")]
}

/// Tags reachable from `rev` (what pushing `rev` makes "pushed" on the server).
pub fn tags_reachable(dir: &Path, rev: &str) -> GitResult<Vec<String>> {
    Ok(run(dir, &["tag", "--merged", rev])?.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

/// `git branch --set-upstream-to=<remote_branch> <local>`.
pub fn set_upstream_args(local: &str, remote_branch: &str) -> Vec<String> {
    vec!["branch".into(), format!("--set-upstream-to={remote_branch}"), local.into()]
}

pub fn unset_upstream_args(local: &str) -> Vec<String> {
    vec!["branch".into(), "--unset-upstream".into(), local.into()]
}

/// Commits of `branch` that no other branch (local or remote) contains: what deleting it
/// would lose. Newest first, as (sha, summary).
pub fn unique_commits(dir: &Path, branch: &str) -> GitResult<Vec<(String, String)>> {
    let head = format!("refs/heads/{branch}");
    let exclude = format!("--exclude={branch}");
    let out = run(dir, &["log", "--format=%H%x1f%s", &head, "--not", &exclude, "--branches", "--remotes"])?;
    Ok(out.lines().filter_map(|l| l.split_once('\x1f').map(|(a, b)| (a.to_string(), b.to_string()))).collect())
}

/// The commit a branch points at.
pub fn branch_sha(dir: &Path, branch: &str) -> GitResult<String> {
    Ok(run(dir, &["rev-parse", "--verify", &format!("refs/heads/{branch}^{{commit}}")])?.trim().to_string())
}

impl PendingOp {
    /// Operations that can drop the commit they stopped on (`--skip`).
    pub fn can_skip(self) -> bool {
        matches!(self, Self::Rebase | Self::CherryPick | Self::Revert)
    }
}

/// Who commits: `user.name` / `user.email` for all projects (global) and for this one
/// (local), plus the values git will really use. Those can come from a file the global or
/// local config includes (`include`, `includeIf`) or from the system config, so
/// completeness is judged on them, never on the two levels alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub global_name: Option<String>,
    pub global_email: Option<String>,
    pub local_name: Option<String>,
    pub local_email: Option<String>,
    /// What git uses, every level and included file considered (last one wins).
    pub effective_name: Option<String>,
    pub effective_email: Option<String>,
}

impl Identity {
    pub fn name(&self) -> Option<&str> {
        self.effective_name.as_deref().or(self.local_name.as_deref()).or(self.global_name.as_deref())
    }

    pub fn email(&self) -> Option<&str> {
        self.effective_email.as_deref().or(self.local_email.as_deref()).or(self.global_email.as_deref())
    }

    /// Both set somewhere, so git can commit.
    pub fn is_complete(&self) -> bool {
        self.name().is_some() && self.email().is_some()
    }

    /// Which level the name in use comes from.
    pub fn source(&self) -> &'static str {
        let local = self.local_name.is_some() || self.local_email.is_some();
        let global = self.global_name.is_some() || self.global_email.is_some();
        match (local, global, self.is_complete()) {
            (true, _, _) => "this project",
            (false, true, _) => "all projects",
            (false, false, true) => "git's system settings",
            _ => "nowhere yet",
        }
    }
}

/// Reads the identity in one `git config` call. Without a file option, git follows
/// `include` / `includeIf` and reports an included value under the scope of the file that
/// includes it, so a `includeIf "gitdir:~/work/"` set in the global config counts as global.
pub fn identity(dir: &Path) -> Identity {
    let mut id = Identity::default();
    // Exit code 1 means "no match": nothing is set anywhere.
    let Ok(out) = run(dir, &["config", "--show-scope", "--get-regexp", r"^user\.(name|email)$"]) else { return id };
    for line in out.lines() {
        // "<scope>\t<key> <value>"; the value may contain spaces.
        let Some((scope, rest)) = line.split_once('\t') else { continue };
        let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
        let value = Some(value.trim().to_string()).filter(|v| !v.is_empty());
        let is_name = key.eq_ignore_ascii_case("user.name");
        match scope {
            "global" if is_name => id.global_name = value.clone(),
            "global" => id.global_email = value.clone(),
            "local" | "worktree" if is_name => id.local_name = value.clone(),
            "local" | "worktree" => id.local_email = value.clone(),
            _ => {}
        }
        // Lines come in the order git reads them, so the last one is the one it uses.
        if is_name {
            id.effective_name = value;
        } else {
            id.effective_email = value;
        }
    }
    id
}

/// `git config [--global|--local] <key> <value>`; an empty value removes the key.
pub fn set_identity_args(global: bool, key: &str, value: &str) -> Vec<String> {
    let scope = if global { "--global" } else { "--local" };
    if value.trim().is_empty() {
        vec!["config".into(), scope.into(), "--unset".into(), key.into()]
    } else {
        vec!["config".into(), scope.into(), key.into(), value.trim().into()]
    }
}

/// How a branch is brought into the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMode {
    /// Fast-forward when possible, a merge commit otherwise (git's default).
    Normal,
    /// All its changes staged as one new commit to write (`--squash`).
    Squash,
    /// Always a merge commit (`--no-ff`).
    NoFastForward,
    /// Only when no merge commit is needed (`--ff-only`).
    FastForwardOnly,
}

pub fn merge_args(mode: MergeMode, source: &str) -> Vec<String> {
    let mut args = vec!["merge".to_string()];
    match mode {
        MergeMode::Normal => args.push("--no-edit".into()),
        MergeMode::Squash => args.push("--squash".into()),
        MergeMode::NoFastForward => args.extend(["--no-ff".into(), "--no-edit".into()]),
        MergeMode::FastForwardOnly => args.push("--ff-only".into()),
    }
    args.push(source.into());
    args
}

/// Commit message for a squash: a summary, and the list of the commits it combines.
pub fn squash_message(dir: &Path, source: &str, target: &str) -> (String, String) {
    // From the target branch rather than HEAD: a drag-and-drop merge switches to it first.
    let list = run(dir, &["log", "--reverse", "--format=- %s", &format!("{target}..{source}")]).unwrap_or_default();
    (format!("Bring {source} into {target}"), list.trim_end().to_string())
}
