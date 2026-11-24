//! The user's settings, kept in `~/.diffr/diffr.jsonc` as codiff keeps its
//! own in `~/.codiff/codiff.jsonc`,
//! and what the app remembers apart from them: the files viewed and the
//! recent repositories.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// The agents that can review changes, by the names settings use.
pub const AGENT_NAMES: &[&str] = &["claude", "codex", "opencode"];

/// What of a review the diff shows on the files, as `aiFileInfo` names it
/// (`aiShow` is read too): the agent's notes, its summaries
/// of the files, both, or none.
pub const AI_FILE_INFO: &[&str] = &["notes", "summaries", "both", "none"];

/// The user's preferences, with codiff's names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub code_font_family: String,
    pub code_font_size: i32,
    pub copy_comments_on_close: bool,
    /// `split` or `unified`.
    pub diff_style: String,
    /// The command that opens a file in an editor; empty for the app's
    /// default, which prefers Zed. Which editor that is, and how a file is
    /// opened at a line, is the app's to decide.
    pub editor_command: String,
    /// Reads and reviews pull requests; empty, the token of the GitHub CLI
    /// is used.
    pub github_token: String,
    /// The agent that reviews changes: `claude`, `codex`, `opencode`,
    /// or empty for the first installed.
    pub ai_agent: String,
    /// The model the agent uses, of any of its providers; empty for its own
    /// default.
    pub ai_model: String,
    /// Codex's fast mode (its `fast` service tier) for the models that have
    /// it: quicker answers, for more usage. Off leaves Codex's own config.
    pub ai_fast: bool,
    /// What of a review the diff shows: `notes`, `summaries` (of the
    /// files) or `both`.
    pub ai_file_info: String,
    /// The ids of the experimental features of reviews enabled (the labs of
    /// `diffr-agent`, such as `diagrams`). Ids this version does not know
    /// are kept, and ignored.
    #[serde(deserialize_with = "string_list")]
    pub labs: Vec<String>,
    pub review_comments_prefix: String,
    /// `left` or `right`.
    pub sidebar_position: String,
    pub show_whitespace: bool,
    /// `system`, `light` or `dark`.
    pub theme: String,
    pub word_wrap: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            code_font_family: String::new(),
            code_font_size: 13,
            copy_comments_on_close: false,
            diff_style: "split".into(),
            editor_command: String::new(),
            github_token: String::new(),
            ai_agent: String::new(),
            ai_model: String::new(),
            ai_fast: false,
            ai_file_info: "both".into(),
            labs: Vec::new(),
            review_comments_prefix: "# Address these Review Comments".into(),
            sidebar_position: "left".into(),
            show_whitespace: false,
            theme: "system".into(),
            word_wrap: false,
        }
    }
}

impl Settings {
    /// Keeps the settings within what they may be.
    pub fn normalize(&mut self) {
        let d = Settings::default();
        self.code_font_family = self.code_font_family.trim().to_string();
        if self.code_font_size == 0 {
            self.code_font_size = d.code_font_size;
        }
        self.code_font_size = self.code_font_size.clamp(10, 32);
        if self.diff_style != "split" && self.diff_style != "unified" {
            self.diff_style = d.diff_style;
        }
        if self.sidebar_position != "left" && self.sidebar_position != "right" {
            self.sidebar_position = d.sidebar_position;
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = d.theme;
        }
        if self.review_comments_prefix.is_empty() {
            self.review_comments_prefix = d.review_comments_prefix;
        }
        self.github_token = self.github_token.trim().to_string();
        self.ai_model = self.ai_model.trim().to_string();
        if !AGENT_NAMES.contains(&self.ai_agent.as_str()) {
            if !self.ai_agent.is_empty() {
                self.ai_model.clear();
            }
            self.ai_agent.clear();
        }
        if !AI_FILE_INFO.contains(&self.ai_file_info.as_str()) {
            self.ai_file_info = d.ai_file_info;
        }
        let mut labs: Vec<String> = Vec::new();
        for l in &self.labs {
            let l = l.trim();
            if !l.is_empty() && !labs.iter().any(|x| x == l) {
                labs.push(l.to_string());
            }
        }
        self.labs = labs;
    }

    /// Parses a settings file, JSONC with the settings under `"settings"`:
    /// missing fields, and fields of the wrong type, keep their defaults.
    /// The result is normalized.
    pub fn parse(data: &[u8]) -> anyhow::Result<Settings> {
        let file: serde_json::Value = serde_json::from_slice(&strip_jsonc(data))?;
        let mut s = Settings::default();
        if let Some(fields) = file.get("settings").and_then(|v| v.as_object()) {
            let serde_json::Value::Object(mut current) = serde_json::to_value(&s)? else {
                unreachable!("settings serialize as an object");
            };
            for (k, v) in fields {
                // Older names of settings.
                let k = &match k.as_str() {
                    "aiShow" => "aiFileInfo".to_string(),
                    _ => k.clone(),
                };
                let Some(old) = current.get(k).cloned() else {
                    continue;
                };
                current.insert(k.clone(), v.clone());
                match serde_json::from_value::<Settings>(serde_json::Value::Object(current.clone()))
                {
                    Ok(next) => s = next,
                    Err(_) => {
                        current.insert(k.clone(), old);
                    }
                }
            }
        }
        s.normalize();
        Ok(s)
    }

    /// The settings file's text: `{"settings": …}`, indented by two spaces,
    /// with a final newline.
    pub fn to_file_string(&self) -> String {
        #[derive(Serialize)]
        struct FileData<'a> {
            settings: &'a Settings,
        }
        let mut out = serde_json::to_string_pretty(&FileData { settings: self })
            .unwrap_or_else(|_| "{}".into());
        out.push('\n');
        out
    }
}

/// Removes the comments and trailing commas of JSONC.
pub fn strip_jsonc(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut in_string = false;
    let mut i = 0;
    while i < data.len() {
        let ch = data[i];
        if in_string {
            out.push(ch);
            if ch == b'\\' && i + 1 < data.len() {
                i += 1;
                out.push(data[i]);
            } else if ch == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if ch == b'"' {
            in_string = true;
            out.push(ch);
        } else if ch == b'/' && data.get(i + 1) == Some(&b'/') {
            while i < data.len() && data[i] != b'\n' {
                i += 1;
            }
            out.push(b'\n');
        } else if ch == b'/' && data.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < data.len() && !(data[i] == b'*' && data[i + 1] == b'/') {
                i += 1;
            }
            i += 1;
        } else if ch == b',' {
            // A comma before a closing bracket goes.
            let mut j = i + 1;
            while j < data.len() && matches!(data[j], b' ' | b'\t' | b'\n' | b'\r') {
                j += 1;
            }
            if !(j < data.len() && (data[j] == b'}' || data[j] == b']')) {
                out.push(ch);
            }
        } else {
            out.push(ch);
        }
        i += 1;
    }
    out
}

/// The settings, kept in step with their file. Safe to share between
/// threads; the UI reloads them with [`Config::load`]
/// and changes them with [`Config::update`].
#[derive(Debug)]
pub struct Config {
    path: PathBuf,
    inner: Mutex<ConfigInner>,
}

#[derive(Debug)]
struct ConfigInner {
    settings: Settings,
    mod_time: Option<SystemTime>,
}

impl Config {
    /// `~/.diffr/diffr.jsonc`.
    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".diffr")
            .join("diffr.jsonc")
    }

    /// The settings of [`Config::default_path`], read.
    pub fn new() -> Config {
        Config::at(Config::default_path())
    }

    /// The settings of a file, read: the defaults when there is none.
    pub fn at(path: impl Into<PathBuf>) -> Config {
        let c = Config {
            path: path.into(),
            inner: Mutex::new(ConfigInner {
                settings: Settings::default(),
                mod_time: None,
            }),
        };
        c.load();
        c
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConfigInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The file of the settings.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings.
    pub fn get(&self) -> Settings {
        self.lock().settings.clone()
    }

    /// Changes the settings, normalizes and writes them, and returns them.
    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> Settings {
        let s = {
            let mut inner = self.lock();
            f(&mut inner.settings);
            inner.settings.normalize();
            inner.settings.clone()
        };
        if let Err(e) = self.write(&s) {
            eprintln!("diffr: {}: {e}", self.path.display());
        }
        s
    }

    /// Reads the file again if it changed since it was last read or
    /// written, and reports whether it did.
    pub fn load(&self) -> bool {
        let Ok(md) = std::fs::metadata(&self.path) else {
            return false;
        };
        let mut inner = self.lock();
        let mt = md.modified().ok();
        if mt.is_some() && mt == inner.mod_time {
            return false;
        }
        inner.mod_time = mt;
        let Ok(data) = std::fs::read(&self.path) else {
            return false;
        };
        match Settings::parse(&data) {
            Ok(s) => {
                inner.settings = s;
                true
            }
            Err(e) => {
                eprintln!("diffr: {}: {e}", self.path.display());
                false
            }
        }
    }

    fn write(&self, s: &Settings) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, s.to_file_string())?;
        if let Ok(md) = std::fs::metadata(&self.path) {
            self.lock().mod_time = md.modified().ok();
        }
        Ok(())
    }

    /// Writes the file with the settings when there is none, for the user
    /// to edit, and returns its path.
    pub fn ensure(&self) -> PathBuf {
        if std::fs::metadata(&self.path).is_err()
            && let Err(e) = self.write(&self.get())
        {
            eprintln!("diffr: {}: {e}", self.path.display());
        }
        self.path.clone()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::new()
    }
}

/// How many repositories File → Open Recent lists.
pub const MAX_RECENT: usize = 10;

/// What the app remembers apart from the settings, as `state.json` keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreData {
    /// Maps repository roots to the fingerprints of the files viewed, by
    /// path.
    #[serde(default, deserialize_with = "nullable")]
    pub viewed: HashMap<String, HashMap<String, String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub sidebar_width: f32,
    #[serde(default)]
    pub sidebar_shown: Option<bool>,
    /// The repositories opened, the last first.
    #[serde(default, deserialize_with = "nullable")]
    pub recent: Vec<String>,
    /// The repository opened last, as kept before `recent`.
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "String::is_empty"
    )]
    pub last_repository: String,
}

/// Reads a list of strings leniently: its items that are not strings go,
/// and a lone string is a list of one. Anything else is an error, so that
/// [`Settings::parse`] keeps the default.
fn string_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Array(items) => Ok(items
            .into_iter()
            .filter_map(|v| match v {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            })
            .collect()),
        serde_json::Value::String(s) => Ok(vec![s]),
        serde_json::Value::Null => Ok(Vec::new()),
        _ => Err(serde::de::Error::custom("expected a list of strings")),
    }
}

fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// The files viewed in each repository, the layout of the window and the
/// recent repositories, kept in a JSON file. Safe to share between threads.
/// A store without a path (as [`Store::default`]) keeps them in memory only.
#[derive(Debug, Default)]
pub struct Store {
    path: Option<PathBuf>,
    data: Mutex<StoreData>,
}

impl Store {
    /// The store's default file: `state.json` in diffr's directory of the
    /// user's data.
    pub fn default_path() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("diffr").join("state.json"))
    }

    /// The store of [`Store::default_path`].
    pub fn open_default() -> Store {
        match Store::default_path() {
            Some(p) => Store::open(p),
            None => Store::default(),
        }
    }

    /// The store of a file, read when there is one.
    pub fn open(path: impl Into<PathBuf>) -> Store {
        let path = path.into();
        let mut data: StoreData = std::fs::read(&path)
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        if data.recent.is_empty() && !data.last_repository.is_empty() {
            data.recent = vec![data.last_repository.clone()];
        }
        data.last_repository.clear();
        Store {
            path: Some(path),
            data: Mutex::new(data),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StoreData> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The fingerprints of the files viewed in a repository, by path.
    pub fn viewed(&self, root: &str) -> HashMap<String, String> {
        self.lock().viewed.get(root).cloned().unwrap_or_default()
    }

    /// Remembers a file viewed with the fingerprint of its change, or
    /// forgets it for an empty fingerprint.
    pub fn set_viewed(&self, root: &str, path: &str, fingerprint: &str) {
        {
            let mut d = self.lock();
            let m = d.viewed.entry(root.to_string()).or_default();
            if fingerprint.is_empty() {
                m.remove(path);
            } else {
                m.insert(path.to_string(), fingerprint.to_string());
            }
        }
        self.save();
    }

    /// The width of the sidebar (280 by default) and whether it shows.
    pub fn layout(&self) -> (f32, bool) {
        let d = self.lock();
        let width = if d.sidebar_width == 0.0 {
            280.0
        } else {
            d.sidebar_width
        };
        (width, d.sidebar_shown.unwrap_or(true))
    }

    /// Remembers the width of the sidebar and whether it shows.
    pub fn set_layout(&self, width: f32, shown: bool) {
        {
            let mut d = self.lock();
            d.sidebar_width = width;
            d.sidebar_shown = Some(shown);
        }
        self.save();
    }

    /// The repositories opened, the last first.
    pub fn recent(&self) -> Vec<String> {
        self.lock().recent.clone()
    }

    /// Puts a repository first among the recent ones, and reports whether
    /// that changed them.
    pub fn add_recent(&self, root: &str) -> bool {
        {
            let mut d = self.lock();
            if d.recent.first().is_some_and(|r| r == root) {
                return false;
            }
            d.recent.retain(|r| r != root);
            d.recent.insert(0, root.to_string());
            d.recent.truncate(MAX_RECENT);
        }
        self.save();
        true
    }

    /// Forgets the recent repositories.
    pub fn clear_recent(&self) {
        self.lock().recent.clear();
        self.save();
    }

    /// A copy of everything the store holds.
    pub fn data(&self) -> StoreData {
        self.lock().clone()
    }

    /// Writes the store to its file, if it has one.
    pub fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(data) = serde_json::to_vec(&*self.lock()) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recent() {
        let s = Store::default();
        for i in 0..MAX_RECENT + 2 {
            s.add_recent(&format!("/repo{i}"));
        }
        let got = s.recent();
        assert!(
            got.len() == MAX_RECENT && got[0] == format!("/repo{}", MAX_RECENT + 1),
            "got {got:?}"
        );
        // Opening one again brings it first, once.
        assert!(
            s.add_recent("/repo5"),
            "moving a repository first changed nothing"
        );
        assert!(
            !s.add_recent("/repo5"),
            "the first repository changed the list"
        );
        let got = s.recent();
        assert!(
            got[0] == "/repo5"
                && got.len() == MAX_RECENT
                && !got[1..].contains(&"/repo5".to_string()),
            "got {got:?}"
        );
        s.clear_recent();
        assert!(s.recent().is_empty(), "the list was not cleared");
    }

    // Further tests.

    #[test]
    fn test_parse_settings() {
        let data = br#"{
  // comment, with "quotes"
  "settings": {
    "codeFontSize": 50, /* clamped */
    "diffStyle": "unified",
    "theme": "neon",
    "wordWrap": "yes",
    "aiAgent": "codex",
    "editorCommand": "code // not a comment",
    "unknown": 1,
  },
}"#;
        let s = Settings::parse(data).unwrap();
        assert_eq!(s.code_font_size, 32);
        assert_eq!(s.diff_style, "unified");
        assert_eq!(s.theme, "system");
        assert!(!s.word_wrap);
        assert_eq!(s.ai_agent, "codex");
        assert_eq!(s.editor_command, "code // not a comment");
        assert_eq!(s.review_comments_prefix, "# Address these Review Comments");
        let round = Settings::parse(s.to_file_string().as_bytes()).unwrap();
        assert_eq!(round, s);
        assert!(
            s.to_file_string()
                .starts_with("{\n  \"settings\": {\n    \"codeFontFamily\": \"\",")
        );
    }

    #[test]
    fn test_ai_settings() {
        // A removed agent's model must not reach the automatic fallback.
        let retired = Settings::parse(
            br#"{"settings":{"aiAgent":"pi","aiModel":"openai/gpt-test"}}"#,
        )
        .unwrap();
        assert!(retired.ai_agent.is_empty());
        assert!(retired.ai_model.is_empty());

        let d = Settings::default();
        assert_eq!(d.ai_file_info, "both");
        assert!(d.labs.is_empty());
        let s = d.to_file_string();
        assert!(s.contains("\"aiFileInfo\": \"both\""), "{s}");
        assert!(s.contains("\"labs\": []"), "{s}");

        let s = Settings::parse(
            br#"{"settings": {"aiFileInfo": "summaries", "labs": [" diagrams", 3, "diagrams", "", "future"]}}"#,
        )
        .unwrap();
        assert_eq!(s.ai_file_info, "summaries");
        assert_eq!(s.labs, ["diagrams", "future"]);
        let round = Settings::parse(s.to_file_string().as_bytes()).unwrap();
        assert_eq!(round, s);

        // Wrong values and types keep the defaults.
        for data in [
            r#"{"settings": {"aiFileInfo": "everything", "labs": {"diagrams": true}}}"#,
            r#"{"settings": {"aiFileInfo": 2, "labs": 1}}"#,
            r#"{"settings": {"aiFileInfo": null, "labs": null}}"#,
        ] {
            let s = Settings::parse(data.as_bytes()).unwrap();
            assert_eq!(s.ai_file_info, "both", "{data}");
            assert!(s.labs.is_empty(), "{data}");
        }
        let s =
            Settings::parse(br#"{"settings": {"aiShow": "notes", "labs": "diagrams"}}"#).unwrap();
        assert_eq!(s.ai_file_info, "notes");
        assert_eq!(s.labs, ["diagrams"]);
    }

    #[test]
    fn test_config_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".diffr").join("diffr.jsonc");
        let c = Config::at(&path);
        assert_eq!(c.get(), Settings::default());
        assert_eq!(c.ensure(), path);
        assert!(path.exists());
        let s = c.update(|s| s.code_font_size = 5);
        assert_eq!(s.code_font_size, 10);
        assert!(!c.load(), "its own write read again");
        let c2 = Config::at(&path);
        assert_eq!(c2.get().code_font_size, 10);
    }

    #[test]
    fn test_store_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"viewed":null,"recent":null,"lastRepository":"/old"}"#,
        )
        .unwrap();
        let s = Store::open(&path);
        assert_eq!(s.recent(), vec!["/old".to_string()]);
        assert_eq!(s.layout(), (280.0, true));
        s.set_viewed("/r", "a.txt", "abc");
        s.set_layout(300.0, false);
        let s2 = Store::open(&path);
        assert_eq!(
            s2.viewed("/r").get("a.txt").map(String::as_str),
            Some("abc")
        );
        assert_eq!(s2.layout(), (300.0, false));
        s2.set_viewed("/r", "a.txt", "");
        assert!(Store::open(&path).viewed("/r").is_empty());
    }
}
