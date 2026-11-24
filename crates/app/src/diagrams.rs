//! The Diagrams lab: the agent's Mermaid diagrams of the change, rendered
//! natively (Mermaid → SVG by diffr-diagram, drawn by GPUI) in the
//! background, shown in the AI review card and the groups' headers, and
//! each in a zoomable window of its own.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use diffr_agent::Diagram;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, FontWeight, MouseButton, Render, ScrollWheelEvent,
    SharedString, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, div, img, prelude::*,
    px, size,
};

use crate::theme::{Palette, alpha};
use crate::ui::{self, icon, pal, tooltip};
use crate::window::Diffr;

/// A diagram drawn: its picture and its size, or why it could not be.
#[derive(Clone)]
pub enum Drawn {
    Image { image: Arc<gpui::Image>, width: f32, height: f32, svg: Arc<String> },
    Error(String),
}

type Key = (u64, bool);
static CACHE: OnceLock<Mutex<HashMap<Key, Drawn>>> = OnceLock::new();
static PENDING: OnceLock<Mutex<HashSet<Key>>> = OnceLock::new();

fn key(mermaid: &str, dark: bool) -> Key {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mermaid.hash(&mut h);
    (h.finish(), dark)
}

fn theme_for(p: &Palette) -> diffr_diagram::DiagramTheme {
    let mut t = if p.dark { diffr_diagram::DiagramTheme::dark() } else { diffr_diagram::DiagramTheme::light() };
    let rgb = |c: gpui::Hsla| {
        let c: gpui::Rgba = c.into();
        let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
        (channel(c.r) << 16) | (channel(c.g) << 8) | channel(c.b)
    };
    t.background = rgb(p.code_bg);
    t.surface = rgb(p.surface);
    t.border = rgb(p.border);
    t.text = rgb(p.text);
    t.muted = rgb(p.text_muted);
    t.accent = rgb(p.accent);
    t.added = rgb(p.add_bar);
    t.changed = rgb(p.modified);
    t.removed = rgb(p.del_bar);
    t.font_family = diffr_diagram::font_family_for(crate::DEFAULT_UI_FONT);
    t
}

fn render(mermaid: &str, dark: bool) -> Drawn {
    let p = if dark { Palette::dark() } else { Palette::light() };
    match diffr_diagram::render_svg(mermaid, &theme_for(&p)) {
        Ok(r) => Drawn::Image {
            image: Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Svg, r.svg.clone().into_bytes())),
            width: r.width,
            height: r.height,
            svg: Arc::new(r.svg),
        },
        Err(e) => Drawn::Error(e),
    }
}

/// The diagram drawn, if it is: else its drawing starts in the background,
/// and `done` runs on the main thread once it is.
fn drawn_or_start(mermaid: &str, dark: bool, cx: &mut App, done: impl FnOnce(&mut App) + 'static) -> Option<Drawn> {
    let k = key(mermaid, dark);
    if let Some(d) = CACHE.get_or_init(Default::default).lock().unwrap().get(&k) {
        return Some(d.clone());
    }
    if PENDING.get_or_init(Default::default).lock().unwrap().insert(k) {
        let src = mermaid.to_string();
        cx.spawn(async move |cx| {
            let d = cx.background_spawn(async move { render(&src, dark) }).await;
            CACHE.get_or_init(Default::default).lock().unwrap().insert(k, d);
            PENDING.get_or_init(Default::default).lock().unwrap().remove(&k);
            cx.update(done);
        })
        .detach();
    }
    None
}

/// Whether the diagrams are folded: one choice for all, kept.
fn folded_path() -> Option<std::path::PathBuf> {
    diffr_git::Store::default_path().and_then(|p| p.parent().map(|d| d.join("diagrams-folded")))
}

pub fn diagrams_folded() -> bool {
    folded_path().is_some_and(|p| p.exists())
}

pub fn set_diagrams_folded(on: bool) {
    if let Some(p) = folded_path() {
        if on {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, b"1");
        } else {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The windows open, by the diagram's source.
static WINDOWS: OnceLock<Mutex<HashMap<u64, WindowHandle<DiagramWindow>>>> = OnceLock::new();

fn window_key(mermaid: &str) -> u64 {
    key(mermaid, false).0
}

pub fn window_open(mermaid: &str, cx: &mut App) -> bool {
    let k = window_key(mermaid);
    let h = WINDOWS.get_or_init(Default::default).lock().unwrap().get(&k).copied();
    match h {
        Some(h) => h.update(cx, |_, _, _| ()).is_ok(),
        None => false,
    }
}

/// Opens a diagram's window, or closes it when it is open.
pub fn toggle_window(title: String, d: Diagram, cx: &mut App) {
    let k = window_key(&d.mermaid);
    let open = WINDOWS.get_or_init(Default::default).lock().unwrap().remove(&k);
    if let Some(h) = open
        && h.update(cx, |_, window, _| window.remove_window()).is_ok()
    {
        return;
    }
    // Sized to the picture: 480×320 at least.
    let dark = cx.global::<ui::Theme>().0.dark;
    let (w, h) = match CACHE.get_or_init(Default::default).lock().unwrap().get(&key(&d.mermaid, dark)) {
        Some(Drawn::Image { width, height, .. }) => (width.max(480.) + 48., height.max(320.) + 96.),
        _ => (900., 700.),
    };
    let bounds = Bounds::centered(None, size(px(w.min(1600.)), px(h.min(1000.))), cx);
    if let Ok(h) = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(SharedString::from(title)), ..Default::default() }),
            app_id: Some("dev.diffr".into()),
            ..Default::default()
        },
        |window, cx| {
            crate::apply_theme(window, cx);
            cx.new(|cx| DiagramWindow { diagram: d, zoom: 1., focus: cx.focus_handle(), scaled: HashMap::new() })
        },
    ) {
        WINDOWS.get_or_init(Default::default).lock().unwrap().insert(k, h);
    }
}

impl Diffr {
    /// A diagram as a card: its bar (fold, open or close its window), its
    /// title and caption, and the picture fit into a width.
    pub fn diagram_card(&mut self, id: &str, title: &str, d: &Diagram, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let folded = diagrams_folded();
        let open = window_open(&d.mermaid, cx);
        let max_width = (self.list_width - 90.).max(320.);
        let win_title = format!("{} — {}", if d.title.is_empty() { title } else { &d.title }, crate::window::window_title(&self.root, &self.source));
        let (d1, d2, d3) = (d.clone(), d.clone(), d.clone());
        let (t1, t2) = (win_title.clone(), win_title);
        let bar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-fold")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(icon("chevron_down", 12., p.text_muted).with_transformation(gpui::Transformation::rotate(gpui::radians(
                        if folded { -std::f32::consts::FRAC_PI_2 } else { 0. },
                    ))))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_muted).child("Diagram"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        set_diagrams_folded(!folded);
                        this.rows_dirty = true;
                        cx.notify();
                    })),
            )
            .child(ui::spacer())
            .child(
                div()
                    .id(SharedString::from(format!("{id}-win")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .text_color(p.accent_foreground)
                    .cursor_pointer()
                    .hover(|s| s.bg(alpha(p.accent, 0.08)))
                    .child(icon("open", 12., p.accent_foreground))
                    .child(if open { "Close Window" } else { "Open in Window" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle_window(t1.clone(), d1.clone(), cx);
                        this.rows_dirty = true;
                        cx.notify();
                    })),
            );
        let mut col = div()
            .id(SharedString::from(format!("{id}-card")))
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(p.code_bg)
            .border_1()
            .border_color(p.card_border)
            .child(bar)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    this.open_context_menu(crate::menus::ContextMenu::Diagram(d2.clone(), t2.clone()), ev.position, cx);
                }),
            );
        if folded {
            return col.into_any_element();
        }
        if !d.title.is_empty() {
            col = col.child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(d.title.clone()));
        }
        if !d.caption.is_empty() {
            col = col.child(div().text_size(px(12.)).child(crate::markdown::markdown(format!("{id}-cap"), &d.caption, p.text_muted, cx)));
        }
        let weak = cx.weak_entity();
        let drawn = drawn_or_start(&d.mermaid, p.dark, cx, move |cx| {
            weak.update(cx, |this, cx| {
                this.rows_dirty = true;
                cx.notify();
            })
            .ok();
        });
        let picture: AnyElement = match drawn {
            None => div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .py(px(10.))
                .child(ui::spinner("diagram-drawing", 14., p.text_muted))
                .child(div().text_size(px(12.)).text_color(p.text_muted).child("Drawing the diagram…"))
                .into_any_element(),
            Some(Drawn::Image { image, width, height, .. }) => {
                let scale = (max_width / width.max(1.)).min(440. / height.max(1.)).min(1.);
                let title = format!("Diagram — {}", crate::window::window_title(&self.root, &self.source));
                div()
                    .id(SharedString::from(format!("{id}-pic")))
                    .flex()
                    .justify_center()
                    .py(px(8.))
                    .cursor_pointer()
                    .child(img(image).w(px(width * scale)).h(px(height * scale)))
                    .tooltip(tooltip(if open { "Close its window" } else { "Open in a window, to zoom" }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle_window(title.clone(), d3.clone(), cx);
                        this.rows_dirty = true;
                        cx.notify();
                    }))
                    .into_any_element()
            }
            Some(Drawn::Error(e)) => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(alpha(p.danger, 0.06))
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.danger_foreground).child("The agent's diagram could not be drawn"))
                .child(div().text_size(px(12.)).text_color(p.danger_foreground).child(e.lines().take(6).collect::<Vec<_>>().join("\n")))
                .child(div().font_family(ui::code_font(cx)).text_size(px(11.)).text_color(p.text_muted).child(d.mermaid.clone()))
                .into_any_element(),
        };
        col.child(picture).into_any_element()
    }

    /// The diagrams of the review, in its card.
    pub fn diagrams_section(&mut self, diagrams: &[Diagram], cx: &mut Context<Self>) -> AnyElement {
        let mut col = div().flex().flex_col().gap(px(8.));
        for (i, d) in diagrams.iter().enumerate() {
            col = col.child(self.diagram_card(&format!("dg{i}"), "The change", d, cx));
        }
        col.into_any_element()
    }
}

/// A diagram in a window of its own: zoomable with Ctrl+scroll and
/// Ctrl+= / Ctrl+- / Ctrl+0, scrollable.
pub struct DiagramWindow {
    diagram: Diagram,
    zoom: f32,
    focus: gpui::FocusHandle,
    /// The picture drawn at each scale, by the scale's bits and the theme.
    scaled: HashMap<(u32, bool), Arc<gpui::Image>>,
}

/// An SVG drawn larger: its width and height times a scale, the view box
/// as it is, so that it rasterizes at the size it shows.
fn scale_svg(svg: &str, s: f32) -> String {
    let Some(start) = svg.find("<svg") else { return svg.to_string() };
    let Some(end) = svg[start..].find('>').map(|e| start + e) else { return svg.to_string() };
    let mut head = svg[start..end].to_string();
    for attr in ["width", "height"] {
        let pat = format!(" {attr}=\"");
        if let Some(i) = head.find(&pat) {
            let v0 = i + pat.len();
            if let Some(len) = head[v0..].find('"') {
                let v: f32 = head[v0..v0 + len].trim_end_matches("px").parse().unwrap_or(0.);
                if v > 0. {
                    head.replace_range(v0..v0 + len, &format!("{}", v * s));
                }
            }
        }
    }
    format!("{}{}{}", &svg[..start], head, &svg[end..])
}

gpui::actions!(diagram_window, [ZoomIn, ZoomOut, ZoomReset]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("DiagramWindow");
    cx.bind_keys([
        gpui::KeyBinding::new("secondary-=", ZoomIn, ctx),
        gpui::KeyBinding::new("secondary-+", ZoomIn, ctx),
        gpui::KeyBinding::new("secondary--", ZoomOut, ctx),
        gpui::KeyBinding::new("secondary-0", ZoomReset, ctx),
    ]);
}

impl Render for DiagramWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::apply_theme(window, cx);
        let p = pal(cx).clone();
        if !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
        }
        let weak = cx.weak_entity();
        let drawn = drawn_or_start(&self.diagram.mermaid, p.dark, cx, move |cx| {
            weak.update(cx, |_, cx| cx.notify()).ok();
        });
        let ww = f32::from(window.bounds().size.width) - 48.;
        let wh = f32::from(window.bounds().size.height) - 96.;
        let body: AnyElement = match drawn {
            None => ui::spinner("diagram-window-spin", 18., p.text_muted),
            Some(Drawn::Error(e)) => div().text_color(p.danger_foreground).child(e).into_any_element(),
            Some(Drawn::Image { width, height, svg, .. }) => {
                let fit = (ww / width.max(1.)).min(wh / height.max(1.)).min(2.);
                let s = fit * self.zoom;
                // Drawn again at the size shown, a quarter step at a time, so
                // that it stays sharp as it zooms.
                // In device pixels: sharp on screens of any density.
                let step = ((s * window.scale_factor() / 2. * 4.).round() / 4.).max(0.25);
                let k = (step.to_bits(), p.dark);
                let image = self.scaled.get(&k).cloned().unwrap_or_else(|| {
                    let img = Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Svg, scale_svg(&svg, step).into_bytes()));
                    // A few scales are kept: those zoomed through last.
                    if self.scaled.len() >= 8 {
                        self.scaled.clear();
                    }
                    self.scaled.insert(k, img.clone());
                    img
                });
                img(image).w(px(width * s)).h(px(height * s)).flex_none().into_any_element()
            }
        };
        div()
            .id("diagram-window")
            .key_context("DiagramWindow")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.zoom = (this.zoom * 1.25).min(8.);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.zoom = (this.zoom / 1.25).max(0.1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                this.zoom = 1.;
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                if ev.modifiers.secondary() {
                    let dy = f32::from(ev.delta.pixel_delta(px(20.)).y);
                    this.zoom = (this.zoom * (1. + dy / 400.)).clamp(0.1, 8.);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(p.code_bg)
            .text_color(p.text)
            .font_family(cx.global::<ui::Fonts>().ui.clone())
            .text_size(px(14.))
            .line_height(gpui::relative(1.36))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(24.))
                    .pt(px(16.))
                    .pb(px(8.))
                    .children((!self.diagram.title.is_empty()).then(|| {
                        div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(self.diagram.title.clone())
                    }))
                    .children((!self.diagram.caption.is_empty()).then(|| {
                        div().text_size(px(12.)).text_color(p.text_muted).child(self.diagram.caption.clone())
                    }))
                    .child(div().text_size(px(11.)).text_color(p.text_muted).child(format!(
                        "{:.0}% · Ctrl+scroll or Ctrl+= / Ctrl+- to zoom, Ctrl+0 to fit",
                        self.zoom * 100.
                    ))),
            )
            .child(
                div()
                    .id("diagram-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .p(px(24.))
                    // Centered while it fits; scrolled from its left edge when not.
                    .child(div().flex().flex_row().child(div().mx_auto().flex_none().child(body))),
            )
    }
}

/// The Mermaid source and the picture of a diagram, to copy.
pub fn copy_image(d: &Diagram, cx: &mut App) {
    let p = cx.global::<ui::Theme>().0.clone();
    match diffr_diagram::render_png(&d.mermaid, &theme_for(&p), 2.) {
        Ok(png) => cx.write_to_clipboard(gpui::ClipboardItem::new_image(&gpui::Image::from_bytes(gpui::ImageFormat::Png, png))),
        Err(_) => {
            // Else the picture as drawn.
            if let Some(Drawn::Image { image, .. }) = CACHE.get_or_init(Default::default).lock().unwrap().get(&key(&d.mermaid, p.dark)).cloned() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
            }
        }
    }
}
