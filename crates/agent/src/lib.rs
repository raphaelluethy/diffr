//! Asks a coding agent installed on the machine (Claude Code, Codex or
//! OpenCode) to review a change: to group its files by intent,
//! summarize it, and note what deserves a reviewer's attention.
//! [`cache`] keeps the reviews, and tells what of them still holds.
//!
//! Everything here blocks: run it on a background thread. [`run()`] takes
//! a [`CancelToken`] the UI sets to stop the agent, which kills its whole
//! process group.
//!
//! # Why the agents' own CLIs, and not the Agent Client Protocol
//!
//! Zed runs external agents through the Agent Client Protocol (its
//! `agent_servers` and `acp_thread` crates): it spawns an ACP server (an
//! adapter such as `claude-agent-acp` or `codex-acp`, or Gemini CLI
//! itself) and speaks JSON-RPC over its stdio — `initialize`,
//! `session/new`, `session/prompt` with `session/update` notifications
//! streamed back, `session/cancel` to stop, `session/request_permission`
//! for the agent to ask before a tool runs, and `fs/read_text_file`,
//! `terminal/*` for the client to serve files and commands. Models are
//! chosen with `session/set_config_option` (the option of category
//! `model`).
//!
//! That fits an editor holding a conversation; it would not improve this
//! one-shot, read-only review, so this crate runs the CLIs:
//!
//! - ACP has no structured output: a turn ends with a `stopReason`, and the
//!   answer is free text. The CLIs take a JSON Schema themselves
//!   (`claude --json-schema`, `codex exec --output-schema`), which is
//!   sturdier than parsing a fenced block, as we must for OpenCode.
//! - The ACP adapters are separate installs (npm packages) on top of the
//!   agents; the CLIs are what users already have on their PATH.
//! - Read-only is enforced by the CLIs' own flags (allowed tools, sandbox,
//!   permission modes). Under ACP the client would have to answer every
//!   permission request and police the paths of `fs/*` itself.
//! - Cancelling by killing the process group is as prompt as
//!   `session/cancel` for a run nobody resumes.
//!
//! What would be worth taking from it later: if a review becomes a
//! conversation (asking follow-ups about a note), ACP gives one protocol
//! for every agent, with streamed tool calls and per-session models, in
//! place of three stream-json dialects. Reusing ACP's `session/update` shape
//! for [`run()`]'s progress would ease that move.

pub mod cache;
pub mod commit;
pub mod labs;
pub mod models;
pub mod proc;
mod run;
pub mod shellenv;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

pub use commit::{CommitMessage, MessageRequest, PlannedCommit};
pub use labs::{DIAGRAM_FIELD, DIAGRAMS, LABS, Lab, LabField, LabFieldKind};
pub use models::{Model, ModelList, models};
pub use run::{CancelToken, TIMEOUT, run};

/// A coding agent's command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Agent {
    /// The command: `claude`, `codex` or `opencode`.
    pub name: &'static str,
    pub label: &'static str,
}

/// The agents the app knows, in the order it prefers them.
pub const ALL: [Agent; 3] = [
    Agent {
        name: "claude",
        label: "Claude Code",
    },
    Agent {
        name: "codex",
        label: "Codex",
    },
    Agent {
        name: "opencode",
        label: "OpenCode",
    },
];

/// The agent of a command.
pub fn by_name(name: &str) -> Option<Agent> {
    ALL.iter().copied().find(|a| a.name == name)
}

/// The agents found in the user's PATH (see [`shellenv::path`]).
pub fn installed() -> Vec<Agent> {
    ALL.iter()
        .copied()
        .filter(|a| shellenv::look_path(a.name).is_some())
        .collect()
}

/// The agent named in the settings, else the first installed.
/// `name` is "" for automatic.
pub fn pick(name: &str) -> Result<Agent, Error> {
    if let Some(a) = by_name(name) {
        return Ok(a);
    }
    installed().into_iter().next().ok_or_else(|| {
        Error::Failed("No coding agent was found. Install Claude Code, Codex or OpenCode, or name one as aiAgent in the config file.".into())
    })
}

/// An error of an agent's run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The run was stopped through its [`CancelToken`].
    Canceled,
    /// Anything else, with a message for the user.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Canceled => f.write_str("canceled"),
            Error::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Failed(e.to_string())
    }
}

/// Reads null as the default value, as Go's JSON does.
pub(crate) fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// An agent's review of a change.
///
/// Its JSON is that of the first analyses, which still load, with the fields
/// added since (`order`, `fileSummaries`, `diagrams`, `labs`, the labs'
/// fields) optional. It reads the answers of both shapes the prompt has
/// asked for:
///
/// - diffr's first: `fileSummaries: [{path, summary}]` of the whole review,
///   and the Diagrams lab's `diagrams: [{title, caption, mermaid}]`;
/// - the second, asked for now: `fileSummaries: [{path, text}]` of each group,
///   and the labs' string fields of the review and of each group (the
///   Diagrams lab's `diagram`, a Mermaid source), saved as they came.
///
/// [`fit`] brings the summaries of the groups into
/// [`file_summaries`](Self::file_summaries), and the review's `diagram`
/// into [`diagrams`](Self::diagrams).
///
/// Severity: the groups and notes marked `critical` are those the UI labels
/// "Needs care"; it labels the other notes "Worth a look". The prompt asks
/// the agent to mark few of them critical.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "RawAnalysis")]
pub struct Analysis {
    #[serde(default, deserialize_with = "nullable")]
    pub overall_summary: String,
    /// The groups in reading order, the order a reviewer should read them.
    #[serde(default, deserialize_with = "nullable")]
    pub groups: Vec<Group>,
    /// Every path of the change once, in the order a reviewer should read
    /// them. Empty in older analyses: see [`Analysis::reading_order`].
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub order: Vec<String>,
    /// What changed in each file and why, in one sentence: those of the
    /// whole review and, once [`fit`], those of its groups.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub file_summaries: Vec<FileSummary>,
    /// The diagrams of the change, by the [`DIAGRAMS`] lab: its `diagram`
    /// (without a title or caption), and those of diffr's earlier answers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagrams: Vec<Diagram>,
    /// The labs' string fields of the review, by name (see
    /// [`Lab::review_fields`]), trimmed and non-empty, e.g. `"diagram"`.
    /// Saved as the answer has them: as properties of the analysis.
    #[serde(flatten)]
    pub lab_fields: BTreeMap<String, String>,
    /// The ids of the labs the review was run with, so the UI can tell
    /// when a lab enabled since is not in it ("Review again to include the
    /// Diagrams lab").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labs: Vec<String>,
    /// The agent that produced it (its command name).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub generated_at: Option<OffsetDateTime>,
}

/// An analysis as read, with the properties it does not know.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAnalysis {
    #[serde(default, deserialize_with = "nullable")]
    overall_summary: String,
    #[serde(default, deserialize_with = "nullable")]
    groups: Vec<Group>,
    #[serde(default, deserialize_with = "nullable")]
    order: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    file_summaries: Vec<FileSummary>,
    #[serde(default, deserialize_with = "nullable")]
    diagrams: Vec<Diagram>,
    #[serde(default, deserialize_with = "nullable")]
    labs: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    agent: String,
    #[serde(default, deserialize_with = "nullable")]
    model: String,
    #[serde(default, with = "time::serde::rfc3339::option")]
    generated_at: Option<OffsetDateTime>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

impl From<RawAnalysis> for Analysis {
    fn from(r: RawAnalysis) -> Self {
        let mut a = Analysis {
            overall_summary: r.overall_summary,
            groups: r.groups,
            order: r.order,
            file_summaries: r.file_summaries,
            diagrams: r.diagrams,
            lab_fields: lab_strings(r.rest, labs::review_field_names()),
            labs: r.labs,
            agent: r.agent,
            model: r.model,
            generated_at: r.generated_at,
        };
        a.sync_lab_fields();
        a
    }
}

/// The string properties of `rest` that are lab fields of `names`.
fn lab_strings<'a>(
    rest: BTreeMap<String, Value>,
    names: impl Iterator<Item = &'a str>,
) -> BTreeMap<String, String> {
    let names: Vec<&str> = names.collect();
    let mut out: BTreeMap<String, String> = rest
        .into_iter()
        .filter(|(k, _)| names.contains(&k.as_str()))
        .filter_map(|(k, v)| match v {
            Value::String(s) => Some((k, s)),
            _ => None,
        })
        .collect();
    tidy_lab_fields(&mut out);
    out
}

/// Trims the lab fields, takes the fences off their diagrams, and drops
/// the empty ones.
fn tidy_lab_fields(fields: &mut BTreeMap<String, String>) {
    for (name, value) in fields.iter_mut() {
        *value = match labs::field(name).map(|f| f.kind) {
            Some(LabFieldKind::Mermaid) => unfence(value).to_string(),
            _ => value.trim().to_string(),
        };
    }
    fields.retain(|_, v| !v.is_empty());
}

/// What changed in a file and why. Read from `{path, summary}` (diffr's
/// first answers) or `{path, text}` (the second shape's, of a group); written as the
/// former.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSummary {
    #[serde(default, deserialize_with = "nullable")]
    pub path: String,
    #[serde(default, deserialize_with = "nullable", alias = "text")]
    pub summary: String,
}

/// A Mermaid diagram of what the change is about. Its nodes may have the
/// classes `added`, `changed` and `removed` (`:::added`) for what the
/// change adds, changes and removes, without their `classDef`s: the viewer
/// styles them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagram {
    #[serde(default, deserialize_with = "nullable")]
    pub title: String,
    #[serde(default, deserialize_with = "nullable")]
    pub caption: String,
    /// The Mermaid source, without fences.
    #[serde(default, deserialize_with = "nullable")]
    pub mermaid: String,
}

/// The most diagrams an analysis keeps.
pub const MAX_DIAGRAMS: usize = 3;

/// A set of files serving one intent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawGroup", into = "RawGroup")]
pub struct Group {
    pub key: String,
    pub label: String,
    pub summary: String,
    /// The group deserves extra care: the UI labels it "Needs care".
    pub critical: bool,
    /// The group's files, in reading order.
    pub file_paths: Vec<String>,
    /// The summaries of the group's files, as the agent answers them:
    /// [`fit`] moves them into [`Analysis::file_summaries`], so this is
    /// empty in a fit analysis. See [`Analysis::file_summary`].
    pub file_summaries: Vec<FileSummary>,
    pub file_notes: Vec<FileNote>,
    pub line_notes: Vec<LineNote>,
    /// The Mermaid diagram of the group's intent, by the [`DIAGRAMS`] lab,
    /// without fences; "" for none. The same as its
    /// [`lab_fields`](Self::lab_fields)' `"diagram"`.
    pub diagram: String,
    /// The labs' string fields of the group, by name (see
    /// [`Lab::group_fields`]), trimmed and non-empty, e.g. `"diagram"`.
    /// Saved as the answer has them: as properties of the group.
    pub lab_fields: BTreeMap<String, String>,
    /// The message to commit the group with on its own, asked of reviews
    /// of the work tree ([`Request::suggest_commits`]).
    pub commit: Option<CommitMessage>,
}

/// A group as read and written: with its lab fields among its
/// properties.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGroup {
    #[serde(default, deserialize_with = "nullable")]
    key: String,
    #[serde(default, deserialize_with = "nullable")]
    label: String,
    #[serde(default, deserialize_with = "nullable")]
    summary: String,
    #[serde(default, deserialize_with = "nullable")]
    critical: bool,
    #[serde(default, deserialize_with = "nullable")]
    file_paths: Vec<String>,
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Vec::is_empty"
    )]
    file_summaries: Vec<FileSummary>,
    #[serde(default, deserialize_with = "nullable")]
    file_notes: Vec<FileNote>,
    #[serde(default, deserialize_with = "nullable")]
    line_notes: Vec<LineNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit: Option<CommitMessage>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

impl From<RawGroup> for Group {
    fn from(r: RawGroup) -> Self {
        let mut g = Group {
            key: r.key,
            label: r.label,
            summary: r.summary,
            critical: r.critical,
            file_paths: r.file_paths,
            file_summaries: r.file_summaries,
            file_notes: r.file_notes,
            line_notes: r.line_notes,
            diagram: String::new(),
            lab_fields: lab_strings(r.rest, labs::group_field_names()),
            commit: r.commit,
        };
        g.sync_lab_fields();
        g
    }
}

impl From<Group> for RawGroup {
    fn from(g: Group) -> Self {
        let mut fields = g.lab_fields;
        if g.diagram.is_empty() {
            fields.remove(DIAGRAM_FIELD);
        } else {
            fields.insert(DIAGRAM_FIELD.to_string(), g.diagram);
        }
        RawGroup {
            key: g.key,
            label: g.label,
            summary: g.summary,
            critical: g.critical,
            file_paths: g.file_paths,
            file_summaries: g.file_summaries,
            file_notes: g.file_notes,
            line_notes: g.line_notes,
            commit: g.commit,
            rest: fields
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        }
    }
}

impl Group {
    /// A lab's string field of the group, e.g. `"diagram"`.
    pub fn lab_field(&self, name: &str) -> Option<&str> {
        self.lab_fields.get(name).map(String::as_str)
    }

    /// Tidies the lab fields and makes [`diagram`](Self::diagram) and
    /// the `"diagram"` field agree, the former winning when set.
    fn sync_lab_fields(&mut self) {
        let diagram = unfence(&self.diagram).to_string();
        if !diagram.is_empty() {
            self.lab_fields.insert(DIAGRAM_FIELD.to_string(), diagram);
        }
        tidy_lab_fields(&mut self.lab_fields);
        self.diagram = self
            .lab_fields
            .get(DIAGRAM_FIELD)
            .cloned()
            .unwrap_or_default();
    }
}

/// A note on a whole file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNote {
    #[serde(default, deserialize_with = "nullable")]
    pub path: String,
    #[serde(default, deserialize_with = "nullable")]
    pub text: String,
    /// "Needs care" in the UI; "Worth a look" when false.
    #[serde(default, deserialize_with = "nullable")]
    pub critical: bool,
}

/// A note on a line: of the new file for the side `"additions"`, of the
/// old one for `"deletions"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineNote {
    #[serde(default, deserialize_with = "nullable")]
    pub path: String,
    #[serde(default, deserialize_with = "nullable")]
    pub side: String,
    #[serde(default, deserialize_with = "nullable")]
    pub line: i64,
    #[serde(default, deserialize_with = "nullable")]
    pub text: String,
    /// "Needs care" in the UI; "Worth a look" when false.
    #[serde(default, deserialize_with = "nullable")]
    pub critical: bool,
}

impl LineNote {
    /// Whether the note is on a removed line, numbered as in the old file.
    pub fn is_old_side(&self) -> bool {
        self.side == "deletions"
    }
}

impl Analysis {
    /// Counts the groups and notes marked critical.
    pub fn critical(&self) -> usize {
        self.groups
            .iter()
            .map(|g| {
                usize::from(g.critical)
                    + g.file_notes.iter().filter(|n| n.critical).count()
                    + g.line_notes.iter().filter(|n| n.critical).count()
            })
            .sum()
    }

    /// Counts the notes, on files and on lines.
    pub fn notes(&self) -> usize {
        self.groups
            .iter()
            .map(|g| g.file_notes.len() + g.line_notes.len())
            .sum()
    }

    /// Every path of the analysis once, in the order a reviewer should
    /// read them: its `order`, then the files of its groups it leaves out,
    /// in their groups' order (all of them for an analysis without one).
    pub fn reading_order(&self) -> Vec<String> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out = Vec::new();
        let grouped = self.groups.iter().flat_map(|g| &g.file_paths);
        for p in self.order.iter().chain(grouped) {
            if seen.insert(p.as_str()) {
                out.push(p.clone());
            }
        }
        out
    }

    /// Whether the saved review contains descriptions, including the
    /// per-group summaries before normalization.
    pub fn has_file_summaries(&self) -> bool {
        self.file_summaries.iter()
            .chain(self.groups.iter().flat_map(|g| &g.file_summaries))
            .any(|s| !s.path.trim().is_empty() && !s.summary.trim().is_empty())
    }

    /// The summary of a file: what changed in it and why. From the
    /// review's summaries, else from its groups' (of an analysis not
    /// [`fit`]).
    pub fn file_summary(&self, path: &str) -> Option<&str> {
        self.file_summaries
            .iter()
            .chain(self.groups.iter().flat_map(|g| &g.file_summaries))
            .filter(|s| s.path == path && !s.summary.trim().is_empty())
            .map(|s| s.summary.as_str())
            .next()
    }

    /// A lab's string field of the review, e.g. `"diagram"`.
    pub fn lab_field(&self, name: &str) -> Option<&str> {
        self.lab_fields.get(name).map(String::as_str)
    }

    /// Tidies the lab fields, and puts the review's `"diagram"` first in
    /// [`diagrams`](Self::diagrams), unless it is there already.
    fn sync_lab_fields(&mut self) {
        tidy_lab_fields(&mut self.lab_fields);
        if let Some(src) = self.lab_fields.get(DIAGRAM_FIELD)
            && !self.diagrams.iter().any(|d| unfence(&d.mermaid) == src)
        {
            self.diagrams.insert(
                0,
                Diagram {
                    mermaid: src.clone(),
                    ..Default::default()
                },
            );
        }
    }

    /// Whether the review was run with a lab.
    pub fn has_lab(&self, id: &str) -> bool {
        self.labs.iter().any(|l| l == id)
    }
}

/// A changed file, as the prompt shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    /// The path before a rename, "" or `path` when not renamed.
    pub old_path: String,
    /// Git's letter: `A`, `M`, `D`, `R` (untracked files are `A`).
    pub status: char,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    pub generated: bool,
    /// The file's hunks as a unified diff (`@@ … @@` headers and `+`/`-`/` `
    /// lines), "" when not shown.
    pub patch: String,
}

/// A change to review.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub title: String,
    pub description: String,
    pub url: String,
    /// The commits' subjects, oldest first.
    pub commits: Vec<String>,
    pub files: Vec<File>,
    /// Where the agent runs: the repository.
    pub dir: PathBuf,
    /// Whether the files in `dir` are those of the change: false when the
    /// change is of another revision.
    pub checkout: bool,
    /// The revision of the change's new side, for `git show`.
    pub rev: String,
    /// The model to review with, "" for the agent's default.
    pub model: String,
    /// The ids of the labs enabled (see [`LABS`]); unknown ones are
    /// ignored.
    pub labs: Vec<String>,
    /// The diffr executable, which runs the labs' tools (`diffr
    /// check-diagram`); empty for the running executable.
    pub tool_bin: PathBuf,
    /// Asks for a commit message with each group, so the groups can be
    /// committed one by one: for uncommitted changes.
    pub suggest_commits: bool,
    /// Codex's fast mode (see [`Model::fast`]).
    pub fast: bool,
}

/// The analysis as a JSON Schema strict enough for structured output:
/// every property required, none other allowed. Without the labs' fields:
/// see [`schema`].
pub const SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["overallSummary", "groups", "order"],
  "properties": {
    "overallSummary": {"type": "string", "description": "The intention of the change (why over what) for a reviewer who has not read it yet, in 2-4 sentences of Markdown."},
    "groups": {
      "type": "array",
      "description": "The groups in reading order: the order a reviewer should read them.",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["key", "label", "summary", "critical", "filePaths", "fileSummaries", "fileNotes", "lineNotes"],
        "properties": {
          "key": {"type": "string", "description": "Short, stable kebab-case id."},
          "label": {"type": "string", "description": "Display name, at most 4 words."},
          "summary": {"type": "string", "description": "The intention of this group, in 1-3 sentences of Markdown."},
          "critical": {"type": "boolean", "description": "Rarely true: only when the whole group needs care (security, data loss, hard to revert)."},
          "filePaths": {"type": "array", "items": {"type": "string"}, "description": "Paths of the manifest in this group, in reading order. Every path goes into exactly one group."},
          "fileSummaries": {
            "type": "array",
            "description": "One for every path of the group.",
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["path", "text"],
              "properties": {
                "path": {"type": "string"},
                "text": {"type": "string", "description": "One plain sentence of at most 20 words: what changes in this file, and its part in the group's intent."}
              }
            }
          },
          "fileNotes": {
            "type": "array",
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["path", "text", "critical"],
              "properties": {
                "path": {"type": "string"},
                "text": {"type": "string", "description": "1-2 sentences a reviewer would otherwise have to work out."},
                "critical": {"type": "boolean", "description": "True only when it needs care; false for what is worth a look."}
              }
            }
          },
          "lineNotes": {
            "type": "array",
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["path", "side", "line", "text", "critical"],
              "properties": {
                "path": {"type": "string"},
                "side": {"type": "string", "enum": ["additions", "deletions"]},
                "line": {"type": "integer"},
                "text": {"type": "string", "description": "1-2 sentences: non-obvious logic, a subtle behavior change, a likely bug, a risk."},
                "critical": {"type": "boolean", "description": "True only when it needs care; false for what is worth a look."}
              }
            }
          }
        }
      }
    },
    "order": {"type": "array", "items": {"type": "string"}, "description": "Every path of the manifest exactly once, in the order a reviewer should read them."}
  }
}"#;

/// The analysis' JSON Schema with the fields of the labs enabled (ids of
/// [`LABS`]; unknown ones are ignored): their review fields as required
/// string properties of the answer, their group fields of each group.
pub fn schema(labs: &[String]) -> String {
    schema_for(labs, false)
}

/// [`schema`], with each group's `commit` when `commits` is set (see
/// [`Request::suggest_commits`]).
pub fn schema_for(labs: &[String], commits: bool) -> String {
    let enabled = labs::enabled(labs);
    let string_prop = |f: &LabField| {
        let description = serde_json::to_string(f.description).unwrap_or_default();
        (f.name.to_string(), format!("{{\"type\": \"string\", \"description\": {description}}}"))
    };
    let review: Vec<(String, String)> = enabled.iter().flat_map(|l| l.review_fields()).map(string_prop).collect();
    let mut group: Vec<(String, String)> = enabled.iter().flat_map(|l| l.group_fields()).map(string_prop).collect();
    if commits {
        group.push(("commit".into(), commit::MESSAGE_SCHEMA.into()));
    }
    if review.is_empty() && group.is_empty() {
        return SCHEMA.to_string();
    }
    // Adds properties (name, schema) to an object of the schema: to its
    // `required` list `required`, and to its properties before `end`, at
    // `indent`.
    fn add(s: &mut String, fields: &[(String, String)], required: &str, end: &str, indent: &str) {
        let mut names = String::new();
        let mut props = String::new();
        for (name, prop) in fields {
            let name = serde_json::to_string(name).unwrap_or_default();
            if names.contains(&name) {
                continue;
            }
            names.push_str(&format!(", {name}"));
            props.push_str(&format!(",\n{indent}{name}: {prop}"));
        }
        let req = required.strip_suffix(']').expect("a required list");
        let i = s
            .find(required)
            .expect("the schema holds the required list");
        s.replace_range(i..i + required.len(), &format!("{req}{names}]"));
        let j = s
            .rfind(end)
            .expect("the schema holds the end of the properties");
        s.insert_str(j, &props);
    }
    let mut s = SCHEMA.to_string();
    add(
        &mut s,
        &group,
        r#""required": ["key", "label", "summary", "critical", "filePaths", "fileSummaries", "fileNotes", "lineNotes"]"#,
        "\n        }\n      }\n    },\n    \"order\"",
        "          ",
    );
    add(
        &mut s,
        &review,
        r#""required": ["overallSummary", "groups", "order"]"#,
        "\n  }\n}",
        "    ",
    );
    s
}

/// The size of the diffs the prompt holds; larger ones are written to a
/// file the agent reads.
pub const INLINE_LIMIT: usize = 200_000;

/// The agent's instructions. `fenced` asks for the answer in a fenced
/// block, from agents that do not take a schema. They include those of
/// the labs of `req.labs`.
pub fn system_prompt(req: &Request, patch_path: &str, fenced: bool) -> String {
    let mut files = if req.checkout {
        "The repository is checked out in the current directory: open its files when the hunks are not enough to tell what a change is for.".to_string()
    } else if !req.rev.is_empty() {
        format!(
            "The current directory holds the repository, but checked out at another revision than this change, so its files may differ. Trust the diffs; where you can run commands, read a file as of the change with `git show {}:<path>`.",
            req.rev
        )
    } else {
        "Trust the diffs: the files of the current directory may differ from the change."
            .to_string()
    };
    if !patch_path.is_empty() {
        files = format!(
            "The diffs are too large for this message: they are in {patch_path}. Read the parts you need. {files}"
        );
    }
    let answer = if fenced {
        "Finish by answering with the JSON object matching the schema in one fenced ```json block, and nothing else."
    } else {
        "Finish by answering with the JSON object matching the schema, and nothing else."
    };
    let enabled = labs::enabled(&req.labs);
    let bin = labs::tool_bin(&req.tool_bin);
    let bin = bin.to_string_lossy();
    let mut lab_text = String::new();
    for l in &enabled {
        lab_text.push_str("\n\n");
        lab_text.push_str(&l.prompt(&bin));
    }
    let tools: Vec<String> = enabled
        .iter()
        .flat_map(|l| l.tools())
        .map(|t| format!("`\"{bin}\" {}`", t.command))
        .collect();
    let never = if tools.is_empty() {
        "Never modify anything: do not edit files, run builds or tests, or commit.".to_string()
    } else {
        format!(
            "Never modify anything: do not edit files, run builds or tests, or commit. The only commands you may run besides reading the repository are {}, as the labs above say.",
            tools.join(", ")
        )
    };
    let lab_step = if enabled.is_empty() {
        String::new()
    } else {
        "\n4. Do what the labs above ask, and check their output as they say.".to_string()
    };
    let answer_step = if enabled.is_empty() { 4 } else { 5 };
    let schema = schema_for(&req.labs, req.suggest_commits);
    let commits = if req.suggest_commits {
        format!(
            "\n\n<commits>\nThese changes are not committed yet: the author will commit them group by group. Make each group a coherent commit that stands on its own (a feature with its tests, a fix, a refactor), and give it a \"commit\" message.\n{}\n</commits>",
            commit::RULES
        )
    } else {
        String::new()
    };
    format!(
        r#"<role>
You review a code change for a colleague. You organize its changed files into review groups, so they can read the change intent by intent instead of file by file, in the order that makes it easiest to understand, and you point out what deserves their attention.
</role>

<grouping_principles>
- Group by intent, not by directory. One feature touching several modules is ONE group.
- Use the fewest groups that still separate independent intents. A typical change has 1-6 groups. A single-file group needs a reason.
- Tests, stories and fixtures belong to the feature they cover. Only tests unrelated to any feature form a group of their own.
- Every path of the manifest goes into exactly one group.
- Commit messages, when listed, hint at the author's intents. Group by the final change, not by commit.
</grouping_principles>

<reading_order>
Order the files by meaning: the order in which a reviewer should read them to understand the change, not the alphabetical order of the manifest.
- List the groups in reading order: the core change first, then what builds on it, then supporting changes, and mechanical changes (lockfiles, generated files, formatting) last.
- List each group's "filePaths" in reading order too: the core change first (where the new behavior is defined: a type, an interface, a schema, the function that changes), then its callers and wiring, then tests, then docs and configuration.
- "order" lists every path of the manifest exactly once, in reading order across the whole change. It normally walks the groups in their order; move a file ahead only when another group cannot be understood without it.
</reading_order>

<output>
- "overallSummary" explains why over what. Each group's "summary" explains its intention.
- Each group's "fileSummaries" gives every file of the group one plain sentence of at most 20 words, shown atop the file: what changes in it, and how it serves the group's intent or the other files (e.g. "Calls the new guard from the history list, as sidebar.go does for the tree."). Describe, do not judge: concerns go in notes. Do not repeat the path or start with "This file". For generated, binary or mechanical files a few words are enough.
- "fileNotes" and "lineNotes" flag what deserves the reviewer's attention, never a summary. They are sparing: add a note only where it saves the reviewer time (non-obvious logic, a subtle behavior change, a likely bug, a risk), never to explain the obvious. Most files need none: leave the arrays empty.
- A line note's "side" is "additions" for a line of the new file (added or unchanged), numbered as in the new file, and "deletions" for a removed line, numbered as in the old file, as the hunk headers count them.
- Keep code, paths and identifiers as they are.
</output>

<severity>
The reviewer sees critical groups and notes labeled "Needs care" and the other notes labeled "Worth a look". A note is already worth a look; "critical" is for what they must not skim past: a likely bug, a security issue, data loss, a breaking change of a public interface or a stored format, something hard to revert. Use it sparingly: most changes have no critical note, few more than one or two. When in doubt, it is not critical.
</severity>{commits}{lab_text}

<workflow>
1. Read the manifest and the diffs, form the groups and decide the reading order.
2. {files}
3. {never}{lab_step}
{answer_step}. {answer}
</workflow>

<schema>
{schema}
</schema>"#
    )
}

/// Describes the change: its title, commits, files and, when `inline`,
/// its diffs.
pub fn user_prompt(req: &Request, inline: bool) -> String {
    use std::fmt::Write;
    let mut b = String::new();
    if !req.title.is_empty() {
        let _ = writeln!(b, "Title: {}", req.title);
    }
    if !req.url.is_empty() {
        let _ = writeln!(b, "Link: {}", req.url);
    }
    let d = req.description.trim();
    if !d.is_empty() {
        let _ = write!(b, "\n---DESCRIPTION---\n{d}\n");
    }
    if req.commits.len() > 1 {
        let _ = write!(
            b,
            "\n---COMMITS--- ({}, oldest first)\n{}\n",
            req.commits.len(),
            req.commits.join("\n")
        );
    }
    let adds: usize = req.files.iter().map(|f| f.additions).sum();
    let dels: usize = req.files.iter().map(|f| f.deletions).sum();
    let _ = write!(
        b,
        "\n---MANIFEST--- ({} files, +{adds}/-{dels})\n{}\n",
        req.files.len(),
        manifest(&req.files)
    );
    if inline {
        let _ = write!(b, "\n---DIFFS---\n{}\n", diffs(&req.files));
    }
    b
}

/// What stands for the diffs in [`instructions`].
const DIFFS_PLACEHOLDER: &str = "(each file's hunks as a unified diff: `### <path> [<status>, +<added>/-<removed>]`, then its `@@ … @@` hunks; or, when they are too large, a note naming the file that holds them)";

/// The instructions the agent receives for a request, as Settings › AI
/// shows them: the system prompt, with the labs' additions, then the
/// description of the change with its diffs left out.
pub fn instructions(req: &Request) -> String {
    let mut user = user_prompt(req, false);
    user.push_str(&format!("\n---DIFFS---\n{DIFFS_PLACEHOLDER}\n"));
    format!("{}\n\n{user}", system_prompt(req, "", false))
}

/// The instructions the agent receives with the labs of ids enabled, for a
/// change of placeholders: [`instructions`] without a change at hand.
pub fn instructions_preview(labs: &[String]) -> String {
    let req = Request {
        checkout: true,
        labs: labs.to_vec(),
        ..Default::default()
    };
    let user = format!(
        "Title: (the change's title)\n\
         Link: (its link, for a pull request)\n\
         \n---DESCRIPTION---\n(its description, when it has one)\n\
         \n---COMMITS--- (<n>, oldest first)\n(the commits' subjects, when there are several)\n\
         \n---MANIFEST--- (<n> files, +<added>/-<removed>)\n(the changed files by directory: `<name>  <status>  +<added>/-<removed>`)\n\
         \n---DIFFS---\n{DIFFS_PLACEHOLDER}\n"
    );
    format!("{}\n\n{user}", system_prompt(&req, "", false))
}

fn renamed(f: &File) -> bool {
    !f.old_path.is_empty() && f.old_path != f.path
}

/// Lists the files by directory, with their status and counts.
pub fn manifest(files: &[File]) -> String {
    use std::fmt::Write;
    let mut sorted: Vec<&File> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut b = String::new();
    let mut dir: Option<&str> = None;
    for f in sorted {
        let (d, name) = match f.path.rfind('/') {
            Some(i) => (&f.path[..=i], &f.path[i + 1..]),
            None => ("", f.path.as_str()),
        };
        if dir != Some(d) {
            dir = Some(d);
            if !d.is_empty() {
                b.push_str(d);
                b.push('\n');
            }
        }
        if !d.is_empty() {
            b.push_str("  ");
        }
        let _ = write!(b, "{name}  {}  +{}/-{}", f.status, f.additions, f.deletions);
        if renamed(f) {
            let _ = write!(b, "  (from {})", f.old_path);
        }
        if f.generated {
            b.push_str("  [generated]");
        } else if f.binary {
            b.push_str("  [binary]");
        }
        b.push('\n');
    }
    if b.ends_with('\n') {
        b.pop();
    }
    b
}

/// Renders the files' hunks, leaving out those of generated and binary
/// files, which a reviewer does not read line by line.
pub fn diffs(files: &[File]) -> String {
    let parts: Vec<String> = files
        .iter()
        .map(|f| {
            let mut header = format!("### {}", f.path);
            if renamed(f) {
                header.push_str(&format!(" (renamed from {})", f.old_path));
            }
            header.push_str(&format!(
                " [{}, +{}/-{}]",
                f.status, f.additions, f.deletions
            ));
            if f.binary {
                format!("{header}\n(binary file, no diff shown)")
            } else if f.generated {
                format!("{header}\n(generated file, diff omitted)")
            } else if f.patch.is_empty() {
                format!("{header}\n(no lines to show)")
            } else {
                format!("{header}\n{}", f.patch.trim_end_matches('\n'))
            }
        })
        .collect();
    parts.join("\n\n")
}

/// Reads an agent's answer, the JSON object alone or fenced, and fits it
/// to the files (see [`fit`]).
pub fn parse(answer: &str, files: &[File]) -> Result<Analysis, Error> {
    let text = extract_json(answer);
    if text.is_empty() {
        return Err(Error::Failed("the agent's answer holds no analysis".into()));
    }
    let a: Analysis = serde_json::from_str(text)
        .map_err(|e| Error::Failed(format!("the agent's answer is not an analysis: {e}")))?;
    fit(a, files)
}

/// Keeps every file in one group exactly, and the notes on files of the
/// change: paths the agent invented go, and those it left out gather in a
/// group of their own, "other-changes", at the end. Likewise `order`
/// keeps every file once, those it left out following in their groups'
/// order; `fileSummaries` gathers the groups' and the review's, one per
/// file of the change at most (a group's first); the labs' fields are
/// trimmed, the empty ones dropped; and `diagrams` keeps the review's
/// `diagram` first, then the first others that have a source,
/// [`MAX_DIAGRAMS`] in all.
pub fn fit(mut a: Analysis, files: &[File]) -> Result<Analysis, Error> {
    let known: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    // Finds a file of the change by its path, also as an absolute one, as
    // OpenCode's explore agent gives them.
    let clean = |p: &str| -> String {
        let p = p.trim();
        let p = p.strip_prefix("./").unwrap_or(p);
        if known.contains(p) || !p.starts_with('/') {
            return p.to_string();
        }
        let mut found: Option<&str> = None;
        for f in files {
            let k = f.path.as_str();
            if p.ends_with(&format!("/{k}")) && found.is_none_or(|x| k.len() > x.len()) {
                found = Some(k);
            }
        }
        found.unwrap_or(p).to_string()
    };
    let mut placed: HashSet<String> = HashSet::new();
    let mut groups: Vec<Group> = Vec::new();
    // The summaries of the groups, then of the review.
    let mut summaries: Vec<FileSummary> = Vec::new();
    for mut g in std::mem::take(&mut a.groups) {
        summaries.append(&mut g.file_summaries);
        g.sync_lab_fields();
        g.commit = g.commit.take().map(CommitMessage::tidy).filter(|m| !m.is_empty());
        let mut paths = Vec::new();
        for p in &g.file_paths {
            let p = clean(p);
            if known.contains(p.as_str()) && !placed.contains(&p) {
                placed.insert(p.clone());
                paths.push(p);
            }
        }
        for n in &mut g.file_notes {
            n.path = clean(&n.path);
        }
        for n in &mut g.line_notes {
            n.path = clean(&n.path);
        }
        g.file_notes
            .retain(|n| known.contains(n.path.as_str()) && !n.text.trim().is_empty());
        g.line_notes.retain(|n| {
            known.contains(n.path.as_str())
                && n.line > 0
                && !n.text.trim().is_empty()
                && (n.side == "additions" || n.side == "deletions")
        });
        if paths.is_empty() {
            continue;
        }
        g.file_paths = paths;
        if g.key.is_empty() {
            g.key = format!("group-{}", groups.len() + 1);
        }
        if g.label.trim().is_empty() {
            g.label = "Changes".into();
        }
        groups.push(g);
    }
    if groups.is_empty() {
        return Err(Error::Failed("the agent grouped none of the files".into()));
    }
    let rest: Vec<String> = files
        .iter()
        .filter(|f| !placed.contains(&f.path))
        .map(|f| f.path.clone())
        .collect();
    if !rest.is_empty() {
        groups.push(Group {
            key: "other-changes".into(),
            label: "Other changes".into(),
            summary: "Files the analysis left out.".into(),
            file_paths: rest,
            ..Default::default()
        });
    }
    a.groups = groups;

    let mut ordered: HashSet<String> = HashSet::new();
    let mut order = Vec::new();
    let grouped: Vec<String> = a.groups.iter().flat_map(|g| g.file_paths.clone()).collect();
    for p in std::mem::take(&mut a.order)
        .iter()
        .map(|p| clean(p))
        .chain(grouped)
    {
        if known.contains(p.as_str()) && ordered.insert(p.clone()) {
            order.push(p);
        }
    }
    a.order = order;

    summaries.append(&mut a.file_summaries);
    let mut summarized: HashSet<String> = HashSet::new();
    a.file_summaries = summaries
        .into_iter()
        .filter_map(|s| {
            let path = clean(&s.path);
            let summary = s.summary.trim().to_string();
            (known.contains(path.as_str())
                && !summary.is_empty()
                && summarized.insert(path.clone()))
            .then_some(FileSummary { path, summary })
        })
        .collect();

    a.sync_lab_fields();
    a.diagrams = std::mem::take(&mut a.diagrams)
        .into_iter()
        .filter_map(|mut d| {
            d.mermaid = unfence(&d.mermaid).to_string();
            d.title = d.title.trim().to_string();
            d.caption = d.caption.trim().to_string();
            (!d.mermaid.is_empty()).then_some(d)
        })
        .take(MAX_DIAGRAMS)
        .collect();
    Ok(a)
}

/// A diagram's source without the ``` fences an agent may have put
/// around it.
fn unfence(s: &str) -> &str {
    let t = s.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    let rest = rest.strip_suffix("```").unwrap_or(rest);
    // The fence's language, as ```mermaid.
    match rest.find('\n') {
        Some(i) if !rest[..i].trim().contains(' ') => rest[i + 1..].trim(),
        _ => rest.trim(),
    }
}

/// The last fenced JSON block of a text, else what lies between its first
/// `{` and its last `}`, else "".
pub fn extract_json(s: &str) -> &str {
    if let Some(i) = s.rfind("```json") {
        let rest = &s[i + "```json".len()..];
        if let Some(j) = rest.find("```") {
            return rest[..j].trim();
        }
    }
    match (s.find('{'), s.rfind('}')) {
        (Some(i), Some(j)) if j > i => &s[i..=j],
        _ => "",
    }
}

/// The groups of an analysis by path: the index of the first group of
/// each file.
pub fn group_of_paths(a: &Analysis) -> HashMap<&str, usize> {
    let mut out = HashMap::new();
    for (gi, g) in a.groups.iter().enumerate() {
        for p in &g.file_paths {
            out.entry(p.as_str()).or_insert(gi);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn files() -> Vec<File> {
        vec![
            File {
                path: "src/greet.go".into(),
                status: 'M',
                additions: 2,
                deletions: 1,
                patch: "@@ -1,3 +1,4 @@ func greet\n-\treturn \"Hello\"\n+\treturn \"Hi\"\n+\t// louder\n".into(),
                ..Default::default()
            },
            File {
                path: "src/greet_test.go".into(),
                status: 'A',
                additions: 5,
                patch: "@@ -0,0 +1,5 @@\n+package src\n".into(),
                ..Default::default()
            },
            File {
                path: "pnpm-lock.yaml".into(),
                status: 'M',
                additions: 900,
                deletions: 800,
                generated: true,
                patch: "@@ -1 +1 @@\n-a\n+b\n".into(),
                ..Default::default()
            },
            File {
                path: "logo.png".into(),
                status: 'A',
                binary: true,
                ..Default::default()
            },
        ]
    }

    pub(crate) const ANSWER: &str = r#"{"overallSummary":"Greets louder.","groups":[
 {"key":"greeting","label":"Greeting","summary":"Shorter greeting.","critical":false,
  "filePaths":["src/greet.go","./src/greet_test.go","invented.go"],
  "fileNotes":[{"path":"src/greet.go","text":"Changes the public output.","critical":true},{"path":"nope.go","text":"x","critical":false}],
  "lineNotes":[{"path":"src/greet.go","side":"additions","line":2,"text":"Callers may match on Hello.","critical":false},{"path":"src/greet.go","side":"sideways","line":2,"text":"x","critical":false}]},
 {"key":"deps","label":"Lockfile","summary":"","critical":false,"filePaths":["pnpm-lock.yaml","src/greet.go"],"fileNotes":[],"lineNotes":[]}
]}"#;

    #[test]
    fn parse_fits_the_files() {
        let a = parse(
            &format!("Here it is:\n```json\n{ANSWER}\n```\nDone."),
            &files(),
        )
        .unwrap();
        assert_eq!(a.overall_summary, "Greets louder.");
        assert_eq!(a.groups.len(), 3);
        let g = &a.groups[0];
        assert_eq!(g.file_paths, ["src/greet.go", "src/greet_test.go"]);
        assert_eq!(g.file_notes.len(), 1);
        assert_eq!(g.line_notes.len(), 1);
        // A path in two groups stays in the first.
        assert_eq!(a.groups[1].file_paths, ["pnpm-lock.yaml"]);
        // The files left out gather at the end.
        let last = &a.groups[2];
        assert_eq!(last.key, "other-changes");
        assert_eq!(last.file_paths, ["logo.png"]);
        assert_eq!(a.critical(), 1);
        assert!(parse("I could not do it.", &files()).is_err());
    }

    #[test]
    fn fit_absolute_paths_and_nulls() {
        let answer = r#"{"overallSummary":null,"groups":[{"key":"","label":" ","filePaths":["/repo/src/greet.go"],"fileNotes":null,"lineNotes":[{"path":"/repo/src/greet.go","side":"deletions","line":1,"text":"Old.","critical":true}]}]}"#;
        let a = parse(answer, &files()).unwrap();
        assert_eq!(a.groups[0].key, "group-1");
        assert_eq!(a.groups[0].label, "Changes");
        assert_eq!(a.groups[0].file_paths, ["src/greet.go"]);
        assert!(a.groups[0].line_notes[0].is_old_side());
        assert_eq!(a.groups[1].file_paths.len(), 3);
    }

    #[test]
    fn prompts() {
        let req = Request {
            title: "Greet".into(),
            description: "Shorter.".into(),
            commits: vec!["a1 First".into(), "b2 Second".into()],
            files: files(),
            rev: "abc123".into(),
            ..Default::default()
        };
        let user = user_prompt(&req, true);
        for want in [
            "Title: Greet",
            "---DESCRIPTION---\nShorter.",
            "---COMMITS--- (2, oldest first)",
            "src/\n  greet.go  M  +2/-1",
            "pnpm-lock.yaml  M  +900/-800  [generated]",
            "### src/greet.go [M, +2/-1]\n@@ -1,3 +1,4 @@",
            "(generated file, diff omitted)",
            "(binary file, no diff shown)",
        ] {
            assert!(user.contains(want), "no {want:?} in\n{user}");
        }
        let sys = system_prompt(&req, "", true);
        assert!(sys.contains("git show abc123:<path>"));
        assert!(sys.contains("fenced ```json block"));
        assert!(sys.contains("\"lineNotes\""));
        let _: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    }

    #[test]
    fn prompt_reading_order_summaries_and_severity() {
        let req = Request {
            files: files(),
            ..Default::default()
        };
        let sys = system_prompt(&req, "", false);
        for want in [
            "<reading_order>",
            "List the groups in reading order",
            "each group's \"filePaths\" in reading order",
            "\"order\" lists every path of the manifest exactly once",
            "Each group's \"fileSummaries\" gives every file of the group one plain sentence of at most 20 words",
            "(e.g. \"Calls the new guard from the history list, as sidebar.go does for the tree.\")",
            "Describe, do not judge: concerns go in notes.",
            "flag what deserves the reviewer's attention, never a summary",
            "\"Needs care\"",
            "\"Worth a look\"",
            "Use it sparingly",
            "\"required\": [\"overallSummary\", \"groups\", \"order\"]",
            "\"required\": [\"key\", \"label\", \"summary\", \"critical\", \"filePaths\", \"fileSummaries\", \"fileNotes\", \"lineNotes\"]",
            "\"required\": [\"path\", \"text\"]",
            "<severity>",
            "4. Finish by answering",
        ] {
            assert!(sys.contains(want), "no {want:?} in\n{sys}");
        }
        // Without labs, nothing of them.
        for not in [
            "check-diagram",
            "<lab ",
            "\"diagrams\"",
            "\"diagram\"",
            ":::added",
        ] {
            assert!(!sys.contains(not), "{not:?} in\n{sys}");
        }
        assert_eq!(schema(&[]), SCHEMA);
        assert_eq!(schema(&["nope".into()]), SCHEMA);
    }

    #[test]
    fn prompt_with_the_diagrams_lab() {
        let req = Request {
            files: files(),
            labs: vec!["diagrams".into()],
            tool_bin: "/opt/diffr/bin/diffr".into(),
            ..Default::default()
        };
        let sys = system_prompt(&req, "", false);
        for want in [
            "<lab name=\"diagrams\">",
            "The review's \"diagram\" shows the change as a whole; a group's \"diagram\" shows that group's intent.",
            "A small change needs one diagram at most",
            "flowchart (LR or TD), sequenceDiagram, stateDiagram-v2, classDiagram or erDiagram",
            "at most about 12 nodes or 10 messages",
            "`:::added`",
            "`:::changed`",
            "`:::removed`",
            "Do not write classDef, style or %%{init}%% lines",
            "quote node labels holding punctuation",
            "NEVER quote the text of a sequence diagram's messages or notes",
            "No HTML and no click handlers.",
            "Check EVERY diagram",
            "run `\"/opt/diffr/bin/diffr\" check-diagram` with the diagram's Mermaid source on stdin",
            "\"/opt/diffr/bin/diffr\" check-diagram <<'MERMAID'",
            "(`\"/opt/diffr/bin/diffr\" --check-diagram` is the same command.)",
            "The only commands you may run besides reading the repository are `\"/opt/diffr/bin/diffr\" check-diagram`",
            "<reading_order>",
            "<severity>",
            "5. Finish by answering",
            "\"diagram\": {\"type\": \"string\"",
        ] {
            assert!(sys.contains(want), "no {want:?} in\n{sys}");
        }
        let v: serde_json::Value = serde_json::from_str(&schema(&req.labs)).unwrap();
        let strs = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            strs(&v["required"]),
            ["overallSummary", "groups", "order", "diagram"]
        );
        assert_eq!(v["properties"]["diagram"]["type"], "string");
        let items = &v["properties"]["groups"]["items"];
        assert_eq!(
            strs(&items["required"]),
            [
                "key",
                "label",
                "summary",
                "critical",
                "filePaths",
                "fileSummaries",
                "fileNotes",
                "lineNotes",
                "diagram"
            ]
        );
        assert_eq!(items["properties"]["diagram"]["type"], "string");
        assert_eq!(
            items["properties"]["fileSummaries"]["items"]["required"],
            serde_json::json!(["path", "text"])
        );
        assert!(v["properties"].get("diagrams").is_none());
        assert!(v["properties"].get("fileSummaries").is_none());
        // Every object of the schema allows no other property, and
        // requires all of its own, as structured output wants.
        fn strict(v: &serde_json::Value) {
            if let Some(props) = v.get("properties").and_then(|p| p.as_object()) {
                assert_eq!(v["additionalProperties"], false);
                let req = v["required"].as_array().unwrap();
                assert_eq!(req.len(), props.len(), "{v}");
                props.values().for_each(strict);
            }
            if let Some(items) = v.get("items") {
                strict(items);
            }
        }
        strict(&v);
        strict(&serde_json::from_str(SCHEMA).unwrap());
    }

    #[test]
    fn commit_suggestions() {
        let req = Request {
            files: files(),
            suggest_commits: true,
            labs: vec!["diagrams".into()],
            ..Default::default()
        };
        let sys = system_prompt(&req, "", false);
        assert!(sys.contains("<commits>") && sys.contains("Conventional Commits"), "{sys}");
        assert!(!system_prompt(&Request::default(), "", false).contains("<commits>"));
        let v: serde_json::Value = serde_json::from_str(&schema_for(&req.labs, true)).unwrap();
        let items = &v["properties"]["groups"]["items"];
        let required: Vec<&str> = items["required"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        assert!(required.ends_with(&["diagram", "commit"]), "{required:?}");
        assert_eq!(items["properties"]["commit"]["required"].as_array().unwrap().len(), 5);

        let answer = r#"{"overallSummary":"x","groups":[
 {"key":"a","label":"A","filePaths":["src/greet.go"],"commit":{"type":"Fix","scope":"","description":"Shorten the greeting.","body":"","breaking":""}},
 {"key":"b","label":"B","filePaths":["pnpm-lock.yaml"],"commit":{"type":"chore","scope":"","description":"  ","body":"","breaking":""}}]}"#;
        let a = parse(answer, &files()).unwrap();
        assert_eq!(a.groups[0].commit.as_ref().unwrap().subject(), "fix: shorten the greeting");
        assert!(a.groups[1].commit.is_none());
        let back: Analysis = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn instructions_views() {
        let req = Request {
            title: "Greet".into(),
            files: files(),
            checkout: true,
            labs: vec!["diagrams".into()],
            tool_bin: "/x/diffr".into(),
            ..Default::default()
        };
        let i = instructions(&req);
        assert!(i.starts_with("<role>"));
        assert!(i.contains("\"/x/diffr\" check-diagram"));
        assert!(i.contains("</schema>\n\nTitle: Greet\n"));
        assert!(i.contains("src/\n  greet.go  M  +2/-1"));
        assert!(i.contains("---DIFFS---\n(each file's hunks"));
        // The diffs themselves are left out.
        assert!(!i.contains("return \"Hi\""));

        let plain = instructions_preview(&[]);
        assert!(plain.contains("<reading_order>"));
        assert!(plain.contains("---MANIFEST--- (<n> files"));
        assert!(!plain.contains("check-diagram"));
        let labs = instructions_preview(&["diagrams".into()]);
        assert!(labs.contains("check-diagram <<'MERMAID'"));
        assert!(labs.contains("\"diagram\": {"));
    }

    #[test]
    fn fit_repairs_order_summaries_and_diagrams() {
        let answer = r#"{"overallSummary":"x","groups":[
 {"key":"a","label":"A","summary":"","critical":false,"filePaths":["src/greet_test.go","src/greet.go"],"fileNotes":[],"lineNotes":[]},
 {"key":"b","label":"B","summary":"","critical":false,"filePaths":["pnpm-lock.yaml"],"fileNotes":[],"lineNotes":[]}],
 "order":["src/greet.go","invented.go","./src/greet.go","/repo/pnpm-lock.yaml"],
 "fileSummaries":[{"path":"src/greet.go","summary":" Shortens the greeting. "},{"path":"src/greet.go","summary":"dup"},{"path":"nope.go","summary":"x"},{"path":"logo.png","summary":""}],
 "diagrams":[{"title":"Flow","caption":"c","mermaid":"```mermaid\nflowchart LR\n  a:::added --> b\n```"},{"title":"Empty","caption":"","mermaid":"  "},{"title":"2","caption":"","mermaid":"graph TD; a-->b"},{"title":"3","caption":"","mermaid":"graph TD; a-->b"},{"title":"4","caption":"","mermaid":"graph TD; a-->b"}]}"#;
        let a = parse(answer, &files()).unwrap();
        // Unknown and repeated paths go; those left out follow in their
        // groups' order, the files of no group last.
        assert_eq!(
            a.order,
            [
                "src/greet.go",
                "pnpm-lock.yaml",
                "src/greet_test.go",
                "logo.png"
            ]
        );
        assert_eq!(a.reading_order(), a.order);
        assert_eq!(a.file_summaries.len(), 1);
        assert_eq!(
            a.file_summary("src/greet.go"),
            Some("Shortens the greeting.")
        );
        assert_eq!(a.file_summary("logo.png"), None);
        assert_eq!(a.diagrams.len(), MAX_DIAGRAMS);
        assert_eq!(a.diagrams[0].mermaid, "flowchart LR\n  a:::added --> b");
        assert_eq!(a.diagrams[1].title, "2");

        // No order at all: the groups'.
        let a = parse(ANSWER, &files()).unwrap();
        assert_eq!(
            a.order,
            [
                "src/greet.go",
                "src/greet_test.go",
                "pnpm-lock.yaml",
                "logo.png"
            ]
        );
        assert_eq!(unfence("```\ngraph TD\n```"), "graph TD");
        assert_eq!(unfence("graph TD"), "graph TD");
    }

    #[test]
    fn old_analyses_load() {
        // As first saved: no order, summaries, diagrams or labs.
        let old = r#"{"overallSummary":"Greets.","groups":[
 {"key":"b","label":"B","summary":"","critical":false,"filePaths":["b.go","a.go"],"fileNotes":null,"lineNotes":[]},
 {"key":"a","label":"A","summary":"","critical":true,"filePaths":["c.go","a.go"],"fileNotes":[],"lineNotes":null}],
 "agent":"codex","generatedAt":"2025-06-01T10:00:00Z"}"#;
        let a: Analysis = serde_json::from_str(old).unwrap();
        assert!(a.order.is_empty() && a.file_summaries.is_empty());
        assert!(a.diagrams.is_empty() && a.labs.is_empty());
        assert_eq!(a.reading_order(), ["b.go", "a.go", "c.go"]);
        assert_eq!(a.file_summary("a.go"), None);
        assert!(!a.has_lab(DIAGRAMS));
        let s = serde_json::to_string(&a).unwrap();
        for not in ["order", "fileSummaries", "diagrams", "labs"] {
            assert!(!s.contains(not), "{not} in {s}");
        }
        // Nulls, as Go writes empty slices.
        let a: Analysis = serde_json::from_str(
            r#"{"overallSummary":"","groups":[],"order":null,"fileSummaries":null,"diagrams":null,"labs":null}"#,
        )
        .unwrap();
        assert!(a.order.is_empty() && a.labs.is_empty());
    }

    #[test]
    fn new_fields_round_trip() {
        let mut a = parse(ANSWER, &files()).unwrap();
        a.file_summaries = vec![FileSummary {
            path: "src/greet.go".into(),
            summary: "Shortens it.".into(),
        }];
        a.diagrams = vec![Diagram {
            title: "Flow".into(),
            caption: "How it greets.".into(),
            mermaid: "flowchart LR\n  a:::changed --> b".into(),
        }];
        a.labs = vec![DIAGRAMS.into()];
        let s = serde_json::to_string(&a).unwrap();
        for want in [
            "\"order\":[\"src/greet.go\"",
            "\"fileSummaries\":[{\"path\":\"src/greet.go\",\"summary\":\"Shortens it.\"}]",
            "\"diagrams\":[{\"title\":\"Flow\",\"caption\":\"How it greets.\",\"mermaid\":",
            "\"labs\":[\"diagrams\"]",
        ] {
            assert!(s.contains(want), "no {want} in {s}");
        }
        let b: Analysis = serde_json::from_str(&s).unwrap();
        assert_eq!(a, b);
        assert!(b.has_lab(DIAGRAMS));
    }

    /// An answer of the second shape: summaries per group, `{path, text}`,
    /// and the Diagrams lab's string fields.
    const GROUPED_ANSWER: &str = r#"{"overallSummary":"Greets louder.","order":["src/greet.go"],
 "diagram":"```mermaid\nflowchart LR\n  greet:::changed --> out\n```",
 "future":7, "notALab":"ignored",
 "groups":[
 {"key":"greeting","label":"Greeting","summary":"Shorter greeting.","critical":false,
  "filePaths":["src/greet.go","src/greet_test.go"],
  "fileSummaries":[{"path":"src/greet.go","text":" Shortens the greeting. "},{"path":"/repo/src/greet_test.go","text":"Checks it."},{"path":"nope.go","text":"x"}],
  "fileNotes":[],"lineNotes":[],
  "diagram":"sequenceDiagram\n  A->>B: hi"},
 {"key":"deps","label":"Lockfile","summary":"","critical":false,"filePaths":["pnpm-lock.yaml","logo.png"],
  "fileSummaries":[{"path":"pnpm-lock.yaml","text":"Bumps."},{"path":"src/greet.go","text":"dup"}],
  "fileNotes":[],"lineNotes":[],"diagram":"  "}
]}"#;

    #[test]
    fn cached_grouped_summaries_do_not_request_another_review() {
        let a: Analysis = serde_json::from_str(GROUPED_ANSWER).unwrap();
        assert!(a.file_summaries.is_empty());
        assert!(a.file_summary("src/greet.go").is_some());
        assert!(a.has_file_summaries(), "Grouped summaries must count before normalization");
        assert!(!Analysis::default().has_file_summaries());
    }

    #[test]
    fn grouped_shape_parses() {
        let a = parse(GROUPED_ANSWER, &files()).unwrap();
        // The groups' summaries move to the review's, the first of a path
        // winning, paths fit to the files.
        assert_eq!(
            a.file_summaries,
            [
                FileSummary {
                    path: "src/greet.go".into(),
                    summary: "Shortens the greeting.".into()
                },
                FileSummary {
                    path: "src/greet_test.go".into(),
                    summary: "Checks it.".into()
                },
                FileSummary {
                    path: "pnpm-lock.yaml".into(),
                    summary: "Bumps.".into()
                },
            ]
        );
        assert!(a.groups.iter().all(|g| g.file_summaries.is_empty()));
        assert_eq!(
            a.file_summary("src/greet.go"),
            Some("Shortens the greeting.")
        );
        // The review's diagram, unfenced, first of the diagrams and among
        // the lab fields; only known lab fields, only strings.
        let src = "flowchart LR\n  greet:::changed --> out";
        assert_eq!(a.diagrams.len(), 1);
        assert_eq!(a.diagrams[0].mermaid, src);
        assert!(a.diagrams[0].title.is_empty());
        assert_eq!(a.lab_field(DIAGRAM_FIELD), Some(src));
        assert_eq!(a.lab_fields.len(), 1);
        // The groups' diagrams; an empty one is none.
        assert_eq!(a.groups[0].diagram, "sequenceDiagram\n  A->>B: hi");
        assert_eq!(
            a.groups[0].lab_field("diagram"),
            Some("sequenceDiagram\n  A->>B: hi")
        );
        assert_eq!(a.groups[1].diagram, "");
        assert!(a.groups[1].lab_fields.is_empty());
        assert_eq!(a.order[0], "src/greet.go");
    }

    #[test]
    fn both_shapes_at_once() {
        // diffr's first shape and its second in one answer: a group's
        // summary before the review's, the review's diagram before the
        // others.
        let answer = r#"{"overallSummary":"x","diagram":"graph TD; z-->y",
 "fileSummaries":[{"path":"src/greet.go","summary":"From the review."},{"path":"logo.png","summary":"A logo."}],
 "diagrams":[{"title":"Old","caption":"c","mermaid":"graph TD; a-->b"},{"title":"Same","caption":"","mermaid":"graph TD; z-->y"}],
 "groups":[{"key":"a","label":"A","filePaths":["src/greet.go"],"fileSummaries":[{"path":"src/greet.go","text":"From the group."}]}]}"#;
        let a = parse(answer, &files()).unwrap();
        assert_eq!(a.file_summary("src/greet.go"), Some("From the group."));
        assert_eq!(a.file_summary("logo.png"), Some("A logo."));
        let sources: Vec<&str> = a.diagrams.iter().map(|d| d.mermaid.as_str()).collect();
        // Already among them: not added twice.
        assert_eq!(sources, ["graph TD; a-->b", "graph TD; z-->y"]);
    }

    #[test]
    fn lab_fields_round_trip() {
        let a = parse(GROUPED_ANSWER, &files()).unwrap();
        let s = serde_json::to_string(&a).unwrap();
        // Saved as answered: properties of the review and of the groups.
        for want in [
            "\"diagram\":\"flowchart LR\\n  greet:::changed --> out\"",
            "\"diagram\":\"sequenceDiagram\\n  A->>B: hi\"",
            "\"fileSummaries\":[{\"path\":\"src/greet.go\",\"summary\":",
        ] {
            assert!(s.contains(want), "no {want} in {s}");
        }
        assert_eq!(s.matches("\"diagram\":").count(), 2, "{s}");
        assert!(!s.contains("labFields") && !s.contains("notALab"), "{s}");
        let b: Analysis = serde_json::from_str(&s).unwrap();
        assert_eq!(a, b);

        // Group::diagram set alone is saved, and lab fields of a future lab
        // set by hand survive as properties.
        let mut g = b.groups[1].clone();
        g.diagram = "graph TD; a-->b".into();
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains("\"diagram\":\"graph TD; a-->b\""), "{s}");
        let back: Group = serde_json::from_str(&s).unwrap();
        assert_eq!(back.diagram, "graph TD; a-->b");
        assert_eq!(back.lab_field("diagram"), Some("graph TD; a-->b"));
        g.diagram.clear();
        let s = serde_json::to_string(&g).unwrap();
        assert!(!s.contains("diagram"), "{s}");
    }

    #[test]
    fn diffr_saved_analyses_load() {
        // As diffr saved them before the second shape: summaries of the
        // review, titled diagrams, labs.
        let saved = r#"{"overallSummary":"x","groups":[{"key":"a","label":"A","summary":"","critical":false,"filePaths":["a.go"],"fileNotes":[],"lineNotes":[]}],
 "order":["a.go"],"fileSummaries":[{"path":"a.go","summary":"Adds a."}],
 "diagrams":[{"title":"Flow","caption":"c","mermaid":"graph TD; a-->b"}],"labs":["diagrams"],
 "agent":"claude","generatedAt":"2025-06-01T10:00:00Z"}"#;
        let a: Analysis = serde_json::from_str(saved).unwrap();
        assert_eq!(a.file_summary("a.go"), Some("Adds a."));
        assert_eq!(a.diagrams.len(), 1);
        assert_eq!(a.diagrams[0].title, "Flow");
        assert!(a.lab_fields.is_empty());
        assert_eq!(a.groups[0].diagram, "");
        assert!(a.groups[0].lab_fields.is_empty());
        assert!(a.has_lab(DIAGRAMS));
        let b: Analysis = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, b);

        // As cache.rs saves them, wrapped, from an answer not fit: the
        // groups' summaries still read.
        let unfit = r#"{"analysis":{"overallSummary":"x","groups":[{"key":"a","label":"A","filePaths":["a.go"],
 "fileSummaries":[{"path":"a.go","text":"Adds a."}],"diagram":"graph TD; a-->b"}]},"fingerprints":{"a.go":"1"}}"#;
        let saved: cache::SavedAnalysis = serde_json::from_str(unfit).unwrap();
        let a = saved.analysis.unwrap();
        assert_eq!(a.file_summary("a.go"), Some("Adds a."));
        assert_eq!(a.groups[0].diagram, "graph TD; a-->b");
    }

    #[test]
    fn extract() {
        assert_eq!(
            extract_json("a ```json\n{1}\n``` b ```json\n{2}\n```"),
            "{2}"
        );
        assert_eq!(extract_json("so {\"a\":{}} ok"), "{\"a\":{}}");
        assert_eq!(extract_json("none"), "");
    }

    #[test]
    fn analysis_round_trips() {
        let mut a = parse(ANSWER, &files()).unwrap();
        a.agent = "claude".into();
        a.generated_at = Some(OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap());
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains("\"generatedAt\":\"2023-11-14T22:13:20Z\""));
        let b: Analysis = serde_json::from_str(&s).unwrap();
        assert_eq!(a, b);
    }
}
