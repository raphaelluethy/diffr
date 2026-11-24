//! Review comments on lines of a diff, and their export as Markdown for an
//! agent or a colleague to address.

use crate::diff::{File, LineKind};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// The side of a diff a comment is on.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub enum Side {
    /// The new file: added and unchanged lines, by their new numbers.
    #[default]
    New,
    /// The old file: deleted and unchanged lines, by their old numbers.
    Old,
}

/// A review comment on a line, or a range of lines of one side, anchored on
/// its last line.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Comment {
    pub id: u64,
    pub path: String,
    pub side: Side,
    /// The first line of the range, 0 for a single line.
    pub start: u32,
    /// The last line, where it shows.
    pub line: u32,
    pub text: String,
}

impl Comment {
    /// Whether the comment has text, and so is not an empty draft.
    pub fn pending(&self) -> bool {
        !self.text.trim().is_empty()
    }

    /// `"New line 12"`, `"Old lines 3-5"`.
    pub fn label(&self) -> String {
        let s = match self.side {
            Side::New => "New",
            Side::Old => "Old",
        };
        if self.start != 0 && self.start != self.line {
            return format!(
                "{s} lines {}-{}",
                self.start.min(self.line),
                self.start.max(self.line)
            );
        }
        format!("{s} line {}", self.line)
    }
}

/// The comments of a review, with the ids they are given.
#[derive(Debug, Clone, Default)]
pub struct Comments {
    /// The comments, in the order they were started.
    pub list: Vec<Comment>,
    next_id: u64,
}

impl Comments {
    /// No comments.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a comment on lines `start..=line` of a side of a file, or
    /// returns the empty draft already there, and returns the id of the
    /// comment to focus. An empty draft elsewhere moves here rather than stay
    /// behind.
    pub fn add(&mut self, path: &str, side: Side, start: u32, line: u32) -> u64 {
        if let Some(c) = self
            .list
            .iter()
            .find(|c| c.path == path && c.side == side && c.line == line && !c.pending())
        {
            return c.id;
        }
        self.list.retain(Comment::pending);
        // Ids are unique across all the collections of the process, so a
        // comment keeps its identity as the review switches sources.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        self.next_id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.list.push(Comment {
            id: self.next_id,
            path: path.to_string(),
            side,
            start,
            line,
            text: String::new(),
        });
        self.next_id
    }

    /// The comment with an id.
    pub fn get(&self, id: u64) -> Option<&Comment> {
        self.list.iter().find(|c| c.id == id)
    }

    /// The comment with an id, to change its text.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Comment> {
        self.list.iter_mut().find(|c| c.id == id)
    }

    /// Deletes the comment with an id.
    pub fn delete(&mut self, id: u64) {
        self.list.retain(|c| c.id != id);
    }

    /// The number of comments with text.
    pub fn pending_count(&self) -> usize {
        self.list.iter().filter(|c| c.pending()).count()
    }

    /// Drops the empty drafts of files gone from the review.
    pub fn prune<'a>(&mut self, paths: impl IntoIterator<Item = &'a str>) {
        let paths: HashSet<&str> = paths.into_iter().collect();
        self.list
            .retain(|c| paths.contains(c.path.as_str()) || c.pending());
    }

    /// Removes every comment.
    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// The pending comments as Markdown: see [`comments_markdown`].
    pub fn markdown<F: CommentSource>(&self, files: &[F], prefix: &str) -> String {
        comments_markdown(files, &self.list, prefix)
    }
}

/// A changed file the comments are on, with the lines of its old and new
/// contents when they are loaded (to give the context of comments on lines
/// outside the hunks).
pub trait CommentSource {
    /// The change.
    fn diff_file(&self) -> &File;
    /// The lines of the old file, once loaded.
    fn old_lines(&self) -> Option<&[String]> {
        None
    }
    /// The lines of the new file, once loaded.
    fn new_lines(&self) -> Option<&[String]> {
        None
    }
}

impl CommentSource for File {
    fn diff_file(&self) -> &File {
        self
    }
}

impl<T: CommentSource + ?Sized> CommentSource for &T {
    fn diff_file(&self) -> &File {
        (**self).diff_file()
    }
    fn old_lines(&self) -> Option<&[String]> {
        (**self).old_lines()
    }
    fn new_lines(&self) -> Option<&[String]> {
        (**self).new_lines()
    }
}

/// A [`File`] and the lines of its two sides.
#[derive(Debug, Clone, Copy)]
pub struct FileContents<'a> {
    pub file: &'a File,
    pub old_lines: Option<&'a [String]>,
    pub new_lines: Option<&'a [String]>,
}

impl CommentSource for FileContents<'_> {
    fn diff_file(&self) -> &File {
        self.file
    }
    fn old_lines(&self) -> Option<&[String]> {
        self.old_lines
    }
    fn new_lines(&self) -> Option<&[String]> {
        self.new_lines
    }
}

/// Writes the pending comments as Markdown, each with the lines around it,
/// for an agent or a colleague to address: in the order of the files, then
/// of their lines, under `prefix` (unless empty). `""` when no comment has
/// text.
pub fn comments_markdown<F: CommentSource>(
    files: &[F],
    comments: &[Comment],
    prefix: &str,
) -> String {
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        index.insert(f.diff_file().path.as_str(), i);
    }
    let mut list: Vec<&Comment> = comments.iter().filter(|c| c.pending()).collect();
    if list.is_empty() {
        return String::new();
    }
    let pos = |c: &Comment| index.get(c.path.as_str()).copied().unwrap_or(files.len());
    list.sort_by(|a, b| {
        pos(a)
            .cmp(&pos(b))
            .then(a.line.cmp(&b.line))
            .then(a.id.cmp(&b.id))
    });
    let mut items = Vec::new();
    for (i, c) in list.iter().enumerate() {
        let mut context = "No patch context available.".to_string();
        if let Some(&fi) = index.get(c.path.as_str()) {
            let f = &files[fi];
            let ctx = comment_context(f, c);
            if !ctx.is_empty() {
                context = ctx;
            } else if !f.diff_file().note.is_empty() {
                context = f.diff_file().note.clone();
            }
        }
        let mut fence = "```".to_string();
        while context.contains(&fence) {
            fence.push('`');
        }
        items.push(format!(
            "{}. **{}** ({})\n\n{}\n\n{}",
            i + 1,
            c.path,
            c.label(),
            indent3(&format!("{fence}diff\n{context}\n{fence}")),
            indent3(c.text.trim())
        ));
    }
    let out = items.join("\n\n");
    if prefix.is_empty() {
        out
    } else {
        format!("{prefix}\n\n{out}")
    }
}

fn indent3(s: &str) -> String {
    s.split('\n')
        .map(|l| format!("   {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The hunk header and the lines around a comment, three on either side, as
/// a diff; for a comment outside the hunks, the lines around it from the
/// file's contents; `""` when neither is known.
pub fn comment_context<F: CommentSource + ?Sized>(f: &F, c: &Comment) -> String {
    struct Row<'a> {
        prefix: char,
        old: u32,
        new: u32,
        text: &'a str,
    }
    for h in &f.diff_file().hunks {
        // Deletions before additions within each change, as diffs read.
        let mut rows = Vec::new();
        let mut i = 0;
        while i < h.lines.len() {
            let l = &h.lines[i];
            if l.kind == LineKind::Context {
                rows.push(Row {
                    prefix: ' ',
                    old: l.old,
                    new: l.new,
                    text: &l.text,
                });
                i += 1;
                continue;
            }
            let mut j = i;
            while j < h.lines.len() && h.lines[j].kind != LineKind::Context {
                j += 1;
            }
            for l in &h.lines[i..j] {
                if l.kind == LineKind::Del {
                    rows.push(Row {
                        prefix: '-',
                        old: l.old,
                        new: 0,
                        text: &l.text,
                    });
                }
            }
            for l in &h.lines[i..j] {
                if l.kind == LineKind::Add {
                    rows.push(Row {
                        prefix: '+',
                        old: 0,
                        new: l.new,
                        text: &l.text,
                    });
                }
            }
            i = j;
        }
        let find = |n: u32| {
            rows.iter().position(|r| match c.side {
                Side::Old => r.prefix != '+' && r.old == n,
                Side::New => r.prefix != '-' && r.new == n,
            })
        };
        let Some(end) = find(c.line) else {
            continue;
        };
        let mut start = end;
        if c.start != 0
            && let Some(s) = find(c.start)
        {
            start = s;
        }
        let lo = start.min(end).saturating_sub(3);
        let hi = (start.max(end) + 4).min(rows.len());
        let mut b = format!(
            "@@ -{},{} +{},{} @@",
            h.old_start, h.old_lines, h.new_start, h.new_lines
        );
        for r in &rows[lo..hi] {
            let num = match r.prefix {
                '+' => r.new.to_string(),
                '-' => r.old.to_string(),
                _ => format!("{}/{}", r.old, r.new),
            };
            let _ = write!(
                b,
                "\n{}{num:>4} | {}",
                r.prefix,
                r.text.trim_end_matches(['\r', '\n'])
            );
        }
        return b;
    }
    gap_context(f, c)
}

/// The lines around a comment on an unchanged line outside the hunks, one
/// the user expanded, from the file's contents.
fn gap_context<F: CommentSource + ?Sized>(f: &F, c: &Comment) -> String {
    let (old_lines, new_lines) = (f.old_lines(), f.new_lines());
    if old_lines.is_none() && new_lines.is_none() {
        return String::new();
    }
    let file = f.diff_file();
    let context_text = |old: u32, new: u32| -> &str {
        if let Some(l) = new_lines
            && new > 0
            && (new as usize) <= l.len()
        {
            return &l[new as usize - 1];
        }
        if let Some(l) = old_lines
            && old > 0
            && (old as usize) <= l.len()
        {
            return &l[old as usize - 1];
        }
        ""
    };
    for i in 0..=file.hunks.len() {
        let g = file.gap_before(
            i,
            old_lines.map(<[String]>::len),
            new_lines.map(<[String]>::len),
        );
        let start = match c.side {
            Side::Old => g.old_start,
            Side::New => g.new_start,
        } as i64;
        let k = c.line as i64 - start;
        if k < 0 || k >= g.count as i64 {
            continue;
        }
        let k = k as u32;
        let (lo, hi) = (k.saturating_sub(3), (k + 4).min(g.count));
        let mut b = format!(
            "@@ -{},{} +{},{} @@",
            g.old_start + lo,
            hi - lo,
            g.new_start + lo,
            hi - lo
        );
        for j in lo..hi {
            let (old, new) = (g.old_start + j, g.new_start + j);
            let _ = write!(
                b,
                "\n {:>4} | {}",
                format!("{old}/{new}"),
                context_text(old, new)
            );
        }
        return b;
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Options, Repo};
    use std::path::Path;

    fn git_in(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn long_text() -> String {
        let mut s = String::new();
        for i in 0..60 {
            s.push_str("line ");
            s.push_str(&"x".repeat(i % 7));
            s.push('\n');
        }
        s
    }

    /// The test repository, reduced to its docs/long.txt.
    fn test_repo() -> (tempfile::TempDir, Repo) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git_in(dir, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/long.txt"), long_text()).unwrap();
        git_in(dir, &["add", "."]);
        git_in(dir, &["commit", "-q", "-m", "First commit"]);
        std::fs::write(
            dir.join("docs/long.txt"),
            long_text().replacen("line xxxxx\n", "line changed\n", 1),
        )
        .unwrap();
        let repo = Repo::open(dir).unwrap();
        (tmp, repo)
    }

    fn lines(s: &str) -> Vec<String> {
        crate::diff::split_lines(s)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn test_comment_on_expanded_line() {
        let (tmp, repo) = test_repo();
        let files = repo.working_tree(Options::default()).unwrap();
        let long = files.iter().find(|f| f.path == "docs/long.txt").unwrap();
        let old_lines = lines(&long_text());
        let new_lines = lines(&std::fs::read_to_string(tmp.path().join("docs/long.txt")).unwrap());
        let sources = [FileContents {
            file: long,
            old_lines: Some(&old_lines),
            new_lines: Some(&new_lines),
        }];
        let mut comments = Comments::new();
        // Line 30 is in the gap after the hunk, on both sides.
        let id = comments.add("docs/long.txt", Side::Old, 30, 30);
        comments.get_mut(id).unwrap().text = "Why?".into();
        let md = comments.markdown(&sources, "# Address these Review Comments");
        assert!(
            md.contains("(Old line 30)")
                && md.contains("   @@ -27,7 +27,7 @@")
                && md.contains(&format!("    30/30 | {}", old_lines[29])),
            "markdown:\n{md}"
        );
        assert!(md.starts_with("# Address these Review Comments\n\n1. **docs/long.txt**"));
    }

    // Further tests.

    #[test]
    fn test_comment_in_hunk_and_drafts() {
        let (_tmp, repo) = test_repo();
        let files = repo.working_tree(Options::default()).unwrap();
        let mut comments = Comments::new();
        let a = comments.add("docs/long.txt", Side::New, 0, 6);
        // The empty draft is reused, then moves.
        assert_eq!(comments.add("docs/long.txt", Side::New, 0, 6), a);
        let b = comments.add("docs/long.txt", Side::Old, 5, 6);
        assert_ne!(a, b);
        assert_eq!(comments.list.len(), 1);
        assert_eq!(comments.pending_count(), 0);
        assert_eq!(comments.markdown(&files, "P"), "");
        comments.get_mut(b).unwrap().text = "  Rename?  ".into();
        assert_eq!(comments.get(b).unwrap().label(), "Old lines 5-6");
        let md = comments.markdown(&files, "");
        let want = [
            "1. **docs/long.txt** (Old lines 5-6)",
            "",
            "   ```diff",
            "   @@ -3,7 +3,7 @@",
            "     3/3 | line xx",
            "     4/4 | line xxx",
            "     5/5 | line xxxx",
            "   -   6 | line xxxxx",
            "   +   6 | line changed",
            "     7/7 | line xxxxxx",
            "     8/8 | line ",
            "   ```",
            "",
            "   Rename?",
        ]
        .join("\n");
        assert_eq!(md, want);
        comments.prune(std::iter::empty());
        assert_eq!(comments.list.len(), 1);
        comments.delete(b);
        assert!(comments.list.is_empty());
    }

    #[test]
    fn test_no_context() {
        let c = Comment {
            id: 1,
            path: "gone.txt".into(),
            line: 3,
            text: "x".into(),
            ..Comment::default()
        };
        let files: [File; 0] = [];
        let md = comments_markdown(&files, &[c], "");
        assert!(md.contains("No patch context available."), "{md}");
    }
}
