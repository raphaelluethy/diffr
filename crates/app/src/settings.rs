//! The Settings window: General, Review, AI, Labs and GitHub. It writes
//! the settings file, which every window watches.

use std::sync::{Arc, OnceLock};

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Entity, FocusHandle, FontWeight, Render, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, div, prelude::*, px, size,
};

use crate::input::{InputEvent, TextInput};
use crate::theme::{alpha, rgba};
use crate::ui::{self, icon, pal, tooltip};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Review,
    Ai,
    Labs,
    GitHub,
}

const TABS: [(Tab, &str, &str); 5] = [
    (Tab::General, "General", "command"),
    (Tab::Review, "Review", "file_diff"),
    (Tab::Ai, "AI", "sparkle"),
    (Tab::Labs, "Labs", "layers"),
    (Tab::GitHub, "GitHub", "pull"),
];

// Discover only on demand, and retain the result across Settings windows.
static MONOSPACE_FONTS: OnceLock<Arc<[String]>> = OnceLock::new();

#[derive(Default)]
enum FontChoices {
    #[default]
    Unloaded,
    Loading,
    Ready(Arc<[String]>),
}

impl FontChoices {
    fn start_loading(&mut self) -> bool {
        if !matches!(self, Self::Unloaded) {
            return false;
        }
        *self = Self::Loading;
        true
    }
}

fn monospace_fonts(names: Vec<String>) -> Arc<[String]> {
    let mut names: Vec<_> = names
        .into_iter()
        .filter(|name| {
            let lower = name.to_lowercase();
            ["mono", "code", "courier", "menlo", "monaco", "consolas", "hack", "iosevka", "fira", "jetbrains", "inconsolata"]
                .iter()
                .any(|part| lower.contains(part))
        })
        .collect();
    names.sort();
    names.dedup();
    names.into()
}

#[derive(PartialEq, Eq)]
struct SettingsSnapshot {
    settings: diffr_git::Settings,
    agents: Option<Vec<diffr_agent::Agent>>,
}

impl SettingsSnapshot {
    fn refresh(&mut self, next: Self) -> bool {
        if *self == next {
            return false;
        }
        *self = next;
        true
    }
}

pub struct SettingsView {
    tab: Tab,
    focus: FocusHandle,
    root: String,
    code_font: Entity<TextInput>,
    editor: Entity<TextInput>,
    prefix: Entity<TextInput>,
    model: Entity<TextInput>,
    token: Entity<TextInput>,
    github_user: Option<Result<String, String>>,
    font_menu: bool,
    fonts: FontChoices,
    show_instructions: bool,
    open_labs: Vec<&'static str>,
    _subs: Vec<gpui::Subscription>,
}

static mut SETTINGS_WINDOW: Option<WindowHandle<SettingsView>> = None;

/// Opens the Settings window, or brings it forward.
pub fn open_settings(root: String, cx: &mut App) {
    // SAFETY: only touched on the main thread.
    #[allow(static_mut_refs)]
    if let Some(h) = unsafe { SETTINGS_WINDOW.as_ref() }
        && h.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(780.), px(580.)), cx);
    let h = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Diffr Settings".into()), ..Default::default() }),
            window_min_size: Some(size(px(640.), px(420.))),
            app_id: Some("dev.diffr".into()),
            ..Default::default()
        },
        |window, cx| {
            crate::apply_theme(window, cx);
            cx.new(|cx| SettingsView::new(root, window, cx))
        },
    );
    if let Ok(h) = h {
        unsafe { SETTINGS_WINDOW = Some(h) };
    }
}

fn text_field(cx: &mut Context<SettingsView>, placeholder: &str, value: &str) -> Entity<TextInput> {
    let i = crate::window::new_input(cx, placeholder, false);
    i.update(cx, |i, cx| i.set_text(value.to_string(), cx));
    i
}

impl SettingsView {
    fn new(root: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = crate::config().get();
        let code_font = text_field(cx, crate::DEFAULT_CODE_FONT, &s.code_font_family);
        let editor = text_field(cx, "zed {file}:{line}", &s.editor_command);
        let prefix = crate::window::new_input(cx, "# Address these Review Comments", true);
        prefix.update(cx, |i, cx| {
            i.min_lines = 2;
            i.set_text(s.review_comments_prefix.clone(), cx)
        });
        let model = text_field(cx, "The agent's default", &s.ai_model);
        let token = text_field(cx, "The GitHub CLI's (gh auth login)", &s.github_token);
        token.update(cx, |i, _| i.masked = true);
        let mut subs = Vec::new();
        let bind = |input: &Entity<TextInput>, f: fn(&mut diffr_git::Settings, String), window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(input, window, move |_, inp, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Changed) {
                    let v = inp.read(cx).text().to_string();
                    crate::config().update(|s| f(s, v));
                }
            })
        };
        subs.push(bind(&code_font, |s, v| s.code_font_family = v, window, cx));
        subs.push(bind(&editor, |s, v| s.editor_command = v, window, cx));
        subs.push(bind(&prefix, |s, v| s.review_comments_prefix = v, window, cx));
        subs.push(bind(&model, |s, v| s.ai_model = v, window, cx));
        subs.push(bind(&token, |s, v| s.github_token = v, window, cx));
        subs.push(cx.observe_window_appearance(window, |_, window, cx| {
            crate::apply_theme(window, cx);
            cx.notify();
        }));
        let mut this = Self {
            tab: Tab::General,
            focus: cx.focus_handle(),
            root,
            code_font,
            editor,
            prefix,
            model,
            token,
            github_user: None,
            font_menu: false,
            fonts: FontChoices::default(),
            show_instructions: false,
            open_labs: Vec::new(),
            _subs: subs,
        };
        this.check_github(cx);
        // The settings change elsewhere too: the menus, the file.
        let mut snapshot = SettingsSnapshot { settings: s, agents: crate::ai::installed_agents() };
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                let ok = this.update_in(cx, |this, window, cx| {
                    let next = SettingsSnapshot { settings: crate::config().get(), agents: crate::ai::installed_agents() };
                    // Check even without a config change: a field may have
                    // been focused when an external change first arrived.
                    this.sync_fields(&next.settings, window, cx);
                    if snapshot.refresh(next) {
                        cx.notify();
                    }
                });
                if ok.is_err() {
                    return;
                }
            }
        })
        .detach();
        this
    }

    /// The text fields show the settings as they are now, unless one is
    /// being edited: the file and the menus change them too.
    fn sync_fields(&mut self, s: &diffr_git::Settings, window: &mut Window, cx: &mut Context<Self>) {
        for (input, value) in [
            (&self.code_font, &s.code_font_family),
            (&self.editor, &s.editor_command),
            (&self.prefix, &s.review_comments_prefix),
            (&self.model, &s.ai_model),
            (&self.token, &s.github_token),
        ] {
            let stale = { let i = input.read(cx); !i.is_focused(window) && i.text() != value.as_str() };
            if stale {
                input.update(cx, |i, cx| i.set_text(value.clone(), cx));
            }
        }
    }

    fn load_fonts(&mut self, cx: &mut Context<Self>) {
        if !self.fonts.start_loading() {
            return;
        }
        if let Some(names) = MONOSPACE_FONTS.get() {
            self.fonts = FontChoices::Ready(names.clone());
            return;
        }
        // App's shared TextSystem is Send + Sync; keep enumeration and
        // filtering off the render/event thread, including the first open.
        let text_system = cx.text_system().clone();
        cx.spawn(async move |this, cx| {
            let names = cx
                .background_spawn(async move {
                    MONOSPACE_FONTS.get_or_init(|| monospace_fonts(text_system.all_font_names())).clone()
                })
                .await;
            this.update(cx, |this, cx| {
                this.fonts = FontChoices::Ready(names);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn check_github(&mut self, cx: &mut Context<Self>) {
        let explicit = crate::config().get().github_token;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let token = if explicit.trim().is_empty() { diffr_github::find_token() } else { explicit };
                    if token.is_empty() {
                        return Err("Not signed in: run gh auth login, or paste a token above.".to_string());
                    }
                    crate::pull::client(token).viewer().map(|u| u.login).map_err(|e| e.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.github_user = Some(res);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// A disclosure: a title that shows or hides a monospaced text, which a
/// button copies.
fn disclosure(
    id: impl Into<SharedString>,
    title: &str,
    open: bool,
    text: String,
    cx: &mut Context<SettingsView>,
    toggle: impl Fn(&mut SettingsView) + 'static,
) -> AnyElement {
    let p = pal(cx).clone();
    let id: SharedString = id.into();
    let copy_text = text.clone();
    let mut col = div().flex().flex_col().gap(px(6.)).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-toggle")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(icon("chevron_down", 12., p.text_muted).with_transformation(gpui::Transformation::rotate(gpui::radians(
                        if open { 0. } else { -std::f32::consts::FRAC_PI_2 },
                    ))))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle(this);
                        cx.notify();
                    })),
            )
            .child(ui::spacer())
            .children(open.then(|| {
                ui::button(SharedString::from(format!("{id}-copy")), cx)
                    .px(px(8.))
                    .py(px(1.))
                    .text_size(px(12.))
                    .child("Copy")
                    .on_click(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_text.clone())))
            })),
    );
    if open {
        col = col.child(
            div()
                .id(SharedString::from(format!("{id}-text")))
                .max_h(px(320.))
                .overflow_y_scroll()
                .px(px(12.))
                .py(px(10.))
                .rounded(px(8.))
                .bg(p.code_bg)
                .border_1()
                .border_color(p.card_border)
                .font_family(ui::code_font(cx))
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(alpha(p.text, 0.85))
                .child(text),
        );
    }
    col.into_any_element()
}

/// A row of a form: a label and its description on the left, a control on
/// the right.
fn row(label: &str, detail: &str, control: impl IntoElement, cx: &App) -> AnyElement {
    let p = pal(cx);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .px(px(14.))
        .py(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(label.to_string()))
                .children((!detail.is_empty()).then(|| div().text_size(px(12.)).text_color(p.text_muted).child(detail.to_string()))),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// A group of rows in a card.
fn section(title: &str, rows: Vec<AnyElement>, cx: &App) -> AnyElement {
    let p = pal(cx);
    let mut card = div().flex().flex_col().rounded(px(12.)).bg(p.code_bg).border_1().border_color(p.card_border);
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        card = card.child(r);
        if i + 1 < n {
            card = card.child(div().h(px(1.)).mx(px(14.)).bg(p.card_border));
        }
    }
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text_muted)
                .child(title.to_uppercase()),
        )
        .child(card)
        .into_any_element()
}

/// A switch, on or off.
fn switch(id: &'static str, on: bool, cx: &mut Context<SettingsView>, f: fn(&mut diffr_git::Settings, bool)) -> AnyElement {
    let p = pal(cx).clone();
    let knob = div().size(px(16.)).rounded_full().bg(gpui::white()).shadow_sm();
    div()
        .id(id)
        .w(px(34.))
        .h(px(20.))
        .rounded(px(10.))
        .p(px(2.))
        .flex()
        .flex_row()
        .when(on, |d| d.justify_end())
        .bg(if on { p.accent } else { p.border })
        .cursor_pointer()
        .child(knob)
        .on_click(cx.listener(move |_, _, _, cx| {
            crate::config().update(|s| f(s, !on));
            cx.notify();
        }))
        .into_any_element()
}

/// A segmented choice of text labels.
fn choice(id: &'static str, options: &[(&'static str, &'static str)], value: &str, cx: &mut Context<SettingsView>, f: fn(&mut diffr_git::Settings, String)) -> AnyElement {
    let p = pal(cx).clone();
    let mut track = ui::segment_track();
    for (i, (v, label)) in options.iter().enumerate() {
        let on = value == *v;
        let v = v.to_string();
        let mut seg = div()
            .id(SharedString::from(format!("{id}-{i}")))
            .h(px(24.))
            .px(px(10.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .text_size(px(12.))
            .text_color(if on { p.text } else { p.text_muted })
            .cursor_pointer()
            .child(label.to_string())
            .on_click(cx.listener(move |_, _, _, cx| {
                let v = v.clone();
                crate::config().update(|s| f(s, v));
                cx.notify();
            }));
        if on {
            seg = seg.bg(p.header_bg).font_weight(FontWeight::SEMIBOLD).shadow(vec![gpui::BoxShadow {
                color: rgba(0, 0, 0, 0.12),
                offset: gpui::point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            }]);
        }
        track = track.child(seg);
    }
    track.into_any_element()
}

fn field(input: &Entity<TextInput>, width: f32, mono: bool, cx: &App) -> AnyElement {
    let p = pal(cx);
    let d = div()
        .w(px(width))
        .px(px(8.))
        .py(px(5.))
        .rounded(px(6.))
        .bg(p.background)
        .border_1()
        .border_color(p.border)
        .text_size(px(13.))
        .line_height(px(18.))
        .child(input.clone());
    if mono { d.font_family(ui::code_font(cx)).into_any_element() } else { d.into_any_element() }
}

impl SettingsView {
    fn general(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = crate::config().get();
        let size = s.code_font_size;
        let p = pal(cx).clone();
        let current = if s.code_font_family.is_empty() { format!("Default ({})", crate::DEFAULT_CODE_FONT) } else { s.code_font_family.clone() };
        let mut picker = div().relative().child(
            ui::button("font-pick", cx)
                .px(px(10.))
                .py(px(3.))
                .text_size(px(13.))
                .child(current)
                .child(icon("chevron_down", 12., p.text_muted))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.font_menu = !this.font_menu;
                    if this.font_menu {
                        this.load_fonts(cx);
                    }
                    cx.notify();
                })),
        );
        if self.font_menu {
            let mut list = div()
                .id("font-list")
                .w(px(260.))
                .max_h(px(300.))
                .overflow_y_scroll()
                .p(px(4.))
                .rounded(px(8.))
                .bg(p.background)
                .border_1()
                .border_color(p.border)
                .shadow_lg()
                .text_size(px(13.));
            let mut options = vec![(String::new(), format!("Default ({})", crate::DEFAULT_CODE_FONT))];
            if let FontChoices::Ready(names) = &self.fonts {
                options.extend(names.iter().map(|name| (name.clone(), name.clone())));
            }
            for (i, (value, label)) in options.into_iter().enumerate() {
                let on = value == s.code_font_family;
                let accent = p.accent;
                let at = p.accent_text;
                let fam = if value.is_empty() { crate::DEFAULT_CODE_FONT.to_string() } else { value.clone() };
                list = list.child(
                    div()
                        .id(SharedString::from(format!("font-{i}")))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .font_family(SharedString::from(fam))
                        .hover(move |st| st.bg(accent).text_color(at))
                        .cursor_pointer()
                        .child(format!("{}{label}", if on { "✓ " } else { "" }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let v = value.clone();
                            crate::config().update(|s| s.code_font_family = v.clone());
                            this.code_font.update(cx, |i, cx| i.set_text(v, cx));
                            this.font_menu = false;
                            cx.notify();
                        })),
                );
            }
            let status = match &self.fonts {
                FontChoices::Unloaded | FontChoices::Loading => Some("Loading fonts…"),
                FontChoices::Ready(names) if names.is_empty() => Some("No matching fonts found."),
                FontChoices::Ready(_) => None,
            };
            if let Some(status) = status {
                list = list.child(div().px(px(8.)).py(px(4.)).text_color(p.text_muted).child(status));
            }
            picker = picker.child(gpui::deferred(gpui::anchored().snap_to_window_with_margin(px(8.)).child(div().mt(px(30.)).child(list))).with_priority(2));
        }
        let stepper = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(ui::button("font-minus", cx).px(px(10.)).py(px(2.)).child("−").on_click(cx.listener(move |_, _, _, cx| {
                crate::config().update(|s| s.code_font_size = (size - 1).clamp(10, 32));
                cx.notify();
            })))
            .child(div().w(px(44.)).text_center().text_size(px(13.)).child(format!("{size} pt")))
            .child(ui::button("font-plus", cx).px(px(10.)).py(px(2.)).child("+").on_click(cx.listener(move |_, _, _, cx| {
                crate::config().update(|s| s.code_font_size = (size + 1).clamp(10, 32));
                cx.notify();
            })));
        vec![
            section(
                "Appearance",
                vec![
                    row(
                        "Theme",
                        "Follows the system unless you choose.",
                        choice("theme", &[("system", "Match System"), ("light", "Light"), ("dark", "Dark")], &s.theme, cx, |s, v| s.theme = v),
                        cx,
                    ),
                    row(
                        "Sidebar",
                        "Which side of the window the file tree is on.",
                        choice("sidebar", &[("left", "Left"), ("right", "Right")], &s.sidebar_position, cx, |s, v| {
                            s.sidebar_position = v
                        }),
                        cx,
                    ),
                    row("Code font", "Monospace fonts installed on this system.", picker, cx),
                    row("Code font size", "Also Ctrl+= and Ctrl+-.", stepper, cx),
                ],
                cx,
            ),
            section(
                "Settings file",
                vec![row(
                    "diffr.jsonc",
                    "The settings, as text; diffr reloads it as it changes.",
                    ui::button("open-config", cx).px(px(10.)).py(px(3.)).text_size(px(13.)).child("Open diffr.jsonc").on_click(cx.listener(
                        |_, _, _, cx| {
                            let path = crate::config().ensure();
                            let s = crate::config().get();
                            let _ = crate::editor::open_editor(&s.editor_command, "/", &path, 0);
                            cx.notify();
                        },
                    )),
                    cx,
                )],
                cx,
            ),
            section(
                "Editor",
                vec![row(
                    "Open files with",
                    "{file}, {line} and {repo} are replaced. Empty is Zed, else VS Code, else the system's choice.",
                    field(&self.editor, 220., true, cx),
                    cx,
                )],
                cx,
            ),
        ]
    }

    fn review(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = crate::config().get();
        vec![
            section(
                "Diffs",
                vec![
                    row(
                        "Layout",
                        "Side by side, or one column.",
                        choice("style", &[("split", "Split"), ("unified", "Unified")], &s.diff_style, cx, |s, v| s.diff_style = v),
                        cx,
                    ),
                    row("Word wrap", "Long lines wrap instead of scrolling sideways.", switch("wrap", s.word_wrap, cx, |s, v| s.word_wrap = v), cx),
                    row(
                        "Show whitespace changes",
                        "Off hides changes of whitespace alone.",
                        switch("ws", s.show_whitespace, cx, |s, v| s.show_whitespace = v),
                        cx,
                    ),
                ],
                cx,
            ),
            section(
                "Comments",
                vec![
                    row(
                        "Copy comments on close",
                        "The review comments go to the clipboard as a window closes.",
                        switch("copyclose", s.copy_comments_on_close, cx, |s, v| s.copy_comments_on_close = v),
                        cx,
                    ),
                    row("Comments heading", "Starts the Markdown of copied comments.", field(&self.prefix, 300., false, cx), cx),
                ],
                cx,
            ),
        ]
    }

    fn ai(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = crate::config().get();
        let p = pal(cx).clone();
        let installed = crate::ai::installed_agents();
        let mut agents: Vec<(&'static str, &'static str)> = vec![("", "Automatic")];
        for a in diffr_agent::ALL {
            agents.push((a.name, a.label));
        }
        let agent_choice = choice("agent", &agents, &s.ai_agent, cx, |s, v| {
            s.ai_agent = v;
            s.ai_model.clear();
        });
        let found = match installed {
            None => "Looking for the agents installed…".to_string(),
            Some(l) if l.is_empty() => "No agent found: install Claude Code, Codex or OpenCode.".to_string(),
            Some(l) => format!("Installed: {}.", l.iter().map(|a| a.label).collect::<Vec<_>>().join(", ")),
        };
        let mut out = vec![section(
            "Agent",
            vec![
                row("Agent", &found, agent_choice, cx),
                row("Model", "As the agent names it, e.g. sonnet. Empty is its default.", field(&self.model, 220., true, cx), cx),
                row(
                    "Codex fast mode",
                    "Codex answers faster, for more usage, on the models that have it. Off leaves Codex's own setting.",
                    switch("aifast", s.ai_fast, cx, |s, v| s.ai_fast = v),
                    cx,
                ),
                row(
                    "In each file",
                    "Notes on the lines that deserve a look, a summary atop the file, or both.",
                    choice(
                        "aishow",
                        &[("notes", "Notes inline"), ("summaries", "A summary atop it"), ("both", "Both"), ("none", "None")],
                        &s.ai_file_info,
                        cx,
                        |s, v| s.ai_file_info = v,
                    ),
                    cx,
                ),
            ],
            cx,
        )];
        out.push(
            div()
                .text_size(px(12.))
                .text_color(p.text_muted)
                .child("The agent reviews read-only, and only when you ask: from the AI Review card, the toolbar's grouping, or Ctrl+Shift+I.")
                .into_any_element(),
        );
        let text = diffr_agent::instructions_preview(&s.labs);
        out.push(disclosure("instructions", "Instructions the agent gets", self.show_instructions, text, cx, |this| {
            this.show_instructions = !this.show_instructions
        }));
        out
    }

    fn labs(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = crate::config().get();
        let p = pal(cx).clone();
        let mut rows = Vec::new();
        for lab in diffr_agent::LABS {
            let on = s.labs.iter().any(|l| l == lab.id);
            let id = lab.id;
            let toggle = div()
                .id(SharedString::from(format!("lab-{id}")))
                .w(px(34.))
                .h(px(20.))
                .rounded(px(10.))
                .p(px(2.))
                .flex()
                .flex_row()
                .when(on, |d| d.justify_end())
                .bg(if on { p.accent } else { p.border })
                .cursor_pointer()
                .child(div().size(px(16.)).rounded_full().bg(gpui::white()).shadow_sm())
                .on_click(cx.listener(move |_, _, _, cx| {
                    crate::config().update(|s| {
                        if on {
                            s.labs.retain(|l| l != id);
                        } else {
                            s.labs.push(id.to_string());
                        }
                    });
                    cx.notify();
                }));
            rows.push(row(lab.name, lab.description, toggle, cx));
            let open = self.open_labs.contains(&id);
            let bin = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "diffr".into());
            rows.push(
                div()
                    .px(px(14.))
                    .pb(px(10.))
                    .child(disclosure(
                        SharedString::from(format!("lab-prompt-{id}")),
                        "What it adds to the agent's instructions",
                        open,
                        lab.prompt(&bin),
                        cx,
                        move |this| {
                            if let Some(i) = this.open_labs.iter().position(|l| *l == id) {
                                this.open_labs.remove(i);
                            } else {
                                this.open_labs.push(id);
                            }
                        },
                    ))
                    .into_any_element(),
            );
        }
        vec![
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(alpha(p.accent, 0.08))
                .child(icon("sparkle", 14., p.accent_foreground))
                .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(p.text).child(
                    "Labs are experiments for the AI review. They take effect at the next review. Tools reach all three agents: Claude Code, Codex and OpenCode.",
                ))
                .into_any_element(),
            section("Experiments", rows, cx),
        ]
    }

    fn github(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = pal(cx).clone();
        let status = match &self.github_user {
            None => div().text_size(px(12.)).text_color(p.text_muted).child("Checking…"),
            Some(Ok(u)) => div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .child(icon("check_circle", 14., p.viewed))
                .child(div().text_size(px(12.)).child(format!("Signed in as {u}"))),
            Some(Err(e)) => div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .child(icon("alert", 14., p.text_muted))
                .child(div().text_size(px(12.)).text_color(p.text_muted).child(e.clone())),
        };
        vec![section(
            "Account",
            vec![
                row(
                    "Token",
                    "Reading public pull requests needs none. Empty uses $DIFFR_GITHUB_TOKEN, $GH_TOKEN, $GITHUB_TOKEN or the GitHub CLI's.",
                    field(&self.token, 260., true, cx),
                    cx,
                ),
                row(
                    "Status",
                    "",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.))
                        .child(status)
                        .child(ui::button("gh-check", cx).px(px(10.)).py(px(2.)).text_size(px(12.)).child("Check").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.github_user = None;
                                this.check_github(cx);
                            }),
                        )),
                    cx,
                ),
            ],
            cx,
        )]
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::apply_theme(window, cx);
        let p = pal(cx).clone();
        let mut nav = div().flex().flex_col().w(px(190.)).flex_none().h_full().bg(p.sidebar_bg).px(px(8.)).pt(px(14.)).gap(px(1.));
        for (tab, label, ic) in TABS {
            let on = tab == self.tab;
            let mut item = div()
                .id(SharedString::from(format!("tab-{label}")))
                .h(px(30.))
                .px(px(10.))
                .gap(px(8.))
                .flex()
                .flex_row()
                .items_center()
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .child(icon(ic, 14., if on { p.text } else { p.text_muted }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    cx.notify();
                }));
            item = if on { item.bg(p.selected).text_color(p.text) } else { item.hover(|s| s.bg(p.surface_hover)) };
            nav = nav.child(item);
        }
        let body = match self.tab {
            Tab::General => self.general(cx),
            Tab::Review => self.review(cx),
            Tab::Ai => self.ai(cx),
            Tab::Labs => self.labs(cx),
            Tab::GitHub => self.github(cx),
        };
        let title = TABS.iter().find(|t| t.0 == self.tab).map(|t| t.1).unwrap_or("");
        let path = crate::config().path().to_string_lossy().to_string();
        let _ = &self.root;
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_row()
            .bg(p.app_bg)
            .text_color(p.text)
            .font_family(cx.global::<ui::Fonts>().ui.clone())
            .text_size(px(14.))
            .line_height(gpui::relative(1.36))
            .child(nav)
            .child(div().w(px(1.)).h_full().bg(p.card_border))
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(18.))
                            .px(px(24.))
                            .py(px(20.))
                            .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).child(title))
                            .children(body)
                            .child(
                                div()
                                    .id("config-path")
                                    .text_size(px(11.))
                                    .text_color(p.text_muted)
                                    .font_family(ui::code_font(cx))
                                    .child(format!("Saved in {path}"))
                                    .tooltip(tooltip("The settings file; it can be edited too.")),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_choices_load_only_once_even_when_empty() {
        let mut fonts = FontChoices::default();
        assert!(matches!(fonts, FontChoices::Unloaded));
        assert!(fonts.start_loading());
        assert!(matches!(fonts, FontChoices::Loading));
        assert!(!fonts.start_loading());
        fonts = FontChoices::Ready(Arc::from([]));
        assert!(!fonts.start_loading());
        assert!(matches!(fonts, FontChoices::Ready(names) if names.is_empty()));
    }

    #[test]
    fn ready_font_choices_do_not_reload() {
        let mut fonts = FontChoices::Ready(Arc::from(["Menlo".to_string()]));
        assert!(!fonts.start_loading());
        assert!(matches!(fonts, FontChoices::Ready(names) if names.as_ref() == ["Menlo"]));
    }

    #[test]
    fn font_names_are_filtered_sorted_and_deduplicated() {
        let fonts = monospace_fonts(
            ["Menlo", "Arial", "JetBrains Mono", "Menlo", "courier", "Helvetica", "Source CODE Pro"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        );
        assert_eq!(fonts.as_ref(), ["JetBrains Mono", "Menlo", "Source CODE Pro", "courier"]);
        assert!(monospace_fonts(vec!["Arial".into()]).is_empty());
    }

    #[test]
    fn settings_snapshot_notifies_only_for_changes() {
        let mut settings = diffr_git::Settings::default();
        let mut snapshot = SettingsSnapshot { settings: settings.clone(), agents: None };
        assert!(!snapshot.refresh(SettingsSnapshot { settings: settings.clone(), agents: None }));
        settings.editor_command = "zed {file}".into();
        assert!(snapshot.refresh(SettingsSnapshot { settings: settings.clone(), agents: None }));
        assert!(!snapshot.refresh(SettingsSnapshot { settings: settings.clone(), agents: None }));
        settings.theme = "dark".into();
        assert!(snapshot.refresh(SettingsSnapshot { settings: settings.clone(), agents: None }));
        assert!(!snapshot.refresh(SettingsSnapshot { settings, agents: None }));
    }

    #[test]
    fn settings_snapshot_detects_agent_discovery_completion() {
        let settings = diffr_git::Settings::default();
        let mut snapshot = SettingsSnapshot { settings: settings.clone(), agents: None };
        // An empty completed discovery must still replace "Looking…".
        assert!(snapshot.refresh(SettingsSnapshot { settings: settings.clone(), agents: Some(vec![]) }));
        assert!(!snapshot.refresh(SettingsSnapshot { settings: settings.clone(), agents: Some(vec![]) }));
        let agents = Some(vec![diffr_agent::ALL[0]]);
        assert!(snapshot.refresh(SettingsSnapshot { settings: settings.clone(), agents: agents.clone() }));
        assert!(!snapshot.refresh(SettingsSnapshot { settings, agents }));
    }
}
