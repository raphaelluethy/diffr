//! Commit messages in the Conventional Commits format
//! (<https://www.conventionalcommits.org/en/v1.0.0/>): written by an agent
//! for the changes staged, and suggested with each group of a review of
//! the work tree, so a group can be committed on its own.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::run::{Job, exec, takes_schema};
use crate::{Agent, CancelToken, Error, File, INLINE_LIMIT, diffs, extract_json, manifest, nullable};

/// The types a commit may have.
pub const TYPES: [&str; 11] = [
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// A commit message, in its parts: `type(scope)!: description`, a body,
/// and the `BREAKING CHANGE:` footer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitMessage {
    /// One of [`TYPES`].
    #[serde(rename = "type", default, deserialize_with = "nullable")]
    pub kind: String,
    /// What it touches, as `parser`; "" for none.
    #[serde(default, deserialize_with = "nullable")]
    pub scope: String,
    /// The subject's text, imperative and lowercase, without a period.
    #[serde(default, deserialize_with = "nullable")]
    pub description: String,
    /// Why, wrapped prose; "" for none.
    #[serde(default, deserialize_with = "nullable")]
    pub body: String,
    /// What breaks, for the footer; "" when nothing does.
    #[serde(default, deserialize_with = "nullable")]
    pub breaking: String,
}

impl CommitMessage {
    /// Trims the parts, and makes them fit the format: a known type,
    /// a scope without parentheses or spaces, a description without the
    /// type an agent may have repeated or a final period.
    pub fn tidy(mut self) -> Self {
        let kind = self.kind.trim().to_lowercase();
        self.kind = if TYPES.contains(&kind.as_str()) { kind } else { "chore".into() };
        self.scope = self
            .scope
            .trim()
            .trim_matches(['(', ')'])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        let mut d = self.description.trim().lines().next().unwrap_or("").trim().to_string();
        // "feat(x): add y" given as the description.
        if let Some((head, rest)) = d.split_once(": ")
            && TYPES.iter().any(|t| head.trim_end_matches('!').starts_with(t))
            && !head.contains(' ')
        {
            d = rest.trim().to_string();
        }
        while d.ends_with('.') {
            d.pop();
        }
        // Lowercase its first letter, unless it starts an acronym or a
        // name in code (`URL`, `Diffr::run`).
        let mut cs = d.chars();
        if let Some(c) = cs.next() {
            let second = cs.next();
            if c.is_uppercase() && second.is_some_and(|s| s.is_lowercase()) {
                d = c.to_lowercase().chain(d.chars().skip(1)).collect();
            }
        }
        self.description = d;
        self.body = self.body.trim().to_string();
        self.breaking = self.breaking.trim().to_string();
        self
    }

    /// Whether there is anything to commit with.
    pub fn is_empty(&self) -> bool {
        self.description.trim().is_empty()
    }

    /// `type(scope)!: description`.
    pub fn subject(&self) -> String {
        let mut s = self.kind.clone();
        if s.is_empty() {
            s.push_str("chore");
        }
        if !self.scope.is_empty() {
            s.push_str(&format!("({})", self.scope));
        }
        if !self.breaking.is_empty() {
            s.push('!');
        }
        format!("{s}: {}", self.description)
    }

    /// The body, then the `BREAKING CHANGE:` footer.
    pub fn body_text(&self) -> String {
        let mut b = self.body.clone();
        if !self.breaking.is_empty() {
            if !b.is_empty() {
                b.push_str("\n\n");
            }
            b.push_str(&format!("BREAKING CHANGE: {}", self.breaking));
        }
        b
    }
}

/// The JSON Schema of a [`CommitMessage`], as a property's.
pub const MESSAGE_SCHEMA: &str = r#"{"type": "object", "additionalProperties": false, "required": ["type", "scope", "description", "body", "breaking"], "description": "A Conventional Commits message.", "properties": {"type": {"type": "string", "enum": ["feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert"]}, "scope": {"type": "string", "description": "A noun for the part of the codebase it touches, e.g. \"parser\", in the repository's own terms; \"\" when it spans too much."}, "description": {"type": "string", "description": "The subject: imperative mood, lowercase, no final period, at most about 60 characters with the type and scope."}, "body": {"type": "string", "description": "Why the change is made and what a reader of the history needs to know, in 1-4 short plain-text sentences or bullet lines (\"- \"); \"\" when the subject says it all."}, "breaking": {"type": "string", "description": "What breaks for users of a public interface or stored format, in one sentence; \"\" when nothing does."}}}"#;

/// The rules of the format, for the prompts.
pub const RULES: &str = r#"Write commit messages in the Conventional Commits format: `type(scope)!: description`.
- "type": feat (a new feature for users), fix (a bug fix), docs, style (formatting only), refactor (neither fixes nor adds), perf, test, build (build system, dependencies), ci, chore (anything else), revert. The type is what the change does for its users, not the files it touches: a fix that adds a test is a fix.
- "scope": a short noun for the area, as the repository's own commits name it when they use scopes; "" when the change spans the codebase.
- "description": imperative mood ("add", not "added" or "adds"), lowercase, no final period, specific (not "update files"). Keep the whole subject line under about 60 characters.
- "body": why the change is made, and anything a reader of the history needs, in a few short plain sentences or "- " lines; "" when the subject says it all. Do not list the files.
- "breaking": only for a breaking change of a public interface or stored format, in one sentence; else "".
- Follow the style of the repository's recent commits where it fits the format (their scopes, their wording)."#;

/// What a commit message is written for.
#[derive(Debug, Clone, Default)]
pub struct MessageRequest {
    /// Where the agent runs: the repository.
    pub dir: PathBuf,
    /// The model, "" for the agent's default.
    pub model: String,
    /// The branch the commit goes onto.
    pub branch: String,
    /// The changes staged, with their diffs.
    pub files: Vec<File>,
    /// The subjects of the latest commits, newest first, for their style.
    pub recent: Vec<String>,
    /// What the author means by it, as the review's group says: a title
    /// and a summary; "" for none.
    pub intent: String,
    /// Codex's fast mode (see [`crate::Model::fast`]).
    pub fast: bool,
}

/// The agent's instructions for a message.
pub fn message_system_prompt(fenced: bool) -> String {
    let answer = if fenced {
        "Answer with the JSON object matching the schema in one fenced ```json block, and nothing else."
    } else {
        "Answer with the JSON object matching the schema, and nothing else."
    };
    format!(
        "<role>\nYou write the commit message for the changes a colleague has staged, from their diffs.\n</role>\n\n\
         <format>\n{RULES}\n</format>\n\n\
         <workflow>\n1. Read the manifest and the diffs; open files of the repository (the current directory) only when the diffs are not enough to tell what the change is for.\n\
         2. Never modify anything: do not edit files, stage, commit or run builds.\n\
         3. {answer}\n</workflow>\n\n<schema>\n{MESSAGE_SCHEMA}\n</schema>"
    )
}

/// Describes the changes staged.
pub fn message_user_prompt(req: &MessageRequest) -> String {
    changes_prompt(req, "STAGED")
}

/// Describes changes, under a heading of their own.
fn changes_prompt(req: &MessageRequest, heading: &str) -> String {
    use std::fmt::Write;
    let mut b = String::new();
    if !req.branch.is_empty() {
        let _ = writeln!(b, "Branch: {}", req.branch);
    }
    if !req.intent.trim().is_empty() {
        let _ = write!(b, "\n---INTENT---\n{}\n", req.intent.trim());
    }
    if !req.recent.is_empty() {
        let _ = write!(b, "\n---RECENT COMMITS--- (newest first)\n{}\n", req.recent.join("\n"));
    }
    let adds: usize = req.files.iter().map(|f| f.additions).sum();
    let dels: usize = req.files.iter().map(|f| f.deletions).sum();
    let _ = write!(
        b,
        "\n---{heading}--- ({} files, +{adds}/-{dels})\n{}\n",
        req.files.len(),
        manifest(&req.files)
    );
    let mut d = diffs(&req.files);
    if d.len() > INLINE_LIMIT {
        let mut end = INLINE_LIMIT;
        while !d.is_char_boundary(end) {
            end -= 1;
        }
        d.truncate(end);
        d.push_str("\n\n(the diffs are cut here: they are too long; read the files for the rest)");
    }
    let _ = write!(b, "\n---DIFFS---\n{d}\n");
    b
}

/// Asks an agent for the commit message of the changes staged, reporting
/// what it does as it goes. It blocks until the agent answers.
pub fn message(
    agent: Agent,
    req: &MessageRequest,
    cancel: &CancelToken,
    mut progress: impl FnMut(&str),
) -> Result<CommitMessage, Error> {
    if req.files.is_empty() {
        return Err(Error::Failed("nothing is staged".into()));
    }
    let job = Job {
        dir: req.dir.clone(),
        model: req.model.clone(),
        system: message_system_prompt(!takes_schema(agent)),
        user: message_user_prompt(req),
        schema: MESSAGE_SCHEMA.to_string(),
        tools: Vec::new(),
        tool_bin: String::new(),
        extra_dir: None,
        fast: req.fast,
    };
    let answer = exec(agent, job, cancel, &mut progress)?;
    let m = match answer.structured {
        Some(v) => serde_json::from_value::<CommitMessage>(v).map_err(|e| e.to_string()),
        None => serde_json::from_str::<CommitMessage>(extract_json(&answer.text)).map_err(|e| e.to_string()),
    }
    .map_err(|e| Error::Failed(format!("{}: the answer is not a commit message: {e}", agent.label)))?
    .tidy();
    if m.is_empty() {
        return Err(Error::Failed(format!("{} wrote no commit message", agent.label)));
    }
    Ok(m)
}

/// A commit of a plan: files to commit together, and their message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedCommit {
    #[serde(default, deserialize_with = "nullable")]
    pub label: String,
    #[serde(default, deserialize_with = "nullable")]
    pub file_paths: Vec<String>,
    /// `None` for the files the agent left out, gathered at the end.
    #[serde(default)]
    pub commit: Option<CommitMessage>,
}

#[derive(Deserialize)]
struct Plan {
    #[serde(default, deserialize_with = "nullable")]
    commits: Vec<PlannedCommit>,
}

/// The JSON Schema of a plan of commits.
pub fn plan_schema() -> String {
    format!(
        r#"{{"type": "object", "additionalProperties": false, "required": ["commits"], "properties": {{"commits": {{"type": "array", "description": "The commits, in the order to make them.", "items": {{"type": "object", "additionalProperties": false, "required": ["label", "filePaths", "commit"], "properties": {{"label": {{"type": "string", "description": "Display name, at most 4 words."}}, "filePaths": {{"type": "array", "items": {{"type": "string"}}, "description": "Paths of the manifest in this commit. Every path goes into exactly one commit."}}, "commit": {MESSAGE_SCHEMA}}}}}}}}}}}"#
    )
}

/// The agent's instructions for a plan of commits.
pub fn plan_system_prompt(fenced: bool) -> String {
    let answer = if fenced {
        "Answer with the JSON object matching the schema in one fenced ```json block, and nothing else."
    } else {
        "Answer with the JSON object matching the schema, and nothing else."
    };
    format!(
        "<role>\nA colleague has uncommitted changes and wants to commit them as a series of focused commits. You split the changed files into those commits and write each one's message.\n</role>\n\n\
         <grouping>\n- One commit per intent: a feature with its tests and docs, a fix, a refactor, a dependency bump. Not one per directory or per file.\n\
         - Use the fewest commits that keep independent intents apart; a small change is one commit.\n\
         - Every path of the manifest goes into exactly one commit. A file serving two intents goes with the one it serves most.\n\
         - Order the commits so each builds on the ones before: what others depend on first.\n</grouping>\n\n\
         <format>\n{RULES}\n</format>\n\n\
         <workflow>\n1. Read the manifest and the diffs; open files of the repository (the current directory) only when the diffs are not enough to tell what a change is for.\n\
         2. Never modify anything: do not edit files, stage, commit or run builds.\n\
         3. {answer}\n</workflow>\n\n<schema>\n{}\n</schema>",
        plan_schema()
    )
}

/// Keeps the plan to the files: paths it invented go, each path stays in
/// its first commit, commits left empty go, and the files it left out
/// gather in a last commit without a message.
pub fn fit_plan(commits: Vec<PlannedCommit>, files: &[File]) -> Vec<PlannedCommit> {
    let known: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut placed: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for mut c in commits {
        c.file_paths = c
            .file_paths
            .iter()
            .map(|p| p.trim().trim_start_matches("./").to_string())
            .filter(|p| known.contains(p.as_str()) && placed.insert(p.clone()))
            .collect();
        if c.file_paths.is_empty() {
            continue;
        }
        c.commit = c.commit.map(CommitMessage::tidy).filter(|m| !m.is_empty());
        if c.label.trim().is_empty() {
            c.label = c.commit.as_ref().map(|m| m.description.clone()).unwrap_or_else(|| "Changes".into());
        }
        out.push(c);
    }
    let rest: Vec<String> = files.iter().filter(|f| !placed.contains(&f.path)).map(|f| f.path.clone()).collect();
    if !rest.is_empty() {
        out.push(PlannedCommit {
            label: "Other changes".into(),
            file_paths: rest,
            commit: None,
        });
    }
    out
}

/// Asks an agent to split changes into commits, with their messages. It
/// blocks until the agent answers.
pub fn plan(
    agent: Agent,
    req: &MessageRequest,
    cancel: &CancelToken,
    mut progress: impl FnMut(&str),
) -> Result<Vec<PlannedCommit>, Error> {
    if req.files.is_empty() {
        return Err(Error::Failed("there are no changes".into()));
    }
    let job = Job {
        dir: req.dir.clone(),
        model: req.model.clone(),
        system: plan_system_prompt(!takes_schema(agent)),
        user: changes_prompt(req, "MANIFEST"),
        schema: plan_schema(),
        tools: Vec::new(),
        tool_bin: String::new(),
        extra_dir: None,
        fast: req.fast,
    };
    let answer = exec(agent, job, cancel, &mut progress)?;
    let p = match answer.structured {
        Some(v) => serde_json::from_value::<Plan>(v).map_err(|e| e.to_string()),
        None => serde_json::from_str::<Plan>(extract_json(&answer.text)).map_err(|e| e.to_string()),
    }
    .map_err(|e| Error::Failed(format!("{}: the answer is not a plan of commits: {e}", agent.label)))?;
    let commits = fit_plan(p.commits, &req.files);
    if commits.iter().all(|c| c.commit.is_none()) {
        return Err(Error::Failed(format!("{} planned no commits", agent.label)));
    }
    Ok(commits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let m = CommitMessage {
            kind: " Feat".into(),
            scope: "(commit view)".into(),
            description: "feat(x): Add AI messages.".into(),
            body: " Why. ".into(),
            breaking: String::new(),
        }
        .tidy();
        assert_eq!(m.subject(), "feat(commit-view): add AI messages");
        assert_eq!(m.body_text(), "Why.");
        let m = CommitMessage {
            kind: "nope".into(),
            description: "URL parsing changes".into(),
            breaking: "Drops the v1 format.".into(),
            ..Default::default()
        }
        .tidy();
        assert_eq!(m.subject(), "chore!: URL parsing changes");
        assert_eq!(m.body_text(), "BREAKING CHANGE: Drops the v1 format.");
        assert!(CommitMessage::default().tidy().is_empty());
    }

    #[test]
    fn plans() {
        let v: serde_json::Value = serde_json::from_str(&plan_schema()).unwrap();
        assert_eq!(v["properties"]["commits"]["items"]["required"].as_array().unwrap().len(), 3);
        assert!(plan_system_prompt(false).contains("exactly one commit"));
        let raw: Plan = serde_json::from_str(r#"{"commits":[
 {"label":"Greeting","filePaths":["./src/greet.go","src/greet_test.go","nope.go"],"commit":{"type":"feat","scope":"","description":"Say hi","body":"","breaking":""}},
 {"label":"","filePaths":["src/greet.go"],"commit":null},
 {"label":"","filePaths":["pnpm-lock.yaml"],"commit":{"type":"build","scope":"deps","description":"bump","body":"","breaking":""}}]}"#).unwrap();
        let p = fit_plan(raw.commits, &crate::tests::files());
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].file_paths, ["src/greet.go", "src/greet_test.go"]);
        assert_eq!(p[0].commit.as_ref().unwrap().subject(), "feat: say hi");
        assert_eq!(p[1].label, "bump");
        assert_eq!(p[2].file_paths, ["logo.png"]);
        assert!(p[2].commit.is_none());
    }

    #[test]
    fn prompts_and_schema() {
        let v: serde_json::Value = serde_json::from_str(MESSAGE_SCHEMA).unwrap();
        assert_eq!(v["properties"]["type"]["enum"].as_array().unwrap().len(), TYPES.len());
        let sys = message_system_prompt(true);
        assert!(sys.contains("Conventional Commits") && sys.contains("fenced ```json"));
        let req = MessageRequest {
            branch: "main".into(),
            files: crate::tests::files(),
            recent: vec!["feat(git): stage files".into()],
            intent: "Greeting: shorter".into(),
            ..Default::default()
        };
        let user = message_user_prompt(&req);
        for want in ["Branch: main", "---INTENT---\nGreeting", "---RECENT COMMITS---", "---STAGED--- (4 files", "### src/greet.go"] {
            assert!(user.contains(want), "no {want:?} in {user}");
        }
        let m: CommitMessage = serde_json::from_str(
            r#"{"type":"fix","scope":null,"description":"handle empty input","body":"","breaking":""}"#,
        )
        .unwrap();
        assert_eq!(m.tidy().subject(), "fix: handle empty input");
    }
}
