//! Interface of phases 4 and 5: compare panel, search modes, bisect banner, the repository
//! menu, commit options, menu items for stash / submodules / cleanup / patches / several
//! commits, and their dialogs.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};

use crate::app::{Action, Dialog, RepoTab, SearchMode};
use crate::extras::{self, CommitOptions};
use crate::git::{self, DiffSource, FileState};
use crate::lfs::human_size;
use crate::theme::Palette;
use crate::util::accent_button;

pub fn width(dialog: &Dialog) -> Option<f32> {
    match dialog {
        Dialog::Onto { .. } | Dialog::Sparse { .. } | Dialog::ApplyPatch { .. } | Dialog::CleanIgnored { .. } => Some(500.0),
        Dialog::AddSubmodule { .. } | Dialog::StashBranch { .. } | Dialog::BisectRun { .. } => Some(440.0),
        Dialog::Grep { .. } => Some(620.0),
        _ => None,
    }
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(20.0).strong());
}

// ---------------------------------------------------------------------------------------------
// Compare panel
// ---------------------------------------------------------------------------------------------

/// Two commits or two branches compared, in the detail panel.
pub fn compare_panel(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let open = tab.diff.as_ref().map(|d| d.file.path.clone());
    let Some(cv) = tab.compare.as_mut() else { return };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let n = cv.files.as_ref().map(|f| f.len());
        let head = match n {
            Some(n) if !cv.branches => format!("{} · {n} file{}", cv.title, if n == 1 { "" } else { "s" }),
            _ => cv.title.clone(),
        };
        ui.label(RichText::new(head).size(16.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                acts.push(if cv.branches { Action::CloseCompare } else { Action::ClearPicked });
            }
        });
    });
    if cv.branches {
        // The two-dots / three-dots difference, said plainly (doc §Comparer).
        let (a, b) = (cv.a_name.clone(), cv.b_name.clone());
        ui.horizontal(|ui| {
            let tabs = [
                format!("{} only on {b}  ·  {} only on {a}", cv.only_b.len(), cv.only_a.len()),
                format!("What {b} changed"),
            ];
            for (i, t) in tabs.iter().enumerate() {
                if ui.selectable_label(cv.page == i, t.as_str()).clicked() {
                    cv.page = i;
                }
            }
        });
        let help = if cv.page == 0 {
            format!("Commits one branch has and the other doesn't (git log {a}..{b} and {b}..{a}).")
        } else {
            format!("Every change {b} made since it left {a}, whatever {a} did meanwhile (git diff {a}...{b}).")
        };
        ui.label(RichText::new(help).size(12.0).color(pal.muted));
        if cv.page == 0 {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for (name, list) in [(b.clone(), &cv.only_b), (a.clone(), &cv.only_a)] {
                    ui.label(RichText::new(format!("Only on {name}  ({})", list.len())).strong());
                    for (sha, s) in list.iter() {
                        let r = ui.add(egui::Button::selectable(false, RichText::new(format!("{}  {s}", git::short(sha)))).frame_when_inactive(false));
                        if r.clicked() {
                            acts.push(Action::SelectRef(sha.clone()));
                        }
                    }
                    ui.add_space(6.0);
                }
            });
            return;
        }
    } else {
        ui.label(RichText::new("Ctrl+click picked these two commits. The files are what changed from the older one to the newer one.").size(12.0).color(pal.muted));
    }
    let Some(files) = &cv.files else {
        crate::ui_detail::loading(ui, "Comparing…");
        return;
    };
    let source = DiffSource::Commit { sha: cv.to.clone(), parent: Some(cv.from.clone()) };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for f in files {
            let (resp, _) = crate::util::file_row(ui, f, pal, open.as_deref() == Some(f.path.as_str()), None, 0.0);
            if resp.clicked() {
                acts.push(Action::OpenDiff(source.clone(), f.clone()));
            }
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Search modes and history paging
// ---------------------------------------------------------------------------------------------

/// The mode chip next to the graph's search field; content modes search on Enter.
pub fn search_mode(ui: &mut egui::Ui, tab: &mut RepoTab, field: &egui::Response, acts: &mut Vec<Action>) {
    let before = tab.search_mode;
    egui::ComboBox::from_id_salt("search_mode").width(110.0).selected_text(tab.search_mode.label()).show_ui(ui, |ui| {
        crate::util::menu_pad(ui);
        for m in SearchMode::ALL {
            let tip = match m {
                SearchMode::Message => "Commit messages and SHAs",
                SearchMode::Author => "Author names",
                SearchMode::ChangedText => "Commits that added or removed this text (git log -S). Press Enter.",
                SearchMode::FilePath => "Commits that touched a file whose path contains this. Press Enter.",
            };
            ui.selectable_value(&mut tab.search_mode, m, m.label()).on_hover_text(tip);
        }
    });
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if tab.search_mode.in_content() && (enter || before != tab.search_mode) && !tab.search.trim().is_empty() {
        acts.push(Action::ContentSearch);
    }
    match &tab.search_hits {
        Some((q, m, None)) if *q == tab.search.trim() && *m == tab.search_mode => {
            ui.add(egui::Spinner::new().size(14.0)).on_hover_text("Searching every branch…");
        }
        Some((q, m, Some(Ok(h)))) if *q == tab.search.trim() && *m == tab.search_mode => {
            ui.label(RichText::new(format!("{} found", h.len())).size(12.0));
        }
        _ => {}
    }
}

/// Whether a commit matches the search, by mode (`None`: no search, everything is lit).
pub fn matches(tab: &RepoTab, c: &git::Commit, q: &str) -> Option<bool> {
    if q.is_empty() {
        return None;
    }
    Some(match tab.search_mode {
        SearchMode::Message => c.summary.to_lowercase().contains(q) || c.body.to_lowercase().contains(q) || c.sha.starts_with(q),
        SearchMode::Author => c.author.to_lowercase().contains(q) || c.email.to_lowercase().contains(q),
        // While git searches, nothing is dimmed.
        _ => match crate::app::content_hits(tab) {
            Some(hits) => hits.contains(&c.sha),
            None => return None,
        },
    })
}

// ---------------------------------------------------------------------------------------------
// Bisect banner
// ---------------------------------------------------------------------------------------------

/// "Testing commit 3 of about 6. Does the problem happen here?" (doc §bisect 2).
pub fn bisect_banner(ui: &mut egui::Ui, tab: &RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let Some(b) = &tab.snap.bisect else { return };
    let total = b.tested + b.steps_left;
    egui::Frame::new().fill(pal.accent.gamma_multiply(0.15)).stroke(Stroke::new(1.0, pal.accent)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let here = tab.snap.head_sha.as_deref().map(git::short).unwrap_or("?");
            ui.label(RichText::new(format!("Testing commit {} of about {total} ({here}).", b.tested + 1)).strong());
            ui.label("Does the problem happen here?");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Stop").on_hover_text("git bisect reset: back to where you started").clicked() {
                    acts.push(Action::BisectStop);
                }
                if ui.button("Can't test").on_hover_text("git bisect skip").clicked() {
                    acts.push(Action::BisectAnswer("skip".into()));
                }
                if ui.button("No, it works").on_hover_text("git bisect good").clicked() {
                    acts.push(Action::BisectAnswer("good".into()));
                }
                if ui.add(egui::Button::new(RichText::new("Yes, broken").color(Color32::WHITE)).fill(pal.removed)).on_hover_text("git bisect bad").clicked() {
                    acts.push(Action::BisectAnswer("bad".into()));
                }
                ui.menu_button("More options", |ui| {
                    crate::util::menu_pad(ui);
                    if ui.button("Run a test command automatically…").on_hover_text("git bisect run").clicked() {
                        acts.push(Action::Dialog(Dialog::BisectRun { command: String::new() }));
                        ui.close();
                    }
                });
            });
        });
    });
}

// ---------------------------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------------------------

/// The repository menu (click on its name): sparse-checkout, full history, maintenance.
pub fn repo_menu(ui: &mut egui::Ui, tab: &RepoTab, acts: &mut Vec<Action>) {
    crate::util::menu_pad(ui);
    if tab.snap.shallow && ui.button("Load full history").on_hover_text("git fetch --unshallow: this copy only has recent history").clicked() {
        acts.push(Action::LoadFullHistory);
        ui.close();
    }
    let sparse = if tab.snap.sparse.is_some() { "Choose folders to download… (some are left out)" } else { "Choose folders to download…" };
    if ui.button(sparse).on_hover_text("sparse-checkout: keep only some folders on disk, for big monorepos").clicked() {
        acts.push(Action::OpenSparse);
        ui.close();
    }
    ui.separator();
    if ui.button("Speed up this repository").on_hover_text("git gc: packs and tidies the history; says the size before and after").clicked() {
        acts.push(Action::SpeedUp);
        ui.close();
    }
    if ui.button("Keep it fast automatically").on_hover_text("git maintenance start: a background task tidies it every hour").clicked() {
        acts.push(Action::KeepFast);
        ui.close();
    }
    if ui.button("Check for problems").on_hover_text("git fsck: checks every object").clicked() {
        acts.push(Action::CheckRepo);
        ui.close();
    }
}

/// Items for several picked commits: copy them onto the current branch, save as a patch.
pub fn picked_menu(ui: &mut egui::Ui, tab: &RepoTab, acts: &mut Vec<Action>) {
    let n = tab.picked_commits.len();
    let target = tab.snap.head_branch.clone().unwrap_or_else(|| "HEAD".into());
    let shas = tab.picked_commits.clone();
    if ui.button(format!("Copy {n} commits onto {target}")).on_hover_text("git cherry-pick, oldest first").clicked() {
        acts.push(Action::CopyCommits { shas: shas.clone(), mention: false, no_commit: false });
        ui.close();
    }
    ui.menu_button("More options", |ui| {
        crate::util::menu_pad(ui);
        if ui.button("Mention the original commit").on_hover_text("git cherry-pick -x").clicked() {
            acts.push(Action::CopyCommits { shas: shas.clone(), mention: true, no_commit: false });
            ui.close();
        }
        if ui.button("Apply the changes without committing").on_hover_text("git cherry-pick --no-commit").clicked() {
            acts.push(Action::CopyCommits { shas: shas.clone(), mention: false, no_commit: true });
            ui.close();
        }
    });
    if ui.button("Save as patch file…").on_hover_text("git format-patch").clicked() {
        acts.push(Action::SavePatch(shas));
        ui.close();
    }
}

/// Extra items of a commit's menu: bisect, save as patch, signature.
pub fn commit_items(ui: &mut egui::Ui, tab: &RepoTab, sha: &str, acts: &mut Vec<Action>) {
    ui.separator();
    if tab.snap.bisect.is_none() && tab.snap.head_sha.as_deref() != Some(sha) {
        if ui.button("Find the commit that broke something…").on_hover_text("This commit works, the current one doesn't: Vegit guides you to the first broken one (git bisect)").clicked() {
            acts.push(Action::StartBisect(sha.to_string()));
            ui.close();
        }
    }
    if ui.button("Save as patch file…").on_hover_text("git format-patch").clicked() {
        acts.push(Action::SavePatch(vec![sha.to_string()]));
        ui.close();
    }
    if ui.button("Search files at this commit…").on_hover_text("git grep: the files as they were at this commit").clicked() {
        acts.push(Action::Dialog(Dialog::Grep { sha: sha.to_string(), query: String::new(), results: None }));
        ui.close();
    }
    if let Some((c, who)) = tab.signatures.get(sha) {
        ui.label(RichText::new(format!("{}{}", extras::signature_text(*c), if who.is_empty() { String::new() } else { format!(" · {who}") })).weak());
    }
}

/// Branch menu items: compare with the current branch, move only my commits.
pub fn branch_items(ui: &mut egui::Ui, tab: &RepoTab, name: &str, local: bool, acts: &mut Vec<Action>) {
    let Some(cur) = tab.snap.head_branch.clone() else { return };
    if name != cur && ui.button(format!("Compare with {cur}")).on_hover_text("Commits only on each side, and what it changed since they split").clicked() {
        acts.push(Action::CompareBranches { a: cur.clone(), b: name.to_string() });
        ui.close();
    }
    if local && ui.button("Move only my commits onto…").on_hover_text("git rebase --onto: when it was started from a branch that was merged as one commit").clicked() {
        acts.push(Action::OpenOnto(name.to_string()));
        ui.close();
    }
}

/// Files menu items: stash only these files.
pub fn stash_files_item(ui: &mut egui::Ui, paths: Vec<String>, acts: &mut Vec<Action>) {
    let n = paths.len();
    let label = if n == 1 { "Stash this file".to_string() } else { format!("Stash these {n} files") };
    if ui.button(label).on_hover_text("git stash push -- <files>").clicked() {
        acts.push(Action::StashFiles(paths));
        ui.close();
    }
}

/// The unstaged list's cleanup menu (right-click on its trash button).
pub fn cleanup_menu(ui: &mut egui::Ui, tab: &RepoTab, acts: &mut Vec<Action>) {
    crate::util::menu_pad(ui);
    let n = tab.snap.status.unstaged.iter().filter(|f| f.state == FileState::Untracked).count();
    if ui.add_enabled(n > 0, egui::Button::new(format!("Delete all new files ({n})"))).on_hover_text("git clean -fd; saved first, Undo brings them back").clicked() {
        acts.push(Action::CleanNew);
        ui.close();
    }
    ui.menu_button("More options", |ui| {
        crate::util::menu_pad(ui);
        if ui.button("Also remove ignored files…").on_hover_text("git clean -fdX: build output, caches; shows the space freed first").clicked() {
            acts.push(Action::OpenCleanIgnored);
            ui.close();
        }
    });
}

/// Submodule row extras: update, sync, remove; "new commits" when it moved.
pub fn submodule_items(ui: &mut egui::Ui, path: &str, pal: &Palette, acts: &mut Vec<Action>) {
    if ui.button("Update to latest").on_hover_text("git submodule update --remote").clicked() {
        acts.push(Action::SubmoduleUpdate(path.to_string()));
        ui.close();
    }
    if ui.button("Sync URL").on_hover_text("git submodule sync: after its URL changed in .gitmodules").clicked() {
        acts.push(Action::SubmoduleSync(path.to_string()));
        ui.close();
    }
    if ui.button(RichText::new("Remove").color(pal.removed)).on_hover_text("deinit, rm and git's own copy, in one go").clicked() {
        acts.push(Action::SubmoduleRemove(path.to_string()));
        ui.close();
    }
}

// ---------------------------------------------------------------------------------------------
// Commit options
// ---------------------------------------------------------------------------------------------

/// "More options" under the commit message, closed by default, remembered per repository.
pub fn commit_options(ui: &mut egui::Ui, tab: &mut RepoTab, pal: &Palette, acts: &mut Vec<Action>) {
    let mut o: CommitOptions = tab.commit_opts.clone();
    // Open or closed is remembered with the options, per repository.
    ui.horizontal(|ui| {
        let arrow = if o.open { "▾" } else { "▸" };
        let label = ui.add(egui::Label::new(RichText::new(format!("{arrow} More options")).size(12.0).color(pal.text2())).sense(egui::Sense::click()));
        if label.clicked() {
            o.open = !o.open;
        }
        let on = [o.signoff, o.sign, o.no_verify].iter().filter(|b| **b).count() + o.coauthors.len();
        if on > 0 {
            ui.label(RichText::new(format!("({on} on)")).size(11.0).color(pal.accent));
        }
    });
    if o.open {
        ui.checkbox(&mut o.signoff, "Add a sign-off line").on_hover_text("--signoff: \"Signed-off-by: you\"");
        if tab.snap.identity.is_complete() && tab.path.as_ref().is_some_and(|p| extras::can_sign(p)) {
            ui.checkbox(&mut o.sign, "Sign this commit").on_hover_text("-S, with your configured key");
        }
        ui.checkbox(&mut o.no_verify, "Skip hooks").on_hover_text("--no-verify");
        if o.no_verify {
            ui.label(RichText::new("Checks the project set up (formatting, tests) won't run for this commit.").size(11.5).color(pal.warn));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Co-authors").size(12.0).color(pal.text2()));
            let mut remove = None;
            for (i, c) in o.coauthors.iter().enumerate() {
                let name = c.split(" <").next().unwrap_or(c);
                if ui.small_button(format!("{name}  ×")).on_hover_text(c).clicked() {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                o.coauthors.remove(i);
            }
            let me = tab.snap.identity.email();
            let authors = extras::authors(&tab.snap.commits, me);
            egui::ComboBox::from_id_salt("coauthor_add").width(120.0).selected_text("Add…").show_ui(ui, |ui| {
                crate::util::menu_pad(ui);
                let mut add = None;
                for a in authors.iter().filter(|a| !o.coauthors.contains(a)).take(30) {
                    if ui.selectable_label(false, a).clicked() {
                        add = Some(a.clone());
                    }
                }
                if let Some(a) = add {
                    o.coauthors.push(a);
                }
            });
        });
    }
    if o != tab.commit_opts {
        acts.push(Action::SetCommitOptions(o));
    }
}

// ---------------------------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------------------------

pub fn dialog(ui: &mut egui::Ui, dialog: &mut Dialog, tab: Option<&RepoTab>, pal: &Palette, acts: &mut Vec<Action>) -> bool {
    match dialog {
        Dialog::Onto { branch, options, new_base, old_base } => {
            title(ui, "Move only my commits");
            ui.label(RichText::new(format!("For a branch started from another branch that was merged as one commit: only {branch}'s own commits move.")).color(pal.muted));
            egui::Grid::new("onto").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Onto");
                combo(ui, "onto_new", options, new_base);
                ui.end_row();
                ui.label("My commits start after");
                combo(ui, "onto_old", options, old_base);
                ui.end_row();
            });
            let (nb, ob) = (options.get(*new_base).cloned().unwrap_or_default(), options.get(*old_base).cloned().unwrap_or_default());
            let n = tab.and_then(|t| t.path.clone()).map(|p| extras::count_between(&p, &ob, branch)).unwrap_or(0);
            ui.label(RichText::new(format!("{n} commit{} of {branch} will move onto {nb}.", if n == 1 { "" } else { "s" })).strong());
            if crate::ui::footer(ui, pal, "Move", n > 0 && nb != ob) {
                acts.push(Action::MoveOnto { branch: branch.clone(), new_base: nb, old_base: ob });
                return true;
            }
            false
        }
        Dialog::AddSubmodule { url, path } => {
            title(ui, "Add a submodule");
            ui.label(RichText::new("Another repository inside this one, at a folder of your choice.").color(pal.muted));
            ui.add(egui::TextEdit::singleline(url).hint_text("https://github.com/owner/library.git").desired_width(f32::INFINITY));
            if path.is_empty() {
                if let Some(name) = url.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']).next().filter(|n| !n.is_empty() && url.contains('/')) {
                    ui.label(RichText::new(format!("Folder (suggested: libs/{name})")).size(12.0).color(pal.muted));
                }
            }
            ui.add(egui::TextEdit::singleline(path).hint_text("libs/library").desired_width(f32::INFINITY));
            let valid = !url.trim().is_empty() && !path.trim().is_empty();
            if crate::ui::footer(ui, pal, "Add", valid) {
                acts.push(Action::AddSubmodule { url: url.trim().into(), path: path.trim().into() });
                return true;
            }
            false
        }
        Dialog::CleanIgnored { files, size } => {
            title(ui, "Remove ignored files?");
            ui.label(format!("{} item{} ({}) that git ignores: build output, caches, downloads.", files.len(), if files.len() == 1 { "" } else { "s" }, human_size(*size)));
            if files.iter().any(|f| f == "Library" || f.starts_with("Library/")) {
                ui.label(RichText::new("Library/ is included: Unity will re-import the whole project next time (this can take a long time).").color(pal.warn));
            }
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for f in files.iter().take(200) {
                    ui.label(RichText::new(f).monospace().size(12.0));
                }
            });
            ui.label(RichText::new("These are not saved first: this can't be undone.").color(pal.removed));
            if crate::ui::footer(ui, pal, "Remove them", true) {
                acts.push(Action::CleanIgnored);
                return true;
            }
            false
        }
        Dialog::Sparse { folders } => {
            title(ui, "Choose folders to download");
            ui.label(RichText::new("Unchecked folders are removed from this computer (they stay in the history). Files at the top level always stay.").color(pal.muted));
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (f, on) in folders.iter_mut() {
                    let depth = f.matches('/').count();
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 18.0);
                        ui.checkbox(on, f.rsplit('/').next().unwrap_or(f));
                    });
                }
            });
            // A parent checked keeps all its children (cone mode): only the outermost count.
            let kept: Vec<String> = folders
                .iter()
                .filter(|(f, on)| *on && !folders.iter().any(|(p, pon)| *pon && f.starts_with(&format!("{p}/"))))
                .map(|(f, _)| f.clone())
                .collect();
            let all = folders.iter().all(|(_, on)| *on);
            let mut close = false;
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if all { "Download everything" } else { "Apply" };
                if ui.add_enabled(all || !kept.is_empty(), accent_button(label, pal)).on_hover_text("git sparse-checkout set --cone / disable").clicked() {
                    acts.push(Action::SetSparse(if all { None } else { Some(kept.clone()) }));
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
            close
        }
        Dialog::ApplyPatch { file, preview } => {
            title(ui, &format!("Apply {}?", file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            match preview {
                Ok(stat) => {
                    ui.label(RichText::new("What it changes:").color(pal.muted));
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        ui.label(RichText::new(stat.as_str()).monospace().size(12.0));
                    });
                }
                Err(e) => {
                    ui.label(RichText::new(format!("It doesn't apply cleanly here: {e}")).color(pal.removed));
                }
            }
            if crate::ui::footer(ui, pal, "Apply", preview.is_ok()) {
                acts.push(Action::ApplyPatch(file.clone()));
                return true;
            }
            false
        }
        Dialog::StashBranch { stash, name } => {
            title(ui, "Turn the stash into a branch");
            ui.label(RichText::new(format!("A new branch from where {stash} was made, with its changes applied; the stash is then removed.")).color(pal.muted));
            ui.add(egui::TextEdit::singleline(name).hint_text("feature/saved-work").desired_width(f32::INFINITY)).request_focus();
            let valid = !name.trim().is_empty() && !name.contains(' ');
            if crate::ui::footer(ui, pal, "Create branch", valid) {
                acts.push(Action::Git { label: format!("Branch {} created from the stash", name.trim()), args: extras::stash_branch_args(name.trim(), stash), background: false });
                return true;
            }
            false
        }
        Dialog::BisectRun { command } => {
            title(ui, "Run a test command automatically");
            ui.label(RichText::new("Vegit runs it on each commit to test: success means it works, failure means broken (exit code 125: can't test).").color(pal.muted));
            ui.add(egui::TextEdit::singleline(command).hint_text("cargo test").desired_width(f32::INFINITY)).request_focus();
            if crate::ui::footer(ui, pal, "Run", !command.trim().is_empty()) {
                acts.push(Action::BisectRun(command.trim().to_string()));
                return true;
            }
            false
        }
        Dialog::Grep { sha, query, results } => {
            title(ui, &format!("Search files at {}", git::short(sha)));
            let mut go = false;
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(query).hint_text("Text to find (any case)").desired_width(ui.available_width() - 80.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }
                if ui.add_enabled(!query.trim().is_empty(), accent_button("Search", pal)).clicked() {
                    go = true;
                }
            });
            if go && !query.trim().is_empty() {
                *results = None;
                acts.push(Action::GrepAt { sha: sha.clone(), query: query.trim().to_string() });
            }
            match results {
                None => {}
                Some(Err(e)) => {
                    ui.label(RichText::new(e.as_str()).color(pal.removed));
                }
                Some(Ok(hits)) if hits.is_empty() => {
                    ui.label(RichText::new("Not found in any file of this commit.").color(pal.muted));
                }
                Some(Ok(hits)) => {
                    ui.label(RichText::new(format!("{} line{}", hits.len(), if hits.len() == 1 { "" } else { "s" })).color(pal.muted));
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        for (path, line, text) in hits.iter() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{path}:{line}")).monospace().size(12.0).color(pal.accent));
                                ui.label(RichText::new(text.trim()).monospace().size(12.0));
                            });
                        }
                    });
                }
            }
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    ui.data_mut(|d| d.insert_temp(egui::Id::unique("dialog_cancel"), true));
                }
            });
            false
        }
        _ => false,
    }
}

fn combo(ui: &mut egui::Ui, id: &str, options: &[String], choice: &mut usize) {
    let shown = options.get(*choice).cloned().unwrap_or_default();
    egui::ComboBox::from_id_salt(id).width(240.0).selected_text(shown).show_ui(ui, |ui| {
        crate::util::menu_pad(ui);
        for (i, o) in options.iter().enumerate() {
            ui.selectable_value(choice, i, o);
        }
    });
}
