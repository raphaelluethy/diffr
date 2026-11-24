//! Reads the changes of a repository and commits them, through the git
//! command line.

mod generated;
mod index;
mod sort;

pub use generated::{GENERATED_DIRS, looks_generated};
pub use index::{Staged, Upstream};
pub use sort::{compare_paths, sort_files};

use crate::diff::{self, File, Status};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The hash of git's empty tree, the base of a repository without commits.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The size of patch beyond which a file's lines are not shown.
pub const MAX_PATCH: u64 = 4 << 20;

/// The number of untracked files listed one by one.
pub const MAX_UNTRACKED: usize = 1000;

/// An error of this module.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// [`Repo::open`] was given a directory outside any work tree.
    #[error("{}: not a git repository", .0.display())]
    NotRepository(PathBuf),
    /// A git command that failed, with what it printed.
    #[error("git {}: {}", .args.join(" "), command_message(.stderr, .cause))]
    Command {
        /// The arguments git ran with.
        args: Vec<String>,
        /// What git printed on its standard error.
        stderr: String,
        /// How it failed: its exit status, or why it did not run.
        cause: String,
    },
    /// Reading or writing a file, or starting a process, failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Any other failure, as a message.
    #[error("{0}")]
    Message(String),
}

fn command_message(stderr: &str, cause: &str) -> String {
    let msg = stderr.trim();
    if msg.is_empty() {
        cause.to_string()
    } else {
        msg.to_string()
    }
}

/// The result of this module's operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Options that change how changes are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    /// Keeps changes of whitespace alone.
    pub show_whitespace: bool,
}

/// A git repository's work tree. Cheap to clone, and safe to use from any
/// thread: every call runs its own git process.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repo {
    root: PathBuf,
}

/// Whether to log every git command with how long it took.
fn debug() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("DIFFR_DEBUG").is_some_and(|v| !v.is_empty()))
}

/// The git to run. On macOS, /usr/bin/git is a shim that finds the developer
/// tools' git each time it runs, which doubles what starting git takes: the
/// git it would run is found once instead.
fn binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let Some(path) = look_path("git") else {
            return PathBuf::from("git");
        };
        if cfg!(target_os = "macos") && path == Path::new("/usr/bin/git") {
            // xcode-select prints the developer directory without asking to
            // install the tools, as xcrun would.
            if let Ok(out) = Command::new("/usr/bin/xcode-select").arg("-p").output()
                && out.status.success()
            {
                let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let real = Path::new(&dir).join("usr").join("bin").join("git");
                if real.is_file() {
                    return real;
                }
            }
        }
        path
    })
}

/// Finds an executable in the PATH.
fn look_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&paths) {
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if is_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    let Ok(md) = std::fs::metadata(p) else {
        return false;
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// A git command, with no console window on Windows.
fn git_command(dir: &Path, args: &[impl AsRef<OsStr>]) -> Command {
    let mut cmd = Command::new(binary());
    cmd.args(args).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn command_error(args: &[&str], stderr: &[u8], cause: String) -> Error {
    Error::Command {
        args: args.iter().map(|s| s.to_string()).collect(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
        cause,
    }
}

/// Runs git in `dir`, with `stdin` written to it, and returns what it
/// printed.
fn run(dir: &Path, stdin: Option<Vec<u8>>, args: &[&str]) -> Result<Vec<u8>> {
    let start = Instant::now();
    let mut cmd = git_command(dir, args);
    // Reading must not take the index lock from the user's own git
    // commands, and must print paths and messages the same everywhere.
    cmd.env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let result = (|| {
        let mut child = cmd
            .spawn()
            .map_err(|e| command_error(args, b"", e.to_string()))?;
        // Write in another thread: git may print before it has read all.
        let writer = match (stdin, child.stdin.take()) {
            (Some(data), Some(mut pipe)) => Some(std::thread::spawn(move || {
                let _ = pipe.write_all(&data);
            })),
            _ => None,
        };
        let out = child
            .wait_with_output()
            .map_err(|e| command_error(args, b"", e.to_string()))?;
        if let Some(w) = writer {
            let _ = w.join();
        }
        if !out.status.success() {
            return Err(command_error(args, &out.stderr, out.status.to_string()));
        }
        Ok(out.stdout)
    })();
    if debug() {
        eprintln!("git {}: {:?}", args.join(" "), start.elapsed());
    }
    result
}

/// The options of every diff: no colors, external tools or prefixes of the
/// user's configuration, and renames detected.
fn diff_args(opts: Options) -> Vec<&'static str> {
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "-M",
        "-U3",
        "--submodule=short",
    ];
    if !opts.show_whitespace {
        args.extend(["--ignore-space-at-eol", "-w"]);
    }
    args
}

fn trim_string(out: &[u8]) -> String {
    String::from_utf8_lossy(out).trim().to_string()
}

fn mod_time_nanos(md: &std::fs::Metadata) -> i128 {
    match md.modified() {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as i128,
            Err(e) => -(e.duration().as_nanos() as i128),
        },
        Err(_) => 0,
    }
}

fn file_mode(md: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        md.mode() as u64
    }
    #[cfg(not(unix))]
    {
        let ft = md.file_type();
        (ft.is_dir() as u64) << 2
            | (ft.is_symlink() as u64) << 1
            | md.permissions().readonly() as u64
    }
}

impl Repo {
    /// Finds the repository holding `dir` (a directory, or a file in one).
    pub fn open(dir: impl AsRef<Path>) -> Result<Repo> {
        let abs = std::path::absolute(dir.as_ref())?;
        let md = std::fs::metadata(&abs)?;
        let abs = if md.is_dir() {
            abs
        } else {
            abs.parent().map(Path::to_path_buf).unwrap_or(abs)
        };
        let out = run(&abs, None, &["rev-parse", "--show-toplevel"])
            .map_err(|_| Error::NotRepository(abs.clone()))?;
        Ok(Repo {
            root: PathBuf::from(trim_string(&out)),
        })
    }

    /// The top directory of the work tree.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs git in the repository and returns what it printed.
    pub fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        run(&self.root, None, args)
    }

    fn run_with_stdin(&self, stdin: Option<Vec<u8>>, args: &[&str]) -> Result<Vec<u8>> {
        run(&self.root, stdin, args)
    }

    fn git_string(&self, args: &[&str]) -> String {
        self.git(args).map(|o| trim_string(&o)).unwrap_or_default()
    }

    fn full_path(&self, path: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in path.split('/') {
            p.push(part);
        }
        p
    }

    /// The name of the branch checked out, or the abbreviated commit of a
    /// detached HEAD.
    pub fn branch(&self) -> String {
        let b = self.git_string(&["symbolic-ref", "--short", "-q", "HEAD"]);
        if !b.is_empty() {
            return b;
        }
        self.git_string(&["rev-parse", "--short", "HEAD"])
    }

    /// A value of git's configuration, `""` when unset.
    pub fn config_value(&self, key: &str) -> String {
        self.git_string(&["config", "--get", key])
    }

    /// Whether the repository has a commit.
    pub fn has_head(&self) -> bool {
        self.git(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok()
    }

    /// The full hash of a revision.
    pub fn resolve(&self, rev: &str) -> Result<String> {
        let spec = format!("{rev}^{{commit}}");
        self.git(&["rev-parse", "--verify", "-q", &spec])
            .map(|o| trim_string(&o))
            .map_err(|_| Error::Message(format!("unknown revision {rev:?}")))
    }

    /// The changes of the work tree and the index against HEAD, untracked
    /// files included.
    pub fn working_tree(&self, opts: Options) -> Result<Vec<File>> {
        let base = if self.has_head() { "HEAD" } else { EMPTY_TREE };
        let mut args = diff_args(opts);
        args.extend([base, "--"]);
        let out = self.git(&args)?;
        let mut files = diff::parse(&out);
        files.extend(self.untracked_files()?);
        self.mark_conflicts(&mut files);
        self.mark_generated(&mut files, None);
        sort_files(&mut files);
        Ok(files)
    }

    /// The changes of the untracked files, with directories of build output
    /// and dependencies as one row each.
    fn untracked_files(&self) -> Result<Vec<File>> {
        let paths = self.untracked()?;
        let mut files = Vec::new();
        for (i, path) in paths.iter().enumerate() {
            if i == MAX_UNTRACKED {
                let more = paths.len() - MAX_UNTRACKED;
                files.push(File {
                    path: format!("Untracked files not shown ({more} more)"),
                    status: Status::Untracked,
                    directory: true,
                    note: format!("{more} untracked files are not shown."),
                    fingerprint: format!("more:{more}"),
                    ..File::default()
                });
                break;
            }
            files.push(self.untracked_file(path));
        }
        let mut globs = Vec::new();
        for g in GENERATED_DIRS {
            globs.push(g.to_string());
            globs.push(format!(":(glob)**/{g}/"));
        }
        let mut args = vec![
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
            "--directory",
            "-z",
            "--",
        ];
        args.extend(globs.iter().map(String::as_str));
        if let Ok(out) = self.git(&args) {
            let mut seen = HashSet::new();
            for p in out.split(|&b| b == 0) {
                let p = String::from_utf8_lossy(p);
                let dir = p.strip_suffix('/').unwrap_or(&p).to_string();
                if dir.is_empty() || !seen.insert(dir.clone()) {
                    continue;
                }
                files.push(File {
                    path: dir.clone(),
                    old_path: dir.clone(),
                    status: Status::Untracked,
                    directory: true,
                    note: "Untracked directory is collapsed by default.".into(),
                    fingerprint: format!("dir:{dir}"),
                    ..File::default()
                });
            }
        }
        for f in &mut files {
            f.old_path = f.path.clone();
        }
        Ok(files)
    }

    /// Makes the change of a new file from its content.
    fn untracked_file(&self, path: &str) -> File {
        let bare = || File {
            path: path.to_string(),
            old_path: path.to_string(),
            status: Status::Untracked,
            ..File::default()
        };
        let full = self.full_path(path);
        let Ok(md) = std::fs::symlink_metadata(&full) else {
            return bare();
        };
        if md.file_type().is_symlink() {
            let target = std::fs::read_link(&full)
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_default();
            return diff::new_file_from_content(path, target.as_bytes(), Status::Untracked);
        }
        if md.len() > MAX_PATCH {
            let mut f = bare();
            f.too_large = true;
            f.fingerprint = format!("{path}:{}:{}", md.len(), mod_time_nanos(&md));
            return f;
        }
        match std::fs::read(&full) {
            Ok(data) => diff::new_file_from_content(path, &data, Status::Untracked),
            Err(_) => bare(),
        }
    }

    /// The paths of the untracked files that are not ignored, outside
    /// directories of build output and dependencies, sorted.
    pub fn untracked(&self) -> Result<Vec<String>> {
        let mut excludes = Vec::new();
        for g in GENERATED_DIRS {
            excludes.push(format!(":(exclude){g}/**"));
            excludes.push(format!(":(exclude)**/{g}/**"));
        }
        let mut args = vec![
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ];
        args.extend(excludes.iter().map(String::as_str));
        let out = self.git(&args)?;
        let mut paths: Vec<String> = out
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect();
        paths.sort();
        Ok(paths)
    }

    /// Marks the files with merge conflicts.
    pub fn mark_conflicts(&self, files: &mut [File]) {
        let Ok(out) = self.git(&["diff", "--name-only", "--diff-filter=U", "-z"]) else {
            return;
        };
        if out.is_empty() {
            return;
        }
        let conflicted: HashSet<String> = out
            .split(|&b| b == 0)
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect();
        for f in files {
            if conflicted.contains(&f.path) {
                f.status = Status::Conflict;
            }
        }
    }

    /// A value that changes when the work tree's changes do, cheaply, to
    /// notice edits made outside the app.
    pub fn status_signature(&self) -> String {
        let out = match self.git(&[
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "-uall",
        ]) {
            Ok(o) => o,
            Err(e) => return format!("error:{e}"),
        };
        let mut b = String::from_utf8_lossy(&out).into_owned();
        // A file changed twice has the same status: its size and time tell.
        let fields: Vec<&[u8]> = out.split(|&c| c == 0).collect();
        let mut i = 0;
        while i < fields.len() {
            let rec = String::from_utf8_lossy(fields[i]);
            let path = if let Some(p) = rec.strip_prefix("? ") {
                p.to_string()
            } else if rec.starts_with("1 ") {
                nth_field(&rec, 8).to_string()
            } else if rec.starts_with("2 ") {
                i += 1; // the original path
                nth_field(&rec, 9).to_string()
            } else if rec.starts_with("u ") {
                nth_field(&rec, 10).to_string()
            } else {
                String::new()
            };
            i += 1;
            if path.is_empty() {
                continue;
            }
            match std::fs::symlink_metadata(self.full_path(&path)) {
                Ok(md) => b.push_str(&format!(
                    "|{path}:{}:{}:{}",
                    file_mode(&md),
                    md.len(),
                    mod_time_nanos(&md)
                )),
                Err(_) => b.push_str(&format!("|{path}:missing")),
            }
        }
        b
    }

    /// `n` commits of the history of HEAD, after skipping `skip`.
    pub fn log(&self, skip: usize, n: usize) -> Result<Vec<Commit>> {
        if !self.has_head() {
            return Ok(Vec::new());
        }
        let format = format!("--format={LOG_FORMAT}");
        let skip = format!("--skip={skip}");
        let n = n.to_string();
        let out = self.git(&["log", "--no-color", &format, &skip, "-n", &n, "HEAD", "--"])?;
        Ok(parse_log(&out))
    }

    /// A commit, by any revision naming it.
    pub fn commit_info(&self, rev: &str) -> Result<Commit> {
        let format = format!("--format={LOG_FORMAT}");
        let out = self.git(&["log", "--no-color", &format, "-n", "1", rev, "--"])?;
        parse_log(&out)
            .into_iter()
            .next()
            .ok_or_else(|| Error::Message(format!("unknown revision {rev:?}")))
    }

    /// The changes of a commit against its first parent.
    pub fn commit_diff(&self, c: &Commit, opts: Options) -> Result<Vec<File>> {
        let base = c.parents.first().map(String::as_str).unwrap_or(EMPTY_TREE);
        let mut args = diff_args(opts);
        args.extend([base, &c.hash, "--"]);
        let out = self.git(&args)?;
        let mut files = diff::parse(&out);
        self.mark_generated(&mut files, Some(&c.hash));
        sort_files(&mut files);
        Ok(files)
    }

    /// The merge base of two revisions.
    pub fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        self.git(&["merge-base", a, b]).map(|o| trim_string(&o))
    }

    /// The changes of the work tree, committed or not, since it branched
    /// from `base`, and the merge base they are against.
    pub fn compare(&self, base: &str, opts: Options) -> Result<(Vec<File>, String)> {
        let mb = self
            .merge_base(base, "HEAD")
            .map_err(|_| Error::Message(format!("no common ancestor of {base} and HEAD")))?;
        let mut args = diff_args(opts);
        args.extend([mb.as_str(), "--"]);
        let out = self.git(&args)?;
        let mut files = diff::parse(&out);
        files.extend(self.untracked_files()?);
        self.mark_generated(&mut files, None);
        sort_files(&mut files);
        Ok((files, mb))
    }

    /// The changes of `head` since it branched from `base`, as a pull request
    /// shows them: against their merge base, which it returns too.
    pub fn range(&self, base: &str, head: &str, opts: Options) -> Result<(Vec<File>, String)> {
        let mb = self.merge_base(base, head).map_err(|_| {
            Error::Message(format!(
                "no common ancestor of {} and {}",
                short_rev(base),
                short_rev(head)
            ))
        })?;
        let mut args = diff_args(opts);
        args.extend([mb.as_str(), head, "--"]);
        let out = self.git(&args)?;
        let mut files = diff::parse(&out);
        self.mark_generated(&mut files, Some(head));
        sort_files(&mut files);
        Ok((files, mb))
    }

    /// The subjects of the commits of `base..head`, oldest first, each as
    /// `"<short hash> <subject>"`.
    pub fn subjects(&self, base: &str, head: &str, limit: usize) -> Vec<String> {
        let limit = limit.to_string();
        let range = format!("{base}..{head}");
        let Ok(out) = self.git(&[
            "log",
            "--no-color",
            "--reverse",
            "--format=%h %s",
            "-n",
            &limit,
            &range,
            "--",
        ]) else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out)
            .trim()
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Commits the files at `paths`, staged from the work tree as they are,
    /// with a message, and returns the new commit's short hash. Other changes
    /// staged in the index stay staged and out of the commit.
    pub fn commit_changes(&self, message: &str, paths: &[impl AsRef<str>]) -> Result<String> {
        if message.trim().is_empty() {
            return Err(Error::Message("the commit message is empty".into()));
        }
        if paths.is_empty() {
            return Err(Error::Message("no files to commit".into()));
        }
        let pathspecs = || {
            let mut b = Vec::new();
            for p in paths {
                b.extend_from_slice(p.as_ref().as_bytes());
                b.push(0);
            }
            b
        };
        // Stage the files chosen, deleted ones included.
        self.run_with_stdin(
            Some(pathspecs()),
            &[
                "--literal-pathspecs",
                "add",
                "-A",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
        )?;
        let msg = TempFile::new("diffr-message-")?;
        std::fs::write(&msg.0, message)?;
        let msg_path = msg.0.to_string_lossy().into_owned();
        self.run_with_stdin(
            Some(pathspecs()),
            &[
                "--literal-pathspecs",
                "commit",
                "-F",
                &msg_path,
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
        )?;
        Ok(self.git_string(&["rev-parse", "--short", "HEAD"]))
    }

    /// Starts `git cat-file --batch` for the repository, to read files of
    /// revisions.
    pub fn new_contents(&self) -> Result<Contents> {
        let mut child = git_command(&self.root, &["cat-file", "--batch"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Message("cat-file: no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Message("cat-file: no stdout".into()))?;
        Ok(Contents {
            inner: Mutex::new(ContentsInner {
                child,
                stdin: Some(stdin),
                stdout: BufReader::with_capacity(64 * 1024, stdout),
            }),
        })
    }

    /// Reads a file of the work tree, `None` when there is none.
    pub fn read_work_tree(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.full_path(path)).ok()
    }

    /// The repository's remotes, in the order of its configuration.
    pub fn remotes(&self) -> Vec<Remote> {
        let Ok(out) = self.git(&["config", "--get-regexp", r"^remote\..*\.url$"]) else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out)
            .trim()
            .split('\n')
            .filter_map(|line| {
                let (key, url) = line.split_once(' ')?;
                let name = key.strip_prefix("remote.").unwrap_or(key);
                let name = name.strip_suffix(".url").unwrap_or(name);
                Some(Remote {
                    name: name.to_string(),
                    url: url.trim().to_string(),
                })
            })
            .collect()
    }

    /// Whether the repository holds a commit.
    pub fn has_commit(&self, rev: &str) -> bool {
        let spec = format!("{rev}^{{commit}}");
        self.git(&["cat-file", "-e", &spec]).is_ok()
    }

    /// Fetches refs of a remote, or of a URL, without their tags and without
    /// asking for credentials: what it fetches is kept by the refs alone, or
    /// until git collects it. Gives up after three minutes.
    pub fn fetch(&self, remote: &str, refspecs: &[&str]) -> Result<()> {
        self.fetch_cancellable(remote, refspecs, &AtomicBool::new(false))
    }

    /// [`Repo::fetch`], which stops early once `cancel` is set.
    pub fn fetch_cancellable(
        &self,
        remote: &str,
        refspecs: &[&str],
        cancel: &AtomicBool,
    ) -> Result<()> {
        let mut args = vec![
            "-c",
            "credential.interactive=never",
            "fetch",
            "--no-tags",
            "--quiet",
            "--no-recurse-submodules",
            remote,
        ];
        args.extend_from_slice(refspecs);
        let mut child = git_command(&self.root, &args)
            // SSH must fail rather than wait for a passphrase no one can type.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_SSH_COMMAND", ssh_command())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| command_error(&args, b"", e.to_string()))?;
        let mut stderr_pipe = child.stderr.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = stderr_pipe.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        });
        let deadline = Instant::now() + Duration::from_secs(180);
        let status = loop {
            if let Some(st) = child.try_wait()? {
                break Ok(st);
            }
            if cancel.load(Ordering::Relaxed) {
                break Err("canceled".to_string());
            }
            if Instant::now() >= deadline {
                break Err("timed out".to_string());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let status = match status {
            Ok(st) => st,
            Err(why) => {
                let _ = child.kill();
                let _ = child.wait();
                let stderr = reader.join().unwrap_or_default();
                return Err(command_error(&args, &stderr, why));
            }
        };
        let stderr = reader.join().unwrap_or_default();
        if !status.success() {
            return Err(command_error(&args, &stderr, status.to_string()));
        }
        Ok(())
    }
}

/// What follows the nth space of a record.
fn nth_field(rec: &str, n: usize) -> &str {
    let mut rec = rec;
    for _ in 0..n {
        match rec.find(' ') {
            Some(i) => rec = &rec[i + 1..],
            None => return "",
        }
    }
    rec
}

/// The user's ssh, told never to ask anything.
fn ssh_command() -> String {
    let ssh = std::env::var("GIT_SSH_COMMAND").unwrap_or_default();
    let ssh = if ssh.is_empty() {
        "ssh".to_string()
    } else {
        ssh
    };
    format!("{ssh} -o BatchMode=yes")
}

fn short_rev(rev: &str) -> &str {
    rev.get(..7).unwrap_or(rev)
}

/// A file in the temporary directory, removed when dropped.
struct TempFile(PathBuf);

impl TempFile {
    fn new(prefix: &str) -> std::io::Result<TempFile> {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for _ in 0..100 {
            let name = format!(
                "{prefix}{}-{nanos}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(TempFile(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other("cannot create a temporary file"))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A commit of the history.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// The author time, in seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
    pub body: String,
    /// The refs pointing at the commit, as `%D` prints them.
    pub refs: String,
    pub additions: usize,
    pub deletions: usize,
    pub files: usize,
}

impl Commit {
    /// The author time as a [`SystemTime`].
    pub fn system_time(&self) -> SystemTime {
        if self.time >= 0 {
            UNIX_EPOCH + Duration::from_secs(self.time as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(self.time.unsigned_abs())
        }
    }
}

const LOG_FORMAT: &str = "%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1f%b%x1e";

fn parse_log(out: &[u8]) -> Vec<Commit> {
    let mut commits = Vec::new();
    for rec in out.split(|&b| b == 0x1e) {
        let start = rec.iter().position(|&b| b != b'\n').unwrap_or(rec.len());
        let rec = &rec[start..];
        if rec.is_empty() {
            continue;
        }
        let rec = String::from_utf8_lossy(rec);
        let f: Vec<&str> = rec.splitn(9, '\x1f').collect();
        if f.len() < 9 {
            continue;
        }
        commits.push(Commit {
            hash: f[0].into(),
            short: f[1].into(),
            parents: f[2].split_whitespace().map(str::to_string).collect(),
            author: f[3].into(),
            email: f[4].into(),
            time: f[5].parse().unwrap_or(0),
            refs: f[6].into(),
            subject: f[7].into(),
            body: f[8].trim().into(),
            ..Commit::default()
        });
    }
    commits
}

/// A remote of the repository.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

/// Reads files of revisions in one `git cat-file --batch` process. Safe to
/// share between threads: reads are serialised. The process stops when the
/// value is closed or dropped.
#[derive(Debug)]
pub struct Contents {
    inner: Mutex<ContentsInner>,
}

#[derive(Debug)]
struct ContentsInner {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Contents {
    /// The content of `rev:path` (`rev` empty for the index, as `:path`),
    /// `None` when there is none.
    pub fn read(&self, rev: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if path.contains('\n') {
            return Ok(None);
        }
        let ContentsInner { stdin, stdout, .. } = &mut *inner;
        let Some(stdin) = stdin.as_mut() else {
            return Err(Error::Message("cat-file: closed".into()));
        };
        stdin.write_all(format!("{rev}:{path}\n").as_bytes())?;
        stdin.flush()?;
        let mut header = Vec::new();
        stdout.read_until(b'\n', &mut header)?;
        if !header.ends_with(b"\n") {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        let header = String::from_utf8_lossy(&header).into_owned();
        let fields: Vec<&str> = header.split_whitespace().collect();
        if fields.len() < 3 || fields[fields.len() - 1] == "missing" {
            return Ok(None);
        }
        let size: usize = fields[2]
            .parse()
            .map_err(|_| Error::Message(format!("cat-file: {header:?}")))?;
        let mut data = vec![0; size + 1]; // and the newline after it
        stdout.read_exact(&mut data)?;
        data.truncate(size);
        Ok(Some(data))
    }

    /// Stops git cat-file.
    pub fn close(self) -> Result<()> {
        let mut inner = self.inner.into_inner().unwrap_or_else(|e| e.into_inner());
        inner.stdin.take();
        inner.child.wait()?;
        Ok(())
    }
}

impl Drop for ContentsInner {
    fn drop(&mut self) {
        if self.stdin.take().is_some() {
            let _ = self.child.wait();
        }
    }
}

const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    send_sync::<Repo>();
    send_sync::<Contents>();
};

#[cfg(test)]
mod tests;
