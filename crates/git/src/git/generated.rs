//! Detection of generated files: by git attributes, or by their names.

use super::Repo;
use crate::diff::File;
use std::collections::HashMap;

/// Directories of build output and dependencies, whose untracked files show
/// as one collapsed row.
pub const GENERATED_DIRS: &[&str] = &[
    ".cache",
    ".next",
    ".parcel-cache",
    ".pnpm-store",
    ".turbo",
    ".yarn",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "out",
    "target",
    "vendor",
];

const GENERATED_SEGMENTS: &[&str] = &[
    "__generated__",
    "__snapshots__",
    ".generated",
    "codegen",
    "gen",
    "generated",
    "generated-sources",
    "generated-src",
];

const GENERATED_NAMES: &[&str] = &[
    "bun.lock",
    "bun.lockb",
    "cargo.lock",
    "gemfile.lock",
    "npm-shrinkwrap.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "poetry.lock",
    "pubspec.lock",
    "uv.lock",
    "yarn.lock",
    "go.sum",
];

const GENERATED_SUFFIXES: &[&str] = &[
    ".d.ts.map",
    ".g.dart",
    ".generated.cjs",
    ".generated.css",
    ".generated.js",
    ".generated.jsx",
    ".generated.mjs",
    ".generated.ts",
    ".generated.tsx",
    ".pb.go",
    ".pb.gw.go",
    ".snap",
    ".snapshot",
    "-generated.js",
    "-generated.ts",
    "-generated.tsx",
    ".min.cjs",
    ".min.css",
    ".min.js",
    ".min.mjs",
    ".pb.cc",
    ".pb.h",
    ".pb.rb",
    "_generated.go",
    "_generated.rs",
    "_pb2.py",
    "_pb2_grpc.py",
];

/// Whether a path is of a file tools usually write, by its name.
pub fn looks_generated(path: &str) -> bool {
    let p = path.replace('\\', "/").to_lowercase();
    let parts: Vec<&str> = p.split('/').collect();
    if parts.iter().any(|part| GENERATED_SEGMENTS.contains(part)) {
        return true;
    }
    let base = parts[parts.len() - 1];
    if GENERATED_NAMES.contains(&base) {
        return true;
    }
    if GENERATED_SUFFIXES.iter().any(|s| base.ends_with(s)) {
        return true;
    }
    base.ends_with(".map") && !base.ends_with(".importmap")
}

impl Repo {
    /// Marks the files git attributes (`linguist-generated`,
    /// `gitlab-generated`) or their names say are generated. `source` is the
    /// commit whose attributes apply, `None` for the work tree's.
    pub fn mark_generated(&self, files: &mut [File], source: Option<&str>) {
        if files.is_empty() {
            return;
        }
        let mut stdin = Vec::new();
        for f in files.iter() {
            stdin.extend_from_slice(f.path.as_bytes());
            stdin.push(0);
        }
        let mut args = vec!["check-attr"];
        if let Some(src) = source.filter(|s| !s.is_empty()) {
            args.push("--source");
            args.push(src);
        }
        args.extend(["-z", "--stdin", "linguist-generated", "gitlab-generated"]);
        let mut decided: HashMap<String, bool> = HashMap::new();
        if let Ok(out) = self.run_with_stdin(Some(stdin), &args) {
            let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
            let mut i = 0;
            while i + 2 < fields.len() {
                let path = String::from_utf8_lossy(fields[i]).into_owned();
                match fields[i + 2] {
                    b"unspecified" => {}
                    b"unset" | b"false" => {
                        decided.entry(path).or_insert(false);
                    }
                    _ => {
                        decided.insert(path, true);
                    }
                }
                i += 3;
            }
        }
        for f in files.iter_mut() {
            f.generated = match decided.get(&f.path) {
                Some(g) => *g,
                None => looks_generated(&f.path),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(looks_generated("pnpm-lock.yaml"));
        assert!(looks_generated("Cargo.lock"));
        assert!(looks_generated("src/__generated__/x.ts"));
        assert!(looks_generated("dist/app.min.js"));
        assert!(looks_generated("a/b.js.map"));
        assert!(!looks_generated("a/b.importmap"));
        assert!(!looks_generated("src/main.rs"));
    }
}
