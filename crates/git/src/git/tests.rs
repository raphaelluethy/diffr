use super::*;
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("run git");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "git {args:?}: {}\n{text}", out.status);
    text
}

pub(crate) fn write(dir: &Path, path: &str, content: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, content).unwrap();
}

fn new_repo() -> (tempfile::TempDir, Repo) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git_in(dir, &["init", "-q", "-b", "main"]);
    git_in(dir, &["config", "user.name", "T"]);
    git_in(dir, &["config", "user.email", "t@example.com"]);
    // Not the user's own ignore file, which may hide node_modules.
    git_in(dir, &["config", "core.excludesFile", "/dev/null"]);
    write(dir, "a.txt", "one\ntwo\nthree\n");
    write(dir, "src/b.go", "package b\n");
    git_in(dir, &["add", "."]);
    git_in(dir, &["commit", "-q", "-m", "first"]);
    let r = Repo::open(dir.join("src")).unwrap();
    (tmp, r)
}

#[test]
fn test_working_tree_and_commit() {
    let (tmp, r) = new_repo();
    let dir = tmp.path();
    write(dir, "a.txt", "one\n2\nthree\n");
    write(dir, "new.md", "hello\n");
    write(dir, "node_modules/x/index.js", "x\n");
    write(dir, "pnpm-lock.yaml", "lock\n");
    std::fs::remove_file(dir.join("src/b.go")).unwrap();

    let files = r.working_tree(Options::default()).unwrap();
    let got: HashMap<&str, &File> = files.iter().map(|f| (f.path.as_str(), f)).collect();
    let f = got.get("a.txt");
    assert!(
        f.is_some_and(|f| f.status == Status::Modified && f.additions == 1 && f.deletions == 1),
        "a.txt: {f:?}"
    );
    let f = got.get("new.md");
    assert!(
        f.is_some_and(|f| f.status == Status::Untracked && f.additions == 1),
        "new.md: {f:?}"
    );
    let f = got.get("src/b.go");
    assert!(
        f.is_some_and(|f| f.status == Status::Deleted),
        "src/b.go: {f:?}"
    );
    let f = got.get("node_modules");
    assert!(f.is_some_and(|f| f.directory), "node_modules: {f:?}");
    let f = got.get("pnpm-lock.yaml");
    assert!(f.is_some_and(|f| f.generated), "pnpm-lock.yaml: {f:?}");
    assert!(
        files[0].path == "src/b.go" && files[1].path == "a.txt",
        "order: {}, {}",
        files[0].path,
        files[1].path
    );

    let sig = r.status_signature();
    write(dir, "a.txt", "one\n22\nthree\n");
    assert_ne!(
        r.status_signature(),
        sig,
        "signature unchanged after an edit"
    );

    let hash = r
        .commit_changes("Change a\n\nDetails.\n", &["a.txt", "src/b.go", "new.md"])
        .unwrap();
    assert!(!hash.is_empty(), "no hash");
    let log = r.log(0, 10).unwrap();
    assert!(
        log.len() == 2 && log[0].subject == "Change a" && log[0].body == "Details.",
        "log {log:?}"
    );
    let changed = r.commit_diff(&log[0], Options::default()).unwrap();
    assert_eq!(changed.len(), 3, "commit diff {changed:?}");
    let status = git_in(dir, &["status", "--porcelain"]);
    assert!(
        status.contains("?? pnpm-lock.yaml") && !status.contains("a.txt"),
        "status after commit:\n{status}"
    );

    let c = r.new_contents().unwrap();
    let data = c.read(&log[1].hash, "a.txt").unwrap();
    assert_eq!(data.as_deref(), Some(&b"one\ntwo\nthree\n"[..]));
    let data = c.read("HEAD", "missing.txt").unwrap();
    assert_eq!(data, None);
    c.close().unwrap();
}

// Further tests.

#[test]
fn test_branch_resolve_and_ranges() {
    let (tmp, r) = new_repo();
    let dir = tmp.path();
    assert_eq!(r.branch(), "main");
    assert!(r.has_head());
    assert_eq!(r.config_value("user.name"), "T");
    assert_eq!(r.config_value("no.such"), "");
    let head = r.resolve("HEAD").unwrap();
    assert_eq!(head.len(), 40);
    assert!(r.has_commit(&head));
    assert!(r.resolve("nope").is_err());
    let info = r.commit_info("HEAD").unwrap();
    assert_eq!(info.subject, "first");
    assert!(info.parents.is_empty());
    let first = r.commit_diff(&info, Options::default()).unwrap();
    assert_eq!(first.len(), 2);

    git_in(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "a.txt", "one\ntwo\nthree\nfour\n");
    git_in(dir, &["commit", "-q", "-am", "four"]);
    let (files, mb) = r.range("main", "feature", Options::default()).unwrap();
    assert_eq!(mb, head);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].additions, 1);
    assert_eq!(r.subjects("main", "feature", 10).len(), 1);
    write(dir, "c.txt", "c\n");
    let (files, _) = r.compare("main", Options::default()).unwrap();
    assert_eq!(files.len(), 2);
    assert!(r.remotes().is_empty());
    git_in(
        dir,
        &["remote", "add", "origin", "https://example.com/x.git"],
    );
    assert_eq!(
        r.remotes(),
        vec![Remote {
            name: "origin".into(),
            url: "https://example.com/x.git".into()
        }]
    );
    assert_eq!(r.read_work_tree("c.txt").as_deref(), Some(&b"c\n"[..]));
    assert!(r.read_work_tree("nope").is_none());
}

#[test]
fn test_open_outside() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(matches!(
        Repo::open(tmp.path()),
        Err(Error::NotRepository(_))
    ));
}
