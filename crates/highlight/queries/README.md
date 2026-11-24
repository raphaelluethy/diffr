# Highlight queries

The grammars' own `highlights.scm` (bundled in each `tree-sitter-*` crate,
permissively licensed, see below) are used as-is, but for the rewrites at
the end; the files here are appended after them.
tree-sitter-highlight lets the **last** pattern matching a node win, so these
re-assert or correct captures where the bundled query's order or choices give
poor results for diffr's classes (see `src/classes.rs`).

- `*.scm` other than below: written for diffr, under the workspace's
  license. For GraphQL, Groovy, HCL, Perl and SystemVerilog, whose crates
  ship no highlights query, `graphql.scm`, `groovy.scm`, `hcl.scm`,
  `perl.scm` and `systemverilog.scm` are the whole query (the
  SystemVerilog keywords are the grammar's word tokens); `*-injections.scm`
  replace or stand in for the grammars' injections.
- `julia-highlights.scm`: tree-sitter-julia 0.23.1's `queries/highlights.scm`
  (MIT, Max Brunsfeld and contributors), vendored unchanged: the crate ships
  it but does not export it.
- `proto-highlights.scm`: tree-sitter-proto 0.6.0's `queries/highlights.scm`
  (MIT, Copyright (c) 2024-2025 Mohammad Ashar Khan), likewise.
- `markdown-injections.scm`: tree-sitter-md 0.5's block injections (MIT,
  Copyright (c) 2021 Matthias Deiml), with `injection.include-children` added
  for inline content.

Zed's own queries (`zed-industries/zed`, `crates/languages/src/*/`) are not
vendored.

The bundled queries are all under permissive licenses: MIT for most,
Apache-2.0 for tree-sitter-elixir and tree-sitter-ini, CC0 for
tree-sitter-clojure-orchard;
tree-sitter-kotlin-sg's query derives from nvim-treesitter's (Apache-2.0).

Some bundled queries are written for Neovim: `src/queryfix.rs` turns their
`#lua-match?` predicates into `#match?` and drops patterns using predicates
tree-sitter does not evaluate (`#has-ancestor?`, …), which would otherwise
match every node; and reverses the patterns of queries written for editors
where the first matching pattern wins (Nix).
