use std::time::{Duration, Instant};

use diffr_highlight::{Class, Language, MAX_BYTES, Seg, language_for, lines};

fn owned(src: &str) -> Vec<String> {
    src.lines().map(str::to_owned).collect()
}

/// Whether line `row` has a segment of exactly `text` of `class`.
fn has(lines: &[String], segs: &[Vec<Seg>], row: usize, text: &str, class: Class) -> bool {
    segs[row]
        .iter()
        .any(|s| &lines[row][s.start as usize..s.end as usize] == text && s.class == class)
}

#[track_caller]
fn expect(lines: &[String], segs: &[Vec<Seg>], row: usize, text: &str, class: Class) {
    assert!(
        has(lines, segs, row, text, class),
        "line {row} {:?}: no {class:?} {text:?} in {:?}",
        lines[row],
        segs[row]
            .iter()
            .map(|s| (s.class, &lines[row][s.start as usize..s.end as usize]))
            .collect::<Vec<_>>()
    );
}

/// Segments are in bounds, ordered, non-empty, never plain, and merged.
#[track_caller]
fn well_formed(lines: &[String], segs: &[Vec<Seg>]) {
    assert_eq!(segs.len(), lines.len());
    for (line, segs) in lines.iter().zip(segs) {
        let mut prev: Option<Seg> = None;
        for s in segs {
            assert!(
                s.start < s.end && s.end as usize <= line.len(),
                "{s:?} in {line:?}"
            );
            assert!(
                line.is_char_boundary(s.start as usize) && line.is_char_boundary(s.end as usize)
            );
            assert_ne!(s.class, Class::Plain);
            if let Some(p) = prev {
                assert!(p.end <= s.start, "{p:?} then {s:?}");
                assert!(
                    !(p.end == s.start && p.class == s.class),
                    "unmerged {p:?} {s:?}"
                );
            }
            prev = Some(*s);
        }
    }
}

#[test]
fn go() {
    let src = owned("package main\n\n// hi\nfunc main() { println(\"x\", 42) }");
    let segs = lines("main.go", &src).unwrap();
    well_formed(&src, &segs);
    expect(&src, &segs, 0, "package", Class::Keyword);
    expect(&src, &segs, 2, "// hi", Class::Comment);
    expect(&src, &segs, 3, "func", Class::Keyword);
    expect(&src, &segs, 3, "main", Class::Function);
    expect(&src, &segs, 3, "println", Class::Function);
    expect(&src, &segs, 3, "\"x\"", Class::String);
    expect(&src, &segs, 3, "42", Class::Number);
    assert!(
        lines("notes.unknownext", &src).is_none(),
        "plain text highlighted"
    );
}

#[test]
fn go_more() {
    let src = owned(
        "type P struct{ X int }\nfunc (p *P) Get() int { return p.X }\nvar s = `a\nb` + \"\\n\"\nvar ok = nil == true",
    );
    let segs = lines("x/p.go", &src).unwrap();
    well_formed(&src, &segs);
    expect(&src, &segs, 0, "type", Class::Keyword);
    expect(&src, &segs, 0, "P", Class::Type);
    expect(&src, &segs, 0, "int", Class::Type);
    expect(&src, &segs, 1, "Get", Class::Function);
    expect(&src, &segs, 1, "return", Class::Keyword);
    // A raw string spanning lines is cut at the line break.
    expect(&src, &segs, 2, "`a", Class::String);
    expect(&src, &segs, 3, "b`", Class::String);
    expect(&src, &segs, 3, "\\n", Class::Escape);
    expect(&src, &segs, 3, "+", Class::Keyword);
    expect(&src, &segs, 4, "nil", Class::LangConst);
    expect(&src, &segs, 4, "true", Class::LangConst);
}

#[test]
fn rust() {
    let src = owned(
        "use std::fmt;\n\
         /// Docs.\n\
         #[derive(Debug)]\n\
         pub struct Foo<'a> { name: &'a str }\n\
         const MAX: usize = 0x10;\n\
         fn main() {\n\
         \x20   let s = \"hi\\n\"; /* block\n\
         \x20      comment */\n\
         \x20   println!(\"{}\", s.len() as f64 + 1.5);\n\
         \x20   let ok = Some(true);\n\
         }",
    );
    let segs = lines("src/main.rs", &src).unwrap();
    well_formed(&src, &segs);
    expect(&src, &segs, 0, "use", Class::Keyword);
    expect(&src, &segs, 1, "/// Docs.", Class::Comment);
    expect(&src, &segs, 2, "Debug", Class::ClassName);
    expect(&src, &segs, 3, "pub", Class::Keyword);
    expect(&src, &segs, 3, "struct", Class::Keyword);
    expect(&src, &segs, 3, "Foo", Class::Type);
    expect(&src, &segs, 3, "str", Class::Type);
    expect(&src, &segs, 4, "MAX", Class::Number);
    expect(&src, &segs, 4, "0x10", Class::Number);
    expect(&src, &segs, 5, "fn", Class::Keyword);
    expect(&src, &segs, 5, "main", Class::Function);
    expect(&src, &segs, 6, "let", Class::Keyword);
    expect(&src, &segs, 6, "\"hi", Class::String);
    expect(&src, &segs, 6, "\\n", Class::Escape);
    expect(&src, &segs, 6, "/* block", Class::Comment);
    expect(&src, &segs, 7, "       comment */", Class::Comment);
    expect(&src, &segs, 8, "println!", Class::Function);
    expect(&src, &segs, 8, "len", Class::Function);
    expect(&src, &segs, 8, "as", Class::Keyword);
    expect(&src, &segs, 8, "1.5", Class::Number);
    expect(&src, &segs, 9, "true", Class::LangConst);
}

#[test]
fn others() {
    /// File name, source, and (row, text, class) expectations.
    type Case = (
        &'static str,
        &'static str,
        &'static [(usize, &'static str, Class)],
    );
    let cases: &[Case] = &[
        (
            "a.py",
            "import os\nclass Foo(Bar):\n    def f(self):\n        return f\"x\" # c",
            &[
                (0, "import", Class::Keyword),
                (1, "Foo", Class::ClassName),
                (2, "f", Class::Function),
                (2, "self", Class::LangConst),
                (3, "# c", Class::Comment),
            ],
        ),
        (
            "a.ts",
            "interface I { x: number }\nconst f = (a: string) => /a+/g.test(a);",
            &[
                (0, "interface", Class::Keyword),
                (0, "I", Class::Type),
                (0, "number", Class::Type),
                (1, "f", Class::Function),
                (1, "/a+/g", Class::Regexp),
            ],
        ),
        (
            "App.tsx",
            "const a = <div className=\"x\">{n}</div>;",
            &[
                (0, "div", Class::Tag),
                (0, "className", Class::Attribute),
                (0, "\"x\"", Class::String),
            ],
        ),
        (
            "a.jsx",
            "export default () => <A b={1} />;",
            &[(0, "export", Class::Keyword), (0, "b", Class::Attribute)],
        ),
        (
            "a.json",
            "{\"k\": [1, true, \"s\"]}",
            &[
                (0, "\"k\"", Class::Tag),
                (0, "1", Class::Number),
                (0, "true", Class::LangConst),
                (0, "\"s\"", Class::String),
            ],
        ),
        (
            "a.c",
            "#include <stdio.h>\nint main(void) { return 'c'; }",
            &[
                (0, "#include", Class::Preproc),
                (1, "int", Class::Type),
                (1, "main", Class::Function),
            ],
        ),
        (
            "a.cc",
            "template <typename T> class X {};",
            &[(0, "template", Class::Keyword), (0, "X", Class::Type)],
        ),
        (
            "A.java",
            "public class A { void f() { return; } }",
            &[(0, "public", Class::Keyword), (0, "f", Class::Function)],
        ),
        (
            "a.rb",
            "def f(x)\n  puts \"#{x}\"\nend",
            &[(0, "def", Class::Keyword), (1, "puts", Class::Function)],
        ),
        (
            "a.sh",
            "# c\nif [ -f x ]; then echo \"a\"; fi",
            &[
                (0, "# c", Class::Comment),
                (1, "if", Class::Keyword),
                (1, "\"a\"", Class::String),
            ],
        ),
        (
            "Cargo.toml",
            "[package]\nname = \"x\"",
            &[
                (0, "package", Class::Type),
                (1, "name", Class::Property),
                (1, "\"x\"", Class::String),
            ],
        ),
        (
            "a.yml",
            "on: [push]\nn: 3",
            &[(0, "on", Class::Property), (1, "3", Class::Number)],
        ),
        (
            "a.css",
            ".a { color: red; }",
            &[(0, "color", Class::Property)],
        ),
        (
            "a.html",
            "<p class=\"x\">hi</p><script>let y = 1;</script>",
            &[
                (0, "p", Class::Tag),
                (0, "class", Class::Attribute),
                (0, "let", Class::Keyword),
            ],
        ),
        (
            "README.md",
            "# Title\nSome `code` [l](http://x)\n```go\nfunc a() {}\n```",
            &[
                (0, "# Title", Class::Heading),
                (1, "`code`", Class::String),
                (1, "http://x", Class::Attribute),
                (2, "```go", Class::String),
                (3, "func", Class::Keyword),
                (3, "a", Class::Function),
            ],
        ),
        (
            "x.diff",
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b",
            &[
                (0, "--- a/x", Class::Deleted),
                (1, "+++ b/x", Class::Inserted),
                (2, "@@ -1 +1 @@", Class::Heading),
                (3, "-a", Class::Deleted),
                (4, "+b", Class::Inserted),
            ],
        ),
        (
            "Makefile",
            "# c\nall: x\n\techo hi",
            &[(0, "# c", Class::Comment)],
        ),
        (
            "Dockerfile",
            "FROM alpine\nRUN echo \"hi\"",
            &[
                (0, "FROM", Class::Keyword),
                (1, "RUN", Class::Keyword),
                (1, "\"hi\"", Class::String),
            ],
        ),
    ];
    for (name, src, want) in cases {
        let src = owned(src);
        let segs = lines(name, &src).unwrap_or_else(|| panic!("{name}: not highlighted"));
        well_formed(&src, &segs);
        for &(row, text, class) in *want {
            assert!(
                has(&src, &segs, row, text, class),
                "{name}:{row}: no {class:?} {text:?} in {:?}",
                segs[row]
            );
        }
    }
}

/// A small sample of every language added for Chroma's wider
/// coverage gives at least keyword, string and comment segments (or, for
/// data languages without keywords, their nearest equivalents).
#[test]
fn smoke() {
    use Class::*;
    /// File name, source, and (row, text, class) expectations.
    type Case = (
        &'static str,
        &'static str,
        &'static [(usize, &'static str, Class)],
    );
    let cases: &[Case] = &[
        (
            "a.swift",
            "// c\nfunc f() -> String { return \"s\" }",
            &[
                (0, "// c", Comment),
                (1, "func", Keyword),
                (1, "f", Function),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.kt",
            "// c\npackage a\nfun f(): String = if (x) \"s\" else null",
            &[
                (0, "// c", Comment),
                (1, "package", Keyword),
                (2, "fun", Keyword),
                (2, "\"s\"", String),
                (2, "null", LangConst),
            ],
        ),
        (
            "a.cs",
            "// c\npublic class A { string s = \"s\"; }",
            &[
                (0, "// c", Comment),
                (1, "public", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.php",
            "<p>hi</p>\n<?php\n// c\nfunction f($x) { return \"s\"; }",
            &[
                (0, "p", Tag),
                (1, "<?php", Preproc),
                (2, "// c", Comment),
                (3, "function", Keyword),
                (3, "\"s\"", String),
            ],
        ),
        (
            "a.lua",
            "-- c\nlocal function f() return \"s\" end",
            &[
                (0, "-- c", Comment),
                (1, "local", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.scala",
            "// c\nobject A { def f = \"s\" }",
            &[
                (0, "// c", Comment),
                (1, "object", Keyword),
                (1, "def", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.hs",
            "-- c\nf :: Int -> String\nf x = if x > 0 then \"s\" else show True",
            &[
                (0, "-- c", Comment),
                (1, "f", Function),
                (1, "Int", Type),
                (2, "if", Keyword),
                (2, "\"s\"", String),
                (2, "True", LangConst),
            ],
        ),
        (
            "a.exs",
            "# c\ndefmodule A do\n  def f(_x), do: \"s\"\nend",
            &[
                (0, "# c", Comment),
                (1, "defmodule", Keyword),
                (2, "def", Keyword),
                (2, "\"s\"", String),
            ],
        ),
        (
            "a.erl",
            "% c\nf(_) -> case 1 of _ -> \"s\" end.",
            &[
                (0, "% c", Comment),
                (1, "case", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.zig",
            "// c\npub fn main() void { const x = \"s\"; }",
            &[
                (0, "// c", Comment),
                (1, "pub", Keyword),
                (1, "fn", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.dart",
            "// c\nclass A { String f() { return 's'; } }",
            &[
                (0, "// c", Comment),
                (1, "class", Keyword),
                (1, "return", Keyword),
                (1, "'s'", String),
            ],
        ),
        (
            "a.ml",
            "(* c *)\nlet f x = if x then \"s\" else \"t\"",
            &[
                (0, "(* c *)", Comment),
                (1, "let", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.mli",
            "(* c *)\nval f : int -> string",
            &[
                (0, "(* c *)", Comment),
                (1, "val", Keyword),
                (1, "int", Type),
            ],
        ),
        (
            "a.R",
            "# c\nf <- function(x) if (x) \"s\" else NULL",
            &[
                (0, "# c", Comment),
                (1, "function", Keyword),
                (1, "\"s\"", String),
                (1, "NULL", LangConst),
            ],
        ),
        (
            "a.jl",
            "# c\nfunction f(x) return \"s\" end",
            &[
                (0, "# c", Comment),
                (1, "function", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.sql",
            "-- c\nSELECT a FROM t WHERE b = 's' AND c IS NULL AND d > 21;",
            &[
                (0, "-- c", Comment),
                (1, "SELECT", Keyword),
                (1, "'s'", String),
                (1, "NULL", LangConst),
                (1, "21", Number),
            ],
        ),
        (
            "a.graphql",
            "# c\nquery Q($id: ID!) { user(id: \"s\") { name } }",
            &[
                (0, "# c", Comment),
                (1, "query", Keyword),
                (1, "Q", Function),
                (1, "ID", Type),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.proto",
            "// c\nsyntax = \"proto3\";\nmessage M { string s = 1; }",
            &[
                (0, "// c", Comment),
                (1, "syntax", Keyword),
                (1, "\"proto3\"", String),
                (2, "message", Keyword),
                (2, "string", Type),
            ],
        ),
        (
            "a.nix",
            "# c\nlet x = \"s\"; in if true then x else builtins.map",
            &[
                (0, "# c", Comment),
                (1, "let", Keyword),
                (1, "\"s\"", String),
                (1, "true", LangConst),
            ],
        ),
        (
            "a.vue",
            "<template>\n  <!-- c -->\n  <a :x=\"go(1)\">{{ n + 1 }}</a>\n</template>\n<script lang=\"ts\">\nconst s: string = \"s\";\n</script>",
            &[
                (1, "<!-- c -->", Comment),
                (2, "a", Tag),
                (2, ":x", Attribute),
                (2, "go", Function),
                (2, "+", Keyword),
                (5, "const", Keyword),
                (5, "string", Type),
                (5, "\"s\"", String),
            ],
        ),
        (
            "a.svelte",
            "<script>let n = \"s\";</script>\n<!-- c -->\n{#if n}<p>{n + 1}</p>{/if}",
            &[
                (0, "let", Keyword),
                (0, "\"s\"", String),
                (1, "<!-- c -->", Comment),
                (2, "#if", Keyword),
                (2, "p", Tag),
                (2, "+", Keyword),
                (2, "/if", Keyword),
            ],
        ),
        (
            "icon.svg",
            "<!-- c -->\n<svg a=\"s\"><![CDATA[x]]></svg>",
            &[
                (0, "<!-- c -->", Comment),
                (1, "svg", Tag),
                (1, "a", Attribute),
                (1, "\"s\"", String),
                (1, "<![CDATA[x]]>", Preproc),
            ],
        ),
        (
            "a.scss",
            "// c\n@if $a == 1 { .b { content: \"s\"; } }",
            &[
                (0, "// c", Comment),
                (1, "@if", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.less",
            "// c\n@import \"s\";",
            &[
                (0, "// c", Comment),
                (1, "@import", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "main.tf",
            "# c\nresource \"a\" \"b\" {\n  n = var.x ? 1 : null\n}",
            &[
                (0, "# c", Comment),
                (1, "resource", Keyword),
                (1, "\"a\"", String),
                (2, "n", Property),
                (2, "null", LangConst),
            ],
        ),
        (
            "a.m",
            "// c\n@implementation A\n- (void)f { NSLog(@\"s\"); if (YES) return; }\n@end",
            &[
                (0, "// c", Comment),
                (1, "@implementation", Keyword),
                (2, "void", Type),
                (2, "NSLog", Function),
                (2, "@\"s\"", String),
                (2, "if", Keyword),
                (2, "YES", LangConst),
            ],
        ),
        (
            "a.pl",
            "# c\nsub f { my $x = \"s\"; return 1 if $x =~ /a+/; }",
            &[
                (0, "# c", Comment),
                (1, "sub", Keyword),
                (1, "f", Function),
                (1, "my", Keyword),
                (1, "\"s\"", String),
                (1, "/a+/", Regexp),
            ],
        ),
        (
            "a.ps1",
            "# c\nfunction F { if ($x -eq $null) { return \"s\" } }",
            &[
                (0, "# c", Comment),
                (1, "function", Keyword),
                (1, "if", Keyword),
                (1, "$null", LangConst),
                (1, "\"s\"", String),
            ],
        ),
        (
            "build.gradle",
            "// c\ndef x = 's'\nif (x != null) { println true }",
            &[
                (0, "// c", Comment),
                (1, "def", Keyword),
                (1, "'s'", String),
                (2, "if", Keyword),
                (2, "println", Function),
                (2, "null", LangConst),
            ],
        ),
        (
            "CMakeLists.txt",
            "# c\nif(WIN32)\n  message(STATUS \"s\")\nendif()",
            &[
                (0, "# c", Comment),
                (1, "if", Keyword),
                (2, "message", Function),
                (2, "\"s\"", String),
                (3, "endif", Keyword),
            ],
        ),
        (
            "a.ini",
            "; c\n[s]\nk = v",
            &[(0, "; c", Comment), (1, "s", Type), (2, "k", Property)],
        ),
        (
            ".env",
            "# c\nexport A=\"s\"",
            &[
                (0, "# c", Comment),
                (1, "export", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.properties",
            "# c\na.b = v",
            &[(0, "# c", Comment), (1, "a.b", Property), (1, "v", String)],
        ),
        (
            "a.clj",
            "; c\n(defn f [x] (if x \"s\" :k))",
            &[
                (0, "; c", Comment),
                (1, "defn", Keyword),
                (1, "f", Function),
                (1, "if", Keyword),
                (1, "\"s\"", String),
                (1, ":k", Number),
            ],
        ),
        (
            "a.elm",
            "-- c\nf x = if x then \"s\" else \"t\"",
            &[
                (0, "-- c", Comment),
                (1, "f", Function),
                (1, "if", Keyword),
                (1, "\"s\"", String),
            ],
        ),
        (
            "a.fish",
            "# c\nfunction f\n  echo \"s $x\"\nend",
            &[
                (0, "# c", Comment),
                (1, "function", Keyword),
                (2, "echo", Function),
                (3, "end", Keyword),
            ],
        ),
        (
            "a.sv",
            "// c\nmodule m; initial $display(\"s\"); endmodule",
            &[
                (0, "// c", Comment),
                (1, "module", Keyword),
                (1, "$display", Function),
                (1, "\"s\"", String),
                (1, "endmodule", Keyword),
            ],
        ),
        (
            "a.v",
            "/* c */\nmodule m(input a); assign b = 8'hFF; endmodule",
            &[
                (0, "/* c */", Comment),
                (1, "module", Keyword),
                (1, "input", Keyword),
                (1, "8'hFF", Number),
            ],
        ),
        (
            "a.vhd",
            "-- c\nentity e is end entity;\narchitecture a of e is begin report \"s\"; end;",
            &[
                (0, "-- c", Comment),
                (1, "entity", Keyword),
                (2, "report", Keyword),
                (2, "\"s\"", String),
            ],
        ),
    ];
    for (name, src, want) in cases {
        let src = owned(src);
        let segs = lines(name, &src).unwrap_or_else(|| panic!("{name}: not highlighted"));
        well_formed(&src, &segs);
        for &(row, text, class) in *want {
            assert!(
                has(&src, &segs, row, text, class),
                "{name}:{row}: no {class:?} {text:?} in {:?}",
                segs[row]
                    .iter()
                    .map(|s| (s.class, &src[row][s.start as usize..s.end as usize]))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn languages() {
    for (name, want) in [
        ("a/main.go", Some(Language::Go)),
        ("lib.rs", Some(Language::Rust)),
        ("types.d.ts", Some(Language::TypeScript)),
        ("App.TSX", Some(Language::Tsx)),
        ("x.jsx", Some(Language::Jsx)),
        ("index.mjs", Some(Language::JavaScript)),
        ("Makefile", Some(Language::Make)),
        ("rules.mk", Some(Language::Make)),
        ("Dockerfile", Some(Language::Dockerfile)),
        ("Dockerfile.dev", Some(Language::Dockerfile)),
        ("Cargo.lock", Some(Language::Toml)),
        (".bashrc", Some(Language::Bash)),
        ("Gemfile", Some(Language::Ruby)),
        ("x.hpp", Some(Language::Cpp)),
        ("x.h", Some(Language::C)),
        ("notes.txt", None),
        ("CMakeLists.txt", Some(Language::CMake)),
        ("x/y.cmake", Some(Language::CMake)),
        ("A.kt", Some(Language::Kotlin)),
        ("build.gradle.kts", Some(Language::Kotlin)),
        ("build.gradle", Some(Language::Groovy)),
        ("Jenkinsfile", Some(Language::Groovy)),
        ("P.cs", Some(Language::CSharp)),
        ("index.php", Some(Language::Php)),
        ("init.lua", Some(Language::Lua)),
        ("A.scala", Some(Language::Scala)),
        ("x.sc", Some(Language::Scala)),
        ("Main.hs", Some(Language::Haskell)),
        ("mix.exs", Some(Language::Elixir)),
        ("a.ex", Some(Language::Elixir)),
        ("a.erl", Some(Language::Erlang)),
        ("build.zig", Some(Language::Zig)),
        ("main.dart", Some(Language::Dart)),
        ("a.ml", Some(Language::OCaml)),
        ("a.mli", Some(Language::OCamlInterface)),
        ("analysis.R", Some(Language::R)),
        ("a.jl", Some(Language::Julia)),
        ("schema.sql", Some(Language::Sql)),
        ("s.graphql", Some(Language::GraphQL)),
        ("q.gql", Some(Language::GraphQL)),
        ("api.proto", Some(Language::Protobuf)),
        ("flake.nix", Some(Language::Nix)),
        ("App.vue", Some(Language::Vue)),
        ("App.svelte", Some(Language::Svelte)),
        ("pom.xml", Some(Language::Xml)),
        ("icon.svg", Some(Language::Xml)),
        ("Info.plist", Some(Language::Xml)),
        ("a.scss", Some(Language::Scss)),
        ("a.less", Some(Language::Less)),
        ("main.tf", Some(Language::Hcl)),
        ("a.hcl", Some(Language::Hcl)),
        ("x.m", Some(Language::ObjectiveC)),
        ("a.pl", Some(Language::Perl)),
        ("A.pm", Some(Language::Perl)),
        ("a.ps1", Some(Language::PowerShell)),
        ("setup.cfg", Some(Language::Ini)),
        ("a.ini", Some(Language::Ini)),
        (".editorconfig", Some(Language::Ini)),
        (".env", Some(Language::Bash)),
        (".env.local", Some(Language::Bash)),
        ("a.clj", Some(Language::Clojure)),
        ("a.cljs", Some(Language::Clojure)),
        ("deps.edn", Some(Language::Clojure)),
        ("Main.elm", Some(Language::Elm)),
        ("config.fish", Some(Language::Fish)),
        ("a.v", Some(Language::SystemVerilog)),
        ("a.sv", Some(Language::SystemVerilog)),
        ("a.vhd", Some(Language::Vhdl)),
        ("a.properties", Some(Language::Properties)),
        ("a.nu", None),
        ("LICENSE", None),
        (".gitignore", None),
        ("main.go.orig", None),
    ] {
        assert_eq!(language_for(name), want, "{name}");
    }
}

#[test]
fn edges() {
    assert!(lines("a.go", &[]).is_none());
    let big = vec!["x".repeat(1000); MAX_BYTES / 1000 + 1];
    assert!(lines("a.go", &big).is_none(), "too big highlighted");
    // Broken and partial code still highlights, with CRs and non-ASCII.
    let src = owned("func (\r\n\"héllo wörld\" // ünïcode\n}}}");
    let segs = lines("a.go", &src).unwrap();
    well_formed(&src, &segs);
    expect(&src, &segs, 1, "// ünïcode", Class::Comment);
    // An empty line in the middle has no segments.
    let src = owned("let a = 1;\n\nlet b = 2;");
    let segs = lines("a.js", &src).unwrap();
    assert!(segs[1].is_empty());
    expect(&src, &segs, 2, "let", Class::Keyword);
}

#[test]
fn fast() {
    let unit = "// Package x does things.\npackage x\n\nimport \"fmt\"\n\ntype T struct {\n\tA int\n\tB string\n}\n\nfunc (t *T) String() string {\n\tif t.A > 0 {\n\t\treturn fmt.Sprintf(\"%d %s\\n\", t.A, t.B)\n\t}\n\treturn \"\"\n}\n";
    let mut src = Vec::new();
    while src.len() < 5000 {
        src.extend(
            unit.lines()
                .skip(if src.is_empty() { 0 } else { 4 })
                .map(str::to_owned),
        );
    }
    for (name, src) in [
        ("big.go", src.clone()),
        (
            "big.rs",
            src.iter().map(|l| l.replace("func", "fn")).collect(),
        ),
    ] {
        lines(name, &src[..10]).unwrap(); // compile the queries first
        let t = Instant::now();
        let segs = lines(name, &src).unwrap();
        let took = t.elapsed();
        assert_eq!(segs.len(), src.len());
        let limit = if cfg!(debug_assertions) {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(500)
        };
        eprintln!("{name}: {} lines in {took:?}", src.len());
        assert!(took < limit, "{name}: {} lines took {took:?}", src.len());
    }
}

#[test]
fn every_language_compiles() {
    for &l in Language::ALL {
        assert!(
            diffr_highlight::highlight_lines(l, &["x".to_string()]).is_some(),
            "{}",
            l.name()
        );
    }
}

#[test]
fn threads() {
    let src = owned("fn main() {}");
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let src = src.clone();
            std::thread::spawn(move || lines("a.rs", &src).unwrap())
        })
        .collect();
    for h in handles {
        let segs = h.join().unwrap();
        assert_eq!(
            segs[0][0],
            Seg {
                start: 0,
                end: 2,
                class: Class::Keyword
            }
        );
    }
}
