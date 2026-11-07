//! End-to-end checks of the git layer against a throwaway repository in the temp folder.
//! They pin down the parsing (log, refs, status) and single-hunk staging, which the UI relies on.

use std::path::{Path, PathBuf};

use crate::diff::ParsedDiff;
use crate::git::{self, DiffSource, FileState};
use crate::graph;

fn temp_repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vegit-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    g(&dir, &["init", "-q", "-b", "main"]);
    g(&dir, &["config", "user.name", "Test User"]);
    g(&dir, &["config", "user.email", "test@example.com"]);
    g(&dir, &["config", "commit.gpgsign", "false"]);
    dir
}

fn g(dir: &Path, args: &[&str]) -> String {
    git::run(dir, args).unwrap_or_else(|e| panic!("git {args:?} failed: {e}"))
}

fn write(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
}

#[test]
fn snapshot_reads_history_branches_and_status() {
    let dir = temp_repo("snap");
    write(&dir, "a.txt", "one\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "first"]);
    g(&dir, &["switch", "-q", "-c", "feature"]);
    write(&dir, "b.txt", "feature\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "feature work"]);
    g(&dir, &["switch", "-q", "main"]);
    write(&dir, "a.txt", "one\ntwo\n");
    g(&dir, &["commit", "-q", "-am", "main work"]);
    g(&dir, &["merge", "-q", "--no-edit", "feature"]);
    g(&dir, &["tag", "v1"]);
    write(&dir, "a.txt", "one\ntwo\nthree\n");
    write(&dir, "new.txt", "new\n");

    let snap = git::load_snapshot(&dir).unwrap();
    assert_eq!(snap.commits.len(), 4);
    assert_eq!(snap.head_branch.as_deref(), Some("main"));
    assert!(snap.refs.iter().any(|r| r.name == "feature"));
    assert!(snap.refs.iter().any(|r| r.name == "v1"));
    assert_eq!(snap.commits[0].parents.len(), 2, "newest commit is the merge");
    assert!(snap.status.unstaged.iter().any(|f| f.path == "a.txt" && f.state == FileState::Modified));
    assert!(snap.status.unstaged.iter().any(|f| f.path == "new.txt" && f.state == FileState::Untracked));

    let layout = graph::layout_for(&snap.commits, snap.head_sha.as_deref());
    assert_eq!(layout.rows.len(), 4);
    assert_eq!(layout.rows[0].col, 0);
    assert!(layout.width >= 2);

    let files = git::commit_files(&dir, &snap.commits[snap.commits.len() - 1]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "a.txt");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn single_hunk_can_be_staged_and_unstaged() {
    let dir = temp_repo("hunk");
    let original: String = (1..=40).map(|i| format!("line {i}\n")).collect();
    write(&dir, "f.txt", &original);
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "base"]);
    // Two changes far apart give two hunks.
    let changed = original.replace("line 2\n", "line 2 changed\n").replace("line 38\n", "line 38 changed\n");
    write(&dir, "f.txt", &changed);

    let status = git::load_status(&dir).unwrap();
    let file = status.unstaged[0].clone();
    let parsed = ParsedDiff::parse(&git::file_diff_ctx(&dir, &DiffSource::Unstaged, &file, false).0);
    assert_eq!(parsed.hunks.len(), 2);
    assert_eq!((parsed.added, parsed.removed), (2, 2));

    git::run_with_input(&dir, &["apply", "--cached", "-"], &parsed.hunk_patch(0).unwrap()).unwrap();
    let staged = ParsedDiff::parse(&git::file_diff_ctx(&dir, &DiffSource::Staged, &file, false).0);
    assert_eq!(staged.hunks.len(), 1);
    assert!(staged.hunks[0].lines.iter().any(|l| l.text == "line 2 changed"));

    git::run_with_input(&dir, &["apply", "--cached", "-R", "-"], &staged.hunk_patch(0).unwrap()).unwrap();
    assert!(git::load_status(&dir).unwrap().staged.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sample_graph_keeps_main_line_in_first_column() {
    let snap = crate::demo::snapshot();
    let layout = graph::layout_for(&snap.commits, snap.head_sha.as_deref());
    // "Initial commit" sits on the first lane, with the same colour as the newest commit.
    let last = layout.rows.last().unwrap();
    assert_eq!(last.col, 0);
    assert_eq!(last.color, layout.rows[0].color);
}

/// A Unity scene is a huge YAML file where saving rewrites many lines. The diff must be
/// produced and parsed off the window thread in reasonable time, with a flat row index
/// so the view only draws what is visible.
#[test]
fn huge_unity_like_scene_diff_is_parsed_into_rows() {
    let dir = temp_repo("unity");
    let scene: String = (0..60_000)
        .map(|i| format!("--- !u!4 &{i}\nTransform:\n  m_LocalPosition: {{x: {i}, y: 0, z: 0}}\n"))
        .collect();
    write(&dir, "Main.unity", &scene);
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "scene"]);
    write(&dir, "Main.unity", &scene.replace("y: 0", "y: 1"));

    let file = git::load_status(&dir).unwrap().unstaged[0].clone();
    let started = std::time::Instant::now();
    let (text, truncated) = git::file_diff_ctx(&dir, &DiffSource::Unstaged, &file, false);
    let parsed = ParsedDiff::parse(&text);
    let took = started.elapsed();
    assert!(!truncated);
    assert_eq!((parsed.added, parsed.removed), (60_000, 60_000));
    assert!(parsed.rows.len() > 120_000);
    assert!(took.as_secs() < 30, "diff took {took:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Clicking a changed picture must show it before and after the change: the "before"
/// comes from git, the "after" from the working folder.
#[test]
fn changed_png_loads_both_sides_for_the_render_view() {
    let dir = temp_repo("png");
    let png = |color: [u8; 4]| {
        let img = image::RgbaImage::from_pixel(8, 6, image::Rgba(color));
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        bytes
    };
    std::fs::write(dir.join("hero.png"), png([255, 0, 0, 255])).unwrap();
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "art"]);
    std::fs::write(dir.join("hero.png"), png([0, 0, 255, 255])).unwrap();

    let file = git::load_status(&dir).unwrap().unstaged[0].clone();
    assert!(crate::preview::kind_of(&file.path).is_some());
    let pair = crate::preview::load_pair(&dir, &DiffSource::Unstaged, &file);
    let before = pair.before.expect("before side").expect("decodes");
    let after = pair.after.expect("after side").expect("decodes");
    assert_eq!((before.width, before.height), (8, 6));
    assert_eq!(before.image.pixels[0].r(), 255);
    assert_eq!(after.image.pixels[0].b(), 255);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------------
// Phase 1 "quick wins": each feature's git layer, against a local bare repository standing in
// for the server.
// ---------------------------------------------------------------------------------------------

/// A repository with one commit on `main`, plus a bare "server" it is not yet connected to.
fn repo_and_server(name: &str) -> (PathBuf, PathBuf) {
    let dir = temp_repo(name);
    write(&dir, "a.txt", "one\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "first"]);
    let server = std::env::temp_dir().join(format!("vegit-test-{name}-server-{}.git", std::process::id()));
    let _ = std::fs::remove_dir_all(&server);
    std::fs::create_dir_all(&server).unwrap();
    g(&server, &["init", "-q", "--bare", "-b", "main"]);
    (dir, server)
}

fn run_args(dir: &Path, args: Vec<String>) -> String {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    g(dir, &refs)
}

fn cleanup(paths: &[&Path]) {
    for p in paths {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// Add, check, rename, re-point and remove a remote; the access check explains a bad address.
#[test]
fn remotes_can_be_added_checked_renamed_repointed_and_removed() {
    let (dir, server) = repo_and_server("remotes");
    let url = server.display().to_string();
    assert_eq!(git::remote_check(&dir, &url), Ok(0), "an empty bare repository is reachable");
    let bad = git::remote_check(&dir, &dir.join("nope").display().to_string()).unwrap_err();
    assert!(bad.contains("Check the URL") || bad.contains("Git could not get in"), "{bad}");

    run_args(&dir, git::remote_add_args("origin", &url));
    let snap = git::load_snapshot(&dir).unwrap();
    assert_eq!(snap.remotes, vec!["origin".to_string()]);
    run_args(&dir, git::remote_rename_args("origin", "server"));
    run_args(&dir, git::remote_set_url_args("server", "https://example.com/team/app.git"));
    let snap = git::load_snapshot(&dir).unwrap();
    assert_eq!(snap.remote_urls, vec![("server".to_string(), "https://example.com/team/app.git".to_string())]);
    run_args(&dir, git::remote_remove_args("server"));
    assert!(git::load_snapshot(&dir).unwrap().remotes.is_empty());
    cleanup(&[&dir, &server]);
}

/// origin first; the same repository under another owner is the fork's "upstream".
#[test]
fn remote_names_are_proposed_from_the_url() {
    assert_eq!(git::suggest_remote_name(&[], "https://github.com/me/app.git"), "origin");
    let origin = vec![("origin".to_string(), "https://github.com/me/app.git".to_string())];
    assert_eq!(git::suggest_remote_name(&origin, "https://github.com/acme/app.git"), "upstream");
    assert_eq!(git::suggest_remote_name(&origin, "git@github.com:alice/other.git"), "alice");
    let taken = vec![origin[0].clone(), ("upstream".to_string(), "https://github.com/x/app.git".to_string())];
    assert_eq!(git::suggest_remote_name(&taken, "https://github.com/acme/app.git"), "acme");
}

/// Tags are pushed by explicit refspec (lightweight ones too), seen on the server, then deleted there.
#[test]
fn tags_are_pushed_one_by_one_and_deleted_on_the_server() {
    let (dir, server) = repo_and_server("tags");
    run_args(&dir, git::remote_add_args("origin", &server.display().to_string()));
    g(&dir, &["push", "-q", "-u", "origin", "main"]);
    g(&dir, &["tag", "light"]);
    g(&dir, &["tag", "-a", "annotated", "-m", "release"]);
    assert!(git::remote_tags(&dir, "origin").unwrap().is_empty());
    let mut reachable = git::tags_reachable(&dir, "main").unwrap();
    reachable.sort();
    assert_eq!(reachable, vec!["annotated".to_string(), "light".to_string()]);

    run_args(&dir, git::push_tags_args("origin", &reachable));
    let on_server = git::remote_tags(&dir, "origin").unwrap();
    assert!(on_server.contains("light") && on_server.contains("annotated"));
    run_args(&dir, git::delete_remote_tag_args("origin", "light"));
    let on_server = git::remote_tags(&dir, "origin").unwrap();
    assert!(!on_server.contains("light") && on_server.contains("annotated"));
    cleanup(&[&dir, &server]);
}

/// Track and stop tracking a remote branch; a branch deleted on the server shows as gone.
#[test]
fn upstream_can_be_set_unset_and_detected_as_gone() {
    let (dir, server) = repo_and_server("upstream");
    run_args(&dir, git::remote_add_args("origin", &server.display().to_string()));
    g(&dir, &["push", "-q", "origin", "main"]);
    g(&dir, &["push", "-q", "origin", "main:feature"]);
    g(&dir, &["fetch", "-q", "origin"]);
    g(&dir, &["branch", "feature"]);

    run_args(&dir, git::set_upstream_args("feature", "origin/feature"));
    let r = |snap: &git::Snapshot| snap.refs.iter().find(|r| r.name == "feature" && r.kind == git::RefKind::Local).cloned().unwrap();
    assert_eq!(r(&git::load_snapshot(&dir).unwrap()).upstream.as_deref(), Some("origin/feature"));
    run_args(&dir, git::unset_upstream_args("feature"));
    assert_eq!(r(&git::load_snapshot(&dir).unwrap()).upstream, None);

    run_args(&dir, git::set_upstream_args("feature", "origin/feature"));
    g(&dir, &["push", "-q", "origin", "--delete", "feature"]);
    g(&dir, &["fetch", "-q", "--prune", "origin"]);
    let feature = r(&git::load_snapshot(&dir).unwrap());
    assert!(feature.gone, "upstream deleted on the server is reported as gone");
    cleanup(&[&dir, &server]);
}

/// A branch whose commits are on no other branch: those commits are listed, and the branch can
/// be recreated on its SHA after a forced delete (what the Undo does).
#[test]
fn unmerged_branch_lists_its_own_commits_and_can_be_restored_after_forced_delete() {
    let dir = temp_repo("forcedelete");
    write(&dir, "a.txt", "one\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "first"]);
    g(&dir, &["switch", "-q", "-c", "feature"]);
    for k in 1..=3 {
        write(&dir, "f.txt", &format!("{k}\n"));
        g(&dir, &["add", "."]);
        g(&dir, &["commit", "-q", "-m", &format!("feature {k}")]);
    }
    g(&dir, &["switch", "-q", "main"]);
    g(&dir, &["branch", "merged"]);

    let own = git::unique_commits(&dir, "feature").unwrap();
    assert_eq!(own.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["feature 3", "feature 2", "feature 1"]);
    assert!(git::unique_commits(&dir, "merged").unwrap().is_empty(), "a branch contained in main loses nothing");
    assert!(git::run(&dir, &["branch", "-d", "feature"]).is_err(), "git refuses the safe delete");

    let sha = git::branch_sha(&dir, "feature").unwrap();
    g(&dir, &["branch", "-D", "feature"]);
    g(&dir, &["branch", "feature", &sha]);
    assert_eq!(git::branch_sha(&dir, "feature").unwrap(), sha);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rebase stopped on a conflict: skipping the commit finishes it.
#[test]
fn skipping_the_conflicting_commit_finishes_the_rebase() {
    let dir = temp_repo("skip");
    write(&dir, "a.txt", "base\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "base"]);
    g(&dir, &["switch", "-q", "-c", "feature"]);
    write(&dir, "a.txt", "feature\n");
    g(&dir, &["commit", "-q", "-am", "feature change"]);
    g(&dir, &["switch", "-q", "main"]);
    write(&dir, "a.txt", "main\n");
    g(&dir, &["commit", "-q", "-am", "main change"]);
    g(&dir, &["switch", "-q", "feature"]);
    assert!(git::run(&dir, &["rebase", "main"]).is_err(), "the rebase stops on the conflict");

    let pending = git::load_snapshot(&dir).unwrap().pending;
    assert_eq!(pending, Some(git::PendingOp::Rebase));
    assert!(pending.unwrap().can_skip());
    assert!(!git::PendingOp::Merge.can_skip());
    g(&dir, &[pending.unwrap().command(), "--skip"]);
    let snap = git::load_snapshot(&dir).unwrap();
    assert_eq!(snap.pending, None);
    assert_eq!(snap.commits[0].summary, "main change", "the skipped commit is dropped");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The identity of this project (local config) is read back and wins over the global one.
/// Only the local level is written here, never the person's global config.
#[test]
fn local_identity_is_written_read_and_preferred() {
    let dir = temp_repo("identity");
    run_args(&dir, git::set_identity_args(false, "user.name", "Local Name"));
    run_args(&dir, git::set_identity_args(false, "user.email", "local@example.com"));
    let id = git::identity(&dir);
    assert_eq!(id.local_name.as_deref(), Some("Local Name"));
    assert_eq!(id.name(), Some("Local Name"));
    assert_eq!(id.email(), Some("local@example.com"));
    assert!(id.is_complete());
    assert_eq!(id.source(), "this project");
    run_args(&dir, git::set_identity_args(false, "user.name", ""));
    assert_eq!(git::identity(&dir).local_name, None, "an empty value removes the local setting");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An identity set in a file the config includes (how `includeIf` setups work) counts: the
/// commit panel must not ask "Who are you?" when git can already commit.
#[test]
fn identity_from_an_included_file_is_found() {
    let dir = temp_repo("identity-include");
    g(&dir, &["config", "--unset", "user.name"]);
    g(&dir, &["config", "--unset", "user.email"]);
    let included = dir.join("identity.inc");
    write(&dir, "identity.inc", "[user]\n\tname = Included Person\n\temail = included@example.com\n");
    let path = included.to_string_lossy().replace('\\', "/");
    g(&dir, &["config", "include.path", &path]);
    let id = git::identity(&dir);
    assert_eq!(id.name(), Some("Included Person"));
    assert_eq!(id.email(), Some("included@example.com"));
    assert!(id.is_complete());
    assert_eq!(id.source(), "this project", "an included value takes the scope of the file that includes it");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The merge options: squash stages without committing, --no-ff always makes a merge commit,
/// --ff-only refuses when a merge commit would be needed.
#[test]
fn merge_options_squash_no_ff_and_ff_only() {
    let dir = temp_repo("mergeopts");
    write(&dir, "a.txt", "one\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "first"]);
    g(&dir, &["switch", "-q", "-c", "feature"]);
    write(&dir, "b.txt", "b\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "add b"]);
    write(&dir, "c.txt", "c\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "add c"]);
    g(&dir, &["switch", "-q", "main"]);

    // Squash: the changes are staged, no commit yet, and the message lists the commits.
    let (summary, body) = git::squash_message(&dir, "feature", "main");
    assert_eq!(summary, "Bring feature into main");
    assert_eq!(body, "- add b\n- add c");
    run_args(&dir, git::merge_args(git::MergeMode::Squash, "feature"));
    let snap = git::load_snapshot(&dir).unwrap();
    let head = snap.head_sha.clone().unwrap();
    assert_eq!(snap.commit_summary(&head), Some("first"), "nothing committed on main yet");
    assert_eq!(snap.status.staged.len(), 2);
    g(&dir, &["reset", "-q", "--hard"]);

    // --no-ff: a merge commit even though a fast-forward was possible.
    run_args(&dir, git::merge_args(git::MergeMode::NoFastForward, "feature"));
    let snap = git::load_snapshot(&dir).unwrap();
    let head = snap.head_sha.clone().unwrap();
    assert_eq!(snap.commits.iter().find(|c| c.sha == head).unwrap().parents.len(), 2);

    // --ff-only: main has moved on, feature diverged, so git refuses.
    g(&dir, &["switch", "-q", "feature"]);
    write(&dir, "d.txt", "d\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "add d"]);
    g(&dir, &["switch", "-q", "main"]);
    let args = git::merge_args(git::MergeMode::FastForwardOnly, "feature");
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    assert!(git::run(&dir, &refs).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------------
// Phase 2 "foundations": the safety net (record, undo, backups, reflog in words) and the
// environment variables of git commands.
// ---------------------------------------------------------------------------------------------

use crate::safety::{self, ResetMode, Step};

/// A repository with `main` at "first".
fn repo_one_commit(name: &str) -> PathBuf {
    let dir = temp_repo(name);
    write(&dir, "a.txt", "one\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "first"]);
    dir
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Runs a command the way the worker does (recorded) and returns how to undo it.
fn recorded(dir: &Path, v: &[&str]) -> safety::UndoEntry {
    let a = args(v);
    let (result, entry) = safety::run_recorded(dir, &[Step::Git(a.clone())], safety::describe(&a));
    result.unwrap();
    entry.expect("the command is recorded")
}

/// Runs an undo (recorded too, which is what Redo uses).
fn undo(dir: &Path, e: &safety::UndoEntry) -> Option<safety::UndoEntry> {
    let (result, redo) = safety::run_recorded(dir, &e.steps, Some((e.what.clone(), e.mode)));
    result.unwrap();
    redo
}

fn head(dir: &Path) -> String {
    g(dir, &["rev-parse", "HEAD"]).trim().to_string()
}

fn text(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).unwrap().replace("\r\n", "\n")
}

/// Extra environment variables reach git (used by backups, later by interactive rebase).
#[test]
fn environment_variables_reach_git() {
    let dir = repo_one_commit("env");
    let ident = git::run_env(&dir, &["var", "GIT_AUTHOR_IDENT"], &[("GIT_AUTHOR_NAME", "Somebody Else"), ("GIT_AUTHOR_EMAIL", "else@example.com")]).unwrap();
    assert!(ident.starts_with("Somebody Else <else@example.com>"), "{ident}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Undoing a commit puts the branch back with its changes staged; Redo brings the commit back.
#[test]
fn undo_commit_keeps_changes_staged_and_redo_restores_it() {
    let dir = repo_one_commit("undocommit");
    let first = head(&dir);
    write(&dir, "a.txt", "two\n");
    g(&dir, &["add", "."]);
    let e = recorded(&dir, &["commit", "-q", "-m", "second"]);
    let second = head(&dir);
    assert_eq!(e.what, "commit");
    assert_eq!(e.branches, vec!["main".to_string()]);
    let redo = undo(&dir, &e).expect("the undo is recorded for Redo");
    assert_eq!(head(&dir), first);
    assert!(g(&dir, &["diff", "--cached", "--name-only"]).contains("a.txt"), "the change is staged again");
    undo(&dir, &redo);
    assert_eq!(head(&dir), second);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Deleting, renaming and creating-and-switching branches are all taken back.
#[test]
fn branch_delete_rename_and_create_are_undone() {
    let dir = repo_one_commit("undobranch");
    g(&dir, &["branch", "feature"]);
    let tip = head(&dir);

    let e = recorded(&dir, &["branch", "-D", "feature"]);
    assert_eq!(e.what, "delete branch");
    undo(&dir, &e);
    assert_eq!(git::branch_sha(&dir, "feature").unwrap(), tip);

    let e = recorded(&dir, &["branch", "-m", "main", "trunk"]);
    undo(&dir, &e);
    assert_eq!(g(&dir, &["symbolic-ref", "--short", "HEAD"]).trim(), "main");
    assert!(git::branch_sha(&dir, "trunk").is_err());

    let e = recorded(&dir, &["switch", "-q", "-c", "new-idea"]);
    assert_eq!(e.what, "create branch");
    undo(&dir, &e);
    assert_eq!(g(&dir, &["symbolic-ref", "--short", "HEAD"]).trim(), "main");
    assert!(git::branch_sha(&dir, "new-idea").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A discard saves the files first (tracked and new ones) under refs/vegit/backups, and its
/// undo writes them back without touching the staging area.
#[test]
fn discarded_files_are_backed_up_and_restored() {
    let dir = repo_one_commit("undodiscard");
    write(&dir, "a.txt", "my precious edit\n");
    write(&dir, "new.txt", "a brand new file\n");
    let steps = vec![
        Step::Backup(vec!["new.txt".into()]),
        Step::Delete("new.txt".into()),
        Step::Git(args(&["restore", "--worktree", "--", "a.txt"])),
    ];
    let (result, entry) = safety::run_recorded(&dir, &steps, Some(("discard (2 files)".into(), ResetMode::Keep)));
    result.unwrap();
    assert_eq!(text(&dir, "a.txt"), "one\n");
    assert!(!dir.join("new.txt").exists());
    assert_eq!(g(&dir, &["for-each-ref", "refs/vegit/backups"]).lines().count(), 2, "one backup per saving step");
    undo(&dir, &entry.expect("recorded"));
    assert_eq!(text(&dir, "a.txt"), "my precious edit\n");
    assert_eq!(text(&dir, "new.txt"), "a brand new file\n");
    assert!(g(&dir, &["diff", "--cached", "--name-only"]).trim().is_empty(), "the staging area is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `reset --hard` no longer loses anything: the branch and the uncommitted changes come back.
#[test]
fn hard_reset_is_undone_with_its_uncommitted_changes() {
    let dir = repo_one_commit("undohard");
    let first = head(&dir);
    write(&dir, "a.txt", "two\n");
    g(&dir, &["commit", "-q", "-am", "second"]);
    let second = head(&dir);
    write(&dir, "a.txt", "work in progress\n");
    let e = recorded(&dir, &["reset", "-q", "--hard", &first]);
    assert_eq!(e.mode, ResetMode::Hard);
    assert_eq!(head(&dir), first);
    undo(&dir, &e);
    assert_eq!(head(&dir), second);
    assert_eq!(text(&dir, "a.txt"), "work in progress\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A drag-and-drop merge into another branch (switch, then merge) is one action: its undo
/// goes back to the first branch and puts the target where it was.
#[test]
fn merge_into_another_branch_is_undone_in_one_step() {
    let dir = repo_one_commit("undomergeinto");
    let first = head(&dir);
    g(&dir, &["switch", "-q", "-c", "feature"]);
    write(&dir, "b.txt", "b\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "add b"]);
    let steps = vec![Step::Git(args(&["switch", "-q", "main"])), Step::Git(git::merge_args(git::MergeMode::NoFastForward, "feature"))];
    let (result, entry) = safety::run_recorded(&dir, &steps, Some(("merge".into(), ResetMode::Keep)));
    result.unwrap();
    assert_ne!(git::branch_sha(&dir, "main").unwrap(), first);
    undo(&dir, &entry.expect("recorded"));
    assert_eq!(g(&dir, &["symbolic-ref", "--short", "HEAD"]).trim(), "feature");
    assert_eq!(git::branch_sha(&dir, "main").unwrap(), first);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Deleting a stash and stashing are both undone.
#[test]
fn stash_drop_and_push_are_undone() {
    let dir = repo_one_commit("undostash");
    write(&dir, "a.txt", "stashed\n");
    g(&dir, &["stash", "push", "-q", "-m", "keep me"]);
    let e = recorded(&dir, &["stash", "drop", "-q"]);
    assert!(g(&dir, &["stash", "list"]).trim().is_empty());
    undo(&dir, &e);
    assert!(g(&dir, &["stash", "list"]).contains("keep me"));

    g(&dir, &["stash", "pop", "-q"]);
    let e = recorded(&dir, &["stash", "push", "-q", "-m", "oops"]);
    undo(&dir, &e);
    assert!(g(&dir, &["stash", "list"]).trim().is_empty());
    assert_eq!(text(&dir, "a.txt"), "stashed\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Staging and configuration are not recorded; aborts and continues neither.
#[test]
fn only_actions_worth_taking_back_are_recorded() {
    for v in [&["add", "--", "a.txt"][..], &["reset", "-q"], &["config", "user.name", "x"], &["rebase", "--abort"], &["merge", "--continue"], &["apply", "--cached", "-"]] {
        assert!(safety::describe(&args(v)).is_none(), "{v:?}");
    }
    assert_eq!(safety::describe(&args(&["commit", "--amend", "--no-edit"])).unwrap().0, "amend");
    assert_eq!(safety::describe(&args(&["reset", "--hard", "HEAD~1"])).unwrap().1, ResetMode::Hard);
}

/// Old backups are deleted, recent ones kept.
#[test]
fn backups_older_than_thirty_days_are_purged() {
    let dir = repo_one_commit("purge");
    let tip = head(&dir);
    g(&dir, &["update-ref", "refs/vegit/backups/1000-1", &tip]);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    g(&dir, &["update-ref", &format!("refs/vegit/backups/{now}-1"), &tip]);
    safety::purge_backups(&dir, safety::BACKUP_DAYS);
    let left = g(&dir, &["for-each-ref", "--format=%(refname)", "refs/vegit/backups"]);
    assert_eq!(left.trim(), format!("refs/vegit/backups/{now}-1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reflog in plain words, without the in-between steps of a rebase.
#[test]
fn reflog_reads_as_sentences() {
    assert_eq!(safety::translate("commit: Fix the login").as_deref(), Some("Committed: Fix the login"));
    assert_eq!(safety::translate("checkout: moving from main to feature").as_deref(), Some("Switched from main to feature"));
    assert_eq!(safety::translate("rebase (finish): returning to refs/heads/feature").as_deref(), Some("Replayed feature on its new base"));
    assert_eq!(safety::translate("rebase (pick): Fix the login"), None);
    assert_eq!(safety::translate("merge feature: Fast-forward").as_deref(), Some("Brought feature in (fast-forward)"));
    assert_eq!(safety::translate(&format!("reset: moving to {}", "a".repeat(40))).as_deref(), Some("Moved the branch to aaaaaaa"));

    let dir = repo_one_commit("activity");
    g(&dir, &["switch", "-q", "-c", "feature"]);
    let act = safety::activity(&dir, 10);
    assert_eq!(act[0].text, "Switched from main to feature");
    assert_eq!(act[1].text, "First commit: first");
    assert!(act[0].time > 1_600_000_000);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------------
// Phase 3 "big pieces": editing history, adding to an earlier commit, conflicts block by
// block, worktrees and Git LFS.
// ---------------------------------------------------------------------------------------------

use crate::conflict::{self, Choice, Segment};
use crate::worktree;
use crate::rewrite::{self, Change};

/// The test's sequence editor: copy the prepared todo over git's (the app uses Vegit itself).
fn cp_editor(todo: &Path) -> String {
    format!("cp \"{}\"", todo.display().to_string().replace('\\', "/"))
}

/// main with four commits c1..c4 on a.txt..d.txt.
fn repo_four_commits(name: &str) -> PathBuf {
    let dir = temp_repo(name);
    for (i, f) in ["a", "b", "c", "d"].iter().enumerate() {
        write(&dir, &format!("{f}.txt"), &format!("{f}\n"));
        g(&dir, &["add", "."]);
        g(&dir, &["commit", "-q", "-m", &format!("c{}", i + 1)]);
    }
    dir
}

fn subjects(dir: &Path) -> Vec<String> {
    g(dir, &["log", "--format=%s"]).lines().map(str::to_string).collect()
}

fn sha_of(dir: &Path, subject: &str) -> String {
    g(dir, &["log", "--format=%H %s"]).lines().find(|l| l.ends_with(&format!(" {subject}"))).unwrap()[..40].to_string()
}

fn run_plan(dir: &Path, plan: &[rewrite::PlanItem]) {
    let steps = rewrite::plan_steps(dir, plan, &cp_editor).unwrap();
    let (result, entry) = safety::run_recorded(dir, &steps, Some(("rebase".into(), ResetMode::Keep)));
    result.unwrap();
    assert!(entry.is_some(), "a history edit can be undone");
}

/// Edit message, combine (both messages / the one below), remove and move, each as one rebase
/// with no editor opening.
#[test]
fn history_edits_run_without_an_editor() {
    let dir = repo_four_commits("rewrite");
    let (_, plan) = rewrite::single_plan(&dir, &sha_of(&dir, "c3"), Change::Reword, "c3 better\n\nwith a body").unwrap();
    run_plan(&dir, &plan);
    assert_eq!(subjects(&dir), ["c4", "c3 better", "c2", "c1"]);
    assert!(g(&dir, &["log", "-n", "1", "--skip", "1", "--format=%B"]).contains("with a body"));

    let (_, plan) = rewrite::single_plan(&dir, &sha_of(&dir, "c4"), Change::Fixup, "").unwrap();
    run_plan(&dir, &plan);
    assert_eq!(subjects(&dir), ["c3 better", "c2", "c1"]);
    assert!(dir.join("d.txt").exists(), "c4's changes are folded into the commit below");

    let (_, plan) = rewrite::single_plan(&dir, &sha_of(&dir, "c2"), Change::Squash, "").unwrap();
    run_plan(&dir, &plan);
    let msg = g(&dir, &["log", "-n", "1", "--skip", "1", "--format=%B"]);
    assert!(msg.contains("c1") && msg.contains("c2") && !msg.contains('#'), "both messages, comments cleaned: {msg}");

    let (_, plan) = rewrite::move_plan(&dir, &sha_of(&dir, "c3 better"), false).unwrap();
    let err = rewrite::check(&plan, &rewrite::history_from(&dir, &plan[0].sha).unwrap());
    assert!(err.is_ok(), "{err:?}");
    run_plan(&dir, &plan);
    assert_eq!(subjects(&dir)[1], "c3 better");

    let (_, plan) = rewrite::single_plan(&dir, &sha_of(&dir, "c3 better"), Change::Drop, "").unwrap();
    run_plan(&dir, &plan);
    assert!(!dir.join("c.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A merge in the range is refused with a sentence; combining the oldest commit too.
#[test]
fn history_edits_refuse_merges_and_impossible_plans() {
    let dir = repo_four_commits("rewritemerge");
    g(&dir, &["switch", "-q", "-c", "side", "HEAD~1"]);
    write(&dir, "s.txt", "s\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "side"]);
    g(&dir, &["switch", "-q", "main"]);
    g(&dir, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
    let from = sha_of(&dir, "c3");
    let history = rewrite::history_from(&dir, &from).unwrap();
    let plan = rewrite::plan_of(&history);
    assert!(rewrite::check(&plan, &history).unwrap_err().contains("merge commit"));
    let mut plan = rewrite::plan_of(&history[..1]);
    plan[0].change = Change::Squash;
    assert!(rewrite::check(&plan, &history[..1]).unwrap_err().contains("no commit below"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Add to an earlier commit": staged changes land in the chosen commit.
#[test]
fn staged_changes_are_added_to_an_earlier_commit() {
    let dir = repo_four_commits("fixup");
    write(&dir, "b.txt", "b fixed\n");
    g(&dir, &["add", "b.txt"]);
    let candidates = rewrite::fixup_candidates(&dir);
    assert_eq!(candidates.len(), 4);
    let target = sha_of(&dir, "c2");
    let (result, _) = safety::run_recorded(&dir, &rewrite::fixup_steps(&dir, &target), Some(("add to an earlier commit".into(), ResetMode::Keep)));
    result.unwrap();
    assert_eq!(subjects(&dir), ["c4", "c3", "c2", "c1"], "no extra fixup commit is left");
    let c2 = sha_of(&dir, "c2");
    assert_eq!(g(&dir, &["show", &format!("{c2}:b.txt")]).replace("\r\n", "\n"), "b fixed\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The hidden sequence-editor mode copies the prepared todo over git's.
#[test]
fn sequence_editor_mode_copies_the_todo() {
    let dir = temp_repo("seqedit");
    write(&dir, "todo", "pick abc\n");
    write(&dir, "git-todo", "noop\n");
    let args = vec!["vegit".to_string(), "--sequence-editor".into(), dir.join("todo").display().to_string(), dir.join("git-todo").display().to_string()];
    assert_eq!(rewrite::sequence_editor_main(&args), Some(0));
    assert_eq!(std::fs::read_to_string(dir.join("git-todo")).unwrap(), "pick abc\n");
    assert_eq!(rewrite::sequence_editor_main(&["vegit".to_string()]), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Blocks are parsed with their original, resolved one at a time, line endings kept.
#[test]
fn conflict_blocks_are_parsed_and_resolved_one_by_one() {
    let text = "top\r\n<<<<<<< HEAD\r\nmine\r\n||||||| base\r\norig\r\n=======\r\ntheirs\r\n>>>>>>> feature\r\nmiddle\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> feature\r\nend\r\n";
    let segs = conflict::parse(text);
    assert_eq!(conflict::count(&segs), 2);
    let b = conflict::blocks(&segs)[0].clone();
    assert_eq!((b.ours.as_str(), b.base.as_deref(), b.theirs.as_str()), ("mine\r\n", Some("orig\r\n"), "theirs\r\n"));
    assert_eq!((b.ours_label.as_str(), b.theirs_label.as_str()), ("HEAD", "feature"));
    let once = conflict::resolve(&segs, 0, &Choice::Theirs);
    assert!(once.starts_with("top\r\ntheirs\r\nmiddle\r\n<<<<<<< HEAD\r\n"), "{once}");
    let segs = conflict::parse(&once);
    assert_eq!(conflict::count(&segs), 1);
    assert_eq!(conflict::resolve(&segs, 0, &Choice::Both), "top\r\ntheirs\r\nmiddle\r\nx\r\ny\r\nend\r\n");
    assert_eq!(conflict::resolve(&segs, 0, &Choice::Text("z".into())), "top\r\ntheirs\r\nmiddle\r\nz\r\nend\r\n");
    assert!(matches!(conflict::parse("<<<<<<< HEAD\nunfinished\n").as_slice(), [Segment::Text(_)]));
}

/// A real merge conflict made through Vegit's git carries the original (zdiff3), and the
/// sides are named after the branches.
#[test]
fn real_conflicts_carry_the_original_and_branch_names() {
    let dir = temp_repo("zdiff3");
    write(&dir, "a.txt", "line\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "base"]);
    g(&dir, &["switch", "-q", "-c", "feature"]);
    write(&dir, "a.txt", "feature line\n");
    g(&dir, &["commit", "-q", "-am", "feature"]);
    g(&dir, &["switch", "-q", "main"]);
    write(&dir, "a.txt", "main line\n");
    g(&dir, &["commit", "-q", "-am", "main"]);
    assert!(git::run(&dir, &["merge", "feature"]).is_err());
    let segs = conflict::parse(&std::fs::read_to_string(dir.join("a.txt")).unwrap());
    let b = conflict::blocks(&segs)[0].clone();
    assert_eq!(b.base.as_deref().map(|s| s.replace("\r\n", "\n")), Some("line\n".to_string()));
    let names = conflict::side_names(&dir, Some(git::PendingOp::Merge), &b.ours_label, &b.theirs_label);
    assert_eq!(names, ("main".to_string(), "feature".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Open a branch in another folder, see it listed with its changes, remove it; a vanished
/// folder is pruned; git's refusal names the folder.
#[test]
fn worktrees_are_added_listed_removed_and_pruned() {
    let dir = repo_one_commit("worktree");
    g(&dir, &["branch", "feature"]);
    let other = worktree::suggest_path(&dir, "feature/login");
    assert!(other.file_name().unwrap().to_string_lossy().ends_with("-feature-login"));
    let path = dir.parent().unwrap().join(format!("vegit-test-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    run_args(&dir, worktree::add_args(&path, "feature", false, None));
    write(&path, "a.txt", "changed there\n");
    let list = worktree::list(&dir);
    assert_eq!(list.len(), 2);
    assert!(list[0].main);
    assert_eq!((list[1].branch.as_deref(), list[1].changes), (Some("feature"), 1));

    let err = git::run(&dir, &["switch", "feature"]).unwrap_err().0;
    assert!(worktree::path_in_error(&err).is_some_and(|p| p.contains("vegit-test-wt")), "{err}");

    assert!(git::run(&dir, &worktree::remove_args(&path, false).iter().map(String::as_str).collect::<Vec<_>>()).is_err(), "changes there: refused");
    run_args(&dir, worktree::remove_args(&path, true));
    assert_eq!(worktree::list(&dir).len(), 1);

    run_args(&dir, worktree::add_args(&path, "fresh", false, Some("main")));
    std::fs::remove_dir_all(&path).unwrap();
    assert!(worktree::list(&dir).iter().any(|w| w.prunable));
    let snap = git::load_snapshot(&dir).unwrap();
    assert_eq!((snap.pruned, snap.worktrees.len()), (1, 1));
    let _ = std::fs::remove_dir_all(&dir);
}

/// LFS: patterns, sizes, ages, lock lists and which changed files LFS stores.
#[test]
fn lfs_tracking_locks_and_sizes() {
    assert_eq!(crate::lfs::pattern_for("Art/hero.psd"), "*.psd");
    assert_eq!(crate::lfs::pattern_for("Makefile"), "Makefile");
    assert_eq!(crate::lfs::human_size(13_002_342), "12.4 MB");
    assert_eq!(crate::lfs::track_args("*.unity", true), ["lfs", "track", "--lockable", "*.unity"]);
    let json = r#"{"ours":[{"id":"1","path":"Assets/Main.unity","owner":{"name":"me"},"locked_at":"2026-11-03T10:00:00Z"}],"theirs":[{"id":"2","path":"Assets/Hero.prefab","owner":{"name":"Alice"},"locked_at":"2026-11-03T10:00:00Z"}]}"#;
    let locks = crate::lfs::parse_verified(json);
    assert_eq!(locks.len(), 2);
    assert!(locks[0].mine && !locks[1].mine);
    assert_eq!(locks[1].owner, "Alice");
    // 2026-11-03 is a Tuesday; four days later.
    let now = 1_793_750_400 + 4 * 86_400;
    assert_eq!(crate::lfs::since(&locks[1].locked_at, now), "since Tuesday");
    assert!(crate::lfs::is_pointer(b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 12\n"));

    let dir = repo_one_commit("lfs");
    write(&dir, ".gitattributes", "*.psd filter=lfs diff=lfs merge=lfs -text\n");
    write(&dir, "hero.psd", "pixels");
    let info = crate::lfs::info(&dir, &["hero.psd".into(), "a.txt".into()]);
    assert!(info.used);
    assert_eq!(info.tracked.iter().collect::<Vec<_>>(), [&"hero.psd".to_string()]);
    assert!(!info.unity);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Unlock offer after a push only names files the push carried: on the first push of a
/// branch (no earlier position on the server), on a later one, and nothing when the push sent
/// nothing (the last update of the remote branch was a fetch).
#[test]
fn pushed_files_are_the_ones_the_push_sent() {
    let (dir, server) = repo_and_server("pushedfiles");
    let url = server.display().to_string().replace('\\', "/");
    g(&dir, &["remote", "add", "origin", &url]);
    g(&dir, &["push", "-q", "-u", "origin", "main"]);
    g(&dir, &["switch", "-q", "-c", "art"]);
    write(&dir, "hero.psd", "v1\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "hero"]);
    g(&dir, &["push", "-q", "-u", "origin", "art"]);
    assert_eq!(git::pushed_files(&dir), ["hero.psd"], "first push of the branch: only its own commits");

    write(&dir, "hero.psd", "v2\n");
    write(&dir, "b.txt", "b\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "more"]);
    g(&dir, &["push", "-q"]);
    assert_eq!(git::pushed_files(&dir), ["b.txt", "hero.psd"]);

    g(&dir, &["fetch", "-q", "origin", "+art:refs/remotes/origin/art"]);
    g(&dir, &["update-ref", "-m", "fetch: forced", "refs/remotes/origin/art", "HEAD~1"]);
    assert!(git::pushed_files(&dir).is_empty(), "the last update wasn't a push");
    cleanup(&[&dir, &server]);
}

/// A history edit that stops on a commit leaves a rebase in progress: no Undo is recorded
/// (git would refuse to switch back), the banner's Abort takes it back.
#[test]
fn an_edit_that_stops_midway_records_no_undo() {
    let dir = repo_four_commits("stopmidway");
    let (_, plan) = rewrite::single_plan(&dir, &sha_of(&dir, "c3"), Change::Edit, "").unwrap();
    let steps = rewrite::plan_steps(&dir, &plan, &cp_editor).unwrap();
    let (result, entry) = safety::run_recorded(&dir, &steps, Some(("rebase".into(), ResetMode::Keep)));
    result.unwrap();
    assert_eq!(git::pending_op(&dir), Some(git::PendingOp::Rebase));
    assert!(entry.is_none());

    // Actions made while it is stopped are still recorded.
    write(&dir, "c.txt", "changed\n");
    let a: Vec<String> = ["commit", "-q", "-am", "while stopped"].iter().map(|s| s.to_string()).collect();
    let (result, entry) = safety::run_recorded(&dir, &[Step::Git(a.clone())], safety::describe(&a));
    result.unwrap();
    assert!(entry.is_some());
    g(&dir, &["rebase", "--abort"]);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------------
// Phases 4 and 5: lines, comparing, searching, paging, stash, copying commits, bisect,
// rebase --onto, submodules, clone, cleanup, sparse-checkout, patches, signatures,
// maintenance and commit options.
// ---------------------------------------------------------------------------------------------

use crate::extras;

/// A file with three changed lines; staging two of them leaves the third unstaged.
#[test]
fn chosen_lines_are_staged_and_unstaged() {
    let dir = temp_repo("lines");
    write(&dir, "a.txt", "one\ntwo\nthree\nfour\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "base"]);
    write(&dir, "a.txt", "one\nTWO\nthree\nFOUR\nfive\n");
    let diff = g(&dir, &["diff", "-U3", "--", "a.txt"]);
    let parsed = crate::diff::ParsedDiff::parse(&diff);
    let patch = parsed.hunk_patch(0).unwrap();
    // Body lines: " one", "-two", "+TWO", " three", "-four", "+FOUR", "+five".
    let partial = extras::line_patch(&patch, &[1, 2], false).unwrap();
    git::run_with_input(&dir, &["apply", "--cached", "-"], &partial).unwrap();
    assert_eq!(g(&dir, &["show", ":a.txt"]).replace("\r\n", "\n"), "one\nTWO\nthree\nfour\n");
    // Unstage it again from the staged diff.
    let staged = crate::diff::ParsedDiff::parse(&g(&dir, &["diff", "--cached", "-U3", "--", "a.txt"]));
    let sp = staged.hunk_patch(0).unwrap();
    let back = extras::line_patch(&sp, &[1, 2], true).unwrap();
    git::run_with_input(&dir, &["apply", "--cached", "-R", "-"], &back).unwrap();
    assert!(g(&dir, &["diff", "--cached", "--name-only"]).trim().is_empty());
    assert!(extras::line_patch(&patch, &[0], false).is_none(), "a context line alone changes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two commits compared, two branches compared (commits only on each, files since the fork).
#[test]
fn commits_and_branches_are_compared() {
    let dir = repo_four_commits("compare");
    let (c1, c4) = (sha_of(&dir, "c1"), sha_of(&dir, "c4"));
    let files = extras::compare_files(&dir, &c1, &c4).unwrap();
    assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["b.txt", "c.txt", "d.txt"]);
    g(&dir, &["switch", "-q", "-c", "feature", &sha_of(&dir, "c2")]);
    write(&dir, "f.txt", "f\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "f"]);
    assert_eq!(extras::only_on(&dir, "main", "feature").len(), 1);
    assert_eq!(extras::only_on(&dir, "feature", "main").len(), 2);
    let base = extras::merge_base(&dir, "main", "feature").unwrap();
    assert_eq!(base, sha_of(&dir, "c2"));
    let changed = extras::compare_files(&dir, &base, "feature").unwrap();
    assert_eq!(changed.len(), 1, "what feature changed since the fork");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Changed text (`log -S`), file paths, and `git grep` at a commit.
#[test]
fn content_is_searched() {
    let dir = repo_four_commits("search");
    write(&dir, "b.txt", "needle here\n");
    g(&dir, &["commit", "-q", "-am", "add needle"]);
    let hits = extras::search_changed_text(&dir, "needle").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits.contains(&sha_of(&dir, "add needle")));
    assert_eq!(extras::search_file_path(&dir, "C.TX").unwrap().len(), 1, "case-insensitive path search");
    let found = extras::grep_at(&dir, &head(&dir), "NEEDLE").unwrap();
    assert_eq!(found, vec![("b.txt".to_string(), 1, "needle here".to_string())]);
    assert!(extras::grep_at(&dir, &head(&dir), "absent").unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Paging: a higher limit loads more; a commit's position tells how far to load.
#[test]
fn history_loads_further_on_demand() {
    let dir = repo_four_commits("paging");
    assert_eq!(extras::position_of(&dir, &sha_of(&dir, "c1")), Some(3));
    assert_eq!(git::commit_limit(&dir), git::MAX_COMMITS);
    git::set_commit_limit(&dir, git::MAX_COMMITS * 2);
    assert_eq!(git::commit_limit(&dir), git::MAX_COMMITS * 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Stash some files only, keep the staged ones, turn a stash into a branch, delete all (undone).
#[test]
fn advanced_stash() {
    let dir = repo_four_commits("stash2");
    write(&dir, "a.txt", "a changed\n");
    write(&dir, "b.txt", "b changed\n");
    run_args(&dir, extras::stash_files_args(&["a.txt".into()], "only a"));
    assert!(g(&dir, &["status", "--porcelain"]).contains("b.txt"), "b stays");
    assert!(!g(&dir, &["status", "--porcelain"]).contains("a.txt"));
    g(&dir, &["add", "b.txt"]);
    g(&dir, &["stash", "push", "-q", "--keep-index", "-m", "keep staged"]);
    assert!(g(&dir, &["diff", "--cached", "--name-only"]).contains("b.txt"), "--keep-index leaves the staged file");
    g(&dir, &["reset", "-q", "--hard"]);
    let e = recorded(&dir, &["stash", "clear"]);
    assert_eq!(e.what, "delete all stashes");
    undo(&dir, &e);
    let list = g(&dir, &["stash", "list"]);
    assert!(list.lines().next().unwrap().contains("keep staged") && list.contains("only a"), "{list}");
    run_args(&dir, extras::stash_branch_args("from-stash", "stash@{1}"));
    assert_eq!(g(&dir, &["symbolic-ref", "--short", "HEAD"]).trim(), "from-stash");
    assert_eq!(text(&dir, "a.txt"), "a changed\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Several commits copied in order, with the original mentioned.
#[test]
fn several_commits_are_copied_oldest_first() {
    let dir = repo_four_commits("multipick");
    let (c3, c4) = (sha_of(&dir, "c3"), sha_of(&dir, "c4"));
    g(&dir, &["switch", "-q", "-c", "other", &sha_of(&dir, "c2")]);
    run_args(&dir, extras::cherry_pick_args(&[c3.clone(), c4], true, false));
    assert_eq!(subjects(&dir)[..2], ["c4".to_string(), "c3".to_string()]);
    assert!(g(&dir, &["log", "-n", "1", "--skip", "1", "--format=%B"]).contains(&format!("cherry picked from commit {c3}")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bisect: the state is read, the answers narrow it down, the first broken commit is named.
#[test]
fn bisect_finds_the_first_broken_commit() {
    let dir = repo_four_commits("bisect");
    let (c1, c3) = (sha_of(&dir, "c1"), sha_of(&dir, "c3"));
    g(&dir, &["bisect", "start", "HEAD", &c1]);
    let st = extras::bisect_state(&dir).unwrap();
    assert_eq!(st.candidates.len(), 3);
    // "broken" means c.txt exists: c3 is the first broken one.
    let mut found = None;
    for _ in 0..4 {
        let verdict = if dir.join("c.txt").exists() { "bad" } else { "good" };
        let out = g(&dir, &["bisect", verdict]);
        if let extras::BisectOutcome::Found(sha) = extras::parse_bisect(&out) {
            found = Some(sha);
            break;
        }
    }
    assert_eq!(found, Some(c3));
    g(&dir, &["bisect", "reset"]);
    assert!(extras::bisect_state(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Move only my commits": a branch built on another one moves onto main alone.
#[test]
fn only_my_commits_move_onto_another_base() {
    let dir = repo_one_commit("onto");
    g(&dir, &["switch", "-q", "-c", "feature-a"]);
    write(&dir, "a2.txt", "a\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "a work"]);
    g(&dir, &["switch", "-q", "-c", "feature-b"]);
    write(&dir, "b2.txt", "b\n");
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "b work"]);
    assert_eq!(extras::count_between(&dir, "feature-a", "feature-b"), 1);
    run_args(&dir, extras::onto_args("main", "feature-a", "feature-b"));
    assert_eq!(subjects(&dir), ["b work", "first"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Clone options, shallow detection, cleanup with its backup, ignored files and their size.
#[test]
fn clone_options_and_cleanup() {
    let src = repo_four_commits("clonesrc");
    let dest = src.parent().unwrap().join(format!("vegit-test-clone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    let url = format!("file://{}", src.display().to_string().replace('\\', "/"));
    let args = extras::clone_args(&url, &dest, Some("main"), true, true);
    assert!(args.contains(&"--depth".to_string()) && args.contains(&"--recurse-submodules".to_string()));
    run_args(src.parent().unwrap(), args);
    assert!(extras::is_shallow(&dest));
    assert_eq!(extras::remote_heads(&src, &url).unwrap(), ["main"]);

    write(&src, "junk.tmp", "x");
    write(&src, ".gitignore", "*.log\n");
    write(&src, "big.log", &"y".repeat(2000));
    let (result, entry) = safety::run_recorded(&src, &extras::clean_steps(&["junk.tmp".into(), ".gitignore".into()]), Some(("delete new files".into(), ResetMode::Keep)));
    result.unwrap();
    assert!(!src.join("junk.tmp").exists() && src.join("big.log").exists(), "ignored files are kept");
    undo(&src, &entry.unwrap());
    assert!(src.join("junk.tmp").exists());
    let (ignored, size) = extras::ignored(&src);
    assert_eq!(ignored, ["big.log"]);
    assert_eq!(size, 2000);
    cleanup(&[&src, &dest]);
}

/// Sparse-checkout: the folders listed, a choice applied, then everything back.
#[test]
fn sparse_checkout_keeps_chosen_folders() {
    let dir = temp_repo("sparse");
    for f in ["Assets/Art/a.txt", "Assets/Code/b.txt", "Docs/c.txt"] {
        std::fs::create_dir_all(dir.join(f).parent().unwrap()).unwrap();
        write(&dir, f, "x\n");
    }
    g(&dir, &["add", "."]);
    g(&dir, &["commit", "-q", "-m", "tree"]);
    let mut fs = extras::folders(&dir);
    fs.sort();
    assert_eq!(fs, ["Assets", "Assets/Art", "Assets/Code", "Docs"]);
    run_args(&dir, extras::sparse_set_args(&["Assets/Code".into()]));
    assert_eq!(extras::sparse_folders(&dir), Some(vec!["Assets/Code".to_string()]));
    assert!(!dir.join("Docs/c.txt").exists());
    g(&dir, &["sparse-checkout", "disable"]);
    assert!(dir.join("Docs/c.txt").exists());
    assert_eq!(extras::sparse_folders(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A commit saved as a patch file is previewed and applied elsewhere, keeping its author.
#[test]
fn patches_are_saved_previewed_and_applied() {
    let dir = repo_four_commits("patch");
    let c4 = sha_of(&dir, "c4");
    let patch = extras::format_patch(&dir, &[c4]).unwrap();
    let file = dir.join("c4.patch");
    std::fs::write(&file, &patch).unwrap();
    g(&dir, &["reset", "-q", "--hard", "HEAD~1"]);
    let preview = extras::patch_preview(&dir, &file).unwrap();
    assert!(preview.contains("d.txt"), "{preview}");
    let args = extras::patch_apply_args(&file);
    assert_eq!(args[0], "am");
    run_args(&dir, args);
    assert_eq!(subjects(&dir)[0], "c4");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Unsigned history has no badge; maintenance measures and checks; commit options build
/// arguments and co-author trailers.
#[test]
fn signatures_maintenance_and_commit_options() {
    let dir = repo_four_commits("maint");
    assert!(extras::signatures(&dir, 10).is_empty());
    assert_eq!(extras::signature_text('G'), "Signed, verified");
    assert!(extras::repo_size(&dir) > 0);
    g(&dir, &["gc", "-q"]);
    assert!(extras::check(&dir).unwrap().is_empty(), "a healthy repository has no problems");
    let opts = extras::CommitOptions { signoff: true, no_verify: true, coauthors: vec!["Ana <ana@example.com>".into()], ..Default::default() };
    assert_eq!(opts.args(), ["--signoff", "--no-verify"]);
    assert_eq!(opts.description("Body"), "Body\n\nCo-authored-by: Ana <ana@example.com>");
    assert_eq!(opts.description(""), "Co-authored-by: Ana <ana@example.com>");
    let snap = git::load_snapshot(&dir).unwrap();
    assert!(extras::authors(&snap.commits, None).len() == 1);
    assert!(extras::authors(&snap.commits, Some(&snap.commits[0].email)).is_empty(), "not myself");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A submodule added, updated, synced and removed in one action.
#[test]
fn submodules_are_added_and_removed() {
    let lib = repo_one_commit("sublib");
    let dir = repo_one_commit("subhost");
    let url = format!("file://{}", lib.display().to_string().replace('\\', "/"));
    g(&dir, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", &url, "libs/lib"]);
    g(&dir, &["commit", "-q", "-m", "add lib"]);
    run_args(&dir, extras::submodule_sync_args("libs/lib"));
    let (result, _) = safety::run_recorded(&dir, &extras::submodule_remove_steps(&dir, "libs/lib"), None);
    result.unwrap();
    assert!(!dir.join("libs/lib").exists());
    assert!(!dir.join(".git/modules/libs/lib").exists());
    assert!(g(&dir, &["diff", "--cached", "--name-only"]).contains(".gitmodules"));
    cleanup(&[&lib, &dir]);
}
