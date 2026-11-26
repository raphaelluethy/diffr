//! diffr: a native, minimal, local diff viewer for reviewing Git changes
//! and committing them, in Rust and GPUI.

mod actions;
mod ai;
mod commit;
mod diagrams;
mod diffview;
mod editor;
mod features;
mod input;
mod markdown;
mod menus;
mod notify;
mod pull;
mod scrollbar;
mod settings;
mod sidebar;
mod state;
mod theme;
mod ui;
mod view;
mod window;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use gpui::{
    App, AppContext, AssetSource, Bounds, Context, FontWeight, Render, SharedString, Size,
    TitlebarOptions, Window, WindowAppearance, WindowBounds, WindowOptions, div, point, prelude::*,
    px, size,
};
use gpui_platform::application;
use rust_embed::RustEmbed;

use crate::state::Source;
use crate::theme::Palette;
use crate::ui::{Fonts, Theme, pal};
use crate::window::{Diffr, window_title};

pub const DEFAULT_CODE_FONT: &str = if cfg!(target_os = "macos") { "Menlo" } else { "DejaVu Sans Mono" };
pub const DEFAULT_UI_FONT: &str = if cfg!(target_os = "macos") { ".SystemUIFont" } else { "Noto Sans" };

#[derive(RustEmbed)]
#[folder = "assets"]
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Self::iter().filter(|p| p.starts_with(path)).map(|p| p.to_string().into()).collect())
    }
}

static CONFIG: OnceLock<diffr_git::Config> = OnceLock::new();

/// The settings file, ~/.diffr/diffr.jsonc.
pub fn config() -> &'static diffr_git::Config {
    CONFIG.get_or_init(|| {
        let c = diffr_git::Config::new();
        c.load();
        c
    })
}

/// Picks the palette for the window's appearance and the theme setting.
pub fn apply_theme(window: &Window, cx: &mut App) {
    let setting = config().get().theme;
    let dark = match setting.as_str() {
        "dark" => true,
        "light" => false,
        _ => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
    };
    let dark = std::env::var("DIFFR_THEME").map(|t| t == "dark").unwrap_or(dark);
    if cx.try_global::<Theme>().is_none_or(|t| t.0.dark != dark) {
        cx.set_global(Theme(if dark { Palette::dark() } else { Palette::light() }));
    }
}

/// What the command line asks for.
#[derive(Debug)]
pub struct Request {
    pub dir: PathBuf,
    pub src: Source,
}

gpui::actions!(diffr_app, [OpenFolder, OpenConfig, InstallCli, OpenRecent, About]);

/// Brings a window of the Window menu forward, by its place in the list.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = diffr_app, no_json)]
pub struct ActivateWindow {
    pub index: usize,
}

/// The About window: the name, the version and the licenses.
struct AboutView;

impl Render for AboutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        apply_theme(window, cx);
        let p = pal(cx).clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .p(px(24.))
            .bg(p.app_bg)
            .text_color(p.text)
            .font_family(cx.global::<Fonts>().ui.clone())
            .child(gpui::img("logo.png").size(px(80.)))
            .child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).child("Diffr"))
            .child(div().text_size(px(12.)).text_color(p.text_muted).child(format!("Version {}", env!("CARGO_PKG_VERSION"))))
            .child(div().text_size(px(12.)).text_align(gpui::TextAlign::Center).text_color(p.text_muted).child(
                "A native diff viewer in Rust and GPUI. GPL-3.0-or-later.\n\
                 File icons © 2025 Pierre Computer Company, Apache License 2.0.\n\
                 Interface icons © Lucide Icons and Contributors, ISC License.\n\
                 The licenses of the libraries it uses come with the app\n\
                 (THIRD_PARTY_LICENSES.html, NOTICE.md).",
            ))
    }
}

fn open_about(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(440.), px(340.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("About Diffr".into()), ..Default::default() }),
            is_resizable: false,
            ..Default::default()
        },
        |_, cx| cx.new(|_| AboutView),
    )
    .ok();
}

/// Asks for a folder and opens its repository in a window of its own.
fn open_folder(cx: &mut App) {
    let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = rx.await
            && let Some(dir) = paths.into_iter().next()
        {
            cx.update(|cx| open_repo(Request { dir, src: Source::WorkingTree }, cx));
        }
    })
    .detach();
}

/// Set once a quit is decided and the comments copied.
static QUIT_COMMITTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Quits, once every window with review comments has asked what to do
/// with them.
pub fn request_quit(cx: &mut App) {
    // Already copied and on its way out: a second Quit changes nothing.
    if QUIT_COMMITTED.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    // First, without side effects: a window with comments not decided on
    // asks, and the quit waits for its answer.
    for w in cx.windows() {
        if let Some(h) = w.downcast::<Diffr>() {
            let asks = h
                .update(cx, |this, window, cx| {
                    let undecided = this.all_pending_comments() > 0
                        && !this.settings.copy_comments_on_close
                        && this.quit_choice.is_none();
                    if undecided {
                        this.quitting = true;
                        this.close_prompt = true;
                        window.activate_window();
                        cx.notify();
                    }
                    undecided
                })
                .unwrap_or(false);
            if asks {
                return;
            }
        }
    }
    // Every window decided: the comments to keep go to the clipboard at
    // once, the agents stop, and the app quits after a moment for a
    // clipboard manager to take them.
    let mut sections = Vec::new();
    let mut prefix = None;
    for w in cx.windows() {
        if let Some(h) = w.downcast::<Diffr>() {
            h.update(cx, |this, _, _| {
                this.stop_all_analyses();
                let copy = this.all_pending_comments() > 0
                    && (this.settings.copy_comments_on_close || this.quit_choice == Some(true));
                if copy {
                    prefix.get_or_insert_with(|| this.settings.review_comments_prefix.clone());
                    sections.extend(this.comment_sections());
                }
            })
            .ok();
        }
    }
    let md = window::join_comment_sections(&sections, prefix.as_deref().unwrap_or(""));
    QUIT_COMMITTED.store(true, std::sync::atomic::Ordering::SeqCst);
    if md.is_empty() {
        cx.quit();
        return;
    }
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(md));
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(1500)).await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Cancels a quit: no window keeps an answer, nor its prompt.
pub fn cancel_quit(cx: &mut App) {
    for w in cx.windows() {
        if let Some(h) = w.downcast::<Diffr>() {
            h.update(cx, |this, _, cx| {
                this.quitting = false;
                this.quit_choice = None;
                this.close_prompt = false;
                cx.notify();
            })
            .ok();
        }
    }
}

/// Installs the command and says how it went: in the window active, and
/// as a notification.
pub fn report_install(cx: &mut App) {
    let (title, body) = match install_cli() {
        Ok(p) => {
            let on_path = std::env::var_os("PATH")
                .is_some_and(|v| std::env::split_paths(&v).any(|d| Some(d.as_path()) == p.parent()));
            let hint = if on_path { String::new() } else { format!(" Add {} to your PATH.", p.parent().map(|d| d.display().to_string()).unwrap_or_default()) };
            ("The diffr command is installed".to_string(), format!("It is at {}. Run diffr in a Git repository to review its changes.{hint}", p.display()))
        }
        Err(e) => ("Could not install the diffr command".to_string(), e.to_string()),
    };
    let msg = format!("{title}. {body}");
    let mut shown = false;
    for w in cx.windows() {
        if let Some(h) = w.downcast::<Diffr>()
            && h.update(cx, |this, window, cx| {
                if window.is_window_active() {
                    this.toast(msg.clone(), cx);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false)
        {
            shown = true;
            break;
        }
    }
    if !shown {
        notify::notify(&title, &body, None);
    }
}

/// A desktop entry: diffr in the launcher, for folders, and for diffr://
/// links.
#[cfg(target_os = "linux")]
fn install_desktop_entry(exe: &Path) {
    let Some(dir) = dirs::data_dir().map(|d| d.join("applications")) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Diffr\nComment=Review the changes of a Git repository\nExec={} %u\nTerminal=false\nCategories=Development;RevisionControl;\nMimeType=x-scheme-handler/diffr;inode/directory;\nActions=open-folder;\n\n[Desktop Action open-folder]\nName=Open Folder…\nExec={}\n",
        exe.display(),
        exe.display()
    );
    let _ = std::fs::write(dir.join("dev.diffr.desktop"), entry);
    let _ = std::process::Command::new("xdg-mime").args(["default", "dev.diffr.desktop", "x-scheme-handler/diffr"]).output();
}

/// Writes the diffr command into ~/.local/bin, which opens the app on the
/// repository it runs in.
pub fn install_cli() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let mut dirs_ = vec![PathBuf::from("/usr/local/bin")];
    if let Some(h) = dirs::home_dir() {
        dirs_.push(h.join(".local").join("bin"));
    }
    let script = format!(
        "#!/bin/sh\n# Reviews the changes of the Git repository here with diffr.\n{:?} --cwd \"$PWD\" \"$@\" >/dev/null 2>&1 &\n",
        exe
    );
    let mut last = None;
    for (i, d) in dirs_.iter().enumerate() {
        if !d.is_dir() && i + 1 < dirs_.len() {
            continue;
        }
        let _ = std::fs::create_dir_all(d);
        let path = d.join("diffr");
        match std::fs::write(&path, &script) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
                }
                #[cfg(target_os = "linux")]
                install_desktop_entry(&exe);
                return Ok(path);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("no directory to install into")))
}

/// The menu bar, with the settings' checkmarks.
pub fn set_menus(cx: &mut App) {
    use crate::actions as a;
    use crate::input as i;
    use gpui::{Menu, MenuItem, OsAction};
    let s = config().get();
    let check = |name: &'static str, action: Box<dyn gpui::Action>, on: bool| MenuItem::Action {
        name: name.into(),
        action,
        os_action: None,
        checked: on,
        disabled: false,
    };
    let menu = |name: &'static str, items: Vec<MenuItem>| Menu { name: name.into(), items, disabled: false };
    let agent = s.ai_agent.as_str();
    let info = s.ai_file_info.as_str();
    let store = diffr_git::Store::open_default();
    let mut recent: Vec<MenuItem> = store
        .recent()
        .into_iter()
        .map(|r| {
            let label = state::abbreviate_home(&r);
            MenuItem::action(label, a::OpenRecentPath { path: r })
        })
        .collect();
    if !recent.is_empty() {
        recent.push(MenuItem::separator());
    }
    recent.push(MenuItem::action("Clear Menu", a::ClearRecent));
    cx.set_menus(vec![
        menu(
            "Diffr",
            vec![
                MenuItem::action("About Diffr", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", a::OpenSettings),
                MenuItem::action("Open Config File…", OpenConfig),
                MenuItem::action("Install Command Line Tool…", InstallCli),
                MenuItem::separator(),
                MenuItem::action("Quit Diffr", a::Quit),
            ],
        ),
        menu(
            "File",
            vec![
                MenuItem::action("Open Folder…", OpenFolder),
                MenuItem::submenu(menu("Open Recent", recent)),
                MenuItem::separator(),
                MenuItem::action("Show Uncommitted Changes", a::ShowWorkTree),
                MenuItem::action("Open Commit…", a::OpenCommitDialog),
                MenuItem::action("Open Branch…", a::OpenBranchDialog),
                MenuItem::action("Open Pull Request…", a::OpenPullDialog),
                MenuItem::separator(),
                MenuItem::action("Commit…", a::ToggleCommit),
                MenuItem::action("Submit Review…", a::SubmitReview),
                MenuItem::separator(),
                MenuItem::action("Open File in Editor", a::OpenInEditor),
                MenuItem::action("Reveal File in File Manager", a::RevealFile),
                MenuItem::separator(),
                MenuItem::action("Close", a::CloseWindow),
            ],
        ),
        menu(
            "Edit",
            vec![
                MenuItem::os_action("Undo", i::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", i::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", i::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", i::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", i::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", i::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find in Diffs", a::FindInDiffs),
                MenuItem::action("Find Next", a::FindNext),
                MenuItem::action("Find Previous", a::FindPrev),
                MenuItem::action("Filter Files", a::FilterFiles),
                MenuItem::separator(),
                MenuItem::action("Copy Review Comments", a::CopyComments),
            ],
        ),
        menu(
            "View",
            vec![
                MenuItem::action("Command Bar…", a::CommandBar),
                MenuItem::action("Toggle Sidebar", a::ToggleSidebar),
                MenuItem::action("Files", a::ShowFiles),
                MenuItem::action("History", a::ShowHistory),
                MenuItem::action("Pull Requests", a::ShowPulls),
                MenuItem::separator(),
                MenuItem::action("Review with AI", a::ReviewWithAi),
                MenuItem::action("Stop the AI Review", a::StopAi),
                check("Show \u{201c}Worth a Look\u{201d} Notes", Box::new(a::ToggleAiNotes), matches!(info, "notes" | "both")),
                MenuItem::submenu(menu(
                    "AI Review in Files",
                    vec![
                        check("Notes Inline", Box::new(a::FileInfoNotes), info == "notes"),
                        check("File Summaries", Box::new(a::FileInfoSummaries), info == "summaries"),
                        check("Both", Box::new(a::FileInfoBoth), info == "both"),
                    ],
                )),
                MenuItem::submenu(menu(
                    "Group Files",
                    vec![
                        MenuItem::action("Not at All", a::GroupNone),
                        MenuItem::action("By Kind", a::GroupKind),
                        MenuItem::action("By AI Review", a::GroupAi),
                    ],
                )),
                MenuItem::submenu(menu(
                    "AI Agent",
                    vec![
                        check("Automatic", Box::new(a::AgentAuto), agent.is_empty()),
                        MenuItem::separator(),
                        check("Claude Code", Box::new(a::AgentClaude), agent == "claude"),
                        check("Codex", Box::new(a::AgentCodex), agent == "codex"),
                        check("OpenCode", Box::new(a::AgentOpencode), agent == "opencode"),
                    ],
                )),
                MenuItem::separator(),
                MenuItem::submenu(menu(
                    "Diff",
                    vec![
                        check("Split", Box::new(a::DiffSplit), s.diff_style != "unified"),
                        check("Unified", Box::new(a::DiffUnified), s.diff_style == "unified"),
                        MenuItem::separator(),
                        check("Word Wrap", Box::new(a::ToggleWrap), s.word_wrap),
                        check("Show Whitespace Changes", Box::new(a::ToggleWhitespace), s.show_whitespace),
                        MenuItem::separator(),
                        MenuItem::action("Collapse All Files", a::CollapseAll),
                        MenuItem::action("Expand All Files", a::ExpandAll),
                    ],
                )),
                MenuItem::submenu(menu(
                    "Comments",
                    vec![check("Copy Review Comments on Close", Box::new(a::ToggleCopyOnClose), s.copy_comments_on_close)],
                )),
                MenuItem::submenu(menu(
                    "Font Size",
                    vec![
                        MenuItem::action("Bigger", a::FontBigger),
                        MenuItem::action("Smaller", a::FontSmaller),
                        MenuItem::action("Actual Size", a::FontReset),
                    ],
                )),
                MenuItem::submenu(menu(
                    "Theme",
                    vec![
                        check("Match System", Box::new(a::ThemeSystem), s.theme == "system" || s.theme.is_empty()),
                        check("Light", Box::new(a::ThemeLight), s.theme == "light"),
                        check("Dark", Box::new(a::ThemeDark), s.theme == "dark"),
                    ],
                )),
                MenuItem::submenu(menu(
                    "Sidebar Position",
                    vec![
                        check("Left", Box::new(a::SidebarLeft), s.sidebar_position != "right"),
                        check("Right", Box::new(a::SidebarRight), s.sidebar_position == "right"),
                    ],
                )),
                MenuItem::separator(),
                MenuItem::action("Enter Full Screen", a::ToggleFullScreen),
            ],
        ),
        menu(
            "Go",
            vec![
                MenuItem::action("Next Hunk", a::NextHunk),
                MenuItem::action("Previous Hunk", a::PrevHunk),
                MenuItem::action("Next File", a::NextFile),
                MenuItem::action("Previous File", a::PrevFile),
                MenuItem::action("Next Group", a::NextGroup),
                MenuItem::action("Previous Group", a::PrevGroup),
                MenuItem::action("Next AI Note", a::NextNote),
                MenuItem::action("Previous AI Note", a::PrevNote),
                MenuItem::separator(),
                MenuItem::action("Refresh Changes", a::Refresh),
            ],
        ),
        menu("Window", {
            // The windows open, to go to one.
            let mut items = Vec::new();
            for (i, w) in cx.windows().iter().enumerate() {
                if let Some(h) = w.downcast::<Diffr>()
                    && let Ok(t) = h.read(cx).map(|d| window_title(&d.root, &d.source))
                {
                    items.push(MenuItem::action(t, ActivateWindow { index: i }));
                }
            }
            if items.is_empty() {
                items.push(MenuItem::action("Open Folder…", OpenFolder));
            }
            items
        }),
        menu("Help", vec![MenuItem::action("Keyboard Shortcuts", a::ShowShortcuts)]),
    ]);
}

fn usage() -> &'static str {
    "Usage: diffr [<commit> | <branch> | <pull request>] [<path>]\n\n\
     Review the uncommitted changes of the Git repository at <path> (default:\n\
     the current directory), a commit, the work tree's changes since it\n\
     branched off <branch>, or a pull request on GitHub, named as\n\
     owner/repo#123 or by its URL.\n\n\
     Options:\n\
     \x20 --commit <ref>   review a commit\n\
     \x20 --branch <ref>   compare the work tree with a branch\n\
     \x20 --pr <number>    review a pull request of the repository's GitHub remote\n\
     \x20 --cwd <dir>      the directory relative paths start from\n\
     \x20 -h, --help       show this help\n"
}

fn resolve(dir: &Path, p: &str) -> PathBuf {
    let p = if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(p))
    } else {
        PathBuf::from(p)
    };
    if p.is_absolute() { p } else { dir.join(p) }
}

/// Whether an argument names a revision rather than a directory:
/// a hash, HEAD or @ with ~ and ^, or @{…}.
fn commit_like(a: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^([0-9a-fA-F]{4,64}|(HEAD|@)([~^][0-9]*)*|.*[~^].*|.*@\{.*\})$").unwrap())
        .is_match(a)
}

fn parse_args(args: &[String], cwd: &Path) -> Result<Request, String> {
    let mut req = Request { dir: cwd.to_path_buf(), src: Source::WorkingTree };
    let mut refs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-h" | "--help" => return Err(usage().into()),
            "--pr" => {
                i += 1;
                let v = args.get(i).ok_or("--pr needs a pull request")?;
                req.src = Source::Pull(v.clone());
            }
            "--commit" | "--branch" => {
                i += 1;
                let v = args.get(i).ok_or(format!("{a} needs a revision"))?;
                req.src = if a == "--commit" { Source::Commit(v.clone()) } else { Source::Branch(v.clone()) };
            }
            "--cwd" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    req.dir = resolve(cwd, v);
                }
            }
            _ if a.starts_with("--cwd=") => req.dir = resolve(cwd, &a["--cwd=".len()..]),
            _ if diffr_github::parse_pull(a).is_some() => req.src = Source::Pull(a.clone()),
            _ if a.starts_with('-') => return Err(format!("unknown option {a}")),
            _ if a.starts_with('/') || a.starts_with("./") || a.starts_with("../") || a.starts_with('~') => {
                req.dir = resolve(cwd, a)
            }
            _ => refs.push(a.clone()),
        }
        i += 1;
    }
    for a in refs {
        let p = resolve(cwd, &a);
        if p.is_dir() && !commit_like(&a) {
            req.dir = p;
            continue;
        }
        if req.src != Source::WorkingTree {
            return Err(format!("unexpected argument {a}"));
        }
        req.src = Source::Branch(a);
    }
    if let Source::Commit(c) = &req.src
        && let Ok(repo) = diffr_git::Repo::open(&req.dir)
        && repo.resolve(c).is_err()
    {
        return Err(format!("{c:?} is no commit of this repository"));
    }
    if let Source::Pull(r) = &req.src
        && let Ok(repo) = diffr_git::Repo::open(&req.dir)
    {
        pull::resolve_pull_arg(&repo, r)?;
    }
    if let Source::Branch(b) = &req.src
        && let Ok(repo) = diffr_git::Repo::open(&req.dir)
    {
        let local = repo.git(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok();
        let remote = repo.git(&["show-ref", "--verify", "--quiet", &format!("refs/remotes/{b}")]).is_ok();
        if !local && !remote {
            match repo.resolve(b) {
                Ok(_) => req.src = Source::Commit(b.clone()),
                Err(_) => return Err(format!("{b:?} is neither a branch nor a commit of this repository")),
            }
        }
    }
    Ok(req)
}

/// The window shown without a repository: what diffr does, its actions,
/// and the repositories opened lately.
struct Welcome {
    /// Why no repository opened, if one was asked for.
    problem: Option<(String, String)>,
}

impl Render for Welcome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        apply_theme(window, cx);
        let p = pal(cx).clone();
        let action = |id: &'static str, icon: &'static str, title: &'static str, detail: &'static str, cx: &App| {
            let hover = p.surface_hover;
            let _ = cx;
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(ui::icon(icon, 16., p.accent_foreground))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_size(px(11.)).text_color(p.text_muted).child(detail)),
                )
        };
        let mut left = div()
            .flex()
            .flex_col()
            .w(px(300.))
            .flex_none()
            .p(px(28.))
            .gap(px(6.))
            .child(
                div()
                    .size(px(64.))
                    .rounded(px(14.))
                    .bg(crate::theme::hex(0xdb2777))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(gpui::white())
                    .text_size(px(32.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("D"),
            )
            .child(div().mt(px(8.)).text_size(px(22.)).font_weight(FontWeight::BOLD).child("Diffr"))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(p.text_muted)
                    .child("Review the changes of a Git repository, a commit, a branch or a pull request."),
            );
        if let Some((title, detail)) = &self.problem {
            left = left.child(
                div()
                    .mt(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(crate::theme::alpha(p.danger, 0.08))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.danger_foreground).child(title.clone()))
                    .child(div().text_size(px(12.)).text_color(p.text_muted).child(detail.clone())),
            );
        }
        left = left
            .child(div().h(px(14.)))
            .child(action("w-open", "folder_open", "Open Folder…", "A Git repository", cx).on_click(|_, _, cx| open_folder(cx)))
            .child(action("w-cli", "command", "Install Command Line Tool…", "Run diffr in a repository", cx).on_click(
                |_, _, cx| report_install(cx),
            ))
            .child(action("w-settings", "sparkle", "Settings…", "Theme, font, agent, GitHub", cx).on_click(|_, _, cx| {
                settings::open_settings(String::new(), cx)
            }));
        let recent = diffr_git::Store::open_default().recent();
        let mut right = div()
            .id("welcome-recent")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .bg(p.sidebar_bg)
            .p(px(12.))
            .gap(px(2.))
            .child(div().px(px(8.)).pb(px(6.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_muted).child("RECENT"));
        if recent.is_empty() {
            right = right.child(div().p(px(8.)).text_size(px(12.)).text_color(p.text_muted).child("Repositories you open show here."));
        }
        for (i, r) in recent.into_iter().enumerate() {
            let path = std::path::Path::new(&r);
            let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let parent = path.parent().map(|d| state::abbreviate_home(&d.to_string_lossy())).unwrap_or_default();
            let (r1, r2) = (r.clone(), r.clone());
            let hover = crate::theme::rgba(127, 127, 127, 0.12);
            right = right.child(
                div()
                    .id(SharedString::from(format!("recent-{i}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(ui::icon("folder", 16., p.text_muted))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(name))
                            .child(div().text_size(px(11.)).text_color(p.text_muted).whitespace_nowrap().overflow_hidden().text_ellipsis().child(parent)),
                    )
                    .tooltip(ui::tooltip(r.clone()))
                    .on_click(cx.listener(move |_, _, window, cx| {
                        open_repo(Request { dir: PathBuf::from(&r1), src: Source::WorkingTree }, cx);
                        window.remove_window();
                    }))
                    .on_mouse_down(gpui::MouseButton::Right, move |_, _, cx| cx.reveal_path(Path::new(&r2))),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(p.app_bg)
            .text_color(p.text)
            .font_family(cx.global::<Fonts>().ui.clone())
            .text_size(px(14.))
            .line_height(gpui::relative(1.36))
            // A folder dropped on the window opens.
            .on_drop(cx.listener(|_, paths: &gpui::ExternalPaths, window, cx| {
                if let Some(dir) = paths.paths().first() {
                    open_repo(Request { dir: dir.clone(), src: Source::WorkingTree }, cx);
                    window.remove_window();
                }
            }))
            .child(left)
            .child(div().w(px(1.)).h_full().bg(p.card_border))
            .child(right)
    }
}

/// Where the window's size and place are kept.
fn window_state_path() -> Option<PathBuf> {
    diffr_git::Store::default_path().and_then(|p| p.parent().map(|d| d.join("window.json")))
}

/// The window's bounds when it last closed: x, y, width, height.
fn saved_bounds() -> Option<Bounds<gpui::Pixels>> {
    let data = std::fs::read(window_state_path()?).ok()?;
    let v: [f32; 4] = serde_json::from_slice(&data).ok()?;
    (v[2] >= 720. && v[3] >= 420.).then(|| Bounds::new(point(px(v[0]), px(v[1])), size(px(v[2]), px(v[3]))))
}

pub fn save_bounds(b: Bounds<gpui::Pixels>) {
    if let Some(p) = window_state_path() {
        let v = [f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)];
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, serde_json::to_vec(&v).unwrap_or_default());
    }
}

fn window_size() -> Size<gpui::Pixels> {
    if let Ok(s) = std::env::var("DIFFR_WINDOW_SIZE")
        && let Some((w, h)) = s.split_once('x')
        && let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>())
    {
        return size(px(w), px(h));
    }
    size(px(1280.), px(860.))
}

pub fn open_repo(req: Request, cx: &mut App) {
    let sz = window_size();
    let fill = std::env::var("DIFFR_WINDOW_SIZE").is_ok();
    let bounds = if fill {
        Bounds::new(point(px(0.), px(0.)), sz)
    } else {
        saved_bounds().unwrap_or_else(|| Bounds::centered(None, sz, cx))
    };
    let repo = match diffr_git::Repo::open(&req.dir) {
        Ok(r) => r,
        Err(e) => {
            let not_repo = e.to_string().to_lowercase().contains("not a git repository")
                || matches!(e, diffr_git::Error::NotRepository(_));
            let (title, detail) = if not_repo {
                (
                    "No Git repository found".to_string(),
                    "diffr was opened outside a Git repository. Run diffr from inside a repository, or choose Open Folder… to open one.".to_string(),
                )
            } else {
                ("Unable to read repository".to_string(), e.to_string())
            };
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(680.), px(440.)), cx))),
                    titlebar: Some(TitlebarOptions { title: Some("Diffr".into()), ..Default::default() }),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Welcome { problem: Some((title, detail)) }),
            )
            .ok();
            return;
        }
    };
    let mut src = req.src;
    if let Source::Commit(r) = &src
        && let Ok(h) = repo.resolve(r)
    {
        src = Source::Commit(h);
    }
    if let Source::Pull(r) = &src
        && let Ok(full) = pull::resolve_pull_arg(&repo, r)
    {
        src = Source::Pull(full);
    }
    // A window on the same repository and source comes forward instead.
    let root = repo.root().to_string_lossy().to_string();
    for w in cx.windows() {
        if let Some(h) = w.downcast::<Diffr>()
            && h.read(cx).is_ok_and(|d| d.root == root && d.launch == src)
        {
            h.update(cx, |_, window, _| window.activate_window()).ok();
            return;
        }
    }
    let title = window_title(&repo.root().to_string_lossy(), &src);
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(18.))),
            }),
            window_min_size: Some(size(px(720.), px(420.))),
            app_id: Some("dev.diffr".into()),
            ..Default::default()
        },
        |window, cx| {
            apply_theme(window, cx);
            cx.new(|cx| Diffr::new(repo, src, window, cx))
        },
    );
    if let Ok(h) = handle {
        h.update(cx, |_, window, _| window.activate_window()).ok();
        set_menus(cx);
    }
}

/// Settings apply to every open window as the file changes, whether in the
/// file or from another window's menus.
fn watch_settings(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let changed = config().load();
            let s = config().get();
            if changed {
                cx.update(set_menus);
            }
            cx.update(|cx| {
                for w in cx.windows() {
                    if let Some(h) = w.downcast::<Diffr>() {
                        let s = s.clone();
                        h.update(cx, |this, window, cx| {
                            apply_theme(window, cx);
                            if this.settings != s {
                                this.apply_settings(s, cx);
                            }
                        })
                        .ok();
                    } else {
                        w.update(cx, |_, window, cx| apply_theme(window, cx)).ok();
                    }
                }
            });
            ();
        }
    })
    .detach();
}

/// `diffr check-diagram [file]`: validates a Mermaid diagram (from the
/// file or stdin), for the agents of the Diagrams lab.
fn check_diagram(args: &[String]) -> ! {
    use std::io::Read;
    // The source: the arguments after the command, a file, or stdin.
    let mut src = String::new();
    let rest = &args[1..];
    let looks_like_file = |a: &str| {
        !a.contains('\n') && !a.contains(' ') && (a.contains('/') || [".mmd", ".mermaid", ".md", ".txt"].iter().any(|e| a.ends_with(e)))
    };
    if rest.len() == 1 && rest[0] != "-" && Path::new(&rest[0]).is_file() {
        src = std::fs::read_to_string(&rest[0]).unwrap_or_default();
    } else if rest.len() == 1 && looks_like_file(&rest[0]) {
        eprintln!("diffr check-diagram: {}: no such file", rest[0]);
        std::process::exit(2);
    } else if !rest.is_empty() && rest[0] != "-" {
        src = rest.join("\n");
    } else if let Err(e) = std::io::stdin().read_to_string(&mut src) {
        eprintln!("diffr check-diagram: {e}");
        std::process::exit(2);
    }
    if src.trim().is_empty() {
        eprintln!("diffr check-diagram: no diagram: give its Mermaid source on stdin");
        std::process::exit(2);
    }
    let (ok, out) = diffr_diagram::check_command_output(&src);
    println!("{out}");
    // DIFFR_DIAGRAM_SVG=<path> keeps what was drawn, to look at.
    if ok
        && let Ok(path) = std::env::var("DIFFR_DIAGRAM_SVG")
        && let Ok(r) = diffr_diagram::render_svg(&src, &diffr_diagram::DiagramTheme::light())
    {
        let _ = std::fs::write(path, r.svg);
    }
    std::process::exit(if ok { 0 } else { 1 });
}

/// A diffr:// URL as arguments: diffr://open?cwd=<dir>&arg=<arg>&arg=…
fn url_args(url: &str) -> Option<Vec<String>> {
    let rest = url.strip_prefix("diffr://")?;
    let (_, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut out = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match k {
            "cwd" => out.extend(["--cwd".to_string(), v]),
            "arg" => out.push(v),
            _ => {}
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// The socket of the running app: a second launch hands it its request.
fn instance_socket() -> Option<PathBuf> {
    if std::env::var("DIFFR_NO_SINGLE_INSTANCE").is_ok() || std::env::var("DIFFR_WINDOW_SIZE").is_ok() {
        return None;
    }
    // The user's own runtime directory; else a name of the user's own in
    // the temporary one.
    if let Some(dir) = std::env::var("XDG_RUNTIME_DIR").ok().map(PathBuf::from) {
        return Some(dir.join("diffr.sock"));
    }
    #[cfg(unix)]
    {
        // A directory of the user's own, which nobody else can enter.
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        let uid = unsafe { libc_getuid() };
        let dir = std::env::temp_dir().join(format!("diffr-{uid}"));
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
        let meta = std::fs::symlink_metadata(&dir).ok()?;
        if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
            return None;
        }
        Some(dir.join("diffr.sock"))
    }
    #[cfg(not(unix))]
    None
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// Hands the request to the running app, if there is one.
#[cfg(unix)]
fn forward_to_running(cwd: &Path, args: &[String]) -> bool {
    use std::io::Write;
    let Some(sock) = instance_socket() else { return false };
    let Ok(mut s) = std::os::unix::net::UnixStream::connect(&sock) else { return false };
    let msg = serde_json::json!({ "cwd": cwd, "args": args }).to_string();
    s.write_all(msg.as_bytes()).is_ok()
}

#[cfg(not(unix))]
fn forward_to_running(_: &Path, _: &[String]) -> bool {
    false
}

/// Listens for the requests of later launches, and opens their windows.
#[cfg(unix)]
fn listen_for_requests(cx: &mut App) {
    use std::io::Read;
    let Some(sock) = instance_socket() else { return };
    let _ = std::fs::remove_file(&sock);
    let Ok(listener) = std::os::unix::net::UnixListener::bind(&sock) else { return };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600));
    }
    let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, Vec<String>)>();
    std::thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let mut buf = String::new();
            if conn.read_to_string(&mut buf).is_ok()
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&buf)
            {
                let cwd = PathBuf::from(v["cwd"].as_str().unwrap_or("."));
                let args = v["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let _ = tx.send((cwd, args));
            }
        }
    });
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            while let Ok((cwd, args)) = rx.try_recv() {
                if let Ok(req) = parse_args(&args, &cwd) {
                    cx.update(|cx| open_repo(req, cx));
                }
            }
        }
    })
    .detach();
}

#[cfg(not(unix))]
fn listen_for_requests(_: &mut App) {}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("check-diagram" | "--check-diagram")) {
        check_diagram(&args);
    }
    // A diffr:// URL, as the desktop entry passes it.
    if let Some(u) = args.first().and_then(|a| url_args(a)) {
        args = u;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let req = match parse_args(&args, &cwd) {
        Ok(mut r) => {
            if args.is_empty() && diffr_git::Repo::open(&r.dir).is_err()
                && let Some(last) = diffr_git::Store::open_default().recent().into_iter().next()
            {
                r.dir = PathBuf::from(last);
            }
            r
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(if e.starts_with("Usage") { 0 } else { 2 });
        }
    };
    // Another diffr runs: it opens the window.
    // With no arguments, the request is what was resolved: the repository
    // here, or the last one opened.
    let forwarded = if args.is_empty() { forward_to_running(&req.dir, &[]) } else { forward_to_running(&cwd, &args) };
    if forwarded {
        return;
    }
    application().with_assets(Assets).run(move |cx: &mut App| {
        let settings = config().get();
        let code = if settings.code_font_family.is_empty() {
            DEFAULT_CODE_FONT.to_string()
        } else {
            settings.code_font_family.clone()
        };
        cx.set_global(Fonts { ui: DEFAULT_UI_FONT.into(), code: code.into() });
        cx.set_global(Theme(Palette::light()));
        input::bind_keys(cx);
        diagrams::bind_keys(cx);
        actions::bind_keys(cx);
        cx.on_action(|_: &actions::Quit, cx| request_quit(cx));
        cx.on_action(|_: &OpenFolder, cx| open_folder(cx));
        cx.on_action(|_: &OpenConfig, cx| {
            let path = config().ensure();
            let _ = editor::open_editor(&config().get().editor_command, "/", &path, 0);
            let _ = cx;
        });
        cx.on_action(|_: &InstallCli, cx| report_install(cx));
        cx.on_action(|_: &About, cx| open_about(cx));
        // The review window handles Close itself and asks first; the others
        // (settings, about, diagrams, welcome) just close.
        // Deferred: the action is dispatched while that window is updating.
        cx.on_action(|_: &actions::CloseWindow, cx| {
            if let Some(w) = cx.active_window() {
                cx.defer(move |cx| {
                    w.update(cx, |_, window, _| window.remove_window()).ok();
                });
            }
        });
        cx.on_action(|a: &ActivateWindow, cx| {
            if let Some(w) = cx.windows().get(a.index).copied() {
                w.update(cx, |_, window, _| window.activate_window()).ok();
            }
        });
        cx.bind_keys([
            gpui::KeyBinding::new("secondary-o", OpenFolder, None),

        ]);
        set_menus(cx);
        cx.on_window_closed(|cx, _| {
            set_menus(cx);
            if cx.windows().is_empty() {
                // On Linux the clipboard is the process's own: a moment for a
                // clipboard manager to take what "Copy and Close" copied.
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    cx.update(|cx| {
                        if cx.windows().is_empty() {
                            cx.quit();
                        }
                    });
                })
                .detach();
            }
        })
        .detach();
        listen_for_requests(cx);
        notify::listen(cx);
        open_repo(req, cx);
        watch_settings(cx);
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diffview::{Mark, code_runs};
    use crate::state::{find_ranges, format_bytes, fuzzy_match, relative_time};

    /// Comments of a source no longer shown keep their lines, and several
    /// sources are named under one heading.
    #[test]
    fn comments_of_several_sources() {
        use crate::window::{SavedFile, join_comment_sections};
        let patch = b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-old()\n+new()\n";
        let file = diffr_git::diff::parse(patch).remove(0);
        let mut comments = diffr_git::Comments::new();
        let id = comments.add("a.rs", diffr_git::Side::New, 2, 2);
        comments.get_mut(id).unwrap().text = "Why new?".into();
        let saved = [SavedFile { file: file.into(), old_lines: None, new_lines: None }];
        let md = comments.markdown(&saved, "");
        assert!(md.contains("old()") && md.contains("new()"), "{md}");
        assert!(!md.contains("No patch context"), "{md}");

        let one = join_comment_sections(&[("repo".into(), md.clone())], "# Address");
        assert_eq!(one, format!("# Address\n\n{}", md.trim_end()));
        let two = join_comment_sections(
            &[("repo".into(), md.clone()), ("repo/abc1234".into(), md.clone()), ("empty".into(), String::new())],
            "# Address",
        );
        assert_eq!(two.matches("# Address").count(), 1, "{two}");
        assert!(two.contains("## repo\n\n") && two.contains("## repo/abc1234\n\n"), "{two}");
        assert!(!two.contains("## empty"), "{two}");
        assert_eq!(join_comment_sections(&[], "# Address"), "");
    }

    #[test]
    fn revisions_are_not_paths() {
        for a in ["HEAD", "HEAD~1", "@", "@{u}", "main^", "abc1234", "0123456789abcdef0123"] {
            assert!(commit_like(a), "{a}");
        }
        for a in ["main", "src", "docs/x", "abc"] {
            assert!(!commit_like(a), "{a}");
        }
    }

    #[test]
    fn diffr_urls() {
        assert_eq!(
            url_args("diffr://open?cwd=%2Ftmp%2Fa%20b&arg=HEAD~1&arg=--pr").unwrap(),
            vec!["--cwd", "/tmp/a b", "HEAD~1", "--pr"]
        );
        assert!(url_args("https://x").is_none());
    }

    #[test]
    fn arguments() {
        let dir = std::env::temp_dir();
        assert!(parse_args(&["--help".into()], &dir).unwrap_err().starts_with("Usage"));
        assert!(parse_args(&["--bogus".into()], &dir).is_err());
        let r = parse_args(&["--cwd=/".into()], &dir).unwrap();
        assert_eq!(r.dir, PathBuf::from("/"));
        assert_eq!(parse_args(&["--pr".into(), "acme/app#3".into()], &dir).unwrap().src, Source::Pull("acme/app#3".into()));
    }

    #[test]
    fn code_runs_expand_tabs_and_mark() {
        let p = crate::theme::Palette::light();
        let marks = [Mark { range: diffr_git::diff::Range { start: 1, end: 3 }, color: p.add_word }];
        let (text, runs) = code_runs("\tab c", &[], &marks, &p);
        assert_eq!(text.as_ref(), "    ab c");
        // The mark covers the tab's expansion and the "a".
        assert!(runs.iter().any(|(r, s)| s.background_color.is_some() && r.end > r.start));
        let (empty, _) = code_runs("", &[], &[], &p);
        assert_eq!(empty.as_ref(), " ");
    }

    #[test]
    fn fuzzy_find_and_times() {
        assert!(fuzzy_match("src/main.rs", "smr"));
        assert!(!fuzzy_match("src/main.rs", "zzz"));
        assert_eq!(find_ranges("Hello hello", "hello").len(), 2);
        assert_eq!(relative_time(1000, 1000 - 30), "just now");
        assert_eq!(relative_time(10_000, 10_000 - 7200), "2h ago");
        assert_eq!(format_bytes(1536), "1.5 KiB");
    }

    #[test]
    fn kinds_of_files() {
        let f = |p: &str| diffr_git::File { path: p.into(), old_path: p.into(), ..Default::default() };
        assert_eq!(crate::features::kind_of(&f("README.md")), 0);
        assert_eq!(crate::features::kind_of(&f("src/main.rs")), 1);
        assert_eq!(crate::features::kind_of(&f("main_test.go")), 2);
        assert_eq!(crate::features::kind_of(&f(".github/workflows/ci.yml")), 3);
        assert_eq!(crate::features::kind_of(&f("Cargo.toml")), 4);
        assert_eq!(crate::features::kind_of(&f("yarn.lock")), 5);
    }
}
