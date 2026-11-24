//! The models each agent can review with.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::run::last_line;
use crate::{Agent, Error, nullable, proc, shellenv};

/// A model an agent can review with.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Model {
    /// What the agent's `--model` takes.
    pub id: String,
    /// Names it among its provider's.
    pub label: String,
    /// Groups OpenCode's models by provider; "" for the others.
    pub provider: String,
    /// The model has a fast mode (Codex's `fast` service tier).
    #[serde(default)]
    pub fast: bool,
}

impl Model {
    fn new(id: &str, label: &str, provider: &str) -> Self {
        Model {
            id: id.into(),
            label: label.into(),
            provider: provider.into(),
            fast: false,
        }
    }
}

/// The models of an agent, and the one it uses when none is named.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<Model>,
    /// The model the agent uses in the directory when none is named, ""
    /// when it does not tell.
    pub default_model: String,
}

/// Claude Code's aliases of its latest models.
pub fn claude_models() -> Vec<Model> {
    [
        ("fable", "Fable"),
        ("opus", "Opus"),
        ("sonnet", "Sonnet"),
        ("haiku", "Haiku"),
    ]
    .iter()
    .map(|(id, label)| Model::new(id, label, ""))
    .collect()
}

const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// Lists the models an agent can review with, and the one it uses in
/// `dir` when none is named. It runs the agent: call it off the UI thread.
pub fn models(agent: Agent, dir: &Path) -> Result<ModelList, Error> {
    if agent.name == "claude" {
        return Ok(ModelList {
            models: claude_models(),
            default_model: String::new(),
        });
    }
    let Some(bin) = shellenv::look_path(agent.name) else {
        return Err(Error::Failed(format!("{} is not installed", agent.label)));
    };
    match agent.name {
        "codex" => {
            let out = output(&bin, Some(dir), &["debug", "models"])?;
            Ok(ModelList {
                models: codex_models(&out)?,
                default_model: String::new(),
            })
        }
        "opencode" => {
            let out = output(&bin, Some(dir), &["models"])?;
            let v2 = opencode_major(&bin) >= 2;
            Ok(ModelList {
                models: provider_models(&out),
                default_model: opencode_model(&bin, dir, "", v2),
            })
        }
        other => Err(Error::Failed(format!("unknown agent {other}"))),
    }
}

/// Runs a command of an agent in `dir` and returns what it prints.
fn output(bin: &Path, dir: Option<&Path>, args: &[&str]) -> Result<Vec<u8>, Error> {
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(std::process::Stdio::null());
    if let Some(d) = dir.filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(d);
    }
    shellenv::apply(&mut cmd);
    proc::hide_console(&mut cmd);
    let name = bin
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let what = format!("{name} {}", args.join(" "));
    match proc::output(&mut cmd, LIST_TIMEOUT) {
        Ok(o) if o.status.success() => Ok(o.stdout),
        Ok(o) => {
            let msg = last_line(&String::from_utf8_lossy(&o.stderr));
            if msg.is_empty() {
                Err(Error::Failed(format!("{what}: {}", o.status)))
            } else {
                Err(Error::Failed(format!("{what}: {msg}")))
            }
        }
        Err(e) => Err(Error::Failed(format!("{what}: {e}"))),
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodexModel {
    #[serde(deserialize_with = "nullable")]
    slug: String,
    #[serde(deserialize_with = "nullable")]
    display_name: String,
    #[serde(deserialize_with = "nullable")]
    visibility: String,
    #[serde(deserialize_with = "nullable")]
    priority: i64,
    #[serde(deserialize_with = "nullable")]
    additional_speed_tiers: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodexCatalog {
    #[serde(deserialize_with = "nullable")]
    models: Vec<CodexModel>,
}

/// Reads Codex's catalog (`codex debug models`): the models it lists, in
/// its order.
pub fn codex_models(data: &[u8]) -> Result<Vec<Model>, Error> {
    let catalog: CodexCatalog = serde_json::from_slice(data)
        .map_err(|e| Error::Failed(format!("could not read the models of Codex: {e}")))?;
    let mut listed: Vec<CodexModel> = catalog
        .models
        .into_iter()
        .filter(|m| !m.slug.is_empty() && (m.visibility.is_empty() || m.visibility == "list"))
        .collect();
    listed.sort_by_key(|m| m.priority);
    Ok(listed
        .into_iter()
        .map(|m| {
            let label = if m.display_name.is_empty() {
                m.slug.clone()
            } else {
                m.display_name
            };
            Model {
                fast: m.additional_speed_tiers.iter().any(|t| t == "fast"),
                id: m.slug,
                label,
                provider: String::new(),
            }
        })
        .collect())
}

/// Reads OpenCode's list of models, one `provider/model` a line.
pub fn provider_models(data: &[u8]) -> Vec<Model> {
    String::from_utf8_lossy(data)
        .lines()
        .filter_map(|line| {
            let (provider, name) = line.trim().split_once('/')?;
            (!provider.is_empty() && !name.is_empty() && !provider.contains([' ', '\t']))
                .then(|| Model::new(&format!("{provider}/{name}"), name, provider))
        })
        .collect()
}

/// The major version of OpenCode, 1 when it does not tell.
pub(crate) fn opencode_major(bin: &Path) -> u32 {
    let Ok(out) = output(bin, None, &["--version"]) else {
        return 1;
    };
    let out = String::from_utf8_lossy(&out);
    let Some(last) = out.split_whitespace().last() else {
        return 1;
    };
    let v = last.strip_prefix('v').unwrap_or(last);
    let major = v.split('.').next().unwrap_or("");
    match major.parse::<u32>() {
        Ok(n) if n > 0 => n,
        _ => 1,
    }
}

/// The model OpenCode reviews with in `dir`: the one chosen, else the one
/// its own interface would start with, of its config or else used last.
/// `opencode run` takes neither of these itself, and falls back on the
/// first model it knows, which may not even write text. With variants, as
/// from OpenCode 2, the variant chosen for the model in OpenCode comes with
/// it. "" leaves the choice to OpenCode.
pub(crate) fn opencode_model(bin: &Path, dir: &Path, chosen: &str, variants: bool) -> String {
    let mut model = chosen.to_string();
    if model.is_empty() {
        model = opencode_config_model(bin, dir);
    }
    let state = opencode_state();
    if model.is_empty()
        && let Some(r) = state.recent.first()
        && !r.provider_id.is_empty()
        && !r.model_id.is_empty()
    {
        model = format!("{}/{}", r.provider_id, r.model_id);
    }
    if model.is_empty() || !variants || model.contains('#') {
        return model;
    }
    if let Some(v) = state.variant.get(&model)
        && !v.is_empty()
        && v != "default"
    {
        model = format!("{model}#{v}");
    }
    model
}

/// The model of OpenCode's config in `dir`, "" for none. OpenCode 2 lists
/// the config's sources, the last winning; OpenCode 1, the config itself.
fn opencode_config_model(bin: &Path, dir: &Path) -> String {
    let Ok(out) = output(bin, Some(dir), &["debug", "config"]) else {
        return String::new();
    };
    let out = out.trim_ascii();
    if out.starts_with(b"{") {
        let conf: Value = serde_json::from_slice(out).unwrap_or(Value::Null);
        return config_model(conf.get("model").unwrap_or(&Value::Null));
    }
    let sources: Vec<Value> = serde_json::from_slice(out).unwrap_or_default();
    for s in sources.iter().rev() {
        if s.get("type").and_then(Value::as_str) == Some("document") {
            let m = config_model(s.pointer("/info/model").unwrap_or(&Value::Null));
            if !m.is_empty() {
                return m;
            }
        }
    }
    String::new()
}

/// Reads a model of OpenCode's config: `"provider/model"`, or an object
/// with the provider, the model and its variant.
pub fn config_model(raw: &Value) -> String {
    if let Some(s) = raw.as_str() {
        return s.to_string();
    }
    let field = |k: &str| raw.get(k).and_then(Value::as_str).unwrap_or_default();
    let (provider, model, variant) = (field("providerID"), field("model"), field("variant"));
    if provider.is_empty() || model.is_empty() {
        return String::new();
    }
    let mut id = format!("{provider}/{model}");
    if !variant.is_empty() && variant != "default" {
        id = format!("{id}#{variant}");
    }
    id
}

/// What OpenCode's interface remembers of the models: those used last,
/// and the variant chosen for each.
#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenCodeStateFile {
    #[serde(deserialize_with = "nullable")]
    recent: Vec<Recent>,
    #[serde(deserialize_with = "nullable")]
    variant: HashMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Recent {
    #[serde(rename = "providerID", deserialize_with = "nullable")]
    provider_id: String,
    #[serde(rename = "modelID", deserialize_with = "nullable")]
    model_id: String,
}

fn opencode_state() -> OpenCodeStateFile {
    let dir = match std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(".local")
            .join("state"),
    };
    std::fs::read(dir.join("opencode").join("model.json"))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

/// A model's name on its button: its label, without its provider's.
pub fn model_name(id: &str, models: &[Model]) -> String {
    let name = models
        .iter()
        .find(|m| m.id == id)
        .map_or(id, |m| m.label.as_str());
    name.rsplit('/').next().unwrap_or(name).to_string()
}

/// Models gathered under a key, as a menu's submenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelGroup {
    pub key: String,
    pub models: Vec<Model>,
}

/// Groups models by a key, in the order of their first.
pub fn group_models(models: &[Model], key: impl Fn(&Model) -> String) -> Vec<ModelGroup> {
    let mut groups: Vec<ModelGroup> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for m in models {
        let k = key(m);
        let i = *at.entry(k.clone()).or_insert_with(|| {
            groups.push(ModelGroup {
                key: k,
                models: Vec::new(),
            });
            groups.len() - 1
        });
        groups[i].models.push(m.clone());
    }
    groups
}

/// The maker of a model a provider serves for others, as `anthropic` of
/// `vercel`'s `anthropic/claude-sonnet-4.6`, "" for none: models' menus
/// group by [`Model::provider`], then by this.
pub fn model_vendor(m: &Model) -> String {
    m.label
        .split_once('/')
        .map(|(v, _)| v.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_lists() {
        let codex = codex_models(
            br#"{"models":[
		{"slug":"gpt-b","display_name":"GPT-B","visibility":"list","priority":5,"additional_speed_tiers":["fast"]},
		{"slug":"gpt-hidden","display_name":"Hidden","visibility":"hide","priority":1},
		{"slug":"gpt-a","display_name":"GPT-A","visibility":"list","priority":2}]}"#,
        )
        .unwrap();
        assert_eq!(
            codex,
            [
                Model::new("gpt-a", "GPT-A", ""),
                Model {
                    fast: true,
                    ..Model::new("gpt-b", "GPT-B", "")
                }
            ]
        );
        let oc =
            provider_models(b"deepseek/deepseek-flash\nvercel/bfl/flux-3-image\n\nnot a model\n");
        assert_eq!(
            oc,
            [
                Model::new("deepseek/deepseek-flash", "deepseek-flash", "deepseek"),
                Model::new("vercel/bfl/flux-3-image", "bfl/flux-3-image", "vercel"),
            ]
        );
        for (raw, want) in [
            (r#""openai/gpt-6""#, "openai/gpt-6"),
            (
                r#"{"providerID":"deepseek","model":"deepseek-flash","variant":"max"}"#,
                "deepseek/deepseek-flash#max",
            ),
            ("null", ""),
        ] {
            let v: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(config_model(&v), want, "{raw}");
        }
    }

    #[test]
    fn menus() {
        let models = [
            Model::new("vercel/anthropic/claude", "anthropic/claude", "vercel"),
            Model::new("vercel/openai/gpt", "openai/gpt", "vercel"),
            Model::new("deepseek/flash", "flash", "deepseek"),
            Model::new("vercel/anthropic/haiku", "anthropic/haiku", "vercel"),
        ];
        let by_provider = group_models(&models, |m| m.provider.clone());
        assert_eq!(by_provider.len(), 2);
        assert_eq!(by_provider[0].key, "vercel");
        assert_eq!(by_provider[0].models.len(), 3);
        let vendors = group_models(&by_provider[0].models, model_vendor);
        assert_eq!(
            vendors.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(),
            ["anthropic", "openai"]
        );
        assert_eq!(model_name("vercel/anthropic/claude", &models), "claude");
        assert_eq!(model_name("x/unknown", &models), "unknown");
    }
}
