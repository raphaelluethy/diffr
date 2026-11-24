//! Turns parser errors into precise, agent-actionable messages.

use mermaid_rs_renderer::ParseError;

/// Location in the original source (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub col: usize,
}

pub(crate) fn describe(src: &str, err: &ParseError) -> String {
    let (loc, msg) = match err {
        ParseError::UnknownParticipant {
            name,
            line,
            candidates,
        } => {
            let line = *line as usize;
            let col = column_of(src, line, name).unwrap_or(1);
            let mut msg = format!(
                "unknown participant '{name}': this diagram declares participants explicitly, \
                 so add `participant {name}` (or `actor {name}`) before using it"
            );
            if !candidates.is_empty() {
                msg.push_str(&format!(" — did you mean {}?", candidates.join(", ")));
            }
            (Some(Location { line, col }), msg)
        }
        ParseError::UnclosedSubgraph { opened_at } => (
            Some(Location {
                line: *opened_at as usize,
                col: first_non_ws_col(src, *opened_at as usize),
            }),
            "block opened here is never closed: add a matching `end`".to_string(),
        ),
        ParseError::UnexpectedToken {
            line,
            col,
            found,
            expected,
        } => {
            let mut loc = Location {
                line: *line as usize,
                col: *col as usize,
            };
            // Errors from the non-strict parser are reported at 1:1 with the
            // header as `found`; recover the real location from the
            // fragment quoted at the end of the message ("...: A -->").
            if loc.line == 1
                && loc.col == 1
                && let Some(better) = locate_fragment(src, expected)
            {
                loc = better;
                (Some(loc), expected.clone())
            } else if loc.line == 1 && loc.col == 1 {
                (Some(loc), expected.clone())
            } else {
                (
                    Some(loc),
                    format!("unexpected '{found}', expected {expected}"),
                )
            }
        }
        ParseError::InvalidDirective {
            line,
            col,
            directive,
            reason,
        } => (
            Some(Location {
                line: *line as usize,
                col: *col as usize,
            }),
            format!("invalid directive '{directive}': {reason}"),
        ),
        other => (None, other.to_string()),
    };
    match loc {
        Some(loc) => with_snippet(src, loc, &msg),
        None => msg,
    }
}

/// `line L, column C: msg` plus the offending source line and a caret.
pub(crate) fn with_snippet(src: &str, loc: Location, msg: &str) -> String {
    let mut out = format!("line {}, column {}: {msg}", loc.line, loc.col);
    if let Some(text) = src.lines().nth(loc.line.saturating_sub(1)) {
        let gutter = loc.line.to_string();
        let pad = " ".repeat(gutter.len());
        let caret_pad: String = text
            .chars()
            .take(loc.col.saturating_sub(1))
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();
        out.push_str(&format!("\n {gutter} | {text}\n {pad} | {caret_pad}^"));
    }
    out
}

/// Message located at the first occurrence of `needle` on `line`.
pub(crate) fn at_text(src: &str, line: usize, needle: &str, msg: &str) -> String {
    let col = column_of(src, line, needle).unwrap_or_else(|| first_non_ws_col(src, line));
    with_snippet(src, Location { line, col }, msg)
}

fn first_non_ws_col(src: &str, line: usize) -> usize {
    src.lines()
        .nth(line.saturating_sub(1))
        .and_then(|l| l.chars().position(|c| !c.is_whitespace()))
        .map_or(1, |p| p + 1)
}

fn column_of(src: &str, line: usize, needle: &str) -> Option<usize> {
    let text = src.lines().nth(line.saturating_sub(1))?;
    let byte = text.find(needle)?;
    Some(text[..byte].chars().count() + 1)
}

/// Finds the source line containing the fragment after the last `": "` of a
/// message.
fn locate_fragment(src: &str, message: &str) -> Option<Location> {
    let (_, fragment) = message.rsplit_once(": ")?;
    let fragment = fragment.trim();
    if fragment.len() < 2 {
        return None;
    }
    for (i, text) in src.lines().enumerate() {
        if let Some(byte) = text.find(fragment) {
            return Some(Location {
                line: i + 1,
                col: text[..byte].chars().count() + 1,
            });
        }
    }
    // The parser may have normalised whitespace; compare trimmed lines.
    for (i, text) in src.lines().enumerate() {
        if text.trim() == fragment || text.split_whitespace().eq(fragment.split_whitespace()) {
            return Some(Location {
                line: i + 1,
                col: first_non_ws_col(src, i + 1),
            });
        }
    }
    None
}
