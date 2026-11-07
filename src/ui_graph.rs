//! The commit history table: ref labels, the coloured lane graph, message, author, date.
//!
//! Columns can be resized from the header and the table scrolls sideways when they are
//! wider than the tile; the header follows the horizontal scroll of the rows. Each
//! column's painting is clipped to the column, so a very wide graph never spills out.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use eframe::epaint::CubicBezierShape;

use crate::app::{Action, Dialog, PullMode, RepoTab, Selection};
use crate::forge;
use crate::git::{self, RefKind};
use crate::icons::{self, Icon};
use crate::theme::Palette;
use crate::util::{abs_time, avatar, short_date_time};

const ROW_H: f32 = 30.0;
const HEAD_H: f32 = 26.0;
const LANE_W: f32 = 16.0;
const MIN_COL: [f32; 6] = [70.0, 40.0, 160.0, 70.0, 118.0, 50.0];
const LABELS: [&str; 6] = ["BRANCH / TAG", "GRAPH", "COMMIT MESSAGE", "AUTHOR", "DATE / TIME", "SHA"];
const MENU_NAMES: [&str; 6] = ["Branch / Tag", "Graph", "Commit message", "Author", "Date / Time", "Sha"];
/// Room kept at the right of the header for the column settings button.
const GEAR_W: f32 = 28.0;
/// Room an icon takes inside a ref label: the icon itself and the gap after it.
const PILL_ICON_W: f32 = 15.0;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    pal: &Palette,
    widths: &mut [f32; 6],
    visible: &mut [bool; 6],
    compact: &mut bool,
    show_desc: bool,
    acts: &mut Vec<Action>,
) {
    let wip = tab.has_wip_row();
    let offset = usize::from(wip);
    let search = tab.search.trim().to_lowercase();
    let author_filter = tab.author_filter.clone();
    let lane_w = if *compact { 10.0 } else { LANE_W };
    if !visible.iter().any(|v| *v) {
        visible[2] = true;
    }
    for i in 0..6 {
        widths[i] = widths[i].max(MIN_COL[i]);
    }

    // Effective widths: hidden columns take no room, and when the columns are narrower than
    // the tile the message column (or the last shown one) takes the slack.
    let avail = ui.available_width();
    let mut eff = [0.0f32; 6];
    for i in 0..6 {
        if visible[i] {
            eff[i] = widths[i];
        }
    }
    let total: f32 = eff.iter().sum();
    if total < avail {
        let slack_col = if visible[2] { 2 } else { (0..6).rev().find(|&i| visible[i]).unwrap_or(2) };
        eff[slack_col] += avail - total;
    }
    let content_w: f32 = eff.iter().sum();

    // Header, shifted by the rows' horizontal scroll of the previous frame.
    let (hrect, _) = ui.allocate_exact_size(vec2(avail, HEAD_H), Sense::hover());
    let hp = ui.painter().with_clip_rect(hrect);
    hp.rect_filled(hrect, CornerRadius::same(10), pal.card);
    let labels_rect = hrect.with_max_x(hrect.right() - GEAR_W);
    let hp = hp.with_clip_rect(labels_rect);
    let mut x = hrect.left() - tab.graph_hscroll;
    let last_visible = (0..6).rev().find(|&i| visible[i]).unwrap_or(2);
    for i in 0..6 {
        if !visible[i] {
            continue;
        }
        let col = Rect::from_min_size(pos2(x, hrect.top()), vec2(eff[i], HEAD_H));
        let label = match (i, &author_filter) {
            (3, Some(a)) => format!("AUTHOR: {a}"),
            _ => LABELS[i].to_string(),
        };
        let color = if i == 3 && author_filter.is_some() { pal.accent } else { pal.muted };
        hp.with_clip_rect(col.intersect(hrect)).text(pos2(col.left() + 10.0, col.center().y), Align2::LEFT_CENTER, label, FontId::proportional(11.5), color);
        if i == 3 {
            // Author header: click to filter the history by author.
            let resp = ui.interact(col.intersect(hrect), ui.scope_id().with("author_header"), Sense::click()).on_hover_text("Filter by author");
            egui::Popup::menu(&resp).show(|ui| {
                crate::util::menu_pad(ui);
                ui.set_min_width(200.0);
                if ui.selectable_label(author_filter.is_none(), "All authors").clicked() {
                    tab.author_filter = None;
                    ui.close();
                }
                ui.separator();
                let mut authors: Vec<(&str, usize)> = Vec::new();
                for c in tab.snap.commits.iter().filter(|c| c.stash.is_none()) {
                    match authors.iter_mut().find(|(a, _)| *a == c.author) {
                        Some((_, n)) => *n += 1,
                        None => authors.push((&c.author, 1)),
                    }
                }
                authors.sort_by(|a, b| b.1.cmp(&a.1));
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (a, n) in authors {
                        let on = author_filter.as_deref() == Some(a);
                        if ui.selectable_label(on, format!("{a}  ({n})")).clicked() {
                            tab.author_filter = if on { None } else { Some(a.to_string()) };
                            ui.close();
                        }
                    }
                });
            });
        }
        x += eff[i];
        // Resize handle on the column's right edge.
        if i != last_visible {
            let handle = Rect::from_center_size(pos2(x, hrect.center().y), vec2(8.0, HEAD_H));
            if hrect.contains(handle.center()) {
                let r = ui.interact(handle, ui.scope_id().with(("col_resize", i)), Sense::drag());
                let hot = r.hovered() || r.dragged();
                if hot {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                hp.line_segment([pos2(x, hrect.top() + 6.0), pos2(x, hrect.bottom() - 6.0)], Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { pal.accent } else { pal.border }));
                if r.dragged() {
                    widths[i] = (widths[i] + r.drag_delta().x).max(MIN_COL[i]);
                }
                if r.double_clicked() && i == 1 {
                    // Double-click the graph edge: fit all lanes.
                    widths[1] = (tab.graph.width as f32 * lane_w + 24.0).max(MIN_COL[1]);
                }
            }
        }
    }
    // Column settings: which columns show, compact lanes, preset layouts.
    let gear = Rect::from_center_size(pos2(hrect.right() - GEAR_W / 2.0, hrect.center().y), vec2(22.0, 22.0));
    let gr = ui.interact(gear, ui.scope_id().with("columns_menu"), Sense::click());
    if gr.hovered() {
        ui.painter().rect_filled(gear, CornerRadius::same(5), pal.hover);
    }
    icons::paint(ui.painter(), gear.shrink(4.0), Icon::Settings, if gr.hovered() { pal.text } else { pal.muted });
    let gr = gr.on_hover_text("Columns");
    egui::Popup::menu(&gr).show(|ui| {
        crate::util::menu_pad(ui);
        ui.set_min_width(250.0);
        for i in 0..6 {
            let mut on = visible[i];
            if ui.checkbox(&mut on, MENU_NAMES[i]).changed() {
                // Keep at least one column.
                if on || visible.iter().filter(|v| **v).count() > 1 {
                    visible[i] = on;
                }
            }
        }
        ui.separator();
        ui.checkbox(compact, "Compact graph column");
        ui.separator();
        if ui.button("Reset columns to default layout").clicked() {
            *widths = crate::app::DEFAULT_COLUMNS;
            *visible = [true; 6];
            *compact = false;
            ui.close();
        }
        if ui.button("Reset columns to compact layout").clicked() {
            *widths = crate::app::COMPACT_COLUMNS;
            *visible = [true, true, true, false, true, false];
            *compact = true;
            ui.close();
        }
    });
    ui.add_space(4.0);

    let total_rows = tab.snap.commits.len() + offset;
    let mut scroll = egui::ScrollArea::both().auto_shrink([false, false]).id_salt("graph_rows");
    if let Some(row) = tab.scroll_to.take() {
        let viewport_h = ui.available_height();
        scroll = scroll.vertical_scroll_offset((row as f32 * ROW_H - viewport_h * 0.4).max(0.0));
    }
    // Middle-click and drag pans the history in both directions.
    let area = ui.available_rect_before_wrap();
    let (pressed_here, down, delta) = ui.input(|i| {
        let here = i.pointer.button_pressed(egui::PointerButton::Middle) && i.pointer.interact_pos().is_some_and(|p| area.contains(p));
        (here, i.pointer.middle_down(), i.pointer.delta())
    });
    if pressed_here {
        tab.graph_panning = true;
    }
    if !down {
        tab.graph_panning = false;
    }
    if tab.graph_panning {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let target = (tab.graph_offset - delta).max(egui::Vec2::ZERO);
        scroll = scroll.scroll_offset(target);
    }

    // show_rows sizes rows with the current item spacing: zero it first, or rows drift
    // and the list stops before the bottom of the tile.
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = scroll.show_rows(ui, ROW_H, total_rows, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let first_top = ui.cursor().top();
        let left = ui.cursor().left();
        let y_of = |row: usize| first_top + (row as f32 - range.start as f32) * ROW_H + ROW_H * 0.5;
        let cols_of = |rect: Rect| {
            let mut x = rect.left();
            let mut out = [Rect::NOTHING; 6];
            for (i, w) in eff.iter().enumerate() {
                out[i] = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, rect.height()));
                x += w;
            }
            out
        };
        let graph_col = cols_of(Rect::from_min_size(pos2(left, 0.0), vec2(content_w, 1.0)))[1];
        let x_of = |col: usize| graph_col.left() + 14.0 + col as f32 * lane_w;

        // Branches open in another folder (worktrees), marked on their labels.
        let folders = crate::ui_big::branch_folders(tab);
        // Pass 1: row backgrounds, text and interaction.
        for row in range.clone() {
            let (rect, resp) = ui.allocate_exact_size(vec2(content_w, ROW_H), Sense::click());
            let cols = cols_of(rect);
            let painter = ui.painter().clone();

            if wip && row == 0 {
                let selected = tab.selection == Selection::Wip && tab.picked_commits.is_empty();
                paint_row_bg(&painter, rect, pal, selected, resp.hovered(), false);
                let n = tab.snap.status.staged.len() + tab.snap.status.unstaged.len();
                let label = if tab.snap.head_sha.is_none() {
                    "No commits yet: stage files and make the first commit".to_string()
                } else {
                    let s = tab.summary.trim();
                    if s.is_empty() { format!("Work in progress  ·  {n} changed file{}", plural(n)) } else { format!("{s}  ·  {n} file{}", plural(n)) }
                };
                painter
                    .with_clip_rect(cols[2])
                    .text(pos2(cols[2].left() + 10.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), pal.muted);
                if resp.clicked() {
                    acts.push(Action::Select(Selection::Wip));
                }
                continue;
            }

            let c = &tab.snap.commits[row - offset];
            let picked = tab.picked_commits.contains(&c.sha);
            let selected = picked || (tab.picked_commits.is_empty() && tab.selection == Selection::Commit(c.sha.clone()));
            // Search (by mode), author filter, and commits a bisect has ruled out are dimmed.
            let missed = crate::ui_extra::matches(tab, c, &search).is_some_and(|m| !m);
            let ruled_out = tab.snap.bisect.as_ref().is_some_and(|b| !b.candidates.contains(&c.sha));
            let dim = missed || ruled_out || author_filter.as_ref().is_some_and(|a| a != &c.author);
            paint_row_bg(&painter, rect, pal, selected, resp.hovered(), row % 2 == 1);
            let text_col = if dim { pal.muted.gamma_multiply(0.6) } else { pal.text };
            let lane_color = pal.lane(tab.graph.rows[row - offset].color);

            // Ref labels; a double-click on a branch label switches to that branch.
            let rp = painter.with_clip_rect(cols[0].shrink2(vec2(4.0, 0.0)));
            let mut x = cols[0].left() + 8.0;
            let mut pills: Vec<(Rect, usize)> = Vec::new();
            // "+N" badge position and how many labels it hides; the list it opens.
            let mut badge: Option<(f32, usize)> = None;
            let mut badge_list: Option<(Rect, Vec<(usize, String, bool)>)> = None;
            if let Some(st) = &c.stash {
                let galley = rp.layout_no_wrap(st.clone(), FontId::proportional(12.0), pal.text2());
                let pill = Rect::from_min_size(pos2(x, rect.center().y - 10.0), vec2(galley.size().x + 14.0 + PILL_ICON_W, 20.0));
                rp.rect_stroke(pill, CornerRadius::same(6), Stroke::new(1.0, pal.muted), StrokeKind::Inside);
                pill_icon(&rp, pill.left() + 6.0, pill.center().y, Icon::Stash, pal.text2());
                rp.galley(pos2(pill.left() + 7.0 + PILL_ICON_W, pill.center().y - galley.size().y / 2.0), galley, pal.text2());
            }
            if let Some(idxs) = tab.refs_by_sha.get(&c.sha) {
                // Each branch name appears once: the local branch when there is one (with a
                // cloud when its remote copy sits on the same commit), otherwise the remote
                // one without its "origin/" prefix.
                let short_of = |name: &str| name.split_once('/').map_or(name.to_string(), |(_, b)| b.to_string());
                let has_local = |short: &str| tab.snap.refs.iter().any(|r| r.kind == RefKind::Local && r.name == short);
                let mut shown: Vec<(usize, String, bool)> = Vec::new(); // (ref index, label, synced)
                for &i in idxs {
                    let r = &tab.snap.refs[i];
                    match r.kind {
                        RefKind::Local => {
                            let synced = idxs.iter().any(|&j| tab.snap.refs[j].kind == RefKind::Remote && short_of(&tab.snap.refs[j].name) == r.name);
                            shown.push((i, r.name.clone(), synced));
                        }
                        RefKind::Remote => {
                            let short = short_of(&r.name);
                            if !has_local(&short) {
                                shown.push((i, short, false));
                            }
                        }
                        RefKind::Tag => shown.push((i, r.name.clone(), false)),
                    }
                }
                shown.sort_by_key(|(i, _, _)| {
                    let r = &tab.snap.refs[*i];
                    let head = r.kind == RefKind::Local && tab.snap.head_branch.as_deref() == Some(r.name.as_str());
                    match r.kind {
                        _ if head => 0,
                        RefKind::Local => 1,
                        RefKind::Remote => 2,
                        RefKind::Tag => 3,
                    }
                });
                let count = shown.len();
                let right = cols[0].right() - 6.0;
                let font = FontId::proportional(12.0);
                let badge_text = |k: usize| format!("+{k}");
                let badge_w = |k: usize| rp.layout_no_wrap(badge_text(k), FontId::proportional(11.5), pal.text).size().x + 12.0;
                for (n, (i, label, synced)) in shown.iter().cloned().enumerate() {
                    let r = &tab.snap.refs[i];
                    let is_head = r.kind == RefKind::Local && tab.snap.head_branch.as_deref() == Some(r.name.as_str());
                    // Branches: solid pill in a deeper version of the lane colour so the white
                    // name stands out. Tags stay outlined.
                    let pill_fill = deepen(lane_color);
                    let (lead, trail, fill, fg) = match r.kind {
                        RefKind::Tag => (Some(Icon::Tag), None, pal.card, pal.text2()),
                        RefKind::Remote => (Some(Icon::Cloud), None, pill_fill, Color32::WHITE),
                        RefKind::Local => (is_head.then_some(Icon::Check), synced.then_some(Icon::Cloud), pill_fill, Color32::WHITE),
                    };
                    let lead_w = if lead.is_some() { PILL_ICON_W } else { 0.0 };
                    let trail_w = if trail.is_some() { PILL_ICON_W + 2.0 } else { 0.0 };
                    let natural_text = rp.layout_no_wrap(label.clone(), font.clone(), fg).size().x;
                    let rest = count - n - 1;
                    // Room for this pill, keeping space for a "+N" badge if others follow.
                    let reserve = if rest > 0 { badge_w(rest) + 4.0 } else { 0.0 };
                    let room = right - x - reserve;
                    let chrome = 14.0 + lead_w + trail_w;
                    let fits = natural_text + chrome <= room;
                    if !fits && n > 0 {
                        // This one (and the rest) go in the "+N" list.
                        badge = Some((x, count - n));
                        break;
                    }
                    let text_w = if fits { natural_text } else { (room - chrome).max(0.0) };
                    if text_w < 14.0 && n > 0 {
                        badge = Some((x, count - n));
                        break;
                    }
                    let mut job = egui::text::LayoutJob::simple_singleline(label, font.clone(), fg);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w.max(14.0));
                    let galley = rp.layout_job(job);
                    let text_w = galley.size().x;
                    let pill = Rect::from_min_size(pos2(x, rect.center().y - 10.0), vec2(text_w + chrome, 20.0));
                    rp.rect_filled(pill, CornerRadius::same(6), fill);
                    if r.kind == RefKind::Tag {
                        rp.rect_stroke(pill, CornerRadius::same(6), Stroke::new(1.0, lane_color), StrokeKind::Inside);
                    } else if is_head {
                        rp.rect_stroke(pill, CornerRadius::same(6), Stroke::new(1.5, Color32::WHITE.gamma_multiply(0.7)), StrokeKind::Inside);
                    }
                    if let Some(icon) = lead {
                        pill_icon(&rp, pill.left() + 6.0, pill.center().y, icon, fg);
                    }
                    rp.galley(pos2(pill.left() + 7.0 + lead_w, pill.center().y - galley.size().y / 2.0), galley, fg);
                    if let Some(icon) = trail {
                        pill_icon(&rp, pill.left() + 9.0 + lead_w + text_w, pill.center().y, icon, fg);
                    }
                    if r.kind != RefKind::Tag {
                        pills.push((pill.intersect(cols[0]), i));
                    }
                    x = pill.right() + 4.0;
                    if rest > 0 && n + 1 < count && x + badge_w(rest) > right {
                        badge = Some((x, rest));
                        break;
                    }
                }
                if let Some((bx, k)) = badge {
                    let w = badge_w(k);
                    let br = Rect::from_min_size(pos2(bx, rect.center().y - 10.0), vec2(w, 20.0));
                    rp.rect_filled(br, CornerRadius::same(6), deepen(lane_color));
                    rp.text(br.center(), Align2::CENTER_CENTER, badge_text(k), FontId::proportional(11.5), Color32::WHITE);
                    badge_list = Some((br.intersect(cols[0]), shown.clone()));
                }
            }

            // Message: summary in the main text colour, then the description in the second shade.
            let mp = painter.with_clip_rect(cols[2].shrink2(vec2(6.0, 0.0)));
            let msg_col = if c.stash.is_some() && !dim { pal.text2() } else { text_col };
            let summary = mp.layout_no_wrap(c.summary.clone(), FontId::proportional(13.5), msg_col);
            let sw = summary.size().x;
            mp.galley(pos2(cols[2].left() + 10.0, rect.center().y - summary.size().y / 2.0), summary, msg_col);
            if show_desc && !c.body.is_empty() {
                let body_col = if dim { pal.muted.gamma_multiply(0.5) } else { pal.muted };
                mp.text(pos2(cols[2].left() + 22.0 + sw, rect.center().y), Align2::LEFT_CENTER, &c.body, FontId::proportional(13.0), body_col);
            }
            // Author
            let ap = painter.with_clip_rect(cols[3]);
            avatar(&ap, pos2(cols[3].left() + 20.0, rect.center().y), 10.0, &c.author, &c.email);
            ap.text(pos2(cols[3].left() + 36.0, rect.center().y), Align2::LEFT_CENTER, &c.author, FontId::proportional(12.5), if dim { text_col } else { pal.text2() });
            // Date and id
            painter.with_clip_rect(cols[4]).text(pos2(cols[4].left() + 10.0, rect.center().y), Align2::LEFT_CENTER, short_date_time(c.time), FontId::proportional(12.5), pal.muted);
            painter.with_clip_rect(cols[5]).text(pos2(cols[5].left() + 10.0, rect.center().y), Align2::LEFT_CENTER, git::short(&c.sha), FontId::monospace(12.0), pal.muted);
            // Signed commits: a small mark after the SHA (green verified, amber otherwise).
            if let Some((sig, _)) = tab.signatures.get(&c.sha) {
                let color = match sig {
                    'G' => pal.added,
                    'B' | 'R' => pal.removed,
                    _ => pal.warn,
                };
                let x = cols[5].left() + 74.0;
                painter.with_clip_rect(cols[5]).circle_filled(pos2(x, rect.center().y), 4.0, color);
            }

            let signed = tab
                .signatures
                .get(&c.sha)
                .map(|(s, who)| format!("\n{}{}", crate::extras::signature_text(*s), if who.is_empty() { String::new() } else { format!(" · {who}") }))
                .unwrap_or_default();
            let resp = resp.on_hover_text_at_pointer(format!("{}\n{} · {}{signed}\nCtrl+click or Shift+click to pick several", c.summary, c.author, abs_time(c.time)));
            if resp.clicked() {
                // Ctrl+click adds or removes a commit, Shift+click picks a range; a plain click
                // goes back to one selected commit.
                let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                if ctrl || shift {
                    acts.push(Action::PickCommit { sha: c.sha.clone(), toggle: ctrl, range: shift });
                } else {
                    acts.push(Action::ClearPicked);
                    acts.push(Action::Select(Selection::Commit(c.sha.clone())));
                }
            }
            for (pill, i) in pills {
                let pr = ui.interact(pill, ui.scope_id().with(("pill", &c.sha, i)), Sense::click_and_drag());
                let r = &tab.snap.refs[i];
                crate::ui_quick::drag_branch(&pr, &r.name);
                if r.kind == RefKind::Local {
                    crate::ui_quick::drop_on_branch(ui, &pr, &r.name, pal, acts);
                }
                // A branch open in another folder: go there rather than fail (doc §Worktrees 3).
                let elsewhere = folders.get(&r.name).cloned();
                if elsewhere.is_some() {
                    // A small folder badge on the label's top-right corner.
                    let c = pos2(pr.rect.right() - 1.0, pr.rect.top() + 1.0);
                    let p = painter.with_clip_rect(cols[0].expand2(vec2(0.0, 4.0)));
                    p.circle_filled(c, 7.0, pal.panel);
                    icons::paint(&p, Rect::from_center_size(c, vec2(10.0, 10.0)), Icon::Folder, pal.accent);
                }
                if pr.double_clicked() {
                    let r = &tab.snap.refs[i];
                    acts.push(match (r.kind.clone(), elsewhere.clone()) {
                        (RefKind::Local, Some(p)) => Action::OpenWorktree(p),
                        (RefKind::Remote, _) => Action::CheckoutRemote(r.name.clone()),
                        _ => Action::CheckoutLocal(r.name.clone()),
                    });
                } else if pr.clicked() {
                    acts.push(Action::Select(Selection::Commit(c.sha.clone())));
                }
                let pr = match &elsewhere {
                    Some(p) => pr.on_hover_text(format!("Open in another folder: {}\nDouble-click to go to its tab", p.display())),
                    None => pr.on_hover_text("Double-click to switch to this branch · drag it onto another branch to merge or replay it"),
                };
                commit_menu(&pr, tab, &c.sha, Some(i), acts);
            }
            if let Some((br, list)) = badge_list {
                let bresp = ui.interact(br, ui.scope_id().with(("badge", &c.sha)), Sense::click());
                let sha = c.sha.clone();
                let lane = deepen(lane_color);
                bresp.on_hover_ui(|ui| {
                    crate::util::menu_pad(ui);
                    ui.set_min_width(220.0);
                    ui.label(RichText::new("Branches and tags on this commit").size(11.5).weak());
                    for (i, label, _) in &list {
                        let r = &tab.snap.refs[*i];
                        let (icon, fill, fg, tip) = match r.kind {
                            RefKind::Tag => (Icon::Tag, pal.card, pal.text2(), "Show this commit"),
                            RefKind::Remote => (Icon::Cloud, lane, Color32::WHITE, "Switch to this branch"),
                            RefKind::Local => (Icon::Branch, lane, Color32::WHITE, "Switch to this branch"),
                        };
                        let b = egui::Button::image_and_text(icons::image(ui.ctx(), icon, 12.0, fg), RichText::new(label).color(fg))
                            .fill(fill)
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(ui.available_width(), 24.0));
                        if ui.add(b).on_hover_text(tip).clicked() {
                            acts.push(match r.kind {
                                RefKind::Remote => Action::CheckoutRemote(r.name.clone()),
                                RefKind::Local => Action::CheckoutLocal(r.name.clone()),
                                RefKind::Tag => Action::SelectRef(sha.clone()),
                            });
                        }
                    }
                });
            }
            if picked && tab.picked_commits.len() > 1 {
                selection_menu(&resp, tab, acts);
            } else {
                commit_menu(&resp, tab, &c.sha, None, acts);
            }
        }

        // Pass 2: the graph, painted over the row backgrounds so lines run uninterrupted,
        // and clipped to its column.
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(Rect::from_x_y_ranges(graph_col.x_range(), f32::NEG_INFINITY..=f32::INFINITY)));
        let start = range.start.saturating_sub(1);
        let stash_colors: Vec<usize> = tab
            .snap
            .commits
            .iter()
            .zip(&tab.graph.rows)
            .filter(|(c, _)| c.stash.is_some())
            .map(|(_, g)| g.color)
            .collect();
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
                // Only the stash's own line is grey; branches passing next to it keep their colour.
                let color = if stash_colors.contains(&e.color) { pal.muted } else { pal.lane(e.color) };
                let stroke = Stroke::new(2.0, color);
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
            let commit = &tab.snap.commits[row - offset];
            let c = pos2(x_of(g.col), y_of(row));
            let color = pal.lane(g.color);
            if commit.stash.is_some() {
                // Stashes: a hollow square, quieter than commits.
                let r = Rect::from_center_size(c, vec2(11.0, 11.0));
                painter.rect_filled(r, CornerRadius::same(3), pal.panel);
                painter.rect_stroke(r, CornerRadius::same(3), Stroke::new(2.0, pal.muted), StrokeKind::Middle);
                continue;
            }
            let is_head = tab.snap.head_sha.as_deref() == Some(commit.sha.as_str());
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
    tab.graph_hscroll = out.state.clamped_offset().x;
    tab.graph_offset = out.state.clamped_offset();
    // Reaching the bottom loads the next 3 000 commits (doc §Historique complet).
    let near_bottom = out.state.clamped_offset().y + out.inner_rect.height() >= out.content_size.y - ROW_H * 10.0;
    if tab.snap.more_history && near_bottom && !tab.loading_more && !tab.is_sample() {
        tab.loading_more = true;
        acts.push(Action::LoadMoreHistory);
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
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

fn git_action(label: String, args: &[&str]) -> Action {
    Action::Git { label, args: args.iter().map(|s| s.to_string()).collect(), background: false }
}

/// Right-click menu of several picked commits. The actions built on picking (copy them onto
/// a branch, compare two) come with the next phases; picking itself is the foundation.
fn selection_menu(resp: &egui::Response, tab: &RepoTab, acts: &mut Vec<Action>) {
    let n = tab.picked_commits.len();
    resp.context_menu(|ui| {
        crate::util::menu_pad(ui);
        ui.label(RichText::new(format!("{n} commits picked")).weak());
        crate::ui_extra::picked_menu(ui, tab, acts);
        ui.separator();
        if ui.button(format!("Copy {n} commit SHAs")).clicked() {
            // Oldest first, the order they were made in.
            let mut picked: Vec<(usize, &String)> = tab.picked_commits.iter().filter_map(|s| tab.commit_index(s).map(|i| (i, s))).collect();
            picked.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
            acts.push(Action::Copy(picked.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join("\n")));
            ui.close();
        }
        if ui.button("Clear the selection").clicked() {
            acts.push(Action::ClearPicked);
            ui.close();
        }
    });
}

/// Right-click menu of a graph row (any row, not only the checked-out one). `on_ref` is the
/// branch label that was right-clicked, if any; otherwise the row's own branch is used.
pub fn commit_menu(resp: &egui::Response, tab: &RepoTab, sha: &str, on_ref: Option<usize>, acts: &mut Vec<Action>) {
    let commit = tab.commit(sha).cloned();
    resp.context_menu(|ui| {
        crate::util::menu_pad(ui);
        ui.set_min_width(240.0);
        let sha = sha.to_string();
        let short = git::short(&sha).to_string();
        if let Some(st) = commit.as_ref().and_then(|c| c.stash.clone()) {
            ui.label(RichText::new(format!("Stash {st}")).weak());
            if ui.button("Apply (keep the stash)").clicked() {
                acts.push(git_action("Stash applied".into(), &["stash", "apply", &st]));
                ui.close();
            }
            if ui.button("Pop (apply and delete)").clicked() {
                acts.push(git_action("Stash popped".into(), &["stash", "pop", &st]));
                ui.close();
            }
            if ui.button("Delete stash").clicked() {
                acts.push(git_action("Stash deleted".into(), &["stash", "drop", &st]));
                ui.close();
            }
            ui.separator();
            if ui.button("Copy commit SHA").clicked() {
                acts.push(Action::Copy(sha.clone()));
                ui.close();
            }
            return;
        }

        let current = tab.snap.head_branch.clone();
        let is_head = tab.snap.head_sha.as_deref() == Some(sha.as_str());
        // The branch this menu is about: the clicked label, else a branch on this commit
        // (another local branch first, then a remote one).
        let refs_here: Vec<usize> = tab.refs_by_sha.get(&sha).cloned().unwrap_or_default();
        let pick = on_ref.or_else(|| {
            let not_tag = |i: &&usize| tab.snap.refs[**i].kind != RefKind::Tag;
            let local_other = refs_here.iter().filter(not_tag).find(|&&i| tab.snap.refs[i].kind == RefKind::Local && Some(&tab.snap.refs[i].name) != current.as_ref());
            local_other.or_else(|| refs_here.iter().filter(not_tag).find(|&&i| tab.snap.refs[i].kind == RefKind::Remote)).or_else(|| refs_here.iter().find(not_tag)).copied()
        });
        let branch = pick.map(|i| tab.snap.refs[i].clone());
        let web = tab.web_repo();
        let cur = current.clone().unwrap_or_else(|| "HEAD".into());

        ui.label(RichText::new(match &branch {
            Some(b) => format!("{}  ·  {short}", b.name),
            None => format!("Commit {short}"),
        }).weak());

        if ui.button("Pull").clicked() {
            acts.push(Action::Pull(PullMode::FastForwardIfPossible));
            ui.close();
        }
        if ui.button("Push").clicked() {
            acts.push(Action::Push { force: false });
            ui.close();
        }
        ui.separator();

        // Merge / rebase between this row and the local branch.
        let target_name = branch.as_ref().map(|b| b.name.clone()).unwrap_or_else(|| short.clone());
        let target_ref = branch.as_ref().map(|b| b.name.clone()).unwrap_or_else(|| sha.clone());
        let is_current = branch.as_ref().is_some_and(|b| b.kind == RefKind::Local && Some(&b.name) == current.as_ref());
        if !is_head && !is_current {
            crate::ui_quick::merge_items(ui, &target_ref, &target_name, &cur, acts);
        }
        if let Some(b) = &branch {
            let local_name = if b.kind == RefKind::Remote { b.name.split_once('/').map(|(_, n)| n.to_string()).unwrap_or(b.name.clone()) } else { b.name.clone() };
            if ui.button(format!("Start a pull request from {local_name}")).clicked() {
                acts.push(Action::OpenPullRequestForm(Some(local_name.clone())));
                ui.close();
            }
            if b.kind == RefKind::Local {
                if ui.button(format!("Rename {}…", b.name)).clicked() {
                    acts.push(Action::Dialog(Dialog::RenameBranch { old: b.name.clone(), name: b.name.clone() }));
                    ui.close();
                }
                if !is_current
                    && ui
                        .button(format!("Delete {}", b.name))
                        .on_hover_text("Deleted at once (Undo in the toast); if it has commits no other branch has, they are listed first.")
                        .clicked()
                {
                    acts.push(Action::DeleteBranch(b.name.clone()));
                    ui.close();
                }
            } else if b.kind == RefKind::Remote {
                if let Some((remote, name)) = b.name.split_once('/') {
                    if ui.button(format!("Delete {} on {remote}", name)).clicked() {
                        acts.push(Action::Dialog(Dialog::Confirm {
                            title: format!("Delete {} on {remote}?", b.name),
                            body: "The branch will be removed from the server for everyone.".into(),
                            confirm: "Delete".into(),
                            then: Box::new(Action::Git { label: format!("Deleted {}", b.name), args: vec!["push".into(), remote.into(), "--delete".into(), name.into()], background: true }),
                        }));
                        ui.close();
                    }
                }
            }
        }
        ui.separator();

        if ui.button("Create branch here…").clicked() {
            acts.push(Action::Dialog(Dialog::NewBranch { at: Some(sha.clone()), name: String::new(), checkout: true }));
            ui.close();
        }
        if ui.button("Create tag here…").clicked() {
            acts.push(Action::Dialog(Dialog::NewTag { at: sha.clone(), name: String::new(), message: String::new() }));
            ui.close();
        }
        if ui.button("Check out this commit").clicked() {
            acts.push(Action::Dialog(Dialog::Confirm {
                title: "Check out this commit?".into(),
                body: "You will be on this commit without a branch (\"detached\"). Create a branch if you want to keep new work.".into(),
                confirm: "Check out".into(),
                then: Box::new(git_action(format!("Checked out {short}"), &["switch", "--detach", &sha])),
            }));
            ui.close();
        }
        if !is_head && ui.button("Cherry-pick onto current branch").clicked() {
            acts.push(git_action(format!("Cherry-picked {short}"), &["cherry-pick", &sha]));
            ui.close();
        }
        if ui.button("Revert this commit").clicked() {
            acts.push(git_action(format!("Reverted {short}"), &["revert", "--no-edit", &sha]));
            ui.close();
        }
        // Edit message, combine, remove, move, Edit history (doc §Rebase interactif).
        crate::ui_big::rewrite_menu(ui, tab, &sha, acts);
        if let Some(b) = branch.as_ref().filter(|b| b.kind != RefKind::Tag) {
            crate::ui_big::worktree_item(ui, tab, &b.name, b.kind.clone(), acts);
            crate::ui_extra::branch_items(ui, tab, &b.name, b.kind == RefKind::Local, acts);
        }
        crate::ui_extra::commit_items(ui, tab, &sha, acts);
        ui.menu_button("Reset current branch to here", |ui| {
            crate::util::menu_pad(ui);
            if ui.button("Soft: keep all changes staged").clicked() {
                acts.push(git_action("Reset (soft)".into(), &["reset", "--soft", &sha]));
                ui.close();
            }
            if ui.button("Mixed: keep changes, unstaged").clicked() {
                acts.push(git_action("Reset (mixed)".into(), &["reset", "--mixed", &sha]));
                ui.close();
            }
            // No confirmation since the safety net: changed files are saved first, Undo restores them.
            if ui.button("Hard: throw away all changes").on_hover_text("Uncommitted changes are saved first: Undo brings them back.").clicked() {
                acts.push(git_action("Reset (hard)".into(), &["reset", "--hard", &sha]));
                ui.close();
            }
        });
        ui.separator();

        if ui.button("Copy commit SHA").clicked() {
            acts.push(Action::Copy(sha.clone()));
            ui.close();
        }
        if let Some(w) = &web {
            if ui.button("Copy commit link").clicked() {
                acts.push(Action::Copy(forge::commit_url(w, &sha)));
                ui.close();
            }
        }
        if let Some(b) = &branch {
            let plain = if b.kind == RefKind::Remote { b.name.split_once('/').map(|(_, n)| n.to_string()).unwrap_or(b.name.clone()) } else { b.name.clone() };
            if let Some(w) = &web {
                if ui.button("Copy branch link").clicked() {
                    acts.push(Action::Copy(forge::branch_url(w, &plain)));
                    ui.close();
                }
            }
            if ui.button("Copy branch name").clicked() {
                acts.push(Action::Copy(plain));
                ui.close();
            }
        }
    });
}

/// A deeper, more saturated version of a lane colour for solid labels, so white text on it
/// stays readable even with light pastel lanes.
fn deepen(c: Color32) -> Color32 {
    let mut h = egui::ecolor::Hsva::from(c);
    h.s = (h.s * 1.2 + 0.12).min(1.0);
    h.v = (h.v * 0.8).min(0.66);
    h.into()
}

/// A 12 px icon in a ref label, its left edge at `x`, centred on `cy`.
fn pill_icon(p: &egui::Painter, x: f32, cy: f32, icon: Icon, color: Color32) {
    icons::paint(p, Rect::from_min_size(pos2(x, cy - 6.0), vec2(12.0, 12.0)), icon, color);
}
