//! Runs an agent on a change: its command line per agent, and the parsing
//! of each agent's stream of JSON lines.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;

use crate::labs::{self, LabTool};
use crate::models::{opencode_major, opencode_model};
use crate::proc::{self, Watchdog};
use crate::{Agent, Analysis, Error, INLINE_LIMIT, Request, diffs, fit, nullable, parse};
use crate::{review_instructions, schema_for, shellenv, system_prompt, user_prompt_with_patch};

/// Bounds a run: the agents cap their own turns, and the user can stop
/// one sooner.
pub const TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// Stops a run: clone it, hand one to [`run`], and [`cancel`](Self::cancel)
/// the other from any thread. The agent's whole process group is killed,
/// before `cancel` returns, so an app quitting right after leaves no agent
/// behind.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<CancelInner>);

#[derive(Debug, Default)]
struct CancelInner {
    cancelled: Arc<AtomicBool>,
    /// The process group running, while it can be signalled: set once the
    /// agent starts, cleared before it is reaped (so its pid is never
    /// another process's).
    pid: Mutex<Option<u32>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.0.pid.lock().unwrap_or_else(|e| e.into_inner()) {
            proc::kill_group(pid);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Notes the group started; kills it at once if the run was cancelled
    /// meanwhile.
    fn started(&self, pid: u32) {
        let mut g = self.0.pid.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some(pid);
        if self.is_cancelled() {
            proc::kill_group(pid);
        }
    }

    /// Notes the group is about to be reaped.
    fn finished(&self) {
        *self.0.pid.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Asks an agent to review a change, reporting what it does as it goes
/// ("Reading main.go", "Thinking…"), and returns its analysis. It blocks
/// until the agent answers, fails, is cancelled ([`Error::Canceled`]) or
/// runs over [`TIMEOUT`].
pub fn run(
    agent: Agent,
    req: &Request,
    cancel: &CancelToken,
    mut progress: impl FnMut(&str),
) -> Result<Analysis, Error> {
    if cancel.is_cancelled() {
        return Err(Error::Canceled);
    }
    // Diffs too large for the prompt go to a file the agent reads, in the
    // repository's git directory, which the agents may read.
    let all_diffs = diffs(&req.files);
    let inline = all_diffs.len() <= INLINE_LIMIT;
    let mut _patch_file = None;
    let mut patch_path = String::new();
    if !inline {
        let git_dir = req.dir.join(".git");
        let dir = if git_dir.is_dir() {
            git_dir
        } else {
            std::env::temp_dir()
        };
        let mut f = tempfile::Builder::new()
            .prefix("diffr-review-")
            .suffix(".patch")
            .tempfile_in(dir)?;
        f.write_all(all_diffs.as_bytes())?;
        f.flush()?;
        patch_path = f.path().to_string_lossy().into_owned();
        _patch_file = Some(f);
    }
    let user = user_prompt_with_patch(req, inline.then_some(all_diffs.as_str()));
    drop(all_diffs);
    let (system, schema) = if takes_schema(agent) {
        (review_instructions(req, &patch_path, false), schema_for(&req.labs, req.suggest_commits))
    } else {
        (system_prompt(req, &patch_path, true), String::new())
    };
    let job = Job {
        dir: req.dir.clone(),
        model: req.model.clone(),
        system,
        user,
        schema,
        // The commands the labs let the agent run, of the diffr executable.
        tools: labs::enabled(&req.labs)
            .iter()
            .flat_map(|l| l.tools().iter().copied())
            .collect(),
        tool_bin: labs::tool_bin(&req.tool_bin).to_string_lossy().into_owned(),
        extra_dir: Path::new(&patch_path)
            .parent()
            .filter(|_| !patch_path.is_empty())
            .map(|p| p.to_string_lossy().into_owned()),
        fast: req.fast,
    };
    let answer = exec(agent, job, cancel, &mut progress)?;
    progress("Organizing the review…");
    let result = match answer.structured {
        Some(v) => serde_json::from_value::<Analysis>(v)
            .map_err(|e| Error::Failed(format!("the agent's answer is not an analysis: {e}")))
            .and_then(|a| fit(a, &req.files)),
        None => parse(&answer.text, &req.files),
    };
    let mut an = result.map_err(|e| Error::Failed(format!("{}: {e}", agent.label)))?;
    an.agent = agent.name.to_string();
    an.model = answer.model;
    an.labs = labs::enabled_ids(&req.labs);
    an.generated_at = Some(OffsetDateTime::now_utc());
    Ok(an)
}

/// Whether an agent takes a JSON Schema for its answer; the others are
/// asked for a fenced block.
pub(crate) fn takes_schema(agent: Agent) -> bool {
    agent.name == "claude" || agent.name == "codex"
}

/// A read-only task for an agent: its instructions, the change, and the
/// shape of its answer.
pub(crate) struct Job {
    /// Where the agent runs: the repository.
    pub dir: PathBuf,
    /// The model, "" for the agent's default.
    pub model: String,
    pub system: String,
    pub user: String,
    /// The JSON Schema of the answer.
    pub schema: String,
    /// The labs' commands the agent may run, of `tool_bin`.
    pub tools: Vec<LabTool>,
    pub tool_bin: String,
    /// A directory outside the repository the agent may read too.
    pub extra_dir: Option<String>,
    /// Codex's fast mode: its `fast` service tier.
    pub fast: bool,
}

/// What an agent answered: the object it validated against the schema
/// itself, else its last text.
pub(crate) struct Answer {
    pub structured: Option<Value>,
    pub text: String,
    /// The model that answered, when the agent says.
    pub model: String,
}

/// Runs an agent on a job, read-only, reporting its steps, until it
/// answers, fails, is cancelled or runs over [`TIMEOUT`].
pub(crate) fn exec(
    agent: Agent,
    job: Job,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(&str),
) -> Result<Answer, Error> {
    let Some(bin) = shellenv::look_path(agent.name) else {
        return Err(Error::Failed(format!(
            "{} is not installed: {} was not found in your PATH",
            agent.label, agent.name
        )));
    };
    let mut r = Runner {
        agent: Some(agent),
        model: job.model.clone(),
        ..Default::default()
    };
    let (system, user, schema) = (job.system, job.user, job.schema);
    let tools = &job.tools;
    let tool_bin = job.tool_bin.as_str();
    let mut stdin_system = None;
    let mut args: Vec<String>;
    let mut env: Vec<(String, String)> = Vec::new();
    // Temporary files that live as long as the run.
    let mut _keep = Vec::new();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match agent.name {
        "claude" => {
            args = s(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--system-prompt",
            ]);
            args.push(system);
            args.push("--json-schema".into());
            args.push(schema);
            args.extend(s(&[
                "--permission-mode",
                "dontAsk",
                "--no-session-persistence",
                "--strict-mcp-config",
                "--allowedTools",
                "Read",
                "Grep",
                "Glob",
                "Bash(git show:*)",
                "Bash(git log:*)",
                "Bash(git diff:*)",
            ]));
            args.extend(claude_tool_rules(tool_bin, tools));
            args.extend(s(&[
                "--disallowedTools",
                "Edit",
                "Write",
                "NotebookEdit",
                "WebFetch",
                "WebSearch",
                "Task",
            ]));
            if let Some(dir) = &job.extra_dir {
                args.push("--add-dir".into());
                args.push(dir.clone());
            }
            if !job.model.is_empty() {
                args.push("--model".into());
                args.push(job.model.clone());
            }
        }
        "codex" => {
            let mut schema_file = tempfile::Builder::new()
                .prefix("diffr-schema-")
                .suffix(".json")
                .tempfile()?;
            // The labs' commands need nothing more: the read-only sandbox
            // lets codex run any command, and keeps it from writing
            // anything, which is as precise as codex can be restricted.
            schema_file.write_all(schema.as_bytes())?;
            schema_file.flush()?;
            let last = tempfile::Builder::new()
                .prefix("diffr-answer-")
                .suffix(".txt")
                .tempfile()?;
            r.last_file = Some(last.path().to_path_buf());
            args = s(&[
                "exec",
                "--json",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--ephemeral",
                "--color",
                "never",
                "--output-schema",
            ]);
            args.push(schema_file.path().to_string_lossy().into_owned());
            args.push("-o".into());
            args.push(last.path().to_string_lossy().into_owned());
            args.push("-C".into());
            args.push(job.dir.to_string_lossy().into_owned());
            if !job.model.is_empty() {
                args.push("-m".into());
                args.push(job.model.clone());
            }
            if job.fast {
                args.push("-c".into());
                args.push("service_tier=\"fast\"".into());
            }
            args.push("-".into());
            _keep.push(schema_file);
            _keep.push(last);
            stdin_system = Some(system);
        }
        "opencode" => {
            args = s(&["run", "--format", "json"]);
            let v2 = opencode_major(&bin) >= 2;
            if v2 {
                // OpenCode 2 runs on its background service, which a config
                // in the environment does not reach, and would keep if it
                // started it: its explore agent may only read and search.
                // It may run any shell command (its prompt tells it to run
                // read-only ones), so the labs' commands need nothing more,
                // and cannot be singled out.
                args.extend(s(&["--agent", "explore"]));
            } else {
                // The default agent with the tools that change anything
                // denied: the plan agent will not read files.
                env.push((
                    "OPENCODE_CONFIG_CONTENT".into(),
                    opencode_config(tool_bin, tools),
                ));
            }
            stdin_system = Some(system);
            let model = opencode_model(&bin, &job.dir, &job.model, v2);
            if !model.is_empty() {
                args.push("--model".into());
                args.push(model.clone());
                r.model = model;
            }
        }
        other => return Err(Error::Failed(format!("unknown agent {other}"))),
    }

    if cancel.is_cancelled() {
        return Err(Error::Canceled);
    }
    let mut cmd = Command::new(&bin);
    cmd.args(&args)
        .current_dir(&job.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    shellenv::apply(&mut cmd);
    cmd.envs(env);
    proc::hide_console(&mut cmd);
    proc::own_group(&mut cmd);
    progress(&format!("Starting {}…", agent.label));
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Failed(format!("could not start {}: {e}", agent.label)))?;
    cancel.started(child.id());
    let mut watchdog = Watchdog::new(child.id(), TIMEOUT, Some(cancel.0.cancelled.clone()));

    let stdin_thread = child.stdin.take().map(|mut w| {
        thread::spawn(move || {
            if let Some(system) = stdin_system {
                let _ = w.write_all(system.as_bytes());
                let _ = w.write_all(b"\n\n");
            }
            let _ = w.write_all(user.as_bytes());
        })
    });
    let stderr_thread = child.stderr.take().map(|mut e| {
        thread::spawn(move || {
            let mut tail = Tail::default();
            let mut buf = [0u8; 8192];
            while let Ok(n) = e.read(&mut buf) {
                if n == 0 {
                    break;
                }
                tail.write(&buf[..n]);
            }
            tail.string()
        })
    });
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => r.line(&line, progress),
            }
        }
    }
    if let Some(t) = stdin_thread {
        let _ = t.join();
    }
    let stderr = stderr_thread
        .and_then(|t| t.join().ok())
        .unwrap_or_default();
    // Stopped before the child is reaped, so its pid cannot be another's.
    watchdog.stop();
    cancel.finished();
    let wait = child.wait();
    // Stopped by `cancel` itself or by the watchdog seeing it.
    if cancel.is_cancelled() {
        return Err(Error::Canceled);
    }
    if watchdog.fired() {
        return Err(Error::Failed(format!(
            "{} took over {} minutes",
            agent.label,
            TIMEOUT.as_secs() / 60
        )));
    }
    if let Some(p) = &r.last_file
        && let Ok(data) = std::fs::read_to_string(p)
        && !data.trim().is_empty()
    {
        r.final_text = data;
    }
    if r.structured.is_none() && r.final_text.trim().is_empty() {
        let mut msg = r.err.clone();
        if msg.is_empty() {
            msg = last_line(&stderr);
        }
        if msg.is_empty() {
            match &wait {
                Ok(st) if !st.success() => msg = st.to_string(),
                Err(e) => msg = e.to_string(),
                _ => {}
            }
        }
        if msg.is_empty() {
            msg = "it answered nothing".into();
        }
        return Err(Error::Failed(format!("{} failed: {msg}", agent.label)));
    }
    Ok(Answer {
        structured: r.structured.take(),
        text: std::mem::take(&mut r.final_text),
        model: r.model,
    })
}

/// The rules of Claude Code's `--allowedTools` that let it run the labs'
/// commands, and nothing else of the shell: for the command and its flag
/// form, one for the executable as it is, and one for it quoted, as the
/// prompt writes it.
pub(crate) fn claude_tool_rules(bin: &str, tools: &[LabTool]) -> Vec<String> {
    tools
        .iter()
        .flat_map(|t| [t.command, t.flag])
        .filter(|c| !c.is_empty())
        .flat_map(|c| {
            [
                format!("Bash({bin} {c}:*)"),
                format!("Bash(\"{bin}\" {c}:*)"),
            ]
        })
        .collect()
}

/// OpenCode 1's config: the default agent with edits, the web and the
/// shell denied, but for the labs' commands. Its rules apply in order,
/// the last that matches winning.
pub(crate) fn opencode_config(bin: &str, tools: &[LabTool]) -> String {
    if tools.is_empty() {
        return r#"{"permission":{"edit":"deny","bash":"deny","webfetch":"deny"}}"#.into();
    }
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let mut bash = String::from(r#"{"*":"deny""#);
    for c in tools
        .iter()
        .flat_map(|t| [t.command, t.flag])
        .filter(|c| !c.is_empty())
    {
        for pattern in [format!("{bin} {c}*"), format!("\"{bin}\" {c}*")] {
            bash.push_str(&format!(",{}:\"allow\"", q(&pattern)));
        }
    }
    bash.push('}');
    format!(r#"{{"permission":{{"edit":"deny","bash":{bash},"webfetch":"deny"}}}}"#)
}

/// Reads an agent's stream of JSON lines.
#[derive(Default)]
struct Runner {
    agent: Option<Agent>,
    model: String,
    // The answer: its text, or the object an agent validated itself.
    final_text: String,
    structured: Option<Value>,
    last_file: Option<std::path::PathBuf>,
    err: String,
    // The type of the event read last.
    last_event: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeEvent {
    #[serde(rename = "type", deserialize_with = "nullable")]
    kind: String,
    #[serde(deserialize_with = "nullable")]
    subtype: String,
    #[serde(deserialize_with = "nullable")]
    model: String,
    #[serde(deserialize_with = "nullable")]
    message: ClaudeMessage,
    #[serde(deserialize_with = "nullable")]
    is_error: bool,
    #[serde(deserialize_with = "nullable")]
    result: String,
    structured_output: Option<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeMessage {
    #[serde(deserialize_with = "nullable")]
    content: Vec<ClaudeContent>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeContent {
    #[serde(rename = "type", deserialize_with = "nullable")]
    kind: String,
    #[serde(deserialize_with = "nullable")]
    name: String,
    input: Value,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodexEvent {
    #[serde(rename = "type", deserialize_with = "nullable")]
    kind: String,
    #[serde(deserialize_with = "nullable")]
    message: String,
    #[serde(deserialize_with = "nullable")]
    error: MessageOnly,
    #[serde(deserialize_with = "nullable")]
    item: CodexItem,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MessageOnly {
    #[serde(deserialize_with = "nullable")]
    message: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodexItem {
    #[serde(rename = "type", deserialize_with = "nullable")]
    kind: String,
    #[serde(deserialize_with = "nullable")]
    text: String,
    #[serde(deserialize_with = "nullable")]
    command: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenCodeEvent {
    #[serde(rename = "type", deserialize_with = "nullable")]
    kind: String,
    #[serde(deserialize_with = "nullable")]
    error: OpenCodeError,
    #[serde(deserialize_with = "nullable")]
    part: OpenCodePart,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenCodeError {
    #[serde(deserialize_with = "nullable")]
    message: String,
    #[serde(deserialize_with = "nullable")]
    data: MessageOnly,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenCodePart {
    #[serde(deserialize_with = "nullable")]
    text: String,
    #[serde(deserialize_with = "nullable")]
    tool: String,
    #[serde(deserialize_with = "nullable")]
    state: OpenCodeState,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenCodeState {
    input: Value,
}

impl Runner {
    fn line(&mut self, data: &[u8], progress: &mut dyn FnMut(&str)) {
        let data = data.trim_ascii();
        if data.first() != Some(&b'{') {
            return;
        }
        match self.agent.map(|a| a.name) {
            Some("claude") => self.claude(data, progress),
            Some("codex") => self.codex(data, progress),
            Some("opencode") => self.opencode(data, progress),
            _ => {}
        }
    }

    fn claude(&mut self, data: &[u8], progress: &mut dyn FnMut(&str)) {
        let Ok(ev) = serde_json::from_slice::<ClaudeEvent>(data) else {
            return;
        };
        match ev.kind.as_str() {
            "system" => {
                if ev.subtype == "init" && !ev.model.is_empty() {
                    self.model = ev.model;
                }
            }
            "assistant" => {
                for c in &ev.message.content {
                    match c.kind.as_str() {
                        "tool_use" if c.name != "StructuredOutput" => {
                            progress(&tool_step(&c.name, &c.input));
                        }
                        "text" | "thinking" => progress("Thinking…"),
                        _ => {}
                    }
                }
            }
            "result" => {
                if ev.is_error {
                    self.err = ev.result;
                    return;
                }
                if let Some(v) = ev.structured_output.filter(|v| !v.is_null()) {
                    self.structured = Some(v);
                }
                self.final_text = ev.result;
            }
            _ => {}
        }
    }

    fn codex(&mut self, data: &[u8], progress: &mut dyn FnMut(&str)) {
        let Ok(ev) = serde_json::from_slice::<CodexEvent>(data) else {
            return;
        };
        match ev.kind.as_str() {
            "item.started" => {
                if ev.item.kind == "command_execution" {
                    progress(&command_step(&ev.item.command));
                }
            }
            "item.completed" => match ev.item.kind.as_str() {
                "agent_message" => self.final_text = ev.item.text,
                "reasoning" => progress("Thinking…"),
                _ => {}
            },
            "error" => self.err = error_message(&ev.message),
            "turn.failed" => self.err = error_message(&ev.error.message),
            _ => {}
        }
    }

    fn opencode(&mut self, data: &[u8], progress: &mut dyn FnMut(&str)) {
        let Ok(ev) = serde_json::from_slice::<OpenCodeEvent>(data) else {
            return;
        };
        // A step that starts again with nothing done is one that failed:
        // OpenCode retries it a while before it gives up.
        let retry = ev.kind == "step_start" && self.last_event == "step_start";
        self.last_event = ev.kind.clone();
        match ev.kind.as_str() {
            "step_start" => progress(if retry { "Retrying…" } else { "Thinking…" }),
            "reasoning" => progress("Thinking…"),
            "tool_use" => progress(&tool_step(&ev.part.tool, &ev.part.state.input)),
            "text" => {
                self.final_text = ev.part.text;
                progress("Thinking…");
            }
            "error" => {
                self.err = ev.error.message;
                if self.err.is_empty() {
                    self.err = ev.error.data.message;
                }
            }
            _ => {}
        }
    }
}

/// Describes a tool call: `Reading main.go`, `Searching for "foo"`.
pub fn tool_step(tool: &str, input: &Value) -> String {
    let s = |keys: &[&str]| -> String {
        keys.iter()
            .find_map(|k| {
                input
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
            })
            .unwrap_or_default()
            .to_string()
    };
    let mut path = s(&["file_path", "filePath", "path"]);
    if !path.is_empty() {
        path = base_name(&path);
    }
    match tool.to_lowercase().as_str() {
        "read" | "view" => {
            if path.is_empty() {
                "Reading files".into()
            } else {
                format!("Reading {path}")
            }
        }
        "grep" | "search" => {
            let p = s(&["pattern", "query"]);
            if p.is_empty() {
                "Searching".into()
            } else {
                format!("Searching for {p:?}")
            }
        }
        "glob" | "find" | "ls" | "list" => "Looking through the files".into(),
        "bash" | "shell" | "exec" => {
            let c = s(&["command", "cmd"]);
            if c.is_empty() {
                format!("Using {tool}")
            } else {
                command_step(&c)
            }
        }
        "check_diagram" => "Checking a diagram".into(),
        _ => format!("Using {tool}"),
    }
}

/// Describes a command: `Running git log -3`, or `Checking a diagram`.
fn command_step(c: &str) -> String {
    let first = first_line(c);
    if first.contains(" check-diagram") || first.contains(" --check-diagram") {
        return "Checking a diagram".into();
    }
    format!("Running {}", short_command(c))
}

fn base_name(p: &str) -> String {
    let t = p.trim_end_matches(['/', '\\']);
    if t.is_empty() {
        return if p.is_empty() { ".".into() } else { "/".into() };
    }
    t.rsplit(['/', '\\']).next().unwrap_or(t).to_string()
}

/// A command as its first line, without the shell codex wraps it in.
pub fn short_command(c: &str) -> String {
    let mut c = c;
    for p in [
        "/bin/zsh -lc ",
        "/bin/bash -lc ",
        "bash -lc ",
        "/bin/sh -c ",
    ] {
        c = c.strip_prefix(p).unwrap_or(c);
    }
    let c = first_line(c).trim_matches(['"', '\'']);
    if c.len() > 60 {
        let mut end = 60;
        while !c.is_char_boundary(end) {
            end -= 1;
        }
        return format!("{}…", &c[..end]);
    }
    c.to_string()
}

/// The message of an error that holds an API's answer as JSON, as codex's
/// do.
fn error_message(s: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        if let Some(m) = v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
        {
            return m.to_string();
        }
        if let Some(m) = v
            .get("message")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
        {
            return m.to_string();
        }
    }
    s.to_string()
}

pub(crate) fn first_line(s: &str) -> &str {
    let s = s.trim();
    match s.find(['\r', '\n']) {
        Some(i) => &s[..i],
        None => s,
    }
}

/// The last line of a command's output that says something.
pub(crate) fn last_line(s: &str) -> String {
    s.trim()
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("Warning:"))
        .unwrap_or_default()
        .to_string()
}

/// Keeps the end of what a command writes.
#[derive(Default)]
pub(crate) struct Tail(Vec<u8>);

impl Tail {
    pub(crate) fn write(&mut self, p: &[u8]) {
        self.0.extend_from_slice(p);
        if self.0.len() > 16 << 10 {
            let cut = self.0.len() - (8 << 10);
            self.0.drain(..cut);
        }
    }

    pub(crate) fn string(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Cancelling kills the agent's group before `cancel` returns, so
    /// quitting right after leaves nothing running.
    #[cfg(unix)]
    #[test]
    fn cancel_kills_at_once() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30"]);
        proc::own_group(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let token = CancelToken::new();
        token.started(child.id());
        token.cancel();
        // SIGKILL is delivered by the time kill(2) returns; the child only
        // needs reaping.
        let start = std::time::Instant::now();
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert!(start.elapsed() < Duration::from_secs(2));
        token.finished();
    }

    /// A run cancelled before its agent started kills it as it starts.
    #[cfg(unix)]
    #[test]
    fn cancel_before_start() {
        let token = CancelToken::new();
        token.cancel();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30"]);
        proc::own_group(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        token.started(child.id());
        assert!(!child.wait().unwrap().success());
        token.finished();
    }

    #[test]
    fn steps() {
        assert_eq!(
            tool_step("Read", &json!({"file_path": "/repo/src/greet.go"})),
            "Reading greet.go"
        );
        assert_eq!(
            tool_step("grep", &json!({"pattern": "Hello"})),
            "Searching for \"Hello\""
        );
        assert_eq!(tool_step("Glob", &Value::Null), "Looking through the files");
        assert_eq!(
            tool_step("bash", &json!({"command": "git log -3"})),
            "Running git log -3"
        );
        assert_eq!(tool_step("Task", &Value::Null), "Using Task");
        assert_eq!(
            short_command("/bin/zsh -lc \"git show abc:src/greet.go\""),
            "git show abc:src/greet.go"
        );
        assert_eq!(short_command(&"é".repeat(40)).chars().count(), 31);
    }

    #[test]
    fn lab_tools() {
        let tools: Vec<LabTool> = labs::lab(labs::DIAGRAMS).unwrap().tools().to_vec();
        assert_eq!(
            claude_tool_rules("/opt/diffr", &tools),
            [
                "Bash(/opt/diffr check-diagram:*)",
                "Bash(\"/opt/diffr\" check-diagram:*)",
                "Bash(/opt/diffr --check-diagram:*)",
                "Bash(\"/opt/diffr\" --check-diagram:*)"
            ]
        );
        assert!(claude_tool_rules("/opt/diffr", &[]).is_empty());
        assert_eq!(
            opencode_config("/opt/diffr", &[]),
            r#"{"permission":{"edit":"deny","bash":"deny","webfetch":"deny"}}"#
        );
        let c = opencode_config("/opt/my \"diffr\"", &tools);
        let v: Value = serde_json::from_str(&c).unwrap();
        assert_eq!(v["permission"]["edit"], "deny");
        assert_eq!(v["permission"]["bash"]["*"], "deny");
        assert_eq!(
            v["permission"]["bash"]["/opt/my \"diffr\" check-diagram*"],
            "allow"
        );
        assert_eq!(
            v["permission"]["bash"]["/opt/my \"diffr\" --check-diagram*"],
            "allow"
        );
        // Deny first, as the last rule that matches wins.
        assert!(c.find(r#""*":"deny""#).unwrap() < c.find("allow").unwrap());
        assert_eq!(
            tool_step(
                "Bash",
                &json!({"command": "\"/opt/diffr\" check-diagram <<'MERMAID'\nflowchart LR\nMERMAID"})
            ),
            "Checking a diagram"
        );
        assert_eq!(
            tool_step("check_diagram", &Value::Null),
            "Checking a diagram"
        );
        assert_eq!(
            tool_step(
                "Bash",
                &json!({"command": "/opt/diffr --check-diagram <<'EOF'\nflowchart LR\nEOF"})
            ),
            "Checking a diagram"
        );
    }

    #[test]
    fn messages() {
        assert_eq!(
            error_message(
                r#"{"type":"error","status":400,"error":{"message":"The model is not supported."}}"#
            ),
            "The model is not supported."
        );
        assert_eq!(error_message("plain"), "plain");
        assert_eq!(last_line("a\nb\nWarning: x\n\n"), "b");
        let mut t = Tail::default();
        t.write(&vec![b'a'; 20 << 10]);
        assert_eq!(t.0.len(), 8 << 10);
    }
}
