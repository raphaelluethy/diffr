//! Changes within lines: the pairs of lines a hunk replaces, and the words
//! of each pair that differ.

use super::{Line, LineKind};
use unicode_general_category::{GeneralCategory, get_general_category};

/// A range of bytes of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    /// The range from `start` to `end`.
    pub fn new(start: usize, end: usize) -> Self {
        Range { start, end }
    }
}

/// A deleted line and the added line that replaces it, by their indices in
/// a hunk's lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Pair {
    pub del: usize,
    pub add: usize,
}

/// Pairs the deleted lines of each run of changes with the added lines that
/// follow them, in order, as a split view shows them side by side.
pub fn pairs(lines: &[Line]) -> Vec<Pair> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Del {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && lines[i].kind == LineKind::Del {
            i += 1;
        }
        let dels = i - start;
        let add_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Add {
            i += 1;
        }
        let adds = i - add_start;
        for k in 0..dels.min(adds) {
            out.push(Pair {
                del: start + k,
                add: add_start + k,
            });
        }
    }
    out
}

/// Bounds the lines compared word by word.
const MAX_WORD_DIFF_LEN: usize = 1000;

/// Returns the ranges of `a` and `b` that differ, word by word, or nothing
/// (two empty vectors) when the lines have too little in common for the
/// ranges to help.
pub fn word_diff(a: &str, b: &str) -> (Vec<Range>, Vec<Range>) {
    if a == b || a.len() > MAX_WORD_DIFF_LEN || b.len() > MAX_WORD_DIFF_LEN {
        return (Vec::new(), Vec::new());
    }
    let (ta, tb) = (tokens(a), tokens(b));
    let tok_a = |r: &Range| &a[r.start..r.end];
    let tok_b = |r: &Range| &b[r.start..r.end];
    // Trim the common prefix and suffix, then find the longest common
    // subsequence of what is left.
    let mut pre = 0;
    while pre < ta.len() && pre < tb.len() && tok_a(&ta[pre]) == tok_b(&tb[pre]) {
        pre += 1;
    }
    let mut suf = 0;
    while suf < ta.len() - pre
        && suf < tb.len() - pre
        && tok_a(&ta[ta.len() - 1 - suf]) == tok_b(&tb[tb.len() - 1 - suf])
    {
        suf += 1;
    }
    let ma = &ta[pre..ta.len() - suf];
    let mb = &tb[pre..tb.len() - suf];
    let mut keep_a = vec![false; ma.len()];
    let mut keep_b = vec![false; mb.len()];
    if ma.len() * mb.len() <= 250_000 {
        lcs(a, b, ma, mb, &mut keep_a, &mut keep_b);
    }
    let ra = changed(ma, &keep_a);
    let rb = changed(mb, &keep_b);
    // Lines that share little are rewritten rather than edited: marking
    // most of them would only add noise.
    let mut same = 0;
    for (i, k) in keep_a.iter().enumerate() {
        if *k {
            same += ma[i].end - ma[i].start;
        }
    }
    for t in &ta[..pre] {
        same += t.end - t.start;
    }
    for t in &ta[ta.len() - suf..] {
        same += t.end - t.start;
    }
    let total = trim_space(a).len().max(trim_space(b).len());
    if total > 0 && same * 10 < total * 3 {
        return (Vec::new(), Vec::new());
    }
    (ra, rb)
}

fn trim_space(s: &str) -> &str {
    s.trim_matches([' ', '\t'])
}

/// Merges the tokens not kept into ranges.
fn changed(toks: &[Range], keep: &[bool]) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::new();
    for (t, k) in toks.iter().zip(keep) {
        if *k {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.end == t.start
        {
            last.end = t.end;
            continue;
        }
        out.push(*t);
    }
    out
}

/// Marks the tokens of the longest common subsequence of `ta` and `tb`.
fn lcs(a: &str, b: &str, ta: &[Range], tb: &[Range], keep_a: &mut [bool], keep_b: &mut [bool]) {
    let (n, m) = (ta.len(), tb.len());
    if n == 0 || m == 0 {
        return;
    }
    let eq = |i: usize, j: usize| a[ta[i].start..ta[i].end] == b[tb[j].start..tb[j].end];
    let w = m + 1;
    let mut dp = vec![0i32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if eq(i, j) {
                dp[(i + 1) * w + j + 1] + 1
            } else {
                dp[(i + 1) * w + j].max(dp[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if eq(i, j) {
            keep_a[i] = true;
            keep_b[j] = true;
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// Splits a line into words, runs of spaces, and single other characters.
fn tokens(s: &str) -> Vec<Range> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let r = s[i..].chars().next().unwrap_or('\0');
        let mut j = i + r.len_utf8();
        if is_word(r) {
            for r2 in s[j..].chars() {
                if !is_word(r2) {
                    break;
                }
                j += r2.len_utf8();
            }
        } else if r == ' ' || r == '\t' {
            while j < s.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
        }
        out.push(Range::new(i, j));
        i = j;
    }
    out
}

/// A word character: `_`, a letter or a decimal digit, as Go's
/// `unicode.IsLetter` and `unicode.IsDigit` decide.
fn is_word(r: char) -> bool {
    if r == '_' || r.is_ascii_alphanumeric() {
        return true;
    }
    if r.is_ascii() {
        return false;
    }
    use GeneralCategory::*;
    matches!(
        get_general_category(r),
        UppercaseLetter
            | LowercaseLetter
            | TitlecaseLetter
            | ModifierLetter
            | OtherLetter
            | DecimalNumber
    )
}
