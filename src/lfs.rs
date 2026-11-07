//! Git LFS for Unity projects (design doc, "Git LFS pour les projets Unity"): which files
//! are stored with LFS, whether LFS is installed, the Unity preset, locks, and reading the
//! real content of a pointer file for previews.
//!
//! Locks talk to the server, so they are read on a network thread; everything else is local.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::git::{self, GitResult};

/// Staging a file bigger than this suggests storing its kind with LFS (doc §LFS 3).
pub const SUGGEST_BYTES: u64 = 50 * 1024 * 1024;
/// GitHub's hard limit for one file, quoted in the suggestion.
pub const HOST_LIMIT_MB: u64 = 100;

/// Patterns the Unity preset stores with LFS, and those it makes lockable (doc §LFS 4).
pub const UNITY_PATTERNS: &[&str] = &["*.psd", "*.fbx", "*.wav", "*.mp4", "*.tga", "*.exr", "*.unitypackage"];
pub const UNITY_LOCKABLE: &[&str] = &["*.unity", "*.prefab"];

/// What the window shows about LFS for one repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LfsInfo {
    /// `.gitattributes` mentions `filter=lfs`.
    pub used: bool,
    /// `git lfs` answers.
    pub installed: bool,
    /// Changed files that LFS stores.
    pub tracked: HashSet<String>,
    /// A Unity project (it has `ProjectSettings/`).
    pub unity: bool,
}

/// One lock on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    pub id: String,
    pub path: String,
    pub owner: String,
    /// When, as the server wrote it (RFC 3339).
    pub locked_at: String,
    pub mine: bool,
}

pub fn uses_lfs(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(".gitattributes")).is_ok_and(|t| t.contains("filter=lfs"))
}

pub fn installed(dir: &Path) -> bool {
    git::run(dir, &["lfs", "version"]).is_ok()
}

pub fn info(dir: &Path, changed: &[String]) -> LfsInfo {
    let used = uses_lfs(dir);
    LfsInfo {
        used,
        installed: !used || installed(dir),
        tracked: if used { tracked(dir, changed) } else { HashSet::new() },
        unity: dir.join("ProjectSettings").is_dir(),
    }
}

/// Among `paths`, those whose `filter` attribute is `lfs`.
pub fn tracked(dir: &Path, paths: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for chunk in paths.chunks(200) {
        let mut args = vec!["check-attr", "filter", "--"];
        args.extend(chunk.iter().map(String::as_str));
        for line in git::run(dir, &args).unwrap_or_default().lines() {
            if let Some(p) = line.strip_suffix(": filter: lfs") {
                out.insert(p.to_string());
            }
        }
    }
    out
}

/// `git lfs track [--lockable] <pattern>`: adds the pattern to `.gitattributes`.
pub fn track_args(pattern: &str, lockable: bool) -> Vec<String> {
    let mut v = vec!["lfs".to_string(), "track".into()];
    if lockable {
        v.push("--lockable".into());
    }
    v.push(pattern.into());
    v
}

/// The pattern the size suggestion proposes: `*.psd` for `Art/hero.psd`, else the file itself.
pub fn pattern_for(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => format!("*.{ext}"),
        _ => path.to_string(),
    }
}

pub fn lock_args(path: &str) -> Vec<String> {
    vec!["lfs".into(), "lock".into(), path.into()]
}

pub fn unlock_args(path: &str, force: bool) -> Vec<String> {
    let mut v = vec!["lfs".to_string(), "unlock".into()];
    if force {
        v.push("--force".into());
    }
    v.push(path.into());
    v
}

/// The locks on the server, telling apart mine (`--verify`) when the server supports it.
pub fn locks(dir: &Path) -> GitResult<Vec<Lock>> {
    match git::run(dir, &["lfs", "locks", "--verify", "--json"]) {
        Ok(out) => Ok(parse_verified(&out)),
        Err(_) => Ok(parse_list(&git::run(dir, &["lfs", "locks", "--json"])?, false)),
    }
}

/// `{"ours": [...], "theirs": [...]}` from `git lfs locks --verify --json`.
pub fn parse_verified(json: &str) -> Vec<Lock> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut out = Vec::new();
    for (key, mine) in [("ours", true), ("theirs", false)] {
        if let Some(arr) = v.get(key) {
            out.extend(parse_list(&arr.to_string(), mine));
        }
    }
    out
}

/// A JSON array of locks.
pub fn parse_list(json: &str, mine: bool) -> Vec<Lock> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    items
        .iter()
        .map(|l| Lock {
            id: l.get("id").map(json_text).unwrap_or_default(),
            path: l.get("path").map(json_text).unwrap_or_default(),
            owner: l.pointer("/owner/name").map(json_text).unwrap_or_default(),
            locked_at: l.get("locked_at").map(json_text).unwrap_or_default(),
            mine,
        })
        .collect()
}

fn json_text(v: &serde_json::Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())
}

/// Locks by path, for the file lists.
pub fn by_path(locks: &[Lock]) -> HashMap<String, Lock> {
    locks.iter().map(|l| (l.path.clone(), l.clone())).collect()
}

/// The content is an LFS pointer rather than the real file.
pub fn is_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs.github.com/spec/v1")
}

/// The real content behind a pointer (`git lfs smudge`), when LFS has it.
pub fn smudge(dir: &Path, pointer: &[u8]) -> Option<Vec<u8>> {
    git::run_bytes_with_input(dir, &["lfs", "smudge"], pointer)
}

/// "12.4 MB" style sizes.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}

/// "since Tuesday" style age of a lock, from its RFC 3339 date.
pub fn since(locked_at: &str, now: i64) -> String {
    let Some(t) = parse_rfc3339(locked_at) else { return String::new() };
    let days = (now - t) / 86_400;
    match days {
        d if d < 1 => "since today".into(),
        1 => "since yesterday".into(),
        d if d < 7 => {
            // 1970-01-01 was a Thursday.
            const NAMES: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
            format!("since {}", NAMES[((t / 86_400) % 7) as usize])
        }
        d => format!("for {d} days"),
    }
}

/// Seconds since 1970 of `YYYY-MM-DDTHH:MM:SS…` (UTC; the offset is ignored).
fn parse_rfc3339(s: &str) -> Option<i64> {
    let (y, m, d) = (s.get(0..4)?.parse::<i64>().ok()?, s.get(5..7)?.parse::<i64>().ok()?, s.get(8..10)?.parse::<i64>().ok()?);
    let (hh, mm, ss) = (s.get(11..13)?.parse::<i64>().ok()?, s.get(14..16)?.parse::<i64>().ok()?, s.get(17..19)?.parse::<i64>().ok()?);
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}
