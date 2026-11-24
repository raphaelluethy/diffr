//! Structural checks the (lenient) upstream parser skips: mistakes that
//! Mermaid itself rejects, or that silently produce a wrong picture.
//! Every check reports a source location.

use std::sync::LazyLock;

use mermaid_rs_renderer::Graph;
use regex::Regex;

use crate::error::{Location, with_snippet};
use crate::style::{CLASS_STMT, Header, STATUS_CLASSES};

type Lint = Result<(), String>;

fn fail(src: &str, line: usize, col: usize, msg: &str) -> Lint {
    Err(with_snippet(src, Location { line, col }, msg))
}

/// Lines that carry diagram statements: `(1-based line, text)` without
/// comments, directives or front matter.
fn statements(src: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut in_front_matter = false;
    let mut seen_header = false;
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        if i == 0 && t == "---" {
            in_front_matter = true;
            continue;
        }
        if in_front_matter {
            in_front_matter = t != "---";
            continue;
        }
        if t.is_empty() || t.starts_with("%%") {
            continue;
        }
        if !seen_header {
            seen_header = true;
            continue;
        }
        out.push((i + 1, line));
    }
    out
}

fn col_of(line: &str, byte: usize) -> usize {
    line[..byte].chars().count() + 1
}

fn indent_col(line: &str) -> usize {
    col_of(line, line.len() - line.trim_start().len())
}

pub(crate) fn check(src: &str, header: Header) -> Lint {
    match header {
        Header::Native => {
            if is_flowchart(src) {
                flowchart_brackets(src)?;
            }
            Ok(())
        }
        Header::Class => {
            braces(src, &CLASS_OPEN)?;
            dangling(src, &CLASS_DANGLING, "relation")
        }
        Header::State => {
            braces(src, &STATE_OPEN)?;
            dangling(src, &STATE_DANGLING, "transition")
        }
        Header::Er => {
            braces(src, &ER_OPEN)?;
            er_relations(src)
        }
        Header::Sequence => sequence_messages(src),
    }
}

fn is_flowchart(src: &str) -> bool {
    src.lines()
        .map(str::trim)
        .find(|t| !t.is_empty() && !t.starts_with("%%") && *t != "---")
        .is_some_and(|t| t.starts_with("flowchart") || t.starts_with("graph"))
}

static CLASS_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*class\s+[^{]*\{\s*$").unwrap());
static STATE_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*state\s+[^{]*\{\s*$").unwrap());
static ER_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*[\w"\-.\[\] ]+\{\s*$"#).unwrap());
static CLOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\}\s*;?\s*$").unwrap());

/// `class X {` / `state X {` / `ENTITY {` blocks must be closed by `}`.
fn braces(src: &str, open: &Regex) -> Lint {
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for (line, text) in statements(src) {
        if open.is_match(text) {
            stack.push((line, indent_col(text)));
        } else if CLOSE.is_match(text) && stack.pop().is_none() {
            return fail(
                src,
                line,
                indent_col(text),
                "'}' without a matching '{' block",
            );
        }
    }
    match stack.last() {
        Some(&(line, col)) => fail(
            src,
            line,
            col,
            "block opened here is never closed: add a line with `}`",
        ),
        None => Ok(()),
    }
}

static CLASS_DANGLING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*[^\s:][^:]*?(<\|--|--\|>|<\|\.\.|\.\.\|>|\*--|--\*|o--|--o|-->|<--|\.\.>|<\.\.|--|\.\.)\s*("[^"]*")?\s*(:.*)?$"#)
        .unwrap()
});
static STATE_DANGLING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[^\s:][^:]*?(-->)\s*(:.*)?$").unwrap());

/// A relation/transition operator with nothing on its right-hand side.
fn dangling(src: &str, re: &Regex, what: &str) -> Lint {
    for (line, text) in statements(src) {
        if let Some(c) = re.captures(text) {
            let op = c.get(1).unwrap();
            return fail(
                src,
                line,
                col_of(text, op.start()),
                &format!(
                    "{what} '{}' has no target: write `A {} B`",
                    op.as_str(),
                    op.as_str()
                ),
            );
        }
    }
    Ok(())
}

static ER_CARDINALITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[|}o][|o]?(--|\.\.)[|o]?[|{o]").unwrap());
static ER_RELATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*("[^"]+"|[\w\-.]+)\s*[|}o][|o]?(--|\.\.)[|o]?[|{o]\s*("[^"]+"|[\w\-.]+)\s*:\s*\S.*$"#)
        .unwrap()
});

/// `A ||--o{ B : label`: both entities and the label are required.
fn er_relations(src: &str) -> Lint {
    for (line, text) in statements(src) {
        let Some(card) = ER_CARDINALITY.find(text) else {
            continue;
        };
        if ER_RELATION.is_match(text) {
            continue;
        }
        let before = text[..card.start()].trim();
        let after = text[card.end()..].trim();
        let (col, msg) = if before.is_empty() {
            (indent_col(text), "relationship is missing its first entity")
        } else if after.is_empty() || after.starts_with(':') {
            (
                col_of(text, card.start()),
                "relationship is missing its second entity",
            )
        } else {
            (
                col_of(text, card.end()),
                "relationship needs a label: `A ||--o{ B : \"has\"`",
            )
        };
        return fail(src, line, col, msg);
    }
    Ok(())
}

static SEQ_ARROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<<-->>|<<->>|-->>|->>|-->|->|--x|-x|--\)|-\)").unwrap());
static SEQ_SKIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(participant|actor|note|loop|alt|else|opt|par|and|critical|option|break|rect|end|box|autonumber|activate|deactivate|create|destroy|title|accTitle|accDescr|links?|properties|details)\b",
    )
    .unwrap()
});

/// Every message needs `: text` after the target participant.
fn sequence_messages(src: &str) -> Lint {
    for (line, text) in statements(src) {
        if SEQ_SKIP.is_match(text) {
            continue;
        }
        let Some(arrow) = SEQ_ARROW.find(text) else {
            continue;
        };
        let lhs = text[..arrow.start()].trim();
        let rhs = &text[arrow.end()..];
        let target = rhs.split(':').next().unwrap_or("").trim();
        let target = target.trim_start_matches(['+', '-']).trim();
        if lhs.is_empty() {
            return fail(
                src,
                line,
                indent_col(text),
                "message is missing its sender: write `A->>B: text`",
            );
        }
        if target.is_empty() {
            return fail(
                src,
                line,
                col_of(text, arrow.end()),
                "message is missing its receiver: write `A->>B: text`",
            );
        }
        if !rhs.contains(':') {
            return fail(
                src,
                line,
                col_of(text, arrow.end()),
                &format!(
                    "message needs text after a colon: write `{lhs}{}{}: text`",
                    arrow.as_str(),
                    target.split_whitespace().next().unwrap_or(target)
                ),
            );
        }
        if target.contains(char::is_whitespace) {
            return fail(
                src,
                line,
                col_of(text, arrow.end()),
                &format!(
                    "participant name '{target}' contains spaces: declare `participant X as {target}` and use X, or add the missing colon"
                ),
            );
        }
    }
    Ok(())
}

/// Unclosed node shapes (`A[text`, `B((text)`, `C{text`) or quotes in a
/// flowchart. The shape opener run (`[`, `[(`, `((`, `{{`...) must be
/// followed later on the line by the mirrored closer run; text in between
/// is free-form.
fn flowchart_brackets(src: &str) -> Lint {
    for (line, text) in statements(src) {
        let t = text.trim_start();
        if [
            "classDef",
            "class ",
            "style ",
            "linkStyle",
            "click ",
            "direction",
        ]
        .iter()
        .any(|k| t.starts_with(k))
        {
            continue;
        }
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut i = 0;
        let mut in_pipe = false;
        while i < chars.len() {
            let (byte, ch) = chars[i];
            match ch {
                '"' => match chars[i + 1..].iter().position(|&(_, c)| c == '"') {
                    Some(off) => i += off + 1,
                    None => return fail(src, line, col_of(text, byte), "unclosed '\"'"),
                },
                '|' => in_pipe = !in_pipe,
                '[' | '(' | '{' if !in_pipe => {
                    let run: String = chars[i..]
                        .iter()
                        .map(|&(_, c)| c)
                        .take_while(|c| matches!(c, '[' | '(' | '{'))
                        .collect();
                    let closer: String = run
                        .chars()
                        .rev()
                        .map(|c| match c {
                            '[' => ']',
                            '(' => ')',
                            _ => '}',
                        })
                        .collect();
                    let body_start = i + run.chars().count();
                    match find_outside_quotes(&chars[body_start..], &closer) {
                        Some(off) => i = body_start + off + closer.chars().count() - 1,
                        None => {
                            return fail(
                                src,
                                line,
                                col_of(text, byte),
                                &format!("unclosed '{run}': node text must end with '{closer}'"),
                            );
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    Ok(())
}

/// Index (in `chars`) where `needle` starts, skipping quoted text.
fn find_outside_quotes(chars: &[(usize, char)], needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].1 == '"' {
            i += chars[i + 1..].iter().position(|&(_, c)| c == '"')? + 2;
            continue;
        }
        if chars[i..].len() >= needle.len()
            && chars[i..i + needle.len()]
                .iter()
                .map(|&(_, c)| c)
                .eq(needle.iter().copied())
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Classes used on nodes must be status classes or defined with `classDef`,
/// and must target existing nodes; otherwise the styling silently vanishes.
pub(crate) fn check_classes(src: &str, graph: &Graph) -> Lint {
    let mut used: Vec<(&String, &String)> = graph
        .node_classes
        .iter()
        .flat_map(|(id, cs)| cs.iter().map(move |c| (id, c)))
        .chain(
            graph
                .subgraph_classes
                .iter()
                .flat_map(|(id, cs)| cs.iter().map(move |c| (id, c))),
        )
        .collect();
    used.sort();
    // The flowchart parser drops `class X cls` for unknown X silently.
    for (line, text) in statements(src) {
        let Some(c) = CLASS_STMT.captures(text) else {
            continue;
        };
        for id in c[1].split(',').map(str::trim) {
            let known = graph.nodes.contains_key(id)
                || graph.subgraphs.iter().any(|s| s.id.as_deref() == Some(id));
            if !known && !id.is_empty() {
                let col = col_of(text, c.get(1).unwrap().start() + c[1].find(id).unwrap_or(0));
                return fail(
                    src,
                    line,
                    col,
                    &format!(
                        "class '{}' is assigned to '{id}', which is not a node in this diagram",
                        &c[2]
                    ),
                );
            }
        }
    }
    let subgraph_ids: Vec<&str> = graph
        .subgraphs
        .iter()
        .filter_map(|s| s.id.as_deref())
        .collect();
    for (id, class) in used {
        if !graph.class_defs.contains_key(class) && !STATUS_CLASSES.contains(&class.as_str()) {
            let (line, col) = find(src, class);
            return fail(
                src,
                line,
                col,
                &format!(
                    "class '{class}' is not defined: use added, changed or removed, or define it with `classDef {class} fill:#...,stroke:#...`"
                ),
            );
        }
        let known = graph.nodes.contains_key(id.as_str())
            || subgraph_ids.contains(&id.as_str())
            || graph.subgraph_classes.contains_key(id.as_str());
        if !known {
            let (line, col) = find(src, id);
            return fail(
                src,
                line,
                col,
                &format!(
                    "class '{class}' is assigned to '{id}', which is not a node in this diagram"
                ),
            );
        }
    }
    Ok(())
}

/// First line mentioning `needle` in a class statement (or anywhere).
fn find(src: &str, needle: &str) -> (usize, usize) {
    let mut fallback = None;
    for (i, text) in src.lines().enumerate() {
        if let Some(byte) = text.find(needle) {
            let t = text.trim_start();
            let pos = (i + 1, col_of(text, byte));
            if t.starts_with("class ") || t.starts_with("cssClass") || text.contains(":::") {
                return pos;
            }
            fallback.get_or_insert(pos);
        }
    }
    fallback.unwrap_or((1, 1))
}
