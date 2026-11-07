//! Window layout in "bento" style: every area is its own rounded tile on a dark canvas.
//! Tab strip on top, a row of header tiles (repository, actions, sync, changes, search),
//! then the branches, history and details tiles; plus home screen, dialogs and toasts.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::access;
use crate::app::{AccessForm, Action, Dialog, PrForm, PullMode, Settings, ToastKind, VegitApp, dirs_home, save_screenshot};
use crate::forge::Forge;
use crate::git::{self, FileState, RefKind};
use crate::icons::{self, Icon};
use crate::theme::{self, LaneMode, Palette, ThemeChoice};
use crate::util::{GAP, accent_button, card, fill_tile, icon_label, tile};
use crate::{ui_detail, ui_graph, ui_side};

/// Frames drawn before the `--screenshot` picture is taken.
const SCREENSHOT_FRAME: u32 = 120;
const HEADER_H: f32 = 44.0;

impl eframe::App for VegitApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ensure_theme(&ctx);
        self.pump_jobs(&ctx);
        let pal = self.palette();

        if ui.input(|i| i.key_pressed(egui::Key::F5)) {
            self.actions.push(Action::Refresh);
        }
        // Ctrl+Z / Ctrl+Y (and Ctrl+Shift+Z), unless a text field or a dialog has the keys.
        if self.dialog.is_none() && !ctx.egui_wants_keyboard_input() {
            let (z, y, shift) = ctx.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::Z), i.modifiers.command && i.key_pressed(egui::Key::Y), i.modifiers.shift));
            if z && !shift {
                self.actions.push(Action::UndoLast);
            } else if y || (z && shift) {
                self.actions.push(Action::Redo);
            }
        }
        let window = ui.max_rect();
        if ctx.input(|i| i.key_pressed(egui::Key::F9)) {
            self.shuffle_backdrop();
        }
        // Canvas and halftone on the background layer; every frame around the tiles is
        // transparent so the pattern shows between them.
        let look = self.backdrop_look();
        let bg = ctx.layer_painter(egui::LayerId::background());
        crate::backdrop::paint(&bg, window, &look, pal.canvas, pal.accent);
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if !maximized {
            resize_edges(&ctx, window);
        }

        egui::Panel::top("tabs")
            .frame(egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 0 }))
            .show_separator_line(false)
            .show(ui, |ui| self.tab_strip(ui, &pal));

        if self.show_home || self.tabs.is_empty() {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(egui::Margin::same(24)))
                .show(ui, |ui| self.home(ui, &pal));
        } else {
            self.repo_view(ui, &pal);
        }

        self.dialogs(&ctx, &pal);
        self.toasts_ui(&ctx, &pal);
        // A .patch file dropped on the window opens its preview.
        self.dropped_files(&ctx);
        // Branch drag and drop: the dragged name follows the pointer; after a drop, its menu.
        crate::ui_quick::drag_ghost(&ctx, &pal);
        if let Some(menu) = self.drop_menu.clone() {
            let local = self.tabs.get(self.active).is_some_and(|t| t.snap.refs.iter().any(|r| r.kind == RefKind::Local && r.name == menu.source));
            let mut acts = Vec::new();
            crate::ui_quick::drop_menu(&ctx, &menu, local, &pal, &mut acts);
            self.actions.extend(acts);
        }
        if !maximized {
            // Without the system frame, a thin theme-coloured outline marks the window edge.
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("window_outline")))
                .rect_stroke(window, CornerRadius::ZERO, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        }
        self.handle(&ctx);
        self.screenshot_tick(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_none() {
            self.save_settings(storage);
        }
    }
}

impl VegitApp {
    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some(job) = self.screenshot.as_mut() else { return };
        job.frame += 1;
        if job.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if job.frame >= SCREENSHOT_FRAME && !job.requested {
            job.requested = true;
            let path = job.path.clone();
            let done = job.done.clone();
            let ctx2 = ctx.clone();
            ctx.request_screenshot(move |image| {
                save_screenshot(&image, &path, &done);
                ctx2.request_repaint();
            });
        }
        // Safety net: never hang if the renderer can't deliver a picture.
        if job.frame > SCREENSHOT_FRAME + 600 {
            eprintln!("Screenshot was not delivered; exiting");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        // The tab strip doubles as the title bar: drag it to move the window, double-click
        // to maximize. Registered first so tabs and buttons on top of it still get clicks.
        let bar = Rect::from_min_size(ui.max_rect().min - vec2(14.0, 8.0), vec2(ui.max_rect().width() + 28.0, 40.0));
        let drag = ui.interact(bar, ui.scope_id().with("title_bar"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (r, _) = ui.allocate_exact_size(vec2(28.0, TAB_H), Sense::hover());
            icons::paint_logo(ui.painter(), Rect::from_center_size(r.center() - vec2(0.0, 2.0), vec2(24.0, 24.0)));
            ui.add_space(6.0);
            let (lr, _) = ui.allocate_exact_size(vec2(44.0, TAB_H), Sense::hover());
            ui.painter().text(lr.left_center() - vec2(0.0, 2.0), egui::Align2::LEFT_CENTER, "Vegit", egui::FontId::proportional(13.5), pal.text);
            ui.add_space(14.0);

            // Tabs: the active one is a raised piece of the header tile below it (rounded top,
            // flared bottom corners); the others are plain labels split by thin bars.
            self.active_tab_rect = None;
            let name_font = egui::FontId::proportional(14.5);
            let mut prev_inactive = false;
            for i in 0..self.tabs.len() {
                let active = i == self.active && !self.show_home;
                let name = self.tabs[i].name.clone();
                let text_w = ui.painter().layout_no_wrap(name.clone(), name_font.clone(), pal.text).size().x.min(220.0);
                let pad = if active { 28.0 } else { 12.0 };
                let w = pad + text_w + 10.0 + 22.0 + pad * 0.6;
                if !active && prev_inactive {
                    let (sep, _) = ui.allocate_exact_size(vec2(1.0, TAB_H), Sense::hover());
                    ui.painter().rect_filled(Rect::from_center_size(sep.center() + vec2(0.0, -2.0), vec2(1.5, 20.0)), CornerRadius::ZERO, pal.border);
                }
                if active {
                    ui.add_space(TAB_FLARE);
                }
                let (r, resp) = ui.allocate_exact_size(vec2(w, TAB_H), Sense::click());
                if active {
                    ui.add_space(TAB_FLARE);
                    paint_tab_shape(ui.painter(), r, pal);
                    self.active_tab_rect = Some(r);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r.shrink2(vec2(0.0, 5.0)), CornerRadius::same(6), pal.hover);
                }
                prev_inactive = !active;
                let text_col = if active { pal.text } else { pal.text2() };
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), name_font.clone(), text_col);
                job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
                let g = ui.painter().layout_job(job);
                let ty = r.center().y - g.size().y / 2.0 - 1.0;
                ui.painter().galley(pos2(r.left() + pad, ty), g, text_col);
                // Close cross at the right of the name.
                let close = Rect::from_center_size(pos2(r.right() - pad * 0.6 - 11.0, r.center().y - 1.0), vec2(20.0, 20.0));
                let cr = ui.interact(close, ui.scope_id().with(("close_tab", i)), Sense::click());
                if cr.hovered() {
                    ui.painter().rect_filled(close, CornerRadius::same(4), pal.hover);
                }
                let c = close.center();
                let xs = Stroke::new(1.5, if cr.hovered() { pal.text } else { pal.accent.gamma_multiply(0.8) });
                ui.painter().line_segment([c - vec2(4.5, 4.5), c + vec2(4.5, 4.5)], xs);
                ui.painter().line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], xs);
                if cr.on_hover_text("Close tab").clicked() {
                    self.actions.push(Action::CloseTab(i));
                } else if resp.on_hover_text(name).clicked() {
                    self.actions.push(Action::SwitchTab(i));
                }
            }
            if prev_inactive {
                let (sep, _) = ui.allocate_exact_size(vec2(1.0, TAB_H), Sense::hover());
                ui.painter().rect_filled(Rect::from_center_size(sep.center() + vec2(0.0, -2.0), vec2(1.5, 20.0)), CornerRadius::ZERO, pal.border);
            }
            // "+" opens the home screen (open, clone or create).
            let (pr, presp) = ui.allocate_exact_size(vec2(34.0, TAB_H), Sense::click());
            let plus = Rect::from_center_size(pr.center() - vec2(0.0, 2.0), vec2(26.0, 26.0));
            if presp.hovered() || self.show_home {
                ui.painter().rect_filled(plus, CornerRadius::same(6), if self.show_home { pal.selected } else { pal.hover });
            }
            let c = plus.center();
            let ps = Stroke::new(1.6, pal.accent);
            ui.painter().line_segment([c - vec2(7.0, 0.0), c + vec2(7.0, 0.0)], ps);
            ui.painter().line_segment([c - vec2(0.0, 7.0), c + vec2(0.0, 7.0)], ps);
            if presp.on_hover_text("Open, clone or create a repository").clicked() {
                self.actions.push(Action::ShowHome);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                window_buttons(ui, pal);
                ui.add_space(6.0);
                let access_btn = egui::Button::image_and_text(icons::image(ui.ctx(), Icon::Key, 16.0, pal.text), "Access").corner_radius(CornerRadius::same(8));
                if ui.add(access_btn).on_hover_text("Sign in to reach private repositories").clicked() {
                    self.actions.push(Action::Dialog(Dialog::Access(AccessForm { host: "github.com".into(), ..Default::default() })));
                }
                if icons::icon_button(ui, Icon::Settings, pal.text2(), pal.panel, pal.hover).on_hover_text("Settings").clicked() {
                    self.actions.push(Action::Dialog(Dialog::Settings));
                }
                self.theme_menu(ui, pal);
                if !self.busy.is_empty() || self.pending > 0 {
                    ui.add(egui::Spinner::new());
                    let what = if self.busy.is_empty() { "Working".to_string() } else { self.busy.join(", ") };
                    ui.label(RichText::new(format!("{what}…")).color(pal.muted));
                }
            });
        });
    }

    fn theme_menu(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let current = self.settings.theme;
        ui.menu_image_text_button(icons::image(ui.ctx(), Icon::Palette, 16.0, pal.text), current.label(), |ui| {
            crate::util::menu_pad(ui);
            ui.set_min_width(330.0);
            ui.label(RichText::new("Theme preset").weak());
            // Whole rows are clickable, not just the name.
            for t in ThemeChoice::ALL {
                if preset_row(ui, t, t == current, pal).clicked() {
                    self.actions.push(Action::SetTheme(t));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("All appearance settings…").clicked() {
                self.actions.push(Action::Dialog(Dialog::Settings));
                ui.close();
            }
        });
    }

    fn repo_view(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let mut acts = std::mem::take(&mut self.actions);
        let busy = !self.busy.is_empty();
        let active = self.active;
        let tree = self.settings.tree_view;
        let diff_mode = self.settings.diff_mode;
        let show_desc = self.settings.show_descriptions;
        let sort_by_status = self.settings.sort_by_status;
        let columns = &mut self.settings.columns;
        let columns_visible = &mut self.settings.columns_visible;
        let compact_graph = &mut self.settings.compact_graph;
        let tab = &mut self.tabs[active];
        let g = GAP;

        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(egui::Margin { left: g * 2, right: g * 2, top: 0, bottom: g }))
            .show_separator_line(false)
            .show(ui, |ui| {
                // One slim tile for the whole header: repository, actions, sync, changes, search.
                tile(pal).inner_margin(egui::Margin::symmetric(10, 2)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.set_min_height(HEADER_H);
                        // The repository's name opens its menu (folders, history, maintenance).
                        let name = ui.add(egui::Label::new(RichText::new(&tab.name).size(17.0).strong()).sense(Sense::click())).on_hover_text("Repository menu");
                        egui::Popup::menu(&name).show(|ui| crate::ui_extra::repo_menu(ui, tab, &mut acts));
                        let branch = tab.snap.head_branch.clone().unwrap_or_else(|| {
                            tab.snap.head_sha.as_deref().map_or("no commits".into(), |h| format!("detached {}", git::short(h)))
                        });
                        let locals: Vec<String> = tab.snap.refs.iter().filter(|r| r.kind == RefKind::Local).map(|r| r.name.clone()).collect();
                        let branch_icon = icons::image(ui.ctx(), Icon::Branch, 15.0, pal.accent);
                        ui.menu_image_text_button(branch_icon, RichText::new(branch).color(pal.accent), |ui| {
                            crate::util::menu_pad(ui);
                            // Room to breathe even with short branch names.
                            ui.set_min_width(240.0);
                            ui.label(RichText::new("Switch branch").weak());
                            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                                for b in &locals {
                                    if ui.selectable_label(Some(b) == tab.snap.head_branch.as_ref(), b).clicked() {
                                        acts.push(Action::CheckoutLocal(b.clone()));
                                        ui.close();
                                    }
                                }
                            });
                            ui.separator();
                            if ui.button("New branch…").clicked() {
                                acts.push(Action::Dialog(Dialog::NewBranch { at: None, name: String::new(), checkout: true }));
                                ui.close();
                            }
                        });
                        divider(ui, pal);

                        // Actions: icons above labels, like a classic git toolbar.
                        ui.spacing_mut().item_spacing.x = 1.0;
                        let has_head = tab.snap.head_sha.is_some();
                        let tb = |ui: &mut egui::Ui, icon: Icon, label: &str, on: bool| icons::tool_button(ui, icon, label, on, pal.text, pal.hover);
                        // Undo names what it takes back; with nothing recorded it can still undo
                        // the last commit. A pushed action is greyed with the reason.
                        let (undo_label, undo_on, undo_tip) = match tab.undo.last() {
                            Some(e) if e.pushed => (format!("Undo {}", e.what), false, "Already on the server: undoing would need a force push.".to_string()),
                            Some(e) => (format!("Undo {}", e.what), true, format!("Take back the last {} (Ctrl+Z)", e.what)),
                            None => {
                                let pushed = tab.snap.current_ref().is_some_and(|r| r.upstream.is_some() && r.ahead == 0);
                                let tip = if pushed { "The last commit is already on the server: undoing would need a force push." } else { "Undo the last commit; its changes go back to the staging area (Ctrl+Z)" };
                                ("Undo".to_string(), has_head && !pushed, tip.to_string())
                            }
                        };
                        if tb(ui, Icon::Undo, &undo_label, undo_on && !busy).on_hover_text(undo_tip).clicked() {
                            acts.push(Action::UndoLast);
                        }
                        let redo_label = tab.redo.last().map_or("Redo".to_string(), |e| format!("Redo {}", e.what));
                        if tb(ui, Icon::Redo, &redo_label, !tab.redo.is_empty() && !busy).on_hover_text("Bring back what Undo took back (Ctrl+Y)").clicked() {
                            acts.push(Action::Redo);
                        }
                        if tb(ui, Icon::Fetch, "Fetch", !busy).on_hover_text("Download what's new from all remotes").clicked() {
                            acts.push(Action::Fetch);
                        }
                        let pull = tb(ui, Icon::Pull, "Pull", !busy).on_hover_text("Bring in the remote branch's commits (right-click for options)");
                        if pull.clicked() {
                            acts.push(Action::Pull(PullMode::FastForwardIfPossible));
                        }
                        pull.context_menu(|ui| {
                            crate::util::menu_pad(ui);
                            for (text, mode) in [
                                ("Fast-forward if possible", PullMode::FastForwardIfPossible),
                                ("Fast-forward only", PullMode::FastForwardOnly),
                                ("Rebase", PullMode::Rebase),
                            ] {
                                if ui.button(text).clicked() {
                                    acts.push(Action::Pull(mode));
                                    ui.close();
                                }
                            }
                        });
                        let push = tb(ui, Icon::Push, "Push", !busy).on_hover_text("Send your commits to the remote (right-click to force)");
                        if push.clicked() {
                            acts.push(Action::Push { force: false });
                        }
                        push.context_menu(|ui| {
                            crate::util::menu_pad(ui);
                            if ui.button("Push…").on_hover_text("Choose the remote, the branch name on the server and the tags to send").clicked() {
                                acts.push(Action::OpenPushDialog);
                                ui.close();
                            }
                            if ui.button("Force push (safely, with lease)").clicked() {
                                acts.push(Action::Dialog(Dialog::Confirm {
                                    title: "Force push?".into(),
                                    body: "The remote branch will be replaced by yours. Use this only after rewriting your own commits.".into(),
                                    confirm: "Force push".into(),
                                    then: Box::new(Action::Push { force: true }),
                                }));
                                ui.close();
                            }
                        });
                        if tb(ui, Icon::Branch, "Branch", true).on_hover_text("Create a new branch from where you are").clicked() {
                            acts.push(Action::Dialog(Dialog::NewBranch { at: None, name: String::new(), checkout: true }));
                        }
                        if tb(ui, Icon::Stash, "Stash", !tab.snap.status.is_clean()).on_hover_text("Put your uncommitted changes aside").clicked() {
                            acts.push(Action::Dialog(Dialog::Stash { message: String::new(), include_untracked: true }));
                        }
                        if tb(ui, Icon::Pop, "Pop", !tab.snap.stashes.is_empty()).on_hover_text("Bring back the last stash").clicked() {
                            acts.push(Action::Git { label: "Stash popped".into(), args: vec!["stash".into(), "pop".into()], background: false });
                        }
                        if tb(ui, Icon::Terminal, "Terminal", !tab.is_sample()).on_hover_text("Open a terminal here").clicked() {
                            acts.push(Action::OpenTerminal);
                        }
                        ui.spacing_mut().item_spacing.x = g as f32;
                        divider(ui, pal);

                        // Sync and changes, as compact numbers.
                        match tab.snap.current_ref().and_then(|r| r.upstream.clone().map(|u| (u, r.ahead, r.behind))) {
                            Some((up, ahead, behind)) => {
                                let ahead_col = if ahead > 0 { pal.accent } else { pal.muted };
                                let behind_col = if behind > 0 { pal.warn } else { pal.muted };
                                icon_label(ui, Icon::ArrowUp, 15.0, RichText::new(ahead.to_string()).size(17.0).strong().color(ahead_col), ahead_col)
                                    .on_hover_text(format!("{ahead} commit(s) to push to {up}"));
                                icon_label(ui, Icon::ArrowDown, 15.0, RichText::new(behind.to_string()).size(17.0).strong().color(behind_col), behind_col)
                                    .on_hover_text(format!("{behind} commit(s) to pull from {up}"));
                            }
                            None => {
                                ui.label(RichText::new("local only").color(pal.muted)).on_hover_text("This branch has no remote branch yet; Push creates it.");
                            }
                        }
                        divider(ui, pal);
                        let st = &tab.snap.status;
                        ui.label(RichText::new((st.unstaged.len() + st.staged.len()).to_string()).size(17.0).strong());
                        ui.label(RichText::new(format!("changes · {} staged", st.staged.len())).size(12.0).color(pal.muted));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::icon_button(ui, Icon::FolderOpen, pal.text, pal.card, pal.hover).on_hover_text("Open the folder").clicked() {
                                acts.push(Action::OpenInExplorer(None));
                            }
                            if icons::icon_button(ui, Icon::Refresh, pal.text, pal.card, pal.hover).on_hover_text("Refresh (F5)").clicked() {
                                acts.push(Action::Refresh);
                            }
                            let width = (ui.available_width() - 130.0).clamp(60.0, 240.0);
                            let field = crate::util::search_field(ui, &mut tab.search, "Find commits", width, pal);
                            // Search by message, author, changed text or file path (doc §Chercher).
                            crate::ui_extra::search_mode(ui, tab, &field, &mut acts);
                        });
                    });
                });

                if let Some(op) = tab.snap.pending {
                    ui.add_space(g as f32);
                    egui::Frame::new()
                        .fill(pal.warn.gamma_multiply(0.18))
                        .stroke(Stroke::new(1.0, pal.warn))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let conflicts = tab.snap.status.unstaged.iter().filter(|f| f.state == FileState::Conflicted).count();
                                ui.label(RichText::new(format!("A {} is in progress.", op.label())).strong());
                                if conflicts > 0 {
                                    ui.label(format!("{conflicts} file(s) have conflicts: open each one to choose block by block."));
                                } else {
                                    ui.label("No conflicts left: you can continue.");
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("Abort").clicked() {
                                        acts.push(Action::Git { label: format!("{} aborted", op.label()), args: vec![op.command().into(), "--abort".into()], background: false });
                                    }
                                    if ui.add_enabled(conflicts == 0, accent_button("Continue", pal)).clicked() {
                                        acts.push(Action::Git { label: format!("{} continued", op.label()), args: vec![op.command().into(), "--continue".into()], background: false });
                                    }
                                    if op.can_skip()
                                        && ui
                                            .button("Skip this commit")
                                            .on_hover_text(format!("git {} --skip\nDrops the commit it stopped on and goes on with the next one.", op.command()))
                                            .clicked()
                                    {
                                        acts.push(Action::Git { label: "Commit skipped".into(), args: vec![op.command().into(), "--skip".into()], background: false });
                                    }
                                });
                            });
                        });
                }
                if tab.snap.bisect.is_some() {
                    ui.add_space(g as f32);
                    crate::ui_extra::bisect_banner(ui, tab, pal, &mut acts);
                }
                if tab.snap.lfs.used && !tab.snap.lfs.installed {
                    ui.add_space(g as f32);
                    crate::ui_big::lfs_banner(ui, tab, pal, &mut acts);
                }
            });

        // The active tab and the header tile are one piece: hide the tile's top border
        // under the tab.
        if let Some(t) = self.active_tab_rect {
            let y = t.bottom();
            ui.painter().rect_filled(Rect::from_min_max(pos2(t.left() + 1.0, y - 1.0), pos2(t.right() - 1.0, y + 1.5)), CornerRadius::ZERO, pal.panel);
        }

        egui::Panel::bottom("statusbar")
            .frame(egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 4 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let where_ = tab.path.as_ref().map_or("Built-in sample project (read-only)".to_string(), |p| p.display().to_string());
                    ui.label(RichText::new(where_).size(11.5).color(pal.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} commits  ·  {} branches  ·  {} tags",
                                tab.snap.commits.len(),
                                tab.snap.refs.iter().filter(|r| r.kind == RefKind::Local).count(),
                                tab.snap.refs.iter().filter(|r| r.kind == RefKind::Tag).count(),
                            ))
                            .size(11.5)
                            .color(pal.muted),
                        );
                    });
                });
            });

        let side_frame = |left: i8, right: i8| egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(egui::Margin { left, right, top: 0, bottom: g });
        egui::Panel::left("sidebar")
            .default_size(260.0)
            .size_range(190.0..=440.0)
            .show_separator_line(false)
            .frame(side_frame(g * 2, 0))
            .show(ui, |ui| fill_tile(ui, pal, |ui| ui_side::show(ui, tab, pal, &mut acts)));

        egui::Panel::right("details")
            .default_size(380.0)
            .size_range(290.0..=660.0)
            .show_separator_line(false)
            .frame(side_frame(0, g * 2))
            .show(ui, |ui| fill_tile(ui, pal, |ui| ui_detail::show(ui, tab, pal, tree, sort_by_status, &mut acts)));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(egui::Margin { left: g, right: g, top: 0, bottom: g }))
            .show(ui, |ui| {
                fill_tile(ui, pal, |ui| {
                    if tab.loading {
                        ui_detail::loading(ui, "Reading the history…");
                    } else if tab.conflict.is_some() {
                        crate::ui_big::conflict_view(ui, tab, pal, &mut acts);
                    } else if tab.file_view.is_some() {
                        ui_detail::file_view(ui, tab, pal, diff_mode, &mut acts);
                    } else if tab.diff.is_some() {
                        ui_detail::diff_view(ui, tab, pal, diff_mode, false, &mut acts);
                    } else if tab.snap.commits.is_empty() && !tab.has_wip_row() {
                        ui.centered_and_justified(|ui| ui.label(RichText::new("This repository has no commits yet.").color(pal.muted)));
                    } else {
                        ui_graph::show(ui, tab, pal, columns, columns_visible, compact_graph, show_desc, &mut acts);
                    }
                });
            });

        acts.append(&mut self.actions);
        self.actions = acts;
    }

    fn home(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let mut acts = Vec::new();
        let busy = !self.busy.is_empty();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new("Welcome to Vegit").size(34.0).strong());
                ui.label(RichText::new("See your project's history, stage changes and share your work.").color(pal.muted));
                ui.add_space(20.0);
            });
            // Tiles are placed in exact rectangles: one width for the block, the same gap
            // between every tile and between rows, nothing sticking out on the right.
            let gap = 10.0;
            let full = ui.available_width();
            let width = full.min(880.0);
            let x0 = ui.cursor().left() + (full - width) / 2.0;
            let mut y = ui.cursor().top();
            let rect_of = |i: usize, n: usize, y: f32| {
                let w = (width - gap * (n as f32 - 1.0)) / n as f32;
                Rect::from_min_size(pos2(x0 + i as f32 * (w + gap), y), vec2(w, 10_000.0))
            };
            let mut bottom = y;
            let place = |ui: &mut egui::Ui, r: Rect, f: &mut dyn FnMut(&mut egui::Ui)| {
                let got = ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| f(ui)).response.rect;
                got.bottom()
            };

            // Row 1: open and clone.
            bottom = bottom.max(place(ui, rect_of(0, 2, y), &mut |ui| {
                home_tile(ui, pal, "row0", |ui| {
                    ui.label(RichText::new("Open a repository").strong().size(18.0));
                    ui.label(RichText::new("Pick a folder that is already a git project.").color(pal.muted));
                }, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.home.open_path).hint_text("C:\\path\\to\\project").desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        if ui.button("Browse…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().set_title("Open a repository").pick_folder() {
                                self.home.open_path = p.display().to_string();
                                acts.push(Action::OpenRepo(p));
                            }
                        }
                        if ui.add_enabled(!self.home.open_path.trim().is_empty(), accent_button("Open", pal)).clicked() {
                            acts.push(Action::OpenRepo(PathBuf::from(self.home.open_path.trim())));
                        }
                    });
                });
            }));
            bottom = bottom.max(place(ui, rect_of(1, 2, y), &mut |ui| {
                home_tile(ui, pal, "row0", |ui| {
                    ui.label(RichText::new("Clone from a URL").strong().size(18.0));
                    ui.label(RichText::new("Public or private: for a private one a sign-in window opens, or use Access first.").color(pal.muted));
                }, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.home.clone_url).hint_text("https://github.com/team/project.git").desired_width(f32::INFINITY));
                    // Button first from the right, then the field takes exactly what is left,
                    // so the row never pushes the tile wider.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Browse…").clicked() {
                                if let Some(p) = rfd::FileDialog::new().set_title("Clone into which folder?").pick_folder() {
                                    self.home.clone_dest = p.display().to_string();
                                }
                            }
                            ui.add(egui::TextEdit::singleline(&mut self.home.clone_dest).desired_width(ui.available_width()));
                        });
                    });
                    let url = self.home.clone_url.trim().to_string();
                    // Clone options, closed by default (doc: branch, recent history, submodules).
                    egui::CollapsingHeader::new(RichText::new("More options").color(pal.text2())).id_salt("clone_more").default_open(false).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Branch");
                            match &self.home.heads {
                                Some(Ok(heads)) if !heads.is_empty() => {
                                    let shown = if self.home.clone_branch.is_empty() { "default".to_string() } else { self.home.clone_branch.clone() };
                                    egui::ComboBox::from_id_salt("clone_branch").width(200.0).selected_text(shown).show_ui(ui, |ui| {
                                        crate::util::menu_pad(ui);
                                        ui.selectable_value(&mut self.home.clone_branch, String::new(), "default");
                                        for h in heads {
                                            ui.selectable_value(&mut self.home.clone_branch, h.clone(), h);
                                        }
                                    });
                                }
                                _ => {
                                    ui.add(egui::TextEdit::singleline(&mut self.home.clone_branch).hint_text("default").desired_width(160.0));
                                    if ui.add_enabled(!url.is_empty(), egui::Button::new("List branches")).on_hover_text("git ls-remote --heads").clicked() {
                                        acts.push(Action::ListHeads(url.clone()));
                                    }
                                    if let Some(Err(e)) = &self.home.heads {
                                        ui.label(RichText::new(crate::git::explain_remote_error(e)).size(11.5).color(pal.removed));
                                    }
                                }
                            }
                        });
                        ui.checkbox(&mut self.home.clone_recent, "Only recent history (faster)").on_hover_text("--depth 1: the full history can be loaded later from the repository menu");
                        ui.checkbox(&mut self.home.clone_submodules, "Include submodules").on_hover_text("--recurse-submodules");
                    });
                    if ui.add_enabled(!url.is_empty() && !busy, accent_button("Clone", pal)).clicked() {
                        let name = url.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']).next().unwrap_or("repo").to_string();
                        let dest = PathBuf::from(self.home.clone_dest.trim()).join(name);
                        acts.push(Action::Clone { url, dest });
                    }
                });
            }));
            y = bottom + gap;

            // Row 2: new, private access, sample. Their actions sit at the bottom left.
            bottom = bottom.max(place(ui, rect_of(0, 3, y), &mut |ui| {
                home_tile(ui, pal, "row1", |ui| {
                    ui.label(RichText::new("New repository").strong().size(18.0));
                    ui.label(RichText::new("Turn a folder into a git project.").color(pal.muted));
                }, |ui| {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Browse…").clicked() {
                                if let Some(p) = rfd::FileDialog::new().set_directory(dirs_home()).pick_folder() {
                                    self.home.init_path = p.display().to_string();
                                }
                            }
                            ui.add(egui::TextEdit::singleline(&mut self.home.init_path).hint_text("Folder").desired_width(ui.available_width()));
                        });
                    });
                    if ui.add_enabled(!self.home.init_path.trim().is_empty(), accent_button("Create", pal)).clicked() {
                        acts.push(Action::InitRepo(PathBuf::from(self.home.init_path.trim())));
                    }
                });
            }));
            bottom = bottom.max(place(ui, rect_of(1, 3, y), &mut |ui| {
                home_tile(ui, pal, "row1", |ui| {
                    ui.label(RichText::new("Private repositories").strong().size(18.0));
                    ui.label(RichText::new("Sign in once with a token or an SSH key. Free, like git itself.").color(pal.muted));
                }, |ui| {
                    if ui.add(accent_button("Set up access", pal)).clicked() {
                        acts.push(Action::Dialog(Dialog::Access(AccessForm { host: "github.com".into(), ..Default::default() })));
                    }
                });
            }));
            bottom = bottom.max(place(ui, rect_of(2, 3, y), &mut |ui| {
                home_tile(ui, pal, "row1", |ui| {
                    ui.label(RichText::new("Try the sample").strong().size(18.0));
                    ui.label(RichText::new("A small made-up project. Nothing on disk is touched.").color(pal.muted));
                }, |ui| {
                    if ui.button("Open the sample").clicked() {
                        acts.push(Action::OpenSample);
                    }
                });
            }));
            y = bottom + gap;

            // Recent: each line is one big clickable row.
            if !self.settings.recent.is_empty() {
                let recent = self.settings.recent.clone();
                bottom = bottom.max(place(ui, rect_of(0, 1, y), &mut |ui| {
                    home_tile(ui, pal, "recent", |ui| {
                        ui.label(RichText::new("Recent").strong().size(18.0));
                        ui.add_space(2.0);
                        for r in &recent {
                            let name = std::path::Path::new(r).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.clone());
                            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                            let p = ui.painter();
                            if resp.hovered() {
                                p.rect_filled(row, CornerRadius::same(5), pal.hover);
                            }
                            let g = p.layout_no_wrap(name, egui::FontId::proportional(14.0), pal.text);
                            let w = g.size().x;
                            p.galley(pos2(row.left() + 8.0, row.center().y - g.size().y / 2.0), g, pal.text);
                            p.text(pos2(row.left() + 18.0 + w, row.center().y), egui::Align2::LEFT_CENTER, r, egui::FontId::proportional(12.0), pal.muted);
                            if resp.on_hover_text("Open this repository").clicked() {
                                acts.push(Action::OpenRepo(PathBuf::from(r)));
                            }
                        }
                    }, |_| {});
                }));
            }
            // Tell the scroll area how tall the content is.
            let used = Rect::from_min_max(pos2(x0, ui.min_rect().top()), pos2(x0 + width, bottom + 16.0));
            ui.allocate_rect(used, Sense::hover());
        });
        // Tiles of a row take the height of the tallest one, measured on the previous frame.
        let ctx = ui.ctx().clone();
        let mut changed = false;
        ctx.data_mut(|d| {
            for row in ["row0", "row1", "recent"] {
                let id = egui::Id::unique(("home_row", row));
                let next: f32 = d.get_temp(id.with("next")).unwrap_or(0.0);
                let cur: f32 = d.get_temp(id).unwrap_or(0.0);
                if (next - cur).abs() > 0.5 {
                    d.insert_temp(id, next);
                    changed = true;
                }
                d.insert_temp(id.with("next"), 0.0f32);
            }
        });
        if changed {
            ctx.request_repaint();
        }
        self.actions.extend(acts);
    }

    fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let settings = &mut self.settings;
        let tab = if self.show_home { None } else { self.tabs.get(self.active) };
        let identity = tab.filter(|t| !t.loading).map(|t| &t.snap.identity);
        let mut keep = true;
        let mut acts = Vec::new();
        let modal = egui::Modal::new(egui::Id::unique("dialog"))
            .frame(egui::Frame::new().fill(pal.panel).stroke(Stroke::new(1.0, pal.border)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                ui.set_width(match dialog {
                    Dialog::NewPullRequest(_) => 620.0,
                    Dialog::Access(_) => 540.0,
                    Dialog::Settings => 700.0,
                    ref d => crate::ui_quick::width(d).unwrap_or(400.0),
                });
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                match &mut dialog {
                    Dialog::NewBranch { at, name, checkout } => {
                        ui.label(RichText::new("New branch").size(20.0).strong());
                        let from = at.as_deref().map_or("where you are now".to_string(), |a| if a.len() == 40 { git::short(a).to_string() } else { a.to_string() });
                        ui.label(RichText::new(format!("Starts from {from}")).color(pal.muted));
                        ui.add(egui::TextEdit::singleline(name).hint_text("feature/my-idea").desired_width(f32::INFINITY)).request_focus();
                        // "Work on it in a separate folder": a worktree instead of switching here.
                        let sep_id = egui::Id::unique("new_branch_separate");
                        let mut separate = ui.data(|d| d.get_temp::<bool>(sep_id)).unwrap_or(false);
                        ui.add_enabled(!separate, egui::Checkbox::new(checkout, "Switch to it right away"));
                        ui.checkbox(&mut separate, "Work on it in a separate folder").on_hover_text("git worktree add -b: its own folder and tab; this folder stays as it is");
                        ui.data_mut(|d| d.insert_temp(sep_id, separate));
                        let valid = !name.trim().is_empty() && !name.contains(' ');
                        if separate && (footer(ui, pal, "Next…", valid) || (enter && valid)) {
                            let start = at.clone().unwrap_or_else(|| "HEAD".into());
                            acts.push(Action::OpenWorktreeForm { branch: name.trim().to_string(), remote: false, new_from: Some(start) });
                            ui.data_mut(|d| d.remove::<bool>(sep_id));
                            keep = false;
                        } else if !separate && (footer(ui, pal, "Create", valid) || (enter && valid)) {
                            let n = name.trim().to_string();
                            let mut args: Vec<String> = if *checkout { vec!["switch".into(), "-c".into(), n.clone()] } else { vec!["branch".into(), n.clone()] };
                            if let Some(a) = at {
                                args.push(a.clone());
                            }
                            acts.push(Action::Git { label: format!("Created branch {n}"), args, background: false });
                            keep = false;
                        }
                    }
                    Dialog::RenameBranch { old, name } => {
                        ui.label(RichText::new(format!("Rename {old}")).size(20.0).strong());
                        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)).request_focus();
                        let valid = !name.trim().is_empty() && !name.contains(' ') && name.trim() != old;
                        if footer(ui, pal, "Rename", valid) || (enter && valid) {
                            acts.push(Action::Git { label: "Branch renamed".into(), args: vec!["branch".into(), "-m".into(), old.clone(), name.trim().into()], background: false });
                            keep = false;
                        }
                    }
                    Dialog::NewTag { at, name, message } => {
                        ui.label(RichText::new("New tag").size(20.0).strong());
                        ui.label(RichText::new(format!("On commit {}", git::short(at))).color(pal.muted));
                        ui.add(egui::TextEdit::singleline(name).hint_text("v1.0.0").desired_width(f32::INFINITY)).request_focus();
                        ui.add(egui::TextEdit::singleline(message).hint_text("Message (optional, makes an annotated tag)").desired_width(f32::INFINITY));
                        let valid = !name.trim().is_empty() && !name.contains(' ');
                        if footer(ui, pal, "Create tag", valid) {
                            let mut args = vec!["tag".to_string()];
                            if !message.trim().is_empty() {
                                args.extend(["-a".into(), name.trim().into(), "-m".into(), message.trim().into()]);
                            } else {
                                args.push(name.trim().into());
                            }
                            args.push(at.clone());
                            acts.push(Action::CreateTag { name: name.trim().to_string(), args });
                            keep = false;
                        }
                    }
                    Dialog::Stash { message, include_untracked } => {
                        ui.label(RichText::new("Stash changes").size(20.0).strong());
                        ui.label(RichText::new("Your uncommitted changes are put aside and the folder goes back to the last commit.").color(pal.muted));
                        ui.add(egui::TextEdit::singleline(message).hint_text("What were you working on? (optional)").desired_width(f32::INFINITY)).request_focus();
                        ui.checkbox(include_untracked, "Include new files");
                        if footer(ui, pal, "Stash", true) || enter {
                            let mut args = vec!["stash".to_string(), "push".into()];
                            if *include_untracked {
                                args.push("-u".into());
                            }
                            if !message.trim().is_empty() {
                                args.extend(["-m".into(), message.trim().into()]);
                            }
                            acts.push(Action::Git { label: "Changes stashed".into(), args, background: false });
                            keep = false;
                        }
                    }
                    Dialog::Confirm { title, body, confirm, then } => {
                        ui.label(RichText::new(title.as_str()).size(20.0).strong());
                        ui.label(body.as_str());
                        if footer(ui, pal, confirm, true) {
                            acts.push((**then).clone());
                            keep = false;
                        }
                    }
                    Dialog::NewPullRequest(form) => {
                        if pr_dialog(ui, form, pal, &mut acts) {
                            keep = false;
                        }
                    }
                    Dialog::Access(form) => access_dialog(ui, form, pal, &mut acts),
                    Dialog::Settings => settings_dialog(ui, settings, identity, pal, &mut acts),
                    d => {
                        if crate::ui_quick::dialog(ui, d, tab, pal, &mut acts) {
                            keep = false;
                        }
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) || ui.data(|d| d.get_temp::<bool>(egui::Id::unique("dialog_cancel"))).unwrap_or(false) {
                    keep = false;
                }
            });
        ctx.data_mut(|d| d.remove::<bool>(egui::Id::unique("dialog_cancel")));
        if modal.should_close() {
            keep = false;
        }
        if keep {
            self.dialog = Some(dialog);
        }
        self.actions.extend(acts);
    }

    fn toasts_ui(&mut self, ctx: &egui::Context, pal: &Palette) {
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(400.0);
                let mut dismiss = None;
                let mut chosen = None;
                for (i, t) in self.toasts.iter().enumerate() {
                    let color = match t.kind {
                        ToastKind::Info => pal.accent,
                        ToastKind::Success => pal.added,
                        ToastKind::Error => pal.removed,
                    };
                    let resp = egui::Frame::new()
                        .fill(pal.card)
                        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(egui::Margin { left: 16, right: 14, top: 10, bottom: 10 })
                        .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(70) })
                        .show(ui, |ui| {
                            let r = ui.max_rect();
                            ui.painter().circle_filled(pos2(r.left() - 7.0, r.top() + 8.0), 3.5, color);
                            match &t.action {
                                // Text on the left, the follow-up button on the right.
                                Some((label, action)) => {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&t.text).color(pal.text));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.add(accent_button(label, pal)).clicked() {
                                                chosen = Some((i, action.clone()));
                                            }
                                        });
                                    });
                                }
                                None => {
                                    ui.label(RichText::new(&t.text).color(pal.text));
                                }
                            }
                        })
                        .response;
                    // A toast with a button is closed by the button, so the click lands on it.
                    if t.action.is_none() && resp.interact(Sense::click()).clicked() {
                        dismiss = Some(i);
                    }
                    ui.add_space(6.0);
                }
                if let Some((i, action)) = chosen {
                    self.actions.push(action);
                    dismiss = Some(i);
                }
                if let Some(i) = dismiss {
                    self.toasts.remove(i);
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// "Create pull request" window: pick the service, the source repository and branch,
/// the destination repository and branch, then title, description and draft.
fn pr_dialog(ui: &mut egui::Ui, form: &mut PrForm, pal: &Palette, acts: &mut Vec<Action>) -> bool {
    let noun = if form.forge == Forge::GitLab { "Merge Request" } else { "Pull Request" };
    let mut done = false;
    // Title row with the pull request icon and a close cross.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        icons::paint(ui.painter(), r, Icon::PullRequest, pal.text2());
        ui.label(RichText::new(format!("Create {noun}")).size(20.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::ghost_button(ui, Icon::Close, 16.0, pal.text2(), pal.hover).on_hover_text("Close").clicked() {
                ui.data_mut(|d| d.insert_temp(egui::Id::unique("dialog_cancel"), true));
            }
        });
    });
    ui.add_space(8.0);

    // Service tabs.
    ui.horizontal(|ui| {
        let total = ui.available_width();
        let w = (total / 4.0).floor() - 6.0;
        for forge in [Forge::GitHub, Forge::GitLab, Forge::Bitbucket, Forge::Azure] {
            let available = form.remotes.iter().any(|r| r.forge == forge);
            let active = form.forge == forge;
            let (r, resp) = ui.allocate_exact_size(vec2(w, 58.0), if available { Sense::click() } else { Sense::hover() });
            let p = ui.painter();
            if resp.hovered() && available && !active {
                p.rect_filled(r, CornerRadius::same(10), pal.hover);
            }
            let color = if active { pal.text } else if available { pal.text2() } else { pal.muted.gamma_multiply(0.6) };
            // A neutral badge with the service's initial (no brand logos).
            let badge = Rect::from_center_size(pos2(r.center().x, r.top() + 18.0), vec2(26.0, 26.0));
            p.rect_stroke(badge, CornerRadius::same(8), Stroke::new(1.6, color), egui::StrokeKind::Inside);
            p.text(badge.center(), Align2::CENTER_CENTER, &forge.name()[..1], egui::FontId::proportional(14.0), color);
            p.text(pos2(r.center().x, r.bottom() - 11.0), Align2::CENTER_CENTER, forge.name(), egui::FontId::proportional(12.5), color);
            if active {
                let line = Rect::from_center_size(pos2(r.center().x, r.bottom() + 1.0), vec2(w * 0.6, 2.0));
                p.rect_filled(line, CornerRadius::same(1), pal.accent);
            }
            let resp = if available { resp } else { resp.on_hover_text(format!("No remote of this repository is on {}", forge.name())) };
            if resp.clicked() && available && !active {
                form.select_forge(forge);
            }
        }
    });
    ui.add_space(4.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, CornerRadius::ZERO, pal.border);
    ui.add_space(10.0);

    // From → To.
    let of_forge: Vec<usize> = (0..form.remotes.len()).filter(|&i| form.remotes[i].forge == form.forge).collect();
    let remote_label = |f: &PrForm, i: usize| f.remotes.get(i).map_or("Select…".to_string(), |r| format!("{} · {}", r.remote, r.path));
    let col_w = ((ui.available_width() - 40.0) / 2.0).floor();
    egui::Grid::new("pr_grid").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label(RichText::new("From repo").color(pal.text2()));
        ui.label("");
        ui.label(RichText::new("To repo").color(pal.text2()));
        ui.end_row();

        let from_text = remote_label(form, form.from);
        let mut changed_from = false;
        egui::ComboBox::from_id_salt("pr_from_repo").width(col_w).selected_text(from_text).show_ui(ui, |ui| {
            crate::util::menu_pad(ui);
            for &i in &of_forge {
                let label = remote_label(form, i);
                changed_from |= ui.selectable_value(&mut form.from, i, label).changed();
            }
        });
        ui.add(icons::image(ui.ctx(), Icon::StatusRenamed, 18.0, pal.text2()));
        let to_text = remote_label(form, form.to);
        let mut changed_to = false;
        egui::ComboBox::from_id_salt("pr_to_repo").width(col_w).selected_text(to_text).show_ui(ui, |ui| {
            crate::util::menu_pad(ui);
            for &i in &of_forge {
                let label = remote_label(form, i);
                changed_to |= ui.selectable_value(&mut form.to, i, label).changed();
            }
        });
        if changed_to {
            form.to_branch = form.default_target();
        }
        let _ = changed_from;
        ui.end_row();

        ui.label(RichText::new("Branch").color(pal.text2()));
        ui.label("");
        ui.label(RichText::new("Branch").color(pal.text2()));
        ui.end_row();

        let mut from_branches: Vec<String> = form.branches_of(form.from).iter().map(|s| s.to_string()).collect();
        if let Some(cur) = &form.current {
            if !from_branches.contains(cur) {
                from_branches.insert(0, cur.clone());
            }
        }
        let from_sel = if form.from_branch.is_empty() { "Select…".to_string() } else { form.from_branch.clone() };
        egui::ComboBox::from_id_salt("pr_from_branch").width(col_w).selected_text(from_sel).show_ui(ui, |ui| {
            crate::util::menu_pad(ui);
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for b in &from_branches {
                    ui.selectable_value(&mut form.from_branch, b.clone(), b);
                }
            });
        });
        ui.label("");
        let to_branches: Vec<String> = form.branches_of(form.to).iter().map(|s| s.to_string()).collect();
        let to_sel = if form.to_branch.is_empty() { "Select…".to_string() } else { form.to_branch.clone() };
        egui::ComboBox::from_id_salt("pr_to_branch").width(col_w).selected_text(to_sel).show_ui(ui, |ui| {
            crate::util::menu_pad(ui);
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for b in &to_branches {
                    ui.selectable_value(&mut form.to_branch, b.clone(), b);
                }
            });
        });
        ui.end_row();
    });

    let pushed = form.from_is_pushed();
    if !form.from_branch.is_empty() && !pushed {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let remote = form.remotes.get(form.from).map_or("the remote", |r| r.remote.as_str());
            ui.label(RichText::new(format!("“{}” isn't on {remote} yet.", form.from_branch)).color(pal.warn));
            if form.current.as_deref() == Some(form.from_branch.as_str()) && ui.small_button("Push it").clicked() {
                acts.push(Action::Push { force: false });
            }
        });
    }
    let same = form.from == form.to && form.from_branch == form.to_branch;
    if same && !form.from_branch.is_empty() {
        ui.label(RichText::new("Pick two different branches.").color(pal.warn));
    }

    ui.add_space(10.0);
    ui.label(RichText::new("Title").color(pal.text2()));
    ui.add(egui::TextEdit::singleline(&mut form.title).hint_text(format!("{noun} title")).desired_width(f32::INFINITY));
    ui.add_space(6.0);
    ui.label(RichText::new("Description").color(pal.text2()));
    ui.add(egui::TextEdit::multiline(&mut form.body).hint_text(format!("{noun} description")).desired_rows(6).desired_width(f32::INFINITY));
    ui.horizontal(|ui| {
        ui.checkbox(&mut form.draft, "Submit as draft");
        ui.label(RichText::new("(?)").size(11.5).color(pal.accent)).on_hover_text("A draft can't be merged yet; reviewers know it's still in progress.");
    });
    ui.add_space(10.0);

    let valid = !form.title.trim().is_empty() && !form.from_branch.is_empty() && !form.to_branch.is_empty() && pushed && !same;
    let w = ((ui.available_width() - 10.0) / 2.0).floor();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let cancel = egui::Button::new(RichText::new("Cancel").color(pal.text2()))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(CornerRadius::same(10))
            .min_size(vec2(w, 36.0));
        if ui.add(cancel).clicked() {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("dialog_cancel"), true));
        }
        let create = accent_button(format!("Create {noun}"), pal).min_size(vec2(w, 36.0));
        if ui.add_enabled(valid, create).clicked() {
            acts.push(Action::CreatePullRequest(form.clone()));
            done = true;
        }
    });
    done
}

/// One preset as a full-width clickable row: colour swatch, name, fonts.
fn preset_row(ui: &mut egui::Ui, t: ThemeChoice, selected: bool, pal: &Palette) -> egui::Response {
    let tp = Palette::of(t);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(260.0), 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(r, CornerRadius::same(8), pal.selected);
    } else if resp.hovered() {
        p.rect_filled(r, CornerRadius::same(8), pal.hover);
    }
    let sw = Rect::from_min_size(pos2(r.left() + 8.0, r.center().y - 9.0), vec2(64.0, 18.0));
    p.rect_filled(sw, CornerRadius::same(9), tp.canvas);
    for (k, col) in [tp.panel, tp.accent, tp.lane(1), tp.lane(2)].iter().enumerate() {
        p.circle_filled(pos2(sw.left() + 10.0 + k as f32 * 15.0, sw.center().y), 5.5, *col);
    }
    p.text(pos2(sw.right() + 12.0, r.center().y), Align2::LEFT_CENTER, t.label(), egui::FontId::proportional(13.5), pal.text);
    p.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, t.fonts().label, egui::FontId::proportional(11.0), pal.muted);
    resp
}

/// Font picker: the preset's font, a list of installed fonts, or any font file.
fn font_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<String>, preset_label: &str) {
    let fonts = theme::installed_fonts();
    let shown = match value {
        None => format!("Preset ({preset_label})"),
        Some(f) => fonts.iter().find(|(_, file)| file == f).map(|(n, _)| n.to_string()).unwrap_or_else(|| {
            std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.clone())
        }),
    };
    egui::ComboBox::from_id_salt(id).width(260.0).selected_text(shown).show_ui(ui, |ui| {
        crate::util::menu_pad(ui);
        if ui.selectable_label(value.is_none(), format!("Preset ({preset_label})")).clicked() {
            *value = None;
        }
        for (name, file) in fonts {
            if ui.selectable_label(value.as_deref() == Some(file), name).clicked() {
                *value = Some(file.to_string());
            }
        }
        ui.separator();
        if ui.button("Other font file…").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc"]).pick_file() {
                *value = Some(p.display().to_string());
            }
        }
    });
}

/// The settings window: appearance (preset, interface colour, fonts, branch colours)
/// and graph options. Changes apply immediately and are saved with the window.
fn settings_dialog(ui: &mut egui::Ui, s: &mut Settings, identity: Option<&git::Identity>, pal: &Palette, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Settings").size(22.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::ghost_button(ui, Icon::Close, 16.0, pal.text2(), pal.hover).on_hover_text("Close").clicked() {
                ui.data_mut(|d| d.insert_temp(egui::Id::unique("dialog_cancel"), true));
            }
        });
    });
    ui.add_space(6.0);
    egui::ScrollArea::vertical().max_height(620.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // Appearance
        card(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Theme preset").strong());
            ui.label(RichText::new("The starting point; the options below adjust it.").size(12.0).color(pal.muted));
            ui.add_space(4.0);
            for t in ThemeChoice::ALL {
                if preset_row(ui, t, s.theme == t, pal).clicked() {
                    s.theme = t;
                }
            }
        });
        ui.add_space(8.0);
        card(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Look").strong());
            ui.add_space(4.0);
            egui::Grid::new("look_grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                ui.label(RichText::new("Interface colour").color(pal.text2()));
                ui.horizontal(|ui| {
                    let mut custom = s.accent.is_some();
                    if ui.checkbox(&mut custom, "Custom").changed() {
                        s.accent = if custom { Some(pal.accent.to_array()[..3].try_into().unwrap_or([45, 140, 255])) } else { None };
                    }
                    if let Some(c) = s.accent.as_mut() {
                        ui.color_edit_button_srgb(c);
                    } else {
                        ui.label(RichText::new("from the preset").color(pal.muted));
                    }
                });
                ui.end_row();

                let preset_fonts = s.theme.fonts();
                let (ui_label, code_label) = preset_fonts.label.split_once(" · ").unwrap_or((preset_fonts.label, preset_fonts.label));
                ui.label(RichText::new("Interface font").color(pal.text2()));
                font_combo(ui, "ui_font", &mut s.ui_font, ui_label);
                ui.end_row();
                ui.label(RichText::new("Code font").color(pal.text2()));
                font_combo(ui, "code_font", &mut s.code_font, code_label);
                ui.end_row();
                ui.label(RichText::new("Text size").color(pal.text2()));
                ui.add(egui::Slider::new(&mut s.text_size, 11.0..=18.0).step_by(0.5));
                ui.end_row();

                ui.label(RichText::new("Branch colours").color(pal.text2()));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("lane_mode").width(200.0).selected_text(s.lane_mode.label()).show_ui(ui, |ui| {
                        crate::util::menu_pad(ui);
                        for m in LaneMode::ALL {
                            ui.selectable_value(&mut s.lane_mode, m, m.label());
                        }
                    });
                    match s.lane_mode {
                        LaneMode::Unicolor => {
                            ui.color_edit_button_srgb(&mut s.lane_color);
                        }
                        LaneMode::Random => {
                            if ui.button("Shuffle").clicked() {
                                s.lane_seed = s.lane_seed.wrapping_mul(31).wrapping_add(17);
                            }
                        }
                        _ => {}
                    }
                });
                ui.end_row();
                ui.label("");
                // Preview of the eight lane colours as little branch lines.
                let (r, _) = ui.allocate_exact_size(vec2(260.0, 26.0), Sense::hover());
                let p = ui.painter();
                for (i, c) in pal.lanes.iter().enumerate() {
                    let x = r.left() + 12.0 + i as f32 * 30.0;
                    p.line_segment([pos2(x, r.top() + 2.0), pos2(x, r.bottom() - 2.0)], Stroke::new(2.5, *c));
                    p.circle_filled(pos2(x, r.center().y), 6.0, *c);
                    p.circle_filled(pos2(x, r.center().y), 3.0, pal.card);
                }
                ui.end_row();
            });
        });
        ui.add_space(8.0);
        card(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Background").strong());
            ui.label(RichText::new("A halftone pattern behind the tiles, in the interface colour.").size(12.0).color(pal.muted));
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new("Advanced background settings").color(pal.text2()))
                .id_salt("bg_advanced")
                .default_open(true)
                .show(ui, |ui| {
                    use crate::backdrop::{Gradient, Graphic};
                    egui::Grid::new("bg_grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                        ui.label(RichText::new("Pattern type").color(pal.text2()));
                        egui::ComboBox::from_id_salt("bg_graphic").width(220.0).selected_text(s.bg_graphic.label()).show_ui(ui, |ui| {
                            crate::util::menu_pad(ui);
                            for g in Graphic::ALL {
                                ui.selectable_value(&mut s.bg_graphic, g, g.label());
                            }
                        });
                        ui.end_row();
                        let random = s.bg_graphic == Graphic::Random;
                        ui.label(RichText::new("Gradient").color(pal.text2()));
                        ui.add_enabled_ui(!random, |ui| {
                            egui::ComboBox::from_id_salt("bg_gradient").width(220.0).selected_text(s.bg_gradient.label()).show_ui(ui, |ui| {
                                crate::util::menu_pad(ui);
                                for g in Gradient::ALL {
                                    ui.selectable_value(&mut s.bg_gradient, g, g.label());
                                }
                            });
                        })
                        .response
                        .on_hover_text("How the halftone is sampled: which areas get big shapes and which get small ones.");
                        ui.end_row();
                        ui.label(RichText::new("Strength").color(pal.text2()));
                        ui.add(egui::Slider::new(&mut s.bg_strength, 0.0..=1.0).show_value(false));
                        ui.end_row();
                        ui.label(RichText::new("Cell size").color(pal.text2()));
                        ui.add(egui::Slider::new(&mut s.bg_cell, 8.0..=40.0).step_by(1.0).suffix(" pt"));
                        ui.end_row();
                    });
                    if s.bg_graphic == Graphic::Random {
                        ui.label(RichText::new("Random picks a pattern and a gradient at each launch; F9 picks another.").size(12.0).color(pal.muted));
                    }
                });
        });
        ui.add_space(8.0);
        card(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("History graph").strong());
            ui.add_space(4.0);
            ui.checkbox(&mut s.pin_head, "Keep the current branch on the left of the graph");
            ui.checkbox(&mut s.show_descriptions, "Show commit descriptions after the summary");
            ui.checkbox(&mut s.show_avatars, "Show profile pictures")
                .on_hover_text("Asks Gravatar (or GitHub for GitHub addresses) with a hash of each author's e-mail.");
            if ui.button("Reset column widths").clicked() {
                s.columns = crate::app::DEFAULT_COLUMNS;
                s.columns_visible = [true; 6];
            }
        });
        ui.add_space(8.0);
        card(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            crate::ui_quick::identity_card(ui, identity, pal, acts);
        });
    });
    ui.add_space(10.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(accent_button("Done", pal)).clicked() {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("dialog_cancel"), true));
        }
        if ui.button("Reset appearance").on_hover_text("Back to the preset's colours and fonts").clicked() {
            let d = Settings::default();
            s.accent = None;
            s.ui_font = None;
            s.code_font = None;
            s.text_size = d.text_size;
            s.lane_mode = d.lane_mode;
        }
    });
}

/// The "Access" window: everything needed to reach private repositories, without paying.
fn access_dialog(ui: &mut egui::Ui, form: &mut AccessForm, pal: &Palette, acts: &mut Vec<Action>) {
    if !form.checked {
        form.helper = access::credential_helper();
        form.ssh_key = access::ssh_public_key();
        form.checked = true;
    }
    ui.label(RichText::new("Private repositories").size(20.0).strong());
    ui.label(
        RichText::new("Git is free and works with private repositories: it just needs to know who you are. Pick one of the ways below; you only do it once per site.")
            .color(pal.muted),
    );
    ui.add_space(8.0);

    card(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("1 · Sign in with a token (HTTPS)").strong());
        ui.horizontal(|ui| {
            for h in ["github.com", "gitlab.com", "bitbucket.org"] {
                if ui.selectable_label(form.host == h, h).clicked() {
                    form.host = h.to_string();
                }
            }
        });
        egui::Grid::new("access_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Site");
            ui.add(egui::TextEdit::singleline(&mut form.host).desired_width(330.0));
            ui.end_row();
            ui.label("User name");
            ui.add(egui::TextEdit::singleline(&mut form.user).hint_text("your-login").desired_width(330.0));
            ui.end_row();
            ui.label("Token");
            ui.add(egui::TextEdit::singleline(&mut form.token).password(true).hint_text("personal access token").desired_width(330.0));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if let Some((tokens, _)) = access::host_pages(&form.host) {
                ui.hyperlink_to("Create a token on the site ↗", tokens);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = !form.host.trim().is_empty() && !form.user.trim().is_empty() && !form.token.trim().is_empty() && form.helper.is_some();
                if ui.add_enabled(ok, accent_button("Save sign-in", pal)).clicked() {
                    acts.push(Action::SaveToken { host: form.host.clone(), user: form.user.clone(), token: form.token.clone() });
                    form.token.clear();
                }
            });
        });
        match &form.helper {
            Some(h) if h.contains("manager") => {
                ui.label(
                    RichText::new("Git Credential Manager keeps the token in the Windows vault. You can also just clone: it opens a browser sign-in the first time.")
                        .size(11.5)
                        .color(pal.muted),
                );
            }
            Some(h) => {
                ui.label(RichText::new(format!("Sign-ins are kept by the \"{h}\" credential helper.")).size(11.5).color(pal.muted));
            }
            None => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("No credential helper is set up, so git can't remember sign-ins.").size(11.5).color(pal.warn));
                    if ui.small_button("Use Git Credential Manager").clicked() {
                        acts.push(Action::UseCredentialManager);
                        form.checked = false;
                    }
                });
            }
        }
    });
    ui.add_space(6.0);

    card(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("2 · Or use an SSH key (git@… addresses)").strong());
        match &form.ssh_key {
            Some((name, key)) => {
                let shown = if key.len() > 64 { format!("{}…{}", &key[..40], &key[key.len() - 18..]) } else { key.clone() };
                ui.label(RichText::new(format!("{name}: {shown}")).monospace().size(11.5));
                ui.horizontal(|ui| {
                    if ui.button("Copy public key").clicked() {
                        acts.push(Action::Copy(key.clone()));
                    }
                    if let Some((_, keys)) = access::host_pages(&form.host) {
                        ui.hyperlink_to("Add it to your account ↗", keys);
                    }
                });
            }
            None => {
                ui.label(RichText::new("No SSH key on this computer yet.").color(pal.muted));
                if ui.button("Create an SSH key").clicked() {
                    acts.push(Action::CreateSshKey);
                    form.checked = false;
                }
            }
        }
    });
    ui.add_space(6.0);

    card(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("3 · Check that it works").strong());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut form.test_url).hint_text("https://github.com/you/private-repo.git").desired_width(ui.available_width() - 80.0));
            if ui.add_enabled(!form.test_url.trim().is_empty(), egui::Button::new("Test")).clicked() {
                acts.push(Action::TestAccess(form.test_url.trim().to_string()));
            }
        });
    });

    ui.add_space(10.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button("Close").clicked() {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("dialog_cancel"), true));
        }
    });
}

/// Minimize, maximize and close, drawn in the theme since the system frame is hidden.
fn window_buttons(ui: &mut egui::Ui, pal: &Palette) {
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    for kind in 0..3 {
        // Laid out right to left: close, maximize, minimize.
        let (r, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
        let hover = resp.hovered();
        let p = ui.painter();
        if hover {
            p.rect_filled(r, CornerRadius::same(8), if kind == 0 { pal.removed } else { pal.hover });
        }
        let icon = match kind {
            0 => Icon::Close,
            1 if maximized => Icon::WindowRestore,
            1 => Icon::WindowMaximize,
            _ => Icon::WindowMinimize,
        };
        let color = if hover && kind == 0 { Color32::WHITE } else { pal.text2() };
        icons::paint(p, Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, color);
        if resp.clicked() {
            let cmd = match kind {
                0 => egui::ViewportCommand::Close,
                1 => egui::ViewportCommand::Maximized(!maximized),
                _ => egui::ViewportCommand::Minimized(true),
            };
            ui.ctx().send_viewport_cmd(cmd);
        }
    }
}

/// Lets the borderless window be resized from its edges and corners.
fn resize_edges(ctx: &egui::Context, window: Rect) {
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let m = 6.0;
    let (l, r, t, b) = (pos.x - window.left() < m, window.right() - pos.x < m, pos.y - window.top() < m, window.bottom() - pos.y < m);
    use egui::{CursorIcon as C, ResizeDirection as D};
    let dir = match (l, r, t, b) {
        (true, _, true, _) => Some((D::NorthWest, C::ResizeNwSe)),
        (_, true, true, _) => Some((D::NorthEast, C::ResizeNeSw)),
        (true, _, _, true) => Some((D::SouthWest, C::ResizeNeSw)),
        (_, true, _, true) => Some((D::SouthEast, C::ResizeNwSe)),
        (true, ..) => Some((D::West, C::ResizeHorizontal)),
        (_, true, ..) => Some((D::East, C::ResizeHorizontal)),
        (_, _, true, _) => Some((D::North, C::ResizeVertical)),
        (.., true) => Some((D::South, C::ResizeVertical)),
        _ => None,
    };
    if let Some((d, cursor)) = dir {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(d));
        }
    }
}

/// A home-screen tile: its body at the top, its footer (inputs and buttons) at the bottom
/// left, and the height of the tallest tile of the same row (measured on the previous frame).
fn home_tile(ui: &mut egui::Ui, pal: &Palette, row: &str, body: impl FnOnce(&mut egui::Ui), footer: impl FnOnce(&mut egui::Ui)) {
    let id = egui::Id::unique(("home_row", row));
    let target: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    // Remember this tile's footer height between frames (tiles are told apart by position).
    let foot_id = egui::Id::unique(("home_foot", row, ui.max_rect().left() as i32));
    let foot_prev: f32 = ui.data(|d| d.get_temp(foot_id)).unwrap_or(0.0);
    tile(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            let spacing = ui.spacing().item_spacing.y;
            // Measure the parts alone (scopes), so the padding never feeds on itself.
            let body_h = ui.scope(|ui| body(ui)).response.rect.height();
            let gap_min = 8.0;
            let extra = (target - body_h - foot_prev - 2.0 * spacing - gap_min).max(0.0);
            ui.add_space(gap_min + extra);
            let foot_h = ui.scope(|ui| footer(ui)).response.rect.height();
            let natural = body_h + foot_h + 2.0 * spacing + gap_min;
            ui.data_mut(|d| {
                d.insert_temp(foot_id, foot_h);
                let key = id.with("next");
                let m: f32 = d.get_temp(key).unwrap_or(0.0);
                d.insert_temp(key, m.max(natural));
            });
        });
    });
}

/// Height of the tab strip; tabs touch the header tile right below it.
const TAB_H: f32 = 36.0;
/// Radius of the curves where the active tab flares into the header tile.
const TAB_FLARE: f32 = 8.0;

/// The active tab: rounded top corners and inverted curves at the bottom, so it flows into
/// the header tile like a folder tab. Filled with the tile colour and outlined like it.
fn paint_tab_shape(p: &egui::Painter, r: Rect, pal: &Palette) {
    use egui::epaint::{Mesh, PathShape};
    let (l, rt, t, b) = (r.left(), r.right(), r.top(), r.bottom());
    let top_r = 8.0;
    let f = TAB_FLARE;
    // Body.
    p.rect_filled(Rect::from_min_max(pos2(l, t), pos2(rt, b + 1.0)), CornerRadius { nw: top_r as u8, ne: top_r as u8, sw: 0, se: 0 }, pal.panel);
    // Flares: a fan from the bottom corner to a quarter circle bulging toward it.
    let arc = |cx: f32, cy: f32, a0: f32, a1: f32| -> Vec<egui::Pos2> {
        (0..=10).map(|k| {
            let a = a0 + (a1 - a0) * k as f32 / 10.0;
            pos2(cx + f * a.cos(), cy + f * a.sin())
        }).collect()
    };
    use std::f32::consts::{FRAC_PI_2, PI};
    let left_arc = arc(l - f, b - f, FRAC_PI_2, 0.0); // from (l-f, b) to (l, b-f)
    let right_arc = arc(rt + f, b - f, FRAC_PI_2, PI); // from (rt+f, b) to (rt, b-f)
    for (corner, pts) in [(pos2(l, b), &left_arc), (pos2(rt, b), &right_arc)] {
        let mut mesh = Mesh::default();
        mesh.colored_vertex(corner, pal.panel);
        for q in pts.iter() {
            mesh.colored_vertex(*q, pal.panel);
        }
        for k in 1..pts.len() as u32 {
            mesh.add_triangle(0, k, k + 1);
        }
        p.add(egui::Shape::mesh(mesh));
    }
    // Outline: left flare, up, over the top, down, right flare.
    let mut outline: Vec<egui::Pos2> = left_arc.clone();
    outline.push(pos2(l, t + top_r));
    outline.extend((0..=6).map(|k| {
        let a = PI + FRAC_PI_2 * k as f32 / 6.0;
        pos2(l + top_r + top_r * a.cos(), t + top_r + top_r * a.sin())
    }));
    outline.extend((0..=6).map(|k| {
        let a = PI + FRAC_PI_2 + FRAC_PI_2 * k as f32 / 6.0;
        pos2(rt - top_r + top_r * a.cos(), t + top_r + top_r * a.sin())
    }));
    outline.push(pos2(rt, b - f));
    outline.extend(right_arc.iter().rev().copied());
    p.add(PathShape::line(outline, Stroke::new(1.0, pal.border)));
}

/// Thin vertical line between groups of the header.
fn divider(ui: &mut egui::Ui, pal: &Palette) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 26.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, pal.border);
}

/// Cancel / confirm buttons at the bottom of a dialog. Returns true when confirmed.
pub(crate) fn footer(ui: &mut egui::Ui, pal: &Palette, confirm: &str, enabled: bool) -> bool {
    ui.add_space(10.0);
    let mut confirmed = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            confirmed = ui.add_enabled(enabled, accent_button(confirm, pal)).clicked();
            if ui.button("Cancel").clicked() {
                ui.data_mut(|d| d.insert_temp(egui::Id::unique("dialog_cancel"), true));
            }
        });
    });
    confirmed
}
