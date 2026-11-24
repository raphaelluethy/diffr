//! Groups the files of a review, as pulls.review does: by their kind
//! (docs, code, tests, config, dependencies, generated, other), or by
//! intent as an agent's review has them; and counts what of each group
//! was viewed. The parts of grouping that are not UI.

use std::cmp::Ordering;
use std::collections::HashMap;

use diffr_agent::Analysis;
use serde::{Deserialize, Serialize};

/// How the review groups its files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupMode {
    /// By the agent's review once there is one, else by kind for pull
    /// requests, else not at all.
    #[default]
    Auto,
    None,
    Kind,
    Ai,
}

/// How the files group now: as the user chose (`pref`), else as fits the
/// source. Grouping by an AI review there is not yet falls back to
/// automatic.
pub fn effective_mode(pref: GroupMode, has_analysis: bool, is_pull: bool) -> GroupMode {
    let mut mode = pref;
    if mode == GroupMode::Ai && !has_analysis {
        mode = GroupMode::Auto;
    }
    if mode != GroupMode::Auto {
        return mode;
    }
    if has_analysis {
        GroupMode::Ai
    } else if is_pull {
        GroupMode::Kind
    } else {
        GroupMode::None
    }
}

/// A kind of file, as pulls.review's rules sort them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Kind {
    pub key: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
}

/// The kinds of files, in the order their groups show.
pub const KINDS: [Kind; 7] = [
    Kind {
        key: "docs",
        label: "Docs",
        summary: "Documentation updates.",
    },
    Kind {
        key: "code",
        label: "Code",
        summary: "Source changes.",
    },
    Kind {
        key: "tests",
        label: "Tests",
        summary: "Test coverage for the change.",
    },
    Kind {
        key: "config",
        label: "Config",
        summary: "Configuration changes.",
    },
    Kind {
        key: "deps",
        label: "Dependencies",
        summary: "Dependency version changes.",
    },
    Kind {
        key: "generated",
        label: "Generated",
        summary: "Generated or build output changes.",
    },
    Kind {
        key: "other",
        label: "Other",
        summary: "Other changes that don't fit an existing category.",
    },
];

fn kind_index(key: &str) -> usize {
    KINDS.iter().position(|k| k.key == key).unwrap_or(1)
}

/// A file of the review, as grouping needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupFile {
    pub path: String,
    /// The path before a rename, "" or `path` when not renamed.
    pub old_path: String,
    pub binary: bool,
    /// Generated, as by its name (lockfiles, `.min.js`…) or attributes.
    pub generated: bool,
    /// A collapsed directory of untracked files.
    pub directory: bool,
    pub too_large: bool,
    pub additions: usize,
    pub deletions: usize,
    /// What the file's content is, to tell whether it was viewed as it is
    /// now; "" when unknown.
    pub fingerprint: String,
}

impl GroupFile {
    /// Whether its lines count in a group's +/−.
    pub fn countable(&self) -> bool {
        !self.binary && !self.directory && !self.too_large
    }

    /// Whether it was viewed as it is now, `viewed` holding the
    /// fingerprints of the files viewed by path.
    pub fn is_viewed(&self, viewed: &HashMap<String, String>) -> bool {
        !self.fingerprint.is_empty() && viewed.get(&self.path) == Some(&self.fingerprint)
    }
}

/// The index in [`KINDS`] of a file's kind. The first rule that matches
/// wins: a document under `.github` is a document.
pub fn kind_of(f: &GroupFile) -> usize {
    if f.directory {
        return kind_index("generated");
    }
    if f.binary {
        return kind_index("other");
    }
    let p = f.path.to_lowercase();
    let parts: Vec<&str> = p.split('/').collect();
    let (base, dirs) = parts.split_last().map_or(("", &[][..]), |(b, d)| (*b, d));
    let in_dir = |names: &[&str]| dirs.iter().any(|d| names.contains(d));
    let ext = match base.rfind('.') {
        Some(i) if i > 0 => &base[i..],
        _ => "",
    };
    if matches!(ext, ".md" | ".mdx" | ".mdc" | ".rst" | ".adoc")
        || in_dir(&["docs"])
        || base.starts_with("readme")
    {
        return kind_index("docs");
    }
    if base.contains(".test.")
        || base.contains(".spec.")
        || in_dir(&["__tests__", "test", "tests", "testdata"])
        || base.ends_with("_test.go")
        || base.ends_with("_test.py")
        || (base.starts_with("test_") && ext == ".py")
        || base.ends_with("_spec.rb")
    {
        return kind_index("tests");
    }
    if (dirs.is_empty()
        && (base.contains(".config.")
            || base.starts_with(".git")
            || (base.starts_with('.') && (base.ends_with("rc") || base.contains("rc.")))))
        || dirs.first() == Some(&".github")
        || (base.starts_with("tsconfig") && ext == ".json")
        || base == "dockerfile"
        || base == ".dockerignore"
        || base == "makefile"
        || (base.starts_with("docker-compose") && (ext == ".yml" || ext == ".yaml"))
    {
        return kind_index("config");
    }
    if [
        "package.json",
        "pnpm-workspace.yaml",
        "cargo.toml",
        "go.mod",
        "gemfile",
        "requirements.txt",
    ]
    .contains(&base)
    {
        return kind_index("deps");
    }
    if f.generated || in_dir(&["dist"]) || ext == ".lock" || base.contains(".generated.") {
        return kind_index("generated");
    }
    kind_index("code")
}

/// A group of the files, which show together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileGroup {
    pub key: String,
    pub label: String,
    pub summary: String,
    /// Deserves extra care ("Needs care").
    pub critical: bool,
    /// One of an agent's review (a sparkle), not a kind (layers).
    pub ai: bool,
}

/// The files grouped, and ordered by their groups.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grouping {
    pub groups: Vec<FileGroup>,
    /// The group of each file, by its index in the files given; `None`
    /// when not grouped.
    pub group_of: Vec<Option<usize>>,
    /// The indices of the files in the order they show: by group, then as
    /// a file tree lists them.
    pub order: Vec<usize>,
}

impl Grouping {
    /// The indices of a group's files, in their order.
    pub fn files_of(&self, group: usize) -> Vec<usize> {
        self.order
            .iter()
            .copied()
            .filter(|&i| self.group_of[i] == Some(group))
            .collect()
    }
}

/// Groups the files as `mode` says ([`effective_mode`] first). `analysis`
/// is needed for [`GroupMode::Ai`] only.
pub fn group(mode: GroupMode, files: &[GroupFile], analysis: Option<&Analysis>) -> Grouping {
    match (mode, analysis) {
        (GroupMode::Kind, _) => group_by_kind(files),
        (GroupMode::Ai, Some(a)) => group_by_analysis(a, files),
        _ => finish(Vec::new(), vec![None; files.len()], files),
    }
}

/// Groups the files by kind: the groups in [`KINDS`]' order, the empty ones
/// left out.
pub fn group_by_kind(files: &[GroupFile]) -> Grouping {
    let ranks: Vec<usize> = files.iter().map(kind_of).collect();
    let mut used = [None; KINDS.len()];
    let mut groups = Vec::new();
    for (k, kind) in KINDS.iter().enumerate() {
        if ranks.contains(&k) {
            used[k] = Some(groups.len());
            groups.push(FileGroup {
                key: kind.key.into(),
                label: kind.label.into(),
                summary: kind.summary.into(),
                ..Default::default()
            });
        }
    }
    let group_of = ranks.iter().map(|&r| used[r]).collect();
    finish(groups, group_of, files)
}

/// Groups the files by intent, as an agent's review has them. Files new
/// since the review gather at the end ("Not in the review"), and groups
/// left without files go.
pub fn group_by_analysis(a: &Analysis, files: &[GroupFile]) -> Grouping {
    let mut groups: Vec<FileGroup> = a
        .groups
        .iter()
        .map(|g| FileGroup {
            key: g.key.clone(),
            label: g.label.clone(),
            summary: g.summary.clone(),
            critical: g.critical,
            ai: true,
        })
        .collect();
    let by_path = diffr_agent::group_of_paths(a);
    let mut rest = None;
    let mut group_of: Vec<usize> = files
        .iter()
        .map(|f| {
            by_path
                .get(f.path.as_str())
                .or_else(|| {
                    (f.old_path != f.path)
                        .then(|| by_path.get(f.old_path.as_str()))
                        .flatten()
                })
                .copied()
                .unwrap_or_else(|| {
                    *rest.get_or_insert_with(|| {
                        groups.push(FileGroup {
                            key: "unreviewed".into(),
                            label: "Not in the review".into(),
                            summary: "Files that changed since the agent's review.".into(),
                            critical: false,
                            ai: true,
                        });
                        groups.len() - 1
                    })
                })
        })
        .collect();
    // Groups left without files go, the indices following.
    let mut count = vec![0; groups.len()];
    for &g in &group_of {
        count[g] += 1;
    }
    let mut remap = vec![0; groups.len()];
    let mut kept = Vec::new();
    for (gi, g) in groups.into_iter().enumerate() {
        if count[gi] > 0 {
            remap[gi] = kept.len();
            kept.push(g);
        }
    }
    for g in &mut group_of {
        *g = remap[*g];
    }
    finish(kept, group_of.into_iter().map(Some).collect(), files)
}

fn finish(groups: Vec<FileGroup>, group_of: Vec<Option<usize>>, files: &[GroupFile]) -> Grouping {
    let mut order: Vec<usize> = (0..files.len()).collect();
    // Ungrouped files (None) come first.
    order.sort_by(|&a, &b| {
        group_of[a]
            .cmp(&group_of[b])
            .then_with(|| compare_paths(&files[a].path, &files[b].path))
    });
    Grouping {
        groups,
        group_of,
        order,
    }
}

/// Orders two paths as a file tree lists them: at each level, directories
/// before files, each by name, case aside first.
pub fn compare_paths(a: &str, b: &str) -> Ordering {
    let r#as: Vec<&str> = a.split('/').collect();
    let bs: Vec<&str> = b.split('/').collect();
    for i in 0..r#as.len().min(bs.len()) {
        let a_dir = i < r#as.len() - 1;
        let b_dir = i < bs.len() - 1;
        if a_dir != b_dir {
            return if a_dir {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let c = r#as[i]
            .to_lowercase()
            .cmp(&bs[i].to_lowercase())
            .then_with(|| r#as[i].cmp(bs[i]));
        if c != Ordering::Equal {
            return c;
        }
    }
    r#as.len().cmp(&bs.len())
}

/// What of a group was viewed, and its size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupCount {
    pub viewed: usize,
    pub total: usize,
    /// The lines added and removed of its countable files.
    pub additions: usize,
    pub deletions: usize,
}

impl GroupCount {
    /// Every file of the group was viewed.
    pub fn done(&self) -> bool {
        self.total > 0 && self.viewed == self.total
    }
}

/// Counts, for each group, the files viewed (as they are now: `viewed`
/// holds the fingerprints viewed by path), the files, and their lines.
pub fn group_counts(
    grouping: &Grouping,
    files: &[GroupFile],
    viewed: &HashMap<String, String>,
) -> Vec<GroupCount> {
    let mut out = vec![GroupCount::default(); grouping.groups.len()];
    for (i, f) in files.iter().enumerate() {
        let Some(g) = grouping.group_of.get(i).copied().flatten() else {
            continue;
        };
        let c = &mut out[g];
        c.total += 1;
        if f.is_viewed(viewed) {
            c.viewed += 1;
        }
        if f.countable() {
            c.additions += f.additions;
            c.deletions += f.deletions;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use diffr_agent::Group;

    fn file(path: &str) -> GroupFile {
        GroupFile {
            path: path.into(),
            ..Default::default()
        }
    }

    #[test]
    fn kinds() {
        for (path, want) in [
            ("README.md", "docs"),
            ("docs/guide/setup.ts", "docs"),
            (".github/CONTRIBUTING.md", "docs"),
            ("src/app.go", "code"),
            ("src/app_test.go", "tests"),
            ("web/button.spec.tsx", "tests"),
            ("tests/fixtures/a.json", "tests"),
            ("vite.config.ts", "config"),
            (".eslintrc.json", "config"),
            (".github/workflows/ci.yml", "config"),
            ("packages/a/tsconfig.json", "config"),
            ("Dockerfile", "config"),
            ("go.mod", "deps"),
            ("packages/a/package.json", "deps"),
            ("dist/app.js", "generated"),
            ("Cargo.lock", "generated"),
            ("api/types.generated.ts", "generated"),
            ("packages/web/src/vite.ts", "code"),
            ("packages/web/config.ts", "code"),
            ("packages/web/src/.eslintrc", "code"),
        ] {
            assert_eq!(KINDS[kind_of(&file(path))].key, want, "{path}");
        }
        let lock = GroupFile {
            generated: true,
            ..file("pnpm-lock.yaml")
        };
        assert_eq!(KINDS[kind_of(&lock)].key, "generated");
        let png = GroupFile {
            binary: true,
            ..file("logo.png")
        };
        assert_eq!(KINDS[kind_of(&png)].key, "other");
    }

    #[test]
    fn modes() {
        use GroupMode::*;
        assert_eq!(effective_mode(Auto, false, true), Kind);
        assert_eq!(effective_mode(Auto, false, false), None);
        assert_eq!(effective_mode(Auto, true, false), Ai);
        assert_eq!(effective_mode(Ai, false, true), Kind);
        assert_eq!(effective_mode(None, true, true), None);
    }

    #[test]
    fn by_kind() {
        let files = vec![
            file("src/b.go"),
            file("README.md"),
            file("src/a.go"),
            file("src/a_test.go"),
        ];
        let g = group_by_kind(&files);
        let keys: Vec<&str> = g.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["docs", "code", "tests"]);
        assert_eq!(g.order, [1, 2, 0, 3]);
        assert_eq!(g.files_of(1), [2, 0]);
        let mut viewed = HashMap::new();
        let mut files = files;
        files[0].fingerprint = "f0".into();
        files[0].additions = 3;
        files[2].fingerprint = "f2".into();
        viewed.insert("src/b.go".to_string(), "f0".to_string());
        viewed.insert("src/a.go".to_string(), "old".to_string());
        let counts = group_counts(&g, &files, &viewed);
        assert_eq!(
            counts[1],
            GroupCount {
                viewed: 1,
                total: 2,
                additions: 3,
                deletions: 0
            }
        );
        assert!(!counts[1].done());
    }

    #[test]
    fn by_analysis() {
        let a = Analysis {
            groups: vec![
                Group {
                    key: "core".into(),
                    label: "Core".into(),
                    critical: true,
                    file_paths: vec!["src/old.go".into(), "src/a.go".into()],
                    ..Default::default()
                },
                Group {
                    key: "gone".into(),
                    label: "Gone".into(),
                    file_paths: vec!["deleted.go".into()],
                    ..Default::default()
                },
                Group {
                    key: "docs".into(),
                    label: "Docs".into(),
                    file_paths: vec!["README.md".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let files = vec![
            file("README.md"),
            GroupFile {
                old_path: "src/old.go".into(),
                ..file("src/new.go")
            },
            file("src/a.go"),
            file("fresh.go"),
        ];
        let g = group(GroupMode::Ai, &files, Some(&a));
        let keys: Vec<&str> = g.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["core", "docs", "unreviewed"]);
        assert!(g.groups[0].critical && g.groups[0].ai);
        assert_eq!(g.group_of, [Some(1), Some(0), Some(0), Some(2)]);
        assert_eq!(g.order, [2, 1, 0, 3]);

        let none = group(GroupMode::None, &files, None);
        assert!(none.groups.is_empty());
        assert_eq!(none.order, [2, 1, 3, 0]);
    }

    #[test]
    fn tree_order() {
        let mut paths = vec!["b.go", "a/z.go", "A.go", "a.go", "a/b/c.go"];
        paths.sort_by(|a, b| compare_paths(a, b));
        assert_eq!(paths, ["a/b/c.go", "a/z.go", "A.go", "a.go", "b.go"]);
    }
}
