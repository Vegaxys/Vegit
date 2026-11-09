//! Lane layout for the commit graph.
//!
//! Commits arrive newest first. Each lane remembers which commit it is waiting for;
//! a commit takes the first lane waiting for it, and its first parent inherits that lane
//! so a branch stays on one straight column. Lane indices are reused but never
//! compacted, which keeps lines from shifting sideways between rows.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::git::Commit;

/// One line segment between the centre of row `r` and the centre of row `r + 1`.
#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub from_col: usize,
    pub to_col: usize,
    pub color: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub col: usize,
    pub color: usize,
    /// Segments leaving this row toward the next one.
    pub edges: Vec<Edge>,
    pub is_merge: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    pub rows: Vec<Row>,
    pub width: usize,
}

/// "Pin the current branch to the left" setting, read by the worker when it lays out graphs.
pub static PIN_HEAD: AtomicBool = AtomicBool::new(true);

/// Lays out the graph, with the checked-out branch on the first column when the pin
/// setting is on and `head` is given.
pub fn layout_for(commits: &[Commit], head: Option<&str>) -> GraphLayout {
    let pin = if PIN_HEAD.load(Ordering::Relaxed) { head } else { None };
    layout(commits, pin)
}

/// `pin`: a commit whose first-parent line keeps lane 0 for itself; no other branch may use
/// that lane, so the current branch reads as a straight line on the left.
pub fn layout(commits: &[Commit], pin: Option<&str>) -> GraphLayout {
    let index: HashMap<&str, usize> = commits.iter().enumerate().map(|(i, c)| (c.sha.as_str(), i)).collect();
    let pin = pin.filter(|p| index.contains_key(p));

    // lanes[i] = sha the lane is waiting for, with the lane's colour, and whether a stash
    // opened it: a stash never takes its base commit's line from the real branch.
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut next_color = 0usize;
    let first_free = usize::from(pin.is_some());
    if let Some(p) = pin {
        lanes.push(Some((p.to_string(), 0, false)));
        next_color = 1;
    }
    let mut rows: Vec<Row> = Vec::with_capacity(commits.len());
    // For each row: lanes after the row was processed, and which lanes the row itself started.
    let mut after_states: Vec<Vec<Option<Lane>>> = Vec::with_capacity(commits.len());
    let mut started: Vec<Vec<usize>> = Vec::with_capacity(commits.len());
    let mut joins: Vec<Vec<(usize, usize)>> = Vec::with_capacity(commits.len()); // (lane, color) of already-open parents
    let mut width = 1usize;

    for c in commits {
        let is_stash = c.stash.is_some();
        // A real branch's lane first; a stash's lane only when nothing else waits here.
        let waiting = |l: &Option<Lane>, stash_ok: bool| l.as_ref().is_some_and(|(s, _, st)| s == &c.sha && (stash_ok || !st));
        let col = match lanes.iter().position(|l| waiting(l, false)).or_else(|| lanes.iter().position(|l| waiting(l, true))) {
            Some(i) => i,
            None => {
                let i = free_lane(&mut lanes, first_free);
                lanes[i] = Some((c.sha.clone(), next_color, is_stash));
                next_color += 1;
                i
            }
        };
        let color = lanes[col].as_ref().map(|(_, k, _)| *k).unwrap_or(0);
        // Every other lane waiting for this commit ends here (branch merged into it).
        for l in lanes.iter_mut() {
            if l.as_ref().is_some_and(|(s, _, _)| s == &c.sha) {
                *l = None;
            }
        }

        let mut row_started = Vec::new();
        let mut row_joins = Vec::new();
        // Only parents that are in the loaded history get a line; older ones are cut off.
        let parents: Vec<&String> = c.parents.iter().filter(|p| index.contains_key(p.as_str())).collect();
        for (pi, p) in parents.iter().enumerate() {
            if let Some(existing) = lanes.iter().position(|l| l.as_ref().is_some_and(|(s, _, _)| s == *p)) {
                // A first parent already waited for further right: keep this lane going
                // straight instead, and let the right-hand lane merge into it later.
                // That keeps the main line on one column with one colour. A lane a stash
                // opened never pulls a real commit sideways: the stash is the one that joins.
                let stash_lane = lanes[existing].as_ref().is_some_and(|(_, _, st)| *st);
                if pi > 0 || is_stash || (existing < col && !stash_lane) {
                    let k = lanes[existing].as_ref().map(|(_, k, _)| *k).unwrap_or(0);
                    row_joins.push((existing, if pi == 0 { color } else { k }));
                    continue;
                }
            }
            let (lane, lane_color) = if pi == 0 {
                (col, color)
            } else {
                let i = free_lane(&mut lanes, first_free);
                next_color += 1;
                (i, next_color - 1)
            };
            lanes[lane] = Some(((*p).clone(), lane_color, is_stash));
            row_started.push(lane);
        }

        width = width.max(lanes.len());
        rows.push(Row { col, color, edges: Vec::new(), is_merge: parents.len() > 1 });
        after_states.push(lanes.clone());
        started.push(row_started);
        joins.push(row_joins);
    }

    // Second pass: knowing every row's column, turn lane states into segments.
    for r in 0..rows.len() {
        let next_sha = commits.get(r + 1).map(|c| c.sha.as_str());
        let next_col = rows.get(r + 1).map(|row| row.col);
        let col = rows[r].col;
        let mut edges = Vec::new();
        for (i, lane) in after_states[r].iter().enumerate() {
            let Some((sha, color, _)) = lane else { continue };
            let to_col = if Some(sha.as_str()) == next_sha { next_col.unwrap_or(i) } else { i };
            let from_col = if started[r].contains(&i) { col } else { i };
            edges.push(Edge { from_col, to_col, color: *color });
        }
        for &(lane, color) in &joins[r] {
            let sha = after_states[r][lane].as_ref().map(|(s, _, _)| s.as_str());
            let to_col = if sha == next_sha { next_col.unwrap_or(lane) } else { lane };
            edges.push(Edge { from_col: col, to_col, color });
        }
        rows[r].edges = edges;
    }

    GraphLayout { rows, width }
}

/// What a lane waits for: (commit, colour, opened by a stash).
type Lane = (String, usize, bool);

fn free_lane(lanes: &mut Vec<Option<Lane>>, first: usize) -> usize {
    while lanes.len() < first {
        lanes.push(None);
    }
    match lanes.iter().skip(first).position(Option::is_none) {
        Some(i) => i + first,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}
