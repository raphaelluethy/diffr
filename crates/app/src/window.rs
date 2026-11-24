//! The window reviewing a repository: its state, and loading the changes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diffr_git::{Comments, Commit, File, Options, Repo, Settings, Status};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, ListAlignment, ListState, ScrollStrategy,
    UniformListScrollHandle, Window, px,
};

use crate::input::{InputEvent, TextInput};
use crate::state::*;

/// A row of the file tree: a file, or a directory with the chain of
/// directories holding only it compacted into one row.
#[derive(Clone, Debug)]
pub struct TreeNode {
    pub name: String,
    pub dir: bool,
    pub file: usize,
    pub path: String,
    pub group: i32,
    pub children: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TreeRow {
    pub key: String,
    pub depth: usize,
}

/// The review of one source, kept while the window shows another.
#[derive(Default)]
pub struct Session {
    pub comments: Comments,
    pub comment_inputs: HashMap<u64, Entity<TextInput>>,
    pub viewed: HashMap<String, String>,
    /// The files its comments are on, with their lines, for the context
    /// of the comments copied while another source shows.
    pub files: Vec<SavedFile>,
}

/// A file a source's comments are on, kept when the window shows another.
pub struct SavedFile {
    pub file: diffr_git::File,
    pub old_lines: Option<Vec<String>>,
    pub new_lines: Option<Vec<String>>,
}

impl diffr_git::CommentSource for SavedFile {
    fn diff_file(&self) -> &diffr_git::File {
        &self.file
    }
    fn old_lines(&self) -> Option<&[String]> {
        self.old_lines.as_deref()
    }
    fn new_lines(&self) -> Option<&[String]> {
        self.new_lines.as_deref()
    }
}

/// Review comments of several sources as one Markdown text: the prefix
/// once, then each source's comments, under its name when there are
/// several. `sections` are (name, comments without the prefix).
pub fn join_comment_sections(sections: &[(String, String)], prefix: &str) -> String {
    let sections: Vec<&(String, String)> = sections.iter().filter(|s| !s.1.trim().is_empty()).collect();
    let body = match sections.as_slice() {
        [] => return String::new(),
        [(_, md)] => md.trim_end().to_string(),
        many => many
            .iter()
            .map(|(name, md)| format!("## {name}\n\n{}", md.trim_end()))
            .collect::<Vec<_>>()
            .join("\n\n"),
    };
    if prefix.is_empty() { body } else { format!("{prefix}\n\n{body}") }
}

/// A match of the find bar: a line, or a file whose path holds the query
/// (line -1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    pub file: usize,
    pub hunk: i32,
    pub line: i32,
}

pub const SIDEBAR_DEFAULT: f32 = 292.;
pub const SIDEBAR_MIN: f32 = 220.;
pub const SIDEBAR_MAX: f32 = 640.;
pub const SIDEBAR_COLLAPSE: f32 = 80.;
pub const HISTORY_PAGE: usize = 200;
const MAX_CONTENT: usize = 2 << 20;
const MAX_IMAGE: usize = 32 << 20;

pub struct Diffr {
    pub repo: Repo,
    pub root: String,
    pub launch: Source,
    pub source: Source,
    pub settings: Settings,
    pub store: Arc<diffr_git::Store>,
    pub focus: FocusHandle,

    pub branch: String,
    pub files: Vec<FileState>,
    pub commit: Option<Commit>,
    pub base: String,
    pub loading: bool,
    pub loaded_once: bool,
    pub load_err: Option<String>,
    pub gen_: u64,
    pub load_start: Instant,
    pub load_status: String,
    pub viewed: HashMap<String, String>,

    // The sidebar.
    pub sidebar_shown: bool,
    pub sidebar_width: f32,
    pub drag_width: f32,
    pub tab: usize,
    pub filter: Entity<TextInput>,
    pub history_filter: Entity<TextInput>,
    pub pulls_filter: Entity<TextInput>,
    pub tree_scroll: UniformListScrollHandle,
    pub tree_sel: String,
    pub tree_rows: Vec<TreeRow>,
    pub closed_dirs: HashSet<String>,
    pub tree_items: HashMap<String, TreeNode>,
    pub tree_roots: Vec<String>,
    pub tree_focus: FocusHandle,
    pub history: Vec<Commit>,
    pub history_limit: usize,
    pub history_more: bool,
    pub history_loading: bool,
    pub history_list: ListState,
    pub history_focus: FocusHandle,
    pub reloaded: HashSet<String>,

    // The diff surface.
    pub rows: Vec<Row>,
    pub rows_dirty: bool,
    pub list: ListState,
    pub list_focus: FocusHandle,
    pub current: usize,
    pub hscroll: HashMap<String, f32>,
    pub scroll_selection_dirty: bool,
    pub last_scroll_top: gpui::ListOffset,
    pub char_w: f32,
    pub char_key: (String, f32),
    pub sel_file: i32,
    pub sel_hunk: i32,
    pub focused_once: bool,

    // Find in diffs.
    pub finding: bool,
    pub find_input: Entity<TextInput>,
    pub matches: Vec<Match>,
    pub match_ix: usize,
    pub matches_for: String,
    pub file_matches: HashSet<usize>,

    pub sessions: HashMap<Source, Session>,

    // Review comments.
    pub comments: Comments,
    pub comment_inputs: HashMap<u64, Entity<TextInput>>,
    pub user: String,
    pub copied_at: Option<Instant>,

    // Committing.
    pub commit_open: bool,
    pub subject: Entity<TextInput>,
    pub body: Entity<TextInput>,
    /// The files staged in git's index, by path (the work tree's only).
    pub staged: HashMap<String, diffr_git::Staged>,
    /// Held by whatever changes the index, one at a time.
    pub index_lock: Arc<Mutex<()>>,
    /// Changes of the index under way.
    pub staging: usize,
    /// Has the agent write the message once the staging is done.
    pub generate_after_stage: bool,
    pub upstream: diffr_git::Upstream,
    pub push_busy: bool,
    /// What the last push did, shown in the commit view until the next.
    pub push_done: String,
    pub msg_gen: crate::commit::MessageGen,
    /// The agent splitting the changes into commits, and its plan.
    pub plan_gen: crate::commit::MessageGen,
    pub commit_plan: Option<Vec<crate::commit::CommitGroup>>,
    pub commit_busy: bool,
    pub commit_err: String,
    pub commit_done: String,
    pub committed_files: usize,
    pub commit_output: String,

    // The command bar.
    pub palette_open: bool,
    pub palette_input: Entity<TextInput>,
    pub palette_row: usize,

    pub help: bool,
    pub toast: Option<(String, Instant)>,

    /// Set when the work tree changed since the last load.
    pub changed: bool,
    pub signature: String,

    pub ext: crate::features::Ext,
    pub _subs: Vec<gpui::Subscription>,

    pub comment_focused: Option<u64>,
    pub resizing: Option<(f32, f32)>,
    pub context_menu: Option<(crate::menus::ContextMenu, gpui::Point<gpui::Pixels>)>,
    pub dialog: Option<crate::menus::DialogKind>,
    pub dialog_input: Entity<TextInput>,
    pub dialog_err: String,
    pub dialog_busy: bool,
    pub bounds_gen: u64,
    pub scroll_drag: Option<crate::scrollbar::ScrollDrag>,
    /// The comment asked about discarding.
    pub discarding: Option<u64>,
    pub line_drag: Option<crate::diffview::LineDrag>,
    pub drag_edge: i32,
    /// The diff surface's width, as of its last layout.
    pub list_width: f32,
    pub close_prompt: bool,
    pub close_confirmed: bool,
    /// The close prompt is part of quitting: the next window is asked next.
    pub quitting: bool,
    /// This window's answer while quitting: copy its comments, or not.
    pub quit_choice: Option<bool>,
    pub window_active: bool,
    pub branches: Vec<String>,
    /// The in-window menu bar: the menu open, if any.
    pub app_menu: Option<usize>,
    pub app_menu_at: gpui::Point<gpui::Pixels>,
    /// The item chosen with the keyboard: (menu, index among its commands).
    pub app_menu_item: Option<(usize, usize)>,
    /// Where the menu button was last drawn, for F10 to open the menu
    /// under it.
    pub app_menu_anchor: std::rc::Rc<std::cell::Cell<Option<gpui::Point<gpui::Pixels>>>>,
}

/// What a background load returns.
struct Loaded {
    files: Vec<File>,
    commit: Option<Commit>,
    base: String,
    branch: String,
    sig: String,
    old_rev: String,
    new_rev: String,
    err: Option<String>,
    pull: Option<crate::features::PulledRequest>,
    /// The index and the upstream, of the work tree.
    staged: Option<(HashMap<String, diffr_git::Staged>, diffr_git::Upstream)>,
}

/// A file's contents, read and tokenized.
pub struct Content {
    path: String,
    old_lines: Option<Vec<String>>,
    new_lines: Option<Vec<String>>,
    old_hl: Option<Vec<Vec<diffr_highlight::Seg>>>,
    new_hl: Option<Vec<Vec<diffr_highlight::Seg>>>,
    old_image: Option<Picture>,
    new_image: Option<Picture>,
}

fn picture(data: Vec<u8>, path: &str) -> Option<Picture> {
    let fmt = image::ImageFormat::from_path(path).ok()?;
    let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(&data), fmt)
        .into_dimensions()
        .ok()?;
    let gfmt = match fmt {
        image::ImageFormat::Png => gpui::ImageFormat::Png,
        image::ImageFormat::Jpeg => gpui::ImageFormat::Jpeg,
        image::ImageFormat::Gif => gpui::ImageFormat::Gif,
        image::ImageFormat::WebP => gpui::ImageFormat::Webp,
        image::ImageFormat::Bmp => gpui::ImageFormat::Bmp,
        image::ImageFormat::Ico => gpui::ImageFormat::Ico,
        _ => return None,
    };
    let bytes = data.len();
    Some(Picture { image: Arc::new(gpui::Image::from_bytes(gfmt, data)), width: w, height: h, bytes })
}

/// Reads both sides of a file: lines of text, or a picture.
fn read_content(repo: &Repo, contents: &diffr_git::Contents, f: &File, old_rev: &str, new_rev: &str) -> Content {
    let raw = |rev: &str, path: &str, limit: usize| -> Option<Vec<u8>> {
        let data = if rev.is_empty() {
            repo.read_work_tree(path)
        } else {
            contents.read(rev, path).ok().flatten()
        }?;
        (data.len() <= limit).then_some(data)
    };
    let mut c = Content {
        path: f.path.clone(),
        old_lines: None,
        new_lines: None,
        old_hl: None,
        new_hl: None,
        old_image: None,
        new_image: None,
    };
    let has_old = !old_rev.is_empty() && !matches!(f.status, Status::Added | Status::Untracked);
    let has_new = f.status != Status::Deleted;
    if f.binary && is_image(&f.path) {
        if has_old {
            c.old_image = raw(old_rev, &f.old_path, MAX_IMAGE).and_then(|d| picture(d, &f.old_path));
        }
        if has_new {
            c.new_image = raw(new_rev, &f.path, MAX_IMAGE).and_then(|d| picture(d, &f.path));
        }
        return c;
    }
    if f.binary || f.directory || f.too_large {
        return c;
    }
    let text = |data: Option<Vec<u8>>| -> Option<Vec<String>> {
        let data = data?;
        if diffr_git::diff::is_binary(&data) {
            return None;
        }
        let s = String::from_utf8_lossy(&data);
        Some(diffr_git::diff::split_lines(&s).into_iter().map(str::to_string).collect())
    };
    if has_old {
        c.old_lines = text(raw(old_rev, &f.old_path, MAX_CONTENT));
    }
    if has_new {
        c.new_lines = text(raw(new_rev, &f.path, MAX_CONTENT));
    }
    if let Some(l) = &c.old_lines {
        c.old_hl = diffr_highlight::lines(&f.old_path, l);
    }
    if let Some(l) = &c.new_lines {
        c.new_hl = diffr_highlight::lines(&f.path, l);
    }
    c
}

impl Content {
    fn apply(self, f: &mut FileState) {
        f.old_lines = self.old_lines;
        f.new_lines = self.new_lines;
        f.old_hl = self.old_hl;
        f.new_hl = self.new_hl;
        f.old_image = self.old_image;
        f.new_image = self.new_image;
        f.loaded = true;
        f.metrics = None;
    }
}

pub fn new_input(cx: &mut App, placeholder: &str, multiline: bool) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        let t = TextInput::new(cx, placeholder);
        if multiline { t.multiline() } else { t }
    })
}

impl Diffr {
    pub fn new(repo: Repo, src: Source, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Arc::new(diffr_git::Store::open_default());
        let settings = crate::config().get();
        let (mut width, shown) = store.layout();
        if let Some(w) = std::env::var("DIFFR_SIDEBAR_WIDTH").ok().and_then(|w| w.parse().ok()) {
            width = w;
        }
        let root = repo.root().to_string_lossy().to_string();
        let viewed = viewed_key(&root, &src).map(|k| store.viewed(&k)).unwrap_or_default();
        let filter = new_input(cx, "Filter files", false);
        let history_filter = new_input(cx, "Filter history", false);
        let pulls_filter = new_input(cx, "Filter pull requests", false);
        let find_input = new_input(cx, "Find in diffs", false);
        let palette_input = new_input(cx, "Type a command", false);
        let subject = new_input(cx, "Subject", false);
        let body = new_input(cx, "Describe the change in a paragraph or two…", true);
        let dialog_input = new_input(cx, "", false);
        body.update(cx, |b, _| b.min_lines = 1);

        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(&filter, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => {
                this.matches_for = "\0".into();
                this.build_tree();
                this.rows_dirty = true;
                cx.notify();
            }
            InputEvent::Cancel => {
                this.filter.update(cx, |f, cx| f.set_text("", cx));
                this.build_tree();
                this.rows_dirty = true;
                window.focus(&this.list_focus, cx);
                cx.notify();
            }
            InputEvent::Down | InputEvent::Submit => {
                window.focus(&this.tree_focus, cx);
                this.tree_move(1, cx);
            }
            _ => cx.notify(),
        }));
        for input in [&history_filter, &pulls_filter] {
            subs.push(cx.subscribe_in(input, window, |this, inp, ev: &InputEvent, window, cx| match ev {
                InputEvent::Cancel => {
                    inp.update(cx, |f, cx| f.set_text("", cx));
                    window.focus(&this.list_focus, cx);
                    cx.notify();
                }
                _ => cx.notify(),
            }));
        }
        subs.push(cx.subscribe_in(&find_input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => {
                this.rows_dirty = true;
                this.match_ix = 0;
                this.update_matches(cx);
                this.show_match();
                cx.notify();
            }
            InputEvent::Submit | InputEvent::Down => this.step_match(1, cx),
            InputEvent::Up => this.step_match(-1, cx),
            InputEvent::Cancel => this.close_find(window, cx),
            _ => cx.notify(),
        }));
        subs.push(cx.subscribe_in(&palette_input, window, |this, _, ev: &InputEvent, window, cx| {
            this.palette_event(ev, window, cx)
        }));
        for input in [&subject, &body] {
            subs.push(cx.subscribe_in(input, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Submit => this.do_commit(window, cx),
                InputEvent::Cancel => {
                    this.commit_open = false;
                    window.focus(&this.list_focus, cx);
                    cx.notify();
                }
                _ => cx.notify(),
            }));
        }
        subs.push(cx.subscribe_in(&dialog_input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit => this.dialog_submit(window, cx),
            InputEvent::Cancel => {
                this.dialog = None;
                window.focus(&this.list_focus, cx);
                cx.notify();
            }
            _ => cx.notify(),
        }));
        subs.push(cx.observe_window_bounds(window, |this, window, cx| {
            // Kept a moment after the last change, as the window settles.
            this.bounds_gen += 1;
            let g = this.bounds_gen;
            let b = window.bounds();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                if this.update(cx, |this, _| this.bounds_gen == g).unwrap_or(false)
                    && std::env::var("DIFFR_WINDOW_SIZE").is_err()
                {
                    cx.background_spawn(async move { crate::save_bounds(b) }).await;
                }
            })
            .detach();
        }));
        subs.push(cx.observe_window_activation(window, |this, window, cx| {
            this.window_active = window.is_window_active();
            // Back to the window: look for changes at once.
            if this.window_active && !this.loading && !this.signature.is_empty() {
                let (repo, sig) = (this.repo.clone(), this.signature.clone());
                cx.spawn(async move |this, cx| {
                    let was = sig.clone();
                    let now = cx.background_spawn(async move { repo.status_signature() }).await;
                    if now != sig {
                        this.update(cx, |this, cx| {
                            if !this.loading && this.staging == 0 && this.signature == was && matches!(this.source, Source::WorkingTree | Source::Branch(_)) {
                                this.changed = true;
                                cx.notify();
                            }
                        })
                        .ok();
                    }
                })
                .detach();
            }
        }));
        subs.push(cx.observe_window_appearance(window, |_, window, cx| {
            crate::apply_theme(window, cx);
            cx.notify();
        }));

        // The review comments go to the clipboard as the window closes,
        // when the settings ask.
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| this.should_close(cx)).unwrap_or(true)
        });
        subs.push(cx.on_release(|this, _| this.stop_all_analyses()));
        let gh = crate::features::Ext::new(&repo, &src, cx);
        let mut this = Self {
            repo,
            root,
            launch: src.clone(),
            source: src,
            settings,
            store,
            focus: cx.focus_handle(),
            branch: String::new(),
            files: Vec::new(),
            commit: None,
            base: String::new(),
            loading: false,
            loaded_once: false,
            load_err: None,
            gen_: 0,
            load_start: Instant::now(),
            load_status: String::new(),
            viewed,
            sidebar_shown: shown,
            sidebar_width: if width >= SIDEBAR_MIN { width } else { SIDEBAR_DEFAULT },
            drag_width: 0.,
            tab: 0,
            filter,
            history_filter,
            pulls_filter,
            tree_scroll: UniformListScrollHandle::new(),
            tree_sel: String::new(),
            tree_rows: Vec::new(),
            closed_dirs: HashSet::new(),
            tree_items: HashMap::new(),
            tree_roots: Vec::new(),
            tree_focus: cx.focus_handle(),
            history: Vec::new(),
            history_limit: HISTORY_PAGE,
            history_more: false,
            history_loading: false,
            history_list: ListState::new(0, ListAlignment::Top, px(400.)),
            history_focus: cx.focus_handle(),
            reloaded: HashSet::new(),
            rows: Vec::new(),
            rows_dirty: true,
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            list_focus: cx.focus_handle(),
            current: 0,
            hscroll: HashMap::new(),
            scroll_selection_dirty: false,
            last_scroll_top: gpui::ListOffset::default(),
            char_w: 0.,
            char_key: (String::new(), 0.),
            sel_file: -1,
            sel_hunk: -1,
            focused_once: false,
            finding: false,
            find_input,
            matches: Vec::new(),
            match_ix: 0,
            matches_for: String::new(),
            file_matches: HashSet::new(),
            sessions: HashMap::new(),
            comments: Comments::new(),
            comment_inputs: HashMap::new(),
            user: String::new(),
            copied_at: None,
            commit_open: false,
            subject,
            body,
            staged: HashMap::new(),
            index_lock: Arc::new(Mutex::new(())),
            staging: 0,
            generate_after_stage: false,
            upstream: diffr_git::Upstream::default(),
            push_busy: false,
            push_done: String::new(),
            msg_gen: Default::default(),
            plan_gen: Default::default(),
            commit_plan: None,
            commit_busy: false,
            commit_err: String::new(),
            commit_done: String::new(),
            committed_files: 0,
            commit_output: String::new(),
            palette_open: false,
            palette_input,
            palette_row: 0,
            help: false,
            toast: None,
            changed: false,
            signature: String::new(),
            ext: gh,
            _subs: subs,
            comment_focused: None,
            resizing: None,
            context_menu: None,
            dialog: None,
            dialog_input,
            dialog_err: String::new(),
            dialog_busy: false,
            bounds_gen: 0,
            scroll_drag: None,
            discarding: None,
            line_drag: None,
            drag_edge: 0,
            list_width: 960.,
            close_prompt: false,
            close_confirmed: false,
            quitting: false,
            quit_choice: None,
            window_active: true,
            branches: Vec::new(),
            app_menu: None,
            app_menu_at: gpui::point(px(8.), px(46.)),
            app_menu_item: None,
            app_menu_anchor: Default::default(),
        };
        this.list.set_scroll_handler(cx.listener(|this, _: &gpui::ListScrollEvent, _, cx| {
            this.scroll_selection_dirty = true;
            cx.notify();
        }));
        this.drag_width = this.sidebar_width;
        this.load(cx);
        this.load_history(cx);
        this.load_user(cx);
        this.watch(cx);
        this.ext_init(cx);
        // The repository goes first in Open Recent, which the menus show.
        this.store.add_recent(&this.root);
        crate::set_menus(cx);
        this
    }

    pub fn split(&self) -> bool {
        self.settings.diff_style != "unified"
    }

    pub fn split_file(&self, f: &FileState) -> bool {
        self.split() && !f.one_sided()
    }

    pub fn code_size(&self) -> f32 {
        self.settings.code_font_size.clamp(8, 32) as f32
    }

    /// The height of a line of code: 20 at 13.
    pub fn line_height(&self) -> f32 {
        (self.code_size() * 20. / 13.).round()
    }

    pub fn filter_text(&self, cx: &App) -> String {
        self.filter.read(cx).text().trim().to_string()
    }

    pub fn query(&self, cx: &App) -> String {
        self.find_input.read(cx).text().to_string()
    }

    pub fn searching(&self, cx: &App) -> bool {
        self.finding && !self.query(cx).trim().is_empty()
    }

    pub fn is_viewed(&self, f: &FileState) -> bool {
        !f.file.fingerprint.is_empty() && self.viewed.get(&f.file.path) == Some(&f.file.fingerprint)
    }

    pub fn set_viewed(&mut self, i: usize, viewed: bool, cx: &mut Context<Self>) {
        let Some(f) = self.files.get_mut(i) else { return };
        let mut fp = String::new();
        if viewed {
            fp = f.file.fingerprint.clone();
            self.viewed.insert(f.file.path.clone(), fp.clone());
        } else {
            self.viewed.remove(&f.file.path);
        }
        f.collapsed = viewed;
        let path = f.file.path.clone();
        if let Some(key) = viewed_key(&self.root, &self.source) {
            let store = self.store.clone();
            cx.background_spawn(async move { store.set_viewed(&key, &path, &fp) }).detach();
        }
        self.rows_dirty = true;
        cx.notify();
    }

    pub fn file_visible(&self, i: usize, cx: &App) -> bool {
        let f = &self.files[i];
        if !f.has_content() {
            return false;
        }
        if !fuzzy_match(&f.file.path, &self.filter_text(cx)) {
            return false;
        }
        if self.searching(cx) {
            return self.file_matches.contains(&i);
        }
        true
    }

    pub fn force_open(&self, i: usize, cx: &App) -> bool {
        self.searching(cx) && self.file_matches.contains(&i)
    }

    pub fn stale(&self, gen_: u64) -> bool {
        self.gen_ != gen_
    }

    /// Reads the changes of the source, then the contents of the files.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.gen_ += 1;
        let gen_ = self.gen_;
        let src = self.source.clone();
        let opts = Options { show_whitespace: self.settings.show_whitespace };
        self.loading = true;
        self.load_start = Instant::now();
        self.load_err = None;
        self.changed = false;
        self.load_status.clear();
        let repo = self.repo.clone();
        let known = self.commit.clone().filter(|c| matches!(&src, Source::Commit(h) if *h == c.hash));
        let pull_job = self.pull_job(&src);
        let loaded_files: HashSet<String> = self
            .files
            .iter()
            .filter(|f| f.loaded)
            .map(|f| format!("{}\0{}", f.file.path, f.file.fingerprint))
            .collect();
        // Load slowly: the window says so after a while.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(220)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        let status_cb = self.status_sink(gen_, cx);
        cx.spawn(async move |this, cx| {
            let job = cx.background_spawn(async move {
                let branch = repo.branch();
                let mut out = Loaded {
                    files: Vec::new(),
                    commit: known,
                    base: String::new(),
                    branch,
                    sig: String::new(),
                    old_rev: String::new(),
                    new_rev: String::new(),
                    err: None,
                    pull: None,
                    staged: None,
                };
                let res: anyhow::Result<()> = (|| {
                    match &src {
                        Source::WorkingTree => {
                            out.sig = repo.status_signature();
                            out.files = repo.working_tree(opts)?;
                            out.staged = Some((repo.index_status().unwrap_or_default(), repo.upstream()));
                            if repo.has_head() {
                                out.old_rev = "HEAD".into();
                            }
                        }
                        Source::Commit(h) => {
                            if out.commit.is_none() {
                                out.commit = Some(repo.commit_info(h)?);
                            }
                            let c = out.commit.as_ref().unwrap();
                            out.files = repo.commit_diff(c, opts)?;
                            out.old_rev = c.parents.first().cloned().unwrap_or_default();
                            out.new_rev = h.clone();
                        }
                        Source::Branch(b) => {
                            out.sig = repo.status_signature();
                            let (files, base) = repo.compare(b, opts)?;
                            out.files = files;
                            out.old_rev = base.clone();
                            out.base = base;
                        }
                        Source::Pull(_) => {
                            let pulled = crate::features::fetch_pull(&repo, pull_job, opts, status_cb)?;
                            out.files = pulled.files.clone();
                            out.base = pulled.merge_base.clone();
                            out.old_rev = pulled.merge_base.clone();
                            out.new_rev = pulled.head.clone();
                            out.pull = Some(pulled);
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = res {
                    out.err = Some(error_text(&e));
                }
                out
            });
            let out = job.await;
            let Some(rest) = this
                .update(cx, |this, cx| {
                    if this.stale(gen_) {
                        return None;
                    }
                    this.loading = false;
                    this.load_err = out.err.clone();
                    this.load_status.clear();
                    this.branch = out.branch.clone();
                    if out.err.is_none() {
                        this.commit = out.commit.clone();
                    }
                    if let Some(p) = out.pull {
                        this.apply_pull(p, cx);
                    }
                    this.base = out.base.clone();
                    match out.staged.clone() {
                        Some((staged, up)) => {
                            if this.staging == 0 {
                                this.staged = staged;
                            }
                            this.upstream = up;
                        }
                        None => this.staged.clear(),
                    }
                    if !out.sig.is_empty() {
                        this.signature = out.sig.clone();
                    }
                    this.set_files(out.files, cx);
                    if !this.loaded_once
                        && this.files.is_empty()
                        && matches!(this.source, Source::WorkingTree | Source::Branch(_))
                    {
                        // Nothing to review: the history shows instead.
                        this.tab = 1;
                    }
                    this.loaded_once = true;
                    cx.notify();
                    if out.err.is_some() {
                        return None;
                    }
                    let rest: Vec<File> = this
                        .files
                        .iter()
                        .filter(|f| !f.loaded && !loaded_files.contains(&format!("{}\0{}", f.file.path, f.file.fingerprint)))
                        .map(|f| f.file.clone())
                        .collect();
                    Some((rest, this.repo.clone()))
                })
                .ok()
                .flatten()
            else {
                return;
            };
            let (files, repo) = rest;
            if files.is_empty() {
                return;
            }
            // Read and color the contents on a few threads, and show them as
            // they come.
            let results: Arc<Mutex<(Vec<Content>, bool)>> = Default::default();
            let sink = results.clone();
            let (old_rev, new_rev) = (out.old_rev, out.new_rev);
            cx.background_spawn(async move {
                let Ok(contents) = repo.new_contents() else {
                    sink.lock().unwrap().1 = true;
                    return;
                };
                let next = Mutex::new(0usize);
                let workers = (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) / 4).clamp(1, 3);
                std::thread::scope(|s| {
                    for _ in 0..workers {
                        s.spawn(|| loop {
                            let i = {
                                let mut n = next.lock().unwrap();
                                let i = *n;
                                *n += 1;
                                i
                            };
                            let Some(f) = files.get(i) else { break };
                            let c = read_content(&repo, &contents, f, &old_rev, &new_rev);
                            sink.lock().unwrap().0.push(c);
                        });
                    }
                });
                let _ = contents.close();
                sink.lock().unwrap().1 = true;
            })
            .detach();
            let mut first = true;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(if first { 30 } else { 60 }))
                    .await;
                first = false;
                let (batch, done) = {
                    let mut r = results.lock().unwrap();
                    (std::mem::take(&mut r.0), r.1)
                };
                let keep = this
                    .update(cx, |this, cx| {
                        if this.stale(gen_) {
                            return false;
                        }
                        if !batch.is_empty() {
                            let by_path: HashMap<String, usize> =
                                this.files.iter().enumerate().map(|(i, f)| (f.file.path.clone(), i)).collect();
                            for c in batch {
                                if let Some(&i) = by_path.get(&c.path) {
                                    c.apply(&mut this.files[i]);
                                }
                            }
                            this.rows_dirty = true;
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep || done && results.lock().unwrap().0.is_empty() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Shows newly read files, keeping what the user did to those that did
    /// not change.
    pub fn set_files(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        let mut prev: HashMap<String, FileState> =
            self.files.drain(..).map(|f| (f.file.path.clone(), f)).collect();
        let refresh = !prev.is_empty();
        self.reloaded.clear();
        let mut next = Vec::with_capacity(files.len());
        for f in files {
            let path = f.path.clone();
            let generated = f.generated || f.directory;
            match prev.remove(&path) {
                Some(mut p) if p.file.fingerprint == f.fingerprint => {
                    p.file = f;
                    p.words = None;
                    next.push(p);
                }
                _ => {
                    let mut fs = FileState::new(f);
                    fs.collapsed = if refresh {
                        self.reloaded.insert(path);
                        generated
                    } else {
                        self.is_viewed(&fs) || generated
                    };
                    next.push(fs);
                }
            }
        }
        self.files = next;
        let paths: Vec<String> = self.files.iter().map(|f| f.file.path.clone()).collect();
        self.comments.prune(paths.iter().map(|s| s.as_str()));
        self.regroup(cx);
        self.build_tree_with(cx);
        self.rows_dirty = true;
    }

    fn load_user(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let name = cx
                .background_spawn(async move {
                    let n = repo.config_value("user.name").trim().to_string();
                    if n.is_empty() { repo.config_value("user.email").trim().to_string() } else { n }
                })
                .await;
            this.update(cx, |this, _| this.user = name).ok();
        })
        .detach();
    }

    pub fn user_name(&self) -> String {
        if self.user.is_empty() { "Git user".into() } else { self.user.clone() }
    }

    pub fn load_history(&mut self, cx: &mut Context<Self>) {
        let limit = self.history_limit;
        self.history_loading = true;
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let commits = cx.background_spawn(async move { repo.log(0, limit).unwrap_or_default() }).await;
            this.update(cx, |this, cx| {
                this.history_more = commits.len() >= limit;
                this.history = commits;
                this.history_loading = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Notices changes of the work tree by polling, as codiff does.
    fn watch(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                // Often while the window is active, rarely otherwise.
                let active = this.update(cx, |this, _| this.window_active).unwrap_or(false);
                cx.background_executor().timer(Duration::from_millis(if active { 2500 } else { 10000 })).await;
                let Ok(Some((repo, sig))) = this.update(cx, |this, _| {
                    if this.loading || this.changed || this.signature.is_empty() {
                        return None;
                    }
                    match this.source {
                        Source::WorkingTree | Source::Branch(_) => Some((this.repo.clone(), this.signature.clone())),
                        _ => None,
                    }
                }) else {
                    if this.upgrade().is_none() {
                        return;
                    }
                    let _ = this.update(cx, |this, cx| this.check_pull(cx));
                    continue;
                };
                let was = sig.clone();
                let now = cx.background_spawn(async move { repo.status_signature() }).await;
                if now != sig {
                    let _ = this.update(cx, |this, cx| {
                        // A change of the index made here is not an edit.
                        if !this.loading && this.staging == 0 && this.signature == was && this.signature != now {
                            this.changed = true;
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
        self.load_history(cx);
        self.refresh_pulls(cx);
    }

    /// Shows another source in the window.
    pub fn set_source(&mut self, src: Source, window: &mut Window, cx: &mut Context<Self>) {
        if self.source == src {
            return;
        }
        self.commit_open = false;
        self.focused_once = true;
        let old = std::mem::replace(&mut self.source, src.clone());
        let commented: HashSet<&str> =
            self.comments.list.iter().filter(|c| c.pending()).map(|c| c.path.as_str()).collect();
        let files = self
            .files
            .iter()
            .filter(|f| commented.contains(f.file.path.as_str()))
            .map(|f| SavedFile { file: f.file.clone(), old_lines: f.old_lines.clone(), new_lines: f.new_lines.clone() })
            .collect();
        let session = Session {
            comments: std::mem::take(&mut self.comments),
            comment_inputs: std::mem::take(&mut self.comment_inputs),
            viewed: std::mem::take(&mut self.viewed),
            files,
        };
        self.sessions.insert(old, session);
        if let Some(s) = self.sessions.remove(&src) {
            self.comments = s.comments;
            self.comment_inputs = s.comment_inputs;
            self.viewed = s.viewed;
        }
        if self.viewed.is_empty()
            && let Some(key) = viewed_key(&self.root, &src)
        {
            self.viewed = self.store.viewed(&key);
        }
        self.files.clear();
        self.rows.clear();
        self.commit = None;
        self.switch_ext(&src, cx);
        self.build_tree();
        self.load_err = None;
        self.list.reset(0);
        self.sel_file = -1;
        self.sel_hunk = -1;
        self.finding = false;
        self.find_input.update(cx, |f, cx| f.set_text("", cx));
        self.hscroll.clear();
        self.current = 0;
        if let Source::Commit(h) = &src {
            self.commit = self.history.iter().find(|c| &c.hash == h).cloned();
        }
        window.set_window_title(&window_title(&self.root, &src));
        crate::set_menus(cx);
        self.load(cx);
    }

    pub fn launch_work_tree(&self) -> Source {
        if matches!(self.launch, Source::Branch(_)) { self.launch.clone() } else { Source::WorkingTree }
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_shown = !self.sidebar_shown;
        if self.sidebar_shown && self.sidebar_width < SIDEBAR_MIN {
            self.sidebar_width = SIDEBAR_DEFAULT;
        }
        self.save_layout(cx);
        cx.notify();
    }

    pub fn save_layout(&self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let (w, s) = (self.sidebar_width, self.sidebar_shown);
        cx.background_spawn(async move { store.set_layout(w, s) }).detach();
    }

    pub fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        for f in &mut self.files {
            f.collapsed = collapsed;
        }
        self.rows_dirty = true;
        cx.notify();
    }

    pub fn toast(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.toast = Some((msg.into(), Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(2100)).await;
            this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|t| t.1.elapsed() >= Duration::from_secs(2)) {
                    this.toast = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn open_in_editor(&mut self, path: &str, line: u32, cx: &mut Context<Self>) {
        let mut abs = std::path::Path::new(&self.root).join(path);
        let mut line = line;
        if !abs.exists() {
            abs = std::path::PathBuf::from(&self.root);
            line = 0;
        }
        let cmd = self.settings.editor_command.clone();
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move { crate::editor::open_editor(&cmd, &root, &abs, line) })
                .await;
            if let Err(e) = res {
                this.update(cx, |this, cx| this.toast(format!("Could not open the file: {e}"), cx)).ok();
            }
        })
        .detach();
    }

    pub fn open_in_editor_abs(&mut self, path: &str, cx: &mut Context<Self>) {
        let cmd = self.settings.editor_command.clone();
        let root = self.root.clone();
        let abs = std::path::PathBuf::from(path);
        cx.background_spawn(async move {
            let _ = crate::editor::open_editor(&cmd, &root, &abs, 0);
        })
        .detach();
    }

    // ---- The rows of the diff surface ----

    pub fn build_rows(&mut self, cx: &mut Context<Self>) {
        self.update_matches(cx);
        let mut rows = Vec::new();
        if matches!(self.source, Source::Commit(_)) && self.commit.is_some() {
            rows.push(Row::new(RowKind::Commit, 0));
        }
        if self.show_pull_header() {
            rows.push(Row::new(RowKind::Pull, 0));
        }
        if self.show_summary() {
            rows.push(Row::new(RowKind::Summary, 0));
        }
        let mut placed = self.new_placement();
        let mut group = -1;
        let split = self.split();
        for fi in 0..self.files.len() {
            if !self.file_visible(fi, cx) {
                continue;
            }
            let fgroup = self.files[fi].group;
            if fgroup >= 0 && fgroup != group {
                group = fgroup;
                let mut r = Row::new(RowKind::Group, fi);
                r.gap = group;
                rows.push(r);
            }
            rows.push(Row::new(RowKind::Header, fi));
            if self.files[fi].collapsed && !self.force_open(fi, cx) {
                rows.push(Row::new(RowKind::End, fi));
                continue;
            }
            self.append_file_notes(&mut rows, fi);
            let has_image = self.files[fi].old_image.is_some() || self.files[fi].new_image.is_some();
            if has_image || self.files[fi].note().is_some() {
                rows.push(Row::new(if has_image { RowKind::Image } else { RowKind::Note }, fi));
                self.append_unplaced(&mut rows, fi, &mut placed);
                rows.push(Row::new(RowKind::End, fi));
                continue;
            }
            self.files[fi].compute_words();
            let f = &self.files[fi];
            let split = split && !f.one_sided();
            let add_gap = |rows: &mut Vec<Row>, this: &Self, placed: &mut crate::features::Placement, g: diffr_git::diff::Gap| {
                if g.count == 0 {
                    return;
                }
                let f = &this.files[fi];
                let mut shown = f.expanded.get(&g.index).copied().unwrap_or_default();
                if !f.can_expand() {
                    shown = GapShown::default();
                } else if g.count <= INLINE_GAP {
                    shown = GapShown { top: g.count, bottom: 0 };
                }
                let mut hidden = g.count as i64 - shown.top as i64 - shown.bottom as i64;
                if hidden <= 0 {
                    shown = GapShown { top: g.count, bottom: 0 };
                    hidden = 0;
                }
                for k in 0..shown.top {
                    this.append_context(rows, fi, g.old_start + k, g.new_start + k, placed);
                }
                if hidden > 0 {
                    let mut r = Row::new(RowKind::Gap, fi);
                    r.gap = g.index as i32;
                    r.old = g.old_start + shown.top;
                    r.new = g.new_start + shown.top;
                    r.count = hidden as u32;
                    rows.push(r);
                }
                for k in g.count - shown.bottom..g.count {
                    this.append_context(rows, fi, g.old_start + k, g.new_start + k, placed);
                }
            };
            let nh = f.file.hunks.len();
            for hi in 0..nh {
                let g = self.files[fi].gap_before(hi);
                add_gap(&mut rows, self, &mut placed, g);
                let lines = &self.files[fi].file.hunks[hi].lines;
                let h = hi as i32;
                if !split {
                    for li in 0..lines.len() {
                        let mut r = Row::new(RowKind::Line, fi);
                        r.hunk = h;
                        r.a = li as i32;
                        rows.push(r);
                        self.append_comments(&mut rows, fi, &lines[li], &mut placed);
                    }
                    continue;
                }
                let mut li = 0;
                while li < lines.len() {
                    if lines[li].kind == diffr_git::LineKind::Context {
                        let mut r = Row::new(RowKind::Line, fi);
                        r.hunk = h;
                        r.a = li as i32;
                        r.b = li as i32;
                        rows.push(r);
                        self.append_comments(&mut rows, fi, &lines[li], &mut placed);
                        li += 1;
                        continue;
                    }
                    let start = li;
                    while li < lines.len() && lines[li].kind == diffr_git::LineKind::Del {
                        li += 1;
                    }
                    let dels = li - start;
                    let add_start = li;
                    while li < lines.len() && lines[li].kind == diffr_git::LineKind::Add {
                        li += 1;
                    }
                    let adds = li - add_start;
                    if dels == 0 && adds == 0 {
                        li += 1;
                        continue;
                    }
                    for k in 0..dels.max(adds) {
                        let mut r = Row::new(RowKind::Line, fi);
                        r.hunk = h;
                        if k < dels {
                            r.a = (start + k) as i32;
                        }
                        if k < adds {
                            r.b = (add_start + k) as i32;
                        }
                        rows.push(r.clone());
                        if r.a >= 0 {
                            self.append_comments(&mut rows, fi, &lines[r.a as usize], &mut placed);
                        }
                        if r.b >= 0 {
                            self.append_comments(&mut rows, fi, &lines[r.b as usize], &mut placed);
                        }
                    }
                }
            }
            if nh > 0 {
                let g = self.files[fi].gap_before(nh);
                add_gap(&mut rows, self, &mut placed, g);
            }
            self.append_unplaced(&mut rows, fi, &mut placed);
            rows.push(Row::new(RowKind::End, fi));
        }
        // The list keeps its place: by the file at the top.
        let top = self.list.logical_scroll_top();
        let anchor = self.rows.get(top.item_ix).map(|r| (r.kind, r.file, r.hunk, r.a, r.b, r.old, r.new));
        self.rows = rows;
        self.rows_dirty = false;
        self.list.reset(self.rows.len());
        if let Some(a) = anchor {
            let ix = self
                .rows
                .iter()
                .position(|r| (r.kind, r.file, r.hunk, r.a, r.b, r.old, r.new) == a)
                .or_else(|| self.rows.iter().position(|r| r.file == a.1 && r.kind == RowKind::Header));
            if let Some(ix) = ix {
                self.list.scroll_to(gpui::ListOffset { item_ix: ix, offset_in_item: top.offset_in_item });
            }
        }
    }

    fn append_context(&self, rows: &mut Vec<Row>, fi: usize, old: u32, new: u32, placed: &mut crate::features::Placement) {
        let mut r = Row::new(RowKind::Line, fi);
        r.old = old;
        r.new = new;
        rows.push(r);
        let line = diffr_git::Line { kind: diffr_git::LineKind::Context, old, new, ..Default::default() };
        self.append_comments(rows, fi, &line, placed);
    }

    /// Adds the threads, the notes and the comments on a line.
    fn append_comments(&self, rows: &mut Vec<Row>, fi: usize, l: &diffr_git::Line, placed: &mut crate::features::Placement) {
        let path = &self.files[fi].file.path;
        self.append_line_extras(rows, fi, l, placed);
        for (ci, c) in self.comments.list.iter().enumerate() {
            if &c.path != path {
                continue;
            }
            let on = (c.side == Side::Old && l.kind != diffr_git::LineKind::Add && c.line == l.old)
                || (c.side == Side::New && l.kind != diffr_git::LineKind::Del && c.line == l.new);
            if on {
                let mut r = Row::new(RowKind::Comment, fi);
                r.item = ci;
                rows.push(r);
            }
        }
    }

    /// Scrolls the surface to a file's card.
    pub fn reveal_file(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.rows_dirty {
            self.build_rows(cx);
        }
        if let Some(r) = self.rows.iter().position(|r| r.kind == RowKind::Header && r.file == i) {
            self.list.scroll_to(gpui::ListOffset { item_ix: r, offset_in_item: px(0.) });
        }
        self.current = i;
        self.scroll_selection_dirty = false;
        cx.notify();
    }

    // ---- The file tree ----

    pub fn build_tree_with(&mut self, _cx: &mut Context<Self>) {
        self.build_tree_inner(None);
    }

    pub fn build_tree(&mut self) {
        self.build_tree_inner(None);
    }

    fn build_tree_inner(&mut self, _: Option<()>) {
        // The filter is read from the field when the tree is drawn; here
        // every file with content counts, then filtered in visible_tree.
        let mut items: HashMap<String, TreeNode> = HashMap::new();
        #[derive(Default)]
        struct Dir {
            dirs: HashMap<String, Dir>,
            order: Vec<(bool, String)>,
            files: HashMap<String, usize>,
        }
        fn tree(files: &[(usize, &str)], prefix: &str, items: &mut HashMap<String, TreeNode>) -> Vec<String> {
            let mut root = Dir::default();
            for &(i, path) in files {
                let parts: Vec<&str> = path.split('/').collect();
                let mut d = &mut root;
                for p in &parts[..parts.len() - 1] {
                    if !d.dirs.contains_key(*p) {
                        d.dirs.insert(p.to_string(), Dir::default());
                        d.order.push((true, p.to_string()));
                    }
                    d = d.dirs.get_mut(*p).unwrap();
                }
                let name = parts[parts.len() - 1].to_string();
                d.files.insert(name.clone(), i);
                d.order.push((false, name));
            }
            fn walk(d: &Dir, path: &str, prefix: &str, files: &[(usize, &str)], items: &mut HashMap<String, TreeNode>) -> Vec<String> {
                let mut keys = Vec::new();
                for (is_dir, name) in &d.order {
                    if !is_dir {
                        let i = d.files[name];
                        let fp = files.iter().find(|f| f.0 == i).map(|f| f.1).unwrap_or("");
                        let key = format!("f:{fp}");
                        items.insert(
                            key.clone(),
                            TreeNode { name: name.clone(), dir: false, file: i, path: String::new(), group: -1, children: vec![] },
                        );
                        keys.push(key);
                        continue;
                    }
                    let mut sub = &d.dirs[name];
                    let mut label = name.clone();
                    let mut dir = format!("{path}{name}");
                    while sub.order.len() == 1 && sub.order[0].0 {
                        let child = &sub.order[0].1;
                        label = format!("{label}/{child}");
                        dir = format!("{dir}/{child}");
                        sub = &sub.dirs[child];
                    }
                    let key = format!("d:{prefix}{dir}");
                    let children = walk(sub, &format!("{dir}/"), prefix, files, items);
                    items.insert(
                        key.clone(),
                        TreeNode { name: label, dir: true, file: usize::MAX, path: dir, group: -1, children },
                    );
                    keys.push(key);
                }
                keys
            }
            walk(&root, "", prefix, files, items)
        }
        let groups = self.group_labels();
        let mut by_group: Vec<Vec<(usize, &str)>> = vec![Vec::new(); groups.len()];
        let mut ungrouped = Vec::new();
        for (i, f) in self.files.iter().enumerate() {
            if !f.has_content() {
                continue;
            }
            if f.group >= 0 && (f.group as usize) < groups.len() {
                by_group[f.group as usize].push((i, f.file.path.as_str()));
            } else {
                ungrouped.push((i, f.file.path.as_str()));
            }
        }
        let mut roots = Vec::new();
        for (gi, files) in by_group.iter().enumerate() {
            if files.is_empty() {
                continue;
            }
            let (gkey, glabel) = &groups[gi];
            let key = format!("g:{gkey}");
            let children = tree(files, &format!("{gkey}:"), &mut items);
            items.insert(
                key.clone(),
                TreeNode { name: glabel.clone(), dir: true, file: usize::MAX, path: String::new(), group: gi as i32, children },
            );
            roots.push(key);
        }
        roots.extend(tree(&ungrouped, "", &mut items));
        self.tree_roots = roots;
        self.tree_items = items;
    }

    /// The rows of the tree, inside the directories open, of the files that
    /// pass the filter.
    pub fn visible_tree(&self, cx: &App) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        let filter = self.filter_text(cx);
        fn walk(this: &Diffr, keys: &[String], depth: usize, rows: &mut Vec<TreeRow>, filter: &str, cx: &App) {
            for k in keys {
                let Some(n) = this.tree_items.get(k) else { continue };
                if !this.tree_has_visible(k, filter, cx) {
                    continue;
                }
                rows.push(TreeRow { key: k.clone(), depth });
                if n.dir && !this.closed_dirs.contains(k) {
                    walk(this, &n.children, depth + 1, rows, filter, cx);
                }
            }
        }
        walk(self, &self.tree_roots, 0, &mut rows, &filter, cx);
        rows
    }

    fn tree_has_visible(&self, key: &str, filter: &str, cx: &App) -> bool {
        let Some(n) = self.tree_items.get(key) else { return false };
        if !n.dir {
            let f = &self.files[n.file];
            if !fuzzy_match(&f.file.path, filter) {
                return false;
            }
            if self.searching(cx) {
                return self.file_matches.contains(&n.file);
            }
            return true;
        }
        n.children.iter().any(|c| self.tree_has_visible(c, filter, cx))
    }

    pub fn tree_count(&self, key: &str, cx: &App) -> usize {
        let Some(n) = self.tree_items.get(key) else { return 0 };
        if !n.dir {
            return 1;
        }
        let filter = self.filter_text(cx);
        n.children
            .iter()
            .map(|k| if self.tree_has_visible(k, &filter, cx) { self.tree_count(k, cx) } else { 0 })
            .sum()
    }

    /// Chooses a row of the tree.
    pub fn choose_tree(&mut self, key: &str, cx: &mut Context<Self>) {
        self.tree_sel = key.to_string();
        if let Some(n) = self.tree_items.get(key)
            && !n.dir
        {
            let f = n.file;
            self.commit_open = false;
            self.reveal_file(f, cx);
        }
        cx.notify();
    }

    pub fn tree_move(&mut self, delta: i32, cx: &mut Context<Self>) {
        let rows = self.visible_tree(cx);
        if rows.is_empty() {
            return;
        }
        let at = rows.iter().position(|r| r.key == self.tree_sel).map(|i| i as i32).unwrap_or(-1);
        let next = (at + delta).clamp(0, rows.len() as i32 - 1) as usize;
        let key = rows[next].key.clone();
        self.choose_tree(&key, cx);
        self.tree_scroll.scroll_to_item(next, ScrollStrategy::Nearest);
    }

    /// Chooses a file's row in the tree, as the surface scrolls to it.
    pub fn select_tree_file(&mut self, i: usize, cx: &App) {
        let key = format!("f:{}", self.files[i].file.path);
        if self.tree_sel == key {
            return;
        }
        self.tree_sel = key.clone();
        let path = self.files[i].file.path.clone();
        let group = self.files[i].group;
        let to_open: Vec<String> = self
            .closed_dirs
            .iter()
            .filter(|k| {
                self.tree_items.get(*k).is_some_and(|n| {
                    n.dir
                        && ((n.group >= 0 && n.group == group)
                            || (!n.path.is_empty() && path.starts_with(&format!("{}/", n.path))))
                })
            })
            .cloned()
            .collect();
        for k in to_open {
            self.closed_dirs.remove(&k);
        }
        if let Some(r) = self.visible_tree(cx).iter().position(|r| r.key == key) {
            self.tree_scroll.scroll_to_item(r, ScrollStrategy::Nearest);
        }
    }

    // ---- Hunks chosen with J and K ----

    /// Where J and K go: the changed hunks of the rows, and the files
    /// collapsed (hunk -1), as (file, hunk, row).
    fn anchors(&mut self, cx: &mut Context<Self>) -> Vec<(usize, i32, usize)> {
        if self.rows_dirty {
            self.build_rows(cx);
        }
        let mut out: Vec<(usize, i32, usize)> = Vec::new();
        let mut seen = HashSet::new();
        for (i, r) in self.rows.iter().enumerate() {
            match r.kind {
                RowKind::Header => {
                    if self.files[r.file].collapsed && !self.force_open(r.file, cx) {
                        out.push((r.file, -1, i));
                    }
                }
                RowKind::Line if r.hunk >= 0 => {
                    if seen.contains(&(r.file, r.hunk)) {
                        continue;
                    }
                    let f = &self.files[r.file];
                    let changed = f.line_at(r.hunk, r.a).is_some_and(|l| l.kind != diffr_git::LineKind::Context)
                        || f.line_at(r.hunk, r.b).is_some_and(|l| l.kind != diffr_git::LineKind::Context);
                    if changed {
                        seen.insert((r.file, r.hunk));
                        out.push((r.file, r.hunk, i));
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub fn next_hunk(&mut self, dir: i32, cx: &mut Context<Self>) {
        let anchors = self.anchors(cx);
        if anchors.is_empty() {
            return;
        }
        let cur = anchors.iter().position(|h| h.0 as i32 == self.sel_file && h.1 == self.sel_hunk);
        let at = match cur {
            Some(c) => (c as i32 + dir).clamp(0, anchors.len() as i32 - 1) as usize,
            None => {
                let first = self.list.logical_scroll_top().item_ix;
                if dir > 0 {
                    anchors.iter().position(|h| h.2 > first).unwrap_or(0)
                } else {
                    anchors.iter().rposition(|h| h.2 < first).unwrap_or(0)
                }
            }
        };
        let (f, h, row) = anchors[at];
        self.sel_file = f as i32;
        self.sel_hunk = h;
        self.ext.note_sel = None;
        if h < 0 {
            self.list.scroll_to(gpui::ListOffset { item_ix: row, offset_in_item: px(0.) });
        } else {
            // Centered, about: a few rows of context above it.
            self.list.scroll_to(gpui::ListOffset { item_ix: row.saturating_sub(8), offset_in_item: px(0.) });
        }
        self.current = f;
        self.scroll_selection_dirty = false;
        self.select_tree_file(f, cx);
        cx.notify();
    }

    pub fn in_line_drag(&self, fi: usize, side: Side, num: u32) -> bool {
        self.line_drag.as_ref().is_some_and(|d| {
            d.file == fi && d.side == side && num > 0 && num >= d.start.min(d.end) && num <= d.start.max(d.end) && d.start != d.end
        })
    }

    /// Ends a drag across lines: a comment on them.
    pub fn finish_line_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.line_drag.take() else { return };
        self.drag_edge = 0;
        let (a, b) = (d.start.min(d.end), d.start.max(d.end));
        self.add_comment(&d.path, d.side, a, b, window, cx);
    }

    /// Scrolls while a drag across lines holds the pointer at the top or
    /// the bottom edge of the surface.
    pub fn line_drag_edge(&mut self, y: f32, cx: &mut Context<Self>) {
        if self.line_drag.is_none() {
            return;
        }
        let b = self.list.viewport_bounds();
        let (top, bottom) = (f32::from(b.origin.y), f32::from(b.origin.y + b.size.height));
        let edge = if y < top + 36. {
            -1
        } else if y > bottom - 36. {
            1
        } else {
            0
        };
        if edge == self.drag_edge {
            return;
        }
        self.drag_edge = edge;
        if edge == 0 {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let go = this
                    .update(cx, |this, cx| {
                        if this.line_drag.is_none() || this.drag_edge == 0 {
                            return false;
                        }
                        this.list.scroll_by(px(this.drag_edge as f32 * 12.));
                        this.scroll_selection_dirty = true;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !go {
                    return;
                }
            }
        })
        .detach();
    }

    pub fn is_selected_line(&self, fi: usize, s: &LineSide) -> bool {
        self.sel_file == fi as i32
            && self.sel_hunk >= 0
            && s.hunk == self.sel_hunk
            && s.kind.is_some_and(|k| k != diffr_git::LineKind::Context)
    }

    /// Comments on the hunk chosen: on its last changed line.
    pub fn comment_on_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sel_file < 0 || self.sel_hunk < 0 {
            return;
        }
        let f = &self.files[self.sel_file as usize];
        let Some(h) = f.file.hunks.get(self.sel_hunk as usize) else { return };
        let changed: Vec<&diffr_git::Line> = h.lines.iter().filter(|l| l.kind != diffr_git::LineKind::Context).collect();
        let adds: Vec<&&diffr_git::Line> = changed.iter().filter(|l| l.kind == diffr_git::LineKind::Add).collect();
        let (side, start, end) = if let (Some(a), Some(b)) = (adds.first(), adds.last()) {
            (Side::New, a.new, b.new)
        } else if let (Some(a), Some(b)) = (changed.first(), changed.last()) {
            (Side::Old, a.old, b.old)
        } else {
            return;
        };
        let path = f.file.path.clone();
        self.add_comment(&path, side, start, end, window, cx);
    }

    /// Adds a comment on lines of a file, or focuses the one there.
    pub fn add_comment(&mut self, path: &str, side: Side, start: u32, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = self.comments.list.iter().find(|c| c.path == path && c.side == side && c.line == line) {
            if let Some(inp) = self.comment_inputs.get(&c.id) {
                inp.update(cx, |i, cx| i.focus(window, cx));
            }
            return;
        }
        // An empty draft elsewhere moves here rather than stay behind.
        let empty: Vec<u64> = self.comments.list.iter().filter(|c| !c.pending()).map(|c| c.id).collect();
        for id in empty {
            self.comments.delete(id);
            self.comment_inputs.remove(&id);
        }
        let start = if start == line { 0 } else { start };
        let id = self.comments.add(path, side, start, line);
        let input = new_input(cx, "Write a review comment…", true);
        input.update(cx, |i, _| i.min_lines = 2);
        let sub = cx.subscribe_in(&input, window, move |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => {
                let text = this.comment_inputs.get(&id).map(|i| i.read(cx).text().to_string()).unwrap_or_default();
                if let Some(c) = this.comments.get_mut(id) {
                    c.text = text;
                }
                cx.notify();
            }
            InputEvent::Submit => {
                if this.source.is_pull() && this.can_write_pull() {
                    this.post_comment(id, true, cx);
                } else {
                    window.focus(&this.list_focus, cx);
                }
            }
            InputEvent::Cancel => {
                // A comment with text asks first; an empty one goes.
                if this.comments.get(id).is_some_and(|c| c.pending()) {
                    this.discarding = Some(id);
                    cx.notify();
                } else {
                    this.delete_comment(id, cx);
                    window.focus(&this.list_focus, cx);
                }
            }
            InputEvent::Blurred
                if this.comments.get(id).is_some_and(|c| c.text.trim().is_empty()) => {
                    this.delete_comment(id, cx);
                }
            _ => {}
        });
        self._subs.push(sub);
        input.update(cx, |i, cx| i.focus(window, cx));
        self.comment_inputs.insert(id, input);
        self.rows_dirty = true;
        cx.notify();
    }

    pub fn delete_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        self.comments.delete(id);
        self.comment_inputs.remove(&id);
        self.rows_dirty = true;
        cx.notify();
    }

    pub fn pending_comments(&self) -> usize {
        self.comments.pending_count()
    }

    pub fn comments_markdown(&self) -> String {
        let files: Vec<diffr_git::FileContents> = self
            .files
            .iter()
            .map(|f| diffr_git::FileContents {
                file: &f.file,
                old_lines: f.old_lines.as_deref(),
                new_lines: f.new_lines.as_deref(),
            })
            .collect();
        self.comments.markdown(&files, &self.settings.review_comments_prefix)
    }

    /// Whether the window may close: with review comments, it asks first
    /// (Copy and Close / Cancel / Close), unless the settings copy them.
    pub fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        let n = self.all_pending_comments();
        if n == 0 || self.close_confirmed {
            self.stop_all_analyses();
            return true;
        }
        if self.settings.copy_comments_on_close {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.all_comments_markdown()));
            self.stop_all_analyses();
            return true;
        }
        self.close_prompt = true;
        cx.notify();
        false
    }

    /// The comments with text, of every source the window reviewed.
    pub fn all_pending_comments(&self) -> usize {
        self.pending_comments() + self.sessions.values().map(|s| s.comments.pending_count()).sum::<usize>()
    }

    /// The comments of every source, each (its name, its comments as
    /// Markdown without the prefix) with the lines around them.
    pub fn comment_sections(&self) -> Vec<(String, String)> {
        let name = |src: &Source| window_title(&self.root, src).trim_end_matches(" · Diffr").to_string();
        let mut out = Vec::new();
        if self.pending_comments() > 0 {
            let files: Vec<diffr_git::FileContents> = self
                .files
                .iter()
                .map(|f| diffr_git::FileContents {
                    file: &f.file,
                    old_lines: f.old_lines.as_deref(),
                    new_lines: f.new_lines.as_deref(),
                })
                .collect();
            out.push((name(&self.source), self.comments.markdown(&files, "")));
        }
        let mut others: Vec<(&Source, &Session)> =
            self.sessions.iter().filter(|(_, s)| s.comments.pending_count() > 0).collect();
        others.sort_by_key(|(src, _)| name(src));
        for (src, s) in others {
            out.push((name(src), s.comments.markdown(&s.files, "")));
        }
        out
    }

    /// The comments of every source the window reviewed, as Markdown.
    pub fn all_comments_markdown(&self) -> String {
        join_comment_sections(&self.comment_sections(), &self.settings.review_comments_prefix)
    }

    /// Stops the agents reviewing for the window, as it closes.
    pub fn stop_all_analyses(&mut self) {
        for st in self.ext.analyses.values() {
            if let Some(c) = &st.cancel {
                c.cancel();
            }
        }
    }

    /// After the close prompt: the window closes, or, quitting, the app
    /// asks the next window.
    pub fn finish_close(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_prompt = false;
        if self.quitting {
            // Quitting: the answer is kept, the app asks the next window.
            self.quit_choice = Some(copy);
            cx.defer(crate::request_quit);
        } else {
            if copy && self.all_pending_comments() > 0 {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.all_comments_markdown()));
            }
            self.close_confirmed = true;
            self.stop_all_analyses();
            window.remove_window();
        }
        cx.notify();
    }

    /// The close prompt dismissed: the window stays, and a quit stops.
    pub fn cancel_close(&mut self, cx: &mut Context<Self>) {
        let quitting = self.quitting;
        self.close_prompt = false;
        if quitting {
            cx.defer(crate::cancel_quit);
        }
        cx.notify();
    }

    /// Closes the window, asking first as `should_close` does.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(cx) {
            window.remove_window();
        }
    }

    /// One comment as Markdown, with the lines around it.
    pub fn comment_markdown(&self, id: u64) -> Option<String> {
        let c = self.comments.get(id)?.clone();
        let files: Vec<diffr_git::FileContents> = self
            .files
            .iter()
            .map(|f| diffr_git::FileContents {
                file: &f.file,
                old_lines: f.old_lines.as_deref(),
                new_lines: f.new_lines.as_deref(),
            })
            .collect();
        Some(diffr_git::comments_markdown(&files, &[c], &self.settings.review_comments_prefix))
    }

    pub fn copy_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(md) = self.comment_markdown(id) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(md));
            self.toast("Copied the comment with its lines", cx);
        }
    }

    pub fn copy_comments(&mut self, cx: &mut Context<Self>) {
        if self.pending_comments() == 0 {
            return;
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.comments_markdown()));
        self.copied_at = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        self.toast(format!("Copied {}", crate::ui::plural(self.pending_comments(), "review comment")), cx);
    }

    // ---- Find in diffs ----

    pub fn update_matches(&mut self, cx: &App) {
        let key = if self.finding { self.query(cx) } else { String::new() };
        if key == self.matches_for {
            return;
        }
        self.matches_for = key.clone();
        self.matches.clear();
        self.file_matches.clear();
        let q = key.trim().to_lowercase();
        if q.is_empty() {
            self.match_ix = 0;
            return;
        }
        let filter = self.filter_text(cx);
        for (fi, f) in self.files.iter().enumerate() {
            if !f.has_content() || !fuzzy_match(&f.file.path, &filter) {
                continue;
            }
            let mut found = false;
            if f.file.path.to_lowercase().contains(&q) || f.file.old_path.to_lowercase().contains(&q) {
                self.matches.push(Match { file: fi, hunk: -1, line: -1 });
                found = true;
            }
            for (hi, h) in f.file.hunks.iter().enumerate() {
                for (li, l) in h.lines.iter().enumerate() {
                    if l.text.to_lowercase().contains(&q) {
                        self.matches.push(Match { file: fi, hunk: hi as i32, line: li as i32 });
                        found = true;
                    }
                }
            }
            if !found && f.note().is_some_and(|n| n.to_lowercase().contains(&q)) {
                self.matches.push(Match { file: fi, hunk: -1, line: -1 });
                found = true;
            }
            if found {
                self.file_matches.insert(fi);
            }
        }
        if self.match_ix >= self.matches.len() {
            self.match_ix = 0;
        }
    }

    pub fn active_match(&self, fi: usize, s: &LineSide) -> bool {
        self.matches
            .get(self.match_ix)
            .is_some_and(|m| m.file == fi && m.hunk == s.hunk && m.line == s.index && s.hunk >= 0)
    }

    pub fn show_match(&mut self) {
        let Some(m) = self.matches.get(self.match_ix).copied() else { return };
        let ix = self.rows.iter().position(|r| {
            r.file == m.file
                && if m.hunk < 0 {
                    r.kind == RowKind::Header
                } else {
                    r.kind == RowKind::Line && r.hunk == m.hunk && (r.a == m.line || r.b == m.line)
                }
        });
        if let Some(ix) = ix {
            self.list.scroll_to_reveal_item(ix);
        }
    }

    pub fn step_match(&mut self, dir: i32, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len() as i32;
        self.match_ix = ((self.match_ix as i32 + dir).rem_euclid(n)) as usize;
        if self.rows_dirty {
            self.build_rows(cx);
        }
        self.show_match();
        cx.notify();
    }

    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finding = true;
        self.find_input.update(cx, |i, cx| i.focus(window, cx));
        self.rows_dirty = true;
        cx.notify();
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finding = false;
        self.find_input.update(cx, |i, cx| i.set_text("", cx));
        self.rows_dirty = true;
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    pub fn char_width(&mut self, window: &mut Window) -> f32 {
        let font = self.code_font_name();
        let key = (font.clone(), self.code_size());
        if self.char_key != key || self.char_w == 0. {
            let run = gpui::TextRun {
                len: 20,
                font: gpui::font(font),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line("00000000000000000000".into(), px(self.code_size()), &[run], None);
            self.char_w = f32::from(line.width) / 20.;
            self.char_key = key;
        }
        self.char_w
    }

    pub fn code_font_name(&self) -> String {
        if self.settings.code_font_family.is_empty() {
            crate::DEFAULT_CODE_FONT.to_string()
        } else {
            self.settings.code_font_family.clone()
        }
    }

    pub fn gutter_width(&mut self, fi: usize, window: &mut Window) -> f32 {
        let digits = self.files[fi].metrics().digits as f32;
        digits * self.char_width(window) + 30.
    }

    pub fn apply_settings(&mut self, s: Settings, cx: &mut Context<Self>) {
        let old = std::mem::replace(&mut self.settings, s);
        if old.show_whitespace != self.settings.show_whitespace {
            self.load(cx);
        }
        self.rows_dirty = true;
        cx.notify();
    }
}

/// The key under which a source's viewed files are remembered: the work
/// tree's and pull requests' only, as commits do not change.
pub fn viewed_key(root: &str, src: &Source) -> Option<String> {
    match src {
        Source::WorkingTree => Some(root.to_string()),
        Source::Branch(b) => Some(format!("{root}@{b}")),
        Source::Pull(p) => Some(format!("{root}#{p}")),
        Source::Commit(_) => None,
    }
}

/// "<repository>[/<source>] · Diffr".
pub fn window_title(root: &str, src: &Source) -> String {
    let mut name = std::path::Path::new(root)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    match src {
        Source::Commit(h) => name = format!("{name}/{}", short_hash(h)),
        Source::Branch(b) => name = format!("{name}/{b}"),
        Source::Pull(p) => {
            if let Some(n) = p.rsplit('#').next() {
                name = format!("{name}/#{n}");
            }
        }
        Source::WorkingTree => {}
    }
    format!("{name} · Diffr")
}

pub fn error_text(e: &anyhow::Error) -> String {
    if let Some(ge) = e.downcast_ref::<diffr_git::Error>() {
        let s = ge.to_string();
        return s.trim().to_string();
    }
    format!("{e:#}")
}
