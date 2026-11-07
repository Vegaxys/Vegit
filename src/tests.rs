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
