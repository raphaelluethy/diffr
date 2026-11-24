//! Syntax highlighting with tree-sitter grammars and highlight queries, the
//! way the Zed editor colors code, sliced into per-line segments.
//!
//! The input is every line of one version of a file: the whole text is
//! parsed once (so multi-line strings and comments come out right), then
//! the highlights are cut at line boundaries.
//!
//! ```
//! use diffr_highlight::{lines, Class};
//! let src: Vec<String> = ["fn main() {}"].iter().map(|s| s.to_string()).collect();
//! let segs = lines("main.rs", &src).unwrap();
//! assert!(segs[0].iter().any(|s| s.class == Class::Keyword && &src[0][s.start as usize..s.end as usize] == "fn"));
//! ```

mod classes;
mod languages;
mod queryfix;

use std::cell::RefCell;

use tree_sitter_highlight::{HighlightEvent, Highlighter};

pub use classes::{Class, class_for_capture};
pub use languages::{Language, language_by_name, language_for};

/// Files larger than this (in bytes, counting a newline per line) are not
/// highlighted.
pub const MAX_BYTES: usize = 1 << 20;

/// A run of one line's bytes of one class. Offsets are byte offsets within
/// the line, `start < end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Seg {
    pub start: u32,
    pub end: u32,
    pub class: Class,
}

thread_local! {
    static HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
}

/// Highlights all the lines of one version of the file named `file_name`
/// (only its name decides the language), and returns each line's segments,
/// in order, without [`Class::Plain`] runs, adjacent runs of one class
/// merged.
///
/// Returns `None` for files of no known language, files over
/// [`MAX_BYTES`], no lines, or if highlighting fails.
pub fn lines(file_name: &str, lines: &[String]) -> Option<Vec<Vec<Seg>>> {
    let lang = language_for(file_name)?;
    highlight_lines(lang, lines)
}

/// Like [`lines`], with the language given.
pub fn highlight_lines(lang: Language, lines: &[String]) -> Option<Vec<Vec<Seg>>> {
    if lines.is_empty() {
        return None;
    }
    let size: usize = lines.iter().map(|l| l.len() + 1).sum();
    if size > MAX_BYTES {
        return None;
    }
    let config = lang.config()?;

    let mut text = String::with_capacity(size);
    let mut starts = Vec::with_capacity(lines.len());
    for l in lines {
        starts.push(text.len());
        text.push_str(l);
        text.push('\n');
    }

    HIGHLIGHTER.with(|h| {
        let mut h = h.borrow_mut();
        let events = h
            .highlight(config, text.as_bytes(), None, |name| {
                languages::injection_language(name).and_then(Language::config)
            })
            .ok()?;

        let mut out: Vec<Vec<Seg>> = vec![Vec::new(); lines.len()];
        let mut stack: Vec<Class> = Vec::new();
        let mut row = 0usize;
        for event in events {
            match event.ok()? {
                HighlightEvent::HighlightStart(h) => stack.push(classes::class_of(h.0)),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    let class = stack.last().copied().unwrap_or(Class::Plain);
                    if class == Class::Plain {
                        continue;
                    }
                    let mut s = start;
                    if s < starts[row] {
                        row = starts.partition_point(|&ls| ls <= s).saturating_sub(1);
                    }
                    while s < end && row < lines.len() {
                        while row + 1 < lines.len() && starts[row + 1] <= s {
                            row += 1;
                        }
                        let ls = starts[row];
                        let le = ls + lines[row].len();
                        let e = end.min(le);
                        if e > s {
                            push(&mut out[row], (s - ls) as u32, (e - ls) as u32, class);
                        }
                        if end <= le {
                            break;
                        }
                        // Skip the newline; continue on the next line.
                        s = le + 1;
                    }
                }
            }
        }
        Some(out)
    })
}

fn push(segs: &mut Vec<Seg>, start: u32, end: u32, class: Class) {
    if let Some(last) = segs.last_mut()
        && last.class == class
        && last.end == start
    {
        last.end = end;
        return;
    }
    segs.push(Seg { start, end, class });
}
