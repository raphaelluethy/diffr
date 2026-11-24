//! The model of a set of changed files, and the parser of the patches git
//! prints into it.

mod words;

pub use words::{Pair, Range, pairs, word_diff};

use serde::{Deserialize, Serialize};

/// How a file changed, as git's status letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Status {
    /// `M`
    #[default]
    Modified,
    /// `A`
    Added,
    /// `D`
    Deleted,
    /// `R`
    Renamed,
    /// `C`
    Copied,
    /// `T`
    TypeChanged,
    /// `?`
    Untracked,
    /// `U`
    Conflict,
}

impl Status {
    /// The status as a word, as file headers show it.
    pub fn label(self) -> &'static str {
        match self {
            Status::Added => "Added",
            Status::Deleted => "Deleted",
            Status::Renamed => "Renamed",
            Status::Copied => "Copied",
            Status::TypeChanged => "Type changed",
            Status::Untracked => "Untracked",
            Status::Conflict => "Conflict",
            Status::Modified => "Modified",
        }
    }

    /// git's status letter.
    pub fn letter(self) -> char {
        match self {
            Status::Modified => 'M',
            Status::Added => 'A',
            Status::Deleted => 'D',
            Status::Renamed => 'R',
            Status::Copied => 'C',
            Status::TypeChanged => 'T',
            Status::Untracked => '?',
            Status::Conflict => 'U',
        }
    }

    /// The status of a git status letter, if it is one.
    pub fn from_letter(c: char) -> Option<Status> {
        Some(match c {
            'M' => Status::Modified,
            'A' => Status::Added,
            'D' => Status::Deleted,
            'R' => Status::Renamed,
            'C' => Status::Copied,
            'T' => Status::TypeChanged,
            '?' => Status::Untracked,
            'U' => Status::Conflict,
            _ => return None,
        })
    }
}

/// What a line of a hunk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LineKind {
    /// An unchanged line, on both sides.
    #[default]
    Context,
    /// An added line, on the new side only.
    Add,
    /// A deleted line, on the old side only.
    Del,
}

/// A line of a hunk. `old` and `new` are its numbers in the old and the new
/// file, 0 on the side it is not in.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Line {
    pub kind: LineKind,
    pub old: u32,
    pub new: u32,
    /// The text without its `+`/`-`/` ` prefix and line ending.
    pub text: String,
    /// Set on the last line of a side without a final newline.
    pub no_newline: bool,
}

/// A run of changed lines with the context around them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// The function or heading git shows after the `@@`.
    pub section: String,
    pub lines: Vec<Line>,
}

impl Hunk {
    /// The numbers of the last lines of the hunk on each side (old, new).
    pub fn last_lines(&self) -> (u32, u32) {
        let o = if self.old_lines == 0 {
            self.old_start
        } else {
            (self.old_start + self.old_lines).saturating_sub(1)
        };
        let n = if self.new_lines == 0 {
            self.new_start
        } else {
            (self.new_start + self.new_lines).saturating_sub(1)
        };
        (o, n)
    }
}

/// A run of unchanged lines between hunks, or before the first or after the
/// last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Gap {
    /// The index of the hunk the gap comes before; `hunks.len()` for the gap
    /// after the last.
    pub index: usize,
    /// The number of its first line in the old file.
    pub old_start: u32,
    /// The number of its first line in the new file.
    pub new_start: u32,
    /// How many lines it holds.
    pub count: u32,
}

/// A changed file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    /// The path before a rename or a copy, else `path`.
    pub old_path: String,
    pub status: Status,
    pub binary: bool,
    /// Mode change, as `"100644 → 100755"`, if any.
    pub mode_change: String,
    pub hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
    /// Set when the patch was dropped for its size.
    pub too_large: bool,
    /// An untracked directory shown collapsed, as `node_modules`, whose files
    /// are not listed.
    pub directory: bool,
    /// A file a tool wrote, as a lockfile: it starts collapsed.
    pub generated: bool,
    /// Explains why the lines are not shown, if they are not.
    pub note: String,
    /// Identifies the content of the change: viewed files are remembered by
    /// it, and show again once it changes.
    pub fingerprint: String,
}

impl File {
    /// The base name of the file.
    pub fn name(&self) -> &str {
        match self.path.rfind('/') {
            Some(i) => &self.path[i + 1..],
            None => &self.path,
        }
    }

    /// The directory of the file, with a trailing slash, or `""`.
    pub fn dir(&self) -> &str {
        match self.path.rfind('/') {
            Some(i) => &self.path[..i + 1],
            None => "",
        }
    }

    /// Sets `additions` and `deletions` from the hunks.
    pub fn count(&mut self) {
        let (mut a, mut d) = (0, 0);
        for h in &self.hunks {
            for l in &h.lines {
                match l.kind {
                    LineKind::Add => a += 1,
                    LineKind::Del => d += 1,
                    LineKind::Context => {}
                }
            }
        }
        self.additions = a;
        self.deletions = d;
    }

    /// Whether the file is all new or all gone, with lines on one side only.
    pub fn one_sided(&self) -> bool {
        matches!(
            self.status,
            Status::Added | Status::Untracked | Status::Deleted
        )
    }

    /// The gap of unchanged lines before hunk `i`; `i == hunks.len()` is the
    /// gap after the last, known once the contents are loaded: pass the
    /// number of lines of the old and the new file when they are.
    pub fn gap_before(&self, i: usize, old_len: Option<usize>, new_len: Option<usize>) -> Gap {
        let (prev_old, prev_new) = if i > 0 {
            self.hunks[i - 1].last_lines()
        } else {
            (0, 0)
        };
        let mut g = Gap {
            index: i,
            old_start: prev_old + 1,
            new_start: prev_new + 1,
            count: 0,
        };
        if let Some(h) = self.hunks.get(i) {
            let first = if h.old_lines == 0 {
                h.old_start + 1
            } else {
                h.old_start
            };
            g.count = first.saturating_sub(g.old_start);
            return g;
        }
        if let Some(n) = new_len {
            g.count = (n as u32).saturating_sub(prev_new);
        } else if let Some(n) = old_len {
            g.count = (n as u32).saturating_sub(prev_old);
        }
        g
    }
}

/// Hashes what identifies a change: the first 16 hex digits of the SHA-1 of
/// the parts, each followed by a NUL.
pub(crate) fn fingerprint(parts: &[&[u8]]) -> String {
    let mut h = sha1_smol::Sha1::new();
    for p in parts {
        h.update(p);
        h.update(&[0]);
    }
    let mut s = h.digest().to_string();
    s.truncate(16);
    s
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Parses the output of `git diff` (or `git show`) with patches.
pub fn parse(patch: &[u8]) -> Vec<File> {
    struct State {
        files: Vec<File>,
        cur: Option<File>,
        hunk: Option<Hunk>,
        raw: Vec<u8>,
    }
    fn finish(s: &mut State) {
        let Some(mut cur) = s.cur.take() else {
            return;
        };
        if let Some(h) = s.hunk.take() {
            cur.hunks.push(h);
        }
        cur.count();
        cur.fingerprint = fingerprint(&[cur.path.as_bytes(), &s.raw]);
        s.files.push(cur);
        s.raw.clear();
    }

    let mut s = State {
        files: Vec::new(),
        cur: None,
        hunk: None,
        raw: Vec::new(),
    };
    let (mut old_n, mut new_n) = (0u32, 0u32);
    // The lines of the hunk still to come, on each side.
    let (mut rem_old, mut rem_new) = (0i64, 0i64);

    for line in scan_lines(patch) {
        if line.starts_with(b"diff --git ")
            || line.starts_with(b"diff --cc ")
            || line.starts_with(b"diff --combined ")
        {
            finish(&mut s);
            let (old, new) = split_git_header(line);
            s.cur = Some(File {
                status: Status::Modified,
                old_path: lossy(&old),
                path: lossy(&new),
                ..File::default()
            });
            s.raw.extend_from_slice(line);
            s.raw.push(b'\n');
            continue;
        }
        let Some(cur) = s.cur.as_mut() else {
            continue;
        };
        s.raw.extend_from_slice(line);
        s.raw.push(b'\n');
        // Lines of files with CRLF endings show without the CR.
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(hunk) = s.hunk.as_mut()
            && (rem_old > 0 || rem_new > 0)
        {
            if let Some(rest) = line.strip_prefix(b"+") {
                new_n += 1;
                rem_new -= 1;
                hunk.lines.push(Line {
                    kind: LineKind::Add,
                    new: new_n,
                    text: lossy(rest),
                    ..Line::default()
                });
                continue;
            }
            if let Some(rest) = line.strip_prefix(b"-") {
                old_n += 1;
                rem_old -= 1;
                hunk.lines.push(Line {
                    kind: LineKind::Del,
                    old: old_n,
                    text: lossy(rest),
                    ..Line::default()
                });
                continue;
            }
            if line.starts_with(b" ") || line.is_empty() {
                old_n += 1;
                new_n += 1;
                rem_old -= 1;
                rem_new -= 1;
                let text = if line.is_empty() { line } else { &line[1..] };
                hunk.lines.push(Line {
                    kind: LineKind::Context,
                    old: old_n,
                    new: new_n,
                    text: lossy(text),
                    no_newline: false,
                });
                continue;
            }
        }
        if line.starts_with(b"\\") {
            // "\ No newline at end of file" after the line it is about.
            if let Some(last) = s.hunk.as_mut().and_then(|h| h.lines.last_mut()) {
                last.no_newline = true;
            }
            continue;
        }
        if line.starts_with(b"@@") {
            if let Some(h) = s.hunk.take() {
                cur.hunks.push(h);
            }
            s.hunk = parse_hunk_header(line);
            let Some(h) = s.hunk.as_ref() else {
                continue;
            };
            old_n = h.old_start.saturating_sub(1);
            new_n = h.new_start.saturating_sub(1);
            if h.old_lines == 0 {
                old_n = h.old_start;
            }
            if h.new_lines == 0 {
                new_n = h.new_start;
            }
            rem_old = h.old_lines as i64;
            rem_new = h.new_lines as i64;
        } else if line.starts_with(b"new file mode") {
            cur.status = Status::Added;
        } else if line.starts_with(b"deleted file mode") {
            cur.status = Status::Deleted;
        } else if let Some(rest) = line.strip_prefix(b"old mode ") {
            cur.mode_change = lossy(rest);
        } else if let Some(rest) = line.strip_prefix(b"new mode ") {
            cur.mode_change.push_str(" → ");
            cur.mode_change.push_str(&lossy(rest));
        } else if let Some(rest) = line.strip_prefix(b"rename from ") {
            cur.status = Status::Renamed;
            cur.old_path = lossy(&unquote(rest));
        } else if let Some(rest) = line.strip_prefix(b"rename to ") {
            cur.status = Status::Renamed;
            cur.path = lossy(&unquote(rest));
        } else if let Some(rest) = line.strip_prefix(b"copy from ") {
            cur.status = Status::Copied;
            cur.old_path = lossy(&unquote(rest));
        } else if let Some(rest) = line.strip_prefix(b"copy to ") {
            cur.status = Status::Copied;
            cur.path = lossy(&unquote(rest));
        } else if let Some(rest) = line.strip_prefix(b"--- ") {
            let p = strip_prefix(&unquote(rest)).to_vec();
            if !p.is_empty() {
                cur.old_path = lossy(&p);
            }
        } else if let Some(rest) = line.strip_prefix(b"+++ ") {
            let p = strip_prefix(&unquote(rest)).to_vec();
            if !p.is_empty() {
                cur.path = lossy(&p);
            }
        } else if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
            cur.binary = true;
        }
    }
    finish(&mut s);
    for f in &mut s.files {
        if f.path.is_empty() {
            f.path = f.old_path.clone();
        }
        if f.old_path.is_empty() {
            f.old_path = f.path.clone();
        }
    }
    s.files
}

/// Splits data into lines as Go's `bufio.ScanLines` does: on `\n`, each
/// without one trailing `\r`, and no empty line after a final newline.
fn scan_lines(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let empty = data.is_empty();
    let data = data.strip_suffix(b"\n").unwrap_or(data);
    data.split(|&b| b == b'\n')
        .filter(move |_| !empty)
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
}

/// Takes the `a/` or `b/` off a path of `---`/`+++` lines, `""` for
/// `/dev/null`.
fn strip_prefix(p: &[u8]) -> &[u8] {
    let p = p.strip_suffix(b"\t").unwrap_or(p);
    if p == b"/dev/null" {
        return b"";
    }
    if p.len() > 2 && matches!(p[0], b'a' | b'b' | b'i' | b'w' | b'c' | b'o') && p[1] == b'/' {
        return &p[2..];
    }
    p
}

/// Returns the paths of `diff --git a/x b/y`.
fn split_git_header(line: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut rest = line;
    for p in [&b"diff --git "[..], b"diff --cc ", b"diff --combined "] {
        rest = rest.strip_prefix(p).unwrap_or(rest);
    }
    if rest.starts_with(b"\"") {
        // Quoted: "a/x" "b/y", or "a/x" b/y.
        if let Some(end) = closing_quote(rest) {
            let a = unquote(&rest[..end + 1]);
            let b = unquote(rest[end + 1..].trim_ascii());
            return (strip_prefix(&a).to_vec(), strip_prefix(&b).to_vec());
        }
    }
    if !rest.contains(&b' ') {
        // diff --cc path
        return (rest.to_vec(), rest.to_vec());
    }
    // a/x b/x: both halves are the same length when the path is unchanged.
    let n = rest.len();
    if n % 2 == 1 {
        let (a, b) = (&rest[..n / 2], &rest[n / 2 + 1..]);
        if strip_prefix(a) == strip_prefix(b) {
            return (strip_prefix(a).to_vec(), strip_prefix(b).to_vec());
        }
    }
    if let Some(i) = find(rest, b" b/") {
        return (
            strip_prefix(&rest[..i]).to_vec(),
            strip_prefix(&rest[i + 1..]).to_vec(),
        );
    }
    (rest.to_vec(), rest.to_vec())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn closing_quote(s: &[u8]) -> Option<usize> {
    let mut i = 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 1,
            b'"' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Undoes git's C-style quoting of paths; a string that is not quoted, or
/// whose quoting is invalid, loses only its quotes.
fn unquote(s: &[u8]) -> Vec<u8> {
    if s.len() < 2 || s[0] != b'"' || s[s.len() - 1] != b'"' {
        return s.to_vec();
    }
    c_unquote(&s[1..s.len() - 1]).unwrap_or_else(|| s[1..s.len() - 1].to_vec())
}

/// Decodes the escapes of the inside of a double-quoted Go/C string, as
/// `strconv.Unquote` does; `None` when it is not valid.
fn c_unquote(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    let hex = |b: &[u8]| -> Option<u32> {
        let s = std::str::from_utf8(b).ok()?;
        if !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(s, 16).ok()
    };
    while i < s.len() {
        let c = s[i];
        match c {
            b'"' | b'\n' => return None,
            b'\\' => {
                let e = *s.get(i + 1)?;
                i += 2;
                match e {
                    b'a' => out.push(7),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'v' => out.push(11),
                    b'\\' => out.push(b'\\'),
                    b'"' => out.push(b'"'),
                    b'0'..=b'7' => {
                        let d = s.get(i - 1..i + 2)?;
                        if !d.iter().all(|b| (b'0'..=b'7').contains(b)) {
                            return None;
                        }
                        let v = (d[0] - b'0') as u32 * 64
                            + (d[1] - b'0') as u32 * 8
                            + (d[2] - b'0') as u32;
                        if v > 255 {
                            return None;
                        }
                        out.push(v as u8);
                        i += 2;
                    }
                    b'x' => {
                        let v = hex(s.get(i..i + 2)?)?;
                        out.push(v as u8);
                        i += 2;
                    }
                    b'u' | b'U' => {
                        let n = if e == b'u' { 4 } else { 8 };
                        let v = hex(s.get(i..i + n)?)?;
                        let ch = char::from_u32(v)?;
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        i += n;
                    }
                    _ => return None,
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(out)
}

/// Parses `@@ -a,b +c,d @@ section`.
fn parse_hunk_header(line: &[u8]) -> Option<Hunk> {
    let end = find(&line[2..], b"@@")?;
    let ranges = String::from_utf8_lossy(&line[2..end + 2]).into_owned();
    let mut h = Hunk {
        section: lossy(line[end + 4..].trim_ascii()),
        ..Hunk::default()
    };
    for r in ranges.split_whitespace() {
        if r.len() < 2 {
            continue;
        }
        let (start, count) = parse_range(&r[1..]);
        match r.as_bytes()[0] {
            b'-' => (h.old_start, h.old_lines) = (start, count),
            b'+' => (h.new_start, h.new_lines) = (start, count),
            _ => {}
        }
    }
    Some(h)
}

fn parse_range(s: &str) -> (u32, u32) {
    match s.split_once(',') {
        None => (s.parse().unwrap_or(0), 1),
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
    }
}

/// Makes the change of a file that is all new, as an untracked file, from
/// its content.
pub fn new_file_from_content(path: &str, content: &[u8], status: Status) -> File {
    let mut f = File {
        path: path.to_string(),
        old_path: path.to_string(),
        status,
        ..File::default()
    };
    if is_binary(content) {
        f.binary = true;
        f.fingerprint = fingerprint(&[path.as_bytes(), content]);
        return f;
    }
    let text = String::from_utf8_lossy(content);
    let lines = split_lines(&text);
    if !lines.is_empty() {
        let mut h = Hunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: lines.len() as u32,
            ..Hunk::default()
        };
        h.lines = lines
            .into_iter()
            .enumerate()
            .map(|(i, l)| Line {
                kind: LineKind::Add,
                new: i as u32 + 1,
                text: l.to_string(),
                ..Line::default()
            })
            .collect();
        if !content.ends_with(b"\n")
            && let Some(last) = h.lines.last_mut()
        {
            last.no_newline = true;
        }
        f.hunks = vec![h];
    }
    f.count();
    f.fingerprint = fingerprint(&[path.as_bytes(), content]);
    f
}

/// Whether content looks binary, as git decides: a NUL byte in its first
/// 8000 bytes.
pub fn is_binary(content: &[u8]) -> bool {
    content[..content.len().min(8000)].contains(&0)
}

/// Splits text into lines without their line endings; a final newline does
/// not start another line.
pub fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

#[cfg(test)]
mod tests;
