//! The window's layout: the sidebar, the toolbar, and the review or the
//! commit view.

use std::time::Duration;

use gpui::{
    AnimationExt, AnyElement, Context, FontWeight, IntoElement, MouseButton, Render, Window, div,
    prelude::*, px,
};

use crate::sidebar::{TITLE_BAR_HEIGHT, window_controls_inset};
use crate::state::*;
use crate::theme::{alpha, mix, over, rgba};
use crate::ui::{self, chip, icon, icon_button, pal, segment_track, tooltip};
use crate::window::Diffr;

impl Render for Diffr {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx).clone();
        self.comment_focused = self
            .comment_inputs
            .iter()
            .find(|(_, i)| i.read(cx).is_focused(window))
            .map(|(id, _)| *id);
        // The window's keys need something focused: the root, at least.
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        if !self.focused_once && !self.files.is_empty() {
            self.focused_once = true;
            window.focus(&self.list_focus, cx);
        }
        let right = self.settings.sidebar_position == "right";
        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(p.app_bg)
            .child(self.toolbar(window, cx))
            .child(self.main_area(window, cx));
        let mut row = div().flex().flex_row().size_full();
        if self.sidebar_shown && !right {
            let sb = self.sidebar(window, cx);
            row = row.child(sb).child(self.sidebar_resizer(cx));
        }
        row = row.child(main);
        if self.sidebar_shown && right {
            let rz = self.sidebar_resizer(cx);
            row = row.child(rz).child(self.sidebar(window, cx));
        }
        let mut root = div()
            .id("diffr-root")
            .key_context("Diffr")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(p.sidebar_bg)
            .text_color(p.text)
            .font_family(cx.global::<ui::Fonts>().ui.clone())
            .text_size(px(14.))
            .line_height(gpui::relative(1.36))
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, window, cx| {
                if this.line_drag.is_some() {
                    if ev.dragging() {
                        this.line_drag_edge(f32::from(ev.position.y), cx);
                    } else {
                        this.finish_line_drag(window, cx);
                    }
                    return;
                }
                if !this.on_scroll_drag(ev, cx) {
                    this.on_resize_move(ev, window, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| {
                this.finish_line_drag(window, cx);
                this.end_scroll_drag(cx);
                if this.resizing.take().is_some() {
                    this.save_layout(cx);
                }
            }))
            .child(row);
        root = crate::actions::register(root, cx);
        root = root.children(self.overlays(window, cx));
        root
    }
}

impl Diffr {
    /// The bar along the top of the review.
    fn toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let wide = f32::from(window.viewport_size().width) - if self.sidebar_shown { self.sidebar_width } else { 0. } > 930.;
        let name = std::path::Path::new(&self.root)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let parent = std::path::Path::new(&self.root)
            .parent()
            .map(|d| abbreviate_home(&d.to_string_lossy()))
            .unwrap_or_default();
        let mut bar = div()
            .h(px(TITLE_BAR_HEIGHT))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .pl(px(if self.sidebar_shown { 14. } else { 8. + window_controls_inset(window) }))
            .pr(px(12.))
            .gap(px(8.))
            .border_b_1()
            .border_color(p.card_border)
            .bg(alpha(p.header_bg, 0.6));
        if !self.sidebar_shown {
            bar = bar.children(self.app_menu_button(cx));
            bar = bar.child(self.sidebar_toggle(cx));
        }
        bar = bar.child(
            div()
                .id("repo-title")
                .min_w(px(64.))
                .max_w(px(if wide { 130. } else { 150. }))
                .flex()
                .flex_col()
                .gap(px(1.))
                .flex_shrink(1.)
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::BOLD)
                        .whitespace_nowrap().overflow_hidden().text_ellipsis()
                        .child(name),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.text_muted)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(if self.source == Source::WorkingTree && !wide { "Work tree".to_string() } else { parent }),
                )
                .tooltip(tooltip(format!("{}{}", self.root, if self.source == Source::WorkingTree { " — Working changes" } else { "" }))),
        );
        if !self.branch.is_empty() && !self.source.is_pull() {
            bar = bar.child(chip("branch", self.branch.clone(), p.text_muted, false, cx).max_w(px(100.)));
        }
        match self.source.clone() {
            Source::Commit(h) => {
                bar = bar.child(chip("commit", short_hash(&h).to_string(), p.reference, true, cx));
            }
            Source::Branch(b) => {
                bar = bar.child(chip("branch", format!("Local + branch vs {b}"), p.reference, false, cx));
            }
            Source::Pull(_) => {
                for c in self.pull_chips(cx) {
                    bar = bar.child(c);
                }
            }
            Source::WorkingTree => {}
        }
        if self.source == Source::WorkingTree && wide {
            bar = bar.child(div().text_size(px(12.)).text_color(p.text_muted)
                .whitespace_nowrap().child("Working changes"));
        }
        if !self.files.is_empty() {
            let viewed = self.files.iter().filter(|f| self.is_viewed(f)).count();
            bar = bar.child(div().id("review-progress").text_size(px(12.)).text_color(p.text_muted)
                .whitespace_nowrap().child(format!("{viewed}/{} viewed", self.files.len()))
                .tooltip(tooltip("Viewed files across the entire review, including filtered files")));
            if viewed < self.files.len() {
                let destination = next_matching_file(self.files.len(), self.current,
                    |i| self.file_visible(i, cx) && !self.is_viewed(&self.files[i]));
                let narrow = f32::from(window.viewport_size().width)
                    - if self.sidebar_shown { self.sidebar_width } else { 0. } < 560.;
                let mut next = (if destination.is_some() {
                    ui::button("next-unviewed", cx)
                } else {
                    div().id("next-unviewed").flex().items_center().gap(px(6.))
                        .rounded(px(6.)).bg(p.background).border_1().border_color(p.border)
                }).h(px(28.)).px(px(7.))
                    .flex_none().flex().items_center().rounded(px(5.)).text_size(px(12.))
                    .text_color(p.text);
                next = if narrow {
                    next.child(icon("arrow_down", 16., if destination.is_some() { p.text } else { p.text_muted }))
                } else {
                    next.child(icon("arrow_down", 13., if destination.is_some() { p.text } else { p.text_muted })).child(if wide { "Next unviewed" } else { "Next" })
                };
                next = if destination.is_some() {
                    next.cursor_pointer()
                        .tooltip(tooltip("Next unviewed file in current results (U)"))
                        .on_click(cx.listener(|this, _, _, cx| this.next_unviewed(cx)))
                } else {
                    next.text_color(p.text_muted)
                        .tooltip(tooltip("No unviewed files in current results"))
                };
                bar = bar.child(next);
            } else {
                bar = bar.child(div().flex_none().flex().items_center().gap(px(4.))
                    .text_size(px(12.)).text_color(p.text_muted)
                    .child(icon("check", 13., p.text_muted)).child("All viewed"));
            }
        }

        if self.loading && !self.files.is_empty() {
            bar = bar.child(ui::spinner("toolbar-spin", 14., p.text_muted));
        }
        bar = bar.child(ui::spacer());
        if !(self.commit_open && self.source == Source::WorkingTree) {
            bar = bar.child(
                icon_button("find-toggle", "search", "Find in diffs (Ctrl+F)", cx).on_click(cx.listener(
                    |this, _, window, cx| {
                        if this.finding {
                            this.close_find(window, cx)
                        } else {
                            this.open_find(window, cx)
                        }
                    },
                )),
            );
            let n = self.pending_comments();
            let copied = self.copied_recently();
            let mut cb = div()
                .id("copy-comments")
                .h(px(28.))
                .px(px(8.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .rounded(px(7.))
                .text_color(p.text_muted)
                .tooltip(tooltip(if n == 0 {
                    "No review comments to copy yet".to_string()
                } else {
                    format!("Copy all {} as Markdown (Ctrl+Shift+C)", ui::plural(n, "review comment"))
                }))
                .child(if copied { icon("check", 15., p.viewed) } else { icon("comment", 15., p.text_muted) })
                .child(
                    div()
                        .font_family(ui::code_font(cx))
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .child(n.to_string()),
                );
            if n == 0 {
                cb = cb.opacity(0.5);
            } else {
                cb = cb
                    .hover(|s| s.bg(p.surface_hover))
                    .on_click(cx.listener(|this, _, _, cx| this.copy_comments(cx)));
            }
            if n > 0 { bar = bar.child(cb); }
            if self.source.is_pull() {
                bar = bar.child(self.review_button(cx));
            }
            // The eye shows or hides the review's notes, once there is one.
            if self.analysis_result().is_some() {
                let shown = self.show_notes();
                let tip = if shown {
                    "Hide the AI review's \u{201c}Worth a look\u{201d} notes (Ctrl+Alt+N)"
                } else {
                    "Show the AI review's \u{201c}Worth a look\u{201d} notes (Ctrl+Alt+N)"
                };
                bar = bar.child(
                    icon_button("notes-eye", if shown { "eye" } else { "eye_off" }, tip, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_ai_notes(cx))),
                );
            }
            bar = bar.child(self.group_control(window, cx));
            bar = bar.child(self.layout_control(wide, cx));
        }
        bar.into_any_element()
    }

    /// Switches between split and unified diffs.
    fn layout_control(&mut self, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let split = self.split();
        if !wide {
            return segment_track().bg(pal(cx).surface)
                .child(ui::segment("layout-split", "split", "Split", split, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.set_split(true, cx))))
                .child(ui::segment("layout-unified", "unified", "Unified", !split, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.set_split(false, cx))))
                .into_any_element();
        }
        segment_track().bg(pal(cx).surface)
            .child(ui::segment_with("layout-split", div().flex().items_center().gap(px(5.)).child(icon("split", 15., pal(cx).text)).child("Split").into_any_element(), "Split", split, cx).w(px(66.)).text_size(px(12.)).on_click(cx.listener(|this, _, _, cx| {
                this.set_split(true, cx);
            })))
            .child(ui::segment_with("layout-unified", div().flex().items_center().gap(px(5.)).child(icon("unified", 15., pal(cx).text)).child("Unified").into_any_element(), "Unified", !split, cx).w(px(78.)).text_size(px(12.)).on_click(cx.listener(
                |this, _, _, cx| {
                    this.set_split(false, cx);
                },
            )))
            .into_any_element()
    }

    pub fn set_split(&mut self, split: bool, cx: &mut Context<Self>) {
        if split == self.split() {
            return;
        }
        let style = if split { "split" } else { "unified" }.to_string();
        self.settings.diff_style = style.clone();
        crate::config().update(|s| s.diff_style = style);
        self.rows_dirty = true;
        cx.notify();
    }

    /// The review, the commit view, or why there is nothing.
    fn main_area(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        if self.commit_open && self.source == Source::WorkingTree {
            return self.commit_view(window, cx);
        }
        let mut col = div().flex().flex_col().flex_1().min_h_0();
        if self.changed {
            let tone = mix(p.viewed, p.text, 0.6);
            let what = if self.source.is_pull() { "New commits were pushed," } else { "Local changes detected," };
            col = col.child(
                div().flex().flex_row().justify_center().pt(px(8.)).px(px(12.)).child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .pl(px(12.))
                        .pr(px(6.))
                        .py(px(5.))
                        .rounded(px(16.))
                        .bg(over(p.code_bg, alpha(p.viewed, 0.1)))
                        .border_1()
                        .border_color(alpha(p.viewed, 0.2))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tone)
                        .child(what)
                        .child(
                            div()
                                .id("refresh-banner")
                                .underline()
                                .cursor_pointer()
                                .child("refresh to see them.")
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        )
                        .child(
                            icon_button("dismiss-banner", "close", "Dismiss", cx)
                                .size(px(22.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.changed = false;
                                    cx.notify();
                                })),
                        ),
                ),
            );
        }
        if !self.files.is_empty() && self.load_err.is_none()
            && let Some(s) = self.scope_bar(cx) {
                col = col.child(s);
            }
        if self.finding {
            col = col.child(self.find_bar(window, cx));
        }
        let waiting = self.loading && self.files.is_empty();
        let slow = waiting && self.load_start.elapsed() > Duration::from_millis(200);
        if matches!(self.source, Source::Commit(_)) && self.commit.is_some() && self.load_err.is_none() && self.files.is_empty() {
            col = col.child(div().pt(px(11.)).px(px(12.)).child(self.commit_message(cx)));
        }
        let body: AnyElement = if let Some(err) = self.load_err.clone() {
            if self.source.is_pull() {
                empty_panel(
                    "Unable to open the pull request",
                    &err,
                    Some(
                        ui::button("try-again", cx)
                            .child("Try Again")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                            .into_any_element(),
                    ),
                    cx,
                )
            } else {
                empty_panel("Unable to read repository", &err, None, cx)
            }
        } else if slow {
            let status = if self.load_status.is_empty() { "Thinking…".to_string() } else { self.load_status.clone() };
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .italic()
                        .text_size(px(13.))
                        .font_family(ui::code_font(cx))
                        .text_color(p.text_muted)
                        .child(status)
                        .with_animation(
                            "pulse",
                            gpui::Animation::new(Duration::from_millis(1600)).repeat().with_easing(gpui::bounce(gpui::ease_in_out)),
                            |el, t| el.opacity(0.5 + 0.5 * t),
                        ),
                )
                .into_any_element()
        } else if waiting {
            div().flex_1().into_any_element()
        } else if self.files.is_empty() {
            let (title, detail) = match &self.source {
                Source::Commit(h) => ("No changes in commit".to_string(), short_hash(h).to_string()),
                Source::Branch(b) => ("No changes".to_string(), b.clone()),
                Source::Pull(r) => ("No changes in pull request".to_string(), r.clone()),
                Source::WorkingTree => ("No local changes".to_string(), abbreviate_home(&self.root)),
            };
            let action = (self.source == Source::WorkingTree && !self.history.is_empty()).then(|| {
                ui::button("show-history", cx)
                    .child("Show History")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tab = 1;
                        this.sidebar_shown = true;
                        cx.notify();
                    }))
                    .into_any_element()
            });
            empty_panel(&title, &detail, action, cx)
        } else {
            if self.rows_dirty {
                self.build_rows(cx);
            }
            if self.rows.is_empty() {
                let (title, detail) = if self.searching(cx) {
                    ("No matches in diffs", self.query(cx).trim().to_string())
                } else {
                    ("No matching files", self.filter_text(cx))
                };
                let detail = if detail.is_empty() { "Whitespace-only changes hidden".to_string() } else { detail };
                empty_panel(title, &detail, None, cx)
            } else {
                self.diff_list(window, cx)
            }
        };
        col.child(body).into_any_element()
    }

    fn find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let focused = self.find_input.read(cx).is_focused(window);
        let count = if self.query(cx).trim().is_empty() {
            String::new()
        } else if self.matches.is_empty() {
            "No results".into()
        } else {
            format!("{} of {}", self.match_ix + 1, self.matches.len())
        };
        let mut field = div()
            .w(px(360.))
            .h(px(34.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .rounded(px(6.))
            .bg(p.background)
            .border_1()
            .border_color(if focused { p.accent } else { p.border })
            .text_size(px(14.))
            .line_height(px(20.))
            .child(icon("search", 13., p.text_muted))
            .child(div().flex_1().min_w_0().child(self.find_input.clone()));
        if focused {
            field = field.shadow(vec![gpui::BoxShadow {
                color: alpha(p.focus, 0.4),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(1.),
                inset: false,
            }]);
        }
        let step = |id: &'static str, name: &'static str, tip: &'static str, dir: i32, cx: &mut Context<Self>| {
            ui::button(id, cx)
                .px(px(8.))
                .py(px(6.))
                .child(icon(name, 13., p.text))
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |this, _, _, cx| this.step_match(dir, cx)))
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(6.))
            .border_b_1()
            .border_color(p.card_border)
            .bg(p.surface)
            .child(field)
            .child(div().w(px(72.)).text_size(px(14.)).text_color(p.text_muted).child(count))
            .child(step("find-prev", "chevron_up", "Previous match (Shift+Enter)", -1, cx))
            .child(step("find-next", "chevron_down", "Next match (Enter)", 1, cx))
            .child(
                ui::button("find-done", cx)
                    .child("Done")
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            )
            .into_any_element()
    }
}

pub fn check_mark(on: bool, mixed: bool, cx: &gpui::App) -> gpui::Div {
    let p = pal(cx);
    let b = div().size(px(16.)).rounded(px(4.)).flex().items_center().justify_center().flex_none();
    if on {
        b.bg(p.accent).child(icon("check", 11., p.accent_text))
    } else if mixed {
        b.bg(p.accent).child(div().w(px(8.)).h(px(2.)).rounded(px(1.)).bg(p.accent_text))
    } else {
        b.border(px(1.5)).border_color(rgba(127, 127, 127, 0.45)).bg(p.code_bg)
    }
}

/// Says why there is nothing to show.
pub fn empty_panel(title: &str, detail: &str, action: Option<AnyElement>, cx: &gpui::App) -> AnyElement {
    let p = pal(cx);
    let mut card = div()
        .max_w(px(520.))
        .p(px(28.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .rounded(px(16.))
        .bg(p.header_bg)
        .border_1()
        .border_color(p.card_border)
        .items_center()
        .child(icon("file_diff", 28., p.text_muted))
        .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_center().child(title.to_string()));
    if !detail.is_empty() {
        card = card.child(
            div()
                .text_size(px(13.))
                .font_family(ui::code_font(cx))
                .text_color(p.text_muted)
                .text_center()
                .child(detail.to_string()),
        );
    }
    if let Some(a) = action {
        card = card.child(div().mt(px(6.)).flex().flex_row().gap(px(8.)).child(a));
    }
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.))
        .child(card)
        .into_any_element()
}
