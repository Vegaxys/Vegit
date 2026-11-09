//! One pull request in detail, and the edits the pull request view makes (GitHub).
//!
//! Reading gathers what the view shows in one go (description, commits with their
//! co-authors, comments, reviews, checks, files); edits are single API calls. Everything
//! here is blocking and runs on a background thread. Other services are listed and created
//! in `forge.rs`; this detailed view needs GitHub's API for now.

use std::path::Path;

use serde_json::{Value, json};

use crate::access;
use crate::forge::{self, Forge, RemoteRepo, text_at};

#[derive(Clone, Debug, Default)]
pub struct PrCommit {
    pub sha: String,
    pub summary: String,
    pub author: String,
    /// "Name <email>" from the `Co-authored-by:` trailers.
    pub coauthors: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PrComment {
    pub author: String,
    pub body: String,
    pub when: String,
}

#[derive(Clone, Debug, Default)]
pub struct PrCheck {
    pub name: String,
    /// "success", "failure", "pending", "neutral", "skipped"…
    pub state: String,
}

#[derive(Clone, Debug, Default)]
pub struct PrFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    pub status: String,
}

/// Everything the pull request view shows.
#[derive(Clone, Debug, Default)]
pub struct PrDetail {
    pub number: u64,
    pub title: String,
    pub body: String,
    /// "open", "closed" or "merged".
    pub state: String,
    pub draft: bool,
    pub author: String,
    pub source: String,
    pub target: String,
    pub url: String,
    pub created: String,
    pub additions: u64,
    pub deletions: u64,
    /// `None` while GitHub is still computing it.
    pub mergeable: Option<bool>,
    pub reviewers: Vec<String>,
    /// (reviewer, "APPROVED" / "CHANGES_REQUESTED" / "COMMENTED").
    pub reviews: Vec<(String, String)>,
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub commits: Vec<PrCommit>,
    pub comments: Vec<PrComment>,
    pub checks: Vec<PrCheck>,
    pub files: Vec<PrFile>,
    /// For the draft switch (GitHub's GraphQL id).
    pub node_id: String,
}

impl PrDetail {
    /// Everyone who co-wrote a commit of this pull request, once each.
    pub fn coauthors(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for c in self.commits.iter().flat_map(|c| &c.coauthors) {
            if !v.contains(c) {
                v.push(c.clone());
            }
        }
        v
    }

    /// Author, commenters, reviewers and commit authors, once each.
    pub fn participants(&self) -> Vec<String> {
        let mut v = vec![self.author.clone()];
        let others = self.comments.iter().map(|c| &c.author).chain(self.reviews.iter().map(|r| &r.0)).chain(self.commits.iter().map(|c| &c.author));
        for p in others {
            if !p.is_empty() && !v.contains(p) {
                v.push(p.clone());
            }
        }
        v
    }
}

/// `Co-authored-by: Name <email>` trailers of a commit message (any case), in order.
pub fn coauthors_of(message: &str) -> Vec<String> {
    message
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (key, value) = l.split_once(':')?;
            key.trim().eq_ignore_ascii_case("co-authored-by").then(|| value.trim().to_string()).filter(|v| !v.is_empty())
        })
        .collect()
}

/// The repository and its sign-in, for GitHub only.
fn github(dir: &Path) -> Result<(RemoteRepo, Option<(String, String)>, String), String> {
    let repo = forge::repo_of(dir)?;
    if repo.forge != Forge::GitHub {
        return Err(format!("The pull request view works with GitHub for now; open this {} in the browser.", repo.forge.noun()));
    }
    let cred = access::credential_fill(&repo.host, false);
    let base = format!("{}/repos/{}", forge::github_api(&repo), repo.path);
    Ok((repo, cred, base))
}

fn list(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// Reads one pull request with everything the view shows.
pub fn detail(dir: &Path, number: u64) -> Result<PrDetail, String> {
    let (repo, cred, base) = github(dir)?;
    let get = |path: String| forge::call("GET", &format!("{base}{path}"), &repo, &cred, None);
    let p = get(format!("/pulls/{number}"))?;
    let merged = p["merged"].as_bool().unwrap_or(false);
    let mut d = PrDetail {
        number,
        title: text_at(&p, &["title"]),
        body: text_at(&p, &["body"]),
        state: if merged { "merged".into() } else { text_at(&p, &["state"]) },
        draft: p["draft"].as_bool().unwrap_or(false),
        author: text_at(&p, &["user", "login"]),
        source: text_at(&p, &["head", "ref"]),
        target: text_at(&p, &["base", "ref"]),
        url: text_at(&p, &["html_url"]),
        created: text_at(&p, &["created_at"]).replace('T', " ").trim_end_matches('Z').to_string(),
        additions: p["additions"].as_u64().unwrap_or(0),
        deletions: p["deletions"].as_u64().unwrap_or(0),
        mergeable: p["mergeable"].as_bool(),
        reviewers: list(&p["requested_reviewers"]).iter().map(|r| text_at(r, &["login"])).collect(),
        assignees: list(&p["assignees"]).iter().map(|r| text_at(r, &["login"])).collect(),
        labels: list(&p["labels"]).iter().map(|l| text_at(l, &["name"])).collect(),
        milestone: Some(text_at(&p, &["milestone", "title"])).filter(|m| !m.is_empty()),
        node_id: text_at(&p, &["node_id"]),
        ..Default::default()
    };
    // The rest is best effort: a missing part leaves its list empty rather than failing.
    d.commits = list(&get(format!("/pulls/{number}/commits?per_page=100")).unwrap_or_default())
        .iter()
        .map(|c| {
            let message = text_at(c, &["commit", "message"]);
            let login = text_at(c, &["author", "login"]);
            PrCommit {
                sha: text_at(c, &["sha"]),
                summary: message.lines().next().unwrap_or_default().to_string(),
                author: if login.is_empty() { text_at(c, &["commit", "author", "name"]) } else { login },
                coauthors: coauthors_of(&message),
            }
        })
        .collect();
    d.comments = list(&get(format!("/issues/{number}/comments?per_page=100")).unwrap_or_default())
        .iter()
        .map(|c| PrComment { author: text_at(c, &["user", "login"]), body: text_at(c, &["body"]), when: text_at(c, &["created_at"]).replace('T', " ").trim_end_matches('Z').to_string() })
        .collect();
    d.reviews = list(&get(format!("/pulls/{number}/reviews?per_page=100")).unwrap_or_default())
        .iter()
        .map(|r| (text_at(r, &["user", "login"]), text_at(r, &["state"])))
        .filter(|(_, s)| s != "PENDING")
        .collect();
    d.files = list(&get(format!("/pulls/{number}/files?per_page=100")).unwrap_or_default())
        .iter()
        .map(|f| PrFile { path: text_at(f, &["filename"]), additions: f["additions"].as_u64().unwrap_or(0), deletions: f["deletions"].as_u64().unwrap_or(0), status: text_at(f, &["status"]) })
        .collect();
    let head = text_at(&p, &["head", "sha"]);
    if let Ok(runs) = get(format!("/commits/{head}/check-runs?per_page=100")) {
        d.checks = list(&runs["check_runs"])
            .iter()
            .map(|r| {
                let conclusion = text_at(r, &["conclusion"]);
                PrCheck { name: text_at(r, &["name"]), state: if conclusion.is_empty() { "pending".into() } else { conclusion } }
            })
            .collect();
    }
    if let Ok(status) = get(format!("/commits/{head}/status")) {
        d.checks.extend(list(&status["statuses"]).iter().map(|s| PrCheck { name: text_at(s, &["context"]), state: text_at(s, &["state"]) }));
    }
    Ok(d)
}

/// The pull request view open in a tab, with what is being typed in it.
#[derive(Clone, Debug, Default)]
pub struct PrView {
    pub number: u64,
    /// `None` while loading.
    pub detail: Option<Result<PrDetail, String>>,
    /// 0 overview, 1 commits, 2 checks, 3 files changed.
    pub page: usize,
    pub title_edit: Option<String>,
    pub body_edit: Option<String>,
    pub comment: String,
    /// 0 merge commit, 1 squash, 2 rebase.
    pub merge_method: usize,
    pub labels_edit: Option<String>,
    pub reviewer_add: String,
    pub assignee_add: String,
    /// An edit is on its way.
    pub busy: bool,
}

impl PrView {
    pub fn new(number: u64) -> Self {
        Self { number, ..Default::default() }
    }
}

/// One change made from the pull request view.
#[derive(Clone, Debug)]
pub enum PrEdit {
    Title(String),
    Body(String),
    Comment(String),
    /// Close (`false`) or reopen (`true`).
    Open(bool),
    /// "merge", "squash" or "rebase".
    Merge(String),
    Labels(Vec<String>),
    AddReviewer(String),
    RemoveReviewer(String),
    AddAssignee(String),
    RemoveAssignee(String),
    /// Draft (`true`) or ready for review (`false`), by node id.
    Draft(bool, String),
}

impl PrEdit {
    /// What the toast says once it's done.
    pub fn done_text(&self) -> String {
        match self {
            PrEdit::Title(_) => "Title saved".into(),
            PrEdit::Body(_) => "Description saved".into(),
            PrEdit::Comment(_) => "Comment posted".into(),
            PrEdit::Open(true) => "Pull request reopened".into(),
            PrEdit::Open(false) => "Pull request closed".into(),
            PrEdit::Merge(_) => "Pull request merged".into(),
            PrEdit::Labels(_) => "Labels saved".into(),
            PrEdit::AddReviewer(r) => format!("Review requested from {r}"),
            PrEdit::RemoveReviewer(r) => format!("{r} removed from the reviewers"),
            PrEdit::AddAssignee(a) => format!("{a} assigned"),
            PrEdit::RemoveAssignee(a) => format!("{a} unassigned"),
            PrEdit::Draft(true, _) => "Converted to draft".into(),
            PrEdit::Draft(false, _) => "Ready for review".into(),
        }
    }
}

/// Sends one edit to GitHub.
pub fn edit(dir: &Path, number: u64, e: &PrEdit) -> Result<(), String> {
    let (repo, cred, base) = github(dir)?;
    let call = |method: &str, path: String, body: Value| forge::call(method, &format!("{base}{path}"), &repo, &cred, Some(body)).map(|_| ());
    match e {
        PrEdit::Title(t) => call("PATCH", format!("/pulls/{number}"), json!({ "title": t })),
        PrEdit::Body(b) => call("PATCH", format!("/pulls/{number}"), json!({ "body": b })),
        PrEdit::Comment(c) => call("POST", format!("/issues/{number}/comments"), json!({ "body": c })),
        PrEdit::Open(open) => call("PATCH", format!("/pulls/{number}"), json!({ "state": if *open { "open" } else { "closed" } })),
        PrEdit::Merge(method) => call("PUT", format!("/pulls/{number}/merge"), json!({ "merge_method": method })),
        PrEdit::Labels(l) => call("PUT", format!("/issues/{number}/labels"), json!({ "labels": l })),
        PrEdit::AddReviewer(r) => call("POST", format!("/pulls/{number}/requested_reviewers"), json!({ "reviewers": [r] })),
        PrEdit::RemoveReviewer(r) => call("DELETE", format!("/pulls/{number}/requested_reviewers"), json!({ "reviewers": [r] })),
        PrEdit::AddAssignee(a) => call("POST", format!("/issues/{number}/assignees"), json!({ "assignees": [a] })),
        PrEdit::RemoveAssignee(a) => call("DELETE", format!("/issues/{number}/assignees"), json!({ "assignees": [a] })),
        PrEdit::Draft(draft, node) => {
            // Only GitHub's GraphQL API switches a pull request to draft and back.
            let mutation = if *draft { "convertPullRequestToDraft" } else { "markPullRequestReadyForReview" };
            let query = format!("mutation($id: ID!) {{ {mutation}(input: {{pullRequestId: $id}}) {{ clientMutationId }} }}");
            let url = if repo.host == "github.com" { "https://api.github.com/graphql".to_string() } else { format!("https://{}/api/graphql", repo.host) };
            let v = forge::call("POST", &url, &repo, &cred, Some(json!({ "query": query, "variables": { "id": node } })))?;
            match v.get("errors").and_then(|e| e[0]["message"].as_str()) {
                Some(m) => Err(format!("GitHub answered: {m}")),
                None => Ok(()),
            }
        }
    }
}
