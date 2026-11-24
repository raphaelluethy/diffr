//! Finds a token for github.com.

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use diffr_agent::{proc, shellenv};

/// The environment variables a token is read from, in order.
pub const TOKEN_ENV: [&str; 3] = ["DIFFR_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];

/// A token for github.com from the environment ([`TOKEN_ENV`]), else from
/// the GitHub CLI's login (`gh auth token`), "" when there is neither. It
/// may run `gh`: call it off the UI thread.
pub fn find_token() -> String {
    for k in TOKEN_ENV {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    let Some(gh) = shellenv::look_path("gh") else {
        return String::new();
    };
    let mut cmd = Command::new(gh);
    cmd.args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null());
    shellenv::apply(&mut cmd);
    proc::hide_console(&mut cmd);
    match proc::output(&mut cmd, Duration::from_secs(5)) {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// The token for GitHub: the settings' when there is one, else the
/// environment's or the GitHub CLI's, found again every few minutes, as the
/// user may sign in meanwhile. Share one across threads.
#[derive(Debug, Default)]
pub struct TokenCache {
    found: Mutex<Option<(String, Instant)>>,
}

impl TokenCache {
    /// How long a token found stays before it is looked for again.
    pub const TTL: Duration = Duration::from_secs(5 * 60);

    pub fn new() -> Self {
        Self::default()
    }

    /// The token: `explicit` (the settings') when not blank, else the one
    /// found, looked for again once [`Self::TTL`] old. It may run `gh`:
    /// call it off the UI thread.
    pub fn get(&self, explicit: &str) -> String {
        self.get_with(explicit, find_token)
    }

    fn get_with(&self, explicit: &str, find: impl FnOnce() -> String) -> String {
        let explicit = explicit.trim();
        if !explicit.is_empty() {
            return explicit.to_string();
        }
        let mut found = self.found.lock().unwrap_or_else(|e| e.into_inner());
        match &*found {
            Some((t, at)) if at.elapsed() <= Self::TTL => t.clone(),
            _ => {
                let t = find();
                *found = Some((t.clone(), Instant::now()));
                t
            }
        }
    }

    /// The token as last found, without looking for it: for the UI thread.
    pub fn known(&self, explicit: &str) -> String {
        let explicit = explicit.trim();
        if !explicit.is_empty() {
            return explicit.to_string();
        }
        let found = self.found.lock().unwrap_or_else(|e| e.into_inner());
        found.as_ref().map(|(t, _)| t.clone()).unwrap_or_default()
    }

    /// Forgets the token found, as after the user signed in.
    pub fn reset(&self) {
        *self.found.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache() {
        let c = TokenCache::new();
        assert_eq!(c.known(""), "");
        assert_eq!(c.get_with(" mine ", || unreachable!()), "mine");
        assert_eq!(c.get_with("", || "found".into()), "found");
        assert_eq!(c.get_with("", || unreachable!()), "found");
        assert_eq!(c.known(""), "found");
        c.reset();
        assert_eq!(c.get_with("", || "again".into()), "again");
    }
}
