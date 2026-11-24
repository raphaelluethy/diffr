//! Pull requests on GitHub, after pulls.review: the open ones of the
//! repository's remote, and the review of one, fetched into the clone so
//! it reads as local changes do; its threads under their lines, to reply
//! to and resolve; comments alone or in a review, submitted as a comment,
//! an approval or a request for changes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diffr_git::{File, Options, Repo};
use diffr_github::pull::{GitHubRemote, find_github_remote, plain_body};
use diffr_github::{Client, PullRequest, Repo as GhRepo, Reviews, Summary, Thread, TokenCache};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, FontWeight, MouseButton, SharedString,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};

use crate::features::{AiNote, FileGroup, GroupMode};
use crate::input::{InputEvent, TextInput};
use crate::state::*;
use crate::theme::{alpha, hex, mix, rgba};
use crate::ui::{self, icon, icon_button, pal, tooltip};
use crate::window::{Diffr, new_input};

/// A pull request reviewed.
pub struct PullState {
    pub repo: GhRepo,
    pub number: u64,
    pub meta: Option<PullRequest>,
    /// The head commit reviewed.
    pub head: String,
    pub reviews: Option<Reviews>,
    pub reviews_loading: bool,
    pub reviews_err: String,
    pub reviews_at: Option<Instant>,
    pub viewer: String,
    /// Why the user cannot write, "" when they can.
    pub read_only: String,
    pub checked_at: Instant,
    pub desc_open: bool,
    pub replies: HashMap<i64, Entity<TextInput>>,
    pub editing: i64,
    pub edit_input: Option<Entity<TextInput>>,
    pub opened: HashSet<i64>,
    pub busy: usize,
    pub write_err: String,
    pub submit_open: bool,
    pub submit_input: Option<Entity<TextInput>>,
    pub submit_event: String,
    pub submit_busy: bool,
    pub submit_err: String,
    /// The local comments being posted, and why one could not be.
    pub posting: HashSet<u64>,
    pub post_err: HashMap<u64, String>,
}

impl PullState {
    fn new(repo: GhRepo, number: u64) -> Self {
        Self {
            repo,
            number,
            meta: None,
            head: String::new(),
            reviews: None,
            reviews_loading: false,
            reviews_err: String::new(),
            reviews_at: None,
            viewer: String::new(),
            read_only: String::new(),
            checked_at: Instant::now(),
            desc_open: false,
            replies: HashMap::new(),
            editing: 0,
            edit_input: None,
            opened: HashSet::new(),
            busy: 0,
            write_err: String::new(),
            submit_open: false,
            submit_input: None,
            submit_event: String::new(),
            submit_busy: false,
            submit_err: String::new(),
            posting: HashSet::new(),
            post_err: HashMap::new(),
        }
    }

    pub fn can_write(&self) -> bool {
        self.read_only.is_empty() && !self.viewer.is_empty() && self.meta.is_some()
    }
}

/// The window's state of GitHub, the grouping and the AI review.
pub struct Ext {
    pub group_pref: GroupMode,
    pub groups: Vec<FileGroup>,
    pub notes: HashMap<String, Vec<AiNote>>,
    pub note_sel: Option<AiNote>,
    pub show_ai_card: bool,
    pub analyses: HashMap<Source, crate::ai::AnalysisState>,

    pub gh: Option<GitHubRemote>,
    pub pulls: Vec<Summary>,
    pub pulls_loaded: bool,
    pub pulls_loading: bool,
    pub pulls_err: String,
    pub pulls_scroll: UniformListScrollHandle,
    pub pulls_focus: FocusHandle,
    pub prs: HashMap<Source, PullState>,
    pub token: Arc<TokenCache>,
}

impl Ext {
    pub fn new(repo: &Repo, _src: &Source, cx: &mut App) -> Self {
        let remotes = repo.remotes();
        let mut gh = find_github_remote(remotes.iter().map(|r| (r.name.as_str(), r.url.as_str())), &GhRepo::default());
        // `DIFFR_GITHUB_REMOTE=owner/name` names the GitHub repository of
        // the origin remote, for a remote that is not on github.com.
        if let Ok(v) = std::env::var("DIFFR_GITHUB_REMOTE")
            && let Some((o, n)) = v.split_once('/')
        {
            gh = Some(GitHubRemote { repo: GhRepo::new(o, n), remote: "origin".into() });
        }
        Self {
            group_pref: GroupMode::Auto,
            groups: Vec::new(),
            notes: HashMap::new(),
            note_sel: None,
            show_ai_card: false,
            analyses: HashMap::new(),
            gh,
            pulls: Vec::new(),
            pulls_loaded: false,
            pulls_loading: false,
            pulls_err: String::new(),
            pulls_scroll: UniformListScrollHandle::new(),
            pulls_focus: cx.focus_handle(),
            prs: HashMap::new(),
            token: Arc::new(TokenCache::new()),
        }
    }
}

/// A GitHub client; `DIFFR_GITHUB_API` points it at another server, for
/// tests and screenshots.
pub fn client(token: impl Into<String>) -> Client {
    let c = Client::new(token);
    match std::env::var("DIFFR_GITHUB_API") {
        Ok(api) if !api.is_empty() => c.with_api(api),
        _ => c,
    }
}

/// A pull request read and fetched.
#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct PulledRequest {
    pub meta: Option<PullRequest>,
    pub head: String,
    pub merge_base: String,
    pub files: Vec<File>,
    pub commits: Vec<String>,
}

/// What a load of a pull request needs.
#[derive(Clone)]
pub struct PullJob {
    repo: GhRepo,
    number: u64,
    remote: String,
    token: String,
}

pub type StatusSink = Arc<dyn Fn(String) + Send + Sync>;

/// Reads a pull request from GitHub, fetches its commits into the clone,
/// and reads its changes since it branched.
pub fn fetch_pull(repo: &Repo, job: Option<PullJob>, opts: Options, status: StatusSink) -> anyhow::Result<PulledRequest> {
    let job = job.ok_or_else(|| anyhow::anyhow!("not a pull request"))?;
    let n = job.number;
    let client = client(job.token.clone());
    status(format!("Reading #{n} from GitHub…"));
    let meta = client.pull(&job.repo, n).map_err(|e| anyhow::anyhow!("{e}"))?;
    let head = meta.head.sha.clone();
    let mut base = meta.base.sha.clone();
    let refspecs = diffr_github::pull::fetch_refspecs(&meta, repo.has_commit(&head), repo.has_commit(&base));
    if !refspecs.is_empty() {
        status(format!("Fetching #{n} from {}…", job.remote));
        let specs: Vec<&str> = refspecs.iter().map(|s| s.as_str()).collect();
        let mut ferr = repo.fetch(&job.remote, &specs).err().map(|e| e.to_string());
        let url = format!("{}.git", job.repo.url());
        if ferr.is_some() && job.remote != url && repo.fetch(&url, &specs).is_ok() {
            ferr = None;
        }
        if !repo.has_commit(&head) {
            let why = ferr.unwrap_or_else(|| "GitHub did not send them".into());
            anyhow::bail!("could not fetch the commits of #{n} from {}: {}", job.remote, why.trim());
        }
        if !repo.has_commit(&base)
            && let Ok(r) = repo.resolve("FETCH_HEAD")
        {
            base = r;
        }
    }
    status(format!("Reading the changes of #{n}…"));
    let (files, merge_base) = repo.range(&base, &head, opts)?;
    let commits = repo.subjects(&merge_base, &head, 250);
    Ok(PulledRequest { meta: Some(meta), head, merge_base, files, commits })
}

/// Resolves a pull request argument to owner/repo#123.
pub fn resolve_pull_arg(repo: &Repo, arg: &str) -> Result<String, String> {
    let remotes = repo.remotes();
    diffr_github::pull::resolve_pull(
        &repo.root().to_string_lossy(),
        remotes.iter().map(|r| (r.name.as_str(), r.url.as_str())),
        arg,
    )
}

fn pull_of(src: &Source) -> Option<(GhRepo, u64)> {
    match src {
        Source::Pull(r) => diffr_github::parse_pull(r),
        _ => None,
    }
}

fn pull_source(repo: &GhRepo, n: u64) -> Source {
    Source::Pull(diffr_github::pull::pull_ref(repo, n))
}

/// Avatars of GitHub users, fetched once.
static AVATARS: std::sync::OnceLock<Mutex<HashMap<String, Option<Arc<gpui::Image>>>>> = std::sync::OnceLock::new();

fn avatar_image(u: &diffr_github::User) -> Option<Arc<gpui::Image>> {
    if u.login.is_empty() || std::env::var("DIFFR_NO_AVATARS").is_ok() {
        return None;
    }
    let map = AVATARS.get_or_init(Default::default);
    let mut m = map.lock().unwrap();
    if let Some(v) = m.get(&u.login) {
        return v.clone();
    }
    m.insert(u.login.clone(), None);
    let login = u.login.clone();
    let mut url = if u.avatar_url.is_empty() { format!("https://github.com/{login}.png") } else { u.avatar_url.clone() };
    url.push_str(if url.contains('?') { "&s=64" } else { "?s=64" });
    std::thread::spawn(move || {
        let Ok(res) = ureq::get(&url).timeout(Duration::from_secs(20)).call() else { return };
        let mut data = Vec::new();
        if std::io::Read::read_to_end(&mut res.into_reader(), &mut data).is_err() {
            return;
        }
        let fmt = if data.starts_with(&[0x89, b'P', b'N', b'G']) { gpui::ImageFormat::Png } else { gpui::ImageFormat::Jpeg };
        AVATARS.get().unwrap().lock().unwrap().insert(login, Some(Arc::new(gpui::Image::from_bytes(fmt, data))));
    });
    None
}

fn gh_avatar(u: &diffr_github::User, size: f32) -> AnyElement {
    match avatar_image(u) {
        Some(img) => div()
            .size(px(size))
            .flex_none()
            .rounded_full()
            .overflow_hidden()
            .child(gpui::img(img).size(px(size)))
            .into_any_element(),
        None => ui::avatar(&u.login, size).into_any_element(),
    }
}

fn head_line(s: &str) -> &str {
    s.split('\n').next().unwrap_or("")
}

fn ago(t: Option<time::OffsetDateTime>) -> String {
    t.map(|t| format!(" · {}", relative_time(now_secs(), t.unix_timestamp()))).unwrap_or_default()
}

fn thread_label(th: &Thread) -> String {
    let prefix = if th.side == diffr_github::Side::Left { "Old line" } else { "Line" };
    let line = if th.line == 0 { th.original_line } else { th.line };
    if th.start_line != 0 && th.start_line != line {
        format!("{prefix}s {}-{line}", th.start_line)
    } else {
        format!("{prefix} {line}")
    }
}

impl Diffr {
    fn token(&self) -> String {
        self.ext.token.get(&self.settings.github_token)
    }

    pub fn has_github(&self) -> bool {
        self.ext.gh.as_ref().is_some_and(|g| !g.repo.is_zero())
    }

    fn pr(&self) -> Option<&PullState> {
        self.ext.prs.get(&self.source)
    }

    fn pr_mut(&mut self) -> Option<&mut PullState> {
        self.ext.prs.get_mut(&self.source)
    }

    fn pull_for(&mut self, src: &Source) {
        if self.ext.prs.contains_key(src) {
            return;
        }
        if let Some((repo, n)) = pull_of(src) {
            self.ext.prs.insert(src.clone(), PullState::new(repo, n));
        }
    }

    pub fn ext_init(&mut self, cx: &mut Context<Self>) {
        let src = self.source.clone();
        self.pull_for(&src);
        self.load_analysis(&src, cx);
    }

    pub fn switch_ext(&mut self, src: &Source, cx: &mut Context<Self>) {
        self.pull_for(src);
        self.load_analysis(src, cx);
        self.ext.note_sel = None;
    }

    /// What loading the pull request shown needs, on a background thread.
    pub fn pull_job(&self, src: &Source) -> Option<PullJob> {
        let (repo, number) = pull_of(src)?;
        let remotes = self.repo.remotes();
        let remote = diffr_github::pull::pull_remote(
            self.ext.gh.as_ref(),
            remotes.iter().map(|r| (r.name.as_str(), r.url.as_str())),
            &repo,
        );
        Some(PullJob { repo, number, remote, token: self.token() })
    }

    /// Where a slow load says what it does.
    pub fn status_sink(&self, gen_: u64, cx: &mut Context<Self>) -> StatusSink {
        let shared: Arc<Mutex<Option<String>>> = Default::default();
        let sink = shared.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let msg = shared.lock().unwrap().take();
                let alive = this
                    .update(cx, |this, cx| {
                        if this.stale(gen_) || !this.loading {
                            return false;
                        }
                        if let Some(m) = msg {
                            this.load_status = m;
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    return;
                }
            }
        })
        .detach();
        Arc::new(move |s| *sink.lock().unwrap() = Some(s))
    }

    pub fn apply_pull(&mut self, p: PulledRequest, cx: &mut Context<Self>) {
        if let Some(pr) = self.pr_mut() {
            pr.meta = p.meta;
            pr.head = p.head;
            pr.checked_at = Instant::now();
        }
        self.load_reviews(cx);
    }

    pub fn pull_request_info(&self) -> Option<(String, String, String, String)> {
        let pr = self.pr()?;
        let m = pr.meta.as_ref()?;
        Some((m.title.clone(), m.body.clone(), m.html_url.clone(), pr.head.clone()))
    }

    pub fn show_pull_header(&self) -> bool {
        self.pr().is_some_and(|p| p.meta.is_some())
    }

    pub fn can_write_pull(&self) -> bool {
        self.pr().is_some_and(|p| p.can_write())
    }

    pub fn load_reviews(&mut self, cx: &mut Context<Self>) {
        let src = self.source.clone();
        let token = self.token();
        let Some(pr) = self.pr_mut() else { return };
        if pr.reviews_loading {
            return;
        }
        pr.reviews_loading = true;
        let need_viewer = pr.viewer.is_empty() && pr.read_only.is_empty();
        let (repo, n) = (pr.repo.clone(), pr.number);
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let client = client(token.clone());
                    let reviews = client.reviews(&repo, n);
                    let (mut viewer, mut read_only) = (String::new(), String::new());
                    if need_viewer {
                        if token.is_empty() {
                            read_only = diffr_github::pull::READ_ONLY_NO_TOKEN.to_string();
                        } else if let Ok(u) = client.viewer() {
                            viewer = u.login;
                        }
                    }
                    (reviews, viewer, read_only)
                })
                .await;
            this.update(cx, |this, cx| {
                let (reviews, viewer, read_only) = res;
                let Some(pr) = this.ext.prs.get_mut(&src) else { return };
                pr.reviews_loading = false;
                pr.reviews_at = Some(Instant::now());
                match reviews {
                    Ok(r) => {
                        pr.reviews_err.clear();
                        pr.reviews = Some(r);
                    }
                    Err(e) => pr.reviews_err = e.to_string(),
                }
                if !viewer.is_empty() {
                    pr.viewer = viewer;
                }
                if !read_only.is_empty() {
                    pr.read_only = read_only;
                }
                if this.source == src {
                    this.rows_dirty = true;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Writes to GitHub on a background thread, then reads the reviews again.
    fn write(
        &mut self,
        f: impl FnOnce(&Client, &GhRepo, u64) -> Result<(), diffr_github::Error> + Send + 'static,
        done: impl FnOnce(&mut Diffr, &Source, Result<(), String>, &mut Context<Diffr>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let src = self.source.clone();
        let token = self.token();
        let Some(pr) = self.pr_mut() else { return };
        pr.busy += 1;
        pr.write_err.clear();
        let (repo, n) = (pr.repo.clone(), pr.number);
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { f(&client(token), &repo, n) }).await;
            this.update(cx, |this, cx| {
                if let Some(pr) = this.ext.prs.get_mut(&src) {
                    pr.busy = pr.busy.saturating_sub(1);
                    if let Err(e) = &res {
                        pr.write_err = e.to_string();
                        if e.is_forbidden() {
                            pr.read_only = diffr_github::pull::read_only_forbidden(&pr.repo);
                        }
                    }
                }
                let ok = res.is_ok();
                done(this, &src, res.map_err(|e| e.to_string()), cx);
                if ok && this.source == src {
                    if let Some(pr) = this.ext.prs.get_mut(&src) {
                        pr.reviews_loading = false;
                    }
                    this.load_reviews(cx);
                }
                this.rows_dirty = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Posts a local comment: alone, or into the user's review.
    pub fn post_comment(&mut self, id: u64, review: bool, cx: &mut Context<Self>) {
        let Some(c) = self.comments.get(id).cloned() else { return };
        let Some(pr) = self.pr_mut() else { return };
        if !pr.can_write() || !c.pending() || pr.posting.contains(&id) {
            return;
        }
        let mut nc = diffr_github::NewComment {
            body: c.text.trim().to_string(),
            commit_id: pr.head.clone(),
            path: c.path.clone(),
            side: if c.side == Side::Old { diffr_github::Side::Left } else { diffr_github::Side::Right },
            line: c.line,
            start_line: 0,
        };
        if c.start != 0 && c.start != c.line {
            nc.start_line = c.start.min(c.line);
            nc.line = c.start.max(c.line);
        }
        let pending = pr.reviews.as_ref().and_then(|r| r.pending.clone());
        pr.posting.insert(id);
        pr.post_err.remove(&id);
        self.write(
            move |cl, repo, n| if review { cl.add_to_review(repo, n, pending.as_ref(), &nc) } else { cl.comment(repo, n, &nc) },
            move |this, src, res, cx| {
                if let Some(pr) = this.ext.prs.get_mut(src) {
                    pr.posting.remove(&id);
                    if let Err(e) = &res {
                        pr.post_err.insert(id, e.clone());
                        return;
                    }
                }
                if this.source == *src {
                    this.delete_comment(id, cx);
                } else if let Some(s) = this.sessions.get_mut(src) {
                    s.comments.delete(id);
                    s.comment_inputs.remove(&id);
                }
            },
            cx,
        );
    }

    /// Asks GitHub now and then whether new commits were pushed.
    pub fn check_pull(&mut self, cx: &mut Context<Self>) {
        let has_token = !self.ext.token.known(&self.settings.github_token).is_empty();
        let src = self.source.clone();
        let loading = self.loading;
        let token = self.token();
        let Some(pr) = self.pr_mut() else { return };
        if pr.meta.is_none() || loading {
            return;
        }
        if pr.checked_at.elapsed() < diffr_github::pull::check_interval(has_token) {
            return;
        }
        pr.checked_at = Instant::now();
        let (repo, n) = (pr.repo.clone(), pr.number);
        cx.spawn(async move |this, cx| {
            let Ok(latest) = cx.background_spawn(async move { client(token).pull(&repo, n) }).await else { return };
            this.update(cx, |this, cx| {
                if this.source != src {
                    return;
                }
                let loading = this.loading;
                let Some(pr) = this.ext.prs.get_mut(&src) else { return };
                if diffr_github::pull::has_new_commits(&pr.head, &latest) && !loading {
                    this.changed = true;
                }
                let Some(pr) = this.ext.prs.get_mut(&src) else { return };
                if let Some(m) = pr.meta.as_mut() {
                    diffr_github::pull::refresh_meta(m, &latest);
                }
                if pr.reviews_at.is_none_or(|t| t.elapsed() > diffr_github::pull::REVIEWS_REFRESH) {
                    this.load_reviews(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn ensure_pulls(&mut self, cx: &mut Context<Self>) {
        if !self.ext.pulls_loaded && !self.ext.pulls_loading {
            self.load_pulls(cx);
        }
    }

    pub fn refresh_pulls(&mut self, cx: &mut Context<Self>) {
        if self.ext.pulls_loaded {
            self.load_pulls(cx);
        }
    }

    fn load_pulls(&mut self, cx: &mut Context<Self>) {
        let Some(gh) = self.ext.gh.clone() else { return };
        if self.ext.pulls_loading {
            return;
        }
        self.ext.pulls_loading = true;
        let token = self.token();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { client(token).open_pulls(&gh.repo) }).await;
            this.update(cx, |this, cx| {
                this.ext.pulls_loading = false;
                this.ext.pulls_loaded = true;
                match res {
                    Ok(p) => {
                        this.ext.pulls = p;
                        this.ext.pulls_err.clear();
                    }
                    Err(e) => this.ext.pulls_err = e.to_string(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn show_pulls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_github() {
            self.open_dialog(crate::menus::DialogKind::Pull, window, cx);
            return;
        }
        self.set_tab(2, cx);
    }

    pub fn resolve_pull(&mut self, v: &str) -> Result<Source, String> {
        resolve_pull_arg(&self.repo, v).map(Source::Pull)
    }

    fn pulls_list(&self, cx: &App) -> Vec<usize> {
        let q = self.pulls_filter.read(cx).text().trim().to_lowercase();
        self.ext
            .pulls
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                q.is_empty()
                    || p.title.to_lowercase().contains(&q)
                    || p.author.login.to_lowercase().contains(&q)
                    || q.trim_start_matches('#') == p.number.to_string()
                    || p.head_ref.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn pulls_focused(&self, window: &Window) -> bool {
        self.ext.pulls_focus.is_focused(window)
    }

    pub fn pulls_move(&mut self, d: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(gh) = self.ext.gh.clone() else { return };
        let list = self.pulls_list(cx);
        let cur = list.iter().position(|&i| pull_source(&gh.repo, self.ext.pulls[i].number) == self.source);
        let next = match cur {
            Some(c) => (c as i32 + d).clamp(0, list.len() as i32 - 1) as usize,
            None => 0,
        };
        if let Some(&i) = list.get(next) {
            self.ext.pulls_scroll.scroll_to_item(next, gpui::ScrollStrategy::Nearest);
            let src = pull_source(&gh.repo, self.ext.pulls[i].number);
            self.commit_open = false;
            self.set_source(src, window, cx);
        }
    }

    /// The open pull requests of the remote, in the sidebar.
    pub fn pulls_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        self.ensure_pulls(cx);
        let Some(gh) = self.ext.gh.clone() else { return div().into_any_element() };
        let label = if self.ext.pulls_loaded && self.ext.pulls_err.is_empty() {
            format!("{} open on {}", self.ext.pulls.len(), gh.repo)
        } else {
            format!("Open on {}", gh.repo)
        };
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .pl(px(14.))
            .pr(px(10.))
            .pb(px(4.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text_muted)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(label),
            )
            .children(self.ext.pulls_loading.then(|| ui::spinner("pulls-spin", 12., p.text_muted)))
            .child(
                icon_button("pulls-refresh", "refresh", "Refresh pull requests", cx)
                    .size(px(22.))
                    .on_click(cx.listener(|this, _, _, cx| this.load_pulls(cx))),
            );
        let list = self.pulls_list(cx);
        let mut col = div().flex().flex_col().flex_1().min_h_0().child(header);
        if list.is_empty() {
            let q = !self.pulls_filter.read(cx).text().trim().is_empty();
            let msg = if !self.ext.pulls_err.is_empty() {
                self.ext.pulls_err.clone()
            } else if !self.ext.pulls_loaded {
                "Loading pull requests…".into()
            } else if q {
                "No matching pull requests".into()
            } else {
                "No open pull requests".into()
            };
            let mut empty = div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .p(px(16.))
                .gap(px(8.))
                .child(div().text_size(px(12.)).text_color(p.text_muted).text_center().child(msg));
            if !self.ext.pulls_err.is_empty() {
                empty = empty.child(ui::button("pulls-retry", cx).child("Try Again").on_click(cx.listener(|this, _, _, cx| this.load_pulls(cx))));
            }
            empty = empty.child(
                ui::button("pulls-open", cx)
                    .child("Open Pull Request…")
                    .on_click(cx.listener(|this, _, window, cx| this.open_dialog(crate::menus::DialogKind::Pull, window, cx))),
            );
            return col.child(empty).into_any_element();
        }
        let n = list.len();
        let view = cx.entity();
        col = col.child(
            uniform_list("pulls", n, move |range, window, cx| {
                view.update(cx, |this, cx| {
                    let list = this.pulls_list(cx);
                    range.map(|i| this.pull_row(list[i], window, cx)).collect::<Vec<_>>()
                })
            })
            .track_scroll(&self.ext.pulls_scroll)
            .track_focus(&self.ext.pulls_focus)
            .flex_1()
            .px(px(8.))
            .py(px(2.)),
        );
        col.into_any_element()
    }

    fn pull_row(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let Some(gh) = self.ext.gh.clone() else { return div().into_any_element() };
        let pr = self.ext.pulls[i].clone();
        let src = pull_source(&gh.repo, pr.number);
        let current = self.source == src;
        let focused = self.ext.pulls_focus.is_focused(window);
        let selected = current && focused;
        let (muted, refc) = if selected { (p.accent_text, p.accent_text) } else { (p.text_muted, p.reference) };
        let (ic, mut color) = if pr.draft { ("pull_draft", p.text_muted) } else { ("pull", p.add_bar) };
        if selected {
            color = p.accent_text;
        }
        let mut meta = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .text_size(px(10.))
            .child(div().font_family(code_font).text_color(refc).flex_none().child(format!("#{}", pr.number)))
            .child(div().text_color(muted).whitespace_nowrap().overflow_hidden().min_w_0().child(pr.author.login.clone()))
            .child(
                div()
                    .text_color(muted)
                    .flex_none()
                    .child(pr.updated_at.map(|t| relative_time(now_secs(), t.unix_timestamp())).unwrap_or_default()),
            )
            .child(ui::spacer());
        match pr.review_decision.as_str() {
            "approved" => meta = meta.child(icon("check", 12., if selected { p.accent_text } else { p.add_bar })),
            "changes_requested" => meta = meta.child(icon("x_circle", 12., if selected { p.accent_text } else { p.danger })),
            _ => {}
        }
        if !pr.checks.is_empty() {
            let c = match pr.checks.as_str() {
                "success" => p.add_bar,
                "failure" => p.danger,
                _ => hex(0xd4a72c),
            };
            meta = meta.child(div().size(px(7.)).rounded(px(4.)).bg(c).flex_none());
        }
        let mut row = div()
            .id(SharedString::from(format!("pr-{}", pr.number)))
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.))
            .px(px(8.))
            .py(px(6.))
            .mb(px(1.))
            .h(px(46.))
            .rounded(px(6.))
            .child(div().mt(px(1.)).child(icon(ic, 14., color)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(pr.title.clone()),
                    )
                    .child(meta),
            )
            .tooltip(tooltip(format!("#{} {}\n{} wants to merge {}", pr.number, pr.title, pr.author.login, pr.head_ref)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.focus(&this.ext.pulls_focus, cx);
                    if this.source != src {
                        this.commit_open = false;
                        this.set_source(src.clone(), window, cx);
                    }
                }),
            );
        row = if selected {
            row.bg(p.accent).text_color(p.accent_text)
        } else if current {
            row.bg(rgba(127, 127, 127, 0.2))
        } else {
            row.hover(|s| s.bg(rgba(127, 127, 127, 0.08)))
        };
        row.into_any_element()
    }

    /// The chips of the pull request in the toolbar.
    pub fn pull_chips(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = pal(cx).clone();
        let Some((repo, n)) = pull_of(&self.source) else { return vec![] };
        let mut out = vec![ui::chip("pull", format!("#{n}"), p.reference, false, cx).flex_none().into_any_element()];
        if let Some(m) = self.pr().and_then(|pr| pr.meta.clone()) {
            out.push(
                ui::chip("branch", format!("{} ← {}", m.base.reference, m.head.reference), p.text_muted, false, cx)
                    .into_any_element(),
            );
        } else {
            let _ = repo;
        }
        out
    }

    /// The Review button of the toolbar.
    pub fn review_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some(pr) = self.pr() else { return div().into_any_element() };
        if pr.meta.is_none() {
            return div().into_any_element();
        }
        let pending = pr.reviews.as_ref().map(|r| r.pending_comments()).unwrap_or(0);
        let can = pr.can_write();
        let tip = if pr.read_only.is_empty() { "Submit your review".to_string() } else { pr.read_only.clone() };
        let mut b = ui::button("review-btn", cx)
            .h(px(28.))
            .py(px(0.))
            .child(icon("check_circle", 14., p.text))
            .child("Review")
            .tooltip(tooltip(tip));
        if pending > 0 {
            b = b.child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .px(px(6.))
                    .rounded(px(8.))
                    .bg(alpha(p.reference, 0.2))
                    .text_color(p.reference)
                    .child(pending.to_string()),
            );
        }
        if can {
            b = b.on_click(cx.listener(|this, _, _, cx| this.open_submit(cx)));
        } else {
            b = b.opacity(0.5);
        }
        b.into_any_element()
    }

    pub fn open_submit(&mut self, cx: &mut Context<Self>) {
        let input = new_input(cx, "Leave a comment", true);
        input.update(cx, |i, _| i.min_lines = 1);
        let Some(pr) = self.pr_mut() else { return };
        if !pr.can_write() {
            return;
        }
        if let Some(body) = pr.reviews.as_ref().and_then(|r| r.pending.as_ref()).map(|p| p.body.clone()) {
            input.update(cx, |i, cx| i.set_text(body, cx));
        }
        let pr = self.pr_mut().unwrap();
        pr.submit_open = true;
        pr.submit_err.clear();
        if pr.submit_event.is_empty() {
            pr.submit_event = diffr_github::VERDICT_COMMENT.into();
        }
        pr.submit_input = Some(input);
        cx.notify();
    }

    pub fn open_pull_on_github(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.pr().and_then(|p| p.meta.as_ref()).map(|m| m.html_url.clone()) {
            cx.open_url(&url);
        }
    }

    pub fn close_pull_dialogs(&mut self) -> bool {
        if let Some(pr) = self.pr_mut()
            && pr.submit_open
        {
            pr.submit_open = false;
            return true;
        }
        false
    }

    /// The pull request shown, atop its changes.
    pub fn pull_header(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let code_font = ui::code_font(cx);
        let Some(pr) = self.pr() else { return div().into_any_element() };
        let Some(m) = pr.meta.clone() else { return div().into_any_element() };
        let status = m.status();
        let (ic, color) = match status {
            "Draft" => ("pull_draft", p.text_muted),
            "Merged" => ("merge", hex(0x8250df)),
            "Closed" => ("pull_closed", p.danger),
            _ => ("pull", p.add_bar),
        };
        let badge = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .pl(px(8.))
            .pr(px(10.))
            .py(px(4.))
            .rounded(px(14.))
            .bg(color)
            .text_color(gpui::white())
            .flex_none()
            .child(icon(ic, 13., gpui::white()))
            .child(div().text_size(px(12.)).font_weight(FontWeight::BOLD).child(status));
        let verb = if status == "Merged" {
            format!(" merged {} into ", ui::plural(m.commits as usize, "commit"))
        } else {
            format!(" wants to merge {} into ", ui::plural(m.commits as usize, "commit"))
        };
        let url = m.html_url.clone();
        let url2 = url.clone();
        let num = m.number;
        let mut col = div()
            .flex()
            .flex_col()
            .px(px(16.))
            .py(px(14.))
            .gap(px(10.))
            .rounded(px(crate::diffview::CARD_RADIUS))
            .bg(p.header_bg)
            .border_1()
            .border_color(p.card_border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(10.))
                    .child(badge)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .text_size(px(15.))
                                    .child(div().font_weight(FontWeight::BOLD).child(m.title.clone()))
                                    .child(div().text_color(p.text_muted).child(format!("  #{}", m.number))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(gh_avatar(&m.user, 18.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .flex_wrap()
                                            .text_size(px(12.))
                                            .child(div().font_weight(FontWeight::SEMIBOLD).child(m.user.login.clone()))
                                            .child(div().text_color(p.text_muted).child(verb))
                                            .child(div().font_family(code_font.clone()).text_color(p.reference).child(m.base.reference.clone()))
                                            .child(div().text_color(p.text_muted).child(" from "))
                                            .child(div().font_family(code_font.clone()).text_color(p.reference).child(m.head.label.clone()))
                                            .child(div().text_color(p.text_muted).child(ago(m.created_at))),
                                    ),
                            ),
                    )
                    .child(icon_button("pr-copy", "copy", "Copy link", cx).on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
                        this.toast(format!("Copied the link of #{num}"), cx);
                    })))
                    .child(icon_button("pr-open", "open", "Open on GitHub", cx).on_click(move |_, _, cx| cx.open_url(&url2))),
            );
        let mut stats = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .font_family(code_font.clone())
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().text_color(p.add_text).child(format!("+{}", ui::thousands(m.additions as usize))))
                    .child(div().text_color(p.del_text).child(format!(" −{}", ui::thousands(m.deletions as usize))))
                    .child(div().text_color(p.text_muted).child(format!(" · {}", ui::plural(m.changed_files as usize, "file")))),
            );
        if let Some(r) = pr.reviews.as_ref()
            && !r.threads.is_empty()
        {
            let open = r.threads.iter().filter(|t| !t.resolved && !t.outdated).count();
            stats = stats.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.text_muted)
                    .child(format!("{} · {open} unresolved", ui::plural(r.threads.len(), "thread"))),
            );
        }
        if pr.reviews_loading {
            stats = stats.child(ui::spinner("reviews-spin", 12., p.text_muted));
        }
        col = col.child(stats);
        let body = plain_body(&m.body);
        if !body.is_empty() {
            let long = body.matches('\n').count() > 6 || body.len() > 600;
            let shown = if long && !pr.desc_open { body.lines().take(6).collect::<Vec<_>>().join("\n") } else { body.clone() };
            let mut d = div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().text_size(px(13.)).child(crate::markdown::markdown("pr-body", &shown, alpha(p.text, 0.88), cx)));
            if long {
                d = d.child(
                    div()
                        .id("pr-desc")
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.accent_foreground)
                        .cursor_pointer()
                        .child(if pr.desc_open { "Show less" } else { "Show more" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(pr) = this.pr_mut() {
                                pr.desc_open = !pr.desc_open;
                            }
                            cx.notify();
                        })),
                );
            }
            col = col.child(d);
        }
        if let Some(r) = pr.reviews.as_ref()
            && !r.summaries.is_empty()
        {
            let mut s = div().flex().flex_col().gap(px(8.)).pt(px(10.)).border_t_1().border_color(p.card_border);
            for rs in &r.summaries {
                let (verdict, color, ic) = match rs.state.as_str() {
                    "approved" => ("approved", p.add_bar, "check_circle"),
                    "changes_requested" => ("requested changes", p.danger, "x_circle"),
                    "dismissed" => ("was dismissed", p.text_muted, "comment"),
                    _ => ("commented", p.text_muted, "comment"),
                };
                let mut item = div().flex().flex_col().gap(px(4.)).child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(ic, 14., color))
                        .child(gh_avatar(&rs.author, 18.))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .text_size(px(12.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(rs.author.login.clone()))
                                .child(div().text_color(color).child(format!(" {verdict}")))
                                .child(div().text_color(p.text_muted).child(ago(rs.submitted_at))),
                        ),
                );
                let b = plain_body(&rs.body);
                if !b.is_empty() {
                    item = item.child(
                        div()
                            .pl(px(44.))
                            .text_size(px(13.))
                            .child(crate::markdown::markdown(format!("rs-{}", rs.id), &b, alpha(p.text, 0.88), cx)),
                    );
                }
                s = s.child(item);
            }
            col = col.child(s);
        }
        let hint = |id: &'static str, ic: &'static str, text: String, color: gpui::Hsla, dismiss: bool, cx: &mut Context<Self>| {
            let mut h = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .pl(px(10.))
                .pr(px(6.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(alpha(color, 0.08))
                .child(icon(ic, 14., color))
                .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(mix(color, p.text, 0.4)).child(text));
            if dismiss {
                h = h.child(icon_button(id, "close", "Dismiss", cx).size(px(22.)).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(pr) = this.pr_mut() {
                        pr.write_err.clear();
                    }
                    cx.notify();
                })));
            }
            h
        };
        let (we, re, ro) = (pr.write_err.clone(), pr.reviews_err.clone(), pr.read_only.clone());
        if !we.is_empty() {
            col = col.child(hint("pr-we", "alert", we, p.danger, true, cx));
        }
        if !re.is_empty() {
            col = col.child(hint("pr-re", "alert", format!("Could not read the review comments: {re}"), p.danger, false, cx));
        }
        if !ro.is_empty() {
            col = col.child(hint("pr-ro", "alert", ro, p.text_muted, false, cx));
        }
        col.into_any_element()
    }

    pub fn threads_of(&self, path: &str) -> Vec<(usize, &Thread)> {
        let Some(r) = self.pr().and_then(|p| p.reviews.as_ref()) else { return vec![] };
        r.threads.iter().enumerate().filter(|(_, t)| t.path == path).collect()
    }

    pub fn open_threads(&self, path: &str) -> usize {
        self.threads_of(path).iter().filter(|(_, t)| !t.resolved && !t.outdated).count()
    }

    /// A review thread of the pull request, under its line.
    pub fn thread_row(&mut self, r: &Row, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some(pr) = self.pr() else { return div().h(px(0.)).into_any_element() };
        let Some(th) = pr.reviews.as_ref().and_then(|rv| rv.threads.get(r.item)).cloned() else {
            return div().h(px(0.)).into_any_element();
        };
        let folded = (th.resolved || th.outdated) && !pr.opened.contains(&th.root_id);
        let can = pr.can_write();
        let busy = pr.busy > 0;
        let viewer = pr.viewer.clone();
        let editing = pr.editing;
        let root = th.root_id;
        let mut head = div()
            .h(px(34.))
            .flex()
            .flex_row()
            .items_center()
            .pl(px(10.))
            .pr(px(4.))
            .gap(px(8.))
            .child(icon("comment", 13., p.text_muted))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text_muted)
                    .flex_none()
                    .child(thread_label(&th)),
            );
        if !folded {
            head = head.border_b_1().border_color(p.card_border);
        }
        let badge = |text: &'static str, color: gpui::Hsla| {
            div()
                .text_size(px(10.))
                .font_weight(FontWeight::BOLD)
                .px(px(6.))
                .py(px(1.))
                .rounded(px(8.))
                .bg(alpha(color, 0.14))
                .text_color(color)
                .flex_none()
                .child(text)
        };
        if th.pending {
            head = head.child(badge("Pending", p.reference));
        }
        if th.resolved {
            head = head.child(badge("Resolved", p.viewed));
        }
        if th.outdated {
            head = head.child(badge("Outdated", p.text_muted));
        }
        if folded && let Some(first) = th.comments.first() {
            head = head.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("{} ", first.author.login)))
                    .child(div().text_color(p.text_muted).child(head_line(&plain_body(&first.body)).to_string())),
            );
        } else {
            head = head.child(ui::spacer());
        }
        if th.resolved || th.outdated {
            head = head.child(
                div()
                    .id(SharedString::from(format!("th-show-{root}")))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.accent_foreground)
                    .cursor_pointer()
                    .child(if folded { "Show" } else { "Hide" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(pr) = this.pr_mut()
                            && !pr.opened.remove(&root)
                        {
                            pr.opened.insert(root);
                        }
                        this.rows_dirty = true;
                        cx.notify();
                    })),
            );
        }
        if can && !th.node_id.is_empty() && !th.pending {
            let node = th.node_id.clone();
            let resolved = !th.resolved;
            let mut b = ui::button(SharedString::from(format!("th-res-{root}")), cx)
                .h(px(24.))
                .py(px(0.))
                .child(if th.resolved { "Unresolve" } else { "Resolve" });
            if !busy {
                b = b.on_click(cx.listener(move |this, _, _, cx| {
                    let node = node.clone();
                    this.write(move |cl, _, _| cl.resolve(&node, resolved), |_, _, _, _| {}, cx);
                }));
            } else {
                b = b.opacity(0.5);
            }
            head = head.child(b);
        }
        if let Some(url) = th.comments.first().map(|c| c.url.clone()).filter(|u| !u.is_empty()) {
            head = head.child(
                icon_button(SharedString::from(format!("th-open-{root}")), "open", "Open on GitHub", cx)
                    .size(px(24.))
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        let mut box_ = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .rounded(px(12.))
            .border_1()
            .border_color(if th.pending { alpha(p.reference, 0.45) } else { p.card_border })
            .bg(p.header_bg)
            .overflow_hidden()
            .child(head);
        if !folded {
            for c in &th.comments {
                let own = !viewer.is_empty() && c.author.login == viewer && can;
                let cid = c.id;
                let mut top = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::BOLD).child(c.author.login.clone()))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.text_muted)
                            .child(c.created_at.map(|t| relative_time(now_secs(), t.unix_timestamp())).unwrap_or_default()),
                    )
                    .child(ui::spacer());
                if own && editing != cid {
                    let body = c.body.clone();
                    top = top
                        .child(
                            icon_button(SharedString::from(format!("c-edit-{cid}")), "pencil", "Edit comment", cx)
                                .size(px(22.))
                                .on_click(cx.listener(move |this, _, window, cx| this.start_edit(cid, &body, window, cx))),
                        )
                        .child(
                            icon_button(SharedString::from(format!("c-del-{cid}")), "trash", "Delete comment", cx)
                                .size(px(22.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.write(move |cl, repo, _| cl.delete_comment(repo, cid), |_, _, _, _| {}, cx);
                                })),
                        );
                }
                let mut content = div().flex().flex_col().flex_1().min_w_0().gap(px(4.)).child(top);
                if editing == cid
                    && let Some(inp) = self.pr().and_then(|p| p.edit_input.clone())
                {
                    content = content
                        .child(
                            div()
                                .min_h(px(60.))
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(8.))
                                .bg(p.code_bg)
                                .border_1()
                                .border_color(alpha(p.accent, 0.5))
                                .text_size(px(13.))
                                .line_height(px(18.))
                                .child(inp),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .justify_end()
                                .gap(px(8.))
                                .child(ui::button(SharedString::from(format!("c-cancel-{cid}")), cx).child("Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        if let Some(pr) = this.pr_mut() {
                                            pr.editing = 0;
                                        }
                                        cx.notify();
                                    }),
                                ))
                                .child(
                                    ui::primary_button(SharedString::from(format!("c-save-{cid}")), cx)
                                        .child("Save")
                                        .on_click(cx.listener(move |this, _, _, cx| this.save_edit(cx))),
                                ),
                        );
                } else {
                    content = content.child(
                        div().text_size(px(13.)).child(crate::markdown::markdown(format!("tc-{cid}"), &plain_body(&c.body), p.text, cx)),
                    );
                }
                box_ = box_.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(10.))
                        .child(gh_avatar(&c.author, 24.))
                        .child(content),
                );
            }
            if can && !th.pending {
                box_ = box_.child(self.reply_box(root, window, cx));
            }
        }
        crate::diffview::card(&p).px(px(16.)).py(px(8.)).child(box_).into_any_element()
    }

    fn start_edit(&mut self, id: i64, body: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = new_input(cx, "", true);
        input.update(cx, |i, cx| {
            i.min_lines = 3;
            i.set_text(body.to_string(), cx);
            i.focus(window, cx);
        });
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::Submit => this.save_edit(cx),
            InputEvent::Cancel => {
                if let Some(pr) = this.pr_mut() {
                    pr.editing = 0;
                }
                cx.notify();
            }
            _ => {}
        });
        self._subs.push(sub);
        if let Some(pr) = self.pr_mut() {
            pr.editing = id;
            pr.edit_input = Some(input);
        }
        self.rows_dirty = true;
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(pr) = self.pr() else { return };
        let id = pr.editing;
        let body = pr.edit_input.as_ref().map(|i| i.read(cx).text().trim().to_string()).unwrap_or_default();
        if body.is_empty() || id == 0 {
            return;
        }
        self.write(
            move |cl, repo, _| cl.edit_comment(repo, id, &body),
            |this, src, res, _| {
                if res.is_ok()
                    && let Some(pr) = this.ext.prs.get_mut(src)
                {
                    pr.editing = 0;
                }
            },
            cx,
        );
    }

    fn reply_box(&mut self, root: i64, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let input = match self.pr().and_then(|pr| pr.replies.get(&root).cloned()) {
            Some(i) => i,
            None => {
                let i = new_input(cx, "Reply…", true);
                i.update(cx, |i, _| i.min_lines = 3);
                let sub = cx.subscribe_in(&i, window, move |this, _, ev: &InputEvent, _, cx| match ev {
                    InputEvent::Submit => this.send_reply(root, cx),
                    _ => cx.notify(),
                });
                self._subs.push(sub);
                if let Some(pr) = self.pr_mut() {
                    pr.replies.insert(root, i.clone());
                }
                i
            }
        };
        let focused = input.read(cx).is_focused(window);
        let has_text = !input.read(cx).text().trim().is_empty();
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(12.))
            .pt(px(8.))
            .pb(px(10.))
            .border_t_1()
            .border_color(p.card_border)
            .child(
                div()
                    .min_h(px(34.))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(8.))
                    .bg(p.code_bg)
                    .border_1()
                    .border_color(if focused { alpha(p.accent, 0.6) } else { p.card_border })
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .child(input),
            );
        if has_text {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .child(div().text_size(px(11.)).text_color(p.text_muted).child("Ctrl+↩ replies"))
                    .child(
                        ui::primary_button(SharedString::from(format!("reply-{root}")), cx)
                            .child("Reply")
                            .on_click(cx.listener(move |this, _, _, cx| this.send_reply(root, cx))),
                    ),
            );
        }
        col.into_any_element()
    }

    fn send_reply(&mut self, root: i64, cx: &mut Context<Self>) {
        let Some(pr) = self.pr() else { return };
        if pr.busy > 0 {
            return;
        }
        let Some(input) = pr.replies.get(&root).cloned() else { return };
        let body = input.read(cx).text().trim().to_string();
        if body.is_empty() {
            return;
        }
        self.write(
            move |cl, repo, n| cl.reply(repo, n, root, &body),
            move |_, _, res, cx| {
                if res.is_ok() {
                    input.update(cx, |i, cx| i.set_text("", cx));
                }
            },
            cx,
        );
    }

    /// Under a local comment on a pull request: post it alone, or into the
    /// review.
    pub fn pr_comment_actions(&mut self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some(pr) = self.pr() else { return div().into_any_element() };
        let can_write = pr.can_write();
        let err = pr.post_err.get(&id).cloned();
        let posting = pr.posting.contains(&id);
        let has_pending = pr.reviews.as_ref().is_some_and(|r| r.pending.is_some());
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(8.))
            .border_t_1()
            .border_color(p.card_border);
        if let Some(e) = err {
            row = row.child(div().flex_1().min_w_0().text_size(px(12.)).text_color(p.danger_foreground).child(e));
        } else if !can_write {
            return row
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.))
                        .text_color(p.text_muted)
                        .child("Kept here: copy it with the others as Markdown."),
                )
                .into_any_element();
        } else {
            row = row.child(div().flex_1().min_w_0().text_size(px(11.)).text_color(p.text_muted).child("Ctrl+↩ adds it to your review"));
        }
        let can = self.comments.get(id).is_some_and(|c| c.pending()) && !posting;
        let label = if posting {
            "Posting…"
        } else if has_pending {
            "Add Review Comment"
        } else {
            "Start a Review"
        };
        let mut single = ui::button(SharedString::from(format!("single-{id}")), cx).child("Add Single Comment");
        let mut review = ui::primary_button(SharedString::from(format!("review-{id}")), cx).child(label);
        if can {
            single = single.on_click(cx.listener(move |this, _, _, cx| this.post_comment(id, false, cx)));
            review = review.on_click(cx.listener(move |this, _, _, cx| this.post_comment(id, true, cx)));
        } else {
            single = single.opacity(0.5);
            review = review.opacity(0.5);
        }
        row.child(single).child(review).into_any_element()
    }

    /// The dialog submitting the review.
    pub fn pull_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = pal(cx).clone();
        let Some(pr) = self.pr() else { return vec![] };
        if !pr.submit_open {
            return vec![];
        }
        let Some(input) = pr.submit_input.clone() else { return vec![] };
        let own = pr.meta.as_ref().is_some_and(|m| !pr.viewer.is_empty() && m.user.login == pr.viewer);
        let drafts = pr.reviews.as_ref().map(|r| r.pending_comments()).unwrap_or(0);
        let has_pending = pr.reviews.as_ref().is_some_and(|r| r.pending.is_some());
        let title = pr.meta.as_ref().map(|m| m.title.clone()).unwrap_or_default();
        let mut detail = format!("#{} {title}", pr.number);
        if drafts > 0 {
            detail = format!("{} goes with it. {detail}", ui::plural(drafts, "pending comment"));
        }
        let event = if own { diffr_github::VERDICT_COMMENT.to_string() } else { pr.submit_event.clone() };
        let busy = pr.submit_busy;
        let err = pr.submit_err.clone();
        let focused = input.read(cx).is_focused(window);
        let option = |ev: &'static str, label: &'static str, detail: &'static str, disabled: bool, cx: &mut Context<Self>| {
            let on = event == ev;
            let dot = div().size(px(16.)).rounded(px(8.)).flex().items_center().justify_center().flex_none().mt(px(1.));
            let dot = if on {
                dot.bg(p.accent).child(div().size(px(6.)).rounded(px(3.)).bg(p.accent_text))
            } else {
                dot.border(px(1.5)).border_color(rgba(127, 127, 127, 0.45))
            };
            let mut r = div()
                .id(SharedString::from(format!("verdict-{ev}")))
                .flex()
                .flex_row()
                .items_start()
                .gap(px(10.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(8.))
                .child(dot)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(label))
                        .child(div().text_size(px(12.)).text_color(p.text_muted).child(detail)),
                );
            if on {
                r = r.bg(p.surface_hover);
            }
            if disabled {
                r = r.opacity(0.5).tooltip(tooltip("Authors cannot approve their own pull requests, or request changes of them"));
            } else {
                let hover = p.surface_hover;
                r = r.hover(move |s| s.bg(hover)).cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(pr) = this.pr_mut() {
                        pr.submit_event = ev.to_string();
                    }
                    cx.notify();
                }));
            }
            r
        };
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child("Finish Your Review"))
                    .child(div().text_size(px(13.)).text_color(p.text_muted).child(detail)),
            )
            .child(
                div()
                    .min_h(px(110.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .bg(p.surface)
                    .border_1()
                    .border_color(if focused { p.accent } else { p.border })
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .child(input),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(option(diffr_github::VERDICT_COMMENT, "Comment", "Submit general feedback without explicit approval.", false, cx))
                    .child(option(diffr_github::VERDICT_APPROVE, "Approve", "Submit feedback and approve merging these changes.", own, cx))
                    .child(option(
                        diffr_github::VERDICT_REQUEST_CHANGES,
                        "Request changes",
                        "Submit feedback that must be addressed before merging.",
                        own,
                        cx,
                    )),
            );
        if !err.is_empty() {
            content = content.child(div().text_size(px(12.)).text_color(p.danger_foreground).child(err));
        }
        let mut buttons = div().flex().flex_row().gap(px(8.));
        if has_pending {
            buttons = buttons.child(ui::button("discard-review", cx).child("Discard Pending Review").on_click(cx.listener(
                |this, _, _, cx| {
                    let pending = this.pr().and_then(|p| p.reviews.as_ref()).and_then(|r| r.pending.clone());
                    let Some(pending) = pending else { return };
                    if let Some(pr) = this.pr_mut() {
                        pr.submit_busy = true;
                    }
                    this.write(
                        move |cl, repo, n| cl.discard_review(repo, n, &pending),
                        |this, src, res, _| {
                            if let Some(pr) = this.ext.prs.get_mut(src) {
                                pr.submit_busy = false;
                                match res {
                                    Ok(()) => pr.submit_open = false,
                                    Err(e) => pr.submit_err = e,
                                }
                            }
                        },
                        cx,
                    );
                },
            )));
        }
        buttons = buttons
            .child(ui::spacer())
            .child(ui::button("submit-cancel", cx).child("Cancel").on_click(cx.listener(|this, _, _, cx| {
                this.close_pull_dialogs();
                cx.notify();
            })))
            .child(
                ui::primary_button("submit-review", cx)
                    .child(if busy { "Submitting…" } else { "Submit Review" })
                    .on_click(cx.listener(|this, _, _, cx| this.submit_review(cx))),
            );
        content = content.child(buttons);
        vec![crate::menus::modal(
            "submit",
            520.,
            content,
            cx.listener(|this, _, _, cx| {
                this.close_pull_dialogs();
                cx.notify();
            }),
            cx,
        )]
    }

    fn submit_review(&mut self, cx: &mut Context<Self>) {
        let Some(pr) = self.pr() else { return };
        if pr.submit_busy {
            return;
        }
        let body = pr.submit_input.as_ref().map(|i| i.read(cx).text().trim().to_string()).unwrap_or_default();
        let own = pr.meta.as_ref().is_some_and(|m| !pr.viewer.is_empty() && m.user.login == pr.viewer);
        let event = if own { diffr_github::VERDICT_COMMENT.to_string() } else { pr.submit_event.clone() };
        let drafts = pr.reviews.as_ref().map(|r| r.pending_comments()).unwrap_or(0);
        let pending = pr.reviews.as_ref().and_then(|r| r.pending.clone());
        if event != diffr_github::VERDICT_APPROVE && body.is_empty() && drafts == 0 {
            if let Some(pr) = self.pr_mut() {
                pr.submit_err = "Write a summary, or add comments to the review.".into();
            }
            cx.notify();
            return;
        }
        if let Some(pr) = self.pr_mut() {
            pr.submit_busy = true;
            pr.submit_err.clear();
        }
        self.write(
            move |cl, repo, n| cl.submit_review(repo, n, pending.as_ref(), &event, &body),
            |this, src, res, _| {
                if let Some(pr) = this.ext.prs.get_mut(src) {
                    pr.submit_busy = false;
                    match res {
                        Ok(()) => {
                            pr.submit_open = false;
                            pr.submit_input = None;
                        }
                        Err(e) => pr.submit_err = e,
                    }
                }
            },
            cx,
        );
    }
}
