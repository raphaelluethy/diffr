//! Labs: experimental features of the review, which the user turns on one
//! by one (Settings › AI, the `labs` setting). Each lab may add
//!
//! - text to the agent's instructions ([`Lab::prompt`]),
//! - fields to its answer: strings, of the review as a whole
//!   ([`Lab::review_fields`]) or of each group ([`Lab::group_fields`]), which
//!   [`Analysis::lab_fields`](crate::Analysis::lab_fields) and
//!   [`Group::lab_fields`](crate::Group::lab_fields) collect, so that a new
//!   lab needs no new field in the analysis,
//! - tools: subcommands of the diffr executable ([`Request::tool_bin`]) the
//!   agent may run although it is otherwise read-only ([`Lab::tools`]).
//!   How each agent is allowed them is in `run.rs`.
//!
//! A new lab is an entry in [`LABS`] and arms in the `match`es of its
//! methods.
//!
//! [`Request::tool_bin`]: crate::Request::tool_bin

use std::path::{Path, PathBuf};

/// An experimental feature of the review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lab {
    /// Its id in the settings and in [`Analysis::labs`](crate::Analysis::labs).
    pub id: &'static str,
    pub name: &'static str,
    /// One sentence for the settings.
    pub description: &'static str,
}

/// The id of the lab whose agent draws Mermaid diagrams of the change.
pub const DIAGRAMS: &str = "diagrams";

/// The name of the [`DIAGRAMS`] lab's field, of the review and of each
/// group: a Mermaid source, or "".
pub const DIAGRAM_FIELD: &str = "diagram";

/// The labs, in the order the settings list them.
pub const LABS: &[Lab] = &[Lab {
    id: DIAGRAMS,
    name: "Diagrams",
    description: "The agent draws Mermaid diagrams of what the change does: the flow it adds, the parts it connects, the states it goes through. They show in the review's card and atop each group.",
}];

/// The lab of an id.
pub fn lab(id: &str) -> Option<&'static Lab> {
    LABS.iter().find(|l| l.id == id)
}

/// The labs of ids, known ones only, each once, in [`LABS`]' order.
pub fn enabled(ids: &[String]) -> Vec<&'static Lab> {
    LABS.iter()
        .filter(|l| ids.iter().any(|id| id.trim() == l.id))
        .collect()
}

/// The ids of the labs of ids that are known, each once, in [`LABS`]'
/// order: what an analysis records it was run with.
pub fn enabled_ids(ids: &[String]) -> Vec<String> {
    enabled(ids).iter().map(|l| l.id.to_string()).collect()
}

/// How the app shows a lab field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LabFieldKind {
    /// Markdown text.
    Markdown,
    /// A Mermaid diagram's source (see `diffr_diagram`).
    Mermaid,
}

/// A string field a lab adds to the agent's answer, of the review or of
/// each group. Required by the schema; "" when the agent has nothing to
/// put in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LabField {
    /// Its name in the answer's JSON (and in the saved analysis).
    pub name: &'static str,
    /// Its description in the answer's JSON Schema.
    pub description: &'static str,
    pub kind: LabFieldKind,
}

/// The names of the review fields of every lab, known or not enabled.
pub fn review_field_names() -> impl Iterator<Item = &'static str> {
    LABS.iter().flat_map(|l| l.review_fields()).map(|f| f.name)
}

/// The names of the group fields of every lab, known or not enabled.
pub fn group_field_names() -> impl Iterator<Item = &'static str> {
    LABS.iter().flat_map(|l| l.group_fields()).map(|f| f.name)
}

/// The field of a name, of any lab's review or group fields.
pub fn field(name: &str) -> Option<&'static LabField> {
    LABS.iter()
        .flat_map(|l| l.review_fields().iter().chain(l.group_fields()))
        .find(|f| f.name == name)
}

/// A command a lab lets the agent run: `<tool_bin> <command>` (or
/// `<tool_bin> <flag>`, the same), its input on stdin. It must only read:
/// the agents run it within their read-only sandboxes (Codex) or with
/// nothing else allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabTool {
    /// The subcommand of the diffr executable, e.g. `check-diagram`.
    pub command: &'static str,
    /// The same command as a flag, e.g. `--check-diagram`, as agents
    /// may call it.
    pub flag: &'static str,
    /// Its tool name for progress reporting, e.g. `check_diagram`.
    pub tool: &'static str,
    /// What it does, for the agent.
    pub description: &'static str,
    /// The input it reads on stdin, for the agent.
    pub input: &'static str,
}

const CHECK_DIAGRAM: LabTool = LabTool {
    command: "check-diagram",
    flag: "--check-diagram",
    tool: "check_diagram",
    description: "Checks a Mermaid diagram as diffr will render it, and reports its errors. Run it on every diagram before answering.",
    input: "The diagram's Mermaid source, without ``` fences.",
};

/// The diffr executable a request's lab tools run: its `tool_bin`, else the
/// running executable, else `diffr` from the PATH.
pub fn tool_bin(bin: &Path) -> PathBuf {
    if !bin.as_os_str().is_empty() {
        return bin.to_path_buf();
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("diffr"))
}

impl Lab {
    /// The commands the lab lets the agent run.
    pub fn tools(&self) -> &'static [LabTool] {
        match self.id {
            DIAGRAMS => &[CHECK_DIAGRAM],
            _ => &[],
        }
    }

    /// The fields the lab adds to the review's answer.
    pub fn review_fields(&self) -> &'static [LabField] {
        match self.id {
            DIAGRAMS => &[LabField {
                name: DIAGRAM_FIELD,
                description: "A Mermaid diagram of the whole change, without a fence, or \"\" when a picture would not help.",
                kind: LabFieldKind::Mermaid,
            }],
            _ => &[],
        }
    }

    /// The fields the lab adds to each group's answer.
    pub fn group_fields(&self) -> &'static [LabField] {
        match self.id {
            DIAGRAMS => &[LabField {
                name: DIAGRAM_FIELD,
                description: "A Mermaid diagram of this group's intent, without a fence, or \"\" when a picture would not help.",
                kind: LabFieldKind::Mermaid,
            }],
            _ => &[],
        }
    }

    /// The lab's instructions, for an agent that runs `bin` as its tool.
    pub fn prompt(&self, bin: &str) -> String {
        match self.id {
            DIAGRAMS => {
                let cmd = format!("\"{bin}\" {}", CHECK_DIAGRAM.command);
                let flag = format!("\"{bin}\" {}", CHECK_DIAGRAM.flag);
                format!(
                    r#"<lab name="diagrams">
Draw a diagram where a picture explains the change faster than prose: how a call or request flows through the new code, which parts now talk to each other, the states something goes through, the shape of new types.
- The review's "diagram" shows the change as a whole; a group's "diagram" shows that group's intent. Leave a field "" when a picture would only restate the file list, or for small or mechanical changes (lockfiles, renames, formatting, version bumps). A small change needs one diagram at most; many need none.
- Write Mermaid source, without a ``` fence: flowchart (LR or TD), sequenceDiagram, stateDiagram-v2, classDiagram or erDiagram, whichever fits what changed.
- Keep it small: at most about 12 nodes or 10 messages. Name nodes after the functions, types, files and services as the code names them.
- Show the change, not only the system: mark what the change adds with `:::added`, what it alters with `:::changed` and what it removes with `:::removed`, e.g. `cache[Cache]:::added`. Leave the others plain. Do not write classDef, style or %%{{init}}%% lines: diffr colors and themes the diagram.
- In a flowchart, quote node labels holding punctuation, as in A["parse(args)"]. NEVER quote the text of a sequence diagram's messages or notes: write A->>B: parse(args). No HTML and no click handlers.
- Check EVERY diagram before answering: run `{cmd}` with the diagram's Mermaid source on stdin, as a heredoc:
  {cmd} <<'MERMAID'
  flowchart LR
    a["Request"]:::changed --> b[Cache]:::added
  MERMAID
  (`{flag}` is the same command.) It prints "ok", or the errors diffr finds when it renders the diagram, with their line. Fix every error and check the diagram again, until it passes; leave the field "" if it does not. That command only reads its input and changes nothing; it is the one command you may run besides reading the repository. If you have a `check_diagram` tool instead of a shell, call it with the source. If you can run neither, keep to the plainest syntax.
</lab>"#
                )
            }
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry() {
        assert_eq!(lab(DIAGRAMS).unwrap().name, "Diagrams");
        assert!(lab("nope").is_none());
        let ids = vec![" diagrams".to_string(), "nope".into(), "diagrams".into()];
        assert_eq!(enabled_ids(&ids), ["diagrams"]);
        for l in LABS {
            for f in l.review_fields().iter().chain(l.group_fields()) {
                assert!(!f.name.is_empty() && !f.description.is_empty());
                assert!(l.prompt("/bin/diffr").contains(&format!("\"{}\"", f.name)));
            }
            for t in l.tools() {
                let p = l.prompt("/bin/diffr");
                assert!(p.contains(t.command), "{}", l.id);
                assert!(p.contains(t.flag), "{}", l.id);
            }
        }
        assert_eq!(review_field_names().collect::<Vec<_>>(), ["diagram"]);
        assert_eq!(group_field_names().collect::<Vec<_>>(), ["diagram"]);
        assert_eq!(field("diagram").unwrap().kind, LabFieldKind::Mermaid);
        assert!(field("nope").is_none());
        assert_eq!(tool_bin(Path::new("/x/diffr")), Path::new("/x/diffr"));
        assert!(!tool_bin(Path::new("")).as_os_str().is_empty());
    }

    #[test]
    fn diagrams_prompt_names_the_command() {
        let p = lab(DIAGRAMS).unwrap().prompt("/opt/diffr");
        for want in [
            "at most about 12 nodes or 10 messages",
            "A small change needs one diagram at most",
            "quote node labels holding punctuation, as in A[\"parse(args)\"]",
            "NEVER quote the text of a sequence diagram's messages",
            "No HTML and no click handlers",
            "%%{init}%%",
            "`:::removed`",
            "\"/opt/diffr\" check-diagram <<'MERMAID'",
            "(`\"/opt/diffr\" --check-diagram` is the same command.)",
            "`check_diagram` tool",
        ] {
            assert!(p.contains(want), "no {want:?} in\n{p}");
        }
    }
}
