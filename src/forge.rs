//! Pull requests on the hosting service behind a remote: GitHub, GitLab (also
//! self-hosted), Bitbucket Cloud and Azure DevOps.
//!
//! Everything here is blocking and runs on a background thread. The token comes from
//! git's own credential helper (the same sign-in `git push` uses), so the app stores no
//! secret itself. Automatic refreshes never open a sign-in window; only the explicit
//! "Sign in" button may.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::access;
use crate::git;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    Bitbucket,
    Azure,
}

impl Forge {
    pub fn name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Bitbucket => "Bitbucket",
            Self::Azure => "Azure DevOps",
        }
    }

    /// What the service calls a pull request.
    pub fn noun(self) -> &'static str {
        if self == Self::GitLab { "merge request" } else { "pull request" }
    }
}

/// The repository on the hosting service, as found from a remote URL.
#[derive(Clone, Debug)]
pub struct RemoteRepo {
    pub forge: Forge,
    pub host: String,
    /// `owner/name` on GitHub and Bitbucket, the full group path on GitLab,
    /// `organisation/project/repository` on Azure DevOps.
    pub path: String,
    pub remote: String,
}

#[derive(Clone, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub source: String,
    pub target: String,
    pub url: String,
    pub draft: bool,
}

/// Splits `https://host/a/b.git`, `git@host:a/b.git` or `ssh://git@host/a/b` into host and path.
pub fn split_url(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next().unwrap_or(host).to_string(), path.to_string())
    } else {
        // scp-like: user@host:path
        let rest = url.rsplit_once('@').map_or(url, |(_, r)| r);
        let (host, path) = rest.split_once(':')?;
        (host.to_string(), path.to_string())
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git").to_string();
    (!host.is_empty() && !path.is_empty()).then_some((host.to_lowercase(), path))
}

pub fn detect(remote: &str, url: &str) -> Option<RemoteRepo> {
    let (host, path) = split_url(url)?;
    let (forge, host, path) = if host.contains("github") {
        (Forge::GitHub, host, path)
    } else if host.contains("gitlab") {
        (Forge::GitLab, host, path)
    } else if host.contains("bitbucket") {
        (Forge::Bitbucket, "bitbucket.org".to_string(), path)
    } else if host.contains("dev.azure.com") || host.contains("visualstudio.com") {
        // https://dev.azure.com/org/project/_git/repo  or  ssh.dev.azure.com:v3/org/project/repo
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && *p != "_git" && *p != "v3").collect();
        if parts.len() < 3 {
            return None;
        }
        (Forge::Azure, "dev.azure.com".to_string(), parts[parts.len() - 3..].join("/"))
    } else {
        return None;
    };
    Some(RemoteRepo { forge, host, path, remote: remote.to_string() })
}

/// The hosting-service repository of `dir`, preferring the `origin` remote.
pub fn repo_of(dir: &Path) -> Result<RemoteRepo, String> {
    let remotes = git::run(dir, &["remote"]).map_err(|e| e.0)?;
    let names: Vec<&str> = remotes.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if names.is_empty() {
        return Err("This repository has no remote yet.".into());
    }
    let ordered = names.iter().filter(|n| **n == "origin").chain(names.iter().filter(|n| **n != "origin"));
    for name in ordered {
        let url = git::run(dir, &["remote", "get-url", name]).unwrap_or_default();
        if let Some(r) = detect(name, &url) {
            return Ok(r);
        }
    }
    Err("The remote is not on GitHub, GitLab, Bitbucket or Azure DevOps.".into())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn auth_header(forge: Forge, cred: &Option<(String, String)>) -> Option<String> {
    let (user, secret) = cred.as_ref()?;
    Some(match forge {
        Forge::GitHub | Forge::GitLab => format!("Bearer {secret}"),
        Forge::Bitbucket => format!("Basic {}", base64(&format!("{user}:{secret}"))),
        Forge::Azure => format!("Basic {}", base64(&format!(":{secret}"))),
    })
}

fn request(method: &str, url: &str, repo: &RemoteRepo, cred: &Option<(String, String)>, body: Option<Value>) -> Result<Value, String> {
    let agent = agent();
    let mut resp = match (method, body) {
        ("POST", Some(b)) => {
            let mut req = agent.post(url).header("User-Agent", "Vegit").header("Accept", "application/json");
            if let Some(h) = auth_header(repo.forge, cred) {
                req = req.header("Authorization", &h);
            }
            req.send_json(b)
        }
        _ => {
            let mut req = agent.get(url).header("User-Agent", "Vegit").header("Accept", "application/json");
            if let Some(h) = auth_header(repo.forge, cred) {
                req = req.header("Authorization", &h);
            }
            req.call()
        }
    }
    .map_err(|e| format!("Could not reach {}: {e}", repo.forge.name()))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if status == 401 || status == 403 || (status == 404 && cred.is_none()) {
        return Err(format!("{} refused access ({status}). Sign in to see this repository's {}s.", repo.forge.name(), repo.forge.noun()));
    }
    if !(200..300).contains(&status) {
        let detail: String = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("message").or_else(|| v.get("error")).map(|m| m.to_string()))
            .unwrap_or_else(|| text.chars().take(200).collect());
        return Err(format!("{} answered {status}: {detail}", repo.forge.name()));
    }
    serde_json::from_str(&text).map_err(|e| format!("Unexpected answer from {}: {e}", repo.forge.name()))
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn s(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        cur = &cur[*p];
    }
    match cur {
        Value::String(x) => x.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Open pull requests of the repository behind `dir`.
pub fn list(dir: &Path, interactive: bool) -> Result<(RemoteRepo, Vec<PullRequest>), String> {
    let repo = repo_of(dir)?;
    let cred = access::credential_fill(&repo.host, interactive);
    let prs = match repo.forge {
        Forge::GitHub => {
            let api = if repo.host == "github.com" { "https://api.github.com".to_string() } else { format!("https://{}/api/v3", repo.host) };
            let v = request("GET", &format!("{api}/repos/{}/pulls?state=open&per_page=100", repo.path), &repo, &cred, None)?;
            v.as_array()
                .map(|a| {
                    a.iter()
                        .map(|p| PullRequest {
                            number: p["number"].as_u64().unwrap_or(0),
                            title: s(p, &["title"]),
                            author: s(p, &["user", "login"]),
                            source: s(p, &["head", "ref"]),
                            target: s(p, &["base", "ref"]),
                            url: s(p, &["html_url"]),
                            draft: p["draft"].as_bool().unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        Forge::GitLab => {
            let url = format!("https://{}/api/v4/projects/{}/merge_requests?state=opened&per_page=100", repo.host, enc(&repo.path));
            let v = request("GET", &url, &repo, &cred, None)?;
            v.as_array()
                .map(|a| {
                    a.iter()
                        .map(|p| PullRequest {
                            number: p["iid"].as_u64().unwrap_or(0),
                            title: s(p, &["title"]),
                            author: s(p, &["author", "username"]),
                            source: s(p, &["source_branch"]),
                            target: s(p, &["target_branch"]),
                            url: s(p, &["web_url"]),
                            draft: p["draft"].as_bool().unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        Forge::Bitbucket => {
            let url = format!("https://api.bitbucket.org/2.0/repositories/{}/pullrequests?state=OPEN&pagelen=50", repo.path);
            let v = request("GET", &url, &repo, &cred, None)?;
            v["values"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|p| PullRequest {
                            number: p["id"].as_u64().unwrap_or(0),
                            title: s(p, &["title"]),
                            author: s(p, &["author", "display_name"]),
                            source: s(p, &["source", "branch", "name"]),
                            target: s(p, &["destination", "branch", "name"]),
                            url: s(p, &["links", "html", "href"]),
                            draft: p["draft"].as_bool().unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        Forge::Azure => {
            let (org_project, name) = repo.path.rsplit_once('/').unwrap_or((&repo.path, ""));
            let url = format!("https://dev.azure.com/{org_project}/_apis/git/repositories/{name}/pullrequests?searchCriteria.status=active&api-version=7.0");
            let v = request("GET", &url, &repo, &cred, None)?;
            let web = format!("https://dev.azure.com/{org_project}/_git/{name}/pullrequest/");
            v["value"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|p| {
                            let number = p["pullRequestId"].as_u64().unwrap_or(0);
                            PullRequest {
                                number,
                                title: s(p, &["title"]),
                                author: s(p, &["createdBy", "displayName"]),
                                source: s(p, &["sourceRefName"]).trim_start_matches("refs/heads/").to_string(),
                                target: s(p, &["targetRefName"]).trim_start_matches("refs/heads/").to_string(),
                                url: format!("{web}{number}"),
                                draft: p["isDraft"].as_bool().unwrap_or(false),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
    };
    Ok((repo, prs))
}

/// Web page of a commit on the hosting service.
pub fn commit_url(repo: &RemoteRepo, sha: &str) -> String {
    match repo.forge {
        Forge::GitHub => format!("https://{}/{}/commit/{sha}", repo.host, repo.path),
        Forge::GitLab => format!("https://{}/{}/-/commit/{sha}", repo.host, repo.path),
        Forge::Bitbucket => format!("https://bitbucket.org/{}/commits/{sha}", repo.path),
        Forge::Azure => {
            let (op, name) = repo.path.rsplit_once('/').unwrap_or((&repo.path, ""));
            format!("https://dev.azure.com/{op}/_git/{name}/commit/{sha}")
        }
    }
}

/// Home page of the repository on the hosting service.
pub fn repo_url(repo: &RemoteRepo) -> String {
    match repo.forge {
        Forge::GitHub | Forge::GitLab => format!("https://{}/{}", repo.host, repo.path),
        Forge::Bitbucket => format!("https://bitbucket.org/{}", repo.path),
        Forge::Azure => {
            let (op, name) = repo.path.rsplit_once('/').unwrap_or((&repo.path, ""));
            format!("https://dev.azure.com/{op}/_git/{name}")
        }
    }
}

/// Web page of a branch on the hosting service.
pub fn branch_url(repo: &RemoteRepo, branch: &str) -> String {
    match repo.forge {
        Forge::GitHub => format!("https://{}/{}/tree/{branch}", repo.host, repo.path),
        Forge::GitLab => format!("https://{}/{}/-/tree/{branch}", repo.host, repo.path),
        Forge::Bitbucket => format!("https://bitbucket.org/{}/branch/{branch}", repo.path),
        Forge::Azure => {
            let (op, name) = repo.path.rsplit_once('/').unwrap_or((&repo.path, ""));
            format!("https://dev.azure.com/{op}/_git/{name}?version=GB{branch}")
        }
    }
}

/// Every remote of `dir` that is on a supported service, `origin` first.
pub fn remotes_of(dir: &Path) -> Vec<RemoteRepo> {
    let names = git::run(dir, &["remote"]).unwrap_or_default();
    let mut names: Vec<&str> = names.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    names.sort_by_key(|n| *n != "origin");
    names
        .iter()
        .filter_map(|n| detect(n, &git::run(dir, &["remote", "get-url", n]).unwrap_or_default()))
        .collect()
}

/// Opens a pull request from `from`/`from_branch` into `to`/`to_branch` (which may be a
/// different repository, e.g. your fork into the original); returns its web address.
#[allow(clippy::too_many_arguments)]
pub fn create(from: &RemoteRepo, from_branch: &str, to: &RemoteRepo, to_branch: &str, title: &str, body: &str, draft: bool) -> Result<String, String> {
    let repo = to;
    let cred = access::credential_fill(&repo.host, true);
    if cred.is_none() {
        return Err(format!("Sign in to {} first (Access window).", repo.forge.name()));
    }
    let same = from.path == to.path;
    let v = match repo.forge {
        Forge::GitHub => {
            let api = if repo.host == "github.com" { "https://api.github.com".to_string() } else { format!("https://{}/api/v3", repo.host) };
            // A branch of another repository (a fork) is named "owner:branch".
            let head = if same { from_branch.to_string() } else { format!("{}:{from_branch}", from.path.split('/').next().unwrap_or_default()) };
            let b = json!({ "title": title, "body": body, "head": head, "base": to_branch, "draft": draft });
            let v = request("POST", &format!("{api}/repos/{}/pulls", repo.path), repo, &cred, Some(b))?;
            s(&v, &["html_url"])
        }
        Forge::GitLab => {
            let t = if draft { format!("Draft: {title}") } else { title.to_string() };
            let mut b = json!({ "title": t, "description": body, "source_branch": from_branch, "target_branch": to_branch });
            if !same {
                // Between projects GitLab wants the target project's number.
                let target = request("GET", &format!("https://{}/api/v4/projects/{}", to.host, enc(&to.path)), to, &cred, None)?;
                b["target_project_id"] = target["id"].clone();
            }
            let url = format!("https://{}/api/v4/projects/{}/merge_requests", from.host, enc(&from.path));
            let v = request("POST", &url, from, &cred, Some(b))?;
            s(&v, &["web_url"])
        }
        Forge::Bitbucket => {
            let b = json!({ "title": title, "description": body, "draft": draft,
                "source": { "branch": { "name": from_branch }, "repository": { "full_name": from.path } },
                "destination": { "branch": { "name": to_branch } } });
            let url = format!("https://api.bitbucket.org/2.0/repositories/{}/pullrequests", repo.path);
            let v = request("POST", &url, repo, &cred, Some(b))?;
            s(&v, &["links", "html", "href"])
        }
        Forge::Azure => {
            if !same {
                return Err("On Azure DevOps, pick the same repository on both sides.".into());
            }
            let (org_project, name) = repo.path.rsplit_once('/').unwrap_or((&repo.path, ""));
            let b = json!({ "title": title, "description": body, "isDraft": draft,
                "sourceRefName": format!("refs/heads/{from_branch}"), "targetRefName": format!("refs/heads/{to_branch}") });
            let url = format!("https://dev.azure.com/{org_project}/_apis/git/repositories/{name}/pullrequests?api-version=7.0");
            let v = request("POST", &url, repo, &cred, Some(b))?;
            let id = v["pullRequestId"].as_u64().unwrap_or(0);
            format!("https://dev.azure.com/{org_project}/_git/{name}/pullrequest/{id}")
        }
    };
    Ok(v)
}

/// Git commands that bring a pull request's commits into a local branch `pr/<number>`.
/// GitHub and GitLab publish them under a special ref, which also works for forks.
pub fn checkout_commands(repo: &RemoteRepo, pr: &PullRequest) -> Vec<Vec<String>> {
    let local = format!("pr/{}", pr.number);
    let fetch_ref = match repo.forge {
        Forge::GitHub => Some(format!("pull/{}/head:{local}", pr.number)),
        Forge::GitLab => Some(format!("merge-requests/{}/head:{local}", pr.number)),
        Forge::Bitbucket | Forge::Azure => None,
    };
    match fetch_ref {
        Some(r) => vec![
            vec!["fetch".into(), repo.remote.clone(), r],
            vec!["switch".into(), local],
        ],
        None => vec![
            vec!["fetch".into(), repo.remote.clone(), pr.source.clone()],
            vec!["switch".into(), "-c".into(), pr.source.clone(), "--track".into(), format!("{}/{}", repo.remote, pr.source)],
        ],
    }
}

fn base64(input: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = input.as_bytes();
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_of_each_service_are_recognised() {
        let gh = detect("origin", "https://github.com/acme/rocket.git").unwrap();
        assert_eq!((gh.forge, gh.path.as_str()), (Forge::GitHub, "acme/rocket"));
        let gh_ssh = detect("origin", "git@github.com:acme/rocket.git").unwrap();
        assert_eq!(gh_ssh.path, "acme/rocket");
        let gl = detect("origin", "https://gitlab.com/group/sub/app.git").unwrap();
        assert_eq!((gl.forge, gl.path.as_str()), (Forge::GitLab, "group/sub/app"));
        let bb = detect("origin", "git@bitbucket.org:team/site.git").unwrap();
        assert_eq!((bb.forge, bb.path.as_str()), (Forge::Bitbucket, "team/site"));
        let az = detect("origin", "https://org@dev.azure.com/org/Project/_git/Repo").unwrap();
        assert_eq!((az.forge, az.path.as_str()), (Forge::Azure, "org/Project/Repo"));
        let az_ssh = detect("origin", "git@ssh.dev.azure.com:v3/org/Project/Repo").unwrap();
        assert_eq!(az_ssh.path, "org/Project/Repo");
        assert!(detect("origin", "https://example.com/x/y.git").is_none());
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64("user:secret"), "dXNlcjpzZWNyZXQ=");
        assert_eq!(base64(":pat"), "OnBhdA==");
    }
}

#[cfg(test)]
mod live_tests {
    /// Talks to the real GitHub API; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn lists_open_pull_requests_of_a_public_github_repo() {
        let dir = std::env::temp_dir().join(format!("vegit-pr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::git::run(&dir, &["init", "-q"]).unwrap();
        crate::git::run(&dir, &["remote", "add", "origin", "https://github.com/rust-lang/rust.git"]).unwrap();
        let (repo, prs) = super::list(&dir, false).expect("pull requests");
        assert_eq!(repo.path, "rust-lang/rust");
        assert!(!prs.is_empty());
        assert!(prs.iter().all(|p| p.number > 0 && !p.url.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
