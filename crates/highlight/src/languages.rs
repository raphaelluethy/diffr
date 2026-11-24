//! The supported languages: their grammars, queries, and file names.

use std::sync::OnceLock;

use tree_sitter_highlight::HighlightConfiguration;

use crate::classes::NAMES;

/// A language with a grammar and highlight queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Bash,
    C,
    CSharp,
    Clojure,
    CMake,
    Cpp,
    Css,
    Dart,
    Diff,
    Dockerfile,
    Elixir,
    Elm,
    Erlang,
    Fish,
    Go,
    GraphQL,
    Groovy,
    Haskell,
    Hcl,
    Html,
    Ini,
    Java,
    JavaScript,
    Jsx,
    Json,
    Julia,
    Kotlin,
    Less,
    Lua,
    Make,
    Markdown,
    /// Markdown's inline grammar, injected into the paragraphs and headings
    /// of [`Language::Markdown`]; no file is of this language.
    MarkdownInline,
    Nix,
    ObjectiveC,
    OCaml,
    /// OCaml interfaces (`.mli`), a grammar of their own.
    OCamlInterface,
    Perl,
    Php,
    PowerShell,
    Properties,
    Protobuf,
    Python,
    R,
    Ruby,
    Rust,
    Scala,
    Scss,
    Sql,
    Svelte,
    Swift,
    /// Verilog and SystemVerilog, one grammar.
    SystemVerilog,
    Toml,
    TypeScript,
    Tsx,
    Vhdl,
    Vue,
    Xml,
    Yaml,
    Zig,
}

impl Language {
    /// Every language, in order of their discriminants.
    pub const ALL: &[Language] = &[
        Language::Bash,
        Language::C,
        Language::CSharp,
        Language::Clojure,
        Language::CMake,
        Language::Cpp,
        Language::Css,
        Language::Dart,
        Language::Diff,
        Language::Dockerfile,
        Language::Elixir,
        Language::Elm,
        Language::Erlang,
        Language::Fish,
        Language::Go,
        Language::GraphQL,
        Language::Groovy,
        Language::Haskell,
        Language::Hcl,
        Language::Html,
        Language::Ini,
        Language::Java,
        Language::JavaScript,
        Language::Jsx,
        Language::Json,
        Language::Julia,
        Language::Kotlin,
        Language::Less,
        Language::Lua,
        Language::Make,
        Language::Markdown,
        Language::MarkdownInline,
        Language::Nix,
        Language::ObjectiveC,
        Language::OCaml,
        Language::OCamlInterface,
        Language::Perl,
        Language::Php,
        Language::PowerShell,
        Language::Properties,
        Language::Protobuf,
        Language::Python,
        Language::R,
        Language::Ruby,
        Language::Rust,
        Language::Scala,
        Language::Scss,
        Language::Sql,
        Language::Svelte,
        Language::Swift,
        Language::SystemVerilog,
        Language::Toml,
        Language::TypeScript,
        Language::Tsx,
        Language::Vhdl,
        Language::Vue,
        Language::Xml,
        Language::Yaml,
        Language::Zig,
    ];

    /// The language's display name.
    pub fn name(self) -> &'static str {
        match self {
            Language::Bash => "Bash",
            Language::C => "C",
            Language::CSharp => "C#",
            Language::Clojure => "Clojure",
            Language::CMake => "CMake",
            Language::Cpp => "C++",
            Language::Css => "CSS",
            Language::Dart => "Dart",
            Language::Diff => "Diff",
            Language::Dockerfile => "Dockerfile",
            Language::Elixir => "Elixir",
            Language::Elm => "Elm",
            Language::Erlang => "Erlang",
            Language::Fish => "Fish",
            Language::Go => "Go",
            Language::GraphQL => "GraphQL",
            Language::Groovy => "Groovy",
            Language::Haskell => "Haskell",
            Language::Hcl => "HCL",
            Language::Html => "HTML",
            Language::Ini => "INI",
            Language::Java => "Java",
            Language::JavaScript => "JavaScript",
            Language::Jsx => "JSX",
            Language::Json => "JSON",
            Language::Julia => "Julia",
            Language::Kotlin => "Kotlin",
            Language::Less => "Less",
            Language::Lua => "Lua",
            Language::Make => "Makefile",
            Language::Markdown => "Markdown",
            Language::MarkdownInline => "Markdown Inline",
            Language::Nix => "Nix",
            Language::ObjectiveC => "Objective-C",
            Language::OCaml => "OCaml",
            Language::OCamlInterface => "OCaml Interface",
            Language::Perl => "Perl",
            Language::Php => "PHP",
            Language::PowerShell => "PowerShell",
            Language::Properties => "Properties",
            Language::Protobuf => "Protocol Buffer",
            Language::Python => "Python",
            Language::R => "R",
            Language::Ruby => "Ruby",
            Language::Rust => "Rust",
            Language::Scala => "Scala",
            Language::Scss => "SCSS",
            Language::Sql => "SQL",
            Language::Svelte => "Svelte",
            Language::Swift => "Swift",
            Language::SystemVerilog => "SystemVerilog",
            Language::Toml => "TOML",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::Vhdl => "VHDL",
            Language::Vue => "Vue",
            Language::Xml => "XML",
            Language::Yaml => "YAML",
            Language::Zig => "Zig",
        }
    }

    /// The compiled highlight configuration, built on first use and kept
    /// for the life of the process. `None` only if the queries fail to
    /// compile, which the tests rule out.
    pub(crate) fn config(self) -> Option<&'static HighlightConfiguration> {
        static CONFIGS: [OnceLock<Option<HighlightConfiguration>>; SLOTS] =
            [const { OnceLock::new() }; SLOTS];
        CONFIGS[self.slot()].get_or_init(|| build(self)).as_ref()
    }

    /// The configuration slot: JavaScript and JSX share one.
    fn slot(self) -> usize {
        match self {
            Language::Jsx => Language::JavaScript as usize,
            l => l as usize,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::Bash => tree_sitter_bash::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Language::Clojure => tree_sitter_clojure_orchard::LANGUAGE.into(),
            Language::CMake => tree_sitter_cmake::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Css => tree_sitter_css::LANGUAGE.into(),
            Language::Dart => tree_sitter_dart::LANGUAGE.into(),
            Language::Diff => tree_sitter_diff::LANGUAGE.into(),
            Language::Dockerfile => tree_sitter_containerfile::LANGUAGE.into(),
            Language::Elixir => tree_sitter_elixir::LANGUAGE.into(),
            Language::Elm => tree_sitter_elm::LANGUAGE.into(),
            Language::Erlang => tree_sitter_erlang::LANGUAGE.into(),
            Language::Fish => tree_sitter_fish::language(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::GraphQL => tree_sitter_graphql::LANGUAGE.into(),
            Language::Groovy => tree_sitter_groovy::LANGUAGE.into(),
            Language::Haskell => tree_sitter_haskell::LANGUAGE.into(),
            Language::Hcl => tree_sitter_hcl::LANGUAGE.into(),
            Language::Html => tree_sitter_html::LANGUAGE.into(),
            Language::Ini => tree_sitter_ini::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::JavaScript | Language::Jsx => tree_sitter_javascript::LANGUAGE.into(),
            Language::Json => tree_sitter_json::LANGUAGE.into(),
            Language::Julia => tree_sitter_julia::LANGUAGE.into(),
            Language::Kotlin => tree_sitter_kotlin_sg::LANGUAGE.into(),
            Language::Less => tree_sitter_less::language(),
            Language::Lua => tree_sitter_lua::LANGUAGE.into(),
            Language::Make => tree_sitter_make::LANGUAGE.into(),
            Language::Markdown => tree_sitter_md::LANGUAGE.into(),
            Language::MarkdownInline => tree_sitter_md::INLINE_LANGUAGE.into(),
            Language::Nix => tree_sitter_nix::LANGUAGE.into(),
            Language::ObjectiveC => tree_sitter_objc::LANGUAGE.into(),
            Language::OCaml => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
            Language::OCamlInterface => tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
            Language::Perl => tree_sitter_perl::LANGUAGE.into(),
            Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Language::PowerShell => tree_sitter_powershell::LANGUAGE.into(),
            Language::Properties => tree_sitter_properties::LANGUAGE.into(),
            Language::Protobuf => tree_sitter_proto::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::R => tree_sitter_r::LANGUAGE.into(),
            Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Scala => tree_sitter_scala::LANGUAGE.into(),
            Language::Scss => tree_sitter_scss::language(),
            Language::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Language::Svelte => tree_sitter_svelte_ng::LANGUAGE.into(),
            Language::Swift => tree_sitter_swift::LANGUAGE.into(),
            Language::SystemVerilog => tree_sitter_systemverilog::LANGUAGE.into(),
            Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::Vhdl => tree_sitter_vhdl::LANGUAGE.into(),
            Language::Vue => tree_sitter_vue_next::LANGUAGE.into(),
            Language::Xml => tree_sitter_xml::LANGUAGE_XML.into(),
            Language::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Language::Zig => tree_sitter_zig::LANGUAGE.into(),
        }
    }

    /// The highlight queries, in order: later patterns win over earlier
    /// ones for the same node, so ours (in `queries/`) come last.
    fn highlights(self) -> Vec<&'static str> {
        use tree_sitter_javascript as js;
        macro_rules! q {
            ($name:literal) => {
                include_str!(concat!("../queries/", $name, ".scm"))
            };
        }
        match self {
            Language::Bash => vec![tree_sitter_bash::HIGHLIGHT_QUERY, q!("bash")],
            Language::C => vec![tree_sitter_c::HIGHLIGHT_QUERY, q!("c")],
            Language::CSharp => vec![tree_sitter_c_sharp::HIGHLIGHTS_QUERY, q!("csharp")],
            Language::Clojure => vec![tree_sitter_clojure_orchard::HIGHLIGHTS_QUERY, q!("clojure")],
            Language::CMake => vec![tree_sitter_cmake::HIGHLIGHTS_QUERY],
            Language::Cpp => vec![
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY,
                q!("c"),
                q!("cpp"),
            ],
            Language::Css => vec![tree_sitter_css::HIGHLIGHTS_QUERY],
            Language::Dart => vec![tree_sitter_dart::HIGHLIGHTS_QUERY, q!("dart")],
            Language::Diff => vec![q!("diff")],
            Language::Dockerfile => vec![tree_sitter_containerfile::HIGHLIGHTS_QUERY],
            Language::Elixir => vec![tree_sitter_elixir::HIGHLIGHTS_QUERY, q!("elixir")],
            Language::Elm => vec![tree_sitter_elm::HIGHLIGHTS_QUERY, q!("elm")],
            Language::Erlang => vec![tree_sitter_erlang::HIGHLIGHTS_QUERY, q!("erlang")],
            Language::Fish => vec![tree_sitter_fish::HIGHLIGHTS_QUERY, q!("fish")],
            Language::Go => vec![tree_sitter_go::HIGHLIGHTS_QUERY, q!("go")],
            Language::GraphQL => vec![q!("graphql")],
            Language::Groovy => vec![q!("groovy")],
            Language::Haskell => vec![tree_sitter_haskell::HIGHLIGHTS_QUERY, q!("haskell")],
            Language::Hcl => vec![q!("hcl")],
            Language::Html => vec![tree_sitter_html::HIGHLIGHTS_QUERY],
            Language::Ini => vec![tree_sitter_ini::HIGHLIGHTS_QUERY, q!("ini")],
            Language::Java => vec![tree_sitter_java::HIGHLIGHTS_QUERY],
            Language::JavaScript | Language::Jsx => {
                vec![js::HIGHLIGHT_QUERY, js::JSX_HIGHLIGHT_QUERY, q!("ecma")]
            }
            Language::Json => vec![tree_sitter_json::HIGHLIGHTS_QUERY, q!("json")],
            Language::Julia => vec![q!("julia-highlights")],
            Language::Kotlin => vec![tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY, q!("kotlin")],
            Language::Less => vec![
                tree_sitter_css::HIGHLIGHTS_QUERY,
                tree_sitter_less::HIGHLIGHTS_QUERY,
            ],
            Language::Lua => vec![tree_sitter_lua::HIGHLIGHTS_QUERY, q!("lua")],
            Language::Make => vec![tree_sitter_make::HIGHLIGHTS_QUERY],
            Language::Markdown => vec![tree_sitter_md::HIGHLIGHT_QUERY_BLOCK, q!("markdown")],
            Language::MarkdownInline => vec![tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
            Language::Nix => vec![tree_sitter_nix::HIGHLIGHTS_QUERY],
            Language::ObjectiveC => vec![
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_objc::HIGHLIGHTS_QUERY,
                q!("c"),
                q!("objc"),
            ],
            Language::OCaml | Language::OCamlInterface => {
                vec![tree_sitter_ocaml::HIGHLIGHTS_QUERY, q!("ocaml")]
            }
            Language::Perl => vec![q!("perl")],
            Language::Php => vec![tree_sitter_php::HIGHLIGHTS_QUERY, q!("php")],
            Language::PowerShell => {
                vec![tree_sitter_powershell::HIGHLIGHTS_QUERY, q!("powershell")]
            }
            Language::Properties => vec![tree_sitter_properties::HIGHLIGHTS_QUERY],
            Language::Protobuf => vec![q!("proto-highlights"), q!("proto")],
            Language::Python => vec![tree_sitter_python::HIGHLIGHTS_QUERY, q!("python")],
            Language::R => vec![tree_sitter_r::HIGHLIGHTS_QUERY],
            Language::Ruby => vec![tree_sitter_ruby::HIGHLIGHTS_QUERY],
            Language::Rust => vec![tree_sitter_rust::HIGHLIGHTS_QUERY, q!("rust")],
            Language::Scala => vec![tree_sitter_scala::HIGHLIGHTS_QUERY, q!("scala")],
            Language::Scss => vec![
                tree_sitter_css::HIGHLIGHTS_QUERY,
                tree_sitter_scss::HIGHLIGHTS_QUERY,
                q!("scss"),
            ],
            Language::Sql => vec![tree_sitter_sequel::HIGHLIGHTS_QUERY, q!("sql")],
            Language::Svelte => vec![
                tree_sitter_html::HIGHLIGHTS_QUERY,
                tree_sitter_svelte_ng::HIGHLIGHTS_QUERY,
                q!("svelte"),
            ],
            Language::Swift => vec![tree_sitter_swift::HIGHLIGHTS_QUERY],
            Language::SystemVerilog => vec![q!("systemverilog")],
            Language::Toml => vec![tree_sitter_toml_ng::HIGHLIGHTS_QUERY, q!("toml")],
            Language::TypeScript => vec![
                js::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                q!("ecma"),
                q!("typescript"),
            ],
            Language::Tsx => vec![
                js::HIGHLIGHT_QUERY,
                js::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                q!("ecma"),
                q!("typescript"),
            ],
            Language::Vhdl => vec![tree_sitter_vhdl::HIGHLIGHTS_QUERY],
            Language::Vue => vec![tree_sitter_html::HIGHLIGHTS_QUERY, q!("vue")],
            Language::Xml => vec![tree_sitter_xml::XML_HIGHLIGHT_QUERY, q!("xml")],
            Language::Yaml => vec![tree_sitter_yaml::HIGHLIGHTS_QUERY],
            Language::Zig => vec![tree_sitter_zig::HIGHLIGHTS_QUERY],
        }
    }

    /// The number of leading [`Language::highlights`] queries that come
    /// from the grammar's crate (or are vendored from it), not written for
    /// diffr.
    fn bundled(self) -> usize {
        match self {
            Language::Diff
            | Language::GraphQL
            | Language::Groovy
            | Language::Hcl
            | Language::Perl
            | Language::SystemVerilog => 0,
            Language::Cpp | Language::ObjectiveC => 2,
            Language::Less | Language::Scss | Language::Svelte => 2,
            Language::JavaScript | Language::Jsx | Language::TypeScript => 2,
            Language::Tsx => 3,
            _ => 1,
        }
    }

    /// Whether the grammar's bundled queries are written for editors where
    /// the first pattern matching a node wins.
    fn first_wins(self) -> bool {
        matches!(self, Language::Nix)
    }

    fn injections(self) -> &'static str {
        match self {
            Language::Dockerfile => tree_sitter_containerfile::INJECTIONS_QUERY,
            Language::Elixir => tree_sitter_elixir::INJECTIONS_QUERY,
            Language::Haskell => tree_sitter_haskell::INJECTIONS_QUERY,
            Language::Html => tree_sitter_html::INJECTIONS_QUERY,
            Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
                tree_sitter_javascript::INJECTIONS_QUERY
            }
            Language::Markdown => include_str!("../queries/markdown-injections.scm"),
            Language::MarkdownInline => tree_sitter_md::INJECTION_QUERY_INLINE,
            Language::Nix => tree_sitter_nix::INJECTIONS_QUERY,
            // Inline HTML around the `<?php … ?>` blocks.
            Language::Php => include_str!("../queries/php-injections.scm"),
            // Macro arguments are parsed as Rust.
            Language::Rust => tree_sitter_rust::INJECTIONS_QUERY,
            Language::Svelte => include_str!("../queries/svelte-injections.scm"),
            Language::Vue => include_str!("../queries/vue-injections.scm"),
            _ => "",
        }
    }

    fn locals(self) -> &'static str {
        match self {
            // Without scopes, every bare identifier would be a method call.
            Language::Ruby => tree_sitter_ruby::LOCALS_QUERY,
            _ => "",
        }
    }
}

const SLOTS: usize = Language::ALL.len();

fn build(lang: Language) -> Option<HighlightConfiguration> {
    let mut highlights = String::new();
    for (i, q) in lang.highlights().into_iter().enumerate() {
        // `@spell` marks text for spell checking; as the last capture of a
        // pattern it would take the place of the real one.
        let mut q = q.replace(" @spell", "").replace(" @nospell", "");
        if i < lang.bundled() && (lang.first_wins() || crate::queryfix::needs_fix(&q)) {
            q = crate::queryfix::fix(&q, lang.first_wins());
        }
        if lang == Language::Markdown {
            // A highlight starting where an injection starts can come out
            // before the injection's own, nesting them the wrong way round;
            // overriding the capture in a later pattern does not stop it.
            q = q.replace("(code_fence_content) @none", "");
        }
        if i < lang.bundled() && matches!(lang, Language::Vue | Language::Svelte) {
            // Likewise for the injected script, style, and directive
            // values: keep HTML's string color to plain attributes.
            q = q
                .replace(
                    "(attribute_value) @string",
                    "(attribute (quoted_attribute_value (attribute_value) @string))\n\
                     (attribute (attribute_value) @string)",
                )
                .replace("(raw_text) @none", "");
        }
        highlights.push_str(&q);
        highlights.push('\n');
    }
    let mut config = HighlightConfiguration::new(
        lang.grammar(),
        lang.name(),
        &highlights,
        lang.injections(),
        lang.locals(),
    )
    .map_err(|e| {
        if cfg!(debug_assertions) {
            eprintln!("diffr-highlight: {}: {e}", lang.name());
        }
    })
    .ok()?;
    let names: Vec<&str> = NAMES.iter().map(|(n, _)| *n).collect();
    config.configure(&names);
    Some(config)
}

/// The language of an injection, as a fenced code block's info string or
/// an injection query names it.
pub fn language_by_name(name: &str) -> Option<Language> {
    let name = name.trim().to_ascii_lowercase();
    Some(match name.as_str() {
        "bash" | "sh" | "shell" | "zsh" | "ksh" | "console" | "shellsession" | "dotenv" | "env" => {
            Language::Bash
        }
        "c" | "h" => Language::C,
        "csharp" | "c#" | "cs" => Language::CSharp,
        "clojure" | "clj" | "cljs" | "cljc" | "edn" | "clojurescript" => Language::Clojure,
        "cmake" => Language::CMake,
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => Language::Cpp,
        "css" => Language::Css,
        "dart" => Language::Dart,
        "diff" | "patch" => Language::Diff,
        "dockerfile" | "docker" | "containerfile" => Language::Dockerfile,
        "elixir" | "ex" | "exs" => Language::Elixir,
        "elm" => Language::Elm,
        "erlang" | "erl" => Language::Erlang,
        "fish" => Language::Fish,
        "go" | "golang" => Language::Go,
        "graphql" | "gql" => Language::GraphQL,
        "groovy" | "gradle" => Language::Groovy,
        "haskell" | "hs" => Language::Haskell,
        "hcl" | "terraform" | "tf" => Language::Hcl,
        "html" | "htm" | "xhtml" => Language::Html,
        "ini" | "cfg" | "dosini" | "gitconfig" | "systemd" => Language::Ini,
        "java" => Language::Java,
        "javascript" | "js" | "mjs" | "cjs" | "node" => Language::JavaScript,
        "jsx" => Language::Jsx,
        "json" | "jsonc" | "json5" => Language::Json,
        "julia" | "jl" => Language::Julia,
        "kotlin" | "kt" | "kts" => Language::Kotlin,
        "less" => Language::Less,
        "lua" => Language::Lua,
        "make" | "makefile" | "mk" => Language::Make,
        "markdown" | "md" => Language::Markdown,
        "markdown_inline" => Language::MarkdownInline,
        "nix" => Language::Nix,
        "objc" | "objective-c" | "objectivec" | "obj-c" => Language::ObjectiveC,
        "ocaml" | "ml" => Language::OCaml,
        "ocaml_interface" | "mli" => Language::OCamlInterface,
        "perl" | "pl" | "pm" => Language::Perl,
        "php" => Language::Php,
        "powershell" | "ps1" | "pwsh" | "posh" => Language::PowerShell,
        "properties" | "jproperties" => Language::Properties,
        "proto" | "protobuf" => Language::Protobuf,
        "python" | "py" | "python3" => Language::Python,
        "r" | "rlang" | "splus" => Language::R,
        "ruby" | "rb" => Language::Ruby,
        "rust" | "rs" => Language::Rust,
        "scala" | "sc" => Language::Scala,
        "scss" => Language::Scss,
        "sql" | "mysql" | "postgresql" | "postgres" | "sqlite" | "plpgsql" => Language::Sql,
        "svelte" => Language::Svelte,
        "swift" => Language::Swift,
        "verilog" | "systemverilog" | "sv" => Language::SystemVerilog,
        "toml" => Language::Toml,
        "typescript" | "ts" | "mts" | "cts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "vhdl" => Language::Vhdl,
        "vue" => Language::Vue,
        "xml" | "svg" | "xsl" | "xslt" | "plist" | "rss" | "atom" => Language::Xml,
        "yaml" | "yml" => Language::Yaml,
        "zig" => Language::Zig,
        _ => return None,
    })
}

/// The language an injection query names: by name, or for a `<script …>` or
/// `<style …>` start tag (as the Vue and Svelte injections give), by its
/// `lang` attribute, JavaScript and CSS without one.
pub(crate) fn injection_language(name: &str) -> Option<Language> {
    let tag = name.trim_start();
    let default = if tag.starts_with("<script") {
        Language::JavaScript
    } else if tag.starts_with("<style") {
        Language::Css
    } else {
        return language_by_name(name);
    };
    let Some(i) = tag.find("lang=") else {
        return Some(default);
    };
    let value = tag[i + 5..].trim_start_matches(['"', '\'']);
    let end = value
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(value.len());
    match &value[..end] {
        "" => Some(default),
        // PostCSS and Sass are close enough to SCSS.
        "postcss" | "sass" => Some(Language::Scss),
        lang => language_by_name(lang),
    }
}

/// The language of a file, by its name (a path's last component decides):
/// by its whole name, as `Makefile`, else by its extension, ignoring case.
/// `None` for plain text and languages without a grammar here.
pub fn language_for(file_name: &str) -> Option<Language> {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    if let Some(l) = by_whole_name(base) {
        return Some(l);
    }
    let lower = base.to_ascii_lowercase();
    // Dockerfile.dev, Dockerfile.prod, …
    if lower.starts_with("dockerfile.") || lower.starts_with("containerfile.") {
        return Some(Language::Dockerfile);
    }
    // .env.local, .env.production, …
    if lower.starts_with(".env.") {
        return Some(Language::Bash);
    }
    let (stem, ext) = lower.rsplit_once('.')?;
    if stem.is_empty() {
        // A dotfile: .bashrc, .zshrc, …
        return by_whole_name(&lower);
    }
    Some(match ext {
        "sh" | "bash" | "zsh" | "ksh" | "bats" | "ebuild" | "eclass" | "exheres-0" | "exlib"
        | "env" => Language::Bash,
        "c" | "h" => Language::C,
        "cs" | "csx" => Language::CSharp,
        "clj" | "cljs" | "cljc" | "cljd" | "edn" | "bb" => Language::Clojure,
        "cmake" => Language::CMake,
        "cc" | "cpp" | "cxx" | "c++" | "hh" | "hpp" | "hxx" | "h++" | "ipp" | "tpp" | "inl"
        | "ino" => Language::Cpp,
        "css" => Language::Css,
        "dart" => Language::Dart,
        "diff" | "patch" => Language::Diff,
        "dockerfile" | "containerfile" => Language::Dockerfile,
        "ex" | "exs" | "eex" | "leex" => Language::Elixir,
        "elm" => Language::Elm,
        "erl" | "hrl" | "es" | "escript" => Language::Erlang,
        "fish" => Language::Fish,
        "go" => Language::Go,
        "graphql" | "graphqls" | "gql" => Language::GraphQL,
        "groovy" | "gradle" | "gvy" | "gy" | "gsh" => Language::Groovy,
        "hs" | "hs-boot" => Language::Haskell,
        "hcl" | "tf" | "tfvars" | "nomad" => Language::Hcl,
        "html" | "htm" | "xhtml" | "shtml" => Language::Html,
        "ini" | "cfg" | "inf" | "service" | "socket" | "device" | "mount" | "automount"
        | "swap" | "target" | "path" | "timer" | "slice" | "scope" | "desktop" => Language::Ini,
        "java" => Language::Java,
        "js" | "mjs" | "cjs" => Language::JavaScript,
        "jsx" => Language::Jsx,
        "json" | "jsonc" | "json5" | "geojson" | "webmanifest" | "har" => Language::Json,
        "jl" => Language::Julia,
        "kt" | "kts" | "ktm" => Language::Kotlin,
        "less" => Language::Less,
        "lua" | "wlua" | "rockspec" | "luau" => Language::Lua,
        "mk" | "mak" | "make" => Language::Make,
        "md" | "markdown" | "mkd" | "mdown" => Language::Markdown,
        "nix" => Language::Nix,
        "m" | "mm" => Language::ObjectiveC,
        "ml" => Language::OCaml,
        "mli" => Language::OCamlInterface,
        "pl" | "pm" | "t" | "perl" | "psgi" => Language::Perl,
        "php" | "php3" | "php4" | "php5" | "phtml" | "inc" => Language::Php,
        "ps1" | "psm1" | "psd1" => Language::PowerShell,
        "properties" => Language::Properties,
        "proto" => Language::Protobuf,
        "py" | "pyi" | "pyw" => Language::Python,
        "r" | "rprofile" | "renviron" => Language::R,
        "rb" | "rake" | "gemspec" | "ru" | "rbw" | "jbuilder" => Language::Ruby,
        "rs" => Language::Rust,
        "scala" | "sc" | "sbt" => Language::Scala,
        "scss" => Language::Scss,
        "sql" => Language::Sql,
        "svelte" => Language::Svelte,
        "swift" => Language::Swift,
        "v" | "vh" | "sv" | "svh" => Language::SystemVerilog,
        "toml" => Language::Toml,
        "ts" | "mts" | "cts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "vhd" | "vhdl" => Language::Vhdl,
        "vue" => Language::Vue,
        "xml" | "svg" | "plist" | "xsl" | "xslt" | "xsd" | "rss" | "atom" | "wsdl" | "wsf"
        | "xaml" | "csproj" | "vbproj" | "fsproj" | "vcxproj" | "props" | "targets" | "nuspec"
        | "resx" | "storyboard" | "xib" | "kml" | "gpx" | "pom" | "iml" | "xul" | "glade" => {
            Language::Xml
        }
        "yaml" | "yml" => Language::Yaml,
        "zig" | "zon" => Language::Zig,
        _ => return None,
    })
}

fn by_whole_name(base: &str) -> Option<Language> {
    Some(match base {
        "Makefile" | "makefile" | "GNUmakefile" => Language::Make,
        "Dockerfile" | "dockerfile" | "Containerfile" | "containerfile" => Language::Dockerfile,
        "Gemfile" | "Rakefile" | "Guardfile" | "Podfile" | "Vagrantfile" | "Brewfile"
        | "Fastfile" | "Appfile" | "Capfile" => Language::Ruby,
        "Cargo.lock" | "Pipfile" | "poetry.lock" | "uv.lock" => Language::Toml,
        "PKGBUILD" | "APKBUILD" | ".bashrc" | ".bash_profile" | ".bash_logout"
        | ".bash_aliases" | ".profile" | ".zshrc" | ".zshenv" | ".zprofile" | ".zlogin"
        | ".zlogout" | ".envrc" | ".env" => Language::Bash,
        ".babelrc" | ".swcrc" => Language::Json,
        "CMakeLists.txt" => Language::CMake,
        "Jenkinsfile" => Language::Groovy,
        "rebar.config" | "rebar.config.script" | "sys.config" | "Emakefile" => Language::Erlang,
        "mix.lock" => Language::Elixir,
        ".gitconfig" | ".gitmodules" | ".editorconfig" | ".pylintrc" | "pylintrc" => Language::Ini,
        ".Rprofile" | ".Renviron" | ".Rhistory" => Language::R,
        "flake.lock" => Language::Json,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`Language::bundled`] counts the grammars' queries: diffr's own all
    /// say so in their first line, the bundled and vendored ones don't.
    #[test]
    fn bundled_counts() {
        for &l in Language::ALL {
            for (i, q) in l.highlights().into_iter().enumerate() {
                let first = q.lines().next().unwrap_or("");
                let ours = first.starts_with("; diffr") || first.starts_with("; Written for diffr");
                assert_eq!(ours, i >= l.bundled(), "{}: query {i}: {first}", l.name());
            }
        }
    }

    #[test]
    fn injected_tags() {
        for (name, want) in [
            ("<script>", Some(Language::JavaScript)),
            ("<script setup lang=\"ts\">", Some(Language::TypeScript)),
            ("<script lang='tsx'>", Some(Language::Tsx)),
            ("<style scoped>", Some(Language::Css)),
            ("<style lang=\"scss\">", Some(Language::Scss)),
            ("<style lang=\"postcss\">", Some(Language::Scss)),
            ("<style lang=\"less\">", Some(Language::Less)),
            ("javascript", Some(Language::JavaScript)),
            ("phpdoc", None),
        ] {
            assert_eq!(injection_language(name), want, "{name}");
        }
    }
}
