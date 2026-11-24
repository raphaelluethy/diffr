//! The sidebar: the file tree, the history and the pull requests, the
//! filter above them and the total and the Commit button below.

use diffr_git::Status;
use gpui::{
    AnyElement, Context, Entity, FontWeight, MouseButton, MouseMoveEvent, SharedString, Window, div,
    prelude::*, px, uniform_list,
};

use crate::input::TextInput;
use crate::state::*;
use crate::theme::{alpha, rgba};
use crate::ui::{self, icon, icon_button, pal, segment, segment_track, tooltip};
use crate::window::*;

pub const TITLE_BAR_HEIGHT: f32 = 52.;

/// Room at the left of the title bar for the window controls macOS draws
/// over it; none in full screen, where they hide.
pub fn window_controls_inset(window: &Window) -> f32 {
    if cfg!(target_os = "macos") && !window.is_fullscreen() { 78. } else { 0. }
}

impl Diffr {
    pub fn sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        if self.tab == 2 && !self.has_github() {
            self.tab = 0;
        }
        let input = match self.tab {
            0 => self.filter.clone(),
            1 => self.history_filter.clone(),
            _ => self.pulls_filter.clone(),
        };
        let body = match self.tab {
            0 => self.file_tree(window, cx),
            1 => self.history_view(window, cx),
            _ => self.pulls_view(window, cx),
        };
        div()
            .flex()
            .flex_col()
            .w(px(self.sidebar_width))
            .flex_none()
            .h_full()
            .bg(p.sidebar_bg)
            .child(
                div()
                    .h(px(TITLE_BAR_HEIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .pl(px(10. + window_controls_inset(window)))
                    .pr(px(10.))
                    .gap(px(6.))
                    .flex_none()
                    .child(self.tab_control(cx))
                    .child(self.sidebar_toggle(cx))
                    .child(ui::spacer())
                    .children(self.app_menu_button(cx)),
            )
            .child(div().px(px(10.)).pt(px(2.)).pb(px(8.)).child(search_input(&input, window, cx)))
            .when(self.tab == 0 && self.searching(cx), |s| s.child(
                div().px(px(14.)).pb(px(6.)).text_size(px(11.)).text_color(p.text_muted)
                    .child(format!("Filtered by find · {}", ui::plural(
                        (0..self.files.len()).filter(|&i| self.file_visible(i, cx)).count(), "file"))),
            ))
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
            .children(self.sidebar_footer(cx))
            .into_any_element()
    }

    fn tab_control(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let mut items = vec![("tree", "Files (Ctrl+1)"), ("history", "History (Ctrl+2)")];
        if self.has_github() {
            items.push(("pull", "Pull Requests (Ctrl+3)"));
        }
        let mut track = segment_track().bg(p.surface);
        for (i, (name, tip)) in items.into_iter().enumerate() {
            track = track.child(segment(SharedString::from(format!("tab{i}")), name, tip, self.tab == i, cx).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.set_tab(i, cx);
                }),
            ));
        }
        track.into_any_element()
    }

    pub fn set_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.tab != i {
            self.commit_open = false;
        }
        self.tab = i;
        self.sidebar_shown = true;
        if i == 2 {
            self.ensure_pulls(cx);
        }
        cx.notify();
    }

    pub fn sidebar_toggle(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tip = if self.sidebar_shown { "Hide sidebar (Ctrl+Shift+B)" } else { "Show sidebar (Ctrl+Shift+B)" };
        icon_button("sidebar-toggle", "sidebar", tip, cx)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)))
            .into_any_element()
    }

    fn file_tree(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let rows = self.visible_tree(cx);
        if rows.is_empty() {
            let msg = if self.loading && self.files.is_empty() {
                "Loading…"
            } else if !self.filter_text(cx).is_empty() {
                "No matching files"
            } else {
                "No changed files"
            };
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p(px(16.))
                .child(div().text_size(px(12.)).text_color(p.text_muted).child(msg))
                .into_any_element();
        }
        self.tree_rows = rows;
        let n = self.tree_rows.len();
        let view = cx.entity();
        let list = uniform_list("file-tree", n, move |range, window, cx| {
            view.update(cx, |this, cx| {
                range.map(|i| div().h(px(29.)).child(this.tree_row(i, window, cx))).collect::<Vec<_>>()
            })
        })
        .track_scroll(&self.tree_scroll)
        .track_focus(&self.tree_focus)
        .key_context("FileTree")
        .size_full()
        .px(px(8.))
        .py(px(2.));
        let handle = self.tree_scroll.0.borrow().base_handle.clone();
        let sb = self.scrollbar("tree-sb", crate::scrollbar::Scroll::Handle(handle), "tree", cx);
        div().group("tree").relative().flex_1().min_h_0().child(list).child(sb).into_any_element()
    }

    fn tree_row(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let Some(row) = self.tree_rows.get(i).cloned() else {
            return div().into_any_element();
        };
        let Some(n) = self.tree_items.get(&row.key).cloned() else {
            return div().into_any_element();
        };
        let focused = self.tree_focus.is_focused(window);
        let selected = if self.tree_focus.is_focused(window) {
            row.key == self.tree_sel
        } else {
            !n.dir && n.file == self.current
        };
        let sel_focus = selected && focused;
        let text_color = if sel_focus { p.accent_text } else { p.text };
        let muted = if sel_focus { p.accent_text } else { p.text_muted };
        let key = row.key.clone();
        let mut el = div()
            .id(SharedString::from(format!("tr-{}", row.key)))
            .w_full()
            .h(px(28.))
            .flex()
            .flex_row()
            .items_center()
            .pl(px(6. + row.depth as f32 * 14.))
            .pr(px(8.))
            .gap(px(5.))
            .rounded(px(6.))
            .min_w_0()
            .text_color(text_color)
            .on_mouse_down(MouseButton::Left, {
                let key = key.clone();
                cx.listener(move |this, _, window, cx| {
                    window.focus(&this.tree_focus, cx);
                    if this.tree_sel != key || this.commit_open {
                        this.choose_tree(&key, cx);
                    }
                })
            })
            .on_click({
                let key = key.clone();
                let dir = n.dir;
                cx.listener(move |this, _, _, cx| {
                    if dir {
                        if !this.closed_dirs.remove(&key) {
                            this.closed_dirs.insert(key.clone());
                        }
                        cx.notify();
                    }
                })
            });
        el = if sel_focus {
            el.bg(p.accent)
        } else if selected {
            el.bg(p.selected)
        } else {
            el.hover(|s| s.bg(p.surface_hover))
        };
        let arrow = div().size(px(14.)).flex_none().flex().items_center().justify_center();
        if n.dir {
            let closed = self.closed_dirs.contains(&row.key);
            let arrow = arrow.child(icon("chevron_down", 12., muted).with_transformation(gpui::Transformation::rotate(
                gpui::radians(if closed { -std::f32::consts::FRAC_PI_2 } else { 0. }),
            )));
            el = el.child(arrow);
            if n.group >= 0 {
                let (icon_name, critical, summary) = self.group_icon(n.group as usize);
                let color = if critical && !sel_focus { p.danger } else { muted };
                return el
                    .child(icon(icon_name, 14., color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(n.name.clone()),
                    )
                    .child(
                        div()
                            .font_family(code_font)
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted)
                            .flex_none()
                            .child(self.tree_count(&row.key, cx).to_string()),
                    )
                    .tooltip(tooltip(summary))
                    .into_any_element();
            }
            return el
                .child(icon("folder", 14., muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(n.name.clone()),
                )
                .into_any_element();
        }
        el = el.child(arrow);
        let fi_menu = n.file;
        el = el.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                this.open_context_menu(crate::menus::ContextMenu::TreeFile(fi_menu), ev.position, cx);
            }),
        );
        let f = &self.files[n.file];
        let viewed = self.is_viewed(f);
        let tok = diffr_fileicon::token(&f.file.path);
        let icolor = if sel_focus { muted } else { crate::theme::hex(diffr_fileicon::color(tok, p.dark)) };
        el = el
            .child(if f.file.directory {
                icon("folder", 14., muted)
            } else {
                gpui::svg()
                    .path(SharedString::from(diffr_fileicon::icon_path(tok)))
                    .size(px(14.))
                    .flex_none()
                    .text_color(icolor)
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(if viewed && !sel_focus { p.text_muted } else { text_color })
                    .child(n.name.clone()),
            );
        if let Some((count, critical)) = self.file_note_count(&f.file.path) {
            let color = if sel_focus {
                p.accent_text
            } else if critical {
                p.danger
            } else {
                p.accent
            };
            el = el.child(
                div()
                    .id(SharedString::from(format!("trn-{}", row.key)))
                    .child(icon("sparkle", 11., color))
                    .tooltip(tooltip(ui::plural(count, "AI note"))),
            );
        }
        if f.countable() && (f.file.additions > 0 || f.file.deletions > 0) {
            el = el.child(
                div()
                    .font_family(code_font.clone())
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .flex_none()
                    .child(format!("+{} -{}", ui::compact(f.file.additions), ui::compact(f.file.deletions))),
            );
        }
        // Staged: a box to check, as in the commit view.
        if self.can_stage() && !(f.file.directory && f.file.path.starts_with("Untracked files not shown")) {
            let st = self.stage_of(f);
            let fi = n.file;
            let mut b = crate::view::check_mark(st == Some(diffr_git::Staged::Full), st == Some(diffr_git::Staged::Partial), cx).size(px(13.));
            if st.is_none() {
                b = b.opacity(if sel_focus { 0.7 } else { 0.45 });
            }
            el = el.child(
                div()
                    .id(SharedString::from(format!("trs-{}", row.key)))
                    .flex_none()
                    .cursor_pointer()
                    .child(b)
                    .tooltip(tooltip(match st {
                        Some(diffr_git::Staged::Full) => "Staged: click to unstage",
                        Some(diffr_git::Staged::Partial) => "Partly staged: click to stage all of it",
                        None => "Stage for the next commit",
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_stage(fi, cx);
                    })),
            );
        }
        let letter_color = if sel_focus {
            p.accent_text
        } else if viewed {
            p.text_muted
        } else if self.reloaded.contains(&f.file.path) {
            p.reference
        } else {
            status_color(f.file.status, &p)
        };
        el.child(
            div()
                .w(px(12.))
                .flex_none()
                .text_center()
                .font_family(code_font)
                .text_size(px(11.))
                .font_weight(FontWeight::BOLD)
                .text_color(letter_color)
                .child(status_letter(f.file.status)),
        )
        .tooltip(tooltip(format!("{} — {}", f.file.path, match f.file.status {
            Status::Untracked => "Untracked: new file, not yet staged",
            Status::Added => "Added", Status::Deleted => "Deleted",
            Status::Renamed => "Renamed", Status::Copied => "Copied",
            Status::Conflict => "Conflict", _ => "Modified",
        })))
        .into_any_element()
    }

    fn history_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let entries = self.history_entries(cx);
        if entries.is_empty() {
            return div()
                .p(px(12.))
                .text_size(px(12.))
                .text_color(p.text_muted)
                .child("No matching commits")
                .into_any_element();
        }
        let n = entries.len();
        if self.history_list.item_count() != n {
            self.history_list.reset(n);
        }
        let view = cx.entity();
        // More commits load well before the end comes into view.
        let lst = gpui::list(self.history_list.clone(), move |i, window, cx| {
            view.update(cx, |this, cx| {
                let entries = this.history_entries(cx);
                if i + HISTORY_PAGE / 2 >= entries.len() && this.history_more && !this.history_loading {
                    this.history_limit += HISTORY_PAGE;
                    this.load_history(cx);
                }
                if i >= entries.len() {
                    return div().into_any_element();
                }
                div().px(px(8.)).pb(px(1.)).child(this.history_row(&entries, i, window, cx)).into_any_element()
            })
        })
        .size_full()
        .py(px(2.));
        let sb = self.scrollbar("history-sb", crate::scrollbar::Scroll::List(self.history_list.clone()), "history", cx);
        div()
            .id("history")
            .group("history")
            .relative()
            .flex_1()
            .min_h_0()
            .track_focus(&self.history_focus)
            .key_context("History")
            .child(lst)
            .child(sb)
            .into_any_element()
    }

    /// The rows of the History tab: the local changes, then the commits
    /// that pass the filter. None is the local changes.
    pub fn history_entries(&self, cx: &gpui::App) -> Vec<Option<usize>> {
        let q = self.history_filter.read(cx).text().trim().to_lowercase();
        let mut out = Vec::new();
        if q.is_empty() {
            out.push(None);
        }
        for (i, c) in self.history.iter().enumerate() {
            if q.is_empty()
                || c.subject.to_lowercase().contains(&q)
                || c.hash.starts_with(&q)
                || c.author.to_lowercase().contains(&q)
            {
                out.push(Some(i));
            }
        }
        out
    }

    fn history_row(&mut self, entries: &[Option<usize>], i: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let e = entries[i];
        let current = match (&self.source, e) {
            (Source::WorkingTree | Source::Branch(_), None) => true,
            (Source::Commit(h), Some(ci)) => self.history.get(ci).is_some_and(|c| &c.hash == h),
            _ => false,
        };
        let focused = self.history_focus.is_focused(window);
        let (muted, refc) = if current && focused {
            (p.accent_text, p.accent_text)
        } else {
            (p.text_muted, p.reference)
        };
        let mut row = div()
            .id(SharedString::from(format!("hist{i}")))
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(6.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.focus(&this.history_focus, cx);
                    this.open_history(e, window, cx);
                }),
            );
        row = if current && focused {
            row.bg(p.accent).text_color(p.accent_text)
        } else if current {
            row.bg(rgba(127, 127, 127, 0.2)).text_color(p.text)
        } else {
            row.text_color(p.text).hover(|s| s.bg(p.surface_hover))
        };
        let Some(ci) = e else {
            return row
                .h(px(28.))
                .items_center()
                .py(px(0.))
                .child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .font_family(code_font)
                        .text_size(px(12.))
                        .text_color(refc)
                        .child("local"),
                )
                .child(div().text_size(px(12.)).whitespace_nowrap().child("Uncommitted changes"))
                .into_any_element();
        };
        let row = row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                this.open_context_menu(crate::menus::ContextMenu::Commit(ci), ev.position, cx);
            }),
        );
        let c = &self.history[ci];
        let ago = relative_time(now_secs(), c.time);
        row.child(
            div()
                .w(px(56.))
                .flex_none()
                .font_family(code_font)
                .text_size(px(12.))
                .text_color(refc)
                .child(c.short.clone()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(c.subject.clone()),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(11.))
                                .text_color(muted)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(c.author.clone()),
                        )
                        .child(div().text_size(px(11.)).text_color(muted).flex_none().child(ago)),
                ),
        )
        .tooltip(tooltip(c.subject.clone()))
        .into_any_element()
    }

    pub fn open_history(&mut self, e: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_open = false;
        let src = match e {
            None => self.launch_work_tree(),
            Some(ci) => match self.history.get(ci) {
                Some(c) => Source::Commit(c.hash.clone()),
                None => return,
            },
        };
        self.set_source(src, window, cx);
        cx.notify();
    }

    pub fn history_move(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.history_entries(cx);
        let current = entries.iter().position(|e| match (&self.source, e) {
            (Source::WorkingTree | Source::Branch(_), None) => true,
            (Source::Commit(h), Some(ci)) => self.history.get(*ci).is_some_and(|c| &c.hash == h),
            _ => false,
        });
        let next = match current {
            Some(c) => (c as i32 + delta).clamp(0, entries.len() as i32 - 1) as usize,
            None => 0,
        };
        if let Some(e) = entries.get(next).copied() {
            self.history_list.scroll_to_reveal_item(next);
            self.open_history(e, window, cx);
        }
    }

    fn sidebar_footer(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let (mut adds, mut dels, mut counted) = (0, 0, false);
        for i in 0..self.files.len() {
            if self.file_visible(i, cx) && self.files[i].countable() {
                adds += self.files[i].file.additions;
                dels += self.files[i].file.deletions;
                counted = true;
            }
        }
        let can_commit = self.tab == 0 && self.source == Source::WorkingTree && (!self.files.is_empty() || self.upstream.can_push());
        if !(counted && self.tab == 0) && !can_commit {
            return None;
        }
        let mut row = div()
            .min_h(px(46.))
            .flex()
            .flex_row()
            .items_center()
            .px(px(10.))
            .py(px(6.))
            .gap(px(8.))
            .border_t_1()
            .border_color(p.card_border)
            .flex_none();
        if counted && self.tab == 0 {
            row = row.child(
                div()
                    .id("total")
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().text_color(p.text_muted).child("Total:"))
                    .child(div().font_family(code_font.clone()).text_color(p.add_text).child(format!("+{}", ui::thousands(adds))))
                    .child(div().font_family(code_font).text_color(p.del_text).child(format!("-{}", ui::thousands(dels))))
                    .tooltip(tooltip(format!(
                        "Total change: {}, {}",
                        ui::lines_text(adds, "added"),
                        ui::lines_text(dels, "removed")
                    ))),
            );
        }
        row = row.child(ui::spacer());
        if can_commit {
            let staged = self.staged_count();
            let label = if self.commit_open {
                "Review".to_string()
            } else if staged > 0 {
                format!("Commit {staged}")
            } else {
                "Commit".to_string()
            };
            row = row.child(
                ui::button("commit-toggle", cx)
                    .child(icon("commit", 14., p.text))
                    .child(label)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_commit(window, cx))),
            );
        }
        Some(row.into_any_element())
    }

    /// The edge of the sidebar the user drags.
    pub fn sidebar_resizer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        div()
            .w(px(1.))
            .h_full()
            .flex_none()
            .bg(p.card_border)
            .relative()
            .child(
                div()
                    .id("sidebar-resizer")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-4.))
                    .w(px(9.))
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                            this.resizing = Some((f32::from(ev.position.x), this.sidebar_width));
                            cx.stop_propagation();
                        }),
                    ),
            )
            .into_any_element()
    }

    pub fn on_resize_move(&mut self, ev: &MouseMoveEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let Some((x0, w0)) = self.resizing else { return };
        if !ev.dragging() {
            self.resizing = None;
            self.save_layout(cx);
            return;
        }
        let right = self.settings.sidebar_position == "right";
        let dx = f32::from(ev.position.x) - x0;
        let w = if right { w0 - dx } else { w0 + dx };
        if w < SIDEBAR_COLLAPSE {
            self.sidebar_shown = false;
        } else {
            self.sidebar_shown = true;
            self.sidebar_width = w.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        }
        cx.notify();
    }
}

/// A field filtering a list: a magnifying glass, the text, and a button
/// clearing it, as Escape does.
pub fn search_input(input: &Entity<TextInput>, window: &mut Window, cx: &mut Context<Diffr>) -> AnyElement {
    let p = pal(cx).clone();
    let focused = input.read(cx).is_focused(window);
    let empty = input.read(cx).is_empty();
    let mut b = div()
        .h(px(28.))
        .flex()
        .flex_row()
        .items_center()
        .pl(px(8.))
        .pr(px(6.))
        .gap(px(6.))
        .rounded(px(7.))
        .bg(p.surface)
        .text_size(px(13.))
        .text_color(p.text)
        .line_height(px(18.))
        .child(icon("search", 13., p.text_muted))
        .child(div().flex_1().min_w_0().child(input.clone()));
    if focused {
        b = b.shadow(vec![gpui::BoxShadow {
            color: alpha(p.focus, 0.45 / 0.55),
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(3.),
            inset: false,
        }]);
    }
    if !empty {
        let inp = input.clone();
        b = b.child(
            div()
                .id("clear-search")
                .size(px(16.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .bg(alpha(p.text_muted, 0.45))
                .child(icon("close", 10., p.background))
                .on_click(cx.listener(move |_, _, _, cx| {
                    inp.update(cx, |i, cx| {
                        i.set_text("", cx);
                        cx.emit(crate::input::InputEvent::Changed);
                    });
                })),
        );
    }
    b.into_any_element()
}

pub fn status_color(s: Status, p: &crate::theme::Palette) -> gpui::Hsla {
    match s {
        Status::Added | Status::Untracked => p.add_text,
        Status::Deleted => p.del_text,
        Status::Renamed | Status::Copied => p.renamed,
        Status::Conflict => p.danger_foreground,
        _ => p.modified,
    }
}

pub fn status_letter(s: Status) -> String {
    match s {
        Status::Untracked => "U".into(),
        Status::Conflict => "!".into(),
        s => s.letter().to_string(),
    }
}
