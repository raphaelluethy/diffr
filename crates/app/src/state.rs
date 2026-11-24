//! The review's model: the files as they show, and the rows of the diff
//! surface.

use std::collections::HashMap;
use std::sync::Arc;

use diffr_git::diff::{Range as WordRange, pairs, word_diff};
use diffr_git::{File, LineKind, Status};
use diffr_highlight::Seg;

/// What a window reviews.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Uncommitted changes.
    WorkingTree,
    /// A commit, against its first parent.
    Commit(String),
    /// The work tree since it branched from a branch.
    Branch(String),
    /// A pull request on GitHub, as owner/repo#123.
    Pull(String),
}

impl Source {
    pub fn is_pull(&self) -> bool {
        matches!(self, Source::Pull(_))
    }
}

pub use diffr_git::Side;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GapShown {
    pub top: u32,
    pub bottom: u32,
}

/// A changed picture.
#[derive(Clone)]
pub struct Picture {
    pub image: Arc<gpui::Image>,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

/// A changed file as the review shows it.
pub struct FileState {
    pub file: File,
    pub old_lines: Option<Vec<String>>,
    pub new_lines: Option<Vec<String>>,
    pub old_hl: Option<Vec<Vec<Seg>>>,
    pub new_hl: Option<Vec<Vec<Seg>>>,
    pub loaded: bool,
    pub collapsed: bool,
    pub expanded: HashMap<usize, GapShown>,
    pub words: Option<HashMap<(usize, usize), Vec<WordRange>>>,
    pub old_image: Option<Picture>,
    pub new_image: Option<Picture>,
    pub metrics: Option<FileMetrics>,
    /// The index of the file's group, -1 for none.
    pub group: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct FileMetrics {
    pub digits: usize,
    pub max_cols: usize,
}

pub const TAB_WIDTH: usize = 4;

/// How many columns a line takes, tabs expanded.
pub fn columns(s: &str) -> usize {
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            col += TAB_WIDTH - col % TAB_WIDTH;
        } else {
            col += 1;
        }
    }
    col
}

impl FileState {
    pub fn new(file: File) -> Self {
        Self {
            file,
            old_lines: None,
            new_lines: None,
            old_hl: None,
            new_hl: None,
            loaded: false,
            collapsed: false,
            expanded: HashMap::new(),
            words: None,
            old_image: None,
            new_image: None,
            metrics: None,
            group: -1,
        }
    }

    pub fn one_sided(&self) -> bool {
        matches!(self.file.status, Status::Added | Status::Untracked | Status::Deleted)
    }

    pub fn can_expand(&self) -> bool {
        self.new_lines.is_some() || self.old_lines.is_some()
    }

    pub fn countable(&self) -> bool {
        !self.file.binary && !self.file.directory && !self.file.too_large
    }

    pub fn line_at(&self, hunk: i32, i: i32) -> Option<&diffr_git::Line> {
        if hunk < 0 || i < 0 {
            return None;
        }
        self.file.hunks.get(hunk as usize)?.lines.get(i as usize)
    }

    pub fn gap_before(&self, i: usize) -> diffr_git::diff::Gap {
        self.file.gap_before(
            i,
            self.old_lines.as_ref().map(|l| l.len()),
            self.new_lines.as_ref().map(|l| l.len()),
        )
    }

    pub fn context_text(&self, old: u32, new: u32) -> &str {
        if new > 0
            && let Some(l) = self.new_lines.as_ref().and_then(|l| l.get(new as usize - 1))
        {
            return l;
        }
        if old > 0
            && let Some(l) = self.old_lines.as_ref().and_then(|l| l.get(old as usize - 1))
        {
            return l;
        }
        ""
    }

    pub fn compute_words(&mut self) {
        if self.words.is_some() {
            return;
        }
        let mut words = HashMap::new();
        for (hi, h) in self.file.hunks.iter().enumerate() {
            for p in pairs(&h.lines) {
                let (ra, rb) = word_diff(&h.lines[p.del].text, &h.lines[p.add].text);
                if !ra.is_empty() || !rb.is_empty() {
                    words.insert((hi, p.del), ra);
                    words.insert((hi, p.add), rb);
                }
            }
        }
        self.words = Some(words);
    }

    pub fn metrics(&mut self) -> FileMetrics {
        if let Some(m) = self.metrics {
            return m;
        }
        let mut max_num = 0usize;
        let mut max_cols = 0usize;
        for h in &self.file.hunks {
            max_num = max_num
                .max((h.old_start + h.old_lines) as usize)
                .max((h.new_start + h.new_lines) as usize);
            for l in &h.lines {
                max_cols = max_cols.max(columns(&l.text));
            }
        }
        if let Some(l) = &self.old_lines {
            max_num = max_num.max(l.len());
            max_cols = l.iter().fold(max_cols, |m, s| m.max(columns(s)));
        }
        if let Some(l) = &self.new_lines {
            max_num = max_num.max(l.len());
            max_cols = l.iter().fold(max_cols, |m, s| m.max(columns(s)));
        }
        let m = FileMetrics { digits: max_num.to_string().len().max(2), max_cols };
        if self.loaded {
            self.metrics = Some(m);
        }
        m
    }

    /// The first changed line of a file, to open it there.
    pub fn first_line(&self) -> u32 {
        for h in &self.file.hunks {
            for l in &h.lines {
                if l.kind == LineKind::Add {
                    return l.new;
                }
            }
            if h.new_start > 0 {
                return h.new_start;
            }
        }
        0
    }

    /// Why a file's lines do not show, None when they do.
    pub fn note(&self) -> Option<String> {
        let f = &self.file;
        let none = f.hunks.is_empty();
        Some(if !f.note.is_empty() {
            f.note.clone()
        } else if f.binary {
            "Binary file changed.".into()
        } else if f.too_large {
            "File is too large, so Diffr skipped rendering it.".into()
        } else if none && f.old_path != f.path {
            "File renamed without changes.".into()
        } else if none && !f.mode_change.is_empty() {
            format!("File mode changed: {}.", f.mode_change)
        } else if none && matches!(f.status, Status::Added | Status::Untracked) {
            "Empty file added.".into()
        } else if none && f.status == Status::Deleted {
            "Empty file deleted.".into()
        } else if none {
            "No changes to show.".into()
        } else {
            return None;
        })
    }

    /// Whether a file has something to show: files whose only changes were
    /// whitespace, hidden, have nothing.
    pub fn has_content(&self) -> bool {
        let f = &self.file;
        !f.hunks.is_empty()
            || f.binary
            || f.directory
            || f.too_large
            || !f.note.is_empty()
            || f.status != Status::Modified
            || f.old_path != f.path
            || !f.mode_change.is_empty()
    }
}

/// What a row of the diff surface shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowKind {
    Header,
    Note,
    Image,
    Gap,
    Line,
    Comment,
    End,
    Commit,
    Pull,
    Summary,
    Group,
    Thread,
    AiNote,
    FileSummary,
}

/// A row of the diff surface.
#[derive(Clone, Debug)]
pub struct Row {
    pub kind: RowKind,
    pub file: usize,
    /// For lines of hunks: the hunk and the lines' indices on the left (old)
    /// and the right (new), -1 for none. Unified rows use `a` alone.
    /// Expanded context has hunk -1 and its numbers in old and new.
    pub hunk: i32,
    pub a: i32,
    pub b: i32,
    pub old: u32,
    pub new: u32,
    /// For gaps: which gap and how many lines it hides. For groups: the group.
    pub gap: i32,
    pub count: u32,
    /// For comments, threads, notes: their index in their lists.
    pub item: usize,
}

impl Row {
    pub fn new(kind: RowKind, file: usize) -> Self {
        Self { kind, file, hunk: -1, a: -1, b: -1, old: 0, new: 0, gap: 0, count: 0, item: 0 }
    }

    pub fn banner(&self) -> bool {
        matches!(self.kind, RowKind::Commit | RowKind::Pull | RowKind::Summary)
    }
}

/// How many lines a click on a gap's arrows shows, and the most unchanged
/// lines between hunks shown without asking.
pub const EXPAND_STEP: u32 = 100;
pub const INLINE_GAP: u32 = 12;

/// A line of one side of a row.
#[derive(Clone, Default)]
pub struct LineSide<'a> {
    pub present: bool,
    pub kind: Option<LineKind>,
    pub num: u32,
    pub text: &'a str,
    pub segs: &'a [Seg],
    pub words: &'a [WordRange],
    pub hunk: i32,
    pub index: i32,
}

impl FileState {
    pub fn side_of(&self, r: &Row, s: Side, split: bool) -> LineSide<'_> {
        if r.hunk < 0 {
            let text = self.context_text(r.old, r.new);
            let mut ls = LineSide {
                present: true,
                kind: Some(LineKind::Context),
                text,
                hunk: -1,
                index: -1,
                ..Default::default()
            };
            let old_seg = self.old_hl.as_ref().and_then(|h| h.get((r.old as usize).wrapping_sub(1)));
            let new_seg = self.new_hl.as_ref().and_then(|h| h.get((r.new as usize).wrapping_sub(1)));
            if s == Side::Old {
                ls.num = r.old;
                ls.segs = old_seg.or(new_seg).map(|v| v.as_slice()).unwrap_or(&[]);
            } else {
                ls.num = r.new;
                ls.segs = new_seg.map(|v| v.as_slice()).unwrap_or(&[]);
            }
            return ls;
        }
        let idx = if s == Side::New && split { r.b } else { r.a };
        let Some(l) = self.line_at(r.hunk, idx) else {
            return LineSide::default();
        };
        let mut ls = LineSide {
            present: true,
            kind: Some(l.kind),
            text: &l.text,
            hunk: r.hunk,
            index: idx,
            ..Default::default()
        };
        if l.kind == LineKind::Del {
            ls.num = l.old;
            if let Some(v) = self.old_hl.as_ref().and_then(|h| h.get((l.old as usize).wrapping_sub(1))) {
                ls.segs = v;
            }
        } else {
            ls.num = l.new;
            if s == Side::Old && l.kind == LineKind::Context {
                ls.num = l.old;
            }
            if l.new > 0
                && let Some(v) = self.new_hl.as_ref().and_then(|h| h.get(l.new as usize - 1))
            {
                ls.segs = v;
            }
        }
        if let Some(w) = self.words.as_ref().and_then(|w| w.get(&(r.hunk as usize, idx as usize))) {
            ls.words = w;
        }
        ls
    }
}

/// Whether the runes of query appear in s in order.
pub fn fuzzy_match(s: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let s = s.to_lowercase();
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut i = 0;
    for c in s.chars() {
        if i < q.len() && c == q[i] {
            i += 1;
        }
    }
    i >= q.len()
}

/// Where query is in text, ignoring case.
pub fn find_ranges(text: &str, query: &str) -> Vec<WordRange> {
    let query = query.trim();
    if query.is_empty() || query.len() > text.len() {
        return Vec::new();
    }
    let (hay, q) = {
        let l = text.to_lowercase();
        if l.len() == text.len() { (l, query.to_lowercase()) } else { (text.to_string(), query.to_string()) }
    };
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(i) = hay[start..].find(&q) {
        out.push(WordRange { start: start + i, end: start + i + q.len() });
        start += i + q.len();
    }
    out
}

/// How long ago a time was: just now, 5m ago, 3d ago.
pub fn relative_time(now: i64, t: i64) -> String {
    if t == 0 {
        return String::new();
    }
    let d = now - t;
    const M: i64 = 60;
    const H: i64 = 3600;
    const D: i64 = 86400;
    if d < M {
        "just now".into()
    } else if d < H {
        format!("{}m ago", d / M)
    } else if d < D {
        format!("{}h ago", d / H)
    } else if d < 30 * D {
        format!("{}d ago", d / D)
    } else if d < 365 * D {
        format!("{}mo ago", d / (30 * D))
    } else {
        format!("{}y ago", d / (365 * D))
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Writes a size: 512 B, 1.5 KiB, 12 MiB.
pub fn format_bytes(n: usize) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.;
    let mut unit = "KiB";
    for u in ["MiB", "GiB"] {
        if v < 1024. {
            break;
        }
        v /= 1024.;
        unit = u;
    }
    if v < 10. { format!("{v:.1} {unit}") } else { format!("{v:.0} {unit}") }
}

pub fn short_hash(h: &str) -> &str {
    if h.len() > 7 { &h[..7] } else { h }
}

/// Writes the home directory as ~.
pub fn abbreviate_home(p: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if p == home {
            return "~".into();
        }
        if let Some(rest) = p.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    p.to_string()
}

pub fn is_image(path: &str) -> bool {
    let lower = path.to_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".ico"]
        .iter()
        .any(|e| lower.ends_with(e))
}

/// The next eligible file, wrapping once through the review's order.
pub fn next_matching_file(count: usize, current: usize, mut eligible: impl FnMut(usize) -> bool) -> Option<usize> {
    (1..=count).map(|step| (current + step) % count).find(|&i| eligible(i))
}

#[cfg(test)]
mod navigation_tests {
    use super::next_matching_file;

    #[test]
    fn next_unviewed_respects_results_and_wraps() {
        let viewed = [true, true, false, false];
        let visible = [true, true, false, false];
        // Unreviewed files outside current results must not enable navigation.
        assert_eq!(next_matching_file(4, 0, |i| visible[i] && !viewed[i]), None);
        assert_eq!(next_matching_file(4, 3, |i| i == 1), Some(1));
        assert_eq!(next_matching_file(4, 0, |i| !viewed[i]), Some(2));
        assert_eq!(next_matching_file(0, 0, |_| panic!("empty review")), None);
    }
}
