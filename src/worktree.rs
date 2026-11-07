//! Worktrees (design doc, "Worktrees"): a branch open in another folder, shown as a tab.

use std::path::{Path, PathBuf};

use crate::git;

/// One folder of the repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: String,
    /// Checked-out branch, `None` when detached.
    pub branch: Option<String>,
    /// The repository's own folder (the first one listed).
    pub main: bool,
    pub locked: bool,
    /// Its folder is gone: `worktree prune` will drop it.
    pub prunable: bool,
    /// Files with uncommitted changes there.
    pub changes: usize,
}

/// Every worktree, the main one first, with their uncommitted change counts.
pub fn list(dir: &Path) -> Vec<Worktree> {
    let out = git::run(dir, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    let mut all = Vec::new();
    let mut cur: Option<Worktree> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            if let Some(w) = cur.take() {
                all.push(w);
            }
            cur = Some(Worktree { path: PathBuf::from(p), main: all.is_empty(), ..Default::default() });
        } else if let Some(w) = cur.as_mut() {
            if let Some(h) = line.strip_prefix("HEAD ") {
                w.head = h.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                w.branch = Some(b.trim_start_matches("refs/heads/").to_string());
            } else if line.starts_with("locked") {
                w.locked = true;
            } else if line.starts_with("prunable") {
                w.prunable = true;
            }
        }
    }
    if let Some(w) = cur {
        all.push(w);
    }
    for w in all.iter_mut().filter(|w| !w.main && !w.prunable) {
        w.changes = changed_files(&w.path).len();
    }
    all
}

/// Uncommitted files in a worktree folder (for "Remove folder" and the dot in the list).
pub fn changed_files(path: &Path) -> Vec<String> {
    git::run(path, &["status", "--porcelain", "-uall"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.get(3..).map(str::to_string))
        .collect()
}

/// The folder proposed for a branch: next to the repository, `<repo>-<branch>`.
pub fn suggest_path(repo: &Path, branch: &str) -> PathBuf {
    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
    let short = branch.split_once('/').filter(|_| branch.starts_with("origin/")).map_or(branch, |(_, b)| b);
    let safe: String = short.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '-' } else { c }).collect();
    repo.parent().unwrap_or(repo).join(format!("{name}-{safe}"))
}

/// `worktree add` for a local branch, a remote one (a local branch tracking it is made), or
/// a new branch from `start`.
pub fn add_args(path: &Path, branch: &str, remote: bool, new_from: Option<&str>) -> Vec<String> {
    let p = path.display().to_string();
    if let Some(start) = new_from {
        return vec!["worktree".into(), "add".into(), "-b".into(), branch.into(), p, start.into()];
    }
    if remote {
        let local = branch.split_once('/').map_or(branch, |(_, b)| b);
        return vec!["worktree".into(), "add".into(), "--track".into(), "-b".into(), local.into(), p, branch.into()];
    }
    vec!["worktree".into(), "add".into(), p, branch.into()]
}

pub fn remove_args(path: &Path, force: bool) -> Vec<String> {
    let mut v = vec!["worktree".to_string(), "remove".into()];
    if force {
        v.push("--force".into());
    }
    v.push(path.display().to_string());
    v
}

/// The folder a branch is open in, from git's refusal to switch to it.
pub fn path_in_error(e: &str) -> Option<String> {
    let l = e.find("worktree at '").map(|i| i + "worktree at '".len()).or_else(|| e.find("checked out at '").map(|i| i + "checked out at '".len()))?;
    e[l..].split('\'').next().map(str::to_string)
}
