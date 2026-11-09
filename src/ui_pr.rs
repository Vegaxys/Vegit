//! The pull request view: the central panel shows the pull request (title, description,
//! comments, commits, checks, files) and the right panel its people and labels. The
//! sidebar is hidden meanwhile. Every edit is one call to GitHub, then the view reloads.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};

use crate::app::{Action, RepoTab};
use crate::pr::{PrDetail, PrEdit};
use crate::theme::Palette;
use crate::util::{accent_button, avatar};

const MERGE: [(&str, &str); 3] = [("merge", "Create a merge commit"), ("squash", "Squash and merge"), ("rebase", "Rebase and merge")];

fn state_chip(ui: &mut egui::Ui, d: &PrDetail, pal: &Palette) {
    let (text, color) = match (d.state.as_str(), d.draft) {
        ("merged", _) => ("Merged", Color32::from_rgb(137, 87, 229)),
        ("closed", _) => ("Closed", pal.removed),
        (_, true) => ("Draft", pal.muted),
        _ => ("Open", pal.added),
    };
    egui::Frame::new().fill(color).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::symmetric(10, 3)).show(ui, |ui| {
        ui.label(RichText::new(text).color(Color32::WHITE).strong());
    });
}

fn person(ui: &mut egui::Ui, name: &str, pal: &Palette) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
        avatar(ui.painter(), r.center(), 9.0, name, "");
        ui.label(RichText::new(name).color(pal.text2()));
    });
}

/// The central part: header, tabs and the page.
pub fn main(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let Some(v) = tab.pr_view.as_mut() else { return };
    let noun = "pull request";
    ui.horizontal(|ui| {
        if ui.button("← Back").on_hover_text("Back to the history").clicked() {
            acts.push(Action::ClosePrView);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Refresh").clicked() {
                acts.push(Action::RefreshPr);
            }
            if let Some(Ok(d)) = &v.detail {
                if ui.button("Open in the browser").clicked() {
                    acts.push(Action::OpenUrl(d.url.clone()));
                }
            }
            if v.busy {
                ui.add(egui::Spinner::new().size(14.0));
            }
        });
    });
    let d = match &v.detail {
        None => return crate::ui_detail::loading(ui, "Reading the pull request…"),
        Some(Err(e)) => {
            ui.add_space(20.0);
            ui.label(RichText::new(e).color(pal.removed));
            return;
        }
        Some(Ok(d)) => d.clone(),
    };
    ui.add_space(6.0);

    // Title, editable in place.
    match v.title_edit.as_mut() {
        Some(t) => {
            let mut close = false;
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(t).font(egui::FontId::proportional(20.0)).desired_width(ui.available_width() - 150.0));
                if ui.add(accent_button("Save", pal)).clicked() {
                    acts.push(Action::EditPr(PrEdit::Title(t.trim().to_string())));
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
            if close {
                v.title_edit = None;
            }
        }
        None => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{}  #{}", d.title, d.number)).size(21.0).strong());
                if ui.small_button("✎").on_hover_text("Edit the title").clicked() {
                    v.title_edit = Some(d.title.clone());
                }
            });
        }
    }
    ui.horizontal_wrapped(|ui| {
        state_chip(ui, &d, pal);
        ui.label(RichText::new(format!("{} wants to merge {} commit{} into", d.author, d.commits.len(), if d.commits.len() == 1 { "" } else { "s" })).color(pal.text2()));
        ui.label(RichText::new(&d.target).monospace().color(pal.accent));
        ui.label(RichText::new("from").color(pal.text2()));
        ui.label(RichText::new(&d.source).monospace().color(pal.accent));
        ui.label(RichText::new(format!("+{}", d.additions)).color(pal.added).strong());
        ui.label(RichText::new(format!("−{}", d.deletions)).color(pal.removed).strong());
        ui.label(RichText::new(format!("opened {}", d.created)).size(12.0).color(pal.muted));
    });
    ui.add_space(6.0);

    // Tabs: Overview / Commits / Checks / Files changed.
    ui.horizontal(|ui| {
        let tabs = [
            "Overview".to_string(),
            format!("Commits  {}", d.commits.len()),
            format!("Checks  {}", d.checks.len()),
            format!("Files changed  {}", d.files.len()),
        ];
        for (i, t) in tabs.iter().enumerate() {
            if ui.selectable_label(v.page == i, RichText::new(t.as_str()).size(14.0)).clicked() {
                v.page = i;
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match v.page {
            1 => {
                for c in &d.commits {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(crate::git::short(&c.sha)).monospace().color(pal.muted));
                        ui.label(RichText::new(&c.summary).color(pal.text));
                        ui.label(RichText::new(&c.author).size(12.0).color(pal.text2()));
                    });
                    if !c.coauthors.is_empty() {
                        ui.label(RichText::new(format!("      co-authored by {}", c.coauthors.join(", "))).size(12.0).color(pal.muted));
                    }
                }
            }
            2 => {
                if d.checks.is_empty() {
                    ui.label(RichText::new("No checks ran on the last commit.").color(pal.muted));
                }
                for c in &d.checks {
                    let (mark, color) = match c.state.as_str() {
                        "success" => ("✔", pal.added),
                        "failure" | "error" | "timed_out" | "cancelled" | "action_required" => ("✖", pal.removed),
                        "pending" | "queued" | "in_progress" => ("●", pal.warn),
                        _ => ("○", pal.muted),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(mark).color(color));
                        ui.label(&c.name);
                        ui.label(RichText::new(&c.state).size(12.0).color(pal.muted));
                    });
                }
            }
            3 => {
                for f in &d.files {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("+{}", f.additions)).monospace().color(pal.added));
                        ui.label(RichText::new(format!("−{}", f.deletions)).monospace().color(pal.removed));
                        ui.label(RichText::new(&f.path).color(pal.text));
                        ui.label(RichText::new(&f.status).size(11.5).color(pal.muted));
                    });
                }
            }
            _ => overview(ui, v, &d, pal, noun, acts),
        }
    });
}

fn overview(ui: &mut egui::Ui, v: &mut crate::pr::PrView, d: &PrDetail, pal: &Palette, noun: &str, acts: &mut Vec<Action>) {
    // Checks and merge status at a glance, then the merge button.
    let failing = d.checks.iter().filter(|c| matches!(c.state.as_str(), "failure" | "error" | "timed_out" | "cancelled")).count();
    let running = d.checks.iter().filter(|c| matches!(c.state.as_str(), "pending" | "queued" | "in_progress")).count();
    crate::util::card(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (text, color) = match (d.checks.is_empty(), failing, running) {
            (true, _, _) => ("No checks".to_string(), pal.muted),
            (_, f, _) if f > 0 => (format!("{f} check{} failing", if f == 1 { "" } else { "s" }), pal.removed),
            (_, _, r) if r > 0 => (format!("{r} check{} still running", if r == 1 { "" } else { "s" }), pal.warn),
            _ => ("All checks have passed".to_string(), pal.added),
        };
        ui.label(RichText::new(text).color(color).strong());
        let merge_text = match d.mergeable {
            Some(true) => ("This branch has no conflicts with the base branch".to_string(), pal.added),
            Some(false) => ("This branch has conflicts to resolve first".to_string(), pal.removed),
            None => ("GitHub is still checking for conflicts".to_string(), pal.muted),
        };
        ui.label(RichText::new(merge_text.0).color(merge_text.1));
        let approvals = d.reviews.iter().filter(|r| r.1 == "APPROVED").count();
        let changes = d.reviews.iter().filter(|r| r.1 == "CHANGES_REQUESTED").count();
        ui.label(RichText::new(format!("{approvals} approval{}, {changes} change request{}", if approvals == 1 { "" } else { "s" }, if changes == 1 { "" } else { "s" })).color(pal.text2()));
        if d.state == "open" {
            ui.horizontal(|ui| {
                let can = d.mergeable == Some(true) && !d.draft && !v.busy;
                if ui.add_enabled(can, accent_button(MERGE[v.merge_method].1, pal)).on_hover_text("Merges on GitHub; Fetch afterwards to see it here").clicked() {
                    acts.push(Action::EditPr(PrEdit::Merge(MERGE[v.merge_method].0.into())));
                }
                egui::ComboBox::from_id_salt("merge_method").width(26.0).selected_text("").show_ui(ui, |ui| {
                    crate::util::menu_pad(ui);
                    for (i, (_, label)) in MERGE.iter().enumerate() {
                        ui.selectable_value(&mut v.merge_method, i, *label);
                    }
                });
                if d.draft {
                    ui.label(RichText::new("A draft can't be merged: mark it ready for review first.").size(12.0).color(pal.muted));
                }
            });
        }
    });
    ui.add_space(8.0);

    // Description, editable in place.
    crate::util::card(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Description").strong());
            if v.body_edit.is_none() && ui.small_button("✎").on_hover_text("Edit the description").clicked() {
                v.body_edit = Some(d.body.clone());
            }
        });
        match v.body_edit.as_mut() {
            Some(b) => {
                ui.add(egui::TextEdit::multiline(b).desired_rows(10).desired_width(f32::INFINITY));
                let mut close = false;
                ui.horizontal(|ui| {
                    if ui.add(accent_button("Save", pal)).clicked() {
                        acts.push(Action::EditPr(PrEdit::Body(b.clone())));
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
                if close {
                    v.body_edit = None;
                }
            }
            None => {
                let body = if d.body.trim().is_empty() { "No description." } else { d.body.as_str() };
                ui.add(egui::Label::new(RichText::new(body).color(pal.text)).wrap());
            }
        }
        // Co-authors of its commits (pushes made with an AI assistant often carry one).
        let co = d.coauthors();
        if !co.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Co-authored by {}", co.join(", "))).size(12.0).color(pal.text2()));
        }
    });
    ui.add_space(8.0);

    // Comments, then a box to add one.
    ui.label(RichText::new(format!("Comments  ({})", d.comments.len())).strong().size(15.0));
    for c in &d.comments {
        egui::Frame::new().stroke(Stroke::new(1.0, pal.border)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                person(ui, &c.author, pal);
                ui.label(RichText::new(&c.when).size(11.5).color(pal.muted));
            });
            ui.add(egui::Label::new(&c.body).wrap());
        });
        ui.add_space(4.0);
    }
    ui.add(egui::TextEdit::multiline(&mut v.comment).hint_text("Leave a comment").desired_rows(4).desired_width(f32::INFINITY));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!v.comment.trim().is_empty() && !v.busy, accent_button("Comment", pal)).clicked() {
                acts.push(Action::EditPr(PrEdit::Comment(v.comment.trim().to_string())));
                v.comment.clear();
            }
            match d.state.as_str() {
                "open" => {
                    if ui.button(RichText::new(format!("Close {noun}")).color(pal.removed)).clicked() {
                        acts.push(Action::EditPr(PrEdit::Open(false)));
                    }
                }
                "closed" => {
                    if ui.button(format!("Reopen {noun}")).clicked() {
                        acts.push(Action::EditPr(PrEdit::Open(true)));
                    }
                }
                _ => {}
            }
        });
    });
}

/// The right panel: reviewers, assignees, labels, milestone, co-authors, participants.
pub fn side(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let Some(v) = tab.pr_view.as_mut() else { return };
    let Some(Ok(d)) = v.detail.clone() else {
        ui.label(RichText::new("…").color(pal.muted));
        return;
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let heading = |ui: &mut egui::Ui, t: &str| {
            ui.add_space(8.0);
            ui.label(RichText::new(t).strong().color(pal.text2()));
        };
        heading(ui, "Reviewers");
        for r in &d.reviewers {
            ui.horizontal(|ui| {
                person(ui, r, pal);
                ui.label(RichText::new("pending").size(11.0).color(pal.warn));
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    acts.push(Action::EditPr(PrEdit::RemoveReviewer(r.clone())));
                }
            });
        }
        for (r, state) in &d.reviews {
            let (t, c) = match state.as_str() {
                "APPROVED" => ("approved", pal.added),
                "CHANGES_REQUESTED" => ("changes requested", pal.removed),
                _ => ("commented", pal.muted),
            };
            ui.horizontal(|ui| {
                person(ui, r, pal);
                ui.label(RichText::new(t).size(11.0).color(c));
            });
        }
        add_login(ui, &mut v.reviewer_add, "Request a review (login)", |login| acts.push(Action::EditPr(PrEdit::AddReviewer(login))));

        heading(ui, "Assignees");
        if d.assignees.is_empty() {
            ui.label(RichText::new("No one").color(pal.muted));
        }
        for a in &d.assignees {
            ui.horizontal(|ui| {
                person(ui, a, pal);
                if ui.small_button("×").on_hover_text("Unassign").clicked() {
                    acts.push(Action::EditPr(PrEdit::RemoveAssignee(a.clone())));
                }
            });
        }
        add_login(ui, &mut v.assignee_add, "Assign (login)", |login| acts.push(Action::EditPr(PrEdit::AddAssignee(login))));

        heading(ui, "Labels");
        match v.labels_edit.as_mut() {
            Some(l) => {
                ui.add(egui::TextEdit::singleline(l).hint_text("bug, ui, 4.4.0").desired_width(f32::INFINITY));
                let mut close = false;
                ui.horizontal(|ui| {
                    if ui.add(accent_button("Save", pal)).clicked() {
                        let labels = l.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        acts.push(Action::EditPr(PrEdit::Labels(labels)));
                        close = true;
                    } else if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
                if close {
                    v.labels_edit = None;
                }
            }
            None => {
                ui.horizontal_wrapped(|ui| {
                    if d.labels.is_empty() {
                        ui.label(RichText::new("None yet").color(pal.muted));
                    }
                    for l in &d.labels {
                        egui::Frame::new().fill(pal.accent.gamma_multiply(0.25)).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                            ui.label(RichText::new(l).size(12.0));
                        });
                    }
                    if ui.small_button("✎").on_hover_text("Edit the labels").clicked() {
                        v.labels_edit = Some(d.labels.join(", "));
                    }
                });
            }
        }

        heading(ui, "Milestone");
        ui.label(RichText::new(d.milestone.clone().unwrap_or_else(|| "No milestone".into())).color(pal.text2()));

        let co = d.coauthors();
        if !co.is_empty() {
            heading(ui, "Co-authors");
            for c in &co {
                person(ui, c.split(" <").next().unwrap_or(c), pal);
            }
        }

        let people = d.participants();
        heading(ui, &format!("{} participant{}", people.len(), if people.len() == 1 { "" } else { "s" }));
        ui.horizontal_wrapped(|ui| {
            for p in &people {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                avatar(ui.painter(), r.center(), 11.0, p, "");
                resp.on_hover_text(p);
            }
        });

        ui.add_space(10.0);
        ui.separator();
        if d.state == "open" {
            let (text, draft) = if d.draft { ("Ready for review", false) } else { ("Convert to draft", true) };
            if ui.button(text).clicked() {
                acts.push(Action::EditPr(PrEdit::Draft(draft, d.node_id.clone())));
            }
            if ui.button(RichText::new("Close pull request").color(pal.removed)).clicked() {
                acts.push(Action::EditPr(PrEdit::Open(false)));
            }
        }
        if ui.button("Copy link").clicked() {
            acts.push(Action::Copy(d.url.clone()));
        }
    });
}

fn add_login(ui: &mut egui::Ui, text: &mut String, hint: &str, mut add: impl FnMut(String)) {
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(ui.available_width() - 40.0));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.small_button("+").clicked() || enter) && !text.trim().is_empty() {
            add(text.trim().trim_start_matches('@').to_string());
            text.clear();
        }
    });
}
