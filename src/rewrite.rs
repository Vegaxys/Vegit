//! Editing history without an editor (design doc, "Rebase interactif").
//!
//! Every change (edit a message, combine with the commit below, remove, move, or a whole
//! "Edit history" plan) becomes the todo list of `git rebase -i`. Git asks for that list
//! through `GIT_SEQUENCE_EDITOR`; Vegit points it at a command that copies the prepared file
//! over git's (`vegit --sequence-editor <todo>` in the app, `cp` in the tests). New messages
//! go through `exec git commit --amend --only -F <file>`, so no editor ever opens.
//! Merge commits in the range are refused (no `--rebase-merges` in this version).

use std::path::{Path, PathBuf};

use crate::git::{self, GitResult};
use crate::safety::Step;

/// A commit of the range being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCommit {
    pub sha: String,
    pub summary: String,
    /// Full message, for "Edit message…".
    pub message: String,
    pub merge: bool,
}

/// What happens to one commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Keep,
    /// New message (from `PlanItem::message`).
    Reword,
    /// Combine with the commit below, keeping both messages (`squash`).
    Squash,
    /// Combine with the commit below, keeping the one below (`fixup`).
    Fixup,
    /// Combine with the commit below, with a new message.
    SquashNew,
    /// Remove this commit (`drop`).
    Drop,
    /// Stop on it to change files (`edit`).
    Edit,
}

impl Change {
    pub const ALL: [Change; 7] = [Change::Keep, Change::Reword, Change::Squash, Change::Fixup, Change::SquashNew, Change::Drop, Change::Edit];

    /// The words shown in the menus (doc rule 1: the word before the command).
    pub fn label(self) -> &'static str {
        match self {
            Change::Keep => "Keep",
            Change::Reword => "Edit message…",
            Change::Squash => "Combine with the commit below (keep both messages)",
            Change::Fixup => "Combine with the commit below (keep its message)",
            Change::SquashNew => "Combine with the commit below (new message)…",
            Change::Drop => "Remove this commit",
            Change::Edit => "Stop here to change files",
        }
    }

    /// The git term, for the tooltip.
    pub fn git_term(self) -> &'static str {
        match self {
            Change::Keep => "pick",
            Change::Reword => "reword",
            Change::Squash => "squash",
            Change::Fixup => "fixup",
            Change::SquashNew => "squash with a new message",
            Change::Drop => "drop",
            Change::Edit => "edit",
        }
    }

    pub fn needs_message(self) -> bool {
        matches!(self, Change::Reword | Change::SquashNew)
    }

    pub fn combines(self) -> bool {
        matches!(self, Change::Squash | Change::Fixup | Change::SquashNew)
    }
}

/// One line of a plan, oldest commit first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItem {
    pub sha: String,
    pub summary: String,
    pub change: Change,
    pub message: String,
}

/// The commits from `from` (included) up to HEAD, oldest first.
pub fn history_from(dir: &Path, from: &str) -> GitResult<Vec<HistoryCommit>> {
    if git::run(dir, &["merge-base", "--is-ancestor", from, "HEAD"]).is_err() {
        return Err(git::GitError("This commit is not on the current branch.".into()));
    }
    let range = match parent_of(dir, from) {
        Some(p) => format!("{p}..HEAD"),
        None => "HEAD".into(),
    };
    let out = git::run(dir, &["log", "--reverse", "--format=%H%x1f%P%x1f%s%x1f%B%x1e", &range])?;
    Ok(out
        .split('\x1e')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim_start_matches(['\n', '\r']).splitn(4, '\x1f').collect();
            (f.len() == 4).then(|| HistoryCommit {
                sha: f[0].to_string(),
                merge: f[1].split_whitespace().count() > 1,
                summary: f[2].to_string(),
                message: f[3].trim_end().to_string(),
            })
        })
        .collect())
}

pub fn parent_of(dir: &Path, sha: &str) -> Option<String> {
    git::run(dir, &["rev-parse", "-q", "--verify", &format!("{sha}^")]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// A plan that keeps everything, from a history.
pub fn plan_of(history: &[HistoryCommit]) -> Vec<PlanItem> {
    history.iter().map(|c| PlanItem { sha: c.sha.clone(), summary: c.summary.clone(), change: Change::Keep, message: c.message.clone() }).collect()
}

/// Why a plan can't run, in plain words.
pub fn check(plan: &[PlanItem], history: &[HistoryCommit]) -> Result<(), String> {
    if let Some(m) = history.iter().find(|c| c.merge) {
        return Err(format!(
            "{} is a merge commit. Editing history across merges isn't supported yet: start from a commit after it.",
            git::short(&m.sha)
        ));
    }
    let first_kept = plan.iter().find(|p| p.change != Change::Drop);
    if first_kept.is_some_and(|p| p.change.combines()) {
        return Err("The oldest commit has no commit below it to combine with.".into());
    }
    if plan.iter().any(|p| p.change.needs_message() && p.message.trim().is_empty()) {
        return Err("A new message is empty.".into());
    }
    if plan.iter().all(|p| p.change == Change::Drop) && !plan.is_empty() {
        return Err("Every commit would be removed: use Reset instead.".into());
    }
    Ok(())
}

/// How many changes a plan makes compared to `history` (for "Apply 4 changes").
pub fn change_count(plan: &[PlanItem], history: &[HistoryCommit]) -> usize {
    let changed = plan.iter().filter(|p| p.change != Change::Keep).count();
    let moved = plan.iter().zip(history).filter(|(p, h)| p.sha != h.sha).count();
    changed + moved.div_ceil(2)
}

fn quote(p: &Path) -> String {
    format!("\"{}\"", p.display().to_string().replace('\\', "/"))
}

/// The todo list for a plan; new messages are written next to it in `work`.
pub fn todo(plan: &[PlanItem], work: &Path) -> std::io::Result<String> {
    std::fs::create_dir_all(work)?;
    let mut out = String::new();
    for (i, p) in plan.iter().enumerate() {
        let amend = |out: &mut String| -> std::io::Result<()> {
            let file = work.join(format!("message-{i}.txt"));
            std::fs::write(&file, format!("{}\n", p.message.trim_end()))?;
            out.push_str(&format!("exec git commit --amend --only --allow-empty -q -F {}\n", quote(&file)));
            Ok(())
        };
        match p.change {
            Change::Keep => out.push_str(&format!("pick {}\n", p.sha)),
            Change::Reword => {
                out.push_str(&format!("pick {}\n", p.sha));
                amend(&mut out)?;
            }
            Change::Squash => out.push_str(&format!("squash {}\n", p.sha)),
            Change::Fixup => out.push_str(&format!("fixup {}\n", p.sha)),
            Change::SquashNew => {
                out.push_str(&format!("fixup {}\n", p.sha));
                amend(&mut out)?;
            }
            Change::Drop => out.push_str(&format!("drop {}\n", p.sha)),
            Change::Edit => out.push_str(&format!("edit {}\n", p.sha)),
        }
    }
    Ok(out)
}

/// Where a rebase's todo and messages are prepared.
pub fn work_dir(dir: &Path) -> PathBuf {
    let git_dir = git::run(dir, &["rev-parse", "--absolute-git-dir"]).map(|s| PathBuf::from(s.trim())).unwrap_or_else(|_| dir.join(".git"));
    git_dir.join("vegit-rebase")
}

/// The step that runs a plan: `rebase -i --autostash` from the parent of its oldest commit
/// (`--root` when that is the first commit), the todo handed over by `editor(todo_path)`.
pub fn plan_steps(dir: &Path, plan: &[PlanItem], editor: &dyn Fn(&Path) -> String) -> std::io::Result<Vec<Step>> {
    let work = work_dir(dir);
    let text = todo(plan, &work)?;
    let todo_path = work.join("todo");
    std::fs::write(&todo_path, text)?;
    let mut args = vec!["rebase".to_string(), "-i".into(), "--autostash".into()];
    // The base is below the oldest commit of the range, wherever a move put it in the plan:
    // the one whose parent is not in the plan.
    let shas: std::collections::HashSet<&str> = plan.iter().map(|p| p.sha.as_str()).collect();
    let base = plan.iter().find_map(|p| match parent_of(dir, &p.sha) {
        Some(parent) if shas.contains(parent.as_str()) => None,
        other => Some(other),
    });
    match base.flatten() {
        Some(base) => args.push(base),
        None => args.push("--root".into()),
    }
    Ok(vec![Step::GitEnv(args, vec![("GIT_SEQUENCE_EDITOR".into(), editor(&todo_path))])])
}

/// The editor command used by the app: Vegit itself in its hidden mode, copying `todo`.
pub fn vegit_editor(todo: &Path) -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string().replace('\\', "/")).unwrap_or_else(|_| "vegit".into());
    format!("\"{exe}\" --sequence-editor {}", quote(todo))
}

/// The hidden mode: `vegit --sequence-editor <prepared todo> <git's todo>` copies the first
/// over the second, then the process ends.
pub fn sequence_editor_main(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("--sequence-editor") {
        return None;
    }
    let (Some(src), Some(dst)) = (args.get(2), args.last()) else { return Some(2) };
    Some(if std::fs::copy(src, dst).is_ok() { 0 } else { 1 })
}

/// A plan for one direct action on `sha` (from the commit menu), with its own range.
pub fn single_plan(dir: &Path, sha: &str, change: Change, message: &str) -> GitResult<(Vec<HistoryCommit>, Vec<PlanItem>)> {
    // Combining or moving down also touches the commit below: start one commit earlier.
    let start = if change.combines() { parent_of(dir, sha).ok_or_else(|| git::GitError("This is the first commit: nothing below to combine with.".into()))? } else { sha.to_string() };
    let history = history_from(dir, &start)?;
    let mut plan = plan_of(&history);
    if let Some(p) = plan.iter_mut().find(|p| p.sha == sha) {
        p.change = change;
        if change.needs_message() {
            p.message = message.to_string();
        }
    }
    Ok((history, plan))
}

/// "Move up" (towards newer) or "Move down" (towards older) of one commit.
pub fn move_plan(dir: &Path, sha: &str, up: bool) -> GitResult<(Vec<HistoryCommit>, Vec<PlanItem>)> {
    let start = if up { sha.to_string() } else { parent_of(dir, sha).ok_or_else(|| git::GitError("This is the first commit.".into()))? };
    let history = history_from(dir, &start)?;
    let mut plan = plan_of(&history);
    let i = plan.iter().position(|p| p.sha == sha).unwrap_or(0);
    let j = if up { i + 1 } else { i.wrapping_sub(1) };
    if j >= plan.len() {
        return Err(git::GitError(if up { "It is already the newest commit." } else { "It is already the oldest one here." }.into()));
    }
    plan.swap(i, j);
    Ok((history, plan))
}

/// How many commits of the range are already on `upstream` (they would need a force push).
pub fn pushed_count(dir: &Path, oldest: &str, upstream: &str) -> usize {
    let range = match parent_of(dir, oldest) {
        Some(p) => format!("{p}..HEAD"),
        None => "HEAD".into(),
    };
    let all = git::run(dir, &["rev-list", "--count", &range]).ok().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
    let mine = git::run(dir, &["rev-list", "--count", &range, "--not", upstream]).ok().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(all);
    all.saturating_sub(mine)
}

/// Commits "Add to an earlier commit…" offers: not pushed yet, no merges, newest first.
pub fn fixup_candidates(dir: &Path) -> Vec<(String, String)> {
    let range = match git::run(dir, &["rev-parse", "-q", "--verify", "@{upstream}"]) {
        Ok(_) => "@{upstream}..HEAD".to_string(),
        Err(_) => "HEAD".to_string(),
    };
    git::run(dir, &["log", "-n", "30", "--first-parent", "--format=%H%x1f%P%x1f%s", &range])
        .unwrap_or_default()
        .lines()
        .map_while(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            // Stop at the first merge: the autosquash range can't cross it.
            (f.len() == 3 && f[1].split_whitespace().count() <= 1).then(|| (f[0].to_string(), f[2].to_string()))
        })
        .collect()
}

/// "Add to an earlier commit": the staged changes become a fixup commit, folded straight
/// into `target` by an autosquash rebase that accepts its own todo (`:`).
pub fn fixup_steps(dir: &Path, target: &str) -> Vec<Step> {
    let mut rebase = vec!["rebase".to_string(), "-i".into(), "--autosquash".into(), "--autostash".into()];
    match parent_of(dir, target) {
        Some(base) => rebase.push(base),
        None => rebase.push("--root".into()),
    }
    vec![
        Step::Git(vec!["commit".into(), "-q".into(), format!("--fixup={target}")]),
        Step::GitEnv(rebase, vec![("GIT_SEQUENCE_EDITOR".into(), ":".into())]),
    ]
}
