//! Access to private repositories.
//!
//! Git itself has no paywall: a private repository only needs credentials. HTTPS uses a
//! token saved through git's credential helper (Git Credential Manager on Windows, which
//! keeps it in the system vault); SSH uses a key pair in `~/.ssh`. The app never stores
//! secrets itself.

use std::path::PathBuf;
use std::process::Command;

use crate::app::dirs_home;
use crate::git;

/// Errors that usually mean "you are not signed in" rather than a real failure.
pub fn looks_like_auth_error(err: &str) -> bool {
    let e = err.to_lowercase();
    [
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "permission denied (publickey",
        "repository not found",
        "host key verification failed",
        "returned error: 401",
        "returned error: 403",
        "access denied",
    ]
    .iter()
    .any(|p| e.contains(p))
}

/// Input for `git credential approve`.
pub fn credential_input(host: &str, user: &str, token: &str) -> String {
    let host = host.trim().trim_start_matches("https://").trim_end_matches('/');
    format!("protocol=https\nhost={host}\nusername={}\npassword={}\n\n", user.trim(), token.trim())
}

/// The user name and token git would use for `https://host`, from the credential helper.
/// With `interactive` false the helper must not open a sign-in window (automatic
/// refreshes); with true it may (the person pressed "Sign in").
pub fn credential_fill(host: &str, interactive: bool) -> Option<(String, String)> {
    use std::io::Write;
    use std::process::Stdio;
    let mut cmd = Command::new("git");
    cmd.args(["credential", "fill"])
        .current_dir(dirs_home())
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !interactive {
        cmd.env("GCM_INTERACTIVE", "never");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().ok()?;
    child.stdin.take()?.write_all(format!("protocol=https\nhost={host}\n\n").as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(str::to_string);
    let password = field("password=").filter(|p| !p.is_empty())?;
    Some((field("username=").unwrap_or_default(), password))
}

/// The configured credential helper, if any (e.g. `manager`).
pub fn credential_helper() -> Option<String> {
    git::run(&dirs_home(), &["config", "--get", "credential.helper"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn ssh_dir() -> PathBuf {
    dirs_home().join(".ssh")
}

/// The first public key found in `~/.ssh`, as (file name, content).
pub fn ssh_public_key() -> Option<(String, String)> {
    ["id_ed25519.pub", "id_ecdsa.pub", "id_rsa.pub"].iter().find_map(|name| {
        let text = std::fs::read_to_string(ssh_dir().join(name)).ok()?;
        Some((name.to_string(), text.trim().to_string()))
    })
}

/// Creates `~/.ssh/id_ed25519` without a passphrase. Refuses to overwrite an existing key.
pub fn create_ssh_key() -> Result<String, String> {
    let dir = ssh_dir();
    let key = dir.join("id_ed25519");
    if key.exists() {
        return Err("An SSH key already exists; use the one shown.".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let comment = git::run(&dirs_home(), &["config", "--get", "user.email"]).unwrap_or_default().trim().to_string();
    let comment = if comment.is_empty() { "grove".to_string() } else { comment };
    let key_s = key.display().to_string();
    let mut last_err = String::from("ssh-keygen was not found");
    for exe in keygen_candidates() {
        let mut cmd = Command::new(&exe);
        cmd.args(["-q", "-t", "ed25519", "-N", "", "-C", &comment, "-f", &key_s]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        match cmd.output() {
            Ok(o) if o.status.success() => return Ok(format!("{key_s}.pub")),
            Ok(o) => last_err = String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(_) => continue,
        }
    }
    Err(format!("Could not create the key: {last_err}"))
}

fn keygen_candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("ssh-keygen")];
    #[cfg(windows)]
    {
        v.push(PathBuf::from(r"C:\Program Files\Git\usr\bin\ssh-keygen.exe"));
        v.push(PathBuf::from(r"C:\Windows\System32\OpenSSH\ssh-keygen.exe"));
    }
    v
}

/// Pages where a token or an SSH key is created, for the well-known hosts.
pub fn host_pages(host: &str) -> Option<(&'static str, &'static str)> {
    let h = host.trim().to_lowercase();
    if h.contains("github") {
        Some(("https://github.com/settings/tokens", "https://github.com/settings/keys"))
    } else if h.contains("gitlab") {
        Some(("https://gitlab.com/-/user_settings/personal_access_tokens", "https://gitlab.com/-/user_settings/ssh_keys"))
    } else if h.contains("bitbucket") {
        Some(("https://bitbucket.org/account/settings/app-passwords/", "https://bitbucket.org/account/settings/ssh-keys/"))
    } else {
        None
    }
}
