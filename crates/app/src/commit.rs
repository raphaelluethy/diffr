//! Committing the work tree: staging files while reviewing them (in git's
//! index, so the terminal sees the same), the commit view, commit messages
//! the AI agent writes in the Conventional Commits format, the commits the
//! AI review suggests (its groups), and pushing.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use diffr_agent::{CancelToken, CommitMessage};
use diffr_git::{Repo, Staged, Status};
use gpui::{AnyElement, Context, FontWeight, SharedString, Window, div, prelude::*, px};

use crate::sidebar::{status_color, status_letter};
use crate::state::*;
use crate::theme::alpha;
use crate::ui::{self, icon, pal, tooltip};
use crate::view::check_mark;
use crate::window::{Diffr, error_text};

/// The agent writing a commit message.
#[derive(Default)]
pub struct MessageGen {
    pub running: bool,
    pub progress: String,
    pub err: String,
    pub cancel: Option<CancelToken>,
}

/// A commit the AI review suggests: one of its groups, with the files of
/// it that are still changed.
#[derive(Clone)]
pub struct CommitGroup {
    /// The review group's key.
    pub key: String,
    pub label: String,
    pub summary: String,
    pub paths: Vec<String>,
    pub message: Option<CommitMessage>,
}

/// The pseudo-file that stands for the untracked files not listed.
fn listed(f: &FileState) -> bool {
    !(f.file.directory && f.file.path.starts_with("Untracked files not shown"))
}

/// The paths a file stages under: a rename's old path too.
fn stage_paths(f: &FileState) -> Vec<String> {
    let mut v = vec![f.file.path.clone()];
    if f.file.status == Status::Renamed && !f.file.old_path.is_empty() && f.file.old_path != f.file.path {
        v.push(f.file.old_path.clone());
    }
    v
}

impl Diffr {
    /// Whether files can be staged: the work tree's changes only.
    pub fn can_stage(&self) -> bool {
        self.source == Source::WorkingTree
    }

    /// How much of a file is staged.
    pub fn stage_of(&self, f: &FileState) -> Option<Staged> {
        if !listed(f) {
            return None;
        }
        if let Some(s) = self.staged.get(&f.file.path) {
            return Some(*s);
        }
        if f.file.directory {
            let prefix = format!("{}/", f.file.path);
            return self.staged.keys().any(|k| k.starts_with(&prefix)).then_some(Staged::Full);
        }
        None
    }

    /// The files staged, of those listed.
    pub fn staged_count(&self) -> usize {
        self.files.iter().filter(|f| self.stage_of(f).is_some()).count()
    }

    /// Stages a file, or unstages it when all of it is staged.
    pub fn toggle_stage(&mut self, fi: usize, cx: &mut Context<Self>) {
        let Some(f) = self.files.get(fi) else { return };
        let stage = self.stage_of(f) != Some(Staged::Full);
        self.set_staged(vec![fi], stage, cx);
    }

    /// Stages the current file, or unstages it (S).
    pub fn toggle_stage_current(&mut self, cx: &mut Context<Self>) {
        if !self.can_stage() || self.commit_open {
            return;
        }
        let i = self.current;
        if let Some(f) = self.files.get(i) {
            let staged = self.stage_of(f) != Some(Staged::Full);
            let name = f.file.name().to_string();
            self.toggle_stage(i, cx);
            self.toast(format!("{} {name}", if staged { "Staged" } else { "Unstaged" }), cx);
        }
    }

    /// Stages files (by index), or unstages them.
    pub fn set_staged(&mut self, files: Vec<usize>, stage: bool, cx: &mut Context<Self>) {
        if !self.can_stage() {
            return;
        }
        let mut paths = Vec::new();
        for fi in files {
            let Some(f) = self.files.get(fi) else { continue };
            if !listed(f) {
                continue;
            }
            paths.extend(stage_paths(f));
            // Shown at once; the index read afterwards has the last word.
            if stage {
                self.staged.insert(f.file.path.clone(), Staged::Full);
            } else {
                let prefix = format!("{}/", f.file.path);
                self.staged.retain(|k, _| k != &f.file.path && !k.starts_with(&prefix));
            }
        }
        if paths.is_empty() {
            return;
        }
        self.run_index(
            move |repo| if stage { repo.stage(&paths) } else { repo.unstage(&paths) },
            cx,
        );
    }

    /// Stages exactly these paths: those staged besides go back out.
    pub fn stage_only(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let keep: HashSet<&str> = paths.iter().map(String::as_str).collect();
        let mut add = Vec::new();
        for f in &self.files {
            if keep.contains(f.file.path.as_str()) && listed(f) {
                add.extend(stage_paths(f));
            }
        }
        let add_set: HashSet<String> = add.iter().cloned().collect();
        let drop: Vec<String> = self.staged.keys().filter(|k| !add_set.contains(*k)).cloned().collect();
        self.staged.retain(|k, _| add_set.contains(k));
        for p in &paths {
            self.staged.insert(p.clone(), Staged::Full);
        }
        self.run_index(
            move |repo| {
                repo.unstage(&drop)?;
                repo.stage(&add)
            },
            cx,
        );
    }

    /// Runs a change of the index in the background, one at a time, then
    /// reads the index again. The work tree's signature follows, so the
    /// change is not taken for an edit made outside the app.
    fn run_index(
        &mut self,
        op: impl FnOnce(&Repo) -> diffr_git::git::Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.staging += 1;
        self.commit_done.clear();
        self.commit_err.clear();
        let repo = self.repo.clone();
        let lock = self.index_lock.clone();
        cx.spawn(async move |this, cx| {
            let (res, staged, before, after) = cx
                .background_spawn(async move {
                    let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let before = repo.status_signature();
                    let res = op(&repo);
                    let staged = repo.index_status().ok();
                    (res, staged, before, repo.status_signature())
                })
                .await;
            this.update(cx, |this, cx| {
                this.staging -= 1;
                if let Err(e) = res {
                    let msg = error_text(&anyhow::anyhow!(e));
                    this.toast(format!("Staging failed: {}", msg.lines().next().unwrap_or("")), cx);
                }
                if this.staging == 0
                    && let Some(s) = staged
                {
                    this.staged = s;
                }
                if this.signature == before {
                    this.signature = after;
                }
                if this.staging == 0 && std::mem::take(&mut this.generate_after_stage) {
                    this.generate_message(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // ---- The commit view ----

    pub fn toggle_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Only the work tree's changes commit: a commit, a branch or a pull
        // request shows changes the work tree may not have.
        if !self.commit_open && self.source != Source::WorkingTree {
            return;
        }
        self.commit_open = !self.commit_open;
        if self.commit_open {
            self.commit_err.clear();
            self.commit_output.clear();
            self.commit_done.clear();
            self.refresh_index(cx);
            self.subject.update(cx, |i, cx| i.focus(window, cx));
        } else {
            window.focus(&self.list_focus, cx);
        }
        cx.notify();
    }

    /// Reads the index and the upstream again, for changes made outside.
    pub fn refresh_index(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let lock = self.index_lock.clone();
        cx.spawn(async move |this, cx| {
            let (staged, up) = cx
                .background_spawn(async move {
                    let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
                    (repo.index_status().ok(), repo.upstream())
                })
                .await;
            this.update(cx, |this, cx| {
                if this.staging == 0
                    && let Some(s) = staged
                {
                    this.staged = s;
                }
                this.upstream = up;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn do_commit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.commit_busy || self.staging > 0 || self.source != Source::WorkingTree {
            return;
        }
        let subject = self.subject.read(cx).text().trim().to_string();
        let body = self.body.read(cx).text().trim().to_string();
        let count = self.staged_count();
        if count == 0 && self.staged.is_empty() {
            self.commit_err = "Nothing is staged: check the files to commit.".into();
            cx.notify();
            return;
        }
        if subject.is_empty() {
            self.commit_err = "Write a subject, or let the AI write the message.".into();
            cx.notify();
            return;
        }
        let message = if body.is_empty() { format!("{subject}\n") } else { format!("{subject}\n\n{body}\n") };
        self.commit_busy = true;
        self.commit_err.clear();
        self.commit_output.clear();
        self.commit_done.clear();
        self.push_done.clear();
        let repo = self.repo.clone();
        let lock = self.index_lock.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
                    repo.commit_staged(&message)
                })
                .await;
            this.update(cx, |this, cx| {
                this.commit_busy = false;
                match res {
                    Ok(hash) => {
                        this.commit_done = hash;
                        this.committed_files = count.max(1);
                        this.subject.update(cx, |i, cx| i.set_text("", cx));
                        this.body.update(cx, |i, cx| i.set_text("", cx));
                        this.staged.clear();
                        this.msg_gen.err.clear();
                        // What is left to commit, and the commits to push.
                        this.load(cx);
                        this.load_history(cx);
                    }
                    Err(e) => {
                        this.commit_err = "The commit failed — see the output above.".into();
                        this.commit_output = error_text(&anyhow::anyhow!(e));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // ---- Pushing ----

    pub fn push(&mut self, cx: &mut Context<Self>) {
        if self.push_busy || self.commit_busy {
            return;
        }
        self.push_busy = true;
        self.commit_err.clear();
        self.commit_output.clear();
        self.push_done.clear();
        let ahead = self.upstream.ahead;
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let (res, up) = cx
                .background_spawn(async move {
                    let res = repo.push();
                    (res, repo.upstream())
                })
                .await;
            this.update(cx, |this, cx| {
                this.push_busy = false;
                let repo_name = crate::window::window_title(&this.root, &Source::WorkingTree);
                match res {
                    Ok(_) => {
                        let to = if up.name.is_empty() { up.branch.clone() } else { up.name.clone() };
                        let what = if ahead > 0 {
                            format!("Pushed {} to {to}", ui::plural(ahead, "commit"))
                        } else {
                            format!("Pushed {} to {to}", up.branch)
                        };
                        if !this.window_active {
                            crate::notify::notify(&what, &repo_name, Some(&this.root));
                        }
                        if this.commit_open {
                            this.commit_done.clear();
                            this.push_done = what;
                        } else {
                            this.toast(what, cx);
                        }
                    }
                    Err(e) => {
                        let text = error_text(&anyhow::anyhow!(e));
                        if !this.window_active {
                            crate::notify::notify("Push failed", text.lines().next().unwrap_or(""), Some(&this.root));
                        }
                        if this.commit_open {
                            this.commit_err = "The push failed — see the output above.".into();
                            this.commit_output = text;
                        } else {
                            this.toast(format!("Push failed: {}", text.lines().next().unwrap_or("")), cx);
                        }
                    }
                }
                this.upstream = up;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // ---- Messages by the agent ----

    /// What the review says the staged files are for: its groups that
    /// hold them.
    fn staged_intent(&self) -> String {
        let Some(a) = self.ext.analyses.get(&Source::WorkingTree).and_then(|a| a.result.as_ref()) else {
            return String::new();
        };
        let staged: HashSet<&str> = self.staged.keys().map(String::as_str).collect();
        a.groups
            .iter()
            .filter(|g| g.file_paths.iter().any(|p| staged.contains(p.as_str())))
            .map(|g| format!("- {}: {}", g.label, g.summary.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Asks the agent for the commit message of the selected files: those
    /// staged.
    pub fn generate_message(&mut self, cx: &mut Context<Self>) {
        if self.msg_gen.running || self.source != Source::WorkingTree {
            return;
        }
        if self.staging > 0 {
            self.generate_after_stage = true;
            return;
        }
        if self.staged.is_empty() {
            self.msg_gen.err = "Select the files to commit first.".into();
            cx.notify();
            return;
        }
        let mut req = self.message_request();
        req.intent = self.staged_intent();
        let repo = self.repo.clone();
        self.run_agent_job(
            |this| &mut this.msg_gen,
            "Commit message",
            move |agent, cancel, progress| {
                let files = repo.staged_files().map_err(|e| e.to_string())?;
                req.files = files.iter().map(crate::ai::agent_file).collect();
                diffr_agent::commit::message(agent, &req, cancel, progress).map_err(job_error)
            },
            |this, m, cx| this.fill_message(&m, cx),
            cx,
        );
    }

    /// Has the agent split all the changes into commits, with messages.
    pub fn make_commit_groups(&mut self, cx: &mut Context<Self>) {
        if self.plan_gen.running || self.source != Source::WorkingTree {
            return;
        }
        let mut req = self.message_request();
        req.files = self.files.iter().filter(|f| listed(f) && !f.file.directory).map(|f| crate::ai::agent_file(&f.file)).collect();
        if req.files.is_empty() {
            return;
        }
        self.run_agent_job(
            |this| &mut this.plan_gen,
            "Commit groups",
            move |agent, cancel, progress| diffr_agent::commit::plan(agent, &req, cancel, progress).map_err(job_error),
            |this, plan, cx| {
                this.commit_plan = Some(
                    plan.into_iter()
                        .enumerate()
                        .map(|(i, c)| CommitGroup {
                            key: format!("plan-{i}"),
                            summary: String::new(),
                            label: c.label,
                            paths: c.file_paths,
                            message: c.commit,
                        })
                        .collect(),
                );
                cx.notify();
            },
            cx,
        );
    }

    pub fn stop_commit_groups(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.plan_gen.cancel {
            c.cancel();
        }
        cx.notify();
    }

    /// What every request of the commit view starts from.
    fn message_request(&self) -> diffr_agent::MessageRequest {
        diffr_agent::MessageRequest {
            dir: PathBuf::from(&self.root),
            model: self.settings.ai_model.clone(),
            fast: self.use_fast(),
            branch: self.branch.clone(),
            recent: self.history.iter().take(12).map(|c| c.subject.clone()).collect(),
            ..Default::default()
        }
    }

    /// Runs a job of the agent in the background, its progress and outcome
    /// in the state `gen_` picks. Its outcome is posted as a notification,
    /// headed `what`, when the window is not active.
    fn run_agent_job<T: Send + 'static>(
        &mut self,
        gen_: fn(&mut Diffr) -> &mut MessageGen,
        what: &'static str,
        work: impl FnOnce(diffr_agent::Agent, &CancelToken, &mut dyn FnMut(&str)) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Diffr, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let name = self.settings.ai_agent.clone();
        let cancel = CancelToken::new();
        *gen_(self) = MessageGen {
            running: true,
            progress: "Starting…".into(),
            err: String::new(),
            cancel: Some(cancel.clone()),
        };
        let progress: Arc<Mutex<String>> = Default::default();
        let sink = progress.clone();
        let job = cx.background_spawn(async move {
            let agent = diffr_agent::pick(&name).map_err(|e| e.to_string())?;
            work(agent, &cancel, &mut |s: &str| *sink.lock().unwrap() = s.to_string())
        });
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let p = progress.lock().unwrap().clone();
                let running = this
                    .update(cx, |this, cx| {
                        let g = gen_(this);
                        if !p.is_empty() {
                            g.progress = p;
                        }
                        cx.notify();
                        g.running
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
                let g = gen_(this);
                g.running = false;
                g.cancel = None;
                if !this.window_active {
                    let repo_name = crate::window::window_title(&this.root, &Source::WorkingTree);
                    match &res {
                        Ok(_) => crate::notify::notify(&format!("{what} ready"), &repo_name, Some(&this.root)),
                        Err(e) if !e.is_empty() => crate::notify::notify(&format!("{what} failed"), e, Some(&this.root)),
                        Err(_) => {}
                    }
                }
                match res {
                    Ok(v) => done(this, v, cx),
                    Err(e) => gen_(this).err = e,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn stop_message(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.msg_gen.cancel {
            c.cancel();
        }
        cx.notify();
    }

    fn fill_message(&mut self, m: &CommitMessage, cx: &mut Context<Self>) {
        let (subject, body) = (m.subject(), m.body_text());
        self.subject.update(cx, |i, cx| i.set_text(subject, cx));
        self.body.update(cx, |i, cx| i.set_text(body, cx));
        self.commit_err.clear();
    }

    // ---- The commits the review suggests ----

    /// The commits suggested: those of "Make commit groups", else the
    /// groups of the review; with the files of each still changed.
    pub fn commit_groups(&self) -> Vec<CommitGroup> {
        let Some(plan) = &self.commit_plan else {
            return self.review_commit_groups();
        };
        let present: HashSet<&str> = self.files.iter().filter(|f| listed(f)).map(|f| f.file.path.as_str()).collect();
        plan.iter()
            .filter_map(|g| {
                let paths: Vec<String> = g.paths.iter().filter(|p| present.contains(p.as_str())).cloned().collect();
                (!paths.is_empty()).then(|| CommitGroup { paths, ..g.clone() })
            })
            .collect()
    }

    /// The groups of the review of the work tree, as commits, with the
    /// files of each still changed.
    pub fn review_commit_groups(&self) -> Vec<CommitGroup> {
        let Some(a) = self.ext.analyses.get(&Source::WorkingTree).and_then(|a| a.result.as_ref()) else {
            return Vec::new();
        };
        let present: HashSet<&str> = self.files.iter().filter(|f| listed(f)).map(|f| f.file.path.as_str()).collect();
        a.groups
            .iter()
            .filter_map(|g| {
                let paths: Vec<String> = g.file_paths.iter().filter(|p| present.contains(p.as_str())).cloned().collect();
                (!paths.is_empty()).then(|| CommitGroup {
                    key: g.key.clone(),
                    label: g.label.clone(),
                    summary: g.summary.clone(),
                    paths,
                    message: g.commit.clone(),
                })
            })
            .collect()
    }

    /// Stages a suggested commit alone, and takes its message (or has the
    /// agent write one).
    pub fn choose_commit_group(&mut self, g: CommitGroup, cx: &mut Context<Self>) {
        self.stage_only(g.paths.clone(), cx);
        match &g.message {
            Some(m) => self.fill_message(m, cx),
            None => {
                self.subject.update(cx, |i, cx| i.set_text("", cx));
                self.body.update(cx, |i, cx| i.set_text("", cx));
                self.generate_after_stage = true;
            }
        }
        cx.notify();
    }

    /// The state of a set of paths in the index: all, some or none staged.
    fn group_stage(&self, paths: &[String]) -> (bool, bool) {
        let n = paths.iter().filter(|p| self.staged.contains_key(*p)).count();
        let exact = n == paths.len() && self.staged.len() == n && paths.iter().all(|p| self.staged.get(p) == Some(&Staged::Full));
        (exact, n > 0)
    }

    pub fn commit_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let files: Vec<usize> = (0..self.files.len()).filter(|&i| listed(&self.files[i])).collect();
        let (mut chosen, mut adds, mut dels) = (0, 0, 0);
        let mut partial = 0;
        for &i in &files {
            let f = &self.files[i];
            match self.stage_of(f) {
                Some(Staged::Full) => chosen += 1,
                Some(Staged::Partial) => {
                    chosen += 1;
                    partial += 1;
                }
                None => {}
            }
            adds += f.file.additions;
            dels += f.file.deletions;
        }
        let subject_text = self.subject.read(cx).text().to_string();
        let writing = self.msg_gen.running || self.generate_after_stage;
        let can = !self.commit_busy && self.staging == 0 && !writing && chosen > 0 && !subject_text.trim().is_empty();
        let sfocus = self.subject.read(cx).is_focused(window);
        let bfocus = self.body.read(cx).is_focused(window);
        let ring = |on: bool, el: gpui::Div| {
            if on {
                el.border_color(alpha(p.accent, 0.6)).shadow(vec![gpui::BoxShadow {
                    color: alpha(p.accent, 0.2),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(3.),
                    inset: false,
                }])
            } else {
                el
            }
        };
        let n = subject_text.chars().count();
        let all = chosen == files.len() && !files.is_empty() && partial == 0;
        let mixed = chosen > 0 && !all;

        // ---- The files, staged or not ----
        let mut list = card(&p).p(px(4.)).gap(px(1.));
        if files.is_empty() {
            list = list.child(
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .text_size(px(13.))
                    .text_color(p.text_muted)
                    .child("No changes left to commit."),
            );
        } else {
            list = list.child(
                div()
                    .id("commit-all")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .hover(|s| s.bg(p.surface_hover))
                    .cursor_pointer()
                    .child(check_mark(all, mixed, cx))
                    .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child("All files"))
                    .tooltip(tooltip(if all { "Unstage every file" } else { "Stage every file" }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let ix: Vec<usize> = (0..this.files.len()).collect();
                        this.set_staged(ix, !all, cx);
                    })),
            );
        }
        for &i in &files {
            let f = &self.files[i];
            let path = f.file.path.clone();
            let st = self.stage_of(f);
            let on = st == Some(Staged::Full);
            let part = st == Some(Staged::Partial);
            let tok = diffr_fileicon::token(&path);
            let mut row = div()
                .id(SharedString::from(format!("cf-{path}")))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .pl(px(26.))
                .pr(px(8.))
                .py(px(5.))
                .rounded(px(6.))
                .hover(|s| s.bg(p.surface_hover))
                .cursor_pointer()
                .child(check_mark(on, part, cx))
                .child(
                    gpui::svg()
                        .path(SharedString::from(diffr_fileicon::icon_path(tok)))
                        .size(px(14.))
                        .flex_none()
                        .text_color(crate::theme::hex(diffr_fileicon::color(tok, p.dark))),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_w_0()
                        .font_family(code_font.clone())
                        .text_size(px(12.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(div().text_color(p.text_muted).child(f.file.dir().to_string()))
                        .child(div().text_color(p.text).child(f.file.name().to_string())),
                );
            if part {
                row = row.child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(alpha(p.reference, 0.15))
                        .text_color(p.reference)
                        .flex_none()
                        .child("Partly staged"),
                );
            }
            row = row
                .child(
                    div()
                        .font_family(code_font.clone())
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(status_color(f.file.status, &p))
                        .child(status_letter(f.file.status)),
                )
                .tooltip(tooltip(if part {
                    "Partly staged: only the staged part is committed. Click to stage all of it."
                } else if on {
                    "Staged: click to unstage"
                } else {
                    "Click to stage"
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_stage(i, cx)));
            if f.countable() {
                row = row.child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_none()
                        .font_family(code_font.clone())
                        .text_size(px(11.))
                        .child(div().text_color(p.add_text).child(format!("+{}", ui::thousands(f.file.additions))))
                        .child(div().text_color(p.del_text).child(format!(" −{}", ui::thousands(f.file.deletions)))),
                );
            }
            if st.is_none() {
                row = row.opacity(0.55);
            }
            list = list.child(row);
        }

        // ---- The header ----
        let mut head = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(20.))
            .py(px(10.))
            .gap(px(10.))
            .border_b_1()
            .border_color(p.card_border)
            .child(div().text_size(px(17.)).font_weight(FontWeight::BOLD).child("Commit"));
        if !self.branch.is_empty() {
            head = head.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(12.))
                    .bg(alpha(p.reference, 0.13))
                    .text_color(p.reference)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(icon("branch", 12., p.reference))
                    .child(self.branch.clone()),
            );
        }
        let up = &self.upstream;
        if !up.name.is_empty() && (up.ahead > 0 || up.behind > 0) {
            let mut t = String::new();
            if up.ahead > 0 {
                t.push_str(&format!("↑{}", up.ahead));
            }
            if up.behind > 0 {
                if !t.is_empty() {
                    t.push(' ');
                }
                t.push_str(&format!("↓{}", up.behind));
            }
            head = head.child(
                div()
                    .id("upstream")
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_muted)
                    .child(t)
                    .tooltip(tooltip(format!(
                        "{} ahead of {}, {} behind",
                        ui::plural(up.ahead, "commit"),
                        up.name,
                        up.behind
                    ))),
            );
        }
        head = head.child(ui::spacer()).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .text_size(px(12.))
                .child(div().text_color(p.text_muted).child(format!("{} · ", ui::plural(files.len(), "file"))))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .font_family(code_font.clone())
                        .child(div().text_color(p.add_text).child(format!("+{}", ui::thousands(adds))))
                        .child(div().text_color(p.del_text).child(format!(" −{}", ui::thousands(dels)))),
                ),
        );

        let mut form = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(760.))
            .mx_auto()
            .px(px(24.))
            .pt(px(20.))
            .pb(px(32.))
            .gap(px(8.));

        // ---- What the last commit or push did ----
        if !self.push_done.is_empty() {
            let left = files.len();
            let detail = if left > 0 {
                format!("{} still to commit", ui::plural(left, "file"))
            } else {
                "The work tree is clean".to_string()
            };
            form = form.child(banner("check_circle", p.viewed, self.push_done.clone(), detail.into_any_element(), &p).mb(px(8.)));
        } else if !self.commit_done.is_empty() {
            let left = files.len();
            let mut tail = format!(" · onto {}", self.branch);
            if left > 0 {
                tail.push_str(&format!(" · {} left to commit", ui::plural(left, "file")));
            }
            let detail = div()
                .flex()
                .flex_row()
                .child(div().font_family(code_font.clone()).child(self.commit_done.clone()))
                .child(tail)
                .into_any_element();
            let title = format!("Committed {}", ui::plural(self.committed_files, "file"));
            form = form.child(banner("check_circle", p.viewed, title, detail, &p).mb(px(8.)));
        }

        // ---- The commits the review suggests ----
        if !files.is_empty() {
            form = form.child(self.suggestions(cx).mb(px(8.)));
        }

        // ---- The message ----
        let mut msg_head = section_head("Message").gap(px(8.)).child(ui::spacer());
        if writing {
            let progress = if self.msg_gen.running { self.msg_gen.progress.clone() } else { "Staging the files…".to_string() };
            msg_head = msg_head.child(busy("msg-spin", progress, &p));
            if self.msg_gen.running {
                msg_head = msg_head.child(
                    small(ui::button("msg-stop", cx))
                        .child(icon("stop", 12., p.text))
                        .child("Stop")
                        .on_click(cx.listener(|this, _, _, cx| this.stop_message(cx))),
                );
            }
        } else {
            let mut gen_btn = small(ui::button("msg-gen", cx))
                .child(icon("sparkle", 12., p.accent))
                .child("Generate")
                .tooltip(tooltip(if chosen > 0 {
                    format!(
                        "{} writes a Conventional Commits message for the {} staged",
                        self.agent_hint(),
                        ui::plural(chosen, "file")
                    )
                } else {
                    "Stage files first: the message is written from them".to_string()
                }));
            gen_btn = if chosen > 0 {
                gen_btn.on_click(cx.listener(|this, _, _, cx| this.generate_message(cx)))
            } else {
                gen_btn.opacity(0.5)
            };
            msg_head = msg_head.child(gen_btn);
        }
        form = form.child(msg_head);
        if !self.msg_gen.err.is_empty() {
            form = form.child(div().text_size(px(12.)).text_color(p.danger_foreground).child(self.msg_gen.err.clone()));
        }
        let mut subject_box = ring(
            sfocus,
            card(&p)
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(9.))
                .text_size(px(14.))
                .line_height(px(20.))
                .font_weight(FontWeight::MEDIUM)
                .child(div().flex_1().min_w_0().child(self.subject.clone())),
        );
        if n > 0 {
            subject_box = subject_box.child(
                div()
                    .id("subject-len")
                    .flex_none()
                    .text_size(px(11.))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(if n > 72 { p.danger } else if n > 50 { p.reference } else { p.text_muted })
                    .child(format!("{n}/50"))
                    .tooltip(tooltip("Keep the subject to 50 characters; git cuts it at 72")),
            );
        }
        let mut body_box = ring(
            bfocus,
            card(&p)
                .min_h(px(96.))
                .px(px(12.))
                .py(px(9.))
                .text_size(px(13.))
                .line_height(px(19.))
                .child(self.body.clone()),
        );
        if writing {
            subject_box = subject_box.opacity(0.6);
            body_box = body_box.opacity(0.6);
        }
        let mut staged_head = section_head("Staged files").mt(px(12.)).gap(px(8.)).child(ui::spacer());
        if self.staging > 0 {
            staged_head = staged_head.child(busy("stage-spin", "Updating the index…", &p));
        } else {
            staged_head = staged_head.child(
                div().text_size(px(12.)).text_color(p.text_muted).child(format!("{chosen} of {} staged", files.len())),
            );
        }
        let form = form
            .child(subject_box)
            .child(section_head("Description").mt(px(4.)))
            .child(body_box)
            .child(staged_head)
            .child(list);
        let form = if self.commit_output.is_empty() {
            form
        } else {
            form.child(
                card(&p)
                    .mt(px(8.))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .pl(px(12.))
                            .pr(px(6.))
                            .py(px(4.))
                            .border_b_1()
                            .border_color(p.card_border)
                            .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child("Git output"))
                            .child(ui::icon_button("commit-output-close", "close", "Dismiss", cx).size(px(24.)).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.commit_output.clear();
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(10.))
                            .font_family(code_font.clone())
                            .text_size(px(12.))
                            .child(self.commit_output.clone()),
                    ),
            )
        };

        // ---- The footer ----
        let mut commit_btn = small(ui::primary_button("do-commit", cx));
        commit_btn = if self.commit_busy {
            commit_btn.child(ui::spinner("commit-spin", 13., p.accent_text)).child("Committing…")
        } else {
            commit_btn.child(icon("commit", 13., p.accent_text)).child(if chosen > 0 {
                format!("Commit {}", ui::plural(chosen, "file"))
            } else {
                "Commit".to_string()
            })
        };
        commit_btn = if can {
            commit_btn.on_click(cx.listener(|this, _, window, cx| this.do_commit(window, cx)))
        } else {
            commit_btn.opacity(0.5)
        };
        let up = &self.upstream;
        let remote = if up.name.is_empty() { "the remote".to_string() } else { up.name.clone() };
        let submit_keys = if cfg!(target_os = "macos") { "⌘↩ commits" } else { "Ctrl+↩ commits" };
        let (foot_text, spin, danger): (String, bool, bool) = if !self.commit_err.is_empty() {
            (self.commit_err.clone(), false, true)
        } else if self.commit_busy {
            ("Committing…".into(), true, false)
        } else if self.push_busy {
            (format!("Pushing to {remote}…"), true, false)
        } else if self.staging > 0 {
            ("Updating the index…".into(), true, false)
        } else if writing {
            (format!("{} is writing the message…", self.agent_hint()), true, false)
        } else if files.is_empty() {
            ("Nothing left to commit".into(), false, false)
        } else if chosen == 0 {
            ("Stage files to commit: here, or with S while reviewing".into(), false, false)
        } else if subject_text.trim().is_empty() {
            ("Write a subject, or generate the message".into(), false, false)
        } else {
            (submit_keys.into(), false, false)
        };
        let mut foot_msg = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .flex_1()
            .min_w_0()
            .text_size(px(12.))
            .text_color(if danger { p.danger_foreground } else { p.text_muted });
        if spin {
            foot_msg = foot_msg.child(ui::spinner("foot-spin", 12., p.text_muted));
        }
        foot_msg = foot_msg.child(foot_text);
        let mut foot = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(20.))
            .py(px(10.))
            .gap(px(8.))
            .border_t_1()
            .border_color(p.card_border)
            .child(foot_msg);
        if up.can_push() || self.push_busy {
            let mut b = small(ui::button("do-push", cx));
            b = if self.push_busy {
                b.child(ui::spinner("push-spin", 13., p.text)).child("Pushing…")
            } else {
                b.child(icon("arrow_up", 13., p.text)).child(if up.name.is_empty() {
                    "Publish branch".to_string()
                } else {
                    format!("Push {}", ui::plural(up.ahead, "commit"))
                })
            };
            b = b.tooltip(tooltip(if up.name.is_empty() {
                format!("Push {} and track it on the remote", up.branch)
            } else {
                format!("Push {} to {}", ui::plural(up.ahead, "commit"), up.name)
            }));
            b = if self.push_busy || self.commit_busy {
                b.opacity(0.6)
            } else {
                b.on_click(cx.listener(|this, _, _, cx| this.push(cx)))
            };
            foot = foot.child(b);
        }
        foot = foot.child(commit_btn);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .bg(p.app_bg)
            .child(head)
            .child(div().id("commit-scroll").flex_1().min_h_0().overflow_y_scroll().child(form))
            .child(foot)
            .into_any_element()
    }

    /// The commits suggested, and the button that has the agent make
    /// them.
    fn suggestions(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let p = pal(cx).clone();
        let groups = self.commit_groups();
        let review_running = self.ext.analyses.get(&Source::WorkingTree).is_some_and(|s| s.running);
        let mut head = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .pt(px(4.))
            .pb(px(2.))
            .child(icon("sparkle", 13., p.accent))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if groups.is_empty() { "Commit groups" } else { "Suggested commits" }),
            )
            .child(ui::spacer());
        if self.plan_gen.running {
            head = head.child(busy("plan-spin", self.plan_gen.progress.clone(), &p)).child(
                small(ui::button("plan-stop", cx))
                    .child(icon("stop", 12., p.text))
                    .child("Stop")
                    .on_click(cx.listener(|this, _, _, cx| this.stop_commit_groups(cx))),
            );
        } else if review_running && groups.is_empty() {
            head = head.child(busy("review-spin", "The AI review is running…", &p));
        } else {
            head = head.child(
                small(ui::button("make-groups", cx))
                    .child(icon("layers", 12., p.text))
                    .child(if groups.is_empty() { "Make groups" } else { "Regroup" })
                    .tooltip(tooltip(format!(
                        "{} splits all the changes into commits, each with a Conventional Commits message",
                        self.agent_hint()
                    )))
                    .on_click(cx.listener(|this, _, _, cx| this.make_commit_groups(cx))),
            );
        }
        let mut c = card(&p).p(px(4.)).gap(px(2.)).child(head);
        if !self.plan_gen.err.is_empty() {
            c = c.child(div().px(px(8.)).text_size(px(12.)).text_color(p.danger_foreground).child(self.plan_gen.err.clone()));
        }
        if groups.is_empty() {
            return c.child(div().px(px(8.)).pb(px(6.)).text_size(px(12.)).text_color(p.text_muted).child(if review_running {
                "Its groups show here when it is done."
            } else {
                "Split the changes into focused commits, each with its message; pick one to stage it alone."
            }));
        }
        let code_font = ui::code_font(cx);
        let any_message = groups.iter().any(|g| g.message.is_some());
        for (gi, g) in groups.into_iter().enumerate() {
            let (exact, some) = self.group_stage(&g.paths);
            let subject = g.message.as_ref().map(|m| m.subject()).unwrap_or_else(|| g.summary.lines().next().unwrap_or("").to_string());
            let files = ui::plural(g.paths.len(), "file");
            let tip = g.paths.join("\n");
            let g2 = g.clone();
            let mut row = div()
                .id(SharedString::from(format!("cg-{gi}")))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(6.))
                .hover(|s| s.bg(p.surface_hover))
                .cursor_pointer()
                .child(check_mark(exact, some && !exact, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(1.))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_baseline()
                                .gap(px(6.))
                                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(g.label.clone()))
                                .child(div().text_size(px(12.)).text_color(p.text_muted).child(files)),
                        )
                        .child(
                            div()
                                .font_family(code_font.clone())
                                .text_size(px(12.))
                                .text_color(p.text_muted)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(subject),
                        ),
                )
                .tooltip(tooltip(format!("Stage only these files and use this message:\n{tip}")))
                .on_click(cx.listener(move |this, _, _, cx| this.choose_commit_group(g2.clone(), cx)));
            if exact {
                row = row.bg(alpha(p.accent, 0.08));
            }
            c = c.child(row);
        }
        if !any_message {
            c = c.child(
                div()
                    .px(px(8.))
                    .pb(px(4.))
                    .text_size(px(12.))
                    .text_color(p.text_muted)
                    .child("These groups have no messages yet: picking one has the AI write its message."),
            );
        }
        c
    }
}

/// A rounded box of the commit view: the fields, the lists.
fn card(p: &crate::theme::Palette) -> gpui::Div {
    div().flex().flex_col().rounded(px(8.)).bg(p.code_bg).border_1().border_color(p.card_border)
}

/// The heading of a section of the commit view.
fn section_head(title: &'static str) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .min_h(px(26.))
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(title)
}

/// A button of the commit view: the size of its text.
fn small(b: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    b.text_size(px(13.)).px(px(10.)).py(px(4.)).gap(px(6.))
}

/// Work under way: a spinner and what is being done.
fn busy(id: &'static str, what: impl Into<SharedString>, p: &crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .min_w_0()
        .text_size(px(12.))
        .text_color(p.text_muted)
        .child(ui::spinner(id, 12., p.text_muted))
        .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(what.into()))
}

/// The outcome of a commit or a push.
fn banner(name: &str, color: gpui::Hsla, title: String, detail: AnyElement, p: &crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(alpha(color, 0.1))
        .border_1()
        .border_color(alpha(color, 0.25))
        .child(icon(name, 18., color))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_size(px(12.)).text_color(p.text_muted).child(detail)),
        )
}

/// An agent's error for the user; "" when the user stopped it.
fn job_error(e: diffr_agent::Error) -> String {
    match e {
        diffr_agent::Error::Canceled => String::new(),
        e => e.to_string(),
    }
}

/// The staged state of the files of a group in the sidebar: all, some.
pub fn tree_group_state(this: &Diffr, files: &[usize]) -> (bool, bool) {
    let staged = files.iter().filter(|&&i| this.files.get(i).is_some_and(|f| this.stage_of(f).is_some())).count();
    (staged == files.len() && !files.is_empty(), staged > 0)
}
