//! The commit history table: ref labels, the coloured lane graph, message, author, date.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use eframe::epaint::CubicBezierShape;

use crate::app::{Action, Dialog, RepoTab, Selection};
use crate::git::{self, RefKind};
use crate::theme::Palette;
use crate::util::{avatar, rel_time, abs_time};

const ROW_H: f32 = 30.0;
const LANE_W: f32 = 16.0;
const REFS_W: f32 = 175.0;
const AUTHOR_W: f32 = 150.0;
const DATE_W: f32 = 96.0;
const SHA_W: f32 = 74.0;

pub fn show(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let wip = tab.has_wip_row();
    let offset = usize::from(wip);
    let graph_w = ((tab.graph.width.max(1) as f32) * LANE_W + 18.0).clamp(60.0, 300.0);
    let search = tab.search.trim().to_lowercase();

    // Header
    let (hrect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(hrect, CornerRadius::same(10), pal.card);
    let cols = Columns::new(hrect, graph_w);
    let hfont = FontId::proportional(11.5);
    for (r, label) in [
        (cols.refs, "BRANCH / TAG"),
        (cols.graph, "GRAPH"),
        (cols.msg, "COMMIT MESSAGE"),
        (cols.author, "AUTHOR"),
        (cols.date, "WHEN"),
        (cols.sha, "ID"),
    ] {
        painter.text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, label, hfont.clone(), pal.muted);
    }
    ui.add_space(4.0);

    let total = tab.snap.commits.len() + offset;
    let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(row) = tab.scroll_to.take() {
        let viewport_h = ui.available_height();
        scroll = scroll.vertical_scroll_offset((row as f32 * ROW_H - viewport_h * 0.4).max(0.0));
    }

    scroll.show_rows(ui, ROW_H, total, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let first_top = ui.cursor().top();
        let width = ui.available_width();
        let y_of = |row: usize| first_top + (row as f32 - range.start as f32) * ROW_H + ROW_H * 0.5;

        // Pass 1: row backgrounds, text and interaction.
        let mut row_rects = Vec::new();
        for row in range.clone() {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            row_rects.push(rect);
            let cols = Columns::new(rect, graph_w);
            let painter = ui.painter();

            if wip && row == 0 {
                let selected = tab.selection == Selection::Wip;
                paint_row_bg(painter, rect, pal, selected, resp.hovered(), false);
                let n = tab.snap.status.staged.len() + tab.snap.status.unstaged.len();
                let label = if tab.snap.head_sha.is_none() {
                    "No commits yet: stage files and make the first commit".to_string()
                } else {
                    let s = tab.summary.trim();
                    if s.is_empty() { format!("Work in progress  ·  {n} changed file{}", plural(n)) } else { format!("{s}  ·  {n} file{}", plural(n)) }
                };
                painter
                    .with_clip_rect(cols.msg)
                    .text(pos2(cols.msg.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), pal.muted);
                if resp.clicked() {
                    acts.push(Action::Select(Selection::Wip));
                }
                continue;
            }

            let c = &tab.snap.commits[row - offset];
            let selected = tab.selection == Selection::Commit(c.sha.clone());
            let dim = !search.is_empty() && !matches_search(c, &search);
            paint_row_bg(painter, rect, pal, selected, resp.hovered(), row % 2 == 1);
            let text_col = if dim { pal.muted.gamma_multiply(0.6) } else { pal.text };

            // Ref labels
            if let Some(idxs) = tab.refs_by_sha.get(&c.sha) {
                let color = pal.lane(tab.graph.rows[row - offset].color);
                let rp = painter.with_clip_rect(cols.refs.shrink2(vec2(4.0, 0.0)));
                let mut x = cols.refs.left() + 8.0;
                let mut ordered: Vec<usize> = idxs.clone();
                ordered.sort_by_key(|&i| match tab.snap.refs[i].kind {
                    RefKind::Local => 0,
                    RefKind::Remote => 1,
                    RefKind::Tag => 2,
                });
                for (n, &i) in ordered.iter().enumerate() {
                    let r = &tab.snap.refs[i];
                    let is_head = r.kind == RefKind::Local && tab.snap.head_branch.as_deref() == Some(r.name.as_str());
                    let (icon, fill, fg) = match r.kind {
                        RefKind::Local if is_head => ("✔ ", color, pal.accent_text),
                        RefKind::Local => ("", color.gamma_multiply(0.22), pal.text),
                        RefKind::Remote => ("☁ ", pal.card, pal.muted),
                        RefKind::Tag => ("🏷 ", pal.card, pal.muted),
                    };
                    let text = format!("{icon}{}", r.name);
                    let galley = rp.layout_no_wrap(text, FontId::proportional(12.0), fg);
                    let pill = Rect::from_min_size(pos2(x, rect.center().y - 10.0), vec2(galley.size().x + 14.0, 20.0));
                    rp.rect_filled(pill, CornerRadius::same(10), fill);
                    if fill == pal.card {
                        rp.rect_stroke(pill, CornerRadius::same(10), Stroke::new(1.0, color), StrokeKind::Inside);
                    }
                    rp.galley(pos2(pill.left() + 7.0, pill.center().y - galley.size().y / 2.0), galley, fg);
                    x = pill.right() + 4.0;
                    if x > cols.refs.right() - 20.0 && n + 1 < ordered.len() {
                        rp.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, format!("+{}", ordered.len() - n - 1), FontId::proportional(11.0), pal.muted);
                        break;
                    }
                }
            }

            // Message: summary in the main text colour, then the description in the second shade.
            let mp = painter.with_clip_rect(cols.msg.shrink2(vec2(6.0, 0.0)));
            let summary = mp.layout_no_wrap(c.summary.clone(), FontId::proportional(13.5), text_col);
            let sw = summary.size().x;
            mp.galley(pos2(cols.msg.left() + 10.0, rect.center().y - summary.size().y / 2.0), summary, text_col);
            if !c.body.is_empty() {
                let body_col = if dim { pal.muted.gamma_multiply(0.5) } else { pal.muted };
                mp.text(pos2(cols.msg.left() + 22.0 + sw, rect.center().y), Align2::LEFT_CENTER, &c.body, FontId::proportional(13.0), body_col);
            }
            // Author
            let ap = painter.with_clip_rect(cols.author);
            avatar(&ap, pos2(cols.author.left() + 20.0, rect.center().y), 10.0, &c.author, &c.email);
            ap.text(pos2(cols.author.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, &c.author, FontId::proportional(12.5), if dim { text_col } else { pal.text2() });
            // Date and id
            painter.text(pos2(cols.date.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, rel_time(c.time), FontId::proportional(12.5), pal.muted);
            painter.text(pos2(cols.sha.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, git::short(&c.sha), FontId::monospace(12.0), pal.muted);

            let resp = resp.on_hover_text_at_pointer(format!("{}\n{} · {}", c.summary, c.author, abs_time(c.time)));
            if resp.clicked() {
                acts.push(Action::Select(Selection::Commit(c.sha.clone())));
            }
            commit_menu(&resp, tab, &c.sha, acts);
        }

        // Pass 2: the graph, painted over the row backgrounds so lines run uninterrupted.
        let painter = ui.painter().clone();
        let left = row_rects.first().map_or(ui.min_rect().left(), |r| r.left()) + REFS_W;
        let x_of = |col: usize| left + 14.0 + col as f32 * LANE_W;
        let start = range.start.saturating_sub(1);
        for row in start..range.end {
            if wip && row == 0 {
                // Dashed link from the WIP row down to the checked-out commit.
                if let Some(head) = tab.snap.head_sha.as_ref().and_then(|h| tab.commit_index(h)) {
                    let col = tab.graph.rows[head].col;
                    let (x, y0, y1) = (x_of(col), y_of(0), y_of(head + 1));
                    let mut y = y0;
                    while y < y1 {
                        painter.line_segment([pos2(x, y), pos2(x, (y + 4.0).min(y1))], Stroke::new(2.0, pal.muted));
                        y += 8.0;
                    }
                    wip_node(&painter, pos2(x, y0), pal);
                } else {
                    wip_node(&painter, pos2(x_of(0), y_of(0)), pal);
                }
                continue;
            }
            let g = &tab.graph.rows[row - offset];
            for e in &g.edges {
                let a = pos2(x_of(e.from_col), y_of(row));
                let b = pos2(x_of(e.to_col), y_of(row + 1));
                let stroke = Stroke::new(2.0, pal.lane(e.color));
                if e.from_col == e.to_col {
                    painter.line_segment([a, b], stroke);
                } else {
                    curve(&painter, a, b, stroke);
                }
            }
        }
        for row in range.clone() {
            if wip && row == 0 {
                continue;
            }
            let g = &tab.graph.rows[row - offset];
            let c = pos2(x_of(g.col), y_of(row));
            let color = pal.lane(g.color);
            let is_head = tab.snap.head_sha.as_deref() == Some(tab.snap.commits[row - offset].sha.as_str());
            if g.is_merge {
                painter.circle_filled(c, 4.5, color);
            } else {
                painter.circle_filled(c, 7.0, color);
                painter.circle_filled(c, 3.5, pal.panel);
            }
            if is_head {
                painter.circle_stroke(c, 10.0, Stroke::new(2.0, color));
            }
        }
    });
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn matches_search(c: &git::Commit, q: &str) -> bool {
    c.summary.to_lowercase().contains(q) || c.author.to_lowercase().contains(q) || c.sha.starts_with(q)
}

struct Columns {
    refs: Rect,
    graph: Rect,
    msg: Rect,
    author: Rect,
    date: Rect,
    sha: Rect,
}

impl Columns {
    fn new(rect: Rect, graph_w: f32) -> Self {
        let y = rect.y_range();
        let mut x = rect.left();
        let mut take = |w: f32| {
            let r = Rect::from_x_y_ranges(x..=x + w, y);
            x += w;
            r
        };
        let refs = take(REFS_W);
        let graph = take(graph_w);
        let msg_w = (rect.width() - REFS_W - graph_w - AUTHOR_W - DATE_W - SHA_W).max(120.0);
        let msg = take(msg_w);
        let author = take(AUTHOR_W);
        let date = take(DATE_W);
        let sha = take(SHA_W);
        Self { refs, graph, msg, author, date, sha }
    }
}

fn paint_row_bg(p: &egui::Painter, rect: Rect, pal: &Palette, selected: bool, hovered: bool, alt: bool) {
    let fill = if selected {
        pal.selected
    } else if hovered {
        pal.hover
    } else if alt {
        pal.row_alt
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        p.rect_filled(rect.shrink2(vec2(0.0, 1.0)), CornerRadius::same(7), fill);
    }
    if selected {
        let bar = Rect::from_min_size(rect.left_top() + vec2(0.0, 5.0), vec2(3.0, rect.height() - 10.0));
        p.rect_filled(bar, CornerRadius::same(2), pal.accent);
    }
}

fn curve(p: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let mid = (a.y + b.y) / 2.0;
    let shape = CubicBezierShape::from_points_stroke([a, pos2(a.x, mid), pos2(b.x, mid), b], false, Color32::TRANSPARENT, stroke);
    p.add(shape);
}

fn wip_node(p: &egui::Painter, c: Pos2, pal: &Palette) {
    p.circle_filled(c, 7.0, pal.panel);
    p.circle_stroke(c, 7.0, Stroke::new(2.0, pal.muted));
    p.line_segment([c - Vec2::X * 3.0, c + Vec2::X * 3.0], Stroke::new(1.5, pal.muted));
}

/// Right-click menu shared by the graph rows.
pub fn commit_menu(resp: &egui::Response, tab: &RepoTab, sha: &str, acts: &mut Vec<Action>) {
    let sha = sha.to_string();
    let short = git::short(&sha).to_string();
    let is_head = tab.snap.head_sha.as_deref() == Some(sha.as_str());
    resp.context_menu(|ui| {
        ui.label(egui::RichText::new(format!("Commit {short}")).weak());
        if ui.button("Create branch here…").clicked() {
            acts.push(Action::Dialog(Dialog::NewBranch { at: Some(sha.clone()), name: String::new(), checkout: true }));
            ui.close();
        }
        if ui.button("Create tag here…").clicked() {
            acts.push(Action::Dialog(Dialog::NewTag { at: sha.clone(), name: String::new(), message: String::new() }));
            ui.close();
        }
        ui.separator();
        if ui.button("Check out this commit").clicked() {
            acts.push(Action::Dialog(Dialog::Confirm {
                title: "Check out this commit?".into(),
                body: "You will be on this commit without a branch (\"detached\"). Create a branch if you want to keep new work.".into(),
                confirm: "Check out".into(),
                then: Box::new(Action::Git { label: format!("Checked out {short}"), args: vec!["switch".into(), "--detach".into(), sha.clone()], background: false }),
            }));
            ui.close();
        }
        if !is_head && ui.button("Cherry-pick onto current branch").clicked() {
            acts.push(Action::Git { label: format!("Cherry-picked {short}"), args: vec!["cherry-pick".into(), sha.clone()], background: false });
            ui.close();
        }
        if ui.button("Revert this commit").clicked() {
            acts.push(Action::Git { label: format!("Reverted {short}"), args: vec!["revert".into(), "--no-edit".into(), sha.clone()], background: false });
            ui.close();
        }
        ui.menu_button("Reset current branch to here", |ui| {
            if ui.button("Soft: keep all changes staged").clicked() {
                acts.push(Action::Git { label: "Reset (soft)".into(), args: vec!["reset".into(), "--soft".into(), sha.clone()], background: false });
                ui.close();
            }
            if ui.button("Mixed: keep changes, unstaged").clicked() {
                acts.push(Action::Git { label: "Reset (mixed)".into(), args: vec!["reset".into(), "--mixed".into(), sha.clone()], background: false });
                ui.close();
            }
            if ui.button("Hard: throw away all changes").clicked() {
                acts.push(Action::Dialog(Dialog::Confirm {
                    title: "Throw away changes?".into(),
                    body: format!("The current branch will point at {short} and every uncommitted change will be lost. This can't be undone."),
                    confirm: "Reset hard".into(),
                    then: Box::new(Action::Git { label: "Reset (hard)".into(), args: vec!["reset".into(), "--hard".into(), sha.clone()], background: false }),
                }));
                ui.close();
            }
        });
        ui.separator();
        if ui.button("Copy commit id").clicked() {
            acts.push(Action::Copy(sha.clone()));
            ui.close();
        }
    });
}
