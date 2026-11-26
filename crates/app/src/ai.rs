//! The AI review: a coding agent installed on the machine reviews the
//! changes, read-only, and the review shows atop them.
//! The agent runs through its own CLI, with its
//! own sign-in, its streamed events read for progress.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use diffr_agent::cache::SourceKind;
use diffr_agent::{Agent, Analysis, CancelToken, Model};
use gpui::{AnyElement, Context, FontWeight, MouseButton, SharedString, Window, div, prelude::*, px};

use crate::features::{GroupMode, patch_text};
use crate::state::*;
use crate::theme::alpha;
use crate::ui::{self, icon, icon_button, pal, tooltip};
use crate::window::Diffr;

/// The review of a source.
#[derive(Default)]
pub struct AnalysisState {
    pub result: Option<Analysis>,
    /// The fingerprints of the files the review saw, by path.
    pub fingerprints: HashMap<String, String>,
    pub running: bool,
    pub progress: String,
    pub started: Option<Instant>,
    pub err: String,
    pub cancel: Option<CancelToken>,
    pub agent: String,
    /// The summary hidden, as a review saved shows at first.
    pub folded: bool,
    pub all_notes: bool,
    /// The card offers a review, before one runs.
    pub offered: bool,
    pub checked: bool,
}

pub const LISTED_NOTES: usize = 5;

/// An item of the agent or model menu: its label, whether it is checked
/// and enabled, and the agent and model it chooses.
pub type MenuItemSpec = (String, bool, bool, Option<(String, String)>);

fn source_kind(src: &Source) -> (SourceKind, String) {
    match src {
        Source::WorkingTree => (SourceKind::WorkingTree, String::new()),
        Source::Commit(h) => (SourceKind::Commit, h.clone()),
        Source::Branch(b) => (SourceKind::Branch, b.clone()),
        Source::Pull(p) => (SourceKind::Pull, p.clone()),
    }
}

fn state_dir() -> Option<PathBuf> {
    diffr_git::Store::default_path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn analysis_path(root: &str, src: &Source) -> Option<PathBuf> {
    let (kind, r) = source_kind(src);
    Some(diffr_agent::cache::analysis_path(&state_dir()?, root, kind, &r))
}

/// The agents installed, found once in the background.
static AGENTS: OnceLock<Mutex<Option<Vec<Agent>>>> = OnceLock::new();

pub fn installed_agents() -> Option<Vec<Agent>> {
    let m = AGENTS.get_or_init(|| {
        std::thread::spawn(|| {
            let list = diffr_agent::installed();
            *AGENTS.get().unwrap().lock().unwrap() = Some(list);
        });
        Mutex::new(None)
    });
    m.lock().unwrap().clone()
}

#[derive(Clone, Default)]
pub struct AgentModels {
    pub models: Vec<Model>,
    pub default_model: String,
    pub err: String,
    pub done: bool,
    fetching: bool,
    fetched: Option<Instant>,
}

static MODELS: OnceLock<Mutex<HashMap<&'static str, AgentModels>>> = OnceLock::new();

/// The models an agent lists, fetched in the background and kept a while.
pub fn models_of(a: Agent, dir: &str) -> AgentModels {
    let m = MODELS.get_or_init(Default::default);
    let mut map = m.lock().unwrap();
    let list = map.entry(a.name).or_default();
    if !list.fetching && (!list.done || list.fetched.is_some_and(|t| t.elapsed() > Duration::from_secs(300))) {
        list.fetching = true;
        let dir = PathBuf::from(dir);
        std::thread::spawn(move || {
            let res = diffr_agent::models(a, &dir);
            let mut map = MODELS.get().unwrap().lock().unwrap();
            let list = map.entry(a.name).or_default();
            match res {
                Ok(l) => {
                    list.models = l.models;
                    list.default_model = l.default_model;
                    list.err.clear();
                }
                Err(e) => list.err = e.to_string(),
            }
            list.done = true;
            list.fetching = false;
            list.fetched = Some(Instant::now());
        });
    }
    list.clone()
}

/// A changed file as the agent is shown it.
pub fn agent_file(f: &diffr_git::File) -> diffr_agent::File {
    diffr_agent::File {
        path: f.path.clone(),
        old_path: f.old_path.clone(),
        status: match f.status {
            diffr_git::Status::Untracked => 'A',
            s => s.letter(),
        },
        additions: f.additions,
        deletions: f.deletions,
        binary: f.binary,
        generated: f.generated,
        patch: if f.generated { String::new() } else { patch_text(f) },
    }
}

pub fn model_name(id: &str, models: &[Model]) -> String {
    let name = models.iter().find(|m| m.id == id).map(|m| m.label.clone()).unwrap_or_else(|| id.to_string());
    name.rsplit('/').next().unwrap_or(&name).to_string()
}

impl Diffr {
    fn analysis_for(&mut self, src: &Source) -> &mut AnalysisState {
        self.ext.analyses.entry(src.clone()).or_default()
    }

    pub fn analysis_result(&self) -> Option<&Analysis> {
        self.ext.analyses.get(&self.source).and_then(|a| a.result.as_ref())
    }

    pub fn analysis_with_fingerprints(&self) -> Option<(Analysis, HashMap<String, String>)> {
        let a = self.ext.analyses.get(&self.source)?;
        Some((a.result.clone()?, a.fingerprints.clone()))
    }

    pub fn analysis_running(&self) -> bool {
        self.ext.analyses.get(&self.source).is_some_and(|a| a.running)
    }

    pub fn show_summary(&self) -> bool {
        self.ext
            .analyses
            .get(&self.source)
            .is_some_and(|st| st.result.is_some() || st.running || !st.err.is_empty() || st.offered)
            || self.ext.show_ai_card
    }

    /// Reads the review saved for a source, once.
    pub fn load_analysis(&mut self, src: &Source, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let st = self.analysis_for(src);
        if st.checked {
            return;
        }
        st.checked = true;
        let Some(path) = analysis_path(&root, src) else { return };
        let src = src.clone();
        cx.spawn(async move |this, cx| {
            let saved = cx.background_spawn(async move { diffr_agent::cache::load_analysis(&path) }).await;
            let Some((a, fps)) = saved else { return };
            this.update(cx, |this, cx| {
                let st = this.analysis_for(&src);
                if st.result.is_some() || st.running {
                    return;
                }
                st.result = Some(a);
                st.fingerprints = fps;
                st.folded = true;
                if this.source == src {
                    this.regroup(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn agent_hint(&self) -> String {
        match diffr_agent::by_name(&self.settings.ai_agent) {
            None => "Claude Code, Codex or OpenCode".into(),
            Some(a) if !self.settings.ai_model.is_empty() => format!("{} · {}", a.label, self.settings.ai_model),
            Some(a) => a.label.into(),
        }
    }

    fn review_agent(&self) -> Option<Agent> {
        diffr_agent::by_name(&self.settings.ai_agent)
            .or_else(|| installed_agents().and_then(|l| l.into_iter().next()))
    }

    /// Starts a review with the selected agent, or reveals the running one.
    pub fn review_with_ai(&mut self, cx: &mut Context<Self>) {
        let src = self.source.clone();
        let st = self.analysis_for(&src);
        if st.running {
            st.folded = false;
            self.rows_dirty = true;
            self.list.scroll_to(gpui::ListOffset { item_ix: 0, offset_in_item: px(0.) });
            cx.notify();
            return;
        }
        self.run_analysis(cx);
    }

    pub fn run_analysis(&mut self, cx: &mut Context<Self>) {
        let src = self.source.clone();
        if self.analysis_running() {
            return;
        }
        if self.loading {
            self.toast("Changes are still loading. Try the review again in a moment.", cx);
            return;
        }
        if self.files.iter().all(|f| f.file.directory) {
            self.toast("There are no changed files to review.", cx);
            return;
        }
        let mut req = diffr_agent::Request {
            dir: PathBuf::from(&self.root),
            model: self.settings.ai_model.clone(),
            labs: self.settings.labs.clone(),
            // The labs' tools are subcommands of this executable.
            tool_bin: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("diffr")),
            ..Default::default()
        };
        let (mut base, mut head) = (String::new(), String::new());
        match &src {
            Source::WorkingTree => {
                req.title = "Uncommitted changes".into();
                req.checkout = true;
                if !self.branch.is_empty() {
                    req.title = format!("{} on {}", req.title, self.branch);
                }
            }
            Source::Branch(b) => {
                req.title = format!("The changes of {} since it branched from {b}", self.branch);
                req.checkout = true;
                base = self.base.clone();
                head = "HEAD".into();
            }
            Source::Commit(h) => {
                if let Some(c) = &self.commit {
                    req.title = c.subject.clone();
                    req.description = c.body.clone();
                }
                req.rev = h.clone();
            }
            Source::Pull(_) => {
                let Some((title, body, url, rev)) = self.pull_request_info() else { return };
                req.title = title;
                req.description = diffr_github::pull::plain_body(&body);
                req.url = url;
                req.rev = rev.clone();
                base = self.base.clone();
                head = rev;
            }
        }
        let files: Vec<_> = self.files.iter().filter(|f| !f.file.directory).map(|f| f.file.clone()).collect();
        // Uncommitted changes are committed group by group.
        req.suggest_commits = src == Source::WorkingTree;
        req.fast = self.use_fast();
        let fingerprints: HashMap<String, String> =
            self.files.iter().map(|f| (f.file.path.clone(), f.file.fingerprint.clone())).collect();
        let name = self.settings.ai_agent.clone();
        let path = analysis_path(&self.root, &src);
        let cancel = CancelToken::new();
        let st = self.analysis_for(&src);
        st.running = true;
        st.err.clear();
        st.progress = "Starting…".into();
        st.started = Some(Instant::now());
        st.cancel = Some(cancel.clone());
        st.folded = false;
        st.offered = false;
        self.ext.show_ai_card = false;
        self.rows_dirty = true;
        self.list.scroll_to(gpui::ListOffset { item_ix: 0, offset_in_item: px(0.) });
        let repo = self.repo.clone();
        let progress: Arc<Mutex<(String, String)>> = Default::default();
        let sink = progress.clone();
        let job = cx.background_spawn(async move {
            if cancel.is_cancelled() {
                return Err(diffr_agent::Error::Canceled);
            }
            let a = diffr_agent::pick(&name)?;
            sink.lock().unwrap().1 = a.label.to_string();
            if !base.is_empty() && !head.is_empty() {
                req.commits = repo.subjects(&base, &head, 100);
            }
            req.files = files.iter().map(|f| agent_file(f)).collect();
            drop(files);
            let res = diffr_agent::run(a, &req, &cancel, |s| sink.lock().unwrap().0 = s.to_string());
            if let (Ok(an), Some(path)) = (&res, &path) {
                let _ = diffr_agent::cache::save_analysis(path, an, &fingerprints);
            }
            res.map(|an| (an, fingerprints))
        });
        // The progress shows as the agent works.
        let src2 = src.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let (p, who) = progress.lock().unwrap().clone();
                let running = this
                    .update(cx, |this, cx| {
                        let st = this.analysis_for(&src2);
                        if !p.is_empty() {
                            st.progress = p;
                        }
                        if !who.is_empty() {
                            st.agent = who;
                        }
                        let running = st.running;
                        if running && this.source == src2 {
                            cx.notify();
                        }
                        running
                    })
                    .unwrap_or(false);
                if !running {
                    return;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let res = job.await;
            this.update(cx, |this, cx| {
                let title = crate::window::window_title(&this.root, &src);
                let (ok, err) = match &res {
                    Ok(_) => (true, String::new()),
                    Err(diffr_agent::Error::Canceled) => (true, String::new()),
                    Err(e) => (false, e.to_string()),
                };
                if !this.window_active && !matches!(res, Err(diffr_agent::Error::Canceled)) {
                    crate::notify::notify(
                        if ok { "AI review ready" } else { "AI review failed" },
                        if ok { &title } else { &err },
                        Some(&this.root),
                    );
                }
                let st = this.analysis_for(&src);
                st.running = false;
                st.cancel = None;
                match res {
                    Err(diffr_agent::Error::Canceled) => st.err.clear(),
                    Err(e) => st.err = e.to_string(),
                    Ok((an, fps)) => {
                        st.result = Some(an);
                        st.fingerprints = fps;
                        st.err.clear();
                    }
                }
                if this.source == src {
                    this.regroup(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn stop_analysis(&mut self, cx: &mut Context<Self>) {
        if let Some(st) = self.ext.analyses.get(&self.source)
            && let Some(c) = &st.cancel
        {
            c.cancel();
        }
        cx.notify();
    }

    fn changed_since(&self) -> usize {
        let Some(st) = self.ext.analyses.get(&self.source) else { return 0 };
        diffr_agent::cache::changed_since(
            self.files.iter().map(|f| (f.file.path.as_str(), f.file.fingerprint.as_str())),
            &st.fingerprints,
        )
    }

    /// The menu of the agents.
    fn agent_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let label = diffr_agent::by_name(&self.settings.ai_agent).map(|a| a.label).unwrap_or("Automatic");
        menu_button("agent-menu", label, "The agent that reviews", cx)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_context_menu(crate::menus::ContextMenu::Agent, ev.position, cx);
                }),
            )
            .into_any_element()
    }

    /// The menu of the models of the agent.
    fn model_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let a = self.review_agent()?;
        let list = models_of(a, &self.root);
        let chosen = self.settings.ai_model.clone();
        let mut label = if chosen.is_empty() { "Default Model".to_string() } else { model_name(&chosen, &list.models) };
        if self.use_fast() {
            label.push_str(" ⚡");
        }
        let tip = format!(
            "The model that reviews: {}",
            if !chosen.is_empty() {
                chosen
            } else if !list.default_model.is_empty() {
                list.default_model
            } else {
                format!("{}'s default", a.label)
            }
        );
        Some(
            menu_button("model-menu", &label, &tip, cx)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_context_menu(crate::menus::ContextMenu::Model, ev.position, cx);
                    }),
                )
                .into_any_element(),
        )
    }

    /// The items of the agent and model menus: label, checked, enabled,
    /// and the setting chosen.
    pub fn agent_menu_items(&self) -> Vec<MenuItemSpec> {
        let installed = installed_agents();
        let mut v = vec![("Automatic".to_string(), self.settings.ai_agent.is_empty(), true, Some((String::new(), String::new())))];
        v.push(("-".into(), false, false, None));
        for a in diffr_agent::ALL {
            let missing = installed.as_ref().is_some_and(|l| !l.contains(&a));
            let label = if missing { format!("{} (not installed)", a.label) } else { a.label.to_string() };
            v.push((label, self.settings.ai_agent == a.name, !missing, Some((a.name.to_string(), String::new()))));
        }
        v
    }

    pub fn model_menu_items(&self) -> Vec<MenuItemSpec> {
        let Some(a) = self.review_agent() else { return vec![] };
        let agent = self.settings.ai_agent.clone();
        let list = models_of(a, &self.root);
        let chosen = self.settings.ai_model.clone();
        let def = if list.default_model.is_empty() { "Default".to_string() } else { format!("Default ({})", list.default_model) };
        let mut v = vec![(def, chosen.is_empty(), true, Some((agent.clone(), String::new())))];
        v.push(("-".into(), false, false, None));
        if !chosen.is_empty() && !list.models.iter().any(|m| m.id == chosen) {
            v.push((chosen.clone(), true, true, Some((agent.clone(), chosen.clone()))));
        }
        if !list.done {
            v.push(("Loading models…".into(), false, false, None));
            return v;
        }
        if !list.err.is_empty() {
            let mut msg = list.err.lines().next().unwrap_or("").trim().to_string();
            if msg.len() > 80 {
                msg.truncate(80);
                msg.push('…');
            }
            v.push((msg, false, false, None));
            return v;
        }
        // Codex's fast mode, for the models that have it.
        if a.name == "codex" {
            let fast_ok = list.models.iter().any(|m| m.fast);
            v.insert(0, ("Fast mode ⚡".into(), self.settings.ai_fast, fast_ok || !list.done, Some(("#fast".into(), String::new()))));
            v.insert(1, ("-".into(), false, false, None));
        }
        let mut provider = String::new();
        for m in &list.models {
            if !m.provider.is_empty() && m.provider != provider {
                provider = m.provider.clone();
                v.push((format!("# {provider}"), false, false, None));
            }
            let label = if a.name == "codex" && m.fast { format!("{} ⚡", m.label) } else { m.label.clone() };
            v.push((label, chosen == m.id, true, Some((agent.clone(), m.id.clone()))));
        }
        v
    }

    /// Whether to ask for Codex's fast mode: set, with Codex, and a model
    /// that has it (or one not known to lack it).
    pub fn use_fast(&self) -> bool {
        if !self.settings.ai_fast || self.review_agent().map(|a| a.name) != Some("codex") {
            return false;
        }
        let Some(a) = self.review_agent() else { return false };
        let list = models_of(a, &self.root);
        let model = if self.settings.ai_model.is_empty() { list.default_model.clone() } else { self.settings.ai_model.clone() };
        list.models.iter().find(|m| m.id == model).is_none_or(|m| m.fast)
    }

    /// Turns Codex's fast mode on or off.
    pub fn toggle_fast(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.ai_fast;
        let s = crate::config().update(|s| s.ai_fast = on);
        self.settings.ai_fast = s.ai_fast;
        cx.notify();
    }

    pub fn choose_agent(&mut self, agent: String, model: String, keep_agent: bool, cx: &mut Context<Self>) {
        let s = crate::config().update(|s| {
            if !keep_agent {
                s.ai_agent = agent.clone();
            }
            s.ai_model = model.clone();
        });
        self.settings.ai_agent = s.ai_agent;
        self.settings.ai_model = s.ai_model;
        cx.notify();
    }

    /// The card of the agent's review, atop the files.
    pub fn summary_card(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let src = self.source.clone();
        if self.ext.show_ai_card && !self.ext.analyses.get(&src).is_some_and(|s| s.result.is_some() || s.running) {
            let st = self.analysis_for(&src);
            st.offered = true;
        }
        let st = self.ext.analyses.get(&src);
        let result = st.and_then(|s| s.result.clone());
        let running = st.is_some_and(|s| s.running);
        let err = st.map(|s| s.err.clone()).unwrap_or_default();
        let folded = st.is_some_and(|s| s.folded);
        let all_notes = st.is_some_and(|s| s.all_notes);
        let progress = st.map(|s| s.progress.clone()).unwrap_or_default();
        let who = st.map(|s| s.agent.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| "The agent".into());
        let elapsed = st.and_then(|s| s.started).map(|t| t.elapsed().as_secs()).unwrap_or(0);

        let mut head = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(icon("sparkle", 15., p.accent_foreground))
            .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).flex_none().child("AI Review"));
        if let Some(a) = &result
            && !running
        {
            let mut by = diffr_agent::by_name(&a.agent).map(|g| g.label.to_string()).unwrap_or(a.agent.clone());
            if !a.model.is_empty() {
                by = format!("{by} · {}", a.model);
            }
            if let Some(t) = a.generated_at {
                by = format!("{by} · {}", relative_time(now_secs(), t.unix_timestamp()));
            }
            head = head.child(
                div()
                    .text_size(px(11.))
                    .text_color(p.text_muted)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .min_w_0()
                    .flex_shrink(1.)
                    .child(by),
            );
        }
        head = head.child(ui::spacer());
        let small = |b: gpui::Stateful<gpui::Div>| b.h(px(26.)).py(px(0.)).px(px(10.)).text_size(px(13.));
        if result.is_none() && !running && err.is_empty() {
            head = head.child(self.agent_menu(cx));
            if let Some(m) = self.model_menu(cx) {
                head = head.child(m);
            }
            head = head
                .child(
                    small(ui::primary_button("ai-run", cx))
                        .child("Review with AI")
                        .on_click(cx.listener(|this, _, _, cx| this.run_analysis(cx))),
                )
                .child(icon_button("ai-dismiss", "close", "Dismiss", cx).size(px(26.)).on_click(cx.listener(
                    |this, _, _, cx| {
                        let src = this.source.clone();
                        this.analysis_for(&src).offered = false;
                        this.ext.show_ai_card = false;
                        if this.ext.group_pref == GroupMode::Ai {
                            this.ext.group_pref = GroupMode::Auto;
                        }
                        this.regroup(cx);
                        cx.notify();
                    },
                )));
        } else if running {
            head = head.child(
                small(ui::button("ai-stop", cx))
                    .child(icon("stop", 12., p.text))
                    .child("Stop")
                    .on_click(cx.listener(|this, _, _, cx| this.stop_analysis(cx))),
            );
        } else {
            head = head.child(self.agent_menu(cx));
            if let Some(m) = self.model_menu(cx) {
                head = head.child(m);
            }
            head = head.child(
                small(ui::button("ai-again", cx))
                    .child(if result.is_some() { "Review Again" } else { "Try Again" })
                    .on_click(cx.listener(|this, _, _, cx| this.run_analysis(cx))),
            );
            if result.is_some() {
                head = head.child(
                    icon_button("ai-fold", "chevron_down", if folded { "Show the summary" } else { "Hide the summary" }, cx)
                        .size(px(26.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            let src = this.source.clone();
                            let st = this.analysis_for(&src);
                            st.folded = !st.folded;
                            cx.notify();
                        })),
                );
            }
        }
        let mut col = div()
            .flex()
            .flex_col()
            .px(px(16.))
            .py(px(12.))
            .gap(px(8.))
            .rounded(px(crate::diffview::CARD_RADIUS))
            .bg(p.header_bg)
            .border_1()
            .border_color(alpha(p.accent, 0.3))
            .child(head);
        if running {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(ui::spinner("ai-spinner", 14., p.text_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(p.text_muted)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(format!("{who}: {progress}")),
                    )
                    .child(
                        div()
                            .font_family(code_font.clone())
                            .text_size(px(11.))
                            .text_color(p.text_muted)
                            .child(format_elapsed(elapsed)),
                    ),
            );
        }
        if result.is_none() && !running && err.is_empty() {
            col = col.child(div().text_size(px(13.)).text_color(p.text_muted).child(
                "A coding agent on this machine can summarize these changes, group the files by intent and note what deserves attention. It reads, never writes, and starts only when you ask.",
            ));
        }
        if !err.is_empty() {
            col = col.child(div().text_size(px(12.)).text_color(p.danger_foreground).child(err));
        }
        let Some(a) = result else { return col.into_any_element() };
        if folded || running {
            return col.into_any_element();
        }
        if !a.overall_summary.trim().is_empty() {
            col = col.child(div().text_size(px(13.)).child(crate::markdown::markdown("ai-summary", a.overall_summary.trim(), p.text, cx)));
        }
        // The labs' output comes right after the summary: their fields as
        // Markdown, the diagrams drawn.
        for (name, value) in &a.lab_fields {
            if name == diffr_agent::DIAGRAM_FIELD || value.trim().is_empty() {
                continue;
            }
            col = col.child(div().text_size(px(13.)).child(crate::markdown::markdown(format!("lab-{name}"), value, p.text, cx)));
        }
        if !a.diagrams.is_empty() {
            col = col.child(self.diagrams_section(&a.diagrams, cx));
        }
        if self.show_file_summaries() && !a.has_file_summaries() {
            col = col.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.text_muted)
                    .child("File summaries are new: review again to get them."),
            );
        }
        let mut stats = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(div().text_size(px(12.)).text_color(p.text_muted).child(format!(
                "{} · {}",
                ui::plural(a.groups.len(), "group"),
                ui::plural(a.notes(), "note")
            )));
        let critical = a.critical();
        let critical_groups = a.groups.iter().filter(|g| g.critical).count();
        let critical_notes = critical - critical_groups;
        let critical_label = match (critical_groups, critical_notes) {
            (groups, 0) => format!("{} {} care", ui::plural(groups, "group"), if groups == 1 { "needs" } else { "need" }),
            (0, notes) => format!("{} {} care", ui::plural(notes, "note"), if notes == 1 { "needs" } else { "need" }),
            (groups, notes) => format!("{} and {} need care", ui::plural(groups, "group"), ui::plural(notes, "note")),
        };
        if critical > 0 {
            stats = stats.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .child(icon("alert", 12., p.danger_foreground))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.danger_foreground)
                            .child(critical_label),
                    ),
            );
        }
        let changed = self.changed_since();
        if changed > 0 {
            stats = stats.child(
                div()
                    .id("ai-changed")
                    .text_size(px(12.))
                    .text_color(p.reference)
                    .child(format!("{} changed since", ui::plural(changed, "file")))
                    .tooltip(tooltip("Notes on the files changed since do not show. Review again to include them.")),
            );
        }
        stats = stats.child(ui::spacer());
        if self.grouping() != GroupMode::Ai {
            stats = stats.child(
                div()
                    .id("ai-group")
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.accent_foreground)
                    .cursor_pointer()
                    .child("Group files by this review")
                    .on_click(cx.listener(|this, _, _, cx| this.choose_grouping(GroupMode::Ai, cx))),
            );
        }
        col = col.child(stats);
        // A lab turned on since the review: it takes another review.
        let missing: Vec<&str> = diffr_agent::LABS
            .iter()
            .filter(|l| self.settings.labs.iter().any(|x| x == l.id) && !a.has_lab(l.id))
            .map(|l| l.name)
            .collect();
        if !missing.is_empty() {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .bg(alpha(p.accent, 0.08))
                    .child(icon("sparkle", 12., p.accent_foreground))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .child(format!("Review again to include the {} lab.", missing.join(" and "))),
                    )
                    .child(
                        div()
                            .id("ai-lab-again")
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.accent_foreground)
                            .cursor_pointer()
                            .child("Review Again")
                            .on_click(cx.listener(|this, _, _, cx| this.run_analysis(cx))),
                    ),
            );
        }
        // What the review adds to the files: notes, summaries, or both.
        let show = self.settings.ai_file_info.clone();
        let mut seg = ui::segment_track().bg(p.surface);
        for (v, label) in [("notes", "Notes"), ("summaries", "Summaries"), ("both", "Both"), ("none", "None")] {
            let on = show == v || (show.is_empty() && v == "both");
            let mut b = div()
                .id(SharedString::from(format!("aishow-{v}")))
                .h(px(22.))
                .px(px(8.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(if on { p.text } else { p.text_muted })
                .cursor_pointer()
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let s = crate::config().update(|s| s.ai_file_info = v.to_string());
                    this.settings.ai_file_info = s.ai_file_info;
                    this.rows_dirty = true;
                    cx.notify();
                }));
            if on {
                b = b.bg(p.header_bg).font_weight(FontWeight::SEMIBOLD);
            }
            seg = seg.child(b);
        }
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .flex_wrap()
                .child(div().text_size(px(12.)).text_color(p.text_muted).child("Show on the files"))
                .child(seg),
        );
        // The notes, the first few or all.
        let mut notes = Vec::new();
        for f in &self.files {
            if let Some(v) = self.ext.notes.get(&f.file.path) {
                let mut v = v.clone();
                v.sort_by(|a, b| b.file.cmp(&a.file).then(a.line.cmp(&b.line)));
                notes.extend(v);
            }
        }
        if !notes.is_empty() {
            let total = notes.len();
            let shown: Vec<_> = if !all_notes && total > LISTED_NOTES { notes[..LISTED_NOTES].to_vec() } else { notes };
            let mut list = div().flex().flex_col().gap(px(1.)).mx(px(-8.));
            for (i, n) in shown.into_iter().enumerate() {
                let base = n.path.rsplit('/').next().unwrap_or(&n.path).to_string();
                let where_ = if n.file {
                    base
                } else if n.side == Side::Old {
                    format!("{base}:{} (old)", n.line)
                } else {
                    format!("{base}:{}", n.line)
                };
                let (accent, ic) = if n.critical { (p.danger_foreground, "alert") } else { (p.accent_foreground, "sparkle") };
                let selected = self.ext.note_sel.as_ref() == Some(&n);
                let hover = p.surface_hover;
                let mut b = div()
                    .id(SharedString::from(format!("ai-note-{i}")))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(icon(ic, 12., accent))
                    .child(
                        div()
                            .font_family(code_font.clone())
                            .text_size(px(11.))
                            .text_color(p.text_muted)
                            .whitespace_nowrap()
                            .flex_none()
                            .max_w(px(260.))
                            .overflow_hidden()
                            .child(where_),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(n.text.clone()),
                    )
                    .tooltip(tooltip(n.path.clone()));
                if selected {
                    b = b.bg(p.surface_hover);
                }
                let n2 = n.clone();
                b = b.on_click(cx.listener(move |this, _, _, cx| this.reveal_note(n2.clone(), cx)));
                list = list.child(b);
            }
            if total > LISTED_NOTES {
                let label = if all_notes { "Show fewer".to_string() } else { format!("Show {} more", total - LISTED_NOTES) };
                list = list.child(
                    div()
                        .id("ai-more")
                        .px(px(8.))
                        .py(px(4.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.accent_foreground)
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let src = this.source.clone();
                            let st = this.analysis_for(&src);
                            st.all_notes = !st.all_notes;
                            cx.notify();
                        })),
                );
            }
            col = col.child(list);
        }
        col.into_any_element()
    }
}

fn format_elapsed(s: u64) -> String {
    if s >= 60 { format!("{}m{}s", s / 60, s % 60) } else { format!("{s}s") }
}

/// A button opening a menu: its label and a chevron.
pub fn menu_button(id: &'static str, label: &str, tip: &str, cx: &gpui::App) -> gpui::Stateful<gpui::Div> {
    let p = pal(cx);
    ui::button(id, cx)
        .h(px(26.))
        .py(px(0.))
        .px(px(10.))
        .gap(px(6.))
        .text_size(px(13.))
        .child(label.to_string())
        .child(icon("chevron_down", 13., p.text_muted))
        .tooltip(tooltip(tip.to_string()))
}
