//! Left sidebar: local branches, remotes, tags and stashes.

use eframe::egui::{self, RichText};

use crate::app::{Action, Dialog, PrState, RepoTab};
use crate::git::RefKind;
use crate::theme::Palette;

pub fn show(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    ui.add_space(6.0);
    ui.add(
        egui::TextEdit::singleline(&mut tab.side_filter)
            .hint_text("🔍  Filter branches")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(6.0);
    let filter = tab.side_filter.trim().to_lowercase();
    let keep = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Local branches
        let locals: Vec<usize> = (0..tab.snap.refs.len())
            .filter(|&i| tab.snap.refs[i].kind == RefKind::Local && keep(&tab.snap.refs[i].name))
            .collect();
        section(ui, "local", &format!("💻  Local  ({})", locals.len()), pal, |ui| {
            if locals.is_empty() {
                ui.label(RichText::new("No branches yet").color(pal.muted));
            }
            for i in locals {
                let r = &tab.snap.refs[i];
                let is_head = tab.snap.head_branch.as_deref() == Some(r.name.as_str());
                let mut text = if is_head { format!("✔ {}", r.name) } else { format!("   {}", r.name) };
                if r.ahead > 0 {
                    text.push_str(&format!("   ↑{}", r.ahead));
                }
                if r.behind > 0 {
                    text.push_str(&format!(" ↓{}", r.behind));
                }
                let rich = if is_head { RichText::new(text).strong().color(pal.accent) } else { RichText::new(text).color(pal.text2()) };
                let resp = ui.add(egui::Button::selectable(false, rich).frame_when_inactive(false)).on_hover_text(
                    "Click to show in the graph · double-click to switch to it · right-click for more",
                );
                if resp.clicked() {
                    acts.push(Action::SelectRef(r.target.clone()));
                }
                if resp.double_clicked() && !is_head {
                    acts.push(Action::CheckoutLocal(r.name.clone()));
                }
                let name = r.name.clone();
                let current = tab.snap.head_branch.clone();
                resp.context_menu(|ui| {
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
        });

        // Remote branches, grouped by remote
        for remote in tab.snap.remotes.clone() {
            let prefix = format!("{remote}/");
            let items: Vec<usize> = (0..tab.snap.refs.len())
                .filter(|&i| {
                    let r = &tab.snap.refs[i];
                    r.kind == RefKind::Remote && r.name.starts_with(&prefix) && keep(&r.name)
                })
                .collect();
            section(ui, &format!("remote-{remote}"), &format!("☁  {remote}  ({})", items.len()), pal, |ui| {
                for i in items {
                    let r = &tab.snap.refs[i];
                    let short = r.name[prefix.len()..].to_string();
                    let resp = ui
                        .add(egui::Button::selectable(false, RichText::new(format!("   {short}")).color(pal.text2())).frame_when_inactive(false))
                        .on_hover_text("Double-click to check it out as a local branch");
                    if resp.clicked() {
                        acts.push(Action::SelectRef(r.target.clone()));
                    }
                    if resp.double_clicked() {
                        acts.push(Action::CheckoutRemote(r.name.clone()));
                    }
                    let full = r.name.clone();
                    let rem = remote.clone();
                    resp.context_menu(|ui| {
                        if ui.button("Check out as local branch").clicked() {
                            acts.push(Action::CheckoutRemote(full.clone()));
                            ui.close();
                        }
                        if ui.button(format!("Merge {full} into current")).clicked() {
                            acts.push(Action::Git { label: format!("Merged {full}"), args: vec!["merge".into(), "--no-edit".into(), full.clone()], background: false });
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
            });
        }

        // Pull requests (or merge requests) on the hosting service.
        let pr_count = match &tab.prs {
            PrState::Ready(_, prs) => prs.len().to_string(),
            _ => "·".into(),
        };
        let current = tab.snap.head_branch.clone();
        section(ui, "prs", &format!("⇄  Pull requests  ({pr_count})"), pal, |ui| match &tab.prs {
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} · {}", repo.forge.name(), repo.path)).color(pal.muted).size(11.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can = current.is_some();
                        if ui.add_enabled(can, egui::Button::new(RichText::new("+ New").size(12.0))).on_hover_text(format!("Open a {} from the current branch", repo.forge.noun())).clicked() {
                            acts.push(Action::OpenPullRequestForm);
                        }
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
        section(ui, "tags", &format!("🏷  Tags  ({})", tags.len()), pal, |ui| {
            for i in tags.into_iter().rev() {
                let r = &tab.snap.refs[i];
                let resp = ui.add(egui::Button::selectable(false, RichText::new(format!("   {}", r.name)).color(pal.text2())).frame_when_inactive(false));
                if resp.clicked() {
                    acts.push(Action::SelectRef(r.target.clone()));
                }
                let name = r.name.clone();
                resp.context_menu(|ui| {
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
        section(ui, "stashes", &format!("📦  Stashes  ({})", tab.snap.stashes.len()), pal, |ui| {
            for s in &tab.snap.stashes {
                let resp = ui
                    .add(egui::Button::selectable(false, RichText::new(format!("   {}", s.message)).color(pal.text2())).frame_when_inactive(false).truncate())
                    .on_hover_text(format!("{}\nRight-click to apply, pop or delete", s.reference));
                let r = s.reference.clone();
                resp.context_menu(|ui| {
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
        section(ui, "submodules", &format!("🔗  Submodules  ({})", tab.snap.submodules.len()), pal, |ui| {
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

fn section(ui: &mut egui::Ui, id: &str, title: &str, pal: &Palette, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong().color(pal.text))
        .id_salt(id)
        .default_open(true)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            body(ui);
        });
    ui.add_space(4.0);
}
