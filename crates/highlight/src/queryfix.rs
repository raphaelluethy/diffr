//! Rewrites of bundled highlight queries written for other editors.
//!
//! Some grammars ship queries that rely on Neovim-only predicates
//! (`#lua-match?`, `#has-ancestor?`), which tree-sitter-highlight does not
//! evaluate, so their patterns would match everything; and some are written
//! for editors where the *first* pattern matching a node wins, while
//! tree-sitter-highlight lets the last one win.

/// The predicates tree-sitter evaluates, or tree-sitter-highlight reads.
const SUPPORTED: &[&str] = &[
    "eq?",
    "not-eq?",
    "any-eq?",
    "any-not-eq?",
    "match?",
    "not-match?",
    "any-match?",
    "any-not-match?",
    "any-of?",
    "not-any-of?",
    "set!",
    "is?",
    "is-not?",
];

/// Whether `query` needs [`fix`]: it uses a predicate tree-sitter does not
/// evaluate.
pub(crate) fn needs_fix(query: &str) -> bool {
    predicates(query).any(|p| !SUPPORTED.contains(&p))
}

/// `query` with `#lua-match?` turned into `#match?`, the patterns using
/// other unsupported predicates dropped, and, if `reverse`, the patterns in
/// reverse order, so that the first pattern matching a node wins under
/// tree-sitter-highlight's last-wins rule.
pub(crate) fn fix(query: &str, reverse: bool) -> String {
    let mut patterns: Vec<String> = split(query)
        .into_iter()
        .map(translate_lua_match)
        .filter(|p| predicates(p).all(|name| SUPPORTED.contains(&name)))
        .collect();
    if reverse {
        patterns.reverse();
    }
    patterns.join("\n\n")
}

/// The names of the predicates (`eq?`, `set!`, …) a query uses: the words
/// right after a `(#`.
fn predicates(query: &str) -> impl Iterator<Item = &str> {
    query.match_indices("(#").filter_map(move |(i, _)| {
        let tail = &query[i + 2..];
        let len = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        let name = &tail[..len + usize::from(tail[len..].starts_with(['?', '!']))];
        (len > 0 && name.len() > len).then_some(name)
    })
}

/// Splits a query into its top-level patterns, without the comments
/// between them.
fn split(query: &str) -> Vec<&str> {
    let b = query.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_whitespace() => i += 1,
            b';' => i = skip_line(b, i),
            _ => {
                let start = i;
                i = atom(b, i);
                // Quantifiers and captures after the atom.
                loop {
                    let mut j = i;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    match b.get(j) {
                        Some(b'?' | b'*' | b'+') => i = j + 1,
                        Some(b'@') => {
                            j += 1;
                            while j < b.len() && is_name(b[j]) {
                                j += 1;
                            }
                            i = j;
                        }
                        _ => break,
                    }
                }
                out.push(&query[start..i]);
            }
        }
    }
    out
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-')
}

fn skip_line(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// The end of the atom starting at `i`: a balanced group, a string, or a
/// bare word.
fn atom(b: &[u8], mut i: usize) -> usize {
    match b[i] {
        b'(' | b'[' => {
            let mut depth = 0usize;
            while i < b.len() {
                match b[i] {
                    b'"' => {
                        i = skip_string(b, i);
                        continue;
                    }
                    b';' => {
                        i = skip_line(b, i);
                        continue;
                    }
                    b'(' | b'[' => depth += 1,
                    b')' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return i + 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            b.len()
        }
        b'"' => skip_string(b, i),
        _ => {
            let start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'(' | b'[' | b';')
            {
                i += 1;
            }
            i.max(start + 1)
        }
    }
}

/// A pattern with its `#lua-match?` (and `#not-lua-match?`) predicates
/// turned into regex ones.
fn translate_lua_match(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(i) = rest.find("lua-match?") {
        let neg = rest[..i].ends_with("#not-");
        out.push_str(&rest[..i - if neg { 4 } else { 0 }]);
        out.push_str(if neg { "not-match?" } else { "match?" });
        rest = &rest[i + "lua-match?".len()..];
        // The next string is the pattern (after the capture).
        let Some(q) = rest.find('"') else { break };
        out.push_str(&rest[..q]);
        let b = rest.as_bytes();
        let end = skip_string(b, q);
        let body = &rest[q + 1..end.saturating_sub(1).max(q + 1)];
        out.push('"');
        out.push_str(&lua_to_regex(body));
        out.push('"');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// A Lua pattern as a Rust regex, escaped for a query string.
fn lua_to_regex(lua: &str) -> String {
    let mut re = String::new();
    let mut in_class = false;
    let mut chars = lua.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                let Some(k) = chars.next() else { break };
                let set = match k {
                    'a' => Some("A-Za-z"),
                    'd' => Some("0-9"),
                    'l' => Some("a-z"),
                    'u' => Some("A-Z"),
                    'w' => Some("A-Za-z0-9"),
                    'x' => Some("0-9A-Fa-f"),
                    's' => Some(r"\s"),
                    'p' => Some(r"!-/:-@\[-`{-~"),
                    'c' => Some(r"\x00-\x1f"),
                    _ => None,
                };
                match set {
                    Some(s) if in_class => re.push_str(s),
                    Some(s) => {
                        re.push('[');
                        re.push_str(s);
                        re.push(']');
                    }
                    None => {
                        if !k.is_ascii_alphanumeric() {
                            re.push('\\');
                        }
                        re.push(k);
                    }
                }
            }
            '[' if !in_class => {
                in_class = true;
                re.push('[');
                if chars.peek() == Some(&'^') {
                    re.push(chars.next().unwrap());
                }
            }
            ']' if in_class => {
                in_class = false;
                re.push(']');
            }
            // Lua's lazy repetition.
            '-' if !in_class => re.push_str("*?"),
            '{' | '}' | '|' if !in_class => {
                re.push('\\');
                re.push(c);
            }
            '\\' => {
                // Already escaped for the query string: keep the pair.
                re.push('\\');
                if let Some(n) = chars.next() {
                    re.push(n);
                }
                continue;
            }
            c => re.push(c),
        }
    }
    // The query string unescapes backslashes once.
    re.replace('\\', r"\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits() {
        let q =
            "; c\n(a) @x\n\n[\"b\" \")\"] @y @z\n((c) @w (#eq? @w \";\")) ; t\n\"d\" @k\n(e)? @q";
        assert_eq!(
            split(q),
            vec![
                "(a) @x",
                "[\"b\" \")\"] @y @z",
                "((c) @w (#eq? @w \";\"))",
                "\"d\" @k",
                "(e)? @q"
            ]
        );
    }

    #[test]
    fn fixes() {
        let q = "((identifier) @type (#lua-match? @type \"^[%u_][%w_]*\"))\n((identifier) @p (#has-ancestor? @p x))\n(a) @b";
        assert!(needs_fix(q));
        assert_eq!(
            fix(q, false),
            "((identifier) @type (#match? @type \"^[A-Z_][A-Za-z0-9_]*\"))\n\n(a) @b"
        );
        assert_eq!(
            fix(q, true),
            "(a) @b\n\n((identifier) @type (#match? @type \"^[A-Z_][A-Za-z0-9_]*\"))"
        );
        assert!(!needs_fix("((a) @b (#eq? @b \"x\")) \"#include\" @k"));
    }

    #[test]
    fn lua_patterns() {
        assert_eq!(lua_to_regex("^%u+$"), "^[A-Z]+$");
        assert_eq!(lua_to_regex("^#!/"), "^#!/");
        assert_eq!(lua_to_regex("^%s*%-%-"), r"^[\\s]*\\-\\-");
        assert_eq!(lua_to_regex("a-b"), "a*?b");
    }
}
