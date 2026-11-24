//! Pull requests: one, its commits, and the open ones of a repository.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::{Client, Error, Repo, Result, User, nullable};

/// A side of a pull request: the branch, and its commit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    #[serde(default, rename = "ref", deserialize_with = "nullable")]
    pub reference: String,
    #[serde(default, deserialize_with = "nullable")]
    pub sha: String,
    /// `owner:branch`.
    #[serde(default, deserialize_with = "nullable")]
    pub label: String,
}

/// A pull request, as its page shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PullRequest {
    #[serde(default, deserialize_with = "nullable")]
    pub number: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub title: String,
    #[serde(default, deserialize_with = "nullable")]
    pub body: String,
    /// `open` or `closed`.
    #[serde(default, deserialize_with = "nullable")]
    pub state: String,
    #[serde(default, deserialize_with = "nullable")]
    pub draft: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub merged: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub user: User,
    #[serde(default, deserialize_with = "nullable")]
    pub html_url: String,
    #[serde(default, deserialize_with = "nullable")]
    pub base: Branch,
    #[serde(default, deserialize_with = "nullable")]
    pub head: Branch,
    #[serde(default, deserialize_with = "nullable")]
    pub additions: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub deletions: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub changed_files: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub commits: u64,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub updated_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub merged_at: Option<OffsetDateTime>,
}

impl PullRequest {
    /// The pull request's state as its badge shows it: `Open`, `Draft`,
    /// `Merged` or `Closed`.
    pub fn status(&self) -> &'static str {
        if self.merged || self.merged_at.is_some() {
            "Merged"
        } else if self.state == "closed" {
            "Closed"
        } else if self.draft {
            "Draft"
        } else {
            "Open"
        }
    }
}

/// A commit of a pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullCommit {
    #[serde(default, deserialize_with = "nullable")]
    pub sha: String,
    #[serde(default, deserialize_with = "nullable")]
    pub commit: PullCommitInfo,
}

/// What a commit of a pull request says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullCommitInfo {
    #[serde(default, deserialize_with = "nullable")]
    pub message: String,
}

/// A pull request as a list shows it. Without a token, the review
/// decision, the checks and the counts are unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub draft: bool,
    pub author: User,
    pub head_ref: String,
    pub created_at: Option<OffsetDateTime>,
    pub updated_at: Option<OffsetDateTime>,
    pub comments: u64,
    /// `approved`, `changes_requested`, `review_required`, or "".
    pub review_decision: String,
    /// `success`, `failure`, `pending`, or "".
    pub checks: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub labels: Vec<Label>,
}

/// A label of a pull request, with its color as hex.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable")]
    pub color: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RestItem {
    #[serde(deserialize_with = "nullable")]
    number: u64,
    #[serde(deserialize_with = "nullable")]
    title: String,
    #[serde(deserialize_with = "nullable")]
    html_url: String,
    #[serde(deserialize_with = "nullable")]
    draft: bool,
    #[serde(deserialize_with = "nullable")]
    user: User,
    #[serde(deserialize_with = "nullable")]
    head: Branch,
    #[serde(with = "time::serde::rfc3339::option")]
    created_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    updated_at: Option<OffsetDateTime>,
    #[serde(deserialize_with = "nullable")]
    labels: Vec<Label>,
}

const OPEN_PULLS_QUERY: &str = r#"
query OpenPullRequests($query: String!) {
  search(query: $query, type: ISSUE, first: 100) {
    nodes {
      ... on PullRequest {
        number title url isDraft createdAt updatedAt headRefName
        author { login avatarUrl }
        labels(first: 10) { nodes { name color } }
        comments { totalCount }
        reviewDecision
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
        additions deletions changedFiles
      }
    }
  }
}"#;

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SearchNode {
    #[serde(deserialize_with = "nullable")]
    number: u64,
    #[serde(deserialize_with = "nullable")]
    title: String,
    #[serde(deserialize_with = "nullable")]
    url: String,
    #[serde(deserialize_with = "nullable")]
    is_draft: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    created_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    updated_at: Option<OffsetDateTime>,
    #[serde(deserialize_with = "nullable")]
    head_ref_name: String,
    author: Option<GraphQLUser>,
    labels: Option<Nodes<Label>>,
    #[serde(deserialize_with = "nullable")]
    comments: Count,
    #[serde(deserialize_with = "nullable")]
    review_decision: String,
    #[serde(deserialize_with = "nullable")]
    commits: Nodes<CommitNode>,
    #[serde(deserialize_with = "nullable")]
    additions: u64,
    #[serde(deserialize_with = "nullable")]
    deletions: u64,
    #[serde(deserialize_with = "nullable")]
    changed_files: u64,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GraphQLUser {
    #[serde(deserialize_with = "nullable")]
    login: String,
    #[serde(deserialize_with = "nullable")]
    avatar_url: String,
}

#[derive(Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de>"))]
pub(crate) struct Nodes<T> {
    #[serde(deserialize_with = "nullable")]
    pub(crate) nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Nodes { nodes: Vec::new() }
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Count {
    #[serde(deserialize_with = "nullable")]
    total_count: u64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CommitNode {
    #[serde(deserialize_with = "nullable")]
    commit: RollupCommit,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RollupCommit {
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Rollup {
    #[serde(deserialize_with = "nullable")]
    state: String,
}

impl Client {
    /// A pull request.
    pub fn pull(&self, repo: &Repo, number: u64) -> Result<PullRequest> {
        self.json("GET", &format!("/repos/{repo}/pulls/{number}"), None)
    }

    /// The commits of a pull request, oldest first.
    pub fn pull_commits(&self, repo: &Repo, number: u64) -> Result<Vec<PullCommit>> {
        self.paginate(&format!("/repos/{repo}/pulls/{number}/commits"))
    }

    /// The open pull requests of a repository, the recently updated first:
    /// the first hundred. With a token, through GraphQL, with the review
    /// decision, the checks and the counts; without, through REST.
    pub fn open_pulls(&self, repo: &Repo) -> Result<Vec<Summary>> {
        if self.token.is_empty() {
            self.open_pulls_rest(repo)
        } else {
            self.open_pulls_graphql(repo)
        }
    }

    fn open_pulls_rest(&self, repo: &Repo) -> Result<Vec<Summary>> {
        let items: Vec<RestItem> = self.json(
            "GET",
            &format!("/repos/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=100"),
            None,
        )?;
        Ok(items
            .into_iter()
            .map(|it| Summary {
                number: it.number,
                title: it.title,
                url: it.html_url,
                draft: it.draft,
                author: it.user,
                head_ref: it.head.reference,
                created_at: it.created_at,
                updated_at: it.updated_at,
                labels: it.labels,
                ..Default::default()
            })
            .collect())
    }

    fn open_pulls_graphql(&self, repo: &Repo) -> Result<Vec<Summary>> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Data {
            #[serde(deserialize_with = "nullable")]
            search: Nodes<SearchNode>,
        }
        let q = format!("repo:{repo} is:pr is:open sort:updated-desc");
        let data = self.graphql(OPEN_PULLS_QUERY, json!({"query": q}))?;
        let data: Data = from_data(data)?;
        Ok(data
            .search
            .nodes
            .into_iter()
            // Nodes of issues come empty.
            .filter(|n| n.number != 0)
            .map(|n| Summary {
                number: n.number,
                title: n.title,
                url: n.url,
                draft: n.is_draft,
                author: n
                    .author
                    .map(|a| User {
                        login: a.login,
                        avatar_url: a.avatar_url,
                    })
                    .unwrap_or_default(),
                head_ref: n.head_ref_name,
                created_at: n.created_at,
                updated_at: n.updated_at,
                comments: n.comments.total_count,
                review_decision: match n.review_decision.as_str() {
                    "APPROVED" => "approved",
                    "CHANGES_REQUESTED" => "changes_requested",
                    "REVIEW_REQUIRED" => "review_required",
                    _ => "",
                }
                .into(),
                checks: match n
                    .commits
                    .nodes
                    .first()
                    .and_then(|c| c.commit.status_check_rollup.as_ref())
                    .map(|r| r.state.as_str())
                {
                    Some("SUCCESS") => "success",
                    Some("FAILURE" | "ERROR") => "failure",
                    Some("PENDING" | "EXPECTED") => "pending",
                    _ => "",
                }
                .into(),
                additions: n.additions,
                deletions: n.deletions,
                changed_files: n.changed_files,
                labels: n.labels.map(|l| l.nodes).unwrap_or_default(),
            })
            .collect())
    }
}

pub(crate) fn from_data<T: serde::de::DeserializeOwned + Default>(data: Value) -> Result<T> {
    if data.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(data).map_err(|e| Error::Decode(e.to_string()))
}
