//! Git layer of phases 4 and 5 of the design doc: staging lines, comparing, searching the
//! content, paging the history, advanced stash, copying several commits, bisect,
//! `rebase --onto`, submodules, clone options, cleanup, sparse-checkout, patches, signatures,
//! maintenance and commit options. Each function builds arguments or reads git; the app runs
//! local changes through the safety net so they can be undone.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::git::{self, FileChange, FileState, GitResult};
use crate::safety::Step;

// ---------------------------------------------------------------------------------------------
// Staging lines (doc §Stager des lignes)
// ---------------------------------------------------------------------------------------------

/// A patch holding only the `selected` lines of a one-hunk patch (indices among the lines
/// after the `@@` line). Forward (staging): unselected `+` lines are dropped and unselected
/// `-` lines become context. Reverse (unstaging, discarding with `-R`): the other way round.
/// The hunk header is recounted. `None` when nothing selected is a change.
pub fn line_patch(patch: &str, selected: &[usize], reverse: bool) -> Option<String> {
    let mut lines = patch.split_inclusive('\n');
    let mut head = String::new();
    let mut hunk_header = None;
    for l in lines.by_ref() {
        if l.starts_with("@@") {
            hunk_header = Some(l.to_string());
            break;
        }
        head.push_str(l);
    }
    let header = hunk_header?;
    let (old_start, new_start) = starts(&header);
    let selected: HashSet<usize> = selected.iter().copied().collect();
    let (mut body, mut old_n, mut new_n, mut changes) = (String::new(), 0u32, 0u32, 0usize);
    for (i, l) in lines.enumerate() {
        let picked = selected.contains(&i);
        match l.as_bytes().first() {
            Some(b'+') if picked => {
                body.push_str(l);
                new_n += 1;
                changes += 1;
            }
            Some(b'-') if picked => {
                body.push_str(l);
                old_n += 1;
                changes += 1;
            }
            // Not picked: keep the side that stays as it is.
            Some(b'+') if reverse => {
                body.push(' ');
                body.push_str(&l[1..]);
                old_n += 1;
                new_n += 1;
            }
            Some(b'-') if !reverse => {
                body.push(' ');
                body.push_str(&l[1..]);
                old_n += 1;
                new_n += 1;
            }
            Some(b'+') | Some(b'-') => {}
            Some(b'\\') => body.push_str(l), // "\ No newline at end of file"
            _ => {
                body.push_str(l);
                old_n += 1;
                new_n += 1;
            }
        }
    }
    if changes == 0 {
        return None;
    }
    let tail = header.splitn(3, "@@").nth(2).unwrap_or("\n");
    let mut out = head;
    out.push_str(&format!("@@ -{old_start},{old_n} +{new_start},{new_n} @@{tail}"));
    out.push_str(&body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

fn starts(header: &str) -> (u32, u32) {
    let mut old = 1;
    let mut new = 1;
    for part in header.split_whitespace().skip(1).take(2) {
        let num = part[1..].split(',').next().unwrap_or("1").parse().unwrap_or(1);
        if part.starts_with('-') {
            old = num;
        } else if part.starts_with('+') {
            new = num;
        }
    }
    (old, new)
}

// ---------------------------------------------------------------------------------------------
// Comparing (doc §Comparer)
// ---------------------------------------------------------------------------------------------

/// Files that differ between two commits (`diff a b`).
pub fn compare_files(dir: &Path, a: &str, b: &str) -> GitResult<Vec<FileChange>> {
    let out = git::run(dir, &["diff", "--name-status", "-M", a, b])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let state = match f.first()?.chars().next()? {
                'A' => FileState::Added,
                'D' => FileState::Deleted,
                'R' => FileState::Renamed,
                _ => FileState::Modified,
            };
            match (state, f.len()) {
                (FileState::Renamed, 3) => Some(FileChange { path: f[2].into(), old_path: Some(f[1].into()), state }),
                (_, n) if n >= 2 => Some(FileChange { path: f[1].into(), old_path: None, state }),
                _ => None,
            }
        })
        .collect())
}

pub fn merge_base(dir: &Path, a: &str, b: &str) -> Option<String> {
    git::run(dir, &["merge-base", a, b]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn rev(dir: &Path, r: &str) -> Option<String> {
    git::run(dir, &["rev-parse", "-q", "--verify", &format!("{r}^{{commit}}")]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Commits on `b` that `a` doesn't have (`log a..b`), newest first.
pub fn only_on(dir: &Path, a: &str, b: &str) -> Vec<(String, String)> {
    git::run(dir, &["log", "-n", "500", "--format=%H%x1f%s", &format!("{a}..{b}")])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\x1f').map(|(h, s)| (h.to_string(), s.to_string())))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Searching (doc §Chercher)
// ---------------------------------------------------------------------------------------------

const ALL_REFS: [&str; 4] = ["--branches", "--remotes", "--tags", "HEAD"];

/// Commits that add or remove `text` (`log -S`), over every branch.
pub fn search_changed_text(dir: &Path, text: &str) -> GitResult<HashSet<String>> {
    let s = format!("-S{text}");
    let mut args = vec!["log", "--format=%H", s.as_str()];
    args.extend(ALL_REFS);
    Ok(git::run(dir, &args)?.lines().map(str::to_string).collect())
}

/// Commits touching a file whose path contains `text`.
pub fn search_file_path(dir: &Path, text: &str) -> GitResult<HashSet<String>> {
    let spec = format!(":(glob,icase)**/*{text}*");
    let mut args = vec!["log", "--format=%H"];
    args.extend(ALL_REFS);
    args.extend(["--", spec.as_str()]);
    Ok(git::run(dir, &args)?.lines().map(str::to_string).collect())
}

/// `git grep` at a commit: (path, line number, text), first 500.
pub fn grep_at(dir: &Path, sha: &str, text: &str) -> GitResult<Vec<(String, u32, String)>> {
    let out = match git::run(dir, &["grep", "-n", "-I", "-i", "--full-name", "-F", "-e", text, sha, "--"]) {
        Ok(o) => o,
        Err(e) if e.0.is_empty() || e.0.starts_with("git grep failed") => String::new(), // no match: exit 1
        Err(e) => return Err(e),
    };
    let prefix = format!("{sha}:");
    Ok(out
        .lines()
        .take(500)
        .filter_map(|l| {
            let rest = l.strip_prefix(&prefix).unwrap_or(l);
            let mut it = rest.splitn(3, ':');
            Some((it.next()?.to_string(), it.next()?.parse().ok()?, it.next().unwrap_or("").to_string()))
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Full history (doc §Historique complet)
// ---------------------------------------------------------------------------------------------

/// Where `sha` is in the graph order (to load the history down to it), if reachable.
pub fn position_of(dir: &Path, sha: &str) -> Option<usize> {
    let mut args = vec!["rev-list", "--date-order"];
    args.extend(ALL_REFS);
    git::run(dir, &args).ok()?.lines().position(|l| l.starts_with(sha))
}

// ---------------------------------------------------------------------------------------------
// Stash (doc §Stash avancé) and copying several commits (doc §Branches 2)
// ---------------------------------------------------------------------------------------------

/// `stash push` of some files only (new ones included).
pub fn stash_files_args(paths: &[String], message: &str) -> Vec<String> {
    let mut v = vec!["stash".to_string(), "push".into(), "-u".into()];
    if !message.trim().is_empty() {
        v.extend(["-m".into(), message.trim().into()]);
    }
    v.push("--".into());
    v.extend(paths.iter().cloned());
    v
}

pub fn stash_branch_args(name: &str, stash: &str) -> Vec<String> {
    vec!["stash".into(), "branch".into(), name.into(), stash.into()]
}

/// `cherry-pick` of commits given oldest first, optionally with `-x` and `--no-commit`.
pub fn cherry_pick_args(oldest_first: &[String], mention: bool, no_commit: bool) -> Vec<String> {
    let mut v = vec!["cherry-pick".to_string()];
    if mention {
        v.push("-x".into());
    }
    if no_commit {
        v.push("--no-commit".into());
    }
    v.extend(oldest_first.iter().cloned());
    v
}

// ---------------------------------------------------------------------------------------------
// Bisect (doc §Trouver le commit qui a cassé)
// ---------------------------------------------------------------------------------------------

/// A bisect in progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bisect {
    /// Commits that can still be the first broken one (the others are greyed in the graph).
    pub candidates: HashSet<String>,
    /// Answers given so far.
    pub tested: usize,
    /// Roughly how many answers are left.
    pub steps_left: usize,
}

pub fn bisect_state(dir: &Path) -> Option<Bisect> {
    let git_dir = PathBuf::from(git::run(dir, &["rev-parse", "--absolute-git-dir"]).ok()?.trim());
    let log = std::fs::read_to_string(git_dir.join("BISECT_LOG")).ok()?;
    let tested = log.lines().filter(|l| l.starts_with("git bisect good") || l.starts_with("git bisect bad") || l.starts_with("git bisect skip")).count();
    let refs = git::run(dir, &["for-each-ref", "--format=%(refname)", "refs/bisect"]).unwrap_or_default();
    let goods: Vec<String> = refs.lines().filter(|r| r.starts_with("refs/bisect/good")).map(|r| format!("^{r}")).collect();
    let mut args = vec!["rev-list", "refs/bisect/bad"];
    args.extend(goods.iter().map(String::as_str));
    let candidates: HashSet<String> = git::run(dir, &args).unwrap_or_default().lines().map(str::to_string).collect();
    let n = candidates.len().max(1);
    let steps_left = (usize::BITS - n.leading_zeros()) as usize;
    Some(Bisect { candidates, tested, steps_left })
}

/// What a bisect answer led to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BisectOutcome {
    /// Another commit is checked out to test.
    Next,
    /// The first broken commit.
    Found(String),
}

pub fn parse_bisect(out: &str) -> BisectOutcome {
    for l in out.lines() {
        if let Some(sha) = l.strip_suffix(" is the first bad commit") {
            return BisectOutcome::Found(sha.trim().to_string());
        }
    }
    BisectOutcome::Next
}

// ---------------------------------------------------------------------------------------------
// Moving only my commits (`rebase --onto`, doc §Branches 3)
// ---------------------------------------------------------------------------------------------

pub fn onto_args(new_base: &str, old_base: &str, branch: &str) -> Vec<String> {
    vec!["rebase".into(), "--onto".into(), new_base.into(), old_base.into(), branch.into()]
}

/// How many commits `old_base..branch` holds (what would move).
pub fn count_between(dir: &Path, old_base: &str, branch: &str) -> usize {
    git::run(dir, &["rev-list", "--count", &format!("{old_base}..{branch}")]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// Submodules (doc §Dépôt et maintenance)
// ---------------------------------------------------------------------------------------------

pub fn submodule_add_args(url: &str, path: &str) -> Vec<String> {
    vec!["submodule".into(), "add".into(), url.into(), path.into()]
}

pub fn submodule_update_args(path: &str) -> Vec<String> {
    vec!["submodule".into(), "update".into(), "--init".into(), "--remote".into(), "--".into(), path.into()]
}

pub fn submodule_sync_args(path: &str) -> Vec<String> {
    vec!["submodule".into(), "sync".into(), "--".into(), path.into()]
}

/// The three steps git requires to remove a submodule, as one action.
pub fn submodule_remove_steps(dir: &Path, path: &str) -> Vec<Step> {
    let git_dir = git::run(dir, &["rev-parse", "--absolute-git-dir"]).map(|s| PathBuf::from(s.trim())).unwrap_or_else(|_| dir.join(".git"));
    vec![
        Step::Git(vec!["submodule".into(), "deinit".into(), "-f".into(), "--".into(), path.into()]),
        Step::Git(vec!["rm".into(), "-f".into(), "-q".into(), "--".into(), path.into()]),
        Step::DeleteDir(git_dir.join("modules").join(path)),
    ]
}

// ---------------------------------------------------------------------------------------------
// Clone options
// ---------------------------------------------------------------------------------------------

/// `git clone` with the options of the Clone form.
pub fn clone_args(url: &str, dest: &Path, branch: Option<&str>, recent_only: bool, submodules: bool) -> Vec<String> {
    let mut v = vec!["clone".to_string()];
    if let Some(b) = branch.filter(|b| !b.is_empty()) {
        v.extend(["--branch".into(), b.into()]);
    }
    if recent_only {
        v.extend(["--depth".into(), "1".into(), "--no-single-branch".into()]);
    }
    if submodules {
        v.push("--recurse-submodules".into());
    }
    v.push(url.into());
    v.push(dest.display().to_string());
    v
}

/// Branch names of a repository at `url` (one `ls-remote`).
pub fn remote_heads(dir: &Path, url: &str) -> GitResult<Vec<String>> {
    Ok(git::run(dir, &["ls-remote", "--heads", url])?.lines().filter_map(|l| l.split_whitespace().nth(1)?.strip_prefix("refs/heads/").map(str::to_string)).collect())
}

#[cfg(test)]
pub fn is_shallow(dir: &Path) -> bool {
    git::run(dir, &["rev-parse", "--is-shallow-repository"]).is_ok_and(|s| s.trim() == "true")
}

// ---------------------------------------------------------------------------------------------
// Cleanup (doc: "Delete all new files")
// ---------------------------------------------------------------------------------------------

/// Deleting every new file: saved first (Undo), then `clean`.
pub fn clean_steps(untracked: &[String]) -> Vec<Step> {
    vec![Step::Backup(untracked.to_vec()), Step::Git(vec!["clean".into(), "-f".into(), "-d".into(), "-q".into()])]
}

/// Ignored files and folders `clean -X` would delete, with the space they take.
pub fn ignored(dir: &Path) -> (Vec<String>, u64) {
    let list: Vec<String> = git::run(dir, &["clean", "-n", "-d", "-X"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("Would remove ").map(|p| p.trim_end_matches('/').to_string()))
        .collect();
    let size = list.iter().map(|p| size_of(&dir.join(p))).sum();
    (list, size)
}

fn size_of(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------------------------------
// Sparse-checkout
// ---------------------------------------------------------------------------------------------

/// Folders of HEAD up to two levels deep, for "Choose folders to download".
pub fn folders(dir: &Path) -> Vec<String> {
    git::run(dir, &["ls-tree", "-d", "-r", "--name-only", "HEAD"])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.matches('/').count() <= 1)
        .map(str::to_string)
        .collect()
}

/// The folders kept by sparse-checkout, `None` when everything is downloaded.
pub fn sparse_folders(dir: &Path) -> Option<Vec<String>> {
    let on = git::run(dir, &["config", "--bool", "core.sparseCheckout"]).is_ok_and(|s| s.trim() == "true");
    on.then(|| git::run(dir, &["sparse-checkout", "list"]).unwrap_or_default().lines().map(str::to_string).collect())
}

pub fn sparse_set_args(folders: &[String]) -> Vec<String> {
    let mut v = vec!["sparse-checkout".to_string(), "set".into(), "--cone".into()];
    v.extend(folders.iter().cloned());
    v
}

// ---------------------------------------------------------------------------------------------
// Patches
// ---------------------------------------------------------------------------------------------

/// Patch file text for commits given oldest first (`format-patch --stdout`).
pub fn format_patch(dir: &Path, oldest_first: &[String]) -> GitResult<String> {
    let mut out = String::new();
    for sha in oldest_first {
        out.push_str(&git::run(dir, &["format-patch", "--stdout", "-1", sha])?);
    }
    Ok(out)
}

/// What a patch file would change (`apply --stat --check`), or why it can't apply.
pub fn patch_preview(dir: &Path, file: &Path) -> Result<String, String> {
    let f = file.display().to_string();
    git::run(dir, &["apply", "--check", &f]).map_err(|e| e.0)?;
    git::run(dir, &["apply", "--stat", &f]).map_err(|e| e.0)
}

/// Applying a patch file: `am` keeps the author and message of a `format-patch` mail,
/// `apply` just changes the files.
pub fn patch_apply_args(file: &Path) -> Vec<String> {
    let mail = std::fs::read_to_string(file).is_ok_and(|t| t.starts_with("From ") && t.contains("\nSubject:"));
    let f = file.display().to_string();
    if mail { vec!["am".into(), "--3way".into(), f] } else { vec!["apply".into(), f] }
}

// ---------------------------------------------------------------------------------------------
// Signatures
// ---------------------------------------------------------------------------------------------

/// Signature of the last `n` commits: sha → (status letter `%G?`, signer).
pub fn signatures(dir: &Path, n: usize) -> HashMap<String, (char, String)> {
    let mut args = vec!["log".to_string(), format!("-n{n}"), "--format=%H%x1f%G?%x1f%GS".into()];
    args.extend(ALL_REFS.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    git::run(dir, &refs)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            let c = f.get(1)?.chars().next()?;
            (c != 'N').then(|| (f[0].to_string(), (c, f.get(2).unwrap_or(&"").to_string())))
        })
        .collect()
}

/// The words for a `%G?` letter.
pub fn signature_text(c: char) -> &'static str {
    match c {
        'G' => "Signed, verified",
        'U' => "Signed, key not trusted yet",
        'X' | 'Y' => "Signed with an expired key",
        'R' => "Signed with a revoked key",
        'E' => "Signed, but the key isn't known here",
        'B' => "Bad signature",
        _ => "Not signed",
    }
}

// ---------------------------------------------------------------------------------------------
// Maintenance
// ---------------------------------------------------------------------------------------------

/// Space the repository's objects take on disk, in bytes. Measured on the folder itself:
/// `count-objects` rounds small loose objects down to 0 KB on Windows.
pub fn repo_size(dir: &Path) -> u64 {
    let objects = git::run(dir, &["rev-parse", "--git-path", "objects"]).map(|s| PathBuf::from(s.trim())).unwrap_or_else(|_| dir.join(".git/objects"));
    let objects = if objects.is_absolute() { objects } else { dir.join(objects) };
    size_of(&objects)
}

/// Problems `fsck` finds, in its own words (empty: none).
pub fn check(dir: &Path) -> Result<Vec<String>, String> {
    match git::run(dir, &["fsck", "--no-progress", "--no-dangling"]) {
        Ok(out) => Ok(out.lines().map(str::to_string).filter(|l| !l.trim().is_empty()).collect()),
        Err(e) => Ok(e.0.lines().map(str::to_string).filter(|l| !l.trim().is_empty()).collect()),
    }
}

// ---------------------------------------------------------------------------------------------
// Commit options (doc §Options de commit)
// ---------------------------------------------------------------------------------------------

/// The commit panel's "More options", remembered per repository.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CommitOptions {
    pub open: bool,
    pub signoff: bool,
    pub sign: bool,
    pub no_verify: bool,
    /// "Name <email>" of co-authors.
    pub coauthors: Vec<String>,
}

impl CommitOptions {
    pub fn args(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.signoff {
            v.push("--signoff".into());
        }
        if self.sign {
            v.push("-S".into());
        }
        if self.no_verify {
            v.push("--no-verify".into());
        }
        v
    }

    /// The description with the `Co-authored-by` trailers added.
    pub fn description(&self, description: &str) -> String {
        if self.coauthors.is_empty() {
            return description.to_string();
        }
        let trailers: Vec<String> = self.coauthors.iter().map(|c| format!("Co-authored-by: {c}")).collect();
        let d = description.trim_end();
        if d.is_empty() { trailers.join("\n") } else { format!("{d}\n\n{}", trailers.join("\n")) }
    }
}

/// A signing key is configured (so "Sign this commit" can be offered).
pub fn can_sign(dir: &Path) -> bool {
    git::run(dir, &["config", "--get", "user.signingkey"]).is_ok_and(|s| !s.trim().is_empty())
}

/// Authors of the loaded history as "Name <email>", most frequent first.
pub fn authors(commits: &[git::Commit], me: Option<&str>) -> Vec<String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for c in commits {
        if Some(c.email.as_str()) != me {
            *count.entry(format!("{} <{}>", c.author, c.email)).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(a, _)| a).collect()
}
