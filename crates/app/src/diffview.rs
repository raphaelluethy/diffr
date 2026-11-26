//! The diff surface: the files' cards, their headers, lines, gaps and
//! comments.

use std::ops::Range;
use std::time::{Duration, Instant};

use diffr_git::diff::Range as WordRange;
use diffr_git::{LineKind, Status};
use diffr_highlight::Seg;
use gpui::{
    AnyElement, App, Context, Div, FontWeight, HighlightStyle, Hsla, MouseButton, ScrollWheelEvent,
    SharedString, StyledText, Window, div, img, list, prelude::*, px,
};

use crate::state::*;
use crate::theme::{Palette, alpha, over, rgba};
use crate::ui::{self, check_box, icon, icon_button, pal, tooltip};
use crate::window::Diffr;

pub const CARD_RADIUS: f32 = 10.;

/// Lines being selected by a drag, to comment on: of one side of a file.
#[derive(Clone, Debug)]
pub struct LineDrag {
    pub file: usize,
    pub path: String,
    pub side: Side,
    pub start: u32,
    pub end: u32,
}

/// A range of a line drawn with a background: a changed word or a match.
pub struct Mark {
    pub range: WordRange,
    pub color: Hsla,
}

/// Styles a line of code: the colors of its tokens and the backgrounds of
/// its marks, later marks over earlier ones. Tabs become spaces up to the
/// next tab stop.
pub fn code_runs(text: &str, segs: &[Seg], marks: &[Mark], p: &Palette) -> (SharedString, Vec<(Range<usize>, HighlightStyle)>) {
    if text.is_empty() {
        return (" ".into(), Vec::new());
    }
    let mut cuts = vec![0, text.len()];
    for s in segs {
        cuts.push(s.start as usize);
        cuts.push(s.end as usize);
    }
    for m in marks {
        cuts.push(m.range.start);
        cuts.push(m.range.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = String::with_capacity(text.len());
    let mut runs: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut col = 0usize;
    let mut si = 0;
    for w in cuts.windows(2) {
        let (start, end) = (w[0], w[1].min(text.len()));
        if start >= text.len() || end <= start || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        while si < segs.len() && (segs[si].end as usize) <= start {
            si += 1;
        }
        let color = if si < segs.len() && (segs[si].start as usize) <= start {
            p.syntax[segs[si].class as usize]
        } else {
            p.code
        };
        let mut bg = None;
        for m in marks {
            if m.range.start <= start && end <= m.range.end {
                bg = Some(m.color);
            }
        }
        let part = &text[start..end];
        let from = out.len();
        if part.contains('\t') {
            for c in part.chars() {
                if c == '\t' {
                    let n = TAB_WIDTH - col % TAB_WIDTH;
                    out.extend(std::iter::repeat_n(' ', n));
                    col += n;
                } else {
                    out.push(c);
                    col += 1;
                }
            }
        } else {
            out.push_str(part);
            col += part.chars().count();
        }
        let to = out.len();
        let style = HighlightStyle { color: Some(color), background_color: bg, ..Default::default() };
        if let Some(last) = runs.last_mut()
            && last.0.end == from
            && last.1 == style
        {
            last.0.end = to;
            continue;
        }
        runs.push((from..to, style));
    }
    (out.into(), runs)
}

impl Diffr {
    /// The list of the files' cards, with the header of the file at the
    /// top pinned over it.
    pub fn diff_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.rows_dirty {
            self.build_rows(cx);
        }
        let p = pal(cx).clone();
        // Read now: the list's state is borrowed while its rows render.
        let w = f32::from(self.list.viewport_bounds().size.width);
        if w > 0. {
            self.list_width = w;
        }
        // The file at the top follows the scrolling, and the tree with it.
        let top = self.list.logical_scroll_top();
        let scrolled = top.item_ix != self.last_scroll_top.item_ix
            || top.offset_in_item != self.last_scroll_top.offset_in_item;
        self.last_scroll_top = top;
        let mut first = top.item_ix;
        while first < self.rows.len() && self.rows[first].banner() {
            first += 1;
        }
        if first < self.rows.len() {
            let file = self.rows[first].file;
            if file != self.current && self.scroll_selection_dirty && scrolled {
                self.current = file;
                self.select_tree_file(file, cx);
            }
        }
        self.scroll_selection_dirty = false;
        // The pinned header: of the file whose lines are at the top.
        let mut pinned = None;
        if let Some(r) = self.rows.get(top.item_ix)
            && !r.banner()
            && r.kind != RowKind::Group
            && (r.kind != RowKind::Header || top.offset_in_item > px(8.))
        {
            let f = r.file;
            let collapsed_end = r.kind == RowKind::End;
            if !collapsed_end {
                pinned = Some(self.file_header(usize::MAX, f, true, window, cx));
            }
        }
        let view = cx.entity();
        let lst = list(self.list.clone(), move |ix, window, cx| {
            view.update(cx, |this, cx| this.diff_row(ix, window, cx))
        })
        .size_full();
        let sb = self.scrollbar("diff-sb", crate::scrollbar::Scroll::List(self.list.clone()), "diff-surface", cx);
        div()
            .id("diff-surface")
            .group("diff-surface")
            .relative()
            .flex_1()
            .min_h_0()
            .bg(p.app_bg)
            .track_focus(&self.list_focus)
            .key_context("DiffList")
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                if !this.list_focus.is_focused(window) && !this.any_input_focused(window, cx) {
                    window.focus(&this.list_focus, cx);
                }
            }))
            .child(div().size_full().px(px(12.)).child(lst))
            .children(pinned.map(|h| div().absolute().top_0().left(px(12.)).right(px(12.)).child(h)))
            .child(sb)
            .into_any_element()
    }

    pub fn any_input_focused(&self, window: &Window, cx: &App) -> bool {
        self.filter.read(cx).is_focused(window)
            || self.find_input.read(cx).is_focused(window)
            || self.comment_inputs.values().any(|i| i.read(cx).is_focused(window))
    }

    fn diff_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(r) = self.rows.get(ix).cloned() else {
            return div().into_any_element();
        };
        let p = pal(cx).clone();
        let el: AnyElement = match r.kind {
            RowKind::Commit => div().pt(px(12.)).child(self.commit_message(cx)).into_any_element(),
            RowKind::Pull => div().pt(px(12.)).child(self.pull_header(window, cx)).into_any_element(),
            RowKind::Summary => div().pt(px(12.)).child(self.summary_card(window, cx)).into_any_element(),
            RowKind::Group => self.group_row(r.gap, cx),
            RowKind::Thread => self.thread_row(&r, window, cx),
            RowKind::AiNote => self.note_row(&r, cx),
            RowKind::FileSummary => self.file_summary_row(&r, cx),
            RowKind::Header => self.file_header(ix, r.file, false, window, cx),
            RowKind::Note => {
                let note = self.files[r.file].note().unwrap_or_default();
                card(&p)
                    .px(px(16.))
                    .py(px(10.))
                    .bg(p.gap_bg)
                    .child(div().text_size(px(12.)).text_color(p.gap_text).child(note))
                    .into_any_element()
            }
            RowKind::Gap => self.gap_row(ix, &r, window, cx),
            RowKind::Image => self.image_row(r.file, cx),
            RowKind::Line => self.line_row(ix, &r, window, cx),
            RowKind::Comment => self.comment_row(&r, cx),
            RowKind::End => {
                let f = &self.files[r.file];
                if f.collapsed && !self.force_open(r.file, cx) {
                    div().h(px(0.)).into_any_element()
                } else {
                    div()
                        .h(px(4.))
                        .bg(p.code_bg)
                        .border_l_1()
                        .border_r_1()
                        .border_b_1()
                        .border_color(p.card_border)
                        .rounded_b(px(CARD_RADIUS))
                        .into_any_element()
                }
            }
        };
        let last = ix + 1 == self.rows.len();
        if last { div().pb(px(24.)).child(el).into_any_element() } else { el }
    }

    pub fn file_header(&mut self, ix: usize, fi: usize, pinned: bool, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let collapsed = self.files[fi].collapsed && !self.force_open(fi, cx);
        let viewed = self.is_viewed(&self.files[fi]);
        let f = &self.files[fi];
        let path = f.file.path.clone();
        let dir = f.file.dir().to_string();
        let name = f.file.name().to_string();
        let path_color = if collapsed || viewed { p.text_muted } else { p.text };
        let focused_current = fi == self.current && !pinned_is_none(ix) && self.focused_once;
        let id_base = if pinned { "pin".to_string() } else { format!("h{ix}") };
        let group = SharedString::from(format!("hdr-{id_base}"));

        let mut h = div()
            .id(SharedString::from(format!("{id_base}-hdr")))
            .group(group.clone())
            .h(px(30.))
            .flex()
            .flex_row()
            .items_center()
            .pl(px(6.))
            .pr(px(8.))
            .gap(px(6.))
            .bg(p.header_bg)
            .border_1()
            .border_color(if focused_current {
                alpha(p.accent, 0.6)
            } else {
                p.card_border
            });
        h = if collapsed { h.rounded(px(CARD_RADIUS)) } else { h.rounded_t(px(CARD_RADIUS)) };

        let chevron = div()
            .size(px(22.))
            .flex_none()
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .group_hover(group.clone(), |s| s.bg(p.surface_hover))
            .child(
                icon("chevron_down", 13., p.text_muted)
                    .with_transformation(gpui::Transformation::rotate(gpui::radians(if collapsed {
                        -std::f32::consts::FRAC_PI_2
                    } else {
                        0.
                    }))),
            );
        let tok = diffr_fileicon::token(&path);
        let ficon = if f.file.directory {
            icon("folder", 14., p.text_muted)
        } else {
            gpui::svg()
                .path(SharedString::from(diffr_fileicon::icon_path(tok)))
                .size(px(14.))
                .flex_none()
                .text_color(crate::theme::hex(diffr_fileicon::color(tok, p.dark)))
        };
        let mut title = div().flex().flex_row().items_center().flex_1().min_w_0().gap(px(6.)).child(
            div()
                .flex()
                .flex_row()
                .font_family(code_font.clone())
                .text_size(px(13.))
                .whitespace_nowrap()
                .overflow_hidden()
                .child(div().text_color(p.text_muted).child(dir))
                .child(div().text_color(path_color).font_weight(FontWeight::SEMIBOLD).child(name.clone())),
        );
        if f.file.old_path != f.file.path && !f.file.old_path.is_empty() {
            title = title.child(
                div()
                    .font_family(code_font.clone())
                    .text_size(px(11.))
                    .text_color(p.text_muted)
                    .whitespace_nowrap()
                    .overflow_hidden().text_ellipsis().min_w_0().max_w(px(200.))
                    .child(format!("← {}", f.file.old_path)),
            );
        }
        let toggle = div()
            .id(SharedString::from(format!("{id_base}-toggle")))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .flex_1()
            .min_w_0()
            .h(px(30.))
            .cursor_pointer()
            .child(chevron)
            .child(ficon)
            .child(title)
            .tooltip(tooltip(path.clone()))
            .on_click(cx.listener(move |this, _, _, cx| {
                let collapsed = this.files[fi].collapsed && !this.force_open(fi, cx);
                this.files[fi].collapsed = !collapsed;
                if collapsed && this.searching(cx) {
                    this.file_matches.insert(fi);
                }
                this.rows_dirty = true;
                cx.notify();
            }));
        h = h.child(toggle);

        // The copy button shows while the pointer is over the header.
        let path2 = path.clone();
        let name2 = name.clone();
        h = h.child(
            icon_button(SharedString::from(format!("{id_base}-copy")), "copy", "Copy path", cx)
                .size(px(24.))
                .opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(path2.clone()));
                    this.toast(format!("Copied {name2}"), cx);
                })),
        );
        let deleted = f.file.status == Status::Deleted || f.file.directory;
        let line = f.first_line();
        let path3 = path.clone();
        let mut open = icon_button(
            SharedString::from(format!("{id_base}-open")),
            "open",
            if f.file.status == Status::Deleted { "Deleted files cannot be opened" } else { "Open file in editor" },
            cx,
        ).size(px(24.));
        if deleted {
            open = open.opacity(0.4);
        } else {
            open = open.on_click(cx.listener(move |this, _, _, cx| this.open_in_editor(&path3, line, cx)));
        }
        h = h.child(open);
        if f.countable() && (f.file.additions > 0 || f.file.deletions > 0) {
            h = h.child(
                div()
                    .id(SharedString::from(format!("{id_base}-counts")))
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .px(px(4.))
                    .py(px(2.))
                    .flex_none()
                    .font_family(code_font.clone())
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().text_color(if f.file.additions == 0 { p.text_muted } else { p.add_text }).child(format!("+{}", ui::thousands(f.file.additions))))
                    .child(div().text_color(if f.file.deletions == 0 { p.text_muted } else { p.del_text }).child(format!("-{}", ui::thousands(f.file.deletions))))
                    .tooltip(tooltip(format!(
                        "{}, {}",
                        ui::lines_text(f.file.additions, "added"),
                        ui::lines_text(f.file.deletions, "removed")
                    ))),
            );
        }
        if f.file.generated {
            h = h.child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .bg(alpha(p.reference, 0.15))
                    .text_color(p.reference)
                    .flex_none()
                    .child("Generated"),
            );
        }
        if let Some((count, critical)) = self.file_note_count(&path).filter(|_| self.show_notes()) {
            h = h.child(
                div()
                    .id(SharedString::from(format!("{id_base}-notes")))
                    .flex_none()
                    .child(icon("sparkle", 13., if critical { p.danger } else { p.accent }))
                    .tooltip(tooltip(ui::plural(count, "AI note"))),
            );
        }
        let threads = self.open_threads(&path);
        if threads > 0 {
            h = h.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .px(px(4.))
                    .py(px(2.))
                    .text_color(p.text_muted)
                    .flex_none()
                    .child(icon("comment", 12., p.text_muted))
                    .child(
                        div()
                            .font_family(code_font.clone())
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(threads.to_string()),
                    ),
            );
        }
        // Staged: in git's index, for the next commit.
        if self.can_stage() && !(f.file.directory && f.file.path.starts_with("Untracked files not shown")) {
            let st = self.stage_of(&self.files[fi]);
            let on = st.is_some();
            let mut sb = div()
                .id(SharedString::from(format!("{id_base}-staged")))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .h(px(24.))
                .px(px(5.))
                .rounded(px(5.))
                .flex_none()
                .hover(|s| s.bg(p.surface_hover))
                .cursor_pointer()
                .child(crate::view::check_mark(st == Some(diffr_git::Staged::Full), st == Some(diffr_git::Staged::Partial), cx).size(px(13.)))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if on { p.accent } else { p.text })
                        .child(if st == Some(diffr_git::Staged::Partial) { "Partly staged" } else if on { "Staged" } else { "Stage" }),
                )
                .tooltip(tooltip(if on { "Unstage: take it out of the next commit (S)" } else { "Stage for the next commit (S)" }))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_stage(fi, cx)));
            if on {
                sb = sb.border_1().border_color(alpha(p.accent, 0.44));
            }
            h = h.child(sb);
        }
        // Viewed: marking a file viewed collapses it.
        let mut vb = div()
            .id(SharedString::from(format!("{id_base}-viewed")))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .h(px(24.))
            .px(px(5.))
            .rounded(px(5.))
            .flex_none()
            .hover(|s| s.bg(p.surface_hover))
            .cursor_pointer()
            .child(check_box(viewed, p.viewed, cx).size(px(13.)))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if viewed { p.viewed } else { p.text })
                    .child("Viewed"),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                let v = this.is_viewed(&this.files[fi]);
                this.set_viewed(fi, !v, cx);
            }));
        if viewed {
            vb = vb.border_color(alpha(p.viewed, 0.44));
        }
        h = h.child(vb);
        let h = h.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                this.open_context_menu(crate::menus::ContextMenu::File(fi), ev.position, cx);
            }),
        );
        // The band above the header spaces the cards, and hides the lines
        // scrolling under it while it is pinned.
        div().pt(px(8.)).bg(p.app_bg).child(h).into_any_element()
    }

    fn gap_row(&mut self, ix: usize, r: &Row, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let fi = r.file;
        let mut gutter = self.gutter_width(fi, window);
        let f = &self.files[fi];
        if !self.split_file(f) && !f.one_sided() {
            gutter = 2. * gutter - 4.;
        }
        let first = r.gap == 0;
        let last = r.gap as usize == f.file.hunks.len();
        let n = r.count;
        let can = f.can_expand();
        let gap = r.gap as usize;
        let expand = move |this: &mut Diffr, top: u32, bottom: u32, cx: &mut Context<Diffr>| {
            let f = &mut this.files[fi];
            let s = f.expanded.entry(gap).or_default();
            s.top += top;
            s.bottom += bottom;
            this.rows_dirty = true;
            cx.notify();
        };
        let button = |id: String, name: &'static str, tip: String, top: u32, bottom: u32, cx: &mut Context<Diffr>| {
            let mut b = div()
                .id(SharedString::from(id))
                .flex_1()
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .border_r_2()
                .border_color(p.code_bg)
                .child(icon(name, 14., p.gap_text))
                .tooltip(tooltip(tip));
            if can {
                let hover = p.surface_hover;
                b = b
                    .hover(move |s| s.bg(hover))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| expand(this, top, bottom, cx)));
            }
            b
        };
        let mut buttons = div().flex().flex_row().w(px(gutter + 4.)).h(px(30.)).flex_none();
        if n <= EXPAND_STEP && first {
            buttons = buttons.child(button(format!("g{ix}a"), "arrow_up", "Show the lines above".into(), n, 0, cx));
        } else if n <= EXPAND_STEP && last {
            buttons = buttons.child(button(format!("g{ix}a"), "arrow_down", "Show the lines below".into(), n, 0, cx));
        } else if n <= EXPAND_STEP {
            buttons = buttons.child(button(format!("g{ix}a"), "expand", "Show the lines".into(), n, 0, cx));
        } else {
            if !first {
                buttons = buttons.child(button(
                    format!("g{ix}d"),
                    "arrow_down",
                    format!("Show {EXPAND_STEP} more lines"),
                    EXPAND_STEP,
                    0,
                    cx,
                ));
            }
            if !last {
                buttons = buttons.child(button(
                    format!("g{ix}u"),
                    "arrow_up",
                    format!("Show {EXPAND_STEP} more lines"),
                    0,
                    EXPAND_STEP,
                    cx,
                ));
            }
        }
        let text = if n == 1 { "1 unmodified line".to_string() } else { format!("{n} unmodified lines") };
        let mut label = div()
            .id(SharedString::from(format!("g{ix}l")))
            .px(px(12.))
            .h(px(30.))
            .flex()
            .items_center()
            .text_size(px(12.))
            .text_color(p.gap_text)
            .child(text);
        if can {
            label = label.cursor_pointer().hover(|s| s.underline()).on_click(cx.listener(move |this, _, _, cx| {
                if n <= EXPAND_STEP {
                    expand(this, n, 0, cx)
                } else if first {
                    expand(this, 0, EXPAND_STEP, cx)
                } else if last {
                    expand(this, EXPAND_STEP, 0, cx)
                } else {
                    expand(this, EXPAND_STEP, EXPAND_STEP, cx)
                }
            }));
        }
        let mut row = card(&p).h(px(30.)).bg(p.gap_bg).items_center().child(buttons).child(label);
        if n > EXPAND_STEP {
            let mut b = div()
                .id(SharedString::from(format!("g{ix}all")))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.gap_text)
                .child("Expand all");
            if can {
                let hover = p.surface_hover;
                b = b
                    .hover(move |s| s.bg(hover))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| expand(this, n, 0, cx)));
            }
            row = row.child(b);
        }
        if let Some(h) = f.file.hunks.get(gap)
            && !h.section.is_empty()
        {
            row = row.child(
                div()
                    .font_family(ui::code_font(cx))
                    .text_size(px(12.))
                    .text_color(p.gap_text)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .flex_1()
                    .min_w_0()
                    .child(h.section.clone()),
            );
        }
        row.into_any_element()
    }

    fn line_row(&mut self, ix: usize, r: &Row, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let fi = r.file;
        let gutter = self.gutter_width(fi, window);
        let lh = self.line_height();
        let split = self.split_file(&self.files[fi]);
        self.files[fi].prepare_words(r, split);
        let one_sided = self.files[fi].one_sided();
        let hs = if self.settings.word_wrap { 0. } else { self.hscroll.get(&self.files[fi].file.path).copied().unwrap_or(0.) };
        let path = self.files[fi].file.path.clone();
        let char_w = self.char_width(window);
        let max_cols = self.files[fi].metrics().max_cols;
        let mut row = card(&p).items_stretch().min_h(px(lh)).id(SharedString::from(format!("l{ix}")));
        if !self.settings.word_wrap {
            let path = path.clone();
            row = row.on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, window, cx| {
                let d = ev.delta.pixel_delta(px(20.));
                if d.x.abs() <= d.y.abs() {
                    return;
                }
                // The row's width: the list's, less its margins.
                let w = f32::from(this.list.viewport_bounds().size.width) - 24.;
                let _ = &window;
                let code = if split { w / 2. - gutter - 30. } else { w - 2. * gutter - 40. };
                let limit = (max_cols as f32 * char_w - code + 40.).max(0.);
                let cur = this.hscroll.get(&path).copied().unwrap_or(0.);
                this.hscroll.insert(path.clone(), (cur - f32::from(d.x)).clamp(0., limit));
                cx.stop_propagation();
                cx.notify();
            }));
        }
        if !split {
            let s = self.files[fi].side_of(r, Side::New, false);
            let sd = if s.kind == Some(LineKind::Del) { Side::Old } else { Side::New };
            let cell = self.line_cell(ix, fi, r, &s, sd, gutter, hs, !one_sided, cx);
            return row.child(cell).into_any_element();
        }
        let left = self.files[fi].side_of(r, Side::Old, true);
        let right = self.files[fi].side_of(r, Side::New, true);
        let lc = self.line_cell(ix, fi, r, &left, Side::Old, gutter, hs, false, cx);
        let rc = self.line_cell(ix, fi, r, &right, Side::New, gutter, hs, false, cx);
        row.child(div().flex().flex_row().flex_1().flex_basis(px(0.)).min_w_0().items_stretch().child(lc))
            .child(div().w(px(1.)).flex_none().bg(p.card_border))
            .child(div().flex().flex_row().flex_1().flex_basis(px(0.)).min_w_0().items_stretch().child(rc))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn line_cell(
        &self,
        ix: usize,
        fi: usize,
        r: &Row,
        s: &LineSide,
        sd: Side,
        gutter: f32,
        hs: f32,
        unified: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = pal(cx).clone();
        let lh = self.line_height();
        let code_font = SharedString::from(self.code_font_name());
        if !s.present {
            return div().flex_1().bg(p.empty_side).min_h(px(lh)).into_any_element();
        }
        let (mut bg, mut gutter_bg, mut bar, num_color, word_color) = match s.kind {
            Some(LineKind::Add) => (p.add_bg, p.add_gutter, Some(p.add_bar), p.add_text, Some(p.add_word)),
            Some(LineKind::Del) => (p.del_bg, p.del_gutter, Some(p.del_bar), p.del_text, Some(p.del_word)),
            _ => (p.code_bg, p.code_bg, None, p.line_number, None),
        };
        // The hunk chosen with the keyboard: a blue bar at its side.
        if self.is_selected_line(fi, s) {
            bar = Some(p.accent);
        }
        if self.in_line_drag(fi, sd, s.num) {
            bg = over(bg, p.selected);
            gutter_bg = over(gutter_bg, p.selected);
        }
        let hover_bg = over(bg, alpha(p.surface_hover, 0.5));
        let hover_gutter = over(gutter_bg, alpha(p.surface_hover, 0.5));
        let group = SharedString::from(format!("cell{ix}{}", if sd == Side::Old { "o" } else { "n" }));
        let code_size = self.code_size();
        let num = |n: u32, width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .pr(px(8.))
                .text_right()
                .font_family(code_font.clone())
                .text_size(px(code_size - 1.))
                .line_height(px(lh))
                .text_color(num_color)
                .child(if n > 0 { n.to_string() } else { String::new() })
        };
        let mut gut = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_start()
            .bg(gutter_bg)
            .group_hover(group.clone(), move |st| st.bg(hover_gutter))
            .child(div().w(px(4.)).self_stretch().bg(bar.unwrap_or(gpui::transparent_black())));
        if unified {
            let (old, new) = if r.hunk < 0 {
                (r.old, r.new)
            } else if let Some(l) = self.files[fi].line_at(r.hunk, s.index) {
                (l.old, l.new)
            } else {
                (0, 0)
            };
            gut = gut.child(num(old, gutter - 4.)).child(num(new, gutter - 16.));
        } else {
            gut = gut.child(num(s.num, gutter - 16.));
        }
        gut = gut.child(
            div().w(px(12.)).flex_none().text_center()
                .font_family(code_font.clone()).text_size(px(code_size - 1.))
                .line_height(px(lh)).text_color(num_color)
                .child(match s.kind {
                    Some(LineKind::Add) => "+",
                    Some(LineKind::Del) => "−",
                    _ => "",
                }),
        );
        // The marks: changed words, and matches of the find bar.
        let mut marks: Vec<Mark> = s
            .words
            .iter()
            .filter_map(|w| word_color.map(|c| Mark { range: *w, color: c }))
            .collect();
        if self.searching(cx) {
            let found = find_ranges(s.text, &self.query(cx));
            if !found.is_empty() {
                let color = if self.active_match(fi, s) { p.match_now } else { p.matched };
                marks.extend(found.into_iter().map(|range| Mark { range, color }));
            }
        }
        let (text, runs) = code_runs(s.text, s.segs, &marks, &p);
        let styled = StyledText::new(text).with_highlights(runs);
        let numbers = if unified { 2. * gutter - 4. } else { gutter };
        let path = self.files[fi].file.path.clone();
        let num_ = s.num;
        let text_owned = s.text.to_string();
        let wrap = self.settings.word_wrap;
        let mut code = div()
            .flex_1()
            .flex_basis(px(0.))
            .min_w_0()
            .overflow_hidden()
            .mx(px(10.))
            .font_family(code_font.clone())
            .text_size(px(code_size))
            .line_height(px(lh))
            .text_color(p.code);
        let inner = div().child(styled);
        let inner = if wrap { inner } else { inner.whitespace_nowrap().ml(px(-hs)) };
        code = code.child(inner);
        let path_c = path.clone();
        let plus = div()
            .id(SharedString::from(format!("plus{ix}{}", if sd == Side::Old { "o" } else { "n" })))
            .absolute()
            .top(px((lh - 18.) / 2.))
            .left(px(numbers - 9.))
            .size(px(18.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .justify_center()
            .bg(p.accent)
            .opacity(0.)
            .group_hover(group.clone(), |st| st.opacity(1.))
            .child(icon("plus", 13., p.accent_text))
            .tooltip(tooltip("Comment on this line"))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.add_comment(&path_c, sd, num_, num_, window, cx);
            }));
        let path_m = path.clone();
        div()
            .id(SharedString::from(format!("cell{ix}{}", if sd == Side::Old { "o" } else { "n" })))
            .group(group.clone())
            .relative()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .items_stretch()
            .bg(bg)
            .hover(move |st| st.bg(hover_bg))
            .cursor_pointer()
            .child(gut)
            .child(code)
            .child(plus)
            // A press starts a selection of lines, a drag extends it, and the
            // release comments on it: one line for a click.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if num_ > 0 {
                        this.line_drag = Some(LineDrag { file: fi, path: path.clone(), side: sd, start: num_, end: num_ });
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &gpui::MouseMoveEvent, _, cx| {
                if !ev.dragging() || num_ == 0 {
                    return;
                }
                if let Some(d) = this.line_drag.as_mut()
                    && d.file == fi
                    && d.side == sd
                    && d.end != num_
                {
                    d.end = num_;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    this.open_context_menu(
                        crate::menus::ContextMenu::Line { file: fi, path: path_m.clone(), side: sd, line: num_, text: text_owned.clone() },
                        ev.position,
                        cx,
                    );
                }),
            )
            .into_any_element()
    }

    fn comment_row(&mut self, r: &Row, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some(c) = self.comments.list.get(r.item).cloned() else {
            return div().into_any_element();
        };
        let Some(input) = self.comment_inputs.get(&c.id).cloned() else {
            return div().into_any_element();
        };
        let id = c.id;
        let user = self.user_name();
        let focused_border = self.comment_focused == Some(id);
        let mut box_ = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .rounded(px(12.))
            .border_1()
            .border_color(if focused_border { alpha(p.accent, 0.6) } else { p.card_border })
            .bg(p.header_bg)
            .overflow_hidden()
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .pl(px(10.))
                    .pr(px(4.))
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(p.card_border)
                    .child(div().text_size(px(12.)).font_weight(FontWeight::BOLD).text_color(p.text).child(user.clone()))
                    .child(div().text_size(px(11.)).text_color(p.text_muted).child(c.label()))
                    .child(ui::spacer())
                    .children(c.pending().then(|| {
                        icon_button(SharedString::from(format!("ccopy{id}")), "copy", "Copy this comment with its lines", cx)
                            .size(px(24.))
                            .on_click(cx.listener(move |this, _, _, cx| this.copy_comment(id, cx)))
                    }))
                    .child(
                        icon_button(SharedString::from(format!("cdel{id}")), "close", "Delete comment", cx)
                            .size(px(24.))
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_comment(id, cx))),
                    ),
            )
            .child(
                div()
                    .min_h(px(64.))
                    .px(px(10.))
                    .py(px(8.))
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .text_color(p.text)
                    .child(input),
            );
        if self.source.is_pull() {
            box_ = box_.child(self.pr_comment_actions(id, cx));
        }
        card(&p)
            .px(px(16.))
            .py(px(8.))
            .gap(px(10.))
            .items_start()
            .child(ui::avatar(&user, 28.))
            .child(box_)
            .into_any_element()
    }

    fn image_row(&self, fi: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let f = &self.files[fi];
        let pane = |label: &str, pic: &Option<Picture>, bar: Hsla| -> Div {
            let mut col = div()
                .flex()
                .flex_col()
                .flex_1()
                .flex_basis(px(0.))
                .min_w_0()
                .p(px(14.))
                .gap(px(8.))
                .items_center();
            let Some(pic) = pic else {
                return col.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_muted)
                        .child(format!("No {} image", label.to_lowercase())),
                );
            };
            let scale = (320. / pic.height.max(1) as f32).min(1.);
            col = col
                .child(
                    div()
                        .w(px(pic.width as f32 * scale))
                        .h(px(pic.height as f32 * scale))
                        .max_w_full()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(p.card_border)
                        .overflow_hidden()
                        .bg(rgba(127, 127, 127, 0.12))
                        .child(img(pic.image.clone()).size_full()),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .child(div().size(px(8.)).rounded(px(4.)).bg(bar))
                        .child(div().text_size(px(11.)).text_color(p.text_muted).child(format!(
                            "{label} · {}×{} · {}",
                            pic.width,
                            pic.height,
                            format_bytes(pic.bytes)
                        ))),
                );
            col
        };
        let added = matches!(f.file.status, Status::Added | Status::Untracked);
        let mut row = card(&p).items_stretch();
        if !added {
            row = row.child(pane("Old", &f.old_image, p.del_bar));
        }
        if f.file.status != Status::Deleted && !added {
            row = row.child(div().w(px(1.)).bg(p.card_border));
        }
        if f.file.status != Status::Deleted {
            row = row.child(pane("New", &f.new_image, p.add_bar));
        }
        row.into_any_element()
    }

    /// The message of the commit reviewed.
    pub fn commit_message(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some(cm) = self.commit.as_ref() else {
            return div().into_any_element();
        };
        let ago = relative_time(now_secs(), cm.time);
        let mut col = div()
            .flex()
            .flex_col()
            .px(px(16.))
            .py(px(12.))
            .gap(px(6.))
            .rounded(px(CARD_RADIUS))
            .bg(p.header_bg)
            .border_1()
            .border_color(p.card_border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .child(ui::avatar(&cm.author, 26.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(p.text)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(cm.subject.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(p.text_muted)
                                    .child(format!("{} committed {}", cm.author, ago)),
                            ),
                    )
                    .child(
                        div()
                            .font_family(ui::code_font(cx))
                            .text_size(px(12.))
                            .text_color(p.reference)
                            .child(short_hash(&cm.hash).to_string()),
                    ),
            );
        if !cm.body.is_empty() {
            col = col.child(
                div()
                    .pt(px(4.))
                    .pl(px(36.))
                    .text_size(px(13.))
                    .text_color(alpha(p.text, 0.85))
                    .child(cm.body.clone()),
            );
        }
        col.into_any_element()
    }

    pub fn copied_recently(&self) -> bool {
        self.copied_at.is_some_and(|t: Instant| t.elapsed() < Duration::from_secs(2))
    }
}

fn pinned_is_none(ix: usize) -> bool {
    ix == usize::MAX
}

/// A row inside a file's card, between its sides.
pub fn card(p: &Palette) -> Div {
    div()
        .w_full()
        .flex()
        .flex_row()
        .bg(p.code_bg)
        .border_l_1()
        .border_r_1()
        .border_color(p.card_border)
}
