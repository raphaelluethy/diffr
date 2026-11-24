//! The reviews kept per source, and what of them still holds: the parts of
//! the review that are not UI.
//!
//! A review is saved as `<state dir>/analyses/<key>.json`, the key naming
//! the repository and the source reviewed ([`analysis_key`]), with the
//! fingerprints of the files as the agent saw them, to tell which changed
//! since ([`changed_since`]): the notes on those do not show
//! ([`notes`]), as the lines they are on may have moved.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::{Analysis, nullable};

/// What a review is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// Uncommitted changes.
    WorkingTree,
    Commit,
    /// A branch's changes since it branched.
    Branch,
    /// A pull request, its ref as `owner/repo#123`.
    Pull,
}

impl SourceKind {
    fn key(self) -> &'static str {
        match self {
            SourceKind::WorkingTree => "worktree",
            SourceKind::Commit => "commit",
            SourceKind::Branch => "branch",
            SourceKind::Pull => "pull",
        }
    }
}

/// Names a source for the reviews saved: the SHA-1, in hex, of the
/// repository's root, the kind and the ref.
pub fn analysis_key(root: &str, kind: SourceKind, reference: &str) -> String {
    let mut h = Sha1::new();
    h.update(root.as_bytes());
    h.update(b"\0");
    h.update(kind.key().as_bytes());
    h.update(b":");
    h.update(reference.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Where the review of a source is saved, under the directory of the
/// app's state.
pub fn analysis_path(state_dir: &Path, root: &str, kind: SourceKind, reference: &str) -> PathBuf {
    state_dir
        .join("analyses")
        .join(format!("{}.json", analysis_key(root, kind, reference)))
}

/// A review saved, with the fingerprints of the files it saw, by path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedAnalysis {
    #[serde(default)]
    pub analysis: Option<Analysis>,
    #[serde(default, deserialize_with = "nullable")]
    pub fingerprints: HashMap<String, String>,
}

/// Saves a review and the fingerprints of the files it saw.
pub fn save_analysis(
    path: &Path,
    analysis: &Analysis,
    fingerprints: &HashMap<String, String>,
) -> io::Result<()> {
    let data = serde_json::to_vec(&SavedAnalysis {
        analysis: Some(analysis.clone()),
        fingerprints: fingerprints.clone(),
    })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, data)
}

/// Reads the review saved at `path`, `None` when there is none, or none
/// with groups.
pub fn load_analysis(path: &Path) -> Option<(Analysis, HashMap<String, String>)> {
    let data = std::fs::read(path).ok()?;
    let saved: SavedAnalysis = serde_json::from_slice(&data).ok()?;
    let a = saved.analysis?;
    if a.groups.is_empty() {
        return None;
    }
    Some((a, saved.fingerprints))
}

/// Counts the files that changed since the review: new, gone or
/// different. `files` are the paths and fingerprints shown now.
pub fn changed_since<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a str)>,
    fingerprints: &HashMap<String, String>,
) -> usize {
    let mut n = 0;
    let mut seen = std::collections::HashSet::new();
    for (path, fp) in files {
        seen.insert(path);
        if fingerprints.get(path).is_none_or(|f| f != fp) {
            n += 1;
        }
    }
    n + fingerprints
        .keys()
        .filter(|p| !seen.contains(p.as_str()))
        .count()
}

/// The side of a line a note is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NoteSide {
    /// A line of the new file (added or unchanged).
    New,
    /// A removed line, numbered as in the old file.
    Old,
}

/// A note of the agent's review on a file, or on a line of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Note {
    pub path: String,
    /// On the whole file; `side` and `line` mean nothing then.
    pub file: bool,
    pub side: NoteSide,
    pub line: i64,
    pub text: String,
    pub critical: bool,
}

impl Note {
    /// Its label in the card: `AI note on this file`,
    /// `AI note · line 3`, `AI note · old line 3`, with ` · needs care`.
    pub fn label(&self) -> String {
        let mut l = if self.file {
            "AI note on this file".to_string()
        } else if self.side == NoteSide::Old {
            format!("AI note · old line {}", self.line)
        } else {
            format!("AI note · line {}", self.line)
        };
        if self.critical {
            l.push_str(" · needs care");
        }
        l
    }
}

/// Gathers the notes of a review on the files unchanged since, by path:
/// on others, the lines they are on may have moved. `files` are the paths
/// and fingerprints shown now; a file the review did not see counts as
/// unchanged.
pub fn notes<'a>(
    analysis: &Analysis,
    fingerprints: &HashMap<String, String>,
    files: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> HashMap<String, Vec<Note>> {
    let current: std::collections::HashSet<&str> = files
        .into_iter()
        .filter(|(p, fp)| fingerprints.get(*p).is_none_or(|f| f == fp))
        .map(|(p, _)| p)
        .collect();
    let mut out: HashMap<String, Vec<Note>> = HashMap::new();
    for g in &analysis.groups {
        for n in &g.file_notes {
            if current.contains(n.path.as_str()) {
                out.entry(n.path.clone()).or_default().push(Note {
                    path: n.path.clone(),
                    file: true,
                    side: NoteSide::New,
                    line: 0,
                    text: n.text.clone(),
                    critical: n.critical,
                });
            }
        }
        for n in &g.line_notes {
            if current.contains(n.path.as_str()) {
                out.entry(n.path.clone()).or_default().push(Note {
                    path: n.path.clone(),
                    file: false,
                    side: if n.is_old_side() {
                        NoteSide::Old
                    } else {
                        NoteSide::New
                    },
                    line: n.line,
                    text: n.text.clone(),
                    critical: n.critical,
                });
            }
        }
    }
    out
}

/// Lists the notes in the order their files show (`paths`), those on a
/// whole file before those on its lines, by line.
pub fn ordered_notes<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    notes: &HashMap<String, Vec<Note>>,
) -> Vec<Note> {
    let mut out = Vec::new();
    for p in paths {
        let Some(list) = notes.get(p) else { continue };
        let mut list = list.clone();
        list.sort_by_key(|n| (!n.file, n.line));
        out.extend(list);
    }
    out
}

/// How many notes the card lists before it is asked for all.
pub const LISTED_NOTES: usize = 5;

/// Where a note is, as the card's list shows it: `greet.go:3`,
/// `greet.go:3 (old)`, or the file's name alone.
pub fn note_location(n: &Note) -> String {
    let base = n.path.rsplit('/').next().unwrap_or(&n.path);
    if n.file {
        base.to_string()
    } else if n.side == NoteSide::Old {
        format!("{base}:{} (old)", n.line)
    } else {
        format!("{base}:{}", n.line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{ANSWER, files};

    fn fps(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn keys() {
        let a = analysis_key("/repo", SourceKind::Pull, "acme/app#1");
        assert_eq!(a.len(), 40);
        assert_ne!(a, analysis_key("/repo", SourceKind::Branch, "acme/app#1"));
        // sha1("/repo\0worktree:")
        assert_eq!(analysis_key("/repo", SourceKind::WorkingTree, ""), {
            let mut h = Sha1::new();
            h.update(b"/repo\x00worktree:");
            h.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        });
    }

    #[test]
    fn save_load_and_notes() {
        let dir = tempfile::tempdir().unwrap();
        let path = analysis_path(dir.path(), "/repo", SourceKind::Commit, "abc");
        let an = crate::parse(ANSWER, &files()).unwrap();
        let saved = fps(&[
            ("src/greet.go", "1"),
            ("src/greet_test.go", "2"),
            ("gone.go", "3"),
        ]);
        save_analysis(&path, &an, &saved).unwrap();
        let (loaded, fp) = load_analysis(&path).unwrap();
        assert_eq!(loaded, an);
        assert_eq!(fp, saved);
        assert!(load_analysis(&dir.path().join("none.json")).is_none());

        let now = [
            ("src/greet.go", "1"),
            ("src/greet_test.go", "changed"),
            ("new.go", "x"),
        ];
        // greet_test changed, new.go is new, gone.go is gone.
        assert_eq!(changed_since(now, &fp), 3);
        let notes = notes(&loaded, &fp, now);
        let list = &notes["src/greet.go"];
        assert_eq!(list.len(), 2);
        let ordered = ordered_notes(["src/greet.go"], &notes);
        assert!(ordered[0].file && ordered[0].critical);
        assert_eq!(ordered[0].label(), "AI note on this file · needs care");
        assert_eq!(ordered[1].label(), "AI note · line 2");
        assert_eq!(note_location(&ordered[1]), "greet.go:2");
        // On a changed file, no notes show.
        let changed = super::notes(&loaded, &fp, [("src/greet.go", "other")]);
        assert!(changed.is_empty());
    }
}
