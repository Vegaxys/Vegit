//! A built-in sample repository held in memory.
//!
//! It lets the first screen (and `--screenshot`) show a realistic graph without touching
//! any folder on disk. Actions that would change a repository are refused in the sample.

use crate::git::{Commit, FileChange, FileState, RefInfo, RefKind, Snapshot, Stash, WorkStatus};

const NOW: i64 = 1_793_600_000;

fn sha(n: u32) -> String {
    format!("{:08x}{:032x}", n.wrapping_mul(2_654_435_761), u128::from(n) * 0x9e37_79b9_7f4a_7c15)
}

pub fn snapshot() -> Snapshot {
    // (id, parents, author, minutes ago, message)
    let spec: &[(u32, &[u32], usize, i64, &str)] = &[
        (30, &[29], 0, 12, "Polish empty state of the search panel"),
        (29, &[27, 28], 1, 55, "Merge branch 'feature/search' into main"),
        (28, &[26], 2, 90, "Search: highlight matches in results"),
        (27, &[25], 0, 140, "Fix crash when the settings file is missing"),
        (26, &[24], 2, 200, "Search: debounce typing in the query box"),
        (31, &[32], 3, 230, "Login: remember the last used email"),
        (25, &[23], 1, 300, "Bump version to 1.4.0"),
        (24, &[22], 2, 380, "Search: first version of the results list"),
        (32, &[23], 3, 460, "Login: show a friendly error on timeout"),
        (23, &[21, 22], 0, 560, "Merge branch 'fix/offline-banner'"),
        (22, &[20], 4, 640, "Offline banner: hide when connection returns"),
        (21, &[19], 1, 760, "Update translations"),
        (20, &[18], 4, 900, "Offline banner: first draft"),
        (19, &[18], 0, 1100, "Refactor settings storage"),
        (18, &[17], 1, 1300, "Add keyboard shortcuts help"),
        (17, &[16], 0, 1500, "Tidy up the toolbar layout"),
        (16, &[15], 2, 1800, "Cache thumbnails on disk"),
        (15, &[14], 1, 2100, "Release 1.3.0"),
        (14, &[13], 0, 2500, "Fix typo in onboarding text"),
        (13, &[12], 3, 2900, "Add dark mode preview"),
        (12, &[11], 0, 3400, "Improve startup time"),
        (11, &[10], 1, 4000, "Set up continuous integration"),
        (10, &[], 0, 4800, "Initial commit"),
    ];
    let authors = [
        ("Alex Martin", "alex@example.com"),
        ("Sam Rivera", "sam@example.com"),
        ("Noor Haddad", "noor@example.com"),
        ("Lee Chen", "lee@example.com"),
        ("Jo Dubois", "jo@example.com"),
    ];
    let mut commits: Vec<Commit> = spec
        .iter()
        .map(|(id, parents, a, mins, msg)| Commit {
            sha: sha(*id),
            parents: parents.iter().map(|p| sha(*p)).collect(),
            author: authors[*a].0.to_string(),
            email: authors[*a].1.to_string(),
            time: NOW - mins * 60,
            summary: (*msg).to_string(),
            body: match id {
                30 => "The panel now explains what to type instead of showing an empty box.",
                28 => "Matches are wrapped in a highlight span; ranking is unchanged.",
                27 => "A missing file now falls back to the defaults and logs a warning.",
                26 => "Waits 150 ms after the last key before searching.",
                31 => "Stored locally only, never sent anywhere.",
                22 => "Listens to the connectivity event instead of polling.",
                19 => "Settings live in one JSON file with a version number.",
                12 => "Assets load in the background while the first screen draws.",
                _ => "",
            }
            .to_string(),
            stash: None,
        })
        .collect();
    let stashes = vec![Stash {
        reference: "stash@{0}".into(),
        message: "WIP on main: try a new icon set".into(),
        sha: sha(99),
        base: Some(sha(25)),
        time: NOW - 270 * 60,
        author: "Alex Martin".into(),
        email: "alex@example.com".into(),
    }];
    crate::git::insert_stashes(&mut commits, &stashes);

    let r = |name: &str, kind: RefKind, id: u32, up: Option<&str>, ahead: u32, behind: u32| RefInfo {
        name: name.to_string(),
        kind,
        target: sha(id),
        upstream: up.map(str::to_string),
        ahead,
        behind,
    };
    let refs = vec![
        r("main", RefKind::Local, 30, Some("origin/main"), 1, 0),
        r("feature/login", RefKind::Local, 31, Some("origin/feature/login"), 0, 0),
        r("feature/search", RefKind::Local, 28, None, 0, 0),
        r("origin/main", RefKind::Remote, 29, None, 0, 0),
        r("origin/feature/login", RefKind::Remote, 31, None, 0, 0),
        r("v1.4.0", RefKind::Tag, 25, None, 0, 0),
        r("v1.3.0", RefKind::Tag, 15, None, 0, 0),
    ];

    let f = |p: &str, s: FileState| FileChange { path: p.to_string(), old_path: None, state: s };
    Snapshot {
        commits,
        refs,
        stashes,
        head_sha: Some(sha(30)),
        head_branch: Some("main".into()),
        status: WorkStatus {
            staged: vec![f("src/search/results.rs", FileState::Modified)],
            unstaged: vec![
                f("src/search/panel.rs", FileState::Modified),
                f("assets/empty-state.svg", FileState::Untracked),
                f("docs/old-notes.md", FileState::Deleted),
            ],
        },
        pending: None,
        remotes: vec!["origin".into()],
        submodules: vec!["libs/shared-ui".into()],
        remote_urls: vec![("origin".into(), "https://github.com/acme/sample-project.git".into())],
    }
}

pub fn files_for(_sha: &str) -> Vec<FileChange> {
    vec![
        FileChange { path: "src/search/panel.rs".into(), old_path: None, state: FileState::Modified },
        FileChange { path: "src/search/empty.rs".into(), old_path: None, state: FileState::Added },
    ]
}

/// The sample's new illustration, shown by the render view.
pub const SAMPLE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="220" viewBox="0 0 320 220">
  <rect x="10" y="10" width="300" height="200" rx="28" fill="#1f2a44"/>
  <circle cx="110" cy="100" r="46" fill="none" stroke="#2d8cff" stroke-width="12"/>
  <line x1="144" y1="134" x2="190" y2="180" stroke="#2d8cff" stroke-width="14" stroke-linecap="round"/>
  <rect x="200" y="62" width="80" height="14" rx="7" fill="#ff8a3d"/>
  <rect x="200" y="92" width="60" height="14" rx="7" fill="#9b7bff"/>
  <rect x="200" y="122" width="70" height="14" rx="7" fill="#3ecf8e"/>
</svg>
"##;

pub fn diff_for(path: &str) -> String {
    if path.ends_with(".svg") {
        let lines: Vec<&str> = SAMPLE_SVG.lines().collect();
        let mut d = format!("diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n", lines.len());
        for l in lines {
            d.push('+');
            d.push_str(l);
            d.push('\n');
        }
        return d;
    }
    format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -10,9 +10,12 @@ impl SearchPanel {{\n     pub fn show(&mut self, ui: &mut Ui) {{\n         let query = self.query.trim();\n-        if query.is_empty() {{\n-            ui.label(\"Nothing here\");\n+        if query.is_empty() && self.results.is_empty() {{\n+            // A friendlier empty state with a hint.\n+            ui.heading(\"Start typing to search\");\n+            ui.label(\"Try a file name, a word or an author.\");\n             return;\n         }}\n         for result in &self.results {{\n             result.show(ui);\n         }}\n@@ -40,4 +43,4 @@ impl SearchPanel {{\n     fn limit(&self) -> usize {{\n-        50\n+        100\n     }}\n }}\n"
    )
}

/// Made-up open pull requests for the sample project.
pub fn pull_requests() -> crate::app::PrState {
    use crate::forge::{Forge, PullRequest, RemoteRepo};
    let repo = RemoteRepo { forge: Forge::GitHub, host: "github.com".into(), path: "acme/sample-project".into(), remote: "origin".into() };
    let pr = |number: u64, title: &str, author: &str, source: &str, draft: bool| PullRequest {
        number,
        title: title.into(),
        author: author.into(),
        source: source.into(),
        target: "main".into(),
        url: format!("https://github.com/acme/sample-project/pull/{number}"),
        draft,
    };
    crate::app::PrState::Ready(
        repo,
        vec![
            pr(42, "Login: remember the last used email", "lee-chen", "feature/login", false),
            pr(41, "Search: results list and highlighting", "noor-h", "feature/search", true),
        ],
    )
}
