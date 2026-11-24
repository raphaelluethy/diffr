//! Reads the pull requests of a repository on GitHub and reviews them,
//! through GitHub's REST and GraphQL APIs, with the pure logic of
//! reviewing a pull request ([`pull`]) and of grouping its files
//! ([`groups`]).
//!
//! The client blocks (it uses `ureq`): call it on a background thread.

pub mod groups;
pub mod pull;
mod pulls;
mod reviews;
mod token;

use std::fmt;
use std::io::Read;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

pub use pulls::{Branch, Label, PullCommit, PullCommitInfo, PullRequest, Summary};
pub use reviews::{
    Comment, NewComment, PendingReview, ReviewSummary, Reviews, Side, Thread, VERDICT_APPROVE,
    VERDICT_COMMENT, VERDICT_REQUEST_CHANGES,
};
pub use token::{TokenCache, find_token};

/// A repository on GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Repo {
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// Whether the repository is unknown.
    pub fn is_zero(&self) -> bool {
        self.owner.is_empty() || self.name.is_empty()
    }

    /// The repository's page.
    pub fn url(&self) -> String {
        format!("https://github.com/{self}")
    }
}

impl fmt::Display for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

const W: &str = "[A-Za-z0-9_.-]";

static REMOTE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:https?://(?:[^@/]+@)?|ssh://(?:[^@/]+@)?|git://|(?:[^@/]+@))github\.com[:/]({W}+)/({W}+?)(?:\.git)?/?$"
    ))
    .unwrap()
});
static PULL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:https?://)?(?:www\.)?github\.com/({W}+)/({W}+)/pull/(\d+)(?:[/?#].*)?$"
    ))
    .unwrap()
});
static PULL_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^(?:({W}+)/({W}+))?#(\d+)$")).unwrap());
static NEXT_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<([^>]+)>;\s*rel="next""#).unwrap());

/// Reads the repository of a GitHub remote's URL, as
/// `git@github.com:owner/repo.git` or `https://github.com/owner/repo`.
pub fn parse_remote(url: &str) -> Option<Repo> {
    let m = REMOTE_URL.captures(url.trim())?;
    Some(Repo::new(&m[1], &m[2]))
}

/// Reads a pull request named as `owner/repo#123`, `#123` or its URL. The
/// repository is zero for `#123`.
pub fn parse_pull(s: &str) -> Option<(Repo, u64)> {
    let s = s.trim();
    let m = PULL_URL.captures(s).or_else(|| PULL_REF.captures(s))?;
    let n: u64 = m[3].parse().ok()?;
    let repo = Repo::new(
        m.get(1).map_or("", |g| g.as_str()),
        m.get(2).map_or("", |g| g.as_str()),
    );
    (n > 0).then_some((repo, n))
}

/// Where the client sends its requests by default.
pub const API: &str = "https://api.github.com";

/// A request GitHub refused, or one that did not reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// GitHub answered with an error status.
    Api { status: u16, message: String },
    /// The request did not get an answer: no network, a timeout.
    Transport(String),
    /// The answer was not what was expected.
    Decode(String),
}

impl Error {
    fn api(status: u16, message: impl Into<String>) -> Self {
        Error::Api {
            status,
            message: message.into(),
        }
    }

    /// Whether GitHub refused a write the token may not make.
    pub fn is_forbidden(&self) -> bool {
        matches!(
            self,
            Error::Api {
                status: 401 | 403,
                ..
            }
        )
    }

    /// The HTTP status GitHub answered with, if any.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api { status: 401, message } => {
                write!(f, "GitHub refused the token: {message}")
            }
            Error::Api { status: 403, message }
                if message.to_lowercase().contains("rate limit") =>
            {
                f.write_str("GitHub's rate limit was reached. Sign in with the GitHub CLI (gh auth login) to raise it.")
            }
            Error::Api { status: 404, .. } => f.write_str(
                "Not found on GitHub. A private repository needs a token: sign in with gh auth login.",
            ),
            Error::Api { status, message } => write!(f, "GitHub: {message} ({status})"),
            Error::Transport(m) => write!(f, "GitHub: {m}"),
            Error::Decode(m) => write!(f, "GitHub: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Whether `err` is GitHub refusing a write the token may not make.
pub fn forbidden(err: &Error) -> bool {
    err.is_forbidden()
}

/// Reads null as the default value, as Go's JSON does.
pub(crate) fn nullable<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// Talks to GitHub's API, with a token or without one: without, only
/// public repositories can be read, and nothing written. It is cheap to
/// clone, and `Send + Sync`.
#[derive(Clone)]
pub struct Client {
    /// The token, "" for none.
    pub token: String,
    /// The API's base URL, [`API`] unless tests set another.
    pub api: String,
    http: ureq::Agent,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("token", &if self.token.is_empty() { "" } else { "…" })
            .field("api", &self.api)
            .finish()
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new("")
    }
}

/// An answer of the REST API: its `Link` header and its body.
struct Answer {
    link: Option<String>,
    body: Vec<u8>,
}

impl Client {
    /// A client of github.com with a token, "" for none.
    pub fn new(token: impl Into<String>) -> Self {
        Client {
            token: token.into(),
            api: API.to_string(),
            http: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(30))
                .build(),
        }
    }

    /// The client with another API base URL, as tests' fake servers.
    pub fn with_api(mut self, api: impl Into<String>) -> Self {
        self.api = api.into().trim_end_matches('/').to_string();
        self
    }

    /// Whether the client has a token.
    pub fn has_token(&self) -> bool {
        !self.token.is_empty()
    }

    /// Sends a request to the REST API.
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Answer> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{path}", self.api)
        };
        let mut req = self
            .http
            .request(method, &url)
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", "diffr");
        if !self.token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.token));
        }
        let res = match body {
            Some(b) => req
                .set("Content-Type", "application/json")
                .send_string(&b.to_string()),
            None => req.call(),
        };
        let (status, res) = match res {
            Ok(r) => (r.status(), r),
            Err(ureq::Error::Status(code, r)) => (code, r),
            Err(ureq::Error::Transport(t)) => return Err(Error::Transport(t.to_string())),
        };
        let link = res.header("Link").map(str::to_string);
        let mut data = Vec::new();
        res.into_reader()
            .read_to_end(&mut data)
            .map_err(|e| Error::Transport(e.to_string()))?;
        if status >= 300 {
            return Err(api_error(status, &data));
        }
        Ok(Answer { link, body: data })
    }

    /// Sends a request and decodes its answer.
    fn json<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let a = self.send(method, path, body)?;
        decode(&a.body)
    }

    /// Reads every page of a list.
    fn paginate<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut url = Some(format!("{path}{sep}per_page=100"));
        let mut all = Vec::new();
        while let Some(u) = url.take() {
            let a = self.send("GET", &u, None)?;
            let page: Vec<T> = if a.body.is_empty() {
                Vec::new()
            } else {
                decode(&a.body)?
            };
            all.extend(page);
            url = a
                .link
                .as_deref()
                .and_then(|l| NEXT_LINK.captures(l))
                .map(|m| m[1].to_string());
        }
        Ok(all)
    }

    /// Runs a query of GitHub's GraphQL API, which needs a token, and
    /// returns its `data`.
    fn graphql(&self, query: &str, vars: Value) -> Result<Value> {
        if self.token.is_empty() {
            return Err(Error::api(401, "GitHub's GraphQL API needs a token"));
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Res {
            data: Value,
            #[serde(deserialize_with = "nullable")]
            errors: Vec<GraphQLError>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct GraphQLError {
            #[serde(deserialize_with = "nullable")]
            message: String,
        }
        let res: Res = self.json(
            "POST",
            "/graphql",
            Some(&json!({"query": query, "variables": vars})),
        )?;
        if !res.errors.is_empty() {
            let msgs: Vec<String> = res.errors.into_iter().map(|e| e.message).collect();
            return Err(Error::api(422, msgs.join("; ")));
        }
        Ok(res.data)
    }

    /// The account of the token.
    pub fn viewer(&self) -> Result<User> {
        self.json("GET", "/user", None)
    }
}

fn decode<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    let data = if data.is_empty() {
        b"null".as_slice()
    } else {
        data
    };
    serde_json::from_slice(data).map_err(|e| Error::Decode(e.to_string()))
}

/// Reads GitHub's message, and the details of a validation error.
fn api_error(status: u16, data: &[u8]) -> Error {
    let body: Value = serde_json::from_slice(data).unwrap_or(Value::Null);
    let mut msg = body
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if msg.is_empty() {
        msg = status_text(status).to_string();
    }
    let details: Vec<&str> = body
        .get("errors")
        .and_then(Value::as_array)
        .map(|errs| {
            errs.iter()
                .filter_map(|e| match e {
                    Value::String(s) => Some(s.as_str()),
                    Value::Object(o) => o
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if !details.is_empty() {
        msg = format!("{msg}: {}", details.join("; "));
    }
    Error::api(status, msg)
}

fn status_text(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        410 => "Gone",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// An account on GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct User {
    #[serde(default, deserialize_with = "nullable")]
    pub login: String,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes() {
        for (url, want) in [
            ("git@github.com:acme/app.git", "acme/app"),
            ("https://github.com/acme/app", "acme/app"),
            ("https://github.com/acme/app.git", "acme/app"),
            ("https://token@github.com/a-b/c.d.git", "a-b/c.d"),
            ("ssh://git@github.com/acme/app.git", "acme/app"),
            ("git@gitlab.com:acme/app.git", ""),
            ("/srv/repos/app.git", ""),
        ] {
            let got = parse_remote(url).map(|r| r.to_string()).unwrap_or_default();
            assert_eq!(got, want, "{url}");
        }
    }

    #[test]
    fn pulls() {
        for (s, repo, n) in [
            (
                "https://github.com/acme/app/pull/12",
                "acme/app",
                12,
            ),
            (
                "https://github.com/acme/app/pull/12/files",
                "acme/app",
                12,
            ),
            ("github.com/acme/app/pull/3", "acme/app", 3),
            ("acme/app#7", "acme/app", 7),
            ("#42", "/", 42),
        ] {
            let (r, got) = parse_pull(s).unwrap_or_else(|| panic!("{s} did not parse"));
            assert_eq!((r.to_string().as_str(), got), (repo, n), "{s}");
        }
        for bad in [
            "main",
            "HEAD~1",
            "12",
            "#0",
            "https://github.com/a/b/issues/1",
        ] {
            assert!(parse_pull(bad).is_none(), "{bad} parsed");
        }
    }

    #[test]
    fn errors() {
        let e = api_error(
            422,
            br#"{"message":"Validation Failed","errors":[{"message":"line must be part of the diff"},"x"]}"#,
        );
        assert_eq!(
            e.to_string(),
            "GitHub: Validation Failed: line must be part of the diff; x (422)"
        );
        assert!(api_error(403, b"{}").is_forbidden());
        assert_eq!(api_error(404, b"").status(), Some(404));
        assert!(
            api_error(403, br#"{"message":"API rate limit exceeded"}"#)
                .to_string()
                .contains("rate limit was reached")
        );
    }

    #[test]
    fn client_is_send() {
        fn check<T: Send + Sync>() {}
        check::<Client>();
        check::<Reviews>();
        check::<Error>();
    }
}
