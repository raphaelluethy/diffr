//! Review threads, comments and reviews of a pull request.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;

use crate::pulls::{Nodes, from_data};
use crate::{Client, Repo, Result, User, nullable};

/// The side of a diff a review comment is on: `LEFT` for the old file's
/// lines, `RIGHT` for the new file's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    #[serde(rename = "LEFT")]
    Left,
    #[default]
    #[serde(rename = "RIGHT")]
    Right,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Left => "LEFT",
            Side::Right => "RIGHT",
        }
    }
}

/// A review comment and its replies, on a line or a range of lines of a
/// file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub root_id: i64,
    /// Identifies the thread in GraphQL, to resolve it; "" when not known,
    /// without a token.
    pub node_id: String,
    pub path: String,
    pub side: Side,
    /// The last line; 0 once outdated.
    pub line: u32,
    /// The first line of a range, else 0.
    pub start_line: u32,
    pub original_line: u32,
    /// Set once the lines it was on changed.
    pub outdated: bool,
    pub resolved: bool,
    /// A draft of the viewer's review, which only they see.
    pub pending: bool,
    pub comments: Vec<Comment>,
}

/// A comment of a thread.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: i64,
    pub author: User,
    pub body: String,
    pub created_at: Option<OffsetDateTime>,
    pub url: String,
    pub pending: bool,
}

/// A submitted review with a verdict or a body.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub id: i64,
    pub author: User,
    /// `approved`, `changes_requested`, `commented` or `dismissed`.
    pub state: String,
    pub body: String,
    pub submitted_at: Option<OffsetDateTime>,
    pub url: String,
}

/// The viewer's review not yet submitted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingReview {
    pub id: i64,
    pub node_id: String,
    pub body: String,
}

/// The review threads and the reviews of a pull request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reviews {
    pub threads: Vec<Thread>,
    pub summaries: Vec<ReviewSummary>,
    pub pending: Option<PendingReview>,
}

impl Reviews {
    /// Counts the comments of the threads.
    pub fn comments(&self) -> usize {
        self.threads.iter().map(|t| t.comments.len()).sum()
    }

    /// Counts the drafts of the viewer's pending review.
    pub fn pending_comments(&self) -> usize {
        self.threads
            .iter()
            .filter(|t| t.pending)
            .map(|t| t.comments.len())
            .sum()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ReviewCommentJson {
    #[serde(deserialize_with = "nullable")]
    pub(crate) id: i64,
    #[serde(deserialize_with = "nullable")]
    pub(crate) in_reply_to_id: i64,
    #[serde(deserialize_with = "nullable")]
    pub(crate) path: String,
    pub(crate) side: Option<Side>,
    pub(crate) line: Option<u32>,
    pub(crate) start_line: Option<u32>,
    pub(crate) original_line: Option<u32>,
    pub(crate) user: Option<User>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) body: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub(crate) created_at: Option<OffsetDateTime>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) html_url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ReviewJson {
    #[serde(deserialize_with = "nullable")]
    pub(crate) id: i64,
    #[serde(deserialize_with = "nullable")]
    pub(crate) node_id: String,
    pub(crate) user: Option<User>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) state: String,
    #[serde(deserialize_with = "nullable")]
    pub(crate) body: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub(crate) submitted_at: Option<OffsetDateTime>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) html_url: String,
}

/// A thread's state in GraphQL, which REST does not tell.
#[derive(Debug, Clone, Default)]
pub(crate) struct Resolution {
    pub(crate) node_id: String,
    pub(crate) resolved: bool,
}

/// Threads the comments under their first by `in_reply_to_id`, and splits
/// the reviews into the submitted and the viewer's pending one.
pub(crate) fn normalize(
    mut comments: Vec<ReviewCommentJson>,
    reviews: &[ReviewJson],
    pending: Vec<ReviewCommentJson>,
    res: &HashMap<i64, Resolution>,
) -> Reviews {
    let mut out = Reviews::default();
    let mut threads: HashMap<i64, usize> = HashMap::new();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut add = |out: &mut Reviews, c: ReviewCommentJson, is_pending: bool| {
        if !seen.insert(c.id) {
            return;
        }
        let root = if c.in_reply_to_id != 0 {
            c.in_reply_to_id
        } else {
            c.id
        };
        let ti = *threads.entry(root).or_insert_with(|| {
            let mut t = Thread {
                root_id: root,
                path: c.path.clone(),
                side: c.side.unwrap_or_default(),
                pending: is_pending,
                ..Default::default()
            };
            match c.line {
                Some(l) => t.line = l,
                None => t.outdated = true,
            }
            t.start_line = c.start_line.unwrap_or(0);
            t.original_line = c.original_line.unwrap_or(0);
            if let Some(r) = res.get(&root) {
                t.node_id = r.node_id.clone();
                t.resolved = r.resolved;
            }
            out.threads.push(t);
            out.threads.len() - 1
        });
        out.threads[ti].comments.push(Comment {
            id: c.id,
            author: c.user.unwrap_or_default(),
            body: c.body,
            created_at: c.created_at,
            url: c.html_url,
            pending: is_pending,
        });
    };
    // Roots come before their replies, as GitHub lists them by id.
    comments.sort_by_key(|c| c.id);
    for c in comments {
        add(&mut out, c, false);
    }
    for c in pending {
        add(&mut out, c, true);
    }
    for r in reviews {
        if r.state == "PENDING" {
            out.pending = Some(PendingReview {
                id: r.id,
                node_id: r.node_id.clone(),
                body: r.body.clone(),
            });
            continue;
        }
        // A comment review without a body wraps comments posted alone.
        if r.state == "COMMENTED" && r.body.is_empty() {
            continue;
        }
        let state = match r.state.as_str() {
            "APPROVED" => "approved",
            "CHANGES_REQUESTED" => "changes_requested",
            "COMMENTED" => "commented",
            "DISMISSED" => "dismissed",
            _ => continue,
        };
        out.summaries.push(ReviewSummary {
            id: r.id,
            author: r.user.clone().unwrap_or_default(),
            state: state.into(),
            body: r.body.clone(),
            submitted_at: r.submitted_at,
            url: r.html_url.clone(),
        });
    }
    out
}

const REVIEW_THREADS_QUERY: &str = r#"
query ReviewThreads($owner: String!, $repo: String!, $number: Int!, $cursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes { id isResolved comments(first: 1) { nodes { fullDatabaseId } } }
      }
    }
  }
}"#;

/// A comment to post on a line, or on the lines `start_line..=line` of a
/// side.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewComment {
    pub body: String,
    /// The head commit the lines are of.
    pub commit_id: String,
    pub path: String,
    pub side: Side,
    pub line: u32,
    /// The first line of a range, 0 (or `line`) for one line.
    pub start_line: u32,
}

impl NewComment {
    fn is_range(&self) -> bool {
        self.start_line != 0 && self.start_line != self.line
    }

    fn fields(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("path".into(), json!(self.path));
        m.insert("body".into(), json!(self.body));
        m.insert("side".into(), json!(self.side));
        m.insert("line".into(), json!(self.line));
        if self.is_range() {
            m.insert("start_line".into(), json!(self.start_line));
            m.insert("start_side".into(), json!(self.side));
        }
        m
    }
}

const ADD_THREAD_MUTATION: &str = r#"
mutation AddPendingThread($reviewId: ID!, $path: String!, $body: String!, $line: Int!, $side: DiffSide!, $startLine: Int, $startSide: DiffSide) {
  addPullRequestReviewThread(input: {pullRequestReviewId: $reviewId, path: $path, body: $body, line: $line, side: $side, startLine: $startLine, startSide: $startSide}) {
    thread { id }
  }
}"#;

const RESOLVE_MUTATION: &str =
    "mutation Resolve($id: ID!) { resolveReviewThread(input: {threadId: $id}) { thread { id } } }";
const UNRESOLVE_MUTATION: &str = "mutation Unresolve($id: ID!) { unresolveReviewThread(input: {threadId: $id}) { thread { id } } }";

/// The verdicts of a review, for [`Client::submit_review`].
pub const VERDICT_COMMENT: &str = "COMMENT";
pub const VERDICT_APPROVE: &str = "APPROVE";
pub const VERDICT_REQUEST_CHANGES: &str = "REQUEST_CHANGES";

impl Client {
    /// The review threads, the reviews and the viewer's pending review of
    /// a pull request. With a token, the threads' resolution comes from
    /// GraphQL; without, it is unknown.
    pub fn reviews(&self, repo: &Repo, number: u64) -> Result<Reviews> {
        let base = format!("/repos/{repo}/pulls/{number}");
        let comments: Vec<ReviewCommentJson> = self.paginate(&format!("{base}/comments"))?;
        let reviews: Vec<ReviewJson> = self.paginate(&format!("{base}/reviews"))?;
        let mut pending = Vec::new();
        for r in &reviews {
            if r.state == "PENDING" {
                pending = self.paginate(&format!("{base}/reviews/{}/comments", r.id))?;
            }
        }
        let res = if self.token.is_empty() {
            HashMap::new()
        } else {
            // Resolution is GraphQL's alone: unknown when it fails.
            self.resolutions(repo, number).unwrap_or_default()
        };
        Ok(normalize(comments, &reviews, pending, &res))
    }

    /// Whether each thread is resolved, by the id of its first comment.
    fn resolutions(&self, repo: &Repo, number: u64) -> Result<HashMap<i64, Resolution>> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Data {
            repository: Option<RepoNode>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct RepoNode {
            pull_request: Option<PullNode>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct PullNode {
            #[serde(deserialize_with = "nullable")]
            review_threads: ThreadPage,
        }
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct ThreadPage {
            #[serde(deserialize_with = "nullable")]
            page_info: PageInfo,
            #[serde(deserialize_with = "nullable")]
            nodes: Vec<ThreadNode>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct PageInfo {
            #[serde(deserialize_with = "nullable")]
            has_next_page: bool,
            #[serde(deserialize_with = "nullable")]
            end_cursor: String,
        }
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct ThreadNode {
            #[serde(deserialize_with = "nullable")]
            id: String,
            #[serde(deserialize_with = "nullable")]
            is_resolved: bool,
            #[serde(deserialize_with = "nullable")]
            comments: Nodes<FirstComment>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct FirstComment {
            full_database_id: Value,
        }

        let mut out = HashMap::new();
        let mut cursor = Value::Null;
        loop {
            let vars =
                json!({"owner": repo.owner, "repo": repo.name, "number": number, "cursor": cursor});
            let data: Data = from_data(self.graphql(REVIEW_THREADS_QUERY, vars)?)?;
            let Some(pr) = data.repository.and_then(|r| r.pull_request) else {
                return Ok(out);
            };
            let rt = pr.review_threads;
            for n in rt.nodes {
                let Some(first) = n.comments.nodes.first() else {
                    continue;
                };
                // A BigInt, which GitHub sends as a string.
                let id = match &first.full_database_id {
                    Value::String(s) => s.trim().parse::<i64>().unwrap_or(0),
                    Value::Number(n) => n.as_i64().unwrap_or(0),
                    _ => 0,
                };
                if id != 0 {
                    out.insert(
                        id,
                        Resolution {
                            node_id: n.id,
                            resolved: n.is_resolved,
                        },
                    );
                }
            }
            if !rt.page_info.has_next_page {
                return Ok(out);
            }
            cursor = Value::String(rt.page_info.end_cursor);
        }
    }

    /// Posts a review comment at once, outside a review.
    pub fn comment(&self, repo: &Repo, number: u64, n: &NewComment) -> Result<()> {
        let mut body = n.fields();
        body.insert("commit_id".into(), json!(n.commit_id));
        self.send(
            "POST",
            &format!("/repos/{repo}/pulls/{number}/comments"),
            Some(&Value::Object(body)),
        )
        .map(drop)
    }

    /// Adds a comment to the viewer's pending review, which it starts when
    /// there is none.
    pub fn add_to_review(
        &self,
        repo: &Repo,
        number: u64,
        pending: Option<&PendingReview>,
        n: &NewComment,
    ) -> Result<()> {
        let Some(pending) = pending else {
            // REST attaches comments to a review as it starts only.
            let body = json!({"commit_id": n.commit_id, "comments": [Value::Object(n.fields())]});
            return self
                .send(
                    "POST",
                    &format!("/repos/{repo}/pulls/{number}/reviews"),
                    Some(&body),
                )
                .map(drop);
        };
        let (start_line, start_side) = if n.is_range() {
            (json!(n.start_line), json!(n.side))
        } else {
            (Value::Null, Value::Null)
        };
        let vars = json!({
            "reviewId": pending.node_id, "path": n.path, "body": n.body, "line": n.line,
            "side": n.side, "startLine": start_line, "startSide": start_side,
        });
        self.graphql(ADD_THREAD_MUTATION, vars).map(drop)
    }

    /// Answers a thread, at once.
    pub fn reply(&self, repo: &Repo, number: u64, root: i64, body: &str) -> Result<()> {
        self.send(
            "POST",
            &format!("/repos/{repo}/pulls/{number}/comments/{root}/replies"),
            Some(&json!({"body": body})),
        )
        .map(drop)
    }

    /// Changes the text of a review comment.
    pub fn edit_comment(&self, repo: &Repo, id: i64, body: &str) -> Result<()> {
        self.send(
            "PATCH",
            &format!("/repos/{repo}/pulls/comments/{id}"),
            Some(&json!({"body": body})),
        )
        .map(drop)
    }

    /// Deletes a review comment.
    pub fn delete_comment(&self, repo: &Repo, id: i64) -> Result<()> {
        self.send(
            "DELETE",
            &format!("/repos/{repo}/pulls/comments/{id}"),
            None,
        )
        .map(drop)
    }

    /// Resolves a thread, or unresolves it, by its GraphQL id
    /// ([`Thread::node_id`]).
    pub fn resolve(&self, node_id: &str, resolved: bool) -> Result<()> {
        let q = if resolved {
            RESOLVE_MUTATION
        } else {
            UNRESOLVE_MUTATION
        };
        self.graphql(q, json!({"id": node_id})).map(drop)
    }

    /// Submits the viewer's pending review with a verdict (`event`, one of
    /// the `VERDICT_*`), or a review without comments when there is none
    /// pending.
    pub fn submit_review(
        &self,
        repo: &Repo,
        number: u64,
        pending: Option<&PendingReview>,
        event: &str,
        body: &str,
    ) -> Result<()> {
        let base = format!("/repos/{repo}/pulls/{number}/reviews");
        let payload = json!({"event": event, "body": body});
        let path = match pending {
            Some(p) => format!("{base}/{}/events", p.id),
            None => base,
        };
        self.send("POST", &path, Some(&payload)).map(drop)
    }

    /// Deletes the viewer's pending review and its drafts.
    pub fn discard_review(&self, repo: &Repo, number: u64, pending: &PendingReview) -> Result<()> {
        self.send(
            "DELETE",
            &format!("/repos/{repo}/pulls/{number}/reviews/{}", pending.id),
            None,
        )
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(l: &str) -> Option<User> {
        Some(User {
            login: l.into(),
            ..Default::default()
        })
    }

    fn rc(
        id: i64,
        reply: i64,
        path: &str,
        side: Side,
        line: Option<u32>,
        who: &str,
        body: &str,
    ) -> ReviewCommentJson {
        ReviewCommentJson {
            id,
            in_reply_to_id: reply,
            path: path.into(),
            side: Some(side),
            line,
            user: user(who),
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn normalizes() {
        let mut first = rc(1, 0, "a.go", Side::Right, Some(4), "ada", "Rename this");
        first.start_line = Some(2);
        let mut gone = rc(2, 0, "b.go", Side::Left, None, "ada", "Gone");
        gone.original_line = Some(9);
        let comments = vec![
            rc(3, 1, "a.go", Side::Right, Some(4), "bo", "Agreed"),
            first,
            gone,
        ];
        let review = |id, state: &str, who, body: &str| ReviewJson {
            id,
            state: state.into(),
            user: user(who),
            body: body.into(),
            ..Default::default()
        };
        let mut pending_review = review(12, "PENDING", "ada", "");
        pending_review.node_id = "PRR_x".into();
        let reviews = vec![
            review(10, "APPROVED", "bo", "LGTM"),
            review(11, "COMMENTED", "ada", ""),
            pending_review,
        ];
        let pending = vec![rc(20, 0, "a.go", Side::Right, Some(8), "ada", "Draft")];
        let res = HashMap::from([(
            1,
            Resolution {
                node_id: "T_1".into(),
                resolved: true,
            },
        )]);
        let r = normalize(comments, &reviews, pending, &res);
        assert_eq!(r.threads.len(), 3);
        let t = &r.threads[0];
        assert_eq!(t.root_id, 1);
        assert_eq!(t.comments.len(), 2);
        assert_eq!(t.comments[1].body, "Agreed");
        assert_eq!(t.start_line, 2);
        assert!(t.resolved);
        assert_eq!(t.node_id, "T_1");
        let t = &r.threads[1];
        assert!(t.outdated);
        assert_eq!(t.original_line, 9);
        assert_eq!(t.side, Side::Left);
        assert!(r.threads[2].pending);
        assert_eq!(r.pending_comments(), 1);
        assert_eq!(r.comments(), 4);
        let p = r.pending.as_ref().unwrap();
        assert_eq!((p.id, p.node_id.as_str()), (12, "PRR_x"));
        assert_eq!(r.summaries.len(), 1);
        assert_eq!(r.summaries[0].state, "approved");
        assert_eq!(r.summaries[0].author.login, "bo");
    }
}
