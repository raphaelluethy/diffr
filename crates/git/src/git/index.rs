//! The index: staging files for the next commit, committing what is
//! staged, and pushing the branch.

use std::collections::HashMap;
use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::{EMPTY_TREE, Error, Repo, Result, TempFile, command_error, git_command, nth_field, ssh_command};
use crate::diff::{self, File};

/// How much of a file's change is staged in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Staged {
    /// All of it: the index holds the file as the work tree does.
    Full,
    /// Some of it: the work tree changes it further.
    Partial,
}

/// Where the branch stands against its upstream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Upstream {
    /// The branch checked out, "" when HEAD is detached.
    pub branch: String,
    /// Its upstream, as `origin/main`; "" when it has none.
    pub name: String,
    /// The commits of HEAD the upstream lacks.
    pub ahead: usize,
    /// The commits of the upstream HEAD lacks.
    pub behind: usize,
    /// Whether the repository has a remote to push to.
    pub has_remote: bool,
}

impl Upstream {
    /// Whether there is anything to push: commits ahead, or a branch that
    /// is on no remote yet.
    pub fn can_push(&self) -> bool {
        !self.branch.is_empty() && self.has_remote && (self.name.is_empty() || self.ahead > 0)
    }
}

/// The NUL-separated list git reads paths from.
fn pathspecs(paths: &[impl AsRef<str>]) -> Vec<u8> {
    let mut b = Vec::new();
    for p in paths {
        b.extend_from_slice(p.as_ref().as_bytes());
        b.push(0);
    }
    b
}

/// Reads `git status --porcelain=v2 -z` into the paths staged, and how.
pub(crate) fn parse_index_status(out: &[u8]) -> HashMap<String, Staged> {
    let mut staged = HashMap::new();
    let fields: Vec<&[u8]> = out.split(|&c| c == 0).collect();
    let mut i = 0;
    while i < fields.len() {
        let rec = String::from_utf8_lossy(fields[i]);
        i += 1;
        let (xy, path) = if rec.starts_with("1 ") {
            (nth_field(&rec, 1), nth_field(&rec, 8))
        } else if rec.starts_with("2 ") {
            // The original path follows, in a field of its own.
            i += 1;
            (nth_field(&rec, 1), nth_field(&rec, 9))
        } else {
            // Untracked, ignored and unmerged files are not staged.
            continue;
        };
        let mut xy = xy.chars();
        let (x, y) = (xy.next().unwrap_or('.'), xy.next().unwrap_or('.'));
        if x == '.' {
            continue;
        }
        staged.insert(path.to_string(), if y == '.' { Staged::Full } else { Staged::Partial });
    }
    staged
}

impl Repo {
    /// The files staged in the index, by path, and whether the work tree
    /// changes them further.
    pub fn index_status(&self) -> Result<HashMap<String, Staged>> {
        let out = self.git(&[
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=no",
        ])?;
        Ok(parse_index_status(&out))
    }

    /// Stages the files at `paths` as the work tree has them, deleted ones
    /// included.
    pub fn stage(&self, paths: &[impl AsRef<str>]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.run_with_stdin(
            Some(pathspecs(paths)),
            &[
                "--literal-pathspecs",
                "add",
                "-A",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
        )?;
        Ok(())
    }

    /// Takes the files at `paths` out of the index, back to HEAD's, leaving
    /// the work tree alone.
    pub fn unstage(&self, paths: &[impl AsRef<str>]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        if self.has_head() {
            self.run_with_stdin(
                Some(pathspecs(paths)),
                &[
                    "--literal-pathspecs",
                    "reset",
                    "-q",
                    "HEAD",
                    "--pathspec-from-file=-",
                    "--pathspec-file-nul",
                ],
            )?;
        } else {
            // Before the first commit everything staged is new.
            self.run_with_stdin(
                Some(pathspecs(paths)),
                &[
                    "--literal-pathspecs",
                    "rm",
                    "--cached",
                    "-r",
                    "-q",
                    "--ignore-unmatch",
                    "--pathspec-from-file=-",
                    "--pathspec-file-nul",
                ],
            )?;
        }
        Ok(())
    }

    /// The changes staged in the index against HEAD.
    pub fn staged_files(&self) -> Result<Vec<File>> {
        let base = if self.has_head() { "HEAD" } else { EMPTY_TREE };
        let out = self.git(&[
            "-c",
            "core.quotePath=false",
            "diff",
            "--cached",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-M",
            "-U3",
            "--submodule=short",
            base,
            "--",
        ])?;
        let mut files = diff::parse(&out);
        self.mark_generated(&mut files, None);
        super::sort_files(&mut files);
        Ok(files)
    }

    /// Commits what the index holds, with a message, and returns the new
    /// commit's short hash.
    pub fn commit_staged(&self, message: &str) -> Result<String> {
        if message.trim().is_empty() {
            return Err(Error::Message("the commit message is empty".into()));
        }
        let msg = TempFile::new("diffr-message-")?;
        std::fs::write(&msg.0, message)?;
        let msg_path = msg.0.to_string_lossy().into_owned();
        self.git(&["commit", "-F", &msg_path])?;
        Ok(self.git_string(&["rev-parse", "--short", "HEAD"]))
    }

    /// Where the branch stands against its upstream.
    pub fn upstream(&self) -> Upstream {
        let mut u = Upstream {
            branch: self.git_string(&["symbolic-ref", "--short", "-q", "HEAD"]),
            has_remote: !self.git_string(&["remote"]).is_empty(),
            ..Default::default()
        };
        if u.branch.is_empty() {
            return u;
        }
        u.name = self.git_string(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]);
        if u.name.is_empty() {
            return u;
        }
        let counts = self.git_string(&["rev-list", "--left-right", "--count", "@{upstream}...HEAD"]);
        let mut it = counts.split_whitespace().map(|n| n.parse().unwrap_or(0));
        u.behind = it.next().unwrap_or(0);
        u.ahead = it.next().unwrap_or(0);
        u
    }

    /// Pushes the branch to its upstream, or, for a branch without one, to
    /// the remote `origin` (else the first) as its upstream. Never asks for
    /// credentials; gives up after three minutes. Returns what git said.
    pub fn push(&self) -> Result<String> {
        let up = self.upstream();
        if up.branch.is_empty() {
            return Err(Error::Message("HEAD is detached: check out a branch to push".into()));
        }
        let mut args: Vec<String> = vec!["-c".into(), "credential.interactive=never".into(), "push".into()];
        if up.name.is_empty() {
            let remotes = self.git_string(&["remote"]);
            let remotes: Vec<&str> = remotes.lines().map(str::trim).filter(|r| !r.is_empty()).collect();
            let remote = if remotes.contains(&"origin") {
                "origin"
            } else {
                remotes.first().copied().ok_or_else(|| Error::Message("the repository has no remote to push to".into()))?
            };
            args.extend(["-u".into(), remote.into(), up.branch.clone()]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut child = git_command(&self.root, &args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_SSH_COMMAND", ssh_command())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| command_error(&args, b"", e.to_string()))?;
        let read = |p: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut p) = p {
                    let _ = p.read_to_end(&mut buf);
                }
                buf
            })
        };
        let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let deadline = Instant::now() + Duration::from_secs(180);
        let status = loop {
            if let Some(st) = child.try_wait()? {
                break Some(st);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(30));
        };
        let mut said = out.join().unwrap_or_default();
        said.extend(err.join().unwrap_or_default());
        match status {
            Some(st) if st.success() => Ok(String::from_utf8_lossy(&said).trim().to_string()),
            Some(st) => Err(command_error(&args, &said, st.to_string())),
            None => Err(command_error(&args, &said, "timed out".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_status() {
        let out = b"1 M. N... 100644 100644 100644 a b src/full.rs\x00\
1 MM N... 100644 100644 100644 a b src/partial.rs\x00\
1 .M N... 100644 100644 100644 a b src/unstaged.rs\x00\
2 R. N... 100644 100644 100644 a b R100 new name.rs\x00old name.rs\x00\
1 A. N... 000000 100644 100644 0 b added.rs\x00\
? untracked.rs\x00";
        let s = parse_index_status(out);
        assert_eq!(s.get("src/full.rs"), Some(&Staged::Full));
        assert_eq!(s.get("src/partial.rs"), Some(&Staged::Partial));
        assert_eq!(s.get("src/unstaged.rs"), None);
        assert_eq!(s.get("new name.rs"), Some(&Staged::Full));
        assert_eq!(s.get("old name.rs"), None);
        assert_eq!(s.get("added.rs"), Some(&Staged::Full));
        assert_eq!(s.len(), 4);
    }

    fn repo() -> (tempfile::TempDir, Repo) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git").args(args).current_dir(dir.path()).output().unwrap();
            assert!(ok.status.success(), "{args:?}: {}", String::from_utf8_lossy(&ok.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "commit.gpgsign", "false"]);
        let r = Repo::open(dir.path()).unwrap();
        (dir, r)
    }

    #[test]
    fn stage_unstage_and_commit() {
        let (dir, r) = repo();
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "b\n").unwrap();
        // Before the first commit.
        r.stage(&["a.txt", "b.txt"]).unwrap();
        assert_eq!(r.index_status().unwrap().len(), 2);
        r.unstage(&["b.txt"]).unwrap();
        assert_eq!(r.index_status().unwrap().keys().collect::<Vec<_>>(), ["a.txt"]);
        assert_eq!(r.staged_files().unwrap().len(), 1);
        r.commit_staged("feat: add a\n").unwrap();
        assert!(r.index_status().unwrap().is_empty());
        // b stays out of the commit.
        assert_eq!(r.untracked().unwrap(), ["b.txt"]);

        std::fs::write(dir.path().join("a.txt"), "a2\n").unwrap();
        r.stage(&["a.txt"]).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a3\n").unwrap();
        assert_eq!(r.index_status().unwrap().get("a.txt"), Some(&Staged::Partial));
        r.unstage(&["a.txt"]).unwrap();
        assert!(r.index_status().unwrap().is_empty());
        assert!(r.commit_staged("  ").is_err());

        let up = r.upstream();
        assert_eq!(up.branch, "main");
        assert!(up.name.is_empty() && !up.has_remote && !up.can_push());
        assert!(r.push().is_err());
    }
}
