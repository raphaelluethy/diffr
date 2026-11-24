//! Takes away what an agent may put around or into a diagram that would
//! fail to parse or undo the app's theme.

use std::sync::LazyLock;

use regex::Regex;

use crate::style::{self, Header};

/// A sequence diagram's message or note whose text is quoted:
/// `A->>B: "parse(args)"`.
static QUOTED_MESSAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(\s*[^:"]+:\s*)"(.*)"\s*$"#).unwrap());

/// The source's lines, `None` for those taken away.
fn lines(src: &str) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = src.split('\n').map(|l| Some(l.to_string())).collect();
    let blank = |l: &Option<String>| l.as_deref().is_none_or(|l| l.trim().is_empty());
    let fence = |l: &Option<String>| l.as_deref().is_some_and(|l| l.trim().starts_with("```"));
    // The ``` fences around it, with their language (```mermaid).
    if let Some(i) = out.iter().position(|l| !blank(l))
        && fence(&out[i])
    {
        out[i] = None;
    }
    if let Some(i) = out.iter().rposition(|l| !blank(l))
        && fence(&out[i])
    {
        out[i] = None;
    }
    // `%%{init: …}%%` directives, also over several lines.
    let mut in_directive = false;
    for l in &mut out {
        let Some(text) = l.as_deref() else { continue };
        let t = text.trim();
        if in_directive || t.starts_with("%%{") {
            in_directive = !t.contains("}%%");
            *l = None;
        }
    }
    // A sequence diagram shows quotes as they are: take away those around
    // the text of its messages and notes.
    let kept: String = out
        .iter()
        .map(|l| l.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    if style::detect_header(&kept) == Header::Sequence {
        let mut after_header = false;
        for l in out.iter_mut().flatten() {
            if !after_header {
                after_header = l.trim_start().starts_with("sequenceDiagram");
                continue;
            }
            if let Some(c) = QUOTED_MESSAGE.captures(l) {
                *l = format!("{}{}", &c[1], &c[2]);
            }
        }
    }
    out
}

/// Takes away the ``` fences around a diagram, its `%%{…}%%` directives
/// (which would undo the app's theme), and the quotes around the text of a
/// sequence diagram's messages and notes, which Mermaid would show.
/// [`validate`](crate::validate) and [`render_svg`](crate::render_svg)
/// apply it.
pub fn clean(src: &str) -> String {
    lines(src)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
}

/// [`clean`], with the lines taken away left blank, so that errors keep
/// the lines of the source.
pub(crate) fn clean_in_place(src: &str) -> String {
    lines(src)
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fences_and_directives() {
        let src = "\n```mermaid\n%%{init: {'theme': 'dark'}}%%\nflowchart LR\n  a --> b\n```\n";
        assert_eq!(clean(src), "\nflowchart LR\n  a --> b\n");
        assert_eq!(clean_in_place(src), "\n\n\nflowchart LR\n  a --> b\n\n");
        let multi = "%%{\n  init: {\n    'theme': 'dark'\n  }\n}%%\ngraph TD\n  a-->b";
        assert_eq!(clean(multi), "graph TD\n  a-->b");
        // A plain comment stays.
        assert_eq!(
            clean("graph TD\n  %% a note\n  a-->b"),
            "graph TD\n  %% a note\n  a-->b"
        );
        assert_eq!(clean("```\ngraph TD\n```"), "graph TD");
    }

    #[test]
    fn unquotes_sequence_messages_only() {
        let src = "```\nsequenceDiagram\n  A->>B: \"parse(args)\"\n  Note over A,B: \"a note\"\n  B-->>A: ok \"x\"\n```";
        assert_eq!(
            clean(src),
            "sequenceDiagram\n  A->>B: parse(args)\n  Note over A,B: a note\n  B-->>A: ok \"x\""
        );
        let flow = "flowchart LR\n  A[\"x: y\"] --> B";
        assert_eq!(clean(flow), flow);
        let front = "---\ntitle: \"T\"\n---\nsequenceDiagram\n  A->>B: \"hi\"";
        assert_eq!(
            clean(front),
            "---\ntitle: \"T\"\n---\nsequenceDiagram\n  A->>B: hi"
        );
    }
}
