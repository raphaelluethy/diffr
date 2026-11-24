//! Opening files in the user's editor.

use std::path::Path;
use std::process::Command;

/// Opens a file at a line: with `$DIFFR_EDITOR` or the configured command
/// ({file}, {line} and {repo} replaced), else Zed, else VS Code, else the
/// system's opener.
pub fn open_editor(command: &str, repo: &str, file: &Path, line: u32) -> anyhow::Result<()> {
    let file_s = file.to_string_lossy().to_string();
    let env = std::env::var("DIFFR_EDITOR").unwrap_or_default();
    let cmd = if !env.trim().is_empty() { env } else { command.to_string() };
    if !cmd.trim().is_empty() {
        let has_file = cmd.contains("{file}");
        let line_s = line.max(1).to_string();
        let mut parts: Vec<String> = shell_words(&cmd)
            .into_iter()
            .map(|p| p.replace("{file}", &file_s).replace("{line}", &line_s).replace("{repo}", repo))
            .collect();
        if !has_file {
            parts.push(file_s.clone());
        }
        if parts.is_empty() {
            anyhow::bail!("empty editor command");
        }
        Command::new(&parts[0]).args(&parts[1..]).current_dir(repo).spawn()?;
        return Ok(());
    }
    // Zed: `zed`, or `zeditor` / `zedit` as some Linux packages name it.
    for zed in ["zed", "zeditor", "zedit"] {
        if which(zed) {
            let target = if line > 0 { format!("{file_s}:{line}") } else { file_s.clone() };
            Command::new(zed).arg(target).current_dir(repo).spawn()?;
            return Ok(());
        }
    }
    if cfg!(target_os = "macos") && Path::new("/Applications/Zed.app").exists() {
        let target = if line > 0 { format!("{file_s}:{line}") } else { file_s.clone() };
        Command::new("open").args(["-a", "Zed", &target]).spawn()?;
        return Ok(());
    }
    if which("code") {
        let target = if line > 0 { format!("{file_s}:{line}") } else { file_s.clone() };
        Command::new("code").arg("--goto").arg(target).current_dir(repo).spawn()?;
        return Ok(());
    }
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(opener).arg(&file_s).spawn()?;
    Ok(())
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Splits a command line as a shell would, for quotes and backslashes.
pub fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    in_word = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    out
}
