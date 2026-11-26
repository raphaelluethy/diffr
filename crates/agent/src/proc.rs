//! Prepares and stops the commands the app runs, with the timeout and kill
//! helpers that Go's `exec.CommandContext` gives for free.

use std::io::{self, Read};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Keeps a console program from opening a console window, which only
/// Windows opens.
pub fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Starts a command in a process group of its own, which [`kill_group`]
/// kills whole: agents run tools in processes of their own.
pub fn own_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = cmd;
}

/// Kills a process started with [`own_group`] and every process of its
/// group; on Windows, the process and its tree.
pub fn kill_group(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with a negative pid signals a process group; it
        // touches no memory of ours.
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut cmd);
        let _ = cmd.status();
    }
}

/// Watches a child: kills its group once `cancel` is set or `timeout`
/// passes, until dropped.
pub(crate) struct Watchdog {
    done: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Watchdog {
    pub(crate) fn new(pid: u32, timeout: Duration, cancel: Option<Arc<AtomicBool>>) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let (d, f) = (done.clone(), fired.clone());
        let deadline = Instant::now() + timeout;
        let handle = thread::spawn(move || {
            while !d.load(Ordering::SeqCst) {
                let cancelled = cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst));
                if cancelled || Instant::now() >= deadline {
                    f.store(true, Ordering::SeqCst);
                    kill_group(pid);
                    return;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                let wait = if cancel.is_some() { remaining.min(Duration::from_millis(50)) } else { remaining };
                thread::park_timeout(wait);
            }
        });
        Watchdog {
            done,
            fired,
            handle: Some(handle),
        }
    }

    /// Whether it killed the child.
    pub(crate) fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }

    pub(crate) fn stop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            h.thread().unpark();
            let _ = h.join();
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Runs a command to its end and collects its output, killing it and its
/// process group after `timeout`. Stdin is
/// null unless the command set it.
pub fn output(cmd: &mut Command, timeout: Duration) -> io::Result<Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    own_group(cmd);
    let mut child = cmd.spawn()?;
    let mut watchdog = Watchdog::new(child.id(), timeout, None);
    let (stdout, stderr) = drain(&mut child);
    // Stopped before the child is reaped, so its pid cannot be another's.
    watchdog.stop();
    let status = child.wait()?;
    if watchdog.fired() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("timed out after {timeout:?}"),
        ));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn drain(child: &mut Child) -> (Vec<u8>, Vec<u8>) {
    let err = child.stderr.take().map(|mut e| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = e.read_to_end(&mut buf);
            buf
        })
    });
    let mut out = Vec::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_end(&mut out);
    }
    let err = err.and_then(|h| h.join().ok()).unwrap_or_default();
    (out, err)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn output_times_out() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 5; echo late"]);
        own_group(&mut cmd);
        let start = Instant::now();
        let err = output(&mut cmd, Duration::from_millis(200)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn output_collects() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "echo out; echo err >&2"]);
        let o = output(&mut cmd, Duration::from_secs(5)).unwrap();
        assert_eq!(o.stdout, b"out\n");
        assert_eq!(o.stderr, b"err\n");
    }
}
