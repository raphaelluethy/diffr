//! The window's commands and their keys. "secondary" is ⌘ on macOS and
//! Ctrl elsewhere.

use gpui::{App, Context, KeyBinding, Stateful, Div, actions, prelude::*};

use crate::window::Diffr;

actions!(
    diffr,
    [
        ToggleSidebar,
        ShowFiles,
        ShowHistory,
        ShowPulls,
        FindInDiffs,
        FilterFiles,
        CommandBar,
        NextHunk,
        PrevHunk,
        NextNote,
        PrevNote,
        CommentHunk,
        ToggleWrap,
        OpenInEditor,
        Refresh,
        FontBigger,
        FontSmaller,
        FontReset,
        ShowShortcuts,
        ReviewWithAi,
        ToggleLayout,
        CopyComments,
        CollapseAll,
        ExpandAll,
        Dismiss,
        ToggleCommit,
        ListUp,
        ListDown,
        ListLeft,
        ListRight,
        OpenCommitDialog,
        OpenBranchDialog,
        OpenPullDialog,
        Quit,
        ThemeSystem,
        ThemeLight,
        ThemeDark,
        GroupNone,
        GroupKind,
        GroupAi,
        DiffSplit,
        DiffUnified,
        ToggleWhitespace,
        ToggleCopyOnClose,
        AgentAuto,
        AgentClaude,
        AgentCodex,
        AgentOpencode,
        NextFile,
        PrevFile,
        NextGroup,
        PrevGroup,
        GoTop,
        GoBottom,
        HalfPageDown,
        HalfPageUp,
        PageDown,
        PageUp,
        LineDown,
        LineUp,
        FoldFile,
        ViewedNext,
        NextUnviewed,
        OpenSettings,
        ToggleAiNotes,
        FindNext,
        FindPrev,
        RevealFile,
        CloseWindow,
        ToggleFullScreen,
        ToggleAppMenu,
        SubmitReview,
        StopAi,
        ClearRecent,
        ShowWorkTree,
        SidebarLeft,
        SidebarRight,
        FileInfoNotes,
        FileInfoSummaries,
        FileInfoBoth,
        ToggleStage,
        Push,
        WriteCommitMessage,
        CommitStaged,
    ]
);

/// Opens a repository of the Open Recent menu.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = diffr, no_json)]
pub struct OpenRecentPath {
    pub path: String,
}

pub fn bind_keys(cx: &mut App) {
    let review = Some("Diffr && !TextInput");
    let any = Some("Diffr");
    cx.bind_keys([
        KeyBinding::new("secondary-shift-b", ToggleSidebar, any),
        KeyBinding::new("secondary-1", ShowFiles, any),
        KeyBinding::new("secondary-2", ShowHistory, any),
        KeyBinding::new("secondary-3", ShowPulls, any),
        KeyBinding::new("secondary-f", FindInDiffs, any),
        KeyBinding::new("secondary-p", FilterFiles, any),
        KeyBinding::new("secondary-k", CommandBar, any),
        KeyBinding::new("secondary-shift-p", CommandBar, any),
        KeyBinding::new("secondary-shift-i", ReviewWithAi, any),
        KeyBinding::new("secondary-shift-o", OpenInEditor, any),
        KeyBinding::new("secondary-r", Refresh, any),
        KeyBinding::new("secondary-=", FontBigger, any),
        KeyBinding::new("secondary-+", FontBigger, any),
        KeyBinding::new("secondary--", FontSmaller, any),
        KeyBinding::new("secondary-0", FontReset, any),
        KeyBinding::new("alt-z", ToggleWrap, any),
        // Every window: settings, about and diagrams too.
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-shift-c", CopyComments, any),
        KeyBinding::new("secondary-shift-m", ToggleCommit, any),
        KeyBinding::new("secondary-shift-enter", SubmitReview, any),
        KeyBinding::new("secondary-alt-n", ToggleAiNotes, any),
        KeyBinding::new("secondary-g", FindNext, any),
        KeyBinding::new("secondary-shift-g", FindPrev, any),
        KeyBinding::new("f3", FindNext, any),
        KeyBinding::new("shift-f3", FindPrev, any),
        KeyBinding::new("secondary-shift-r", RevealFile, any),
        KeyBinding::new("secondary-w", CloseWindow, None),
        KeyBinding::new("secondary-shift-/", ShowShortcuts, any),
        KeyBinding::new("secondary-shift-?", ShowShortcuts, any),
        KeyBinding::new("f11", ToggleFullScreen, any),
        KeyBinding::new("f10", ToggleAppMenu, any),
        KeyBinding::new("j", NextHunk, review),
        KeyBinding::new("ctrl-down", NextHunk, review),
        KeyBinding::new("k", PrevHunk, review),
        KeyBinding::new("ctrl-up", PrevHunk, review),
        KeyBinding::new("n", NextNote, review),
        KeyBinding::new("shift-n", PrevNote, review),
        KeyBinding::new("enter", CommentHunk, review),
        KeyBinding::new("shift-?", ShowShortcuts, review),
        KeyBinding::new("?", ShowShortcuts, review),
        KeyBinding::new("escape", Dismiss, review),
        // Vim's moves: files, groups, the ends, pages and lines.
        KeyBinding::new("]", NextFile, review),
        KeyBinding::new("[", PrevFile, review),
        KeyBinding::new("shift-j", NextFile, review),
        KeyBinding::new("shift-k", PrevFile, review),
        KeyBinding::new("}", NextGroup, review),
        KeyBinding::new("{", PrevGroup, review),
        KeyBinding::new("shift-}", NextGroup, review),
        KeyBinding::new("shift-{", PrevGroup, review),
        KeyBinding::new("shift-]", NextGroup, review),
        KeyBinding::new("shift-[", PrevGroup, review),
        KeyBinding::new("g g", GoTop, review),
        KeyBinding::new("shift-g", GoBottom, review),
        KeyBinding::new("home", GoTop, review),
        KeyBinding::new("end", GoBottom, review),
        KeyBinding::new("ctrl-d", HalfPageDown, review),
        KeyBinding::new("ctrl-u", HalfPageUp, review),
        KeyBinding::new("pagedown", PageDown, review),
        KeyBinding::new("pageup", PageUp, review),
        KeyBinding::new("space", PageDown, review),
        KeyBinding::new("shift-space", PageUp, review),
        KeyBinding::new("ctrl-e", LineDown, review),
        KeyBinding::new("ctrl-y", LineUp, review),
        KeyBinding::new("o", FoldFile, review),
        KeyBinding::new("x", FoldFile, review),
        KeyBinding::new("v", ViewedNext, review),
        KeyBinding::new("u", NextUnviewed, review),
        KeyBinding::new("s", ToggleStage, review),
        KeyBinding::new("secondary-enter", CommitStaged, review),
        KeyBinding::new("/", FindInDiffs, review),
        KeyBinding::new("secondary-,", OpenSettings, any),
        KeyBinding::new("up", ListUp, review),
        KeyBinding::new("down", ListDown, review),
        KeyBinding::new("left", ListLeft, review),
        KeyBinding::new("right", ListRight, review),
    ]);
    // ⌃F and ⌃B page as in Vim where they do not find (macOS: ⌘F finds).
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("ctrl-f", PageDown, review),
            KeyBinding::new("ctrl-b", PageUp, review),
            KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, any),
        ]);
    }
}

/// Registers the handlers of the commands on the window's root.
pub fn register(root: Stateful<Div>, cx: &mut Context<Diffr>) -> Stateful<Div> {
    root.on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
        .on_action(cx.listener(|this, _: &ShowFiles, _, cx| this.set_tab(0, cx)))
        .on_action(cx.listener(|this, _: &ShowHistory, _, cx| this.set_tab(1, cx)))
        .on_action(cx.listener(|this, _: &ShowPulls, window, cx| this.show_pulls(window, cx)))
        .on_action(cx.listener(|this, _: &FindInDiffs, window, cx| this.open_find(window, cx)))
        .on_action(cx.listener(|this, _: &FilterFiles, window, cx| {
            this.tab = 0;
            this.sidebar_shown = true;
            this.filter.update(cx, |i, cx| i.focus(window, cx));
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &CommandBar, window, cx| this.toggle_palette(window, cx)))
        .on_action(cx.listener(|this, _: &NextHunk, _, cx| this.next_hunk(1, cx)))
        .on_action(cx.listener(|this, _: &PrevHunk, _, cx| this.next_hunk(-1, cx)))
        .on_action(cx.listener(|this, _: &NextNote, _, cx| this.next_note(1, cx)))
        .on_action(cx.listener(|this, _: &PrevNote, _, cx| this.next_note(-1, cx)))
        .on_action(cx.listener(|this, _: &CommentHunk, window, cx| {
            // In the menu bar, Enter runs the command chosen.
            if this.app_menu.is_some() {
                this.run_app_menu_item(window, cx);
                return;
            }
            if this.tree_focus.is_focused(window) || this.history_focus.is_focused(window) {
                return;
            }
            this.comment_on_selection(window, cx)
        }))
        .on_action(cx.listener(|this, _: &ToggleWrap, _, cx| {
            let wrap = !this.settings.word_wrap;
            this.settings.word_wrap = wrap;
            crate::config().update(|s| s.word_wrap = wrap);
            this.rows_dirty = true;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &OpenInEditor, _, cx| this.open_current(cx)))
        .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
        .on_action(cx.listener(|this, _: &FontBigger, _, cx| this.change_font_size(1, cx)))
        .on_action(cx.listener(|this, _: &FontSmaller, _, cx| this.change_font_size(-1, cx)))
        .on_action(cx.listener(|this, _: &FontReset, _, cx| this.change_font_size(0, cx)))
        .on_action(cx.listener(|this, _: &ShowShortcuts, _, cx| {
            this.help = true;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ReviewWithAi, _, cx| this.review_with_ai(cx)))
        .on_action(cx.listener(|this, _: &ToggleLayout, _, cx| {
            let s = !this.split();
            this.set_split(s, cx)
        }))
        .on_action(cx.listener(|this, _: &CopyComments, _, cx| this.copy_comments(cx)))
        .on_action(cx.listener(|this, _: &CollapseAll, _, cx| this.set_all_collapsed(true, cx)))
        .on_action(cx.listener(|this, _: &ExpandAll, _, cx| this.set_all_collapsed(false, cx)))
        .on_action(cx.listener(|this, _: &ToggleCommit, window, cx| this.toggle_commit(window, cx)))
        .on_action(cx.listener(|this, _: &ToggleStage, _, cx| this.toggle_stage_current(cx)))
        .on_action(cx.listener(|this, _: &Push, _, cx| this.push(cx)))
        .on_action(cx.listener(|this, _: &CommitStaged, window, cx| {
            if this.commit_open {
                this.do_commit(window, cx)
            }
        }))
        .on_action(cx.listener(|this, _: &WriteCommitMessage, window, cx| {
            if !this.commit_open {
                this.toggle_commit(window, cx);
            }
            this.generate_message(cx)
        }))
        .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
        .on_action(cx.listener(|this, _: &ListUp, window, cx| this.list_key(0, -1, window, cx)))
        .on_action(cx.listener(|this, _: &ListDown, window, cx| this.list_key(0, 1, window, cx)))
        .on_action(cx.listener(|this, _: &ListLeft, window, cx| this.list_key(-1, 0, window, cx)))
        .on_action(cx.listener(|this, _: &ListRight, window, cx| this.list_key(1, 0, window, cx)))
        .on_action(cx.listener(|this, _: &OpenCommitDialog, window, cx| {
            this.open_dialog(crate::menus::DialogKind::Commit, window, cx)
        }))
        .on_action(cx.listener(|this, _: &OpenBranchDialog, window, cx| {
            this.open_dialog(crate::menus::DialogKind::Branch, window, cx)
        }))
        .on_action(cx.listener(|this, _: &OpenPullDialog, window, cx| {
            this.open_dialog(crate::menus::DialogKind::Pull, window, cx)
        }))
        .on_action(cx.listener(|_, _: &ThemeSystem, _, _| set_setting(|s| s.theme = "system".into())))
        .on_action(cx.listener(|_, _: &ThemeLight, _, _| set_setting(|s| s.theme = "light".into())))
        .on_action(cx.listener(|_, _: &ThemeDark, _, _| set_setting(|s| s.theme = "dark".into())))
        .on_action(cx.listener(|this, _: &GroupNone, _, cx| this.choose_grouping(crate::features::GroupMode::None, cx)))
        .on_action(cx.listener(|this, _: &GroupKind, _, cx| this.choose_grouping(crate::features::GroupMode::Kind, cx)))
        .on_action(cx.listener(|this, _: &GroupAi, _, cx| this.choose_grouping(crate::features::GroupMode::Ai, cx)))
        .on_action(cx.listener(|this, _: &DiffSplit, _, cx| this.set_split(true, cx)))
        .on_action(cx.listener(|this, _: &DiffUnified, _, cx| this.set_split(false, cx)))
        .on_action(cx.listener(|this, _: &ToggleWhitespace, _, cx| {
            let ws = !this.settings.show_whitespace;
            let s = crate::config().update(|s| s.show_whitespace = ws);
            this.apply_settings(s, cx);
        }))
        .on_action(cx.listener(|this, _: &ToggleCopyOnClose, _, _| {
            let v = !this.settings.copy_comments_on_close;
            this.settings.copy_comments_on_close = v;
            set_setting(|s| s.copy_comments_on_close = v);
        }))
        .on_action(cx.listener(|this, _: &AgentAuto, _, cx| this.choose_agent(String::new(), String::new(), false, cx)))
        .on_action(cx.listener(|this, _: &AgentClaude, _, cx| this.choose_agent("claude".into(), String::new(), false, cx)))
        .on_action(cx.listener(|this, _: &AgentCodex, _, cx| this.choose_agent("codex".into(), String::new(), false, cx)))
        .on_action(cx.listener(|this, _: &AgentOpencode, _, cx| this.choose_agent("opencode".into(), String::new(), false, cx)))
        .on_action(cx.listener(|this, _: &NextUnviewed, _, cx| this.next_unviewed(cx)))
        .on_action(cx.listener(|this, _: &NextFile, _, cx| this.next_file(1, cx)))
        .on_action(cx.listener(|this, _: &PrevFile, _, cx| this.next_file(-1, cx)))
        .on_action(cx.listener(|this, _: &NextGroup, _, cx| this.next_group(1, cx)))
        .on_action(cx.listener(|this, _: &PrevGroup, _, cx| this.next_group(-1, cx)))
        .on_action(cx.listener(|this, _: &GoTop, _, cx| {
            this.list.scroll_to(gpui::ListOffset { item_ix: 0, offset_in_item: gpui::px(0.) });
            let heads = this.header_rows(cx);
            if let Some(&(_, f)) = heads.first() {
                this.current = f;
                this.scroll_selection_dirty = false;
                this.select_tree_file(f, cx);
            }
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &GoBottom, _, cx| {
            this.list.scroll_to_end();
            let heads = this.header_rows(cx);
            if let Some(&(_, f)) = heads.last() {
                this.current = f;
                this.scroll_selection_dirty = false;
                this.select_tree_file(f, cx);
            }
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &HalfPageDown, _, cx| this.scroll_pages(0.5, cx)))
        .on_action(cx.listener(|this, _: &HalfPageUp, _, cx| this.scroll_pages(-0.5, cx)))
        .on_action(cx.listener(|this, _: &PageDown, _, cx| this.scroll_pages(0.9, cx)))
        .on_action(cx.listener(|this, _: &PageUp, _, cx| this.scroll_pages(-0.9, cx)))
        .on_action(cx.listener(|this, _: &LineDown, _, cx| {
            let lh = this.line_height();
            this.list.scroll_by(gpui::px(lh));
            this.scroll_selection_dirty = true;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &LineUp, _, cx| {
            let lh = this.line_height();
            this.list.scroll_by(gpui::px(-lh));
            this.scroll_selection_dirty = true;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &FoldFile, _, cx| {
            let i = this.current;
            if let Some(f) = this.files.get_mut(i) {
                f.collapsed = !f.collapsed;
                this.rows_dirty = true;
                this.reveal_file(i, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ViewedNext, _, cx| {
            // Marks the file viewed and goes on; unmarks it and stays.
            let i = this.current;
            if i < this.files.len() {
                if this.is_viewed(&this.files[i]) {
                    this.set_viewed(i, false, cx);
                } else {
                    this.set_viewed(i, true, cx);
                    this.next_file(1, cx);
                }
            }
        }))
        .on_action(cx.listener(|this, _: &OpenSettings, _, cx| crate::settings::open_settings(this.root.clone(), cx)))
        .on_action(cx.listener(|this, _: &ToggleAiNotes, _, cx| this.toggle_ai_notes(cx)))
        .on_action(cx.listener(|this, _: &ToggleAppMenu, _, cx| {
            if !cfg!(target_os = "macos") {
                this.app_menu = if this.app_menu.is_some() { None } else { Some(0) };
                this.app_menu_item = None;
                // Under the button, wherever the pointer last clicked.
                if let Some(at) = this.app_menu_anchor.get() {
                    this.app_menu_at = at;
                }
                cx.notify();
            }
        }))
        .on_action(cx.listener(|this, _: &FindNext, window, cx| {
            if this.finding {
                this.step_match(1, cx)
            } else {
                this.open_find(window, cx)
            }
        }))
        .on_action(cx.listener(|this, _: &FindPrev, _, cx| this.step_match(-1, cx)))
        .on_action(cx.listener(|this, _: &RevealFile, _, cx| {
            if let Some(path) = this.files.get(this.current).map(|f| f.file.path.clone()) {
                this.reveal_in_file_manager(&path, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &CloseWindow, window, cx| this.request_close(window, cx)))
        .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
        .on_action(cx.listener(|this, _: &SubmitReview, _, cx| {
            if this.source.is_pull() {
                this.open_submit(cx)
            }
        }))
        .on_action(cx.listener(|this, _: &StopAi, _, cx| this.stop_analysis(cx)))
        .on_action(cx.listener(|this, _: &ClearRecent, _, cx| {
            this.store.clear_recent();
            crate::set_menus(cx);
        }))
        .on_action(cx.listener(|this, _: &ShowWorkTree, window, cx| {
            let s = this.launch_work_tree();
            this.set_source(s, window, cx)
        }))
        .on_action(cx.listener(|_, _: &SidebarLeft, _, _| set_setting(|s| s.sidebar_position = "left".into())))
        .on_action(cx.listener(|_, _: &SidebarRight, _, _| set_setting(|s| s.sidebar_position = "right".into())))
        .on_action(cx.listener(|this, _: &FileInfoNotes, _, cx| this.set_file_info("notes", cx)))
        .on_action(cx.listener(|this, _: &FileInfoSummaries, _, cx| this.set_file_info("summaries", cx)))
        .on_action(cx.listener(|this, _: &FileInfoBoth, _, cx| this.set_file_info("both", cx)))
        .on_action(cx.listener(|this, a: &OpenRecentPath, _, cx| {
            let _ = this;
            crate::open_repo(crate::Request { dir: std::path::PathBuf::from(&a.path), src: crate::state::Source::WorkingTree }, cx);
        }))
        .on_action(cx.listener(|this, _: &crate::OpenRecent, window, cx| {
            this.open_dialog(crate::menus::DialogKind::Recent, window, cx)
        }))
        .on_action(cx.listener(|_, _: &Quit, _, cx| cx.defer(crate::request_quit)))
}

/// Changes a setting in the file; the windows take it from there.
fn set_setting(f: impl FnOnce(&mut diffr_git::Settings)) {
    crate::config().update(f);
}

impl Diffr {
    /// Shows or hides the review's notes, keeping the summaries as they are:
    /// both ↔ summaries, notes ↔ none.
    pub fn toggle_ai_notes(&mut self, cx: &mut Context<Self>) {
        let next = match self.settings.ai_file_info.as_str() {
            "both" => "summaries",
            "summaries" => "both",
            "notes" => "none",
            _ => "notes",
        };
        self.set_file_info(next, cx);
    }

    pub fn set_file_info(&mut self, v: &str, cx: &mut Context<Self>) {
        let s = crate::config().update(|s| s.ai_file_info = v.to_string());
        self.settings.ai_file_info = s.ai_file_info;
        self.rows_dirty = true;
        crate::set_menus(cx);
        cx.notify();
    }

    /// Shows a file of the repository in the file manager.
    pub fn reveal_in_file_manager(&mut self, path: &str, cx: &mut Context<Self>) {
        let abs = std::path::Path::new(&self.root).join(path);
        let target = if abs.exists() { abs } else { std::path::PathBuf::from(&self.root) };
        cx.reveal_path(&target);
    }

    /// The rows of the files' headers, in the surface's order.
    fn header_rows(&mut self, cx: &mut Context<Self>) -> Vec<(usize, usize)> {
        if self.rows_dirty {
            self.build_rows(cx);
        }
        self.rows.iter().enumerate().filter(|(_, r)| r.kind == crate::state::RowKind::Header).map(|(i, r)| (i, r.file)).collect()
    }

    pub fn next_unviewed(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = crate::state::next_matching_file(self.files.len(), self.current,
            |i| self.file_visible(i, cx) && !self.is_viewed(&self.files[i])) {
            self.select_tree_file(i, cx);
            self.reveal_file(i, cx);
        }
    }

    /// Goes to the next (1) or the previous (-1) file.
    pub fn next_file(&mut self, dir: i32, cx: &mut Context<Self>) {
        let heads = self.header_rows(cx);
        if heads.is_empty() {
            return;
        }
        let at = heads.iter().position(|h| h.1 == self.current);
        // Back goes to the start of the current file first, as a pager does.
        let top = self.list.logical_scroll_top();
        let at_start = at.is_some_and(|a| heads[a].0 == top.item_ix && top.offset_in_item <= gpui::px(1.));
        let next = match at {
            Some(a) if dir < 0 && !at_start => a,
            Some(a) => (a as i32 + dir).clamp(0, heads.len() as i32 - 1) as usize,
            None => 0,
        };
        let f = heads[next].1;
        self.sel_file = -1;
        self.sel_hunk = -1;
        self.reveal_file(f, cx);
        self.select_tree_file(f, cx);
    }

    /// Goes to the next (1) or the previous (-1) group of files.
    pub fn next_group(&mut self, dir: i32, cx: &mut Context<Self>) {
        if self.rows_dirty {
            self.build_rows(cx);
        }
        let groups: Vec<(usize, usize)> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == crate::state::RowKind::Group)
            .map(|(i, r)| (i, r.file))
            .collect();
        if groups.is_empty() {
            return self.next_file(dir, cx);
        }
        let cur_group = self.files.get(self.current).map(|f| f.group).unwrap_or(-1);
        let at = groups.iter().position(|g| self.files[g.1].group == cur_group);
        let top = self.list.logical_scroll_top().item_ix;
        let next = match at {
            Some(a) if dir < 0 && groups[a].0 != top => a,
            Some(a) => (a as i32 + dir).clamp(0, groups.len() as i32 - 1) as usize,
            None => 0,
        };
        let (row, f) = groups[next];
        self.list.scroll_to(gpui::ListOffset { item_ix: row, offset_in_item: gpui::px(0.) });
        self.current = f;
        self.scroll_selection_dirty = false;
        self.select_tree_file(f, cx);
        cx.notify();
    }

    /// Scrolls by a part of the surface's height.
    pub fn scroll_pages(&mut self, pages: f32, cx: &mut Context<Self>) {
        let h = f32::from(self.list.viewport_bounds().size.height).max(200.);
        self.list.scroll_by(gpui::px(h * pages));
        self.scroll_selection_dirty = true;
        cx.notify();
    }

    pub fn change_font_size(&mut self, delta: i32, cx: &mut Context<Self>) {
        let size = if delta == 0 { 13 } else { (self.settings.code_font_size + delta).clamp(10, 32) };
        self.settings.code_font_size = size;
        crate::config().update(|s| s.code_font_size = size);
        self.rows_dirty = true;
        self.char_w = 0.;
        cx.notify();
    }

    pub fn open_current(&mut self, cx: &mut Context<Self>) {
        if let Some(f) = self.files.get(self.current) {
            let (path, line) = (f.file.path.clone(), f.first_line());
            self.open_in_editor(&path, line, cx);
        }
    }

    /// Escape: closes what is open, else drops the hunk chosen.
    pub fn dismiss(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.app_menu.is_some() {
            self.app_menu = None;
        } else if self.context_menu.is_some() {
            self.context_menu = None;
        } else if self.discarding.is_some() {
            self.discarding = None;
        } else if self.close_prompt {
            self.cancel_close(cx);
        } else if self.help {
            self.help = false;
        } else if self.palette_open {
            self.palette_open = false;
        } else if self.dialog.is_some() {
            self.dialog = None;
        } else if self.close_pull_dialogs() {
        } else if self.finding {
            self.close_find(window, cx);
        } else if self.commit_open {
            self.commit_open = false;
        } else {
            self.sel_file = -1;
            self.sel_hunk = -1;
            self.clear_note_selection();
        }
        cx.notify();
    }

    /// The arrows: they move in the tree or the history when it has the
    /// focus, and scroll the review otherwise.
    pub fn list_key(&mut self, dx: i32, dy: i32, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.palette_open {
            return;
        }
        // In the menu bar: ← and → go from menu to menu, ↑ and ↓ from
        // command to command.
        if let Some(sel) = self.app_menu {
            let n = cx.get_menus().map(|m| m.len()).unwrap_or(1).max(1) as i32;
            if dx != 0 {
                self.app_menu = Some(((sel as i32 + dx).rem_euclid(n)) as usize);
                self.app_menu_item = None;
            } else if dy != 0 {
                let count = crate::menus::app_menu_commands(cx, sel).len() as i32;
                if count > 0 {
                    let at = match self.app_menu_item {
                        Some((m, i)) if m == sel => (i as i32 + dy).rem_euclid(count),
                        _ if dy > 0 => 0,
                        _ => count - 1,
                    };
                    self.app_menu_item = Some((sel, at as usize));
                }
            }
            cx.notify();
            return;
        }
        if self.tree_focus.is_focused(window) {
            if dy != 0 {
                self.tree_move(dy, cx);
                return;
            }
            let rows = self.visible_tree(cx);
            let Some(at) = rows.iter().position(|r| r.key == self.tree_sel) else { return };
            let key = rows[at].key.clone();
            let is_dir = self.tree_items.get(&key).is_some_and(|n| n.dir);
            if dx > 0 && is_dir {
                if !self.closed_dirs.remove(&key) {
                    self.tree_move(1, cx);
                }
            } else if dx < 0 {
                if is_dir && !self.closed_dirs.contains(&key) {
                    self.closed_dirs.insert(key);
                } else if let Some(j) = (0..at).rev().find(|&j| rows[j].depth < rows[at].depth) {
                    let k = rows[j].key.clone();
                    self.choose_tree(&k, cx);
                }
            }
            cx.notify();
            return;
        }
        if self.history_focus.is_focused(window) {
            if dy != 0 {
                self.history_move(dy, window, cx);
            }
            return;
        }
        if self.pulls_focused(window) {
            if dy != 0 {
                self.pulls_move(dy, window, cx);
            }
            return;
        }
        if dy != 0 {
            self.list.scroll_by(gpui::px(dy as f32 * 60.));
        self.scroll_selection_dirty = true;
            cx.notify();
        }
    }
}
