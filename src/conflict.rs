//! Resolving a conflict block by block (design doc, "Résoudre un conflit bloc par bloc").
//!
//! A conflicted file is read as plain text and conflict blocks, with the base version when
//! git wrote it (`zdiff3` style, which Vegit asks for on the operations it runs). Choosing a
//! side rewrites only that block; line endings are kept as they are in the file.
//! The titles use branch names, not "ours" / "theirs": during a rebase git swaps them, and
//! naming the branches avoids the confusion every client reproduces (doc, "Piège à gérer").

use std::path::Path;

use crate::git::{self, PendingOp};

/// One conflict: the two versions, and the original when known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub ours: String,
    pub base: Option<String>,
    pub theirs: String,
    /// The labels after the markers (`HEAD`, `feature`, `abc123 (Fix…)`).
    pub ours_label: String,
    pub theirs_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Conflict(Block),
}

/// How a block is resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Ours,
    Theirs,
    /// Ours, then theirs.
    Both,
    /// Text written by the person ("Edit…").
    Text(String),
}

fn marker<'a>(line: &'a str, m: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(m)?;
    let rest = rest.trim_end_matches(['\n', '\r']);
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix(' ')
    }
}

/// Splits a file into text and conflicts. An unfinished block (no closing marker) stays text.
pub fn parse(text: &str) -> Vec<Segment> {
    enum State {
        Text,
        Ours,
        Base,
        Theirs,
    }
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut raw = String::new();
    let mut block = Block { ours: String::new(), base: None, theirs: String::new(), ours_label: String::new(), theirs_label: String::new() };
    let mut state = State::Text;
    for line in text.split_inclusive('\n') {
        match state {
            State::Text => {
                if let Some(label) = marker(line, "<<<<<<<") {
                    block = Block { ours: String::new(), base: None, theirs: String::new(), ours_label: label.to_string(), theirs_label: String::new() };
                    raw = line.to_string();
                    state = State::Ours;
                } else {
                    plain.push_str(line);
                }
            }
            State::Ours | State::Base => {
                raw.push_str(line);
                if marker(line, "|||||||").is_some() {
                    block.base = Some(String::new());
                    state = State::Base;
                } else if marker(line, "=======") == Some("") {
                    state = State::Theirs;
                } else if matches!(state, State::Ours) {
                    block.ours.push_str(line);
                } else if let Some(b) = block.base.as_mut() {
                    b.push_str(line);
                }
            }
            State::Theirs => {
                raw.push_str(line);
                if let Some(label) = marker(line, ">>>>>>>") {
                    block.theirs_label = label.to_string();
                    if !plain.is_empty() {
                        out.push(Segment::Text(std::mem::take(&mut plain)));
                    }
                    out.push(Segment::Conflict(block.clone()));
                    raw.clear();
                    state = State::Text;
                } else {
                    block.theirs.push_str(line);
                }
            }
        }
    }
    plain.push_str(&raw);
    if !plain.is_empty() {
        out.push(Segment::Text(plain));
    }
    out
}

pub fn count(segs: &[Segment]) -> usize {
    segs.iter().filter(|s| matches!(s, Segment::Conflict(_))).count()
}

/// The conflict blocks, in order.
pub fn blocks(segs: &[Segment]) -> Vec<&Block> {
    segs.iter().filter_map(|s| if let Segment::Conflict(b) = s { Some(b) } else { None }).collect()
}

/// The file with block `index` resolved by `choice`; other blocks keep their markers.
pub fn resolve(segs: &[Segment], index: usize, choice: &Choice) -> String {
    let mut out = String::new();
    let mut k = 0;
    for s in segs {
        match s {
            Segment::Text(t) => out.push_str(t),
            Segment::Conflict(b) if k == index => {
                k += 1;
                match choice {
                    Choice::Ours => out.push_str(&b.ours),
                    Choice::Theirs => out.push_str(&b.theirs),
                    Choice::Both => {
                        out.push_str(&b.ours);
                        if !b.ours.is_empty() && !b.ours.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str(&b.theirs);
                    }
                    Choice::Text(t) => {
                        out.push_str(t);
                        if !t.is_empty() && !t.ends_with('\n') {
                            out.push_str(if b.ours.ends_with("\r\n") { "\r\n" } else { "\n" });
                        }
                    }
                }
            }
            Segment::Conflict(b) => {
                k += 1;
                out.push_str(&render_block(b));
            }
        }
    }
    out
}

fn render_block(b: &Block) -> String {
    let nl = if b.ours.ends_with("\r\n") || b.theirs.ends_with("\r\n") { "\r\n" } else { "\n" };
    let label = |m: &str, l: &str| if l.is_empty() { format!("{m}{nl}") } else { format!("{m} {l}{nl}") };
    let mut s = label("<<<<<<<", &b.ours_label);
    s.push_str(&b.ours);
    if let Some(base) = &b.base {
        s.push_str(&label("|||||||", "base"));
        s.push_str(base);
    }
    s.push_str(&format!("======={nl}"));
    s.push_str(&b.theirs);
    s.push_str(&label(">>>>>>>", &b.theirs_label));
    s
}

/// Branch names for the two sides: (the side git calls "ours", the side it calls "theirs").
pub fn side_names(dir: &Path, pending: Option<PendingOp>, ours_label: &str, theirs_label: &str) -> (String, String) {
    let git_dir = git::run(dir, &["rev-parse", "--absolute-git-dir"]).map(|s| std::path::PathBuf::from(s.trim())).unwrap_or_default();
    let read = |p: &str| std::fs::read_to_string(git_dir.join(p)).ok().map(|s| s.trim().to_string());
    let current = git::run(dir, &["symbolic-ref", "--short", "-q", "HEAD"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    match pending {
        Some(PendingOp::Rebase) => {
            // During a rebase, "ours" is the branch being replayed onto, "theirs" your commit.
            let mine = read("rebase-merge/head-name").or_else(|| read("rebase-apply/head-name")).map(|h| h.trim_start_matches("refs/heads/").to_string());
            let onto = read("rebase-merge/onto").or_else(|| read("rebase-apply/onto")).map(|sha| {
                git::run(dir, &["name-rev", "--name-only", "--refs=refs/heads/*", &sha])
                    .ok()
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty() && n != "undefined")
                    .unwrap_or_else(|| git::short(&sha).to_string())
            });
            (onto.unwrap_or_else(|| ours_label.to_string()), mine.unwrap_or_else(|| theirs_label.to_string()))
        }
        _ => {
            let theirs = read("MERGE_MSG")
                .and_then(|m| m.lines().next().and_then(|l| l.strip_prefix("Merge branch '").and_then(|r| r.split('\'').next()).map(str::to_string)))
                .unwrap_or_else(|| theirs_label.to_string());
            (current.unwrap_or_else(|| ours_label.to_string()), theirs)
        }
    }
}
