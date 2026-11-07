//! Windows and menu pieces of the "quick wins": remotes, Push…, tags on the server,
//! tracking, deleting an unmerged branch, merge options and the git identity.
//!
//! Wording follows the design doc: the words say what happens, the git command is in the
//! tooltip; risky local actions run at once with an Undo, server actions are confirmed.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};

use crate::app::{Action, Dialog, DraggedBranch, DropMenu, PushForm, RemoteCheck, RemoteForm, RepoTab};
use crate::forge;
use crate::git::{self, Identity, MergeMode};
use crate::icons::{self, Icon};
use crate::theme::Palette;
use crate::util::accent_button;

/// Width of the window for the dialogs drawn here, `None` for the others.
pub fn width(dialog: &Dialog) -> Option<f32> {
    match dialog {
        Dialog::AddRemote(_) => Some(500.0),
        Dialog::Push(_) | Dialog::Unmerged { .. } => Some(470.0),
        Dialog::RenameRemote { .. } | Dialog::RemoteUrl { .. } | Dialog::DeleteTag { .. } | Dialog::Track { .. } => Some(420.0),
        _ => None,
    }
}

/// Draws one of the dialogs of this module. Returns true when it should close.
pub fn dialog(ui: &mut egui::Ui, dialog: &mut Dialog, tab: Option<&RepoTab>, pal: &Palette, acts: &mut Vec<Action>) -> bool {
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    match dialog {
        Dialog::AddRemote(form) => add_remote(ui, form, pal, acts),
        Dialog::RenameRemote { old, name } => {
            title(ui, &format!("Rename remote {old}"));
            ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)).request_focus();
            let taken = tab.is_some_and(|t| t.snap.remotes.iter().any(|r| r == name.trim()));
            let valid = valid_name(name) && !taken;
            if taken && name.trim() != old {
                ui.label(RichText::new("Another remote already has this name.").color(pal.warn).size(12.0));
            }
            if crate::ui::footer(ui, pal, "Rename", valid) || (enter && valid) {
                let n = name.trim().to_string();
                acts.push(Action::Git { label: format!("Remote {old} renamed to {n}"), args: git::remote_rename_args(old, &n), background: false });
                return true;
            }
            false
        }
        Dialog::RemoteUrl { name, url } => {
            title(ui, &format!("Change the URL of {name}"));
            ui.label(RichText::new("Where this remote points. Its branches stay listed until the next fetch.").color(pal.muted));
            ui.add(egui::TextEdit::singleline(url).desired_width(f32::INFINITY)).request_focus();
            let valid = !url.trim().is_empty();
            if crate::ui::footer(ui, pal, "Save", valid) || (enter && valid) {
                acts.push(Action::Git { label: format!("URL of {name} changed"), args: git::remote_set_url_args(name, url.trim()), background: false });
                return true;
            }
            false
        }
        Dialog::Push(form) => push(ui, form, tab, pal, acts),
        Dialog::DeleteTag { tag, remote, also_remote } => {
            title(ui, &format!("Delete tag {tag}?"));
            ui.label("The tag is removed from this computer.");
            if let Some(r) = remote.as_ref() {
                ui.checkbox(also_remote, format!("Also delete it on {r}")).on_hover_text(format!("git push {r} --delete refs/tags/{tag}"));
            }
            if crate::ui::footer(ui, pal, "Delete", true) {
                let on = remote.clone().filter(|_| *also_remote);
                acts.push(Action::DeleteTag { tag: tag.clone(), remote: on });
                return true;
            }
            false
        }
        Dialog::Track { local, options, choice } => {
            title(ui, "Track a remote branch");
            ui.label(RichText::new(format!("Pull and Push on {local} will then use the branch chosen here.")).color(pal.muted));
            ui.add_space(4.0);
            if options.is_empty() {
                ui.label(RichText::new("No remote branches yet: fetch first.").color(pal.warn));
            } else {
                let shown = options.get(*choice).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt("track_choice").width(ui.available_width()).selected_text(shown).show_ui(ui, |ui| {
                    crate::util::menu_pad(ui);
                    for (i, o) in options.iter().enumerate() {
                        ui.selectable_value(choice, i, o);
                    }
                });
            }
            let pick = options.get(*choice).cloned();
            if crate::ui::footer(ui, pal, "Track", pick.is_some()) {
                if let Some(p) = pick {
                    acts.push(Action::Git { label: format!("{local} now tracks {p}"), args: git::set_upstream_args(local, &p), background: false });
                }
                return true;
            }
            false
        }
        Dialog::Unmerged { branch, sha, commits } => {
            title(ui, &format!("Delete {branch}?"));
            let n = commits.len();
            let noun = if n == 1 { "commit that is" } else { "commits that are" };
            ui.label(format!("{branch} has {n} {noun} on no other branch:"));
            ui.add_space(4.0);
            egui::Frame::new().fill(pal.canvas).stroke(Stroke::new(1.0, pal.border)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for (s, summary) in commits.iter().take(8) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(git::short(s)).monospace().color(pal.muted));
                        ui.label(summary);
                    });
                }
                if n > 8 {
                    ui.label(RichText::new(format!("…and {} more", n - 8)).color(pal.muted));
                }
            });
            ui.label(RichText::new("Right after deleting, the toast's Undo brings the branch back.").size(12.0).color(pal.muted));
            let mut close = false;
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let danger = egui::Button::new(RichText::new("Delete anyway").color(Color32::WHITE)).fill(pal.removed);
                if ui.add(danger).on_hover_text(format!("git branch -D {branch}")).clicked() {
                    acts.push(Action::ForceDeleteBranch { name: branch.clone(), sha: sha.clone() });
                    close = true;
                }
                if ui.button("Keep it").clicked() {
                    close = true;
                }
            });
            close
        }
        _ => false,
    }
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(20.0).strong());
}

fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.contains(char::is_whitespace)
}

fn add_remote(ui: &mut egui::Ui, form: &mut RemoteForm, pal: &Palette, acts: &mut Vec<Action>) -> bool {
    let now = ui.input(|i| i.time);
    title(ui, "Add remote");
    ui.label(RichText::new("Paste the address of the repository (HTTPS or SSH). Vegit checks that it answers before adding it.").color(pal.muted));
    ui.add_space(6.0);
    ui.label(RichText::new("Address").color(pal.text2()));
    let url_resp = ui.add(egui::TextEdit::singleline(&mut form.url).hint_text("https://github.com/owner/repository.git").desired_width(f32::INFINITY));
    if form.url.is_empty() && form.checked_url.is_empty() {
        url_resp.request_focus();
    }
    if url_resp.changed() {
        form.edited_at = now;
        form.check = RemoteCheck::Idle;
        if !form.name_edited {
            form.name = git::suggest_remote_name(&form.existing, form.url.trim());
        }
    }
    // Check once the typing (or the paste) has settled.
    let url = form.url.trim().to_string();
    if !url.is_empty() && url != form.checked_url && now - form.edited_at > 0.6 {
        form.checked_url = url.clone();
        form.check = RemoteCheck::Checking;
        acts.push(Action::CheckRemote(url.clone()));
    }
    if !url.is_empty() && url != form.checked_url {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    ui.horizontal_wrapped(|ui| match &form.check {
        RemoteCheck::Idle => {
            ui.label(RichText::new(" ").size(12.0));
        }
        RemoteCheck::Checking => {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(RichText::new("Checking access…").size(12.0).color(pal.muted)).on_hover_text("git ls-remote");
        }
        RemoteCheck::Reachable(n) => {
            ui.add(icons::image(ui.ctx(), Icon::Check, 14.0, pal.added));
            let what = if *n == 0 { "it answers (no branches yet)".to_string() } else { format!("it answers · {n} branches") };
            ui.label(RichText::new(format!("Reachable: {what}")).size(12.0).color(pal.added));
        }
        RemoteCheck::Failed(e) => {
            ui.label(RichText::new(e).size(12.0).color(pal.removed));
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Name").color(pal.text2()));
    if ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY)).changed() {
        form.name_edited = true;
    }
    let taken = form.existing.iter().any(|(n, _)| n == form.name.trim());
    if taken {
        ui.label(RichText::new("Another remote already has this name.").size(12.0).color(pal.warn));
    } else {
        ui.label(RichText::new("origin is the usual name for your own copy, upstream for the original of a fork.").size(12.0).color(pal.muted));
    }
    let valid = valid_name(&form.name) && !taken && !url.is_empty() && form.check != RemoteCheck::Checking;
    let confirm = if matches!(form.check, RemoteCheck::Failed(_)) { "Add anyway" } else { "Add remote" };
    if crate::ui::footer(ui, pal, confirm, valid) {
        acts.push(Action::AddRemote { name: form.name.trim().to_string(), url });
        return true;
    }
    false
}

fn push(ui: &mut egui::Ui, form: &mut PushForm, tab: Option<&RepoTab>, pal: &Palette, acts: &mut Vec<Action>) -> bool {
    title(ui, &format!("Push {}", form.branch));
    ui.add_space(4.0);
    egui::Grid::new("push_grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.label(RichText::new("To remote").color(pal.text2()));
        let before = form.remote;
        egui::ComboBox::from_id_salt("push_remote").width(240.0).selected_text(form.remote_name().to_string()).show_ui(ui, |ui| {
            crate::util::menu_pad(ui);
            for (i, r) in form.remotes.iter().enumerate() {
                ui.selectable_value(&mut form.remote, i, r);
            }
        });
        if form.remote != before && tab.is_some_and(|t| !t.remote_tags.contains_key(form.remote_name())) {
            acts.push(Action::CheckRemoteTags(form.remote_name().to_string()));
        }
        ui.end_row();
        ui.label(RichText::new("Branch on the server").color(pal.text2()));
        ui.add(egui::TextEdit::singleline(&mut form.remote_branch).desired_width(240.0));
        ui.end_row();
    });
    ui.add_space(6.0);
    let remote = form.remote_name().to_string();
    match tab.and_then(|t| t.new_tags_for(&remote, &form.reachable)) {
        None if !form.reachable.is_empty() => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(format!("Checking which tags {remote} already has…")).size(12.0).color(pal.muted));
            });
        }
        Some(new) if !new.is_empty() => {
            let n = new.len();
            let text = if n == 1 { format!("Also push 1 new tag ({})", new[0]) } else { format!("Also push {n} new tags") };
            ui.checkbox(&mut form.push_tags, text).on_hover_text(format!("Tags on the pushed commits that {remote} doesn't have:\n{}\n\ngit push {remote} refs/tags/<tag>…", new.join(", ")));
        }
        _ => {}
    }
    if !form.has_upstream {
        ui.label(RichText::new(format!("{} will then follow {remote}/{}: Pull and Push won't ask again.", form.branch, form.remote_branch.trim())).size(12.0).color(pal.muted));
    }
    egui::CollapsingHeader::new(RichText::new("More options").color(pal.text2())).id_salt("push_more").default_open(false).show(ui, |ui| {
        ui.checkbox(&mut form.force, "Replace the branch on the server (after rewriting your own commits)")
            .on_hover_text("git push --force-with-lease: refused if someone else pushed in the meantime.");
    });
    let valid = valid_name(&form.remote_branch);
    if crate::ui::footer(ui, pal, if form.force { "Force push" } else { "Push" }, valid) {
        let tags = match tab.and_then(|t| t.new_tags_for(&remote, &form.reachable)) {
            Some(new) if form.push_tags => new,
            _ => Vec::new(),
        };
        acts.push(Action::PushTo {
            remote,
            branch: form.branch.clone(),
            remote_branch: form.remote_branch.trim().to_string(),
            force: form.force,
            tags,
            set_upstream: !form.has_upstream,
        });
        return true;
    }
    false
}

/// The merge choices of a branch menu: bring `source` into the current branch `cur`
/// (normally or as one commit), replay `cur` on top of it, and the two strict variants.
/// `label` is how `source` is named in the menu.
pub fn merge_items(ui: &mut egui::Ui, source: &str, label: &str, cur: &str, acts: &mut Vec<Action>) {
    if ui.button(format!("Bring {label} into {cur}")).on_hover_text(format!("git merge {source}")).clicked() {
        acts.push(Action::Merge { source: source.to_string(), mode: MergeMode::Normal });
        ui.close();
    }
    if ui
        .button(format!("Bring {label} into {cur} as one commit"))
        .on_hover_text(format!("git merge --squash {source}\nIts changes are staged and the message lists its commits: you commit once."))
        .clicked()
    {
        acts.push(Action::Merge { source: source.to_string(), mode: MergeMode::Squash });
        ui.close();
    }
    if ui.button(format!("Replay {cur} on top of {label}")).on_hover_text(format!("git rebase {source}")).clicked() {
        acts.push(Action::Git { label: format!("Replayed {cur} on top of {label}"), args: vec!["rebase".into(), source.to_string()], background: false });
        ui.close();
    }
    ui.menu_button("More options", |ui| {
        crate::util::menu_pad(ui);
        if ui.button("Always create a merge commit").on_hover_text(format!("git merge --no-ff {source}")).clicked() {
            acts.push(Action::Merge { source: source.to_string(), mode: MergeMode::NoFastForward });
            ui.close();
        }
        if ui.button("Only if no merge commit is needed").on_hover_text(format!("git merge --ff-only {source}\nNothing changes if both branches have new commits.")).clicked() {
            acts.push(Action::Merge { source: source.to_string(), mode: MergeMode::FastForwardOnly });
            ui.close();
        }
    });
}

/// Makes a branch row or label draggable (its response must sense drags).
pub fn drag_branch(resp: &egui::Response, name: &str) {
    resp.dnd_set_drag_payload(DraggedBranch(name.to_string()));
}

/// A local branch as a drop target: highlights while another branch is dragged over it,
/// and on release opens the menu of what to do (doc: "Glisser une branche sur une autre").
pub fn drop_on_branch(ui: &egui::Ui, resp: &egui::Response, target: &str, pal: &Palette, acts: &mut Vec<Action>) {
    if resp.dnd_hover_payload::<DraggedBranch>().is_some_and(|p| p.0 != target) {
        ui.painter().rect_stroke(resp.rect, CornerRadius::same(6), Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
    }
    if let Some(p) = resp.dnd_release_payload::<DraggedBranch>() {
        if p.0 != target {
            acts.push(Action::OpenDropMenu { source: p.0.clone(), target: target.to_string() });
        }
    }
}

/// The menu after a drop: the same phrases as the branch menus (phase 1), with the dropped
/// branch as the source and the one under it as the target.
pub fn drop_menu(ctx: &egui::Context, menu: &DropMenu, is_local_source: bool, pal: &Palette, acts: &mut Vec<Action>) {
    let (s, t) = (menu.source.as_str(), menu.target.as_str());
    let area = egui::Area::new(egui::Id::unique("drop_menu")).order(egui::Order::Foreground).fixed_pos(menu.pos).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            crate::util::menu_pad(ui);
            ui.set_min_width(260.0);
            ui.label(RichText::new(format!("{s}  →  {t}")).color(pal.muted));
            let mut pick = |ui: &mut egui::Ui, text: String, tip: String, action: Action| {
                if ui.button(text).on_hover_text(tip).clicked() {
                    acts.push(action);
                    acts.push(Action::CloseDropMenu);
                }
            };
            let merge = |mode| Action::MergeInto { source: s.to_string(), target: t.to_string(), mode };
            pick(ui, format!("Bring {s} into {t}"), format!("git merge {s} (on {t})"), merge(MergeMode::Normal));
            pick(ui, format!("Bring {s} into {t} as one commit"), format!("git merge --squash {s} (on {t})\nThe commit message lists the combined commits."), merge(MergeMode::Squash));
            if is_local_source {
                let replay = Action::Git { label: format!("Replayed {s} on top of {t}"), args: vec!["rebase".into(), t.to_string(), s.to_string()], background: false };
                pick(ui, format!("Replay {s} on top of {t}"), format!("git rebase {t} {s}"), replay);
            }
            ui.menu_button("More options", |ui| {
                crate::util::menu_pad(ui);
                pick(ui, "Always create a merge commit".into(), format!("git merge --no-ff {s}"), merge(MergeMode::NoFastForward));
                pick(ui, "Only if no merge commit is needed".into(), format!("git merge --ff-only {s}"), merge(MergeMode::FastForwardOnly));
            });
            ui.separator();
            if ui.button("Cancel").clicked() {
                acts.push(Action::CloseDropMenu);
            }
        });
    });
    // A click elsewhere or Escape closes it, like any menu.
    // (Not while its "More options" submenu is open: that click lands in the submenu.)
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || (outside && !egui::Popup::is_any_open(ctx)) {
        acts.push(Action::CloseDropMenu);
    }
}

/// While a branch is dragged: its name follows the pointer.
pub fn drag_ghost(ctx: &egui::Context, pal: &Palette) {
    let Some(p) = egui::DragAndDrop::payload::<DraggedBranch>(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    egui::Area::new(egui::Id::unique("drag_ghost")).order(egui::Order::Tooltip).fixed_pos(pos + egui::vec2(14.0, 8.0)).interactable(false).show(ctx, |ui| {
        egui::Frame::new().fill(pal.accent).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
            ui.label(RichText::new(&p.0).color(Color32::WHITE).size(12.0));
        });
    });
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
}

/// Right-click menu of a remote's sidebar header.
pub fn remote_menu(ui: &mut egui::Ui, tab: &RepoTab, remote: &str, pal: &Palette, acts: &mut Vec<Action>) {
    crate::util::menu_pad(ui);
    let url = tab.snap.remote_urls.iter().find(|(n, _)| n == remote).map(|(_, u)| u.clone()).unwrap_or_default();
    if ui.button("Fetch this remote").on_hover_text(format!("git fetch --prune {remote}")).clicked() {
        acts.push(Action::FetchRemote(remote.to_string()));
        ui.close();
    }
    if ui.button("Rename…").clicked() {
        acts.push(Action::Dialog(Dialog::RenameRemote { old: remote.to_string(), name: remote.to_string() }));
        ui.close();
    }
    if ui.button("Change URL…").clicked() {
        acts.push(Action::Dialog(Dialog::RemoteUrl { name: remote.to_string(), url: url.clone() }));
        ui.close();
    }
    if ui.button("Copy URL").clicked() {
        acts.push(Action::Copy(url.clone()));
        ui.close();
    }
    if let Some(repo) = forge::detect(remote, &url) {
        if ui.button(format!("Open on {}", repo.forge.name())).clicked() {
            acts.push(Action::OpenUrl(forge::repo_url(&repo)));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(RichText::new("Remove…").color(pal.removed)).clicked() {
        acts.push(Action::Dialog(Dialog::Confirm {
            title: format!("Remove remote {remote}?"),
            body: "Its branches disappear from this list. Nothing is deleted on the server.".into(),
            confirm: "Remove".into(),
            then: Box::new(Action::Git { label: format!("Remote {remote} removed"), args: git::remote_remove_args(remote), background: false }),
        }));
        ui.close();
    }
}

/// "Who are you?" in the commit panel, while git doesn't know the name or e-mail to sign
/// commits with.
pub fn identity_form(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    if tab.snap.identity.is_complete() {
        return;
    }
    let id = tab.snap.identity.clone();
    let (name, email) = &mut tab.identity_form;
    if name.is_empty() {
        *name = id.name().unwrap_or_default().to_string();
    }
    if email.is_empty() {
        *email = id.email().unwrap_or_default().to_string();
    }
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(pal.accent.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, pal.accent.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Who are you?").strong());
            ui.label(RichText::new("Git signs every commit with a name and an e-mail, visible to anyone who sees the project.").size(12.0).color(pal.muted));
            ui.add(egui::TextEdit::singleline(name).hint_text("Your name").desired_width(f32::INFINITY));
            ui.add(egui::TextEdit::singleline(email).hint_text("you@example.com").desired_width(f32::INFINITY));
            let valid = !name.trim().is_empty() && email.contains('@');
            ui.horizontal(|ui| {
                if ui.add_enabled(valid, accent_button("For all projects", pal)).on_hover_text("git config --global user.name / user.email").clicked() {
                    acts.push(Action::SetIdentity { global: true, name: name.clone(), email: email.clone() });
                }
                if ui.add_enabled(valid, egui::Button::new("Only this one")).on_hover_text("git config --local user.name / user.email").clicked() {
                    acts.push(Action::SetIdentity { global: false, name: name.clone(), email: email.clone() });
                }
            });
        });
}

/// Settings → Identity: both levels, which one applies, and a form to change either.
pub fn identity_card(ui: &mut egui::Ui, identity: Option<&Identity>, pal: &Palette, acts: &mut Vec<Action>) {
    ui.label(RichText::new("Identity").strong());
    let Some(id) = identity else {
        ui.label(RichText::new("Open a repository to see and change who its commits are signed by.").size(12.0).color(pal.muted));
        return;
    };
    ui.label(RichText::new("The name and e-mail git puts on your commits. This project's values, when set, win over the ones for all projects.").size(12.0).color(pal.muted));
    ui.add_space(4.0);
    let show = |name: &Option<String>, email: &Option<String>| match (name, email) {
        (None, None) => "not set".to_string(),
        (n, e) => format!("{} <{}>", n.as_deref().unwrap_or("(no name)"), e.as_deref().unwrap_or("no e-mail")),
    };
    let local_wins = id.local_name.is_some() || id.local_email.is_some();
    egui::Grid::new("identity_grid").num_columns(3).spacing([14.0, 6.0]).show(ui, |ui| {
        ui.label(RichText::new("All projects").color(pal.text2()));
        ui.label(show(&id.global_name, &id.global_email));
        if !local_wins && id.is_complete() {
            ui.label(RichText::new("in use").size(11.0).color(pal.added));
        } else {
            ui.label("");
        }
        ui.end_row();
        ui.label(RichText::new("This project").color(pal.text2()));
        ui.label(show(&id.local_name, &id.local_email));
        if local_wins {
            ui.label(RichText::new("in use").size(11.0).color(pal.added));
        } else {
            ui.label("");
        }
        ui.end_row();
    });
    if id.is_complete() {
        let who = format!("{} <{}>", id.name().unwrap_or_default(), id.email().unwrap_or_default());
        ui.label(RichText::new(format!("Commits here are signed {who}, set for {}.", id.source())).size(12.0).color(pal.text2()));
    } else {
        ui.label(RichText::new("Git can't commit until both a name and an e-mail are set somewhere.").size(12.0).color(pal.warn));
    }
    // Edit buffer kept across frames while the window is open.
    let key = egui::Id::unique("identity_edit");
    let mut edit: (String, String, bool) = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| {
        (id.name().unwrap_or_default().to_string(), id.email().unwrap_or_default().to_string(), !local_wins)
    });
    egui::CollapsingHeader::new(RichText::new("Change it").color(pal.text2())).id_salt("identity_change").default_open(false).show(ui, |ui| {
        ui.add(egui::TextEdit::singleline(&mut edit.0).hint_text("Your name").desired_width(320.0));
        ui.add(egui::TextEdit::singleline(&mut edit.1).hint_text("you@example.com").desired_width(320.0));
        ui.horizontal(|ui| {
            ui.radio_value(&mut edit.2, true, "For all projects").on_hover_text("git config --global");
            ui.radio_value(&mut edit.2, false, "Only this one").on_hover_text("git config --local");
        });
        let valid = !edit.0.trim().is_empty() && edit.1.contains('@');
        if ui.add_enabled(valid, accent_button("Save identity", pal)).clicked() {
            acts.push(Action::SetIdentity { global: edit.2, name: edit.0.clone(), email: edit.1.clone() });
        }
    });
    ui.data_mut(|d| d.insert_temp(key, edit));
}
