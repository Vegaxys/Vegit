//! The safety net (design doc, "Filet de sécurité"): what Vegit records before a local
//! action so it can be undone, the backups of files a discard would lose, and the reflog in
//! plain sentences for the Activity panel.
//!
//! The idea is generic rather than per command: the refs (branches, tags, HEAD, stashes) are
//! read before and after an action, and the difference gives the commands that put them back.
//! Files a command would throw away (discard, `reset --hard`) are first saved in a commit kept
//! under `refs/vegit/backups/`, so the garbage collector leaves them alone for 30 days.
//! Only local state is covered: the server is never touched by an undo.

use std::collections::BTreeMap;
use std::path::Path;

use crate::git::{self, GitResult};

/// Backups older than this are deleted when a new one is made.
pub const BACKUP_DAYS: u64 = 30;

/// Branches, tags, HEAD and stashes at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefState {
    /// Current branch, `None` when detached.
    pub head_branch: Option<String>,
    pub head: Option<String>,
    /// `refs/heads/*` and `refs/tags/*` → the object they point at.
    pub refs: BTreeMap<String, String>,
    /// Stash entries, newest first, as (commit, message).
    pub stashes: Vec<(String, String)>,
}

pub fn ref_state(dir: &Path) -> RefState {
    let refs = git::run(dir, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads", "refs/tags"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(n, s)| (n.to_string(), s.trim().to_string())))
        .collect();
    let head_branch = git::run(dir, &["symbolic-ref", "-q", "HEAD"])
        .ok()
        .and_then(|s| s.trim().strip_prefix("refs/heads/").map(str::to_string));
    let head = git::run(dir, &["rev-parse", "-q", "--verify", "HEAD"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let stashes = git::run(dir, &["stash", "list", "--format=%H%x1f%gs"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\x1f').map(|(h, m)| (h.to_string(), m.to_string())))
        .collect();
    RefState { head_branch, head, refs, stashes }
}

/// How the current branch is moved back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    /// The undone commits' changes come back staged (undoing a commit).
    Soft,
    /// Local changes are kept; refused if they would be overwritten (merge, rebase…).
    Keep,
    /// Back exactly (undoing a `reset --hard`; the files are restored from the backup after).
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Keep => "--keep",
            Self::Hard => "--hard",
        }
    }
}

/// One step of an action or of its undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Git(Vec<String>),
    /// Git with this text on stdin (a patch).
    Input(Vec<String>, String),
    /// Git with extra environment variables (`GIT_SEQUENCE_EDITOR` for history edits).
    GitEnv(Vec<String>, Vec<(String, String)>),
    /// Replace a file of the working folder with this text (a resolved conflict block).
    Write(String, String),
    /// Delete a file of the working folder (relative path).
    Delete(String),
    /// Delete a whole folder (git's own data of a removed submodule).
    DeleteDir(std::path::PathBuf),
    /// Remove the folders of these files (relative paths) that are left empty, deepest
    /// first, never the working folder itself.
    PruneEmptyDirs(Vec<String>),
    /// Save these files before what follows changes them (undo restores them).
    Backup(Vec<String>),
}

pub fn git_step(args: &[&str]) -> Step {
    Step::Git(args.iter().map(|s| s.to_string()).collect())
}

/// What a recorded action was and how to take it back.
#[derive(Clone, Debug)]
pub struct UndoEntry {
    /// Unique within the session, so an old toast's Undo can't undo something else.
    pub id: u64,
    /// "commit", "rebase", "discard (3 files)", "delete branch"…
    pub what: String,
    pub mode: ResetMode,
    /// Commands that undo it.
    pub steps: Vec<Step>,
    /// The original steps, run again by Redo when undoing moved no ref (a discard).
    pub forward: Vec<Step>,
    /// Local branches it moved, created or deleted.
    pub branches: Vec<String>,
    /// One of those branches was pushed since: undoing would need a force push.
    pub pushed: bool,
}

/// The name and reset mode of a command worth recording, `None` for the others (staging,
/// config, remotes, aborts…), which either change nothing that can be lost or are their own undo.
pub fn describe(args: &[String]) -> Option<(String, ResetMode)> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let has = |f: &str| a.contains(&f);
    let keep = |w: &str| Some((w.to_string(), ResetMode::Keep));
    match *a.first()? {
        "commit" => Some((if has("--amend") { "amend" } else { "commit" }.to_string(), ResetMode::Soft)),
        // Steps of an operation stopped on a conflict: the banner's Abort is their undo.
        "merge" | "rebase" | "cherry-pick" | "revert" | "am" if has("--abort") || has("--quit") || has("--continue") || has("--skip") => None,
        "merge" if has("--squash") => None,
        "merge" => keep("merge"),
        "rebase" => keep("rebase"),
        "cherry-pick" => keep("copy commit"),
        "revert" => keep("revert"),
        "pull" => keep("pull"),
        "reset" if has("--hard") => Some(("reset".into(), ResetMode::Hard)),
        "reset" if has("--soft") || has("--mixed") => Some(("reset".into(), ResetMode::Soft)),
        "reset" => None, // `reset -q`: unstage everything
        "branch" if has("-D") || has("-d") => keep("delete branch"),
        "branch" if has("-m") || has("-M") => keep("rename branch"),
        "branch" if a.iter().any(|x| x.starts_with("--set-upstream") || *x == "--unset-upstream") => None,
        "branch" => keep("create branch"),
        "switch" | "checkout" if has("--") => None,
        "switch" | "checkout" if has("-c") || has("-b") => keep("create branch"),
        "switch" | "checkout" => keep("switch branch"),
        "tag" if has("-d") => keep("delete tag"),
        "tag" => keep("create tag"),
        "stash" => match a.get(1).copied() {
            None | Some("push") => keep("stash"),
            Some("pop") => keep("pop stash"),
            Some("drop") => keep("delete stash"),
            Some("clear") => keep("delete all stashes"),
            Some("branch") => keep("stash to branch"),
            _ => None,
        },
        "restore" if has("--worktree") && !has("--staged") && !a.iter().any(|x| x.starts_with("--source")) => keep("discard"),
        "apply" if has("-R") && !has("--cached") => keep("discard change"),
        _ => None,
    }
}

/// Files a command is about to throw away, to back up first.
pub fn destroyed_paths(dir: &Path, args: &[String]) -> Vec<String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.first().copied() {
        Some("reset") if a.contains(&"--hard") => git::run(dir, &["diff", "--name-only", "HEAD"])
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Some("restore") if a.contains(&"--worktree") && !a.iter().any(|x| x.starts_with("--source")) => {
            a.iter().skip_while(|x| **x != "--").skip(1).map(|s| s.to_string()).collect()
        }
        _ => Vec::new(),
    }
}

/// Files saved before a destructive command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backup {
    /// The commit holding the saved files, kept by a `refs/vegit/backups/` ref.
    pub commit: Option<String>,
    /// Files that existed and were saved.
    pub present: Vec<String>,
    /// Files that didn't exist (a deletion being discarded): undo deletes them again.
    pub absent: Vec<String>,
}

impl Backup {
    /// Steps that put the saved files back.
    pub fn restore_steps(&self) -> Vec<Step> {
        let mut steps = Vec::new();
        if let Some(c) = &self.commit {
            for chunk in self.present.chunks(100) {
                let mut args = vec!["restore".to_string(), format!("--source={c}"), "--worktree".into(), "--".into()];
                args.extend(chunk.iter().cloned());
                steps.push(Step::Git(args));
            }
        }
        steps.extend(self.absent.iter().cloned().map(Step::Delete));
        steps
    }
}

/// Saves the current content of `paths` (tracked or not) in a commit that no branch shows,
/// using a temporary index so the real staging area is untouched.
pub fn backup(dir: &Path, paths: &[String]) -> GitResult<Backup> {
    let (present, absent): (Vec<String>, Vec<String>) = paths.iter().cloned().partition(|p| dir.join(p).is_file());
    if present.is_empty() {
        return Ok(Backup { commit: None, present, absent });
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let index = std::env::temp_dir().join(format!("vegit-backup-{}-{}.idx", std::process::id(), now.as_nanos()));
    let index_s = index.display().to_string();
    // Commits need an author; this one is Vegit's own, whatever the person's identity.
    let envs = [
        ("GIT_INDEX_FILE", index_s.as_str()),
        ("GIT_AUTHOR_NAME", "Vegit"),
        ("GIT_AUTHOR_EMAIL", "vegit@localhost"),
        ("GIT_COMMITTER_NAME", "Vegit"),
        ("GIT_COMMITTER_EMAIL", "vegit@localhost"),
    ];
    let result = (|| {
        for chunk in present.chunks(100) {
            let mut args = vec!["add", "-f", "--"];
            args.extend(chunk.iter().map(String::as_str));
            git::run_env(dir, &args, &envs)?;
        }
        let tree = git::run_env(dir, &["write-tree"], &envs)?.trim().to_string();
        let commit = git::run_env(dir, &["commit-tree", &tree, "-m", "Vegit backup before discarding changes"], &envs)?.trim().to_string();
        git::run(dir, &["update-ref", &format!("refs/vegit/backups/{}-{}", now.as_secs(), now.subsec_nanos()), &commit])?;
        Ok(commit)
    })();
    let _ = std::fs::remove_file(&index);
    purge_backups(dir, BACKUP_DAYS);
    Ok(Backup { commit: Some(result?), present, absent })
}

/// Deletes the backup refs older than `days` (their name starts with the creation time).
pub fn purge_backups(dir: &Path, days: u64) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let refs = git::run(dir, &["for-each-ref", "--format=%(refname)", "refs/vegit/backups"]).unwrap_or_default();
    for r in refs.lines() {
        let made = r.rsplit('/').next().and_then(|n| n.split('-').next()).and_then(|t| t.parse::<u64>().ok());
        if made.is_some_and(|t| now.saturating_sub(t) > days * 86_400) {
            let _ = git::run(dir, &["update-ref", "-d", r]);
        }
    }
}

/// The commands that bring the refs from `after` back to `before`.
///
/// Order matters: deleted refs come back first (so HEAD can return to a renamed branch),
/// then HEAD, then other branches, then the current branch with `mode`, then refs the action
/// created are removed, and last the stashes.
pub fn undo_steps(before: &RefState, after: &RefState, mode: ResetMode) -> Vec<Step> {
    let mut steps = Vec::new();
    let current = before.head_branch.as_ref().map(|b| format!("refs/heads/{b}"));
    for (name, sha) in &before.refs {
        if !after.refs.contains_key(name) {
            steps.push(git_step(&["update-ref", name, sha]));
        }
    }
    match (&before.head_branch, &after.head_branch) {
        (Some(b), a) if a.as_ref() != Some(b) => {
            if before.refs.contains_key(&format!("refs/heads/{b}")) {
                steps.push(git_step(&["switch", b]));
            } else {
                // The branch had no commit yet: point HEAD at it again.
                steps.push(git_step(&["symbolic-ref", "HEAD", &format!("refs/heads/{b}")]));
            }
        }
        (None, a) => {
            if let Some(h) = &before.head {
                if a.is_some() {
                    steps.push(git_step(&["switch", "--detach", h]));
                } else if after.head.as_ref() != Some(h) {
                    steps.push(git_step(&["reset", mode.flag(), h]));
                }
            }
        }
        _ => {}
    }
    for (name, old) in &before.refs {
        if let Some(new) = after.refs.get(name) {
            if new != old && Some(name) != current.as_ref() {
                steps.push(git_step(&["update-ref", name, old]));
            }
        }
    }
    if let Some(c) = &current {
        if let (Some(old), Some(new)) = (before.refs.get(c), after.refs.get(c)) {
            if old != new {
                steps.push(git_step(&["reset", mode.flag(), old]));
            }
        }
    }
    for name in after.refs.keys() {
        if !before.refs.contains_key(name) {
            steps.push(git_step(&["update-ref", "-d", name]));
        }
    }
    let (b, a) = (&before.stashes, &after.stashes);
    if a.len() == b.len() + 1 && a[1..] == b[..] {
        steps.push(git_step(&["stash", "pop"]));
    } else if a.len() < b.len() {
        // Entries that disappeared (a drop, a pop, or "Delete all"): stored back oldest first,
        // so the newest ends on top again.
        for (sha, msg) in b.iter().rev().filter(|e| !a.contains(e)) {
            steps.push(git_step(&["stash", "store", "-m", msg, sha]));
        }
    }
    steps
}

/// Local branches whose position differs between the two states.
pub fn changed_branches(before: &RefState, after: &RefState) -> Vec<String> {
    let mut names: Vec<String> = before
        .refs
        .keys()
        .chain(after.refs.keys())
        .filter(|n| n.starts_with("refs/heads/") && before.refs.get(*n) != after.refs.get(*n))
        .map(|n| n["refs/heads/".len()..].to_string())
        .collect();
    names.dedup();
    names
}

/// Runs steps one after the other and stops at the first failure.
pub fn run_steps(dir: &Path, steps: &[Step], backups: &mut Vec<Backup>, record: bool) -> Result<String, String> {
    let mut out = String::new();
    for step in steps {
        match step {
            Step::Git(args) => {
                if record {
                    let paths = destroyed_paths(dir, args);
                    if !paths.is_empty() {
                        backups.push(backup(dir, &paths).map_err(|e| format!("Could not save the files first, nothing was changed: {e}"))?);
                    }
                }
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                out = git::run(dir, &refs).map_err(|e| e.0)?;
            }
            Step::Input(args, input) => {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                out = git::run_with_input(dir, &refs, input).map_err(|e| e.0)?;
            }
            Step::GitEnv(args, envs) => {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let envs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                out = git::run_env(dir, &refs, &envs).map_err(|e| e.0)?;
            }
            Step::Write(rel, text) => {
                std::fs::write(dir.join(rel), text).map_err(|e| format!("{rel}: {e}"))?;
            }
            Step::DeleteDir(p) => {
                if p.exists() {
                    std::fs::remove_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
                }
            }
            Step::PruneEmptyDirs(paths) => {
                let mut dirs: Vec<&Path> = paths.iter().flat_map(|p| Path::new(p).ancestors().skip(1)).filter(|d| !d.as_os_str().is_empty()).collect();
                dirs.sort();
                dirs.dedup();
                dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
                for d in dirs {
                    let _ = std::fs::remove_dir(dir.join(d)); // refused, as wanted, while not empty
                }
            }
            Step::Delete(rel) => {
                let p = dir.join(rel);
                if p.exists() {
                    std::fs::remove_file(&p).map_err(|e| format!("{rel}: {e}"))?;
                }
            }
            Step::Backup(paths) => {
                if record {
                    backups.push(backup(dir, paths).map_err(|e| format!("Could not save the files first, nothing was changed: {e}"))?);
                }
            }
        }
    }
    Ok(out)
}

/// Runs `steps`; when `what` is given, also returns how to undo them (`None` when they
/// changed nothing that can be put back).
pub fn run_recorded(dir: &Path, steps: &[Step], what: Option<(String, ResetMode)>) -> (Result<String, String>, Option<UndoEntry>) {
    let Some((what, mode)) = what else {
        return (run_steps(dir, steps, &mut Vec::new(), false), None);
    };
    let before = ref_state(dir);
    let was_pending = git::pending_op(dir).is_some();
    let mut backups = Vec::new();
    let result = run_steps(dir, steps, &mut backups, true);
    // A failure leaves either nothing changed or an operation in progress (a conflict),
    // which the banner's Abort takes back better than a ref reset would. The same goes for
    // an action that stops midway on purpose (a history edit stopping on a commit): git
    // refuses to switch branches until it is finished or aborted.
    if result.is_err() || (!was_pending && git::pending_op(dir).is_some()) {
        return (result, None);
    }
    let after = ref_state(dir);
    let mut undo = undo_steps(&before, &after, mode);
    for b in &backups {
        undo.extend(b.restore_steps());
    }
    if undo.is_empty() {
        return (result, None);
    }
    let entry = UndoEntry { id: 0, what, mode, steps: undo, forward: steps.to_vec(), branches: changed_branches(&before, &after), pushed: false };
    (result, Some(entry))
}

/// One line of the Activity panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    /// Where HEAD was after this event: "Show" and "Restore as a new branch" use it.
    pub sha: String,
    pub time: i64,
    pub text: String,
}

/// The last `n` meaningful reflog events of HEAD, in plain sentences.
pub fn activity(dir: &Path, n: usize) -> Vec<Activity> {
    let out = git::run(dir, &["reflog", "-n", &(n * 3).to_string(), "--date=unix", "--format=%H%x1f%gd%x1f%gs"]).unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            if f.len() < 3 {
                return None;
            }
            let time = f[1].split_once('{').and_then(|(_, t)| t.trim_end_matches('}').parse().ok()).unwrap_or(0);
            Some(Activity { sha: f[0].to_string(), time, text: translate(f[2])? })
        })
        .take(n)
        .collect()
}

fn short_ref(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("refs/heads/").unwrap_or(s);
    if s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) { s[..7].to_string() } else { s.to_string() }
}

/// A reflog subject in plain words; `None` for the in-between steps of a rebase.
pub fn translate(subject: &str) -> Option<String> {
    let (kind, rest) = subject.split_once(": ").unwrap_or((subject, ""));
    let rest = rest.trim();
    Some(match kind {
        "commit" => format!("Committed: {rest}"),
        "commit (initial)" => format!("First commit: {rest}"),
        "commit (amend)" => format!("Amended the last commit: {rest}"),
        "commit (merge)" => format!("Committed a merge: {rest}"),
        "checkout" => match rest.strip_prefix("moving from ").and_then(|r| r.split_once(" to ")) {
            Some((a, b)) => format!("Switched from {} to {}", short_ref(a), short_ref(b)),
            None => format!("Switched: {rest}"),
        },
        k if k.starts_with("rebase") || k.starts_with("pull --rebase") => {
            if k.contains("finish") {
                match rest.strip_prefix("returning to ") {
                    Some(r) => format!("Replayed {} on its new base", short_ref(r)),
                    None => "Replayed commits on a new base".into(),
                }
            } else if k.contains("abort") {
                "Cancelled a rebase".into()
            } else {
                return None;
            }
        }
        "reset" => match rest.strip_prefix("moving to ") {
            Some(t) => format!("Moved the branch to {}", short_ref(t)),
            None => format!("Moved the branch: {rest}"),
        },
        k if k.starts_with("merge ") => {
            let ff = if rest.contains("Fast-forward") { " (fast-forward)" } else { "" };
            format!("Brought {} in{ff}", short_ref(&k["merge ".len()..]))
        }
        k if k.starts_with("pull") => "Pulled from the server".into(),
        "cherry-pick" => format!("Copied a commit: {rest}"),
        "revert" => format!("Reverted: {rest}"),
        "branch" => match rest.strip_prefix("Created from ") {
            Some(f) => format!("Created a branch from {}", short_ref(f)),
            None => format!("Branch: {rest}"),
        },
        "clone" => "Cloned the repository".into(),
        _ => subject.to_string(),
    })
}
