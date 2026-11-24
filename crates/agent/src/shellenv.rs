//! Finds the programs of the user's shell: an app opened from Finder, the
//! Dock or a desktop launcher has the system's PATH alone, without the
//! directories of Homebrew, mise or npm where `gh` and the coding agents
//! are.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use crate::proc;

/// Set (to anything) to keep the app's own PATH alone, as tests do.
pub const NO_SHELL_PATH_ENV: &str = "DIFFR_NO_SHELL_PATH";

static PATH: OnceLock<String> = OnceLock::new();

/// The app's own PATH, the user's login shell's, and the usual directories
/// of package managers after them. With [`NO_SHELL_PATH_ENV`] set, the
/// app's own alone. The shell is asked once, on first use: call this off
/// the UI thread the first time.
pub fn path() -> String {
    if std::env::var_os(NO_SHELL_PATH_ENV).is_some_and(|v| !v.is_empty()) {
        return std::env::var("PATH").unwrap_or_default();
    }
    PATH.get_or_init(resolve).clone()
}

fn resolve() -> String {
    let own = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&own).collect();
    let shell = shell_path();
    dirs.extend(std::env::split_paths(&shell));
    if !cfg!(windows) {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for d in [
                ".local/bin",
                ".local/share/mise/shims",
                ".bun/bin",
                ".opencode/bin",
                ".npm-global/bin",
                ".volta/bin",
                ".cargo/bin",
                "go/bin",
            ] {
                dirs.push(home.join(d));
            }
        }
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if d.as_os_str().is_empty() || out.contains(&d) {
            continue;
        }
        out.push(d);
    }
    std::env::join_paths(out)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| own.to_string_lossy().into_owned())
}

const START: &str = "__DIFFR_PATH__";
const END: &str = "__END__";

/// Asks the user's shell for its PATH, as an interactive login shell sets
/// it, "" when it does not tell within 5 seconds.
fn shell_path() -> String {
    if cfg!(windows) {
        return String::new();
    }
    let sh = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut cmd = Command::new(sh);
    // Markers keep out what the shell's configuration prints.
    cmd.args([
        "-i",
        "-l",
        "-c",
        &format!("printf \"{START}%s{END}\" \"$PATH\""),
    ])
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    proc::hide_console(&mut cmd);
    proc::own_group(&mut cmd);
    let out = match proc::output(&mut cmd, Duration::from_secs(5)) {
        Ok(o) => o.stdout,
        Err(_) => return String::new(),
    };
    parse_marked(&String::from_utf8_lossy(&out))
}

fn parse_marked(s: &str) -> String {
    let (Some(start), Some(end)) = (s.find(START), s.rfind(END)) else {
        return String::new();
    };
    if end < start + START.len() {
        return String::new();
    }
    s[start + START.len()..end].to_string()
}

/// Finds a program in [`path()`].
pub fn look_path(name: &str) -> Option<PathBuf> {
    let p = path();
    std::env::split_paths(&p).find_map(|dir| executable_in(&dir, name))
}

fn executable_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let p = dir.join(name);
    if cfg!(windows) {
        for ext in [".exe", ".cmd", ".bat"] {
            let mut q = p.clone().into_os_string();
            q.push(ext);
            let q = PathBuf::from(q);
            if q.is_file() {
                return Some(q);
            }
        }
        return None;
    }
    let meta = std::fs::metadata(&p).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
            return Some(p);
        }
        None
    }
    #[cfg(not(unix))]
    {
        meta.is_file().then_some(p)
    }
}

/// Gives a command [`path()`] as its PATH, for the programs it runs,
/// which run others in turn. The rest of the app's environment is
/// inherited.
pub fn apply(cmd: &mut Command) {
    cmd.env("PATH", path());
}

/// The app's environment with [`path()`] as its PATH.
pub fn environ() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::env::vars().filter(|(k, _)| k != "PATH").collect();
    out.push(("PATH".into(), path()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_output() {
        assert_eq!(
            parse_marked("motd\n__DIFFR_PATH__/a:/b__END__"),
            "/a:/b".to_string()
        );
        assert_eq!(parse_marked("nothing"), "");
    }
}
