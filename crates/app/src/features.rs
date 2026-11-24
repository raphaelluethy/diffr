//! Grouping the files (by kind, or as an agent's review has them), the
//! notes of the agent's review on their lines, and the glue to the pull
//! requests and the AI review.

use std::collections::{HashMap, HashSet};

use diffr_git::{File, LineKind};
use gpui::{AnyElement, App, Context, FontWeight, SharedString, Window, div, prelude::*, px};

use crate::state::*;
use crate::theme::{alpha, rgba};
use crate::ui::{self, icon, pal, segment, segment_track, tooltip};
use crate::window::Diffr;

pub use crate::pull::{Ext, PulledRequest, fetch_pull};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GroupMode {
    /// By the agent's review once there is one, else by kind for pull
    /// requests.
    #[default]
    Auto,
    None,
    Kind,
    Ai,
}

#[derive(Clone, Debug)]
pub struct FileGroup {
    pub key: String,
    pub label: String,
    pub summary: String,
    pub critical: bool,
    pub ai: bool,
}

/// A note of the agent's review: on a line, or on a whole file.
#[derive(Clone, Debug, PartialEq)]
pub struct AiNote {
    pub path: String,
    pub file: bool,
    pub side: Side,
    pub line: u32,
    pub text: String,
    pub critical: bool,
}

/// The threads and notes the rows show under their lines, as the rows are
/// built: the others show at the end of their files.
#[derive(Default)]
pub struct Placement {
    pub threads: HashSet<i64>,
    pub notes: HashSet<(String, usize)>,
}

const KINDS: [(&str, &str, &str); 7] = [
    ("docs", "Docs", "Documentation updates."),
    ("code", "Code", "Source changes."),
    ("tests", "Tests", "Test coverage for the change."),
    ("config", "Config", "Configuration changes."),
    ("deps", "Dependencies", "Dependency version changes."),
    ("generated", "Generated", "Generated or build output changes."),
    ("other", "Other", "Other changes that don't fit an existing category."),
];

pub fn kind_of(f: &File) -> usize {
    let index = |key: &str| KINDS.iter().position(|k| k.0 == key).unwrap_or(1);
    if f.directory {
        return index("generated");
    }
    if f.binary {
        return index("other");
    }
    let p = f.path.to_lowercase();
    let parts: Vec<&str> = p.split('/').collect();
    let base = parts[parts.len() - 1];
    let dirs = &parts[..parts.len() - 1];
    let in_dir = |names: &[&str]| dirs.iter().any(|d| names.contains(d));
    let ext = match base.rfind('.') {
        Some(i) if i > 0 => &base[i..],
        _ => "",
    };
    if [".md", ".mdx", ".mdc", ".rst", ".adoc"].contains(&ext) || in_dir(&["docs"]) || base.starts_with("readme") {
        return index("docs");
    }
    if base.contains(".test.")
        || base.contains(".spec.")
        || in_dir(&["__tests__", "test", "tests", "testdata"])
        || base.ends_with("_test.go")
        || base.ends_with("_test.py")
        || (base.starts_with("test_") && ext == ".py")
        || base.ends_with("_spec.rb")
    {
        return index("tests");
    }
    if (dirs.is_empty()
        && (base.contains(".config.")
            || base.starts_with(".git")
            || (base.starts_with('.') && (base.ends_with("rc") || base.contains("rc.")))))
        || dirs.first() == Some(&".github")
        || (base.starts_with("tsconfig") && ext == ".json")
        || base == "dockerfile"
        || base == ".dockerignore"
        || base == "makefile"
        || (base.starts_with("docker-compose") && (ext == ".yml" || ext == ".yaml"))
    {
        return index("config");
    }
    if ["package.json", "pnpm-workspace.yaml", "cargo.toml", "go.mod", "gemfile", "requirements.txt"].contains(&base) {
        return index("deps");
    }
    if f.generated || in_dir(&["dist"]) || ext == ".lock" || base.contains(".generated.") {
        return index("generated");
    }
    index("code")
}

/// Compares paths as git sorts them: directories before the files beside
/// them.
fn compare_paths(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<&str> = a.split('/').collect();
    let pb: Vec<&str> = b.split('/').collect();
    for i in 0..pa.len().min(pb.len()) {
        let a_dir = i + 1 < pa.len();
        let b_dir = i + 1 < pb.len();
        if pa[i] != pb[i] || a_dir != b_dir {
            if a_dir != b_dir {
                return if a_dir { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
            }
            return pa[i].cmp(pb[i]);
        }
    }
    pa.len().cmp(&pb.len())
}

impl Diffr {
    pub fn grouping(&self) -> GroupMode {
        let an = self.analysis_result().is_some();
        let mut mode = self.ext.group_pref;
        if mode == GroupMode::Ai && !an {
            mode = GroupMode::Auto;
        }
        if mode != GroupMode::Auto {
            return mode;
        }
        if an {
            GroupMode::Ai
        } else if self.source.is_pull() {
            GroupMode::Kind
        } else {
            GroupMode::None
        }
    }

    pub fn choose_grouping(&mut self, mode: GroupMode, cx: &mut Context<Self>) {
        if mode == GroupMode::Ai && self.analysis_result().is_none() {
            // No review yet: the card offers one, atop the files.
            self.ext.group_pref = mode;
            self.ext.show_ai_card = true;
            self.rows_dirty = true;
            self.list.scroll_to(gpui::ListOffset { item_ix: 0, offset_in_item: px(0.) });
            cx.notify();
            return;
        }
        self.ext.group_pref = mode;
        self.regroup(cx);
        cx.notify();
    }

    pub fn regroup(&mut self, _cx: &mut Context<Self>) {
        let path = |files: &[FileState], i: i32| {
            if i >= 0 && (i as usize) < files.len() { files[i as usize].file.path.clone() } else { String::new() }
        };
        let current = path(&self.files, self.current as i32);
        let selected = path(&self.files, self.sel_file);
        let mut groups: Vec<FileGroup> = Vec::new();
        for f in &mut self.files {
            f.group = -1;
        }
        let mode = self.grouping();
        // The agent's reading order: the files a reviewer should read first.
        let order: HashMap<String, usize> = match (mode, self.analysis_result()) {
            (GroupMode::Ai, Some(an)) => an.reading_order().into_iter().enumerate().map(|(i, p)| (p, i)).collect(),
            _ => HashMap::new(),
        };
        match mode {
            GroupMode::Kind => {
                let ranks: Vec<usize> = self.files.iter().map(|f| kind_of(&f.file)).collect();
                let mut used = [-1i32; 7];
                for (k, kind) in KINDS.iter().enumerate() {
                    if ranks.contains(&k) {
                        used[k] = groups.len() as i32;
                        groups.push(FileGroup {
                            key: kind.0.into(),
                            label: kind.1.into(),
                            summary: kind.2.into(),
                            critical: false,
                            ai: false,
                        });
                    }
                }
                for (i, f) in self.files.iter_mut().enumerate() {
                    f.group = used[ranks[i]];
                }
            }
            GroupMode::Ai => {
                let an = self.analysis_result().cloned().unwrap_or_default();
                let mut by_path: HashMap<String, usize> = HashMap::new();
                for (gi, g) in an.groups.iter().enumerate() {
                    groups.push(FileGroup {
                        key: g.key.clone(),
                        label: g.label.clone(),
                        summary: g.summary.clone(),
                        critical: g.critical,
                        ai: true,
                    });
                    for p in &g.file_paths {
                        by_path.entry(p.clone()).or_insert(gi);
                    }
                }
                let mut rest = -1i32;
                for f in &mut self.files {
                    let gi = by_path
                        .get(&f.file.path)
                        .or_else(|| (f.file.old_path != f.file.path).then(|| by_path.get(&f.file.old_path)).flatten())
                        .map(|g| *g as i32);
                    f.group = match gi {
                        Some(g) => g,
                        None => {
                            if rest < 0 {
                                rest = groups.len() as i32;
                                groups.push(FileGroup {
                                    key: "unreviewed".into(),
                                    label: "Not in the review".into(),
                                    summary: "Files that changed since the agent's review.".into(),
                                    critical: false,
                                    ai: true,
                                });
                            }
                            rest
                        }
                    };
                }
                let mut count = vec![0; groups.len()];
                for f in &self.files {
                    count[f.group as usize] += 1;
                }
                let mut remap = vec![-1i32; groups.len()];
                let mut kept = Vec::new();
                for (gi, g) in groups.into_iter().enumerate() {
                    if count[gi] > 0 {
                        remap[gi] = kept.len() as i32;
                        kept.push(g);
                    }
                }
                groups = kept;
                for f in &mut self.files {
                    f.group = remap[f.group as usize];
                }
            }
            _ => {}
        }
        self.files.sort_by(|a, b| {
            a.group.cmp(&b.group).then_with(|| match (order.get(&a.file.path), order.get(&b.file.path)) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => compare_paths(&a.file.path, &b.file.path),
            })
        });
        self.current = 0;
        self.sel_file = -1;
        for (i, f) in self.files.iter().enumerate() {
            if f.file.path == current {
                self.current = i;
            }
            if f.file.path == selected {
                self.sel_file = i as i32;
            }
        }
        if self.sel_file < 0 {
            self.sel_hunk = -1;
        }
        self.ext.groups = groups;
        self.build_notes();
        self.matches_for = "\0".into();
        self.build_tree();
        self.rows_dirty = true;
    }

    pub fn group_labels(&self) -> Vec<(String, String)> {
        self.ext.groups.iter().map(|g| (g.key.clone(), g.label.clone())).collect()
    }

    pub fn group_icon(&self, gi: usize) -> (&'static str, bool, String) {
        match self.ext.groups.get(gi) {
            Some(g) => (if g.ai { "sparkle" } else { "layers" }, g.critical, g.summary.clone()),
            None => ("layers", false, String::new()),
        }
    }

    fn group_files(&self, gi: i32) -> Vec<usize> {
        (0..self.files.len()).filter(|&i| self.files[i].group == gi).collect()
    }

    fn reveal_group(&mut self, gi: usize, cx: &mut Context<Self>) {
        if self.rows_dirty {
            self.build_rows(cx);
        }
        if let Some(r) = self.rows.iter().position(|r| r.kind == RowKind::Group && r.gap == gi as i32) {
            self.list.scroll_to(gpui::ListOffset { item_ix: r, offset_in_item: px(0.) });
            self.current = self.rows[r].file;
            self.scroll_selection_dirty = false;
        }
        cx.notify();
    }

    /// The chips of the groups above the files, with how much of each was
    /// viewed.
    pub fn scope_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.ext.groups.is_empty() {
            return None;
        }
        let p = pal(cx).clone();
        let active = self.files.get(self.current).map(|f| f.group).unwrap_or(-1);
        let mut viewed = vec![0; self.ext.groups.len()];
        let mut total = vec![0; self.ext.groups.len()];
        for f in &self.files {
            if f.group < 0 {
                continue;
            }
            total[f.group as usize] += 1;
            if self.is_viewed(f) {
                viewed[f.group as usize] += 1;
            }
        }
        let mut row = div().flex().flex_row().flex_wrap().gap(px(6.));
        for (gi, g) in self.ext.groups.iter().enumerate() {
            let tip = if g.summary.is_empty() { g.label.clone() } else { format!("{}: {}", g.label, g.summary) };
            let done = viewed[gi] == total[gi] && total[gi] > 0;
            let mut b = div()
                .id(SharedString::from(format!("scope-{}", g.key)))
                .h(px(26.))
                .pl(px(8.))
                .pr(px(10.))
                .gap(px(6.))
                .flex()
                .flex_row()
                .items_center()
                .rounded(px(13.))
                .border_1()
                .border_color(rgba(127, 127, 127, 0.18))
                .flex_none()
                .cursor_pointer()
                .child(icon(if g.ai { "sparkle" } else { "layers" }, 12., if g.critical { p.danger } else { p.text_muted }))
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).whitespace_nowrap().child(g.label.clone()))
                .child(
                    div()
                        .font_family(ui::code_font(cx))
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if done { p.viewed } else { p.text_muted })
                        .child(format!("{}/{}", viewed[gi], total[gi])),
                )
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |this, _, _, cx| this.reveal_group(gi, cx)));
            if gi as i32 == active {
                b = b.bg(alpha(p.accent, 0.14)).border_color(alpha(p.accent, 0.45));
            } else {
                let hover = p.surface_hover;
                b = b.hover(move |s| s.bg(hover));
            }
            row = row.child(b);
        }
        Some(
            div()
                .id("scope-bar")
                .flex_none()
                .pt(px(8.))
                .px(px(12.))
                .child(row)
                .into_any_element(),
        )
    }

    /// The header of a group of files.
    pub fn group_row(&mut self, gi: i32, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some(g) = self.ext.groups.get(gi as usize).cloned() else {
            return div().h(px(0.)).into_any_element();
        };
        let files = self.group_files(gi);
        let (mut viewed, mut adds, mut dels) = (0, 0, 0);
        let mut all_collapsed = true;
        for &i in &files {
            let f = &self.files[i];
            if self.is_viewed(f) {
                viewed += 1;
            }
            if !f.collapsed {
                all_collapsed = false;
            }
            if f.countable() {
                adds += f.file.additions;
                dels += f.file.deletions;
            }
        }
        let n = files.len();
        let mut head = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(icon(if g.ai { "sparkle" } else { "layers" }, 15., if g.critical { p.danger } else { p.text_muted }))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .flex_shrink(1.)
                    .min_w_0()
                    .child(g.label.clone()),
            );
        if g.critical {
            head = head.child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(10.))
                    .bg(alpha(p.danger, 0.14))
                    .text_color(p.danger_foreground)
                    .flex_none()
                    .child("Needs care"),
            );
        }
        let label = if all_collapsed { "Expand" } else { "Collapse" };
        head = head
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().text_color(p.text_muted).child(format!("{} · ", ui::plural(n, "file"))))
                    .child(div().text_color(p.add_text).child(format!("+{}", ui::thousands(adds))))
                    .child(div().text_color(p.del_text).child(format!(" −{}", ui::thousands(dels)))),
            )
            .child(ui::spacer());
        // A group of the review of the work tree is a commit to make.
        if self.can_stage() && n > 0 {
            let (all, some) = crate::commit::tree_group_state(self, &files);
            head = head.child(
                div()
                    .id(SharedString::from(format!("grp-stage-{gi}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .flex_none()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if some { p.accent } else { p.text_muted })
                    .hover(|s| s.bg(p.surface_hover))
                    .cursor_pointer()
                    .child(crate::view::check_mark(all, some && !all, cx).size(px(13.)))
                    .child(if all { "Staged" } else { "Stage group" })
                    .tooltip(tooltip(if all { "Unstage the group's files" } else { "Stage the group's files for the next commit" }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let files = this.group_files(gi);
                        this.set_staged(files, !all, cx);
                    })),
            );
            if g.ai
                && let Some(cg) = self.review_commit_groups().into_iter().find(|c| c.key == g.key)
            {
                let tip = match &cg.message {
                    Some(m) => format!("Stage only this group and commit it as:\n{}", m.subject()),
                    None => "Stage only this group, and have the AI write its message".to_string(),
                };
                head = head.child(
                    div()
                        .id(SharedString::from(format!("grp-commit-{gi}")))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .flex_none()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text_muted)
                        .hover(|s| s.bg(p.surface_hover))
                        .cursor_pointer()
                        .child(icon("commit", 13., p.text_muted))
                        .child("Commit…")
                        .tooltip(tooltip(tip))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_commit_group(cg.clone(), cx);
                            if !this.commit_open {
                                this.toggle_commit(window, cx);
                            }
                        })),
                );
            }
        }
        head = head
            .child(
                div()
                    .text_size(px(12.))
                    .flex_none()
                    .text_color(if viewed == n && n > 0 { p.viewed } else { p.text_muted })
                    .child(format!("{viewed}/{n} viewed")),
            )
            .child(
                div()
                    .id(SharedString::from(format!("grp-col-{gi}")))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text_muted)
                    .hover(|s| s.bg(p.surface_hover))
                    .cursor_pointer()
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        for i in this.group_files(gi) {
                            this.files[i].collapsed = !all_collapsed;
                        }
                        this.rows_dirty = true;
                        cx.notify();
                    })),
            );
        // The agent's group: its diagram and its labs' other fields.
        let agent_group = if g.ai {
            self.analysis_result().and_then(|a| a.groups.iter().find(|x| x.key == g.key).cloned())
        } else {
            None
        };
        let mut col = div().w_full().flex().flex_col().pt(px(20.)).px(px(2.)).gap(px(4.)).bg(p.app_bg).child(head);
        if !g.summary.is_empty() {
            col = col.child(
                div()
                    .pl(px(23.))
                    .text_size(px(13.))
                    .child(crate::markdown::markdown(format!("grp-sum-{gi}"), &g.summary, alpha(p.text, 0.8), cx)),
            );
        }
        if self.can_stage()
            && let Some(m) = agent_group.as_ref().and_then(|a| a.commit.as_ref())
        {
            col = col.child(
                div()
                    .pl(px(23.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("commit", 12., p.text_muted))
                    .child(
                        div()
                            .font_family(ui::code_font(cx))
                            .text_size(px(12.))
                            .text_color(p.text_muted)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(m.subject()),
                    ),
            );
        }
        if let Some(ag) = agent_group.filter(|_| !all_collapsed) {
            for (name, value) in &ag.lab_fields {
                if name == diffr_agent::DIAGRAM_FIELD || value.trim().is_empty() {
                    continue;
                }
                col = col.child(div().pl(px(23.)).text_size(px(13.)).child(crate::markdown::markdown(
                    format!("grp-lab-{gi}-{name}"),
                    value,
                    alpha(p.text, 0.85),
                    cx,
                )));
            }
            if !ag.diagram.trim().is_empty() {
                let d = diffr_agent::Diagram { title: String::new(), caption: String::new(), mermaid: ag.diagram.clone() };
                col = col.child(div().pl(px(23.)).pt(px(6.)).child(self.diagram_card(&format!("gd{gi}"), &g.label, &d, cx)));
            }
        }
        col.into_any_element()
    }

    /// Switches how the files are grouped: none, by kind, by the agent's
    /// review.
    pub fn group_control(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let available = f32::from(window.viewport_size().width)
            - if self.sidebar_shown { self.sidebar_width } else { 0. };
        if available < 560. {
            return ui::icon_button("compact-grouping", "layers", "Change file grouping (Ctrl+K)", cx)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_palette(window, cx);
                    this.palette_input.update(cx, |i, cx| i.set_text("Group", cx));
                })).into_any_element();
        }
        let p = pal(cx).clone();
        let mode = self.grouping();
        let has_ai = self.analysis_result().is_some();
        let running = self.analysis_running();
        let mut track = segment_track()
            .child(segment("grp-none", "list", "No groups", mode == GroupMode::None, cx).on_click(cx.listener(
                |this, _, _, cx| this.choose_grouping(GroupMode::None, cx),
            )))
            .child(segment("grp-kind", "layers", "Group by kind", mode == GroupMode::Kind, cx).on_click(cx.listener(
                |this, _, _, cx| this.choose_grouping(GroupMode::Kind, cx),
            )));
        let tip = if has_ai { "Group by AI review" } else { "Review with AI (Ctrl+Shift+I)" };
        let ai = segment("grp-ai", "sparkle", tip, mode == GroupMode::Ai || self.ext.show_ai_card, cx).on_click(cx.listener(
            move |this, _, _, cx| {
                if has_ai {
                    this.choose_grouping(GroupMode::Ai, cx);
                    this.ext.show_ai_card = true;
                } else {
                    this.review_with_ai(cx);
                }
                cx.notify();
            },
        ));
        let ai = if running {
            ui::segment_with("grp-ai", ui::spinner("ai-seg-spin", 14., p.text_muted), tip, false, cx).into_any_element()
        } else {
            ai.into_any_element()
        };
        track = track.child(ai);
        track.into_any_element()
    }

    // ---- The notes of the agent's review ----

    pub fn build_notes(&mut self) {
        let sel = self.ext.note_sel.clone();
        self.ext.notes.clear();
        self.ext.note_sel = None;
        let Some((result, fps)) = self.analysis_with_fingerprints() else { return };
        let current: HashSet<String> = self
            .files
            .iter()
            .filter(|f| fps.get(&f.file.path).is_none_or(|fp| *fp == f.file.fingerprint))
            .map(|f| f.file.path.clone())
            .collect();
        let mut notes: HashMap<String, Vec<AiNote>> = HashMap::new();
        for g in &result.groups {
            for n in &g.file_notes {
                if current.contains(&n.path) {
                    notes.entry(n.path.clone()).or_default().push(AiNote {
                        path: n.path.clone(),
                        file: true,
                        side: Side::New,
                        line: 0,
                        text: n.text.clone(),
                        critical: n.critical,
                    });
                }
            }
            for n in &g.line_notes {
                if !current.contains(&n.path) {
                    continue;
                }
                notes.entry(n.path.clone()).or_default().push(AiNote {
                    path: n.path.clone(),
                    file: false,
                    side: if n.side == "deletions" { Side::Old } else { Side::New },
                    line: n.line.max(0) as u32,
                    text: n.text.clone(),
                    critical: n.critical,
                });
            }
        }
        self.ext.notes = notes;
        if let Some(sel) = sel
            && self.ext.notes.get(&sel.path).is_some_and(|v| v.contains(&sel))
        {
            self.ext.note_sel = Some(sel);
        }
    }

    pub fn file_note_count(&self, path: &str) -> Option<(usize, bool)> {
        let notes = self.ext.notes.get(path)?;
        if notes.is_empty() {
            return None;
        }
        Some((notes.len(), notes.iter().any(|n| n.critical)))
    }

    pub fn new_placement(&self) -> Placement {
        Placement::default()
    }

    /// Whether the review's notes show, as the setting has it.
    pub fn show_notes(&self) -> bool {
        matches!(self.settings.ai_file_info.as_str(), "notes" | "both")
    }

    /// Whether the files' summaries show.
    pub fn show_file_summaries(&self) -> bool {
        matches!(self.settings.ai_file_info.as_str(), "summaries" | "both")
    }

    pub fn file_summary(&self, path: &str) -> Option<String> {
        if !self.show_file_summaries() {
            return None;
        }
        self.analysis_result()?.file_summary(path).map(str::to_string)
    }

    pub fn append_file_notes(&self, rows: &mut Vec<Row>, fi: usize) {
        let path = &self.files[fi].file.path;
        if self.file_summary(path).is_some() {
            rows.push(Row::new(RowKind::FileSummary, fi));
        }
        if !self.show_notes() {
            return;
        }
        if let Some(notes) = self.ext.notes.get(path) {
            for (i, n) in notes.iter().enumerate() {
                if n.file {
                    let mut r = Row::new(RowKind::AiNote, fi);
                    r.item = i;
                    rows.push(r);
                }
            }
        }
    }

    /// Adds the threads and the notes on a line.
    pub fn append_line_extras(&self, rows: &mut Vec<Row>, fi: usize, l: &diffr_git::Line, placed: &mut Placement) {
        let path = &self.files[fi].file.path;
        let on = |sd: Side, line: u32| {
            (sd == Side::Old && l.kind != LineKind::Add && line == l.old)
                || (sd == Side::New && l.kind != LineKind::Del && line == l.new)
        };
        for (ti, t) in self.threads_of(path) {
            if t.outdated || t.line == 0 || placed.threads.contains(&t.root_id) {
                continue;
            }
            let sd = if t.side == diffr_github::Side::Left { Side::Old } else { Side::New };
            if on(sd, t.line) {
                placed.threads.insert(t.root_id);
                let mut r = Row::new(RowKind::Thread, fi);
                r.item = ti;
                rows.push(r);
            }
        }
        if let Some(notes) = self.ext.notes.get(path).filter(|_| self.show_notes()) {
            for (i, n) in notes.iter().enumerate() {
                if !n.file && !placed.notes.contains(&(path.clone(), i)) && on(n.side, n.line) {
                    placed.notes.insert((path.clone(), i));
                    let mut r = Row::new(RowKind::AiNote, fi);
                    r.item = i;
                    rows.push(r);
                }
            }
        }
    }

    /// Adds the threads and the notes of a file whose lines do not show.
    pub fn append_unplaced(&self, rows: &mut Vec<Row>, fi: usize, placed: &mut Placement) {
        let path = &self.files[fi].file.path;
        for (ti, t) in self.threads_of(path) {
            if !placed.threads.contains(&t.root_id) {
                let mut r = Row::new(RowKind::Thread, fi);
                r.item = ti;
                rows.push(r);
            }
        }
        if let Some(notes) = self.ext.notes.get(path).filter(|_| self.show_notes()) {
            for (i, n) in notes.iter().enumerate() {
                if !n.file && !placed.notes.contains(&(path.clone(), i)) {
                    let mut r = Row::new(RowKind::AiNote, fi);
                    r.item = i;
                    rows.push(r);
                }
            }
        }
    }

    /// The agent's summary of a file, atop its lines.
    pub fn file_summary_row(&mut self, r: &Row, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some(text) = self.file_summary(&self.files[r.file].file.path) else {
            return div().h(px(0.)).into_any_element();
        };
        crate::diffview::card(&p)
            .px(px(14.))
            .py(px(8.))
            .gap(px(8.))
            .items_start()
            .bg(crate::theme::over(p.code_bg, alpha(p.accent, 0.04)))
            .border_b_1()
            .child(div().mt(px(2.)).child(icon("sparkle", 13., p.accent_foreground)))
            .child(
                div()
                    .id(SharedString::from(format!("fsum-{}", r.file)))
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .child(crate::markdown::markdown(format!("fsum-md-{}", r.file), &text, alpha(p.text, 0.8), cx))
                    .tooltip(tooltip("The AI review's summary of this file's change")),
            )
            .into_any_element()
    }

    pub fn note_row(&mut self, r: &Row, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let path = self.files[r.file].file.path.clone();
        let Some(n) = self.ext.notes.get(&path).and_then(|v| v.get(r.item)).cloned() else {
            return div().into_any_element();
        };
        let tone = if n.critical { p.danger } else { p.accent };
        let accent = if n.critical { p.danger_foreground } else { p.accent_foreground };
        let selected = self.ext.note_sel.as_ref() == Some(&n);
        let border = if selected { accent } else { alpha(accent, 0.22) };
        let kind = if n.critical { "Needs care" } else { "Worth a look" };
        let label = if n.file {
            kind.to_string()
        } else if n.side == Side::Old {
            format!("{kind} · old line {}", n.line)
        } else {
            format!("{kind} · line {}", n.line)
        };
        crate::diffview::card(&p)
            .px(px(16.))
            .py(px(6.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .items_start()
                    .bg(alpha(tone, 0.07))
                    .border_1()
                    .border_color(border)
                    .child(div().mt(px(1.)).child(icon(if n.critical { "alert" } else { "sparkle" }, 14., accent)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(accent).child(label))
                            .child(div().text_size(px(13.)).child(crate::markdown::markdown(
                                format!("note-{}-{}", r.file, r.item),
                                &n.text,
                                p.text,
                                cx,
                            ))),
                    ),
            )
            .into_any_element()
    }

    fn ordered_notes(&self) -> Vec<AiNote> {
        let mut out = Vec::new();
        for f in &self.files {
            if let Some(v) = self.ext.notes.get(&f.file.path) {
                let mut v = v.clone();
                v.sort_by(|a, b| b.file.cmp(&a.file).then(a.line.cmp(&b.line)));
                out.extend(v);
            }
        }
        out
    }

    pub fn next_note(&mut self, dir: i32, cx: &mut Context<Self>) {
        let notes = self.ordered_notes();
        if notes.is_empty() {
            return;
        }
        let n = notes.len() as i32;
        let at = match self.ext.note_sel.as_ref().and_then(|s| notes.iter().position(|x| x == s)) {
            Some(a) => ((a as i32 + dir).rem_euclid(n)) as usize,
            None => {
                let file: HashMap<&str, usize> =
                    self.files.iter().enumerate().map(|(i, f)| (f.file.path.as_str(), i)).collect();
                if dir > 0 {
                    notes.iter().position(|x| file.get(x.path.as_str()).copied().unwrap_or(0) >= self.current).unwrap_or(0)
                } else {
                    notes
                        .iter()
                        .rposition(|x| file.get(x.path.as_str()).copied().unwrap_or(0) < self.current)
                        .unwrap_or(notes.len() - 1)
                }
            }
        };
        self.reveal_note(notes[at].clone(), cx);
    }

    pub fn reveal_note(&mut self, n: AiNote, cx: &mut Context<Self>) {
        let Some(fi) = self.files.iter().position(|f| f.file.path == n.path) else { return };
        if !self.file_visible(fi, cx) {
            self.filter.update(cx, |i, cx| i.set_text("", cx));
            if !self.file_visible(fi, cx) {
                self.finding = false;
            }
        }
        if self.files[fi].collapsed && !self.force_open(fi, cx) {
            self.files[fi].collapsed = false;
        }
        self.build_rows(cx);
        let idx = self.ext.notes.get(&n.path).and_then(|v| v.iter().position(|x| *x == n));
        let at = idx
            .and_then(|idx| self.rows.iter().position(|r| r.kind == RowKind::AiNote && r.file == fi && r.item == idx))
            // The notes hidden: the line, else the file.
            .or_else(|| {
                (!n.file).then(|| {
                    self.rows.iter().position(|r| {
                        r.kind == RowKind::Line && r.file == fi && {
                            let f = &self.files[fi];
                            let side_num = |i: i32| f.line_at(r.hunk, i).map(|l| if n.side == Side::Old { l.old } else { l.new });
                            if r.hunk < 0 {
                                (if n.side == Side::Old { r.old } else { r.new }) == n.line
                            } else {
                                side_num(r.a) == Some(n.line) || side_num(r.b) == Some(n.line)
                            }
                        }
                    })
                })
                .flatten()
            })
            .or_else(|| self.rows.iter().position(|r| r.kind == RowKind::Header && r.file == fi));
        if let Some(r) = at {
            self.list.scroll_to(gpui::ListOffset { item_ix: r.saturating_sub(3), offset_in_item: px(0.) });
        }
        self.ext.note_sel = Some(n);
        self.sel_file = -1;
        self.sel_hunk = -1;
        self.current = fi;
        self.scroll_selection_dirty = false;
        self.select_tree_file(fi, cx);
        cx.notify();
    }

    pub fn clear_note_selection(&mut self) {
        self.ext.note_sel = None;
    }
}

/// The changes of the files as a patch, for the agent.
pub fn patch_text(f: &File) -> String {
    if f.too_large || f.binary {
        return String::new();
    }
    let mut b = String::new();
    for h in &f.hunks {
        b.push_str(&format!("@@ -{},{} +{},{} @@", h.old_start, h.old_lines, h.new_start, h.new_lines));
        if !h.section.is_empty() {
            b.push(' ');
            b.push_str(&h.section);
        }
        b.push('\n');
        for l in &h.lines {
            b.push(match l.kind {
                LineKind::Add => '+',
                LineKind::Del => '-',
                _ => ' ',
            });
            b.push_str(l.text.trim_end_matches(['\r', '\n']));
            b.push('\n');
        }
    }
    b
}

pub fn _unused(_: &App) {}
