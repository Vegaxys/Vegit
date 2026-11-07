//! Left sidebar: local branches, remotes, tags and stashes.

use eframe::egui::{self, Color32, RichText};

use crate::app::{Action, Dialog, PrState, RepoTab};
use crate::git::RefKind;
use crate::icons::{self, Icon};
use crate::theme::Palette;

pub fn show(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    ui.add_space(6.0);
    let width = ui.available_width();
    crate::util::search_field(ui, &mut tab.side_filter, "Filter branches", width, pal);
    ui.add_space(6.0);
    let filter = tab.side_filter.trim().to_lowercase();
    let keep = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Branch names never wrap: long ones end with "…".
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let filtering = !filter.is_empty();

        // Local branches, grouped in folders by their "folder/" prefixes.
        let locals: Vec<(String, usize)> = (0..tab.snap.refs.len())
            .filter(|&i| tab.snap.refs[i].kind == RefKind::Local && keep(&tab.snap.refs[i].name))
            .map(|i| (tab.snap.refs[i].name.clone(), i))
            .collect();
        let count = locals.len();
        section(ui, "local", Icon::Computer, &format!("Local  ({count})"), pal, |ui| {
            if locals.is_empty() {
                ui.label(RichText::new("No branches yet").color(pal.muted));
            }
            branch_tree(ui, "local", &locals, filtering, pal, &mut |ui, leaf, i| local_item(ui, tab, i, leaf, pal, acts));
        });

        // Remote branches, one section per remote, also in folders.
        for remote in tab.snap.remotes.clone() {
            let prefix = format!("{remote}/");
            let items: Vec<(String, usize)> = (0..tab.snap.refs.len())
                .filter(|&i| {
                    let r = &tab.snap.refs[i];
                    r.kind == RefKind::Remote && r.name.starts_with(&prefix) && keep(&r.name)
                })
                .map(|i| (tab.snap.refs[i].name[prefix.len()..].to_string(), i))
                .collect();
            section(ui, &format!("remote-{remote}"), Icon::Cloud, &format!("{remote}  ({})", items.len()), pal, |ui| {
                branch_tree(ui, &format!("remote-{remote}"), &items, filtering, pal, &mut |ui, leaf, i| remote_item(ui, tab, i, &remote, leaf, pal, acts));
            });
        }

        // Pull requests (or merge requests) on the hosting service.
        let pr_count = match &tab.prs {
            PrState::Ready(_, prs) => prs.len().to_string(),
            _ => "·".into(),
        };
        let current = tab.snap.head_branch.clone();
        section(ui, "prs", Icon::PullRequest, &format!("Pull requests  ({pr_count})"), pal, |ui| match &tab.prs {
            PrState::NotLoaded => {
                ui.label(RichText::new("Add a remote to see pull requests.").color(pal.muted).size(12.0));
            }
            PrState::Loading => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(RichText::new("Loading…").color(pal.muted).size(12.0));
                });
            }
            PrState::Unavailable(msg) => {
                ui.label(RichText::new(msg).color(pal.muted).size(12.0));
                if ui.small_button("Sign in and retry").clicked() {
                    acts.push(Action::LoadPullRequests { interactive: true });
                }
            }
            PrState::Ready(repo, prs) => {
                // Button first (right-aligned) so the service label is the part that shrinks.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can = current.is_some();
                        let new = egui::Button::new(RichText::new("+ New").size(12.0)).wrap_mode(egui::TextWrapMode::Extend);
                        if ui.add_enabled(can, new).on_hover_text(format!("Open a {} from the current branch", repo.forge.noun())).clicked() {
                            acts.push(Action::OpenPullRequestForm(None));
                        }
                        ui.add(egui::Label::new(RichText::new(format!("{} · {}", repo.forge.name(), repo.path)).color(pal.muted).size(11.5)).truncate());
                    });
                });
                if prs.is_empty() {
                    ui.label(RichText::new(format!("No open {}s.", repo.forge.noun())).color(pal.muted).size(12.0));
                }
                for (i, pr) in prs.iter().enumerate() {
                    let mut text = RichText::new(format!("   #{}  {}", pr.number, pr.title)).color(pal.text2());
                    if pr.draft {
                        text = RichText::new(format!("   #{}  {} (draft)", pr.number, pr.title)).color(pal.muted);
                    }
                    let resp = ui
                        .add(egui::Button::selectable(false, text).frame_when_inactive(false).truncate())
                        .on_hover_text(format!("{}\n{} wants to merge {} into {}\nDouble-click to check it out", pr.title, pr.author, pr.source, pr.target));
                    if resp.double_clicked() {
                        acts.push(Action::CheckoutPullRequest(i));
                    }
                    let url = pr.url.clone();
                    resp.context_menu(|ui| {
                        crate::util::menu_pad(ui);
                        if ui.button("Open in the browser").clicked() {
                            acts.push(Action::OpenUrl(url.clone()));
                            ui.close();
                        }
                        if ui.button("Check out").clicked() {
                            acts.push(Action::CheckoutPullRequest(i));
                            ui.close();
                        }
                        if ui.button("Copy link").clicked() {
                            acts.push(Action::Copy(url.clone()));
                            ui.close();
                        }
                    });
                }
            }
        });

        // Tags
        let tags: Vec<usize> = (0..tab.snap.refs.len())
            .filter(|&i| tab.snap.refs[i].kind == RefKind::Tag && keep(&tab.snap.refs[i].name))
            .collect();
        section(ui, "tags", Icon::Tag, &format!("Tags  ({})", tags.len()), pal, |ui| {
            for i in tags.into_iter().rev() {
                let r = &tab.snap.refs[i];
                let resp = ui.add(egui::Button::selectable(false, RichText::new(format!("   {}", r.name)).color(pal.text2())).frame_when_inactive(false));
                if resp.clicked() {
                    acts.push(Action::SelectRef(r.target.clone()));
                }
                let name = r.name.clone();
                resp.context_menu(|ui| {
                    crate::util::menu_pad(ui);
                    if ui.button("Copy name").clicked() {
                        acts.push(Action::Copy(name.clone()));
                        ui.close();
                    }
                    if ui.button(RichText::new("Delete tag").color(pal.removed)).clicked() {
                        acts.push(Action::Git { label: format!("Deleted tag {name}"), args: vec!["tag".into(), "-d".into(), name.clone()], background: false });
                        ui.close();
                    }
                });
            }
        });

        // Stashes
        section(ui, "stashes", Icon::Box, &format!("Stashes  ({})", tab.snap.stashes.len()), pal, |ui| {
            for s in &tab.snap.stashes {
                let resp = ui
                    .add(egui::Button::selectable(false, RichText::new(format!("   {}", s.message)).color(pal.text2())).frame_when_inactive(false).truncate())
                    .on_hover_text(format!("{}\nRight-click to apply, pop or delete", s.reference));
                let r = s.reference.clone();
                resp.context_menu(|ui| {
                    crate::util::menu_pad(ui);
                    if ui.button("Apply (keep the stash)").clicked() {
                        acts.push(Action::Git { label: "Stash applied".into(), args: vec!["stash".into(), "apply".into(), r.clone()], background: false });
                        ui.close();
                    }
                    if ui.button("Pop (apply and delete)").clicked() {
                        acts.push(Action::Git { label: "Stash popped".into(), args: vec!["stash".into(), "pop".into(), r.clone()], background: false });
                        ui.close();
                    }
                    if ui.button(RichText::new("Delete stash").color(pal.removed)).clicked() {
                        acts.push(Action::Dialog(Dialog::Confirm {
                            title: "Delete this stash?".into(),
                            body: "The saved changes will be lost.".into(),
                            confirm: "Delete".into(),
                            then: Box::new(Action::Git { label: "Stash deleted".into(), args: vec!["stash".into(), "drop".into(), r.clone()], background: false }),
                        }));
                        ui.close();
                    }
                });
            }
        });

        // Submodules: other repositories nested in this one.
        section(ui, "submodules", Icon::Link, &format!("Submodules  ({})", tab.snap.submodules.len()), pal, |ui| {
            if tab.snap.submodules.is_empty() {
                ui.label(RichText::new("None").color(pal.muted));
            }
            for sm in &tab.snap.submodules {
                let resp = ui
                    .add(egui::Button::selectable(false, RichText::new(format!("   {sm}")).color(pal.text2())).frame_when_inactive(false).truncate())
                    .on_hover_text("Double-click to open it in a new tab");
                if resp.double_clicked() {
                    acts.push(Action::OpenSubmodule(sm.clone()));
                }
                let path = sm.clone();
                resp.context_menu(|ui| {
                    crate::util::menu_pad(ui);
                    if ui.button("Open in a new tab").clicked() {
                        acts.push(Action::OpenSubmodule(path.clone()));
                        ui.close();
                    }
                    if ui.button("Update (init and check out)").clicked() {
                        acts.push(Action::Git {
                            label: format!("Submodule {path} updated"),
                            args: vec!["submodule".into(), "update".into(), "--init".into(), "--".into(), path.clone()],
                            background: true,
                        });
                        ui.close();
                    }
                    if ui.button("Show in folder").clicked() {
                        acts.push(Action::OpenInExplorer(Some(path.clone())));
                        ui.close();
                    }
                });
            }
        });
    });
}

/// Draws branches grouped by "folder/" prefixes (feature/login → feature ▸ login).
/// Folders start closed, except while filtering so matches are visible.
fn branch_tree(ui: &mut egui::Ui, id: &str, items: &[(String, usize)], open: bool, pal: &Palette, leaf: &mut dyn FnMut(&mut egui::Ui, &str, usize)) {
    let mut folders: Vec<(&str, Vec<(String, usize)>)> = Vec::new();
    let mut leaves: Vec<(&str, usize)> = Vec::new();
    for (path, i) in items {
        match path.split_once('/') {
            Some((folder, rest)) => match folders.iter_mut().find(|(f, _)| *f == folder) {
                Some((_, v)) => v.push((rest.to_string(), *i)),
                None => folders.push((folder, vec![(rest.to_string(), *i)])),
            },
            None => leaves.push((path.as_str(), *i)),
        }
    }
    folders.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    leaves.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    for (folder, sub) in folders {
        let sub_id = format!("{id}/{folder}");
        let title = RichText::new(format!("{folder}  ({})", sub.len())).color(pal.text2());
        let mut header = egui::CollapsingHeader::new(title).id_salt(&sub_id).default_open(false).icon(chevron(pal.muted));
        if open {
            header = header.open(Some(true));
        }
        header.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            branch_tree(ui, &sub_id, &sub, open, pal, leaf);
        });
    }
    for (name, i) in leaves {
        leaf(ui, name, i);
    }
}

fn local_item(ui: &mut egui::Ui, tab: &RepoTab, i: usize, leaf: &str, pal: &Palette, acts: &mut Vec<Action>) {
    let r = &tab.snap.refs[i];
    let is_head = tab.snap.head_branch.as_deref() == Some(r.name.as_str());
    let color = if is_head { pal.accent } else { pal.text2() };
    // The check marks the current branch; other names keep the same indent.
    let mut atoms = egui::Atoms::default();
    atoms.push_right(icons::image(ui.ctx(), Icon::Check, 13.0, if is_head { color } else { Color32::TRANSPARENT }));
    atoms.push_right(if is_head { RichText::new(leaf).strong().color(color) } else { RichText::new(leaf).color(color) });
    for (count, icon) in [(r.ahead, Icon::ArrowUp), (r.behind, Icon::ArrowDown)] {
        if count > 0 {
            atoms.push_right(icons::image(ui.ctx(), icon, 12.0, color));
            atoms.push_right(RichText::new(count.to_string()).color(color));
        }
    }
    let resp = ui
        .add(egui::Button::selectable(false, atoms).frame_when_inactive(false).truncate())
        .on_hover_text(format!("{}\nClick to show in the graph · double-click to switch to it · right-click for more", r.name));
    if resp.clicked() {
        acts.push(Action::SelectRef(r.target.clone()));
    }
    if resp.double_clicked() && !is_head {
        acts.push(Action::CheckoutLocal(r.name.clone()));
    }
    let name = r.name.clone();
    let current = tab.snap.head_branch.clone();
    resp.context_menu(|ui| {
        crate::util::menu_pad(ui);
        if !is_head && ui.button(format!("Switch to {name}")).clicked() {
            acts.push(Action::CheckoutLocal(name.clone()));
            ui.close();
        }
        if let (false, Some(cur)) = (is_head, current.as_ref()) {
            if ui.button(format!("Merge {name} into {cur}")).clicked() {
                acts.push(Action::Git { label: format!("Merged {name}"), args: vec!["merge".into(), "--no-edit".into(), name.clone()], background: false });
                ui.close();
            }
            if ui.button(format!("Rebase {cur} onto {name}")).clicked() {
                acts.push(Action::Git { label: format!("Rebased onto {name}"), args: vec!["rebase".into(), name.clone()], background: false });
                ui.close();
            }
        }
        if ui.button("Start a pull request…").clicked() {
            acts.push(Action::OpenPullRequestForm(Some(name.clone())));
            ui.close();
        }
        if ui.button("New branch from here…").clicked() {
            acts.push(Action::Dialog(Dialog::NewBranch { at: Some(name.clone()), name: String::new(), checkout: true }));
            ui.close();
        }
        if ui.button("Rename…").clicked() {
            acts.push(Action::Dialog(Dialog::RenameBranch { old: name.clone(), name: name.clone() }));
            ui.close();
        }
        if ui.button("Copy name").clicked() {
            acts.push(Action::Copy(name.clone()));
            ui.close();
        }
        if !is_head {
            ui.separator();
            if ui.button(RichText::new("Delete branch").color(pal.removed)).clicked() {
                acts.push(Action::Dialog(Dialog::Confirm {
                    title: format!("Delete {name}?"),
                    body: "Git refuses if the branch has commits that aren't merged anywhere else.".into(),
                    confirm: "Delete".into(),
                    then: Box::new(Action::Git { label: format!("Deleted {name}"), args: vec!["branch".into(), "-d".into(), name.clone()], background: false }),
                }));
                ui.close();
            }
        }
    });
}

fn remote_item(ui: &mut egui::Ui, tab: &RepoTab, i: usize, remote: &str, leaf: &str, pal: &Palette, acts: &mut Vec<Action>) {
    let r = &tab.snap.refs[i];
    let short = r.name[remote.len() + 1..].to_string();
    let resp = ui
        .add(egui::Button::selectable(false, RichText::new(format!("   {leaf}")).color(pal.text2())).frame_when_inactive(false).truncate())
        .on_hover_text(format!("{}\nDouble-click to switch to it (as a local branch)", r.name));
    if resp.clicked() {
        acts.push(Action::SelectRef(r.target.clone()));
    }
    if resp.double_clicked() {
        acts.push(Action::CheckoutRemote(r.name.clone()));
    }
    let full = r.name.clone();
    let rem = remote.to_string();
    let current = tab.snap.head_branch.clone().unwrap_or_else(|| "current".into());
    resp.context_menu(|ui| {
        crate::util::menu_pad(ui);
        if ui.button("Switch to it (local branch)").clicked() {
            acts.push(Action::CheckoutRemote(full.clone()));
            ui.close();
        }
        if ui.button(format!("Merge {full} into {current}")).clicked() {
            acts.push(Action::Git { label: format!("Merged {full}"), args: vec!["merge".into(), "--no-edit".into(), full.clone()], background: false });
            ui.close();
        }
        if ui.button(format!("Rebase {current} onto {full}")).clicked() {
            acts.push(Action::Git { label: format!("Rebased onto {full}"), args: vec!["rebase".into(), full.clone()], background: false });
            ui.close();
        }
        if ui.button("Copy name").clicked() {
            acts.push(Action::Copy(short.clone()));
            ui.close();
        }
        ui.separator();
        if ui.button(RichText::new("Delete on remote").color(pal.removed)).clicked() {
            acts.push(Action::Dialog(Dialog::Confirm {
                title: format!("Delete {full} on {rem}?"),
                body: "The branch will be removed from the server for everyone.".into(),
                confirm: "Delete".into(),
                then: Box::new(Action::Git { label: format!("Deleted {full}"), args: vec!["push".into(), rem.clone(), "--delete".into(), short.clone()], background: true }),
            }));
            ui.close();
        }
    });
}

/// A foldable sidebar section: chevron, icon, title. Open by default; the state is kept
/// under the same id as the former `CollapsingHeader`, so saved layouts still apply.
fn section(ui: &mut egui::Ui, id: &str, icon: Icon, title: &str, pal: &Palette, body: impl FnOnce(&mut egui::Ui)) {
    let state_id = ui.make_persistent_id(id);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true);
    let header = ui.horizontal(|ui| {
        state.show_toggle_button(ui, chevron(pal.text2()));
        ui.add(icons::image(ui.ctx(), icon, 16.0, pal.text2()));
        let label = ui.add(egui::Label::new(RichText::new(title).strong().color(pal.text)).sense(egui::Sense::click()));
        if label.clicked() {
            state.toggle(ui);
        }
    });
    state.show_body_indented(&header.response, ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        body(ui);
    });
    ui.add_space(4.0);
}

/// Fold marker for collapsing headers: chevron right when closed, down when open.
fn chevron(color: Color32) -> impl FnOnce(&mut egui::Ui, f32, &egui::Response) + 'static {
    move |ui, openness, resp| {
        let icon = if openness > 0.5 { Icon::ChevronDown } else { Icon::ChevronRight };
        icons::paint(ui.painter(), resp.rect.expand(2.0), icon, color);
    }
}
