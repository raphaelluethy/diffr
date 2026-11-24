//! What shows over the window: the command bar, the dialogs opening a
//! commit, a branch or a pull request, the keyboard shortcuts, context
//! menus and toasts.

use gpui::{
    anchored, AnyElement, Context, FontWeight, MouseButton, Pixels, Point, SharedString, Window, deferred,
    div, prelude::*, px,
};

use crate::input::InputEvent;
use crate::state::*;
use crate::theme::rgba;
use crate::ui::{self, icon, pal};
use crate::window::Diffr;

#[derive(Clone, Debug)]
pub enum ContextMenu {
    Agent,
    Model,
    File(usize),
    TreeFile(usize),
    Commit(usize),
    Diagram(diffr_agent::Diagram, String),
    Line { file: usize, path: String, side: Side, line: u32, text: String },
}

type MenuAction = (String, bool, Box<dyn Fn(&mut Diffr, &mut Window, &mut Context<Diffr>)>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DialogKind {
    Recent,
    Commit,
    Branch,
    Pull,
}

/// A command of the command bar.
pub struct Command {
    pub title: &'static str,
    pub hint: String,
    pub keys: &'static str,
    pub run: fn(&mut Diffr, &mut Window, &mut Context<Diffr>),
}

fn cmd(title: &'static str, keys: &'static str, run: fn(&mut Diffr, &mut Window, &mut Context<Diffr>)) -> Command {
    Command { title, hint: String::new(), keys, run }
}

impl Diffr {
    pub fn commands(&self, cx: &gpui::App) -> Vec<Command> {
        let mut v = vec![
            cmd("Focus File Filter", "Ctrl+P", |this, window, cx| {
                this.tab = 0;
                this.sidebar_shown = true;
                this.filter.update(cx, |i, cx| i.focus(window, cx));
            }),
            cmd("Find in Diffs", "Ctrl+F", |this, window, cx| this.open_find(window, cx)),
            Command {
                hint: "Review a commit".into(),
                ..cmd("Open Commit", "", |this, window, cx| this.open_dialog(DialogKind::Commit, window, cx))
            },
            Command {
                hint: "Compare with a branch".into(),
                ..cmd("Open Branch", "", |this, window, cx| this.open_dialog(DialogKind::Branch, window, cx))
            },
            Command {
                hint: "Review a pull request on GitHub".into(),
                ..cmd("Open Pull Request", "", |this, window, cx| this.open_dialog(DialogKind::Pull, window, cx))
            },
            cmd("Open Folder", "Ctrl+O", |_, window, cx| window.dispatch_action(Box::new(crate::OpenFolder), cx)),
            cmd("Open Recent", "", |this, window, cx| this.open_dialog(DialogKind::Recent, window, cx)),
            cmd("Show File Tree", "Ctrl+1", |this, _, cx| this.set_tab(0, cx)),
            cmd("Show History", "Ctrl+2", |this, _, cx| this.set_tab(1, cx)),
            cmd("Show Pull Requests", "Ctrl+3", |this, window, cx| this.show_pulls(window, cx)),
            Command { hint: self.agent_hint(), ..cmd("Review with AI", "Ctrl+Shift+I", |this, _, cx| this.review_with_ai(cx)) },
            cmd("Stop the AI Review", "", |this, _, cx| this.stop_analysis(cx)),
            cmd("Next AI Note", "N", |this, _, cx| this.next_note(1, cx)),
            cmd("Previous AI Note", "Shift+N", |this, _, cx| this.next_note(-1, cx)),
            cmd("Group Files by Kind", "", |this, _, cx| this.choose_grouping(crate::features::GroupMode::Kind, cx)),
            cmd("Group Files by AI Review", "", |this, _, cx| this.choose_grouping(crate::features::GroupMode::Ai, cx)),
            cmd("Ungroup Files", "", |this, _, cx| this.choose_grouping(crate::features::GroupMode::None, cx)),
            cmd("Show Uncommitted Changes", "", |this, window, cx| {
                let s = this.launch_work_tree();
                this.set_source(s, window, cx)
            }),
        ];
        if self.source == Source::WorkingTree {
            v.push(cmd("Commit…", "Ctrl+Shift+M", |this, window, cx| {
                if !this.commit_open {
                    this.toggle_commit(window, cx)
                }
            }));
            if !self.staged.is_empty() {
                v.push(Command {
                    hint: "A Conventional Commits message for the staged changes".into(),
                    ..cmd("Write Commit Message with AI", "", |this, window, cx| {
                        if !this.commit_open {
                            this.toggle_commit(window, cx)
                        }
                        this.generate_message(cx)
                    })
                });
                v.push(cmd("Unstage All", "", |this, _, cx| {
                    let ix: Vec<usize> = (0..this.files.len()).collect();
                    this.set_staged(ix, false, cx)
                }));
            }
            v.push(cmd("Stage All", "", |this, _, cx| {
                let ix: Vec<usize> = (0..this.files.len()).collect();
                this.set_staged(ix, true, cx)
            }));
            v.push(Command {
                hint: self.files.get(self.current).map(|f| f.file.path.clone()).unwrap_or_default(),
                ..cmd("Stage or Unstage File", "S", |this, _, cx| this.toggle_stage_current(cx))
            });
            if self.upstream.can_push() {
                v.push(Command {
                    hint: if self.upstream.name.is_empty() {
                        format!("Publish {}", self.upstream.branch)
                    } else {
                        format!("{} to {}", ui::plural(self.upstream.ahead, "commit"), self.upstream.name)
                    },
                    ..cmd("Push", "", |this, _, cx| this.push(cx))
                });
            }
        }
        v.push(cmd("Copy Review Comments", "Ctrl+Shift+C", |this, _, cx| this.copy_comments(cx)));
        v.push(cmd("Show or Hide AI Notes", "Ctrl+Alt+N", |this, _, cx| this.toggle_ai_notes(cx)));
        if self.source.is_pull() {
            v.push(Command {
                hint: "Approve, comment or request changes".into(),
                ..cmd("Submit Review", "Ctrl+Shift+Enter", |this, _, cx| this.open_submit(cx))
            });
            v.push(cmd("Open Pull Request on GitHub", "", |this, _, cx| this.open_pull_on_github(cx)));
        }
        if self.all_pending_comments() > 0 {
            v.push(cmd("Copy Review Comments and Close", "", |this, window, cx| {
                // Every source's comments, then the window closes.
                this.finish_close(true, window, cx);
            }));
        }
        v.push(cmd("Next Unviewed File", "U", |this, _, cx| this.next_unviewed(cx)));
        let selected = self.files.get(self.current).map(|f| f.file.path.clone()).unwrap_or_default();
        v.push(Command {
            hint: selected.clone(),
            ..cmd("Toggle Viewed", "", |this, _, cx| {
                let i = this.current;
                if i < this.files.len() {
                    let v = this.is_viewed(&this.files[i]);
                    this.set_viewed(i, !v, cx);
                }
            })
        });
        v.push(Command { hint: selected.clone(), ..cmd("Open File in Editor", "Ctrl+Shift+O", |this, _, cx| this.open_current(cx)) });
        v.push(Command {
            hint: selected,
            ..cmd("Reveal File in File Manager", "Ctrl+Shift+R", |this, _, cx| {
                if let Some(p) = this.files.get(this.current).map(|f| f.file.path.clone()) {
                    this.reveal_in_file_manager(&p, cx);
                }
            })
        });
        v.push(cmd("Toggle Sidebar", "Ctrl+Shift+B", |this, _, cx| this.toggle_sidebar(cx)));
        v.push(cmd("Collapse All Files", "", |this, _, cx| this.set_all_collapsed(true, cx)));
        v.push(cmd("Expand All Files", "", |this, _, cx| this.set_all_collapsed(false, cx)));
        v.push(Command {
            hint: if self.split() { "Switch to Unified".into() } else { "Switch to Split".into() },
            ..cmd("Toggle Diff Layout", "", |this, _, cx| {
                let s = !this.split();
                this.set_split(s, cx)
            })
        });
        v.push(Command {
            hint: if self.settings.word_wrap { "Turn off".into() } else { "Turn on".into() },
            ..cmd("Toggle Word Wrap", "Alt+Z", |this, window, cx| {
                window.dispatch_action(Box::new(crate::actions::ToggleWrap), cx);
                let _ = this;
            })
        });
        v.push(Command {
            hint: if self.settings.show_whitespace { "Hide whitespace changes".into() } else { "Show whitespace changes".into() },
            ..cmd("Toggle Whitespace", "", |this, _, cx| {
                let ws = !this.settings.show_whitespace;
                let s = crate::config().update(|s| s.show_whitespace = ws);
                this.apply_settings(s, cx);
            })
        });
        v.push(cmd("Increase Code Font Size", "Ctrl++", |this, _, cx| this.change_font_size(1, cx)));
        v.push(cmd("Decrease Code Font Size", "Ctrl+-", |this, _, cx| this.change_font_size(-1, cx)));
        v.push(cmd("Reset Code Font Size", "Ctrl+0", |this, _, cx| this.change_font_size(0, cx)));
        v.push(cmd("Settings", "Ctrl+,", |this, _, cx| crate::settings::open_settings(this.root.clone(), cx)));
        v.push(Command {
            hint: "Notes, summaries, both or none".into(),
            ..cmd("Show AI Notes and Summaries", "", |this, _, _| {
                let next = match this.settings.ai_file_info.as_str() {
                    "notes" => "summaries",
                    "summaries" => "both",
                    "both" => "none",
                    _ => "notes",
                };
                let s = crate::config().update(|s| s.ai_file_info = next.to_string());
                this.settings.ai_file_info = s.ai_file_info;
                this.rows_dirty = true;
            })
        });
        v.push(cmd("Open Config File", "", |this, _, cx| {
            let path = crate::config().ensure();
            let p = path.to_string_lossy().to_string();
            this.open_in_editor_abs(&p, cx);
        }));
        v.push(cmd("Refresh Changes", "Ctrl+R", |this, _, cx| this.refresh(cx)));
        v.push(cmd("Install Command Line Tool", "", |_, _, cx| crate::report_install(cx)));
        v.push(cmd("Keyboard Shortcuts", "?", |this, _, _| this.help = true));
        let _ = cx;
        v
    }

    pub fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        self.palette_row = 0;
        self.palette_input.update(cx, |i, cx| i.set_text("", cx));
        if self.palette_open {
            self.palette_input.update(cx, |i, cx| i.focus(window, cx));
        } else {
            window.focus(&self.list_focus, cx);
        }
        cx.notify();
    }

    fn palette_matches(&self, cx: &gpui::App) -> Vec<Command> {
        let q = self.palette_input.read(cx).text().trim().to_string();
        self.commands(cx).into_iter().filter(|c| fuzzy_match(c.title, &q)).collect()
    }

    pub fn palette_event(&mut self, ev: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.palette_matches(cx).len();
        match ev {
            InputEvent::Changed => self.palette_row = 0,
            InputEvent::Down if n > 0 => self.palette_row = (self.palette_row + 1) % n,
            InputEvent::Up if n > 0 => self.palette_row = (self.palette_row + n - 1) % n,
            InputEvent::Submit => {
                self.run_command(self.palette_row, window, cx);
                return;
            }
            InputEvent::Cancel => {
                self.palette_open = false;
                window.focus(&self.list_focus, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    fn run_command(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let cmds = self.palette_matches(cx);
        let Some(c) = cmds.get(i) else { return };
        self.palette_open = false;
        window.focus(&self.list_focus, cx);
        (c.run)(self, window, cx);
        cx.notify();
    }

    pub fn open_dialog(&mut self, k: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        if k == DialogKind::Branch {
            // The local branches, the latest first.
            self.branches = self
                .repo
                .git(&["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads"])
                .map(|o| String::from_utf8_lossy(&o).lines().map(str::to_string).filter(|b| !b.is_empty()).collect())
                .unwrap_or_default();
        }
        self.dialog = Some(k);
        self.dialog_err.clear();
        self.dialog_busy = false;
        let ph = match k {
            DialogKind::Recent => "",
            DialogKind::Commit => "HEAD~1 or a commit SHA",
            DialogKind::Branch => "main",
            DialogKind::Pull => "123, owner/repo#123 or its URL",
        };
        self.dialog_input.update(cx, |i, cx| {
            i.set_placeholder(ph);
            i.set_text("", cx);
            i.focus(window, cx);
        });
        cx.notify();
    }

    pub fn dialog_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(k) = self.dialog else { return };
        let v = self.dialog_input.read(cx).text().trim().to_string();
        if v.is_empty() {
            return;
        }
        match k {
            DialogKind::Recent => {}
            DialogKind::Commit => match self.repo.resolve(&v) {
                Ok(hash) => {
                    self.dialog = None;
                    self.set_source(Source::Commit(hash), window, cx);
                }
                Err(_) => self.dialog_err = format!("No commit named {v}"),
            },
            DialogKind::Branch => {
                if self.repo.resolve(&v).is_err() {
                    self.dialog_err = format!("Branch \"{v}\" does not exist in this repository.");
                } else {
                    self.dialog = None;
                    self.set_source(Source::Branch(v), window, cx);
                }
            }
            DialogKind::Pull => match self.resolve_pull(&v) {
                Ok(src) => {
                    self.dialog = None;
                    self.set_source(src, window, cx);
                }
                Err(e) => self.dialog_err = e,
            },
        }
        cx.notify();
    }

    pub fn open_context_menu(&mut self, menu: ContextMenu, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.context_menu = Some((menu, at));
        cx.notify();
    }

    /// Everything over the window.
    pub fn overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(t) = &self.toast
            && t.1.elapsed() < std::time::Duration::from_secs(2)
        {
            let p = pal(cx);
            out.push(
                div()
                    .absolute()
                    .bottom(px(24.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px(px(14.))
                            .py(px(8.))
                            .rounded(px(10.))
                            .bg(if p.dark { crate::theme::hex(0x2c2c2e) } else { crate::theme::hex(0x1f1f22) })
                            .text_color(crate::theme::hex(0xffffff))
                            .text_size(px(13.))
                            .shadow_lg()
                            .child(t.0.clone()),
                    )
                    .into_any_element(),
            );
        }
        if let Some(m) = self.context_menu_view(cx) {
            out.push(m);
        }
        if let Some(m) = self.app_menu_view(window, cx) {
            out.push(m);
        }
        if self.palette_open {
            out.push(self.palette_view(window, cx));
        }
        if self.dialog.is_some() {
            out.push(self.dialog_view(window, cx));
        }
        if self.help {
            out.push(self.help_view(cx));
        }
        if self.discarding.is_some() {
            out.push(self.discard_view(cx));
        }
        if self.close_prompt {
            out.push(self.close_view(cx));
        }
        out.extend(self.pull_overlays(window, cx));
        out
    }

    fn context_menu_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (menu, at) = self.context_menu.clone()?;
        let p = pal(cx).clone();
        let mut items: Vec<MenuAction> = Vec::new();
        let mut seps = Vec::new();
        let mut headers = Vec::new();
        match menu {
            ContextMenu::Agent | ContextMenu::Model => {
                let model = matches!(menu, ContextMenu::Model);
                let list = if model { self.model_menu_items() } else { self.agent_menu_items() };
                for (label, checked, enabled, value) in list {
                    if label == "-" {
                        seps.push(items.len());
                        continue;
                    }
                    if let Some(h) = label.strip_prefix("# ") {
                        headers.push((items.len(), h.to_string()));
                        continue;
                    }
                    let shown = if checked { format!("✓ {label}") } else { format!("    {label}") };
                    match value {
                        Some((agent, _)) if enabled && agent == "#fast" => {
                            items.push((shown, true, Box::new(|t, _, cx| t.toggle_fast(cx))))
                        }
                        Some((agent, m)) if enabled => items.push((
                            shown,
                            true,
                            Box::new(move |t, _, cx| t.choose_agent(agent.clone(), m.clone(), model, cx)),
                        )),
                        _ => items.push((shown, false, Box::new(|_, _, _| {}))),
                    }
                }
            }
            ContextMenu::File(fi) => {
                let Some(f) = self.files.get(fi) else { return None };
                let path = f.file.path.clone();
                let viewed = self.is_viewed(f);
                let line = f.first_line();
                let deleted = f.file.status == diffr_git::Status::Deleted;
                let p1 = path.clone();
                items.push(("Open in Editor".into(), !deleted, Box::new(move |t, _, cx| t.open_in_editor(&p1, line, cx))));
                let pr = path.clone();
                items.push(("Reveal in File Manager".into(), true, Box::new(move |t, _, cx| t.reveal_in_file_manager(&pr, cx))));
                let p2 = path.clone();
                items.push((
                    "Copy Path".into(),
                    true,
                    Box::new(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(p2.clone()))),
                ));
                items.push((
                    format!("{}Viewed", if viewed { "✓ " } else { "" }),
                    true,
                    Box::new(move |t, _, cx| t.set_viewed(fi, !viewed, cx)),
                ));
                if self.can_stage() {
                    let staged = self.stage_of(f) == Some(diffr_git::Staged::Full);
                    items.push((
                        if staged { "Unstage".into() } else { "Stage".into() },
                        true,
                        Box::new(move |t, _, cx| t.set_staged(vec![fi], !staged, cx)),
                    ));
                }
                seps.push(items.len());
                items.push(("Collapse All".into(), true, Box::new(|t, _, cx| t.set_all_collapsed(true, cx))));
                items.push(("Expand All".into(), true, Box::new(|t, _, cx| t.set_all_collapsed(false, cx))));
            }
            ContextMenu::TreeFile(fi) => {
                let Some(f) = self.files.get(fi) else { return None };
                let path = f.file.path.clone();
                let viewed = self.is_viewed(f);
                let line = f.first_line();
                let deleted = f.file.status == diffr_git::Status::Deleted;
                let p1 = path.clone();
                items.push(("Open in Editor".into(), !deleted, Box::new(move |t, _, cx| t.open_in_editor(&p1, line, cx))));
                let p2 = path.clone();
                items.push(("Reveal in File Manager".into(), true, Box::new(move |t, _, cx| t.reveal_in_file_manager(&p2, cx))));
                let p3 = path.clone();
                items.push((
                    "Copy Path".into(),
                    true,
                    Box::new(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(p3.clone()))),
                ));
                seps.push(items.len());
                items.push((
                    format!("{}Viewed", if viewed { "✓ " } else { "" }),
                    true,
                    Box::new(move |t, _, cx| t.set_viewed(fi, !viewed, cx)),
                ));
                if self.can_stage() {
                    let staged = self.stage_of(f) == Some(diffr_git::Staged::Full);
                    items.push((
                        if staged { "Unstage".into() } else { "Stage".into() },
                        true,
                        Box::new(move |t, _, cx| t.set_staged(vec![fi], !staged, cx)),
                    ));
                }
            }
            ContextMenu::Diagram(d, title) => {
                let src = d.mermaid.clone();
                items.push((
                    "Copy Mermaid Source".into(),
                    true,
                    Box::new(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(src.clone()))),
                ));
                let d1 = d.clone();
                items.push(("Copy Image".into(), true, Box::new(move |_, _, cx| crate::diagrams::copy_image(&d1, cx))));
                seps.push(items.len());
                let open = crate::diagrams::window_open(&d.mermaid, cx);
                let d2 = d.clone();
                items.push((
                    if open { "Close Window".into() } else { "Open in Window".into() },
                    true,
                    Box::new(move |t, _, cx| {
                        crate::diagrams::toggle_window(title.clone(), d2.clone(), cx);
                        t.rows_dirty = true;
                    }),
                ));
                let folded = crate::diagrams::diagrams_folded();
                items.push((
                    if folded { "Show Diagrams".into() } else { "Fold Diagrams".into() },
                    true,
                    Box::new(move |t, _, _| {
                        crate::diagrams::set_diagrams_folded(!folded);
                        t.rows_dirty = true;
                    }),
                ));
            }
            ContextMenu::Commit(ci) => {
                let Some(c) = self.history.get(ci).cloned() else { return None };
                let hash = c.hash.clone();
                items.push((
                    "Copy Hash".into(),
                    true,
                    Box::new(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(hash.clone()))),
                ));
                let subject = c.subject.clone();
                items.push((
                    "Copy Subject".into(),
                    true,
                    Box::new(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(subject.clone()))),
                ));
            }
            ContextMenu::Line { file, path, side, line, text } => {
                let p1 = path.clone();
                items.push((
                    format!("Comment on Line {line}"),
                    true,
                    Box::new(move |t, window, cx| t.add_comment(&p1, side, line, line, window, cx)),
                ));
                items.push((
                    "Copy Line".into(),
                    true,
                    Box::new(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()))),
                ));
                if let Some(cid) = self
                    .comments
                    .list
                    .iter()
                    .find(|c| {
                        let first = if c.start == 0 { c.line } else { c.start.min(c.line) };
                        c.path == path && c.side == side && c.pending() && line >= first && line <= c.line
                    })
                    .map(|c| c.id)
                {
                    items.push(("Copy Comment with Lines".into(), true, Box::new(move |t, _, cx| t.copy_comment(cid, cx))));
                }
                let n = self.pending_comments();
                if n > 0 {
                    items.push((format!("Copy Review Comments ({n})"), true, Box::new(|t, _, cx| t.copy_comments(cx))));
                }
                seps.push(items.len());
                let deleted = self.files.get(file).is_some_and(|f| f.file.status == diffr_git::Status::Deleted);
                let p2 = path.clone();
                let l = if side == Side::Old { 0 } else { line };
                items.push(("Open in Editor".into(), !deleted, Box::new(move |t, _, cx| t.open_in_editor(&p2, l, cx))));
                let pr = path.clone();
                items.push(("Reveal in File Manager".into(), true, Box::new(move |t, _, cx| t.reveal_in_file_manager(&pr, cx))));
                let p3 = path.clone();
                items.push((
                    "Copy Path".into(),
                    true,
                    Box::new(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(p3.clone()))),
                ));
            }
        }
        let items = std::rc::Rc::new(items);
        let mut panel = div()
            .min_w(px(200.))
            .p(px(4.))
            .rounded(px(8.))
            .bg(p.background)
            .border_1()
            .border_color(p.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .max_h(px(420.))
            .id("context-panel")
            .overflow_y_scroll()
            .text_size(px(13.));
        for (i, (label, enabled, _)) in items.iter().enumerate() {
            if seps.contains(&i) {
                panel = panel.child(div().h(px(1.)).my(px(4.)).bg(p.border));
            }
            for (_, h) in headers.iter().filter(|h| h.0 == i) {
                panel = panel.child(
                    div().px(px(10.)).pt(px(6.)).pb(px(2.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_muted).child(h.clone()),
                );
            }
            let accent = p.accent;
            let at = p.accent_text;
            let mut row = div()
                .id(SharedString::from(format!("cm{i}")))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(5.))
                .child(label.clone());
            if *enabled {
                let items = items.clone();
                row = row.hover(move |s| s.bg(accent).text_color(at)).on_click(cx.listener(move |this, _, window, cx| {
                    this.context_menu = None;
                    (items[i].2)(this, window, cx);
                    cx.notify();
                }));
            } else {
                row = row.text_color(p.text_muted);
            }
            panel = panel.child(row);
        }
        Some(
            deferred(
                div()
                    .id("context-backdrop")
                    .absolute()
                    .size_full()
                    .top_0()
                    .left_0()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }))
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }))
                    .child(
                        anchored()
                            .position(at)
                            .snap_to_window_with_margin(px(8.))
                            .child(panel.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())),
                    ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn palette_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let cmds = self.palette_matches(cx);
        if self.palette_row >= cmds.len() {
            self.palette_row = cmds.len().saturating_sub(1);
        }
        let mut list = div().id("palette-list").p(px(6.)).max_h(px(380.)).overflow_y_scroll().flex().flex_col();
        if cmds.is_empty() {
            list = list.child(div().p(px(12.)).text_size(px(13.)).text_color(p.text_muted).child("No matching commands"));
        }
        for (i, c) in cmds.iter().enumerate() {
            let on = i == self.palette_row;
            let mut item = div()
                .id(SharedString::from(format!("pc{i}")))
                .min_h(px(34.))
                .px(px(10.))
                .py(px(6.))
                .gap(px(8.))
                .rounded(px(8.))
                .flex()
                .flex_row()
                .items_center()
                .cursor_pointer()
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.palette_row != i {
                        this.palette_row = i;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| this.run_command(i, window, cx)))
                .child(div().text_size(px(13.)).flex_none().child(c.title));
            if on {
                item = item.bg(p.accent).text_color(p.accent_text);
            }
            if !c.hint.is_empty() {
                item = item.child(
                    div()
                        .text_size(px(12.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .min_w_0()
                        .flex_shrink(1.)
                        .text_color(if on { p.accent_text } else { p.text_muted })
                        .child(c.hint.clone()),
                );
            }
            item = item.child(ui::spacer());
            if !c.keys.is_empty() {
                let keys = if cfg!(target_os = "macos") {
                    c.keys.replace("Ctrl+", "⌘").replace("Shift+", "⇧").replace("Alt+", "⌥")
                } else {
                    c.keys.to_string()
                };
                let k = div().text_size(px(11.)).px(px(6.)).py(px(2.)).rounded(px(5.)).flex_none().child(keys);
                item = item.child(if on { k.text_color(p.accent_text) } else { k.bg(p.surface).text_color(p.text_muted) });
            }
            list = list.child(item);
        }
        let _ = window;
        deferred(
            div()
                .id("palette-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(0, 0, 0, 0.12))
                .flex()
                .flex_col()
                .items_center()
                .pt(px(120.))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    this.palette_open = false;
                    window.focus(&this.list_focus, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .w(px(560.))
                        .max_h(px(440.))
                        .rounded(px(16.))
                        .bg(p.header_bg)
                        .border_1()
                        .border_color(p.card_border)
                        .shadow(vec![gpui::BoxShadow {
                            color: rgba(0, 0, 0, 0.28),
                            offset: gpui::point(px(0.), px(20.)),
                            blur_radius: px(60.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .px(px(14.))
                                .py(px(10.))
                                .gap(px(10.))
                                .border_b_1()
                                .border_color(p.card_border)
                                .text_size(px(15.))
                                .line_height(px(22.))
                                .child(icon("command", 16., p.text_muted))
                                .child(div().flex_1().child(self.palette_input.clone())),
                        )
                        .child(list),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn dialog_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let k = self.dialog.unwrap_or(DialogKind::Commit);
        if k == DialogKind::Recent {
            return self.recent_view(cx);
        }
        let (title, desc, label) = match k {
            DialogKind::Recent => unreachable!(),
            DialogKind::Commit => ("Open Commit", "Review a commit by SHA or revision, such as HEAD~1.", "Commit"),
            DialogKind::Branch => ("Open Branch", "Compare the current working tree with a branch.", "Branch name"),
            DialogKind::Pull => (
                "Open Pull Request",
                "Review a pull request of this repository on GitHub, open or not.",
                "Pull request",
            ),
        };
        let focused = self.dialog_input.read(cx).is_focused(window);
        let mut field = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(label))
            .child(
                div()
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .bg(p.background)
                    .border_1()
                    .border_color(if focused { p.accent } else { p.border })
                    .font_family(ui::code_font(cx))
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .child(self.dialog_input.clone()),
            );
        if !self.dialog_err.is_empty() {
            field = field.child(div().text_size(px(12.)).text_color(p.danger_foreground).child(self.dialog_err.clone()));
        }
        if k == DialogKind::Branch && !self.branches.is_empty() {
            let q = self.dialog_input.read(cx).text().trim().to_lowercase();
            let mut list = div()
                .id("branch-list")
                .max_h(px(180.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .rounded(px(6.))
                .border_1()
                .border_color(p.card_border)
                .p(px(4.));
            let mut any = false;
            for (i, b) in self.branches.iter().filter(|b| q.is_empty() || b.to_lowercase().contains(&q)).take(50).enumerate() {
                any = true;
                let name = b.clone();
                let hover = p.surface_hover;
                list = list.child(
                    div()
                        .id(SharedString::from(format!("br-{i}")))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .child(icon("branch", 12., p.text_muted))
                        .child(div().font_family(ui::code_font(cx)).text_size(px(12.)).child(b.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let n = name.clone();
                            this.dialog_input.update(cx, |i, cx| i.set_text(n, cx));
                            this.dialog_submit(window, cx);
                        })),
                );
            }
            if any {
                field = field
                    .child(div().mt(px(4.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_muted).child("Local branches"))
                    .child(list);
            }
        }
        modal(
            "dialog",
            420.,
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child(title))
                        .child(div().text_size(px(13.)).text_color(p.text_muted).child(desc)),
                )
                .child(field)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.))
                        .child(ui::button("dlg-cancel", cx).child("Cancel").on_click(cx.listener(|this, _, window, cx| {
                            this.dialog = None;
                            window.focus(&this.list_focus, cx);
                            cx.notify();
                        })))
                        .child(
                            ui::primary_button("dlg-open", cx)
                                .child(if self.dialog_busy { "Opening…" } else { "Open" })
                                .on_click(cx.listener(|this, _, window, cx| this.dialog_submit(window, cx))),
                        ),
                ),
            cx.listener(|this, _, window, cx| {
                this.dialog = None;
                window.focus(&this.list_focus, cx);
                cx.notify();
            }),
            cx,
        )
    }

    /// The menu bar, in the window: where the system draws no menus (Linux),
    /// the application's menus behind a button, as Zed draws its own.
    pub fn app_menu_button(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if cfg!(target_os = "macos") {
            return None;
        }
        let anchor = self.app_menu_anchor.clone();
        Some(
            div()
                .relative()
                .child(ui::icon_button("app-menu", "menu", "Menu (F10)", cx).on_click(cx.listener(
                    |this, ev: &gpui::ClickEvent, _, cx| {
                        this.app_menu = if this.app_menu.is_some() { None } else { Some(0) };
                        this.app_menu_item = None;
                        // The menu opens under the button that opened it.
                        let p = ev.position();
                        this.app_menu_at = gpui::point(p.x - px(14.), px(46.));
                        cx.notify();
                    },
                )))
                // Where the button is, for F10.
                .child(
                    gpui::canvas(
                        move |bounds, _, _| anchor.set(Some(gpui::point(bounds.origin.x, px(46.)))),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .into_any_element(),
        )
    }

    /// Runs the command of the menu bar chosen with the keyboard.
    pub fn run_app_menu_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.app_menu else { return };
        let Some((_, i)) = self.app_menu_item.filter(|(m, _)| *m == sel) else { return };
        let Some(action) = app_menu_commands(cx, sel).into_iter().nth(i) else { return };
        self.app_menu = None;
        self.app_menu_item = None;
        window.dispatch_action(action, cx);
        cx.notify();
    }

    fn app_menu_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sel = self.app_menu?;
        let menus = cx.get_menus()?;
        let p = pal(cx).clone();
        let mut bar = div().flex().flex_col().w(px(150.)).p(px(4.)).border_r_1().border_color(p.border);
        for (i, m) in menus.iter().enumerate() {
            let on = i == sel;
            let mut item = div()
                .id(SharedString::from(format!("am-{i}")))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(5.))
                .text_size(px(13.))
                .cursor_pointer()
                .child(m.name.clone())
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.app_menu != Some(i) {
                        this.app_menu = Some(i);
                        this.app_menu_item = None;
                        cx.notify();
                    }
                }));
            if on {
                item = item.bg(p.accent).text_color(p.accent_text);
            }
            bar = bar.child(item);
        }
        let mut list = div().id("am-items").flex().flex_col().w(px(300.)).max_h(px(560.)).overflow_y_scroll().p(px(4.));
        let chosen = self.app_menu_item.filter(|(m, _)| *m == sel).map(|(_, i)| i);
        let mut count = 0usize;
        #[allow(clippy::too_many_arguments)]
        fn items(
            list: gpui::Stateful<gpui::Div>,
            entries: &[gpui::OwnedMenuItem],
            depth: usize,
            key: &str,
            p: &crate::theme::Palette,
            chosen: Option<usize>,
            count: &mut usize,
            window: &mut Window,
            cx: &mut Context<Diffr>,
        ) -> gpui::Stateful<gpui::Div> {
            let mut list = list;
            for (j, e) in entries.iter().enumerate() {
                match e {
                    gpui::OwnedMenuItem::Separator => list = list.child(div().h(px(1.)).my(px(4.)).bg(p.border)),
                    gpui::OwnedMenuItem::Submenu(m) => {
                        list = list.child(
                            div()
                                .pl(px(10. + depth as f32 * 12.))
                                .pt(px(6.))
                                .pb(px(2.))
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.text_muted)
                                .child(m.name.clone()),
                        );
                        list = items(list, &m.items, depth + 1, &format!("{key}-{j}"), p, chosen, count, window, cx);
                    }
                    gpui::OwnedMenuItem::SystemMenu(_) => {}
                    gpui::OwnedMenuItem::Action { name, action, checked, disabled, .. } => {
                        let keys = window
                            .highest_precedence_binding_for_action(action.as_ref())
                            .map(|b| b.keystrokes().iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" "))
                            .unwrap_or_default();
                        let action = action.boxed_clone();
                        let (accent, at) = (p.accent, p.accent_text);
                        let mut row = div()
                            .id(SharedString::from(format!("{key}-{j}")))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .pl(px(10. + depth as f32 * 12.))
                            .pr(px(10.))
                            .py(px(4.))
                            .rounded(px(5.))
                            .text_size(px(13.))
                            .child(div().w(px(12.)).flex_none().child(if *checked { "✓" } else { "" }))
                            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name.clone()))
                            .child(div().text_size(px(11.)).text_color(p.text_muted).child(keys));
                        if *disabled {
                            row = row.text_color(p.text_muted);
                        } else {
                            if chosen == Some(*count) {
                                row = row.bg(accent).text_color(at);
                            }
                            *count += 1;
                            row = row.cursor_pointer().hover(move |s| s.bg(accent).text_color(at)).on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.app_menu = None;
                                    window.dispatch_action(action.boxed_clone(), cx);
                                    cx.notify();
                                },
                            ));
                        }
                        list = list.child(row);
                    }
                }
            }
            list
        }
        if let Some(m) = menus.get(sel) {
            list = items(list, &m.items, 0, &format!("am{sel}"), &p, chosen, &mut count, window, cx);
        }
        let panel = div()
            .flex()
            .flex_row()
            .rounded(px(8.))
            .bg(p.background)
            .border_1()
            .border_color(p.border)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(bar)
            .child(list);
        Some(
            deferred(
                div()
                    .id("app-menu-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                        this.app_menu = None;
                        cx.notify();
                    }))
                    .child(anchored().position(self.app_menu_at).snap_to_window_with_margin(px(8.)).child(panel)),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    /// Asks before closing with review comments, which are not saved.
    fn close_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let n = self.all_pending_comments();
        modal(
            "close",
            420.,
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(format!(
                            "Close with {}?",
                            ui::plural(n, "review comment")
                        )))
                        .child(div().text_size(px(13.)).text_color(p.text_muted).child(
                            "Comments are not saved. Copy them as Markdown to keep them.",
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .child(ui::button("close-anyway", cx).child("Close").on_click(cx.listener(|this, _, window, cx| {
                            this.finish_close(false, window, cx);
                        })))
                        .child(ui::spacer())
                        .child(ui::button("close-cancel", cx).child("Cancel").on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_close(cx);
                        })))
                        .child(ui::primary_button("close-copy", cx).child("Copy and Close").on_click(cx.listener(
                            |this, _, window, cx| {
                                this.finish_close(true, window, cx);
                            },
                        ))),
                ),
            cx.listener(|this, _, _, cx| this.cancel_close(cx)),
            cx,
        )
    }

    /// Asks before a review comment's text is lost.
    fn discard_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        modal(
            "discard",
            360.,
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child("Discard this review comment?"))
                        .child(div().text_size(px(13.)).text_color(p.text_muted).child("Its text will be lost.")),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.))
                        .child(ui::button("discard-cancel", cx).child("Cancel").on_click(cx.listener(|this, _, window, cx| {
                            let id = this.discarding.take();
                            if let Some(i) = id.and_then(|id| this.comment_inputs.get(&id).cloned()) {
                                i.update(cx, |i, cx| i.focus(window, cx));
                            }
                            cx.notify();
                        })))
                        .child(
                            ui::primary_button("discard-ok", cx)
                                .bg(p.danger)
                                .child("Discard")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(id) = this.discarding.take() {
                                        this.delete_comment(id, cx);
                                    }
                                    window.focus(&this.list_focus, cx);
                                    cx.notify();
                                })),
                        ),
                ),
            cx.listener(|this, _, _, cx| {
                this.discarding = None;
                cx.notify();
            }),
            cx,
        )
    }

    /// The repositories opened lately.
    fn recent_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let recent = self.store.recent();
        let mut list = div().flex().flex_col().gap(px(2.));
        if recent.is_empty() {
            list = list.child(div().text_size(px(13.)).text_color(p.text_muted).child("No repositories opened lately"));
        }
        for (i, r) in recent.into_iter().enumerate() {
            let hover = p.surface_hover;
            let name = std::path::Path::new(&r).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            list = list.child(
                div()
                    .id(SharedString::from(format!("recent{i}")))
                    .flex()
                    .flex_col()
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(div().text_size(px(11.)).text_color(p.text_muted).child(abbreviate_home(&r)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dialog = None;
                        crate::open_repo(crate::Request { dir: std::path::PathBuf::from(&r), src: Source::WorkingTree }, cx);
                    })),
            );
        }
        modal(
            "recent",
            460.,
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child("Open Recent"))
                .child(list)
                .child(div().flex().flex_row().justify_end().child(ui::button("recent-done", cx).child("Cancel").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.dialog = None;
                        cx.notify();
                    }),
                ))),
            cx.listener(|this, _, _, cx| {
                this.dialog = None;
                cx.notify();
            }),
            cx,
        )
    }

    fn help_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let m = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
        let alt = if cfg!(target_os = "macos") { "⌥" } else { "Alt+" };
        let sh = if cfg!(target_os = "macos") { "⇧" } else { "Shift+" };
        let k = |s: &str| s.replace("M-", m).replace("A-", alt).replace("S-", sh);
        let groups: Vec<(&str, Vec<(&str, String)>)> = vec![
            (
                "Move",
                vec![
                    ("Next / previous hunk", k("J / K")),
                    ("Next / previous file", k("S-J S-K  ] [")),
                    ("Next / previous group", "} {".into()),
                    ("Top / end", k("g g / S-G")),
                    ("Half a page down / up", "Ctrl+D / Ctrl+U".into()),
                    ("Page down / up", if cfg!(target_os = "macos") { "⌃F ⌃B Space".into() } else { "Space / S-Space".replace("S-", sh) }),
                    ("Next / previous AI note", k("N / S-N")),
                ],
            ),
            (
                "Files",
                vec![
                    ("Fold the file", "O, X".into()),
                    ("Mark viewed, go to the next", "V".into()),
                    ("Filter files", k("M-P")),
                    ("Toggle sidebar", k("M-S-B")),
                    ("Open in editor", k("M-S-O")),
                    ("Reveal in file manager", k("M-S-R")),
                ],
            ),
            (
                "Find",
                vec![
                    ("Find in diffs", k("M-F, /")),
                    ("Next / previous match", k("M-G / M-S-G")),
                    ("Command bar", k("M-K")),
                ],
            ),
            (
                "Comments",
                vec![
                    ("Comment on a line", "Click".into()),
                    ("Comment on lines", "Drag".into()),
                    ("Comment on the hunk", "↩".into()),
                    ("Add the comment", k("M-↩")),
                    ("Copy the comments", k("M-S-C")),
                    ("Discard a comment", "Esc".into()),
                ],
            ),
            (
                "Review",
                vec![
                    ("Review with AI", k("M-S-I")),
                    ("Show or hide AI notes", k("M-A-N")),
                    ("Pull requests", k("M-3")),
                    ("Submit the review", k("M-S-↩")),
                    ("Commit", k("M-S-M")),
                    ("Stage or unstage the file", "S".into()),
                    ("Refresh", k("M-R")),
                ],
            ),
            (
                "Code",
                vec![
                    ("Bigger / smaller text", k("M-= / M--")),
                    ("Actual size", k("M-0")),
                    ("Word wrap", k("A-Z")),
                    ("All the shortcuts", "?".into()),
                ],
            ),
        ];
        let col = |g: &(&str, Vec<(&str, String)>)| {
            let mut c = div().flex().flex_col().gap(px(4.)).child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text_muted)
                    .child(g.0.to_uppercase()),
            );
            for (name, keys) in g.1.iter() {
                c = c.child(
                    div()
                        .min_h(px(26.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(div().flex_1().text_size(px(13.)).child(name.to_string()))
                        .child(div().text_size(px(11.)).px(px(6.)).py(px(2.)).rounded(px(5.)).bg(p.surface).child(keys.to_string())),
                );
            }
            c
        };
        let left = div().flex().flex_col().flex_1().gap(px(14.)).child(col(&groups[0])).child(col(&groups[2])).child(col(&groups[4]));
        let right = div().flex().flex_col().flex_1().gap(px(14.)).child(col(&groups[1])).child(col(&groups[3])).child(col(&groups[5]));
        modal(
            "help",
            620.,
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(div().flex_1().text_size(px(15.)).font_weight(FontWeight::BOLD).child("Keyboard Shortcuts"))
                        .child(ui::button("help-done", cx).child("Done").on_click(cx.listener(|this, _, _, cx| {
                            this.help = false;
                            cx.notify();
                        }))),
                )
                .child(div().flex().flex_row().gap(px(28.)).child(left).child(right)),
            cx.listener(|this, _, _, cx| {
                this.help = false;
                cx.notify();
            }),
            cx,
        )
    }
}

/// A dialog over a dimmed window, closed by a click outside it.
pub fn modal(
    id: &str,
    width: f32,
    content: impl IntoElement,
    on_close: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    cx: &gpui::App,
) -> AnyElement {
    let p = pal(cx);
    deferred(
        div()
            .id(SharedString::from(format!("{id}-backdrop")))
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgba(0, 0, 0, 0.25))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, on_close)
            .child(
                div()
                    .w(px(width))
                    .p(px(20.))
                    .rounded(px(12.))
                    .bg(p.background)
                    .border_1()
                    .border_color(p.card_border)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(content),
            ),
    )
    .with_priority(2)
    .into_any_element()
}

/// The commands of a menu of the menu bar that can run, in the order they
/// show (submenus' inline), for the keyboard to go through.
pub fn app_menu_commands(cx: &gpui::App, menu: usize) -> Vec<Box<dyn gpui::Action>> {
    fn walk(entries: &[gpui::OwnedMenuItem], out: &mut Vec<Box<dyn gpui::Action>>) {
        for e in entries {
            match e {
                gpui::OwnedMenuItem::Submenu(m) => walk(&m.items, out),
                gpui::OwnedMenuItem::Action { action, disabled: false, .. } => out.push(action.boxed_clone()),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    if let Some(m) = cx.get_menus().and_then(|ms| ms.into_iter().nth(menu)) {
        walk(&m.items, &mut out);
    }
    out
}
