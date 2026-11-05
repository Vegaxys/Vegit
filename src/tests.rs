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
