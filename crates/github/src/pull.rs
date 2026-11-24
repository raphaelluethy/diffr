//! The pure logic of reviewing a pull request in a local repository: which
//! remote is the pull request's, how a pull request is named, when to ask
//! GitHub again for new commits, and what a description reads as: the
//! parts of reviewing a pull request that are not UI.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

use crate::{PullRequest, Repo, Reviews, parse_pull};

/// A remote of a repository on GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct GitHubRemote {
    pub repo: Repo,
    /// The remote's name, as `origin`.
    pub remote: String,
}

/// The remote of `want` on GitHub, or, for a zero `want`, the remote whose
/// pull requests the repository's are: `upstream`, as forks name the
/// repository they come from, else `origin`, else the first. `remotes` are
/// the repository's, as `(name, url)`.
pub fn find_github_remote<'a>(
    remotes: impl IntoIterator<Item = (&'a str, &'a str)>,
    want: &Repo,
) -> Option<GitHubRemote> {
    let mut found = Vec::new();
    for (name, url) in remotes {
        let Some(gr) = crate::parse_remote(url) else {
            continue;
        };
        if !want.is_zero() {
            if gr.to_string().eq_ignore_ascii_case(&want.to_string()) {
                return Some(GitHubRemote {
                    repo: gr,
                    remote: name.to_string(),
                });
            }
            continue;
        }
        found.push(GitHubRemote {
            repo: gr,
            remote: name.to_string(),
        });
    }
    for name in ["upstream", "origin"] {
        if let Some(f) = found.iter().find(|f| f.remote == name) {
            return Some(f.clone());
        }
    }
    found.into_iter().next()
}

/// A pull request's ref, as sources name them: `owner/repo#123`.
pub fn pull_ref(repo: &Repo, number: u64) -> String {
    format!("{repo}#{number}")
}

/// Names a pull request of a repository fully, as `owner/repo#123`: one
/// named `#123` or `123` alone is of the repository's own remote on
/// GitHub. The repository (at `root`, with `remotes` as `(name, url)`) must
/// have a remote of the pull request's.
pub fn resolve_pull<'a>(
    root: &str,
    remotes: impl IntoIterator<Item = (&'a str, &'a str)>,
    reference: &str,
) -> Result<String, String> {
    let parsed = parse_pull(reference).or_else(|| {
        reference
            .strip_prefix('#')
            .unwrap_or(reference)
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .map(|n| (Repo::default(), n))
    });
    let Some((gh, n)) = parsed else {
        return Err(format!(
            "{reference:?} names no pull request: use 123, owner/repo#123 or its URL"
        ));
    };
    match find_github_remote(remotes, &gh) {
        Some(r) => Ok(pull_ref(&r.repo, n)),
        None if gh.is_zero() => Err(format!("{root} has no remote on GitHub")),
        None => Err(format!(
            "{root} has no remote of {gh}: open a clone of it to review its pull requests"
        )),
    }
}

/// The remote a pull request's commits are fetched from: the repository's
/// remote of its GitHub repository (`own` first, the remote found for the
/// window), else its URL on GitHub.
pub fn pull_remote<'a>(
    own: Option<&GitHubRemote>,
    remotes: impl IntoIterator<Item = (&'a str, &'a str)>,
    repo: &Repo,
) -> String {
    if let Some(g) = own.filter(|g| !g.repo.is_zero())
        && g.repo.to_string().eq_ignore_ascii_case(&repo.to_string())
    {
        return g.remote.clone();
    }
    match find_github_remote(remotes, repo) {
        Some(r) => r.remote,
        None => format!("{}.git", repo.url()),
    }
}

/// The git refspecs to fetch for a pull request: its head as GitHub keeps
/// it when the repository lacks the head commit, and the base branch when
/// it lacks the base commit.
pub fn fetch_refspecs(pull: &PullRequest, has_head: bool, has_base: bool) -> Vec<String> {
    let mut out = Vec::new();
    if !has_head {
        out.push(format!("refs/pull/{}/head", pull.number));
    }
    if !has_base {
        out.push(format!("refs/heads/{}", pull.base.reference));
    }
    out
}

/// What the files viewed of a source are remembered by: the repository's
/// for its work tree and branches, a pull request's own (`root#owner/repo#1`),
/// none for commits, which do not change.
pub fn viewed_key(root: &str, kind: diffr_agent::cache::SourceKind, reference: &str) -> String {
    use diffr_agent::cache::SourceKind;
    match kind {
        SourceKind::Commit => String::new(),
        SourceKind::Pull => format!("{root}#{reference}"),
        SourceKind::WorkingTree | SourceKind::Branch => root.to_string(),
    }
}

/// How often GitHub is asked whether a pull request has new commits:
/// without a token, GitHub allows 60 requests an hour.
pub fn check_interval(has_token: bool) -> Duration {
    if has_token {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(5 * 60)
    }
}

/// How old the review threads may be before a check reads them again.
pub const REVIEWS_REFRESH: Duration = Duration::from_secs(30);

/// Whether the pull request read again has commits the review has not:
/// its head moved from `reviewed_head`.
pub fn has_new_commits(reviewed_head: &str, latest: &PullRequest) -> bool {
    !latest.head.sha.is_empty() && latest.head.sha != reviewed_head
}

/// Takes what may change of a pull request while it is reviewed: its
/// title, description and state. The head is left as reviewed.
pub fn refresh_meta(meta: &mut PullRequest, latest: &PullRequest) {
    meta.title = latest.title.clone();
    meta.body = latest.body.clone();
    meta.state = latest.state.clone();
    meta.draft = latest.draft;
    meta.merged = latest.merged;
    meta.merged_at = latest.merged_at;
}

/// Why nothing can be written without a token.
pub const READ_ONLY_NO_TOKEN: &str = "Sign in with the GitHub CLI (gh auth login), or set githubToken in the config file, to comment and review.";

/// Why nothing can be written once GitHub refused a write.
pub fn read_only_forbidden(repo: &Repo) -> String {
    format!("GitHub refused to let this token write to {repo}.")
}

/// Whether a pull request can be commented on: once GitHub said whose the
/// token is (`viewer`), and refused none of its writes (`read_only` is "").
pub fn can_write(viewer: &str, read_only: &str, has_meta: bool) -> bool {
    read_only.is_empty() && !viewer.is_empty() && has_meta
}

/// The pending review of reviews read, for the writes that add to it.
pub fn pending_of(reviews: Option<&Reviews>) -> Option<&crate::PendingReview> {
    reviews.and_then(|r| r.pending.as_ref())
}

/// A comment drafted on lines `start..=line` (in either order), as GitHub
/// takes it: one line when `start` is 0 or `line`.
pub fn new_comment(
    body: &str,
    commit_id: &str,
    path: &str,
    old_side: bool,
    start: u32,
    line: u32,
) -> crate::NewComment {
    let mut nc = crate::NewComment {
        body: body.trim().to_string(),
        commit_id: commit_id.to_string(),
        path: path.to_string(),
        side: if old_side {
            crate::Side::Left
        } else {
            crate::Side::Right
        },
        line,
        start_line: 0,
    };
    if start != 0 && start != line {
        nc.start_line = start.min(line);
        nc.line = start.max(line);
    }
    nc
}

static HTML_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());

/// The Markdown of a description as text: without the HTML comments of
/// templates, nor the blank lines they leave.
pub fn plain_body(s: &str) -> String {
    let s = s.replace("\r\n", "\n");
    let mut s = HTML_COMMENT.replace_all(&s, "").into_owned();
    while s.contains("\n\n\n") {
        s = s.replace("\n\n\n", "\n\n");
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use diffr_agent::cache::SourceKind;

    const REMOTES: [(&str, &str); 3] = [
        ("fork", "git@github.com:bo/app.git"),
        ("origin", "https://github.com/acme/app"),
        ("other", "/srv/app.git"),
    ];

    #[test]
    fn remotes() {
        let r = find_github_remote(REMOTES, &Repo::default()).unwrap();
        assert_eq!(r.remote, "origin");
        let r = find_github_remote(REMOTES, &Repo::new("BO", "app")).unwrap();
        assert_eq!(r.remote, "fork");
        assert!(find_github_remote(REMOTES, &Repo::new("x", "y")).is_none());
        let up = [
            ("origin", "git@github.com:bo/app.git"),
            ("upstream", "git@github.com:acme/app.git"),
        ];
        assert_eq!(
            find_github_remote(up, &Repo::default()).unwrap().repo.owner,
            "acme"
        );

        assert_eq!(resolve_pull("/r", REMOTES, "#3").unwrap(), "acme/app#3");
        assert_eq!(resolve_pull("/r", REMOTES, "12").unwrap(), "acme/app#12");
        assert_eq!(
            resolve_pull("/r", REMOTES, "https://github.com/bo/app/pull/4").unwrap(),
            "bo/app#4"
        );
        assert!(
            resolve_pull("/r", REMOTES, "main")
                .unwrap_err()
                .contains("names no pull request")
        );
        assert!(
            resolve_pull("/r", REMOTES, "x/y#1")
                .unwrap_err()
                .contains("has no remote of x/y")
        );
        assert!(
            resolve_pull("/r", [], "#1")
                .unwrap_err()
                .contains("has no remote on GitHub")
        );

        let own = GitHubRemote {
            repo: Repo::new("acme", "app"),
            remote: "origin".into(),
        };
        assert_eq!(
            pull_remote(Some(&own), REMOTES, &Repo::new("acme", "app")),
            "origin"
        );
        assert_eq!(
            pull_remote(Some(&own), REMOTES, &Repo::new("bo", "app")),
            "fork"
        );
        assert_eq!(
            pull_remote(None, REMOTES, &Repo::new("cy", "app")),
            "https://github.com/cy/app.git"
        );
    }

    #[test]
    fn keys_and_checks() {
        assert_eq!(
            viewed_key("/r", SourceKind::Pull, "acme/app#1"),
            "/r#acme/app#1"
        );
        assert_eq!(viewed_key("/r", SourceKind::Commit, "abc"), "");
        assert_eq!(viewed_key("/r", SourceKind::WorkingTree, ""), "/r");
        let mut p = PullRequest {
            number: 7,
            ..Default::default()
        };
        p.head.sha = "new".into();
        p.base.reference = "main".into();
        assert!(has_new_commits("old", &p));
        assert!(!has_new_commits("new", &p));
        assert_eq!(
            fetch_refspecs(&p, false, false),
            ["refs/pull/7/head", "refs/heads/main"]
        );
        assert!(fetch_refspecs(&p, true, true).is_empty());
        assert!(check_interval(false) > check_interval(true));
        assert!(can_write("ada", "", true));
        assert!(!can_write("", "", true));
        assert!(!can_write("ada", READ_ONLY_NO_TOKEN, true));
    }

    #[test]
    fn comments() {
        let nc = new_comment(" Hm ", "h", "a.go", true, 5, 3);
        assert_eq!((nc.start_line, nc.line, nc.side), (3, 5, crate::Side::Left));
        assert_eq!(nc.body, "Hm");
        let nc = new_comment("x", "h", "a.go", false, 4, 4);
        assert_eq!((nc.start_line, nc.line), (0, 4));
    }

    #[test]
    fn bodies() {
        assert_eq!(
            plain_body("<!-- template\nline -->\r\nMakes it shorter.\r\n\r\n\r\n\r\nDone.\n"),
            "Makes it shorter.\n\nDone."
        );
    }
}
