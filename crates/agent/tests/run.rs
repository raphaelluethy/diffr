//! Runs fake agents: shell scripts of the agents' names that record how
//! they ran and print what the agent's stream would. As they change the
//! environment (PATH), all cases run in one test, one after another.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use diffr_agent::{CancelToken, Error, File, Request, by_name, models, run};
use serde_json::{Value, json};

const ANSWER: &str = r#"{"overallSummary":"Greets louder.","groups":[
 {"key":"greeting","label":"Greeting","summary":"Shorter greeting.","critical":false,
  "filePaths":["src/greet.go","./src/greet_test.go","invented.go"],
  "fileNotes":[{"path":"src/greet.go","text":"Changes the public output.","critical":true},{"path":"nope.go","text":"x","critical":false}],
  "lineNotes":[{"path":"src/greet.go","side":"additions","line":2,"text":"Callers may match on Hello.","critical":false},{"path":"src/greet.go","side":"sideways","line":2,"text":"x","critical":false}]},
 {"key":"deps","label":"Lockfile","summary":"","critical":false,"filePaths":["pnpm-lock.yaml","src/greet.go"],"fileNotes":[],"lineNotes":[]}
]}"#;

/// An answer of the shape the prompt asks for now: summaries
/// per group, and the Diagrams lab's fields.
const GROUPED_ANSWER: &str = r#"{"overallSummary":"Greets louder.","order":[],
 "diagram":"```mermaid\nflowchart LR\n  greet:::changed --> out\n```",
 "groups":[
 {"key":"greeting","label":"Greeting","summary":"Shorter greeting.","critical":false,
  "filePaths":["src/greet.go","src/greet_test.go","pnpm-lock.yaml","logo.png"],
  "fileSummaries":[{"path":"src/greet.go","text":"Shortens the greeting."}],
  "fileNotes":[],"lineNotes":[],
  "diagram":"sequenceDiagram\n  A->>B: hi"}
]}"#;

fn files() -> Vec<File> {
    vec![
        File {
            path: "src/greet.go".into(),
            status: 'M',
            additions: 2,
            deletions: 1,
            patch:
                "@@ -1,3 +1,4 @@ func greet\n-\treturn \"Hello\"\n+\treturn \"Hi\"\n+\t// louder\n"
                    .into(),
            ..Default::default()
        },
        File {
            path: "src/greet_test.go".into(),
            status: 'A',
            additions: 5,
            patch: "@@ -0,0 +1,5 @@\n+package src\n".into(),
            ..Default::default()
        },
        File {
            path: "pnpm-lock.yaml".into(),
            status: 'M',
            additions: 900,
            deletions: 800,
            generated: true,
            patch: "@@ -1 +1 @@\n-a\n+b\n".into(),
            ..Default::default()
        },
        File {
            path: "logo.png".into(),
            status: 'A',
            binary: true,
            ..Default::default()
        },
    ]
}

const SCRIPT: &str = r#"#!/bin/sh
dir=$(dirname "$0")
# What the agent prints for another command, as its version.
if [ -f "$dir/cmd-$1" ]; then cat "$dir/cmd-$1"; exit 0; fi
printf '%s\n' "$@" > "$dir/args"
env > "$dir/env"
cat > "$dir/stdin"
last=""
while [ $# -gt 0 ]; do
  if [ "$1" = "-o" ]; then last="$2"; fi
  shift
done
if [ -n "$last" ] && [ -f "$dir/last" ]; then cp "$dir/last" "$last"; fi
if [ -f "$dir/sleep" ]; then sleep 30; fi
cat "$dir/out"
"#;

fn set_env(k: &str, v: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: this test binary has a single test, and no other thread of
    // ours runs while the environment changes.
    unsafe { std::env::set_var(k, v) };
}

/// Installs a command of an agent's name, first in PATH.
fn fake_agent(base: &Path, name: &str, out: &str, orig_path: &str) -> PathBuf {
    let dir = tempfile::tempdir_in(base).unwrap().keep();
    std::fs::write(dir.join("out"), out).unwrap();
    let bin = dir.join(name);
    std::fs::write(&bin, SCRIPT).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    set_env("PATH", format!("{}:{orig_path}", dir.display()));
    dir
}

fn line(v: Value) -> String {
    format!("{v}\n")
}

struct Case {
    name: &'static str,
    agent: &'static str,
    out: String,
    last: &'static str,
    cmds: Vec<(&'static str, &'static str)>,
    state: &'static str,
    model: &'static str,
    args: Vec<&'static str>,
    no_args: Vec<&'static str>,
    env: &'static str,
    progress: &'static str,
}

#[test]
fn fake_agents() {
    let base = tempfile::tempdir().unwrap();
    let orig_path = std::env::var("PATH").unwrap_or_default();
    set_env("DIFFR_NO_SHELL_PATH", "1");
    let structured: Value = serde_json::from_str(ANSWER).unwrap();
    let fenced = format!("```json\n{ANSWER}\n```");

    let cases = vec![
        Case {
            name: "claude",
            agent: "claude",
            out: line(json!({"type": "system", "subtype": "init", "model": "claude-test"}))
                + &line(
                    json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "input": {"file_path": "/repo/src/greet.go"}}]}}),
                )
                + &line(
                    json!({"type": "result", "is_error": false, "result": "", "structured_output": structured}),
                ),
            last: "",
            cmds: vec![],
            state: "",
            model: "claude-test",
            args: vec![
                "-p",
                "--json-schema",
                "--permission-mode",
                "dontAsk",
                "Bash(git show:*)",
            ],
            no_args: vec![],
            env: "",
            progress: "Reading greet.go",
        },
        Case {
            name: "codex",
            agent: "codex",
            out: line(
                json!({"type": "item.started", "item": {"type": "command_execution", "command": "/bin/zsh -lc \"git show abc:src/greet.go\""}}),
            ) + &line(
                json!({"type": "item.completed", "item": {"type": "agent_message", "text": ANSWER}}),
            ),
            last: ANSWER,
            cmds: vec![],
            state: "",
            model: "",
            args: vec!["exec", "--sandbox", "read-only", "--output-schema", "-"],
            no_args: vec![],
            env: "",
            progress: "Running git show abc:src/greet.go",
        },
        Case {
            name: "opencode",
            agent: "opencode",
            out: line(
                json!({"type": "tool_use", "part": {"type": "tool", "tool": "grep", "state": {"input": {"pattern": "Hello"}}}}),
            ) + &line(json!({"type": "text", "part": {"type": "text", "text": fenced}})),
            last: "",
            cmds: vec![
                ("--version", "1.0.150\n"),
                ("debug", r#"{"model":"anthropic/claude-sonnet-4-5"}"#),
            ],
            state: "",
            model: "anthropic/claude-sonnet-4-5",
            args: vec![
                "run",
                "--format",
                "json",
                "--model",
                "anthropic/claude-sonnet-4-5",
            ],
            no_args: vec!["--agent"],
            env: r#"OPENCODE_CONFIG_CONTENT={"permission":{"edit":"deny","bash":"deny","webfetch":"deny"}}"#,
            progress: "Searching for \"Hello\"",
        },
        // OpenCode 2 runs the explore agent, with the model last used in
        // OpenCode and its variant, and paths given whole.
        Case {
            name: "opencode2",
            agent: "opencode",
            out: line(json!({"type": "step_start"}))
                + &line(json!({"type": "step_start"}))
                + &line(
                    json!({"type": "text", "part": {"type": "text", "text": fenced.replace("\"src/", "\"/repo/src/")}}),
                ),
            last: "",
            cmds: vec![
                ("--version", "opencode v2.0.22\n"),
                (
                    "debug",
                    r#"[{"type":"document","path":"/home/opencode.jsonc","info":{"lsp":false}}]"#,
                ),
            ],
            state: r#"{"recent":[{"providerID":"deepseek","modelID":"deepseek-flash"}],"variant":{"deepseek/deepseek-flash":"max"}}"#,
            model: "deepseek/deepseek-flash#max",
            args: vec![
                "run",
                "--format",
                "json",
                "--agent",
                "explore",
                "--model",
                "deepseek/deepseek-flash#max",
            ],
            no_args: vec![],
            env: "",
            progress: "Retrying…",
        },
    ];

    for tc in cases {
        let dir = fake_agent(base.path(), tc.agent, &tc.out, &orig_path);
        if !tc.last.is_empty() {
            std::fs::write(dir.join("last"), tc.last).unwrap();
        }
        for (cmd, out) in &tc.cmds {
            std::fs::write(dir.join(format!("cmd-{cmd}")), out).unwrap();
        }
        let state_dir = tempfile::tempdir_in(base.path()).unwrap();
        set_env("XDG_STATE_HOME", state_dir.path());
        if !tc.state.is_empty() {
            std::fs::create_dir_all(state_dir.path().join("opencode")).unwrap();
            std::fs::write(state_dir.path().join("opencode/model.json"), tc.state).unwrap();
        }
        let agent = by_name(tc.agent).unwrap();
        let work = tempfile::tempdir_in(base.path()).unwrap();
        let req = Request {
            title: "Greet".into(),
            files: files(),
            dir: work.path().to_path_buf(),
            rev: "abc".into(),
            ..Default::default()
        };
        let mut steps: Vec<String> = Vec::new();
        let an = run(agent, &req, &CancelToken::new(), |s| {
            steps.push(s.to_string())
        })
        .unwrap_or_else(|e| panic!("{}: {e}", tc.name));
        assert_eq!(an.groups.len(), 3, "{}: {an:?}", tc.name);
        assert_eq!(an.agent, tc.agent, "{}", tc.name);
        assert_eq!(an.model, tc.model, "{}", tc.name);
        assert!(an.generated_at.is_some());
        assert!(
            steps.iter().any(|s| s == tc.progress),
            "{}: progress {steps:?}, want {:?}",
            tc.name,
            tc.progress
        );
        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        for want in &tc.args {
            assert!(args.contains(want), "{}: no {want:?} in {args:?}", tc.name);
        }
        for not in &tc.no_args {
            assert!(!args.contains(not), "{}: {not:?} in {args:?}", tc.name);
        }
        // Without labs, no lab's tool.
        assert!(
            !args
                .iter()
                .any(|a| a.contains("check-diagram") || a.contains("check_diagram")),
            "{}: a lab's tool in {args:?}",
            tc.name
        );
        assert!(an.labs.is_empty(), "{}", tc.name);
        let env = std::fs::read_to_string(dir.join("env")).unwrap();
        let has_config = env.contains("OPENCODE_CONFIG_CONTENT=");
        if tc.env.is_empty() {
            assert!(!has_config, "{}: a config in the environment", tc.name);
        } else {
            assert!(env.contains(tc.env), "{}: environment {env}", tc.name);
        }
        let stdin = std::fs::read_to_string(dir.join("stdin")).unwrap();
        assert!(
            stdin.contains("---MANIFEST---"),
            "{}: stdin {stdin}",
            tc.name
        );
        assert_eq!(stdin.contains("<schema>"), tc.agent == "opencode");
        assert_eq!(stdin.matches("---DIFFS---").count(), 1);
    }

    // The diagrams lab: its prompt, schema and tool, for Claude Code.
    let with_diagrams = |dir: &Path| Request {
        title: "Greet".into(),
        files: files(),
        dir: dir.to_path_buf(),
        labs: vec!["diagrams".into(), "unknown".into()],
        tool_bin: "/opt/diffr/bin/diffr".into(),
        ..Default::default()
    };
    let grouped: Value = serde_json::from_str(GROUPED_ANSWER).unwrap();
    let claude_out = line(
        json!({"type": "result", "is_error": false, "result": "", "structured_output": grouped}),
    );
    let dir = fake_agent(base.path(), "claude", &claude_out, &orig_path);
    let work = tempfile::tempdir_in(base.path()).unwrap();
    let an = run(
        by_name("claude").unwrap(),
        &with_diagrams(work.path()),
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(an.labs, ["diagrams"]);
    assert_eq!(
        an.file_summary("src/greet.go"),
        Some("Shortens the greeting.")
    );
    assert_eq!(an.diagrams.len(), 1);
    assert_eq!(
        an.diagrams[0].mermaid,
        "flowchart LR\n  greet:::changed --> out"
    );
    assert_eq!(
        an.lab_field("diagram"),
        Some(an.diagrams[0].mermaid.as_str())
    );
    assert_eq!(an.groups[0].diagram, "sequenceDiagram\n  A->>B: hi");
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    let allowed = args.iter().position(|a| *a == "--allowedTools").unwrap();
    let disallowed = args.iter().position(|a| *a == "--disallowedTools").unwrap();
    let rule = args
        .iter()
        .position(|a| *a == "Bash(/opt/diffr/bin/diffr check-diagram:*)")
        .unwrap_or_else(|| panic!("no rule in {args:?}"));
    assert!(allowed < rule && rule < disallowed, "{args:?}");
    let all = args.join("\n");
    assert!(!all.contains("<schema>"));
    assert!(all.contains(
        "run `\"/opt/diffr/bin/diffr\" check-diagram` with the diagram's Mermaid source on stdin"
    ));
    assert!(all.contains("Bash(/opt/diffr/bin/diffr --check-diagram:*)"));
    assert!(all.contains("\"required\": [\"overallSummary\", \"groups\", \"order\", \"diagram\"]"));
    assert!(all.contains(
        "\"required\": [\"key\", \"label\", \"summary\", \"critical\", \"filePaths\", \"fileSummaries\", \"fileNotes\", \"lineNotes\", \"diagram\"]"
    ));

    // A large review keeps the complete patch available during the run and
    // removes the temporary file afterwards.
    let mut req = with_diagrams(work.path());
    req.files[0].patch = "+changed line\n".repeat(diffr_agent::INLINE_LIMIT / 10);
    let git_dir = work.path().join(".git");
    std::fs::create_dir(&git_dir).unwrap();
    let expected_patch = diffr_agent::diffs(&req.files);
    let mut patch_path = None;
    run(by_name("claude").unwrap(), &req, &CancelToken::new(), |step| {
        if step == "Starting Claude Code…" {
            let file = std::fs::read_dir(&git_dir).unwrap().next().unwrap().unwrap();
            assert_eq!(std::fs::read_to_string(file.path()).unwrap(), expected_patch);
            patch_path = Some(file.path());
        }
    }).unwrap();
    let patch_path = patch_path.expect("a temporary patch during the run");
    assert!(!patch_path.exists());
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    assert!(args.contains(patch_path.to_str().unwrap()));
    let stdin = std::fs::read_to_string(dir.join("stdin")).unwrap();
    assert!(stdin.contains("---MANIFEST---"));
    assert!(!stdin.contains("---DIFFS---"));

    // Codex failing with an API's error as JSON.
    fake_agent(
        base.path(),
        "codex",
        &line(
            json!({"type": "turn.failed", "error": {"message": r#"{"type":"error","status":400,"error":{"message":"The model is not supported."}}"#}}),
        ),
        &orig_path,
    );
    let work = tempfile::tempdir_in(base.path()).unwrap();
    let req = Request {
        files: files(),
        dir: work.path().to_path_buf(),
        ..Default::default()
    };
    let err = run(by_name("codex").unwrap(), &req, &CancelToken::new(), |_| {}).unwrap_err();
    assert_eq!(err.to_string(), "Codex failed: The model is not supported.");

    // An agent stopped while at work: its whole group is killed.
    let dir = fake_agent(base.path(), "claude", "", &orig_path);
    std::fs::write(dir.join("sleep"), "").unwrap();
    let cancel = CancelToken::new();
    let stopper = {
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c.cancel();
        })
    };
    let start = Instant::now();
    let err = run(by_name("claude").unwrap(), &req, &cancel, |_| {}).unwrap_err();
    stopper.join().unwrap();
    assert_eq!(err, Error::Canceled);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );

    // OpenCode's default model: its config's wins over the one used last.
    let dir = fake_agent(base.path(), "opencode", "", &orig_path);
    std::fs::write(
        dir.join("cmd-models"),
        "deepseek/deepseek-flash\nopenai/gpt-6\n",
    )
    .unwrap();
    std::fs::write(dir.join("cmd---version"), "opencode v2.0.22\n").unwrap();
    std::fs::write(
        dir.join("cmd-debug"),
        r#"[{"type":"document","info":{"model":"openai/gpt-6"}},{"type":"directory"}]"#,
    )
    .unwrap();
    let state = tempfile::tempdir_in(base.path()).unwrap();
    set_env("XDG_STATE_HOME", state.path());
    std::fs::create_dir_all(state.path().join("opencode")).unwrap();
    std::fs::write(
        state.path().join("opencode/model.json"),
        r#"{"recent":[{"providerID":"deepseek","modelID":"deepseek-flash"}],"variant":{"openai/gpt-6":"high"}}"#,
    )
    .unwrap();
    let list = models(by_name("opencode").unwrap(), work.path()).unwrap();
    assert_eq!(list.models.len(), 2);
    assert_eq!(list.default_model, "openai/gpt-6#high");

    // Commit messages: Claude's validated answer, OpenCode's fenced one.
    let msg = json!({"type": "feat", "scope": "greet", "description": "Say hi.", "body": "Shorter.", "breaking": ""});
    let dir = fake_agent(
        base.path(),
        "claude",
        &line(json!({"type": "result", "is_error": false, "result": "", "structured_output": msg})),
        &orig_path,
    );
    let mreq = diffr_agent::MessageRequest {
        dir: work.path().to_path_buf(),
        files: files(),
        recent: vec!["fix(git): read the index".into()],
        ..Default::default()
    };
    let m = diffr_agent::commit::message(by_name("claude").unwrap(), &mreq, &CancelToken::new(), |_| {}).unwrap();
    assert_eq!(m.subject(), "feat(greet): say hi");
    assert_eq!(m.body_text(), "Shorter.");
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    assert!(args.contains("Conventional Commits"), "{args}");
    assert!(args.contains("\"breaking\""), "{args}");
    let stdin = std::fs::read_to_string(dir.join("stdin")).unwrap();
    assert!(stdin.contains("---STAGED--- (4 files") && stdin.contains("fix(git): read the index"), "{stdin}");
    let fenced_msg = format!("```json\n{msg}\n```");
    fake_agent(
        base.path(),
        "opencode",
        &line(json!({"type": "text", "part": {"text": fenced_msg}})),
        &orig_path,
    );
    let m = diffr_agent::commit::message(by_name("opencode").unwrap(), &mreq, &CancelToken::new(), |_| {}).unwrap();
    assert_eq!(m.subject(), "feat(greet): say hi");
    let empty_req = diffr_agent::MessageRequest { files: vec![], ..mreq.clone() };
    assert!(diffr_agent::commit::message(by_name("opencode").unwrap(), &empty_req, &CancelToken::new(), |_| {}).is_err());

    // A review of uncommitted changes asks for a commit per group.
    let dir = fake_agent(
        base.path(),
        "claude",
        &line(json!({"type": "result", "is_error": false, "result": "", "structured_output": structured})),
        &orig_path,
    );
    let creq = Request { suggest_commits: true, ..req.clone() };
    run(by_name("claude").unwrap(), &creq, &CancelToken::new(), |_| {}).unwrap();
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    assert!(args.contains("<commits>") && args.contains("\"commit\": {\"type\": \"object\""), "{args}");

    // Codex's fast mode.
    let dir = fake_agent(
        base.path(),
        "codex",
        &line(json!({"type": "item.completed", "item": {"type": "agent_message", "text": ANSWER}})),
        &orig_path,
    );
    let freq = Request { fast: true, ..req.clone() };
    run(by_name("codex").unwrap(), &freq, &CancelToken::new(), |_| {}).unwrap();
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    assert!(args.contains("-c\nservice_tier=\"fast\""), "{args}");
    run(by_name("codex").unwrap(), &req, &CancelToken::new(), |_| {}).unwrap();
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    assert!(!args.contains("service_tier"), "{args}");

    // Not installed.
    let empty = tempfile::tempdir_in(base.path()).unwrap();
    set_env("PATH", empty.path());
    let err = run(
        by_name("opencode").unwrap(),
        &req,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("not installed"), "{err}");
    assert!(diffr_agent::installed().is_empty());
    set_env("PATH", &orig_path);
}
