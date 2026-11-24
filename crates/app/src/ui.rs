//! Small controls shared by the views: icon buttons, chips, the
//! segmented controls, buttons, avatars and tooltips.

use gpui::{
    AnyElement, AnyView, App, Div, ElementId, FontWeight, Global, Hsla, IntoElement, ParentElement,
    SharedString, Stateful, Styled, Svg, Window, div, prelude::*, px, svg,
};

use crate::theme::{Palette, rgba};

/// The window's palette, light or dark.
pub struct Theme(pub Palette);
impl Global for Theme {}

pub fn pal(cx: &App) -> &Palette {
    &cx.global::<Theme>().0
}

/// The fonts of the interface and of the code.
pub struct Fonts {
    pub ui: SharedString,
    pub code: SharedString,
}
impl Global for Fonts {}

pub fn code_font(cx: &App) -> SharedString {
    cx.global::<Fonts>().code.clone()
}

/// A Lucide icon from assets/icons.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// A tooltip: a short text in a bubble.
pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(if p.dark { crate::theme::hex(0x2c2c2e) } else { crate::theme::hex(0xffffff) })
            .border_1()
            .border_color(p.card_border)
            .shadow_md()
            .text_size(px(12.))
            .text_color(p.text)
            .font_family(cx.global::<Fonts>().ui.clone())
            .child(self.text.clone())
    }
}

pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip { text: text.clone() }).into()
}

/// A button showing an icon alone, as in a toolbar: 28×28.
pub fn icon_button(id: impl Into<ElementId>, name: &str, tip: &str, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    div()
        .id(id.into())
        .size(px(28.))
        .flex_none()
        .rounded(px(7.))
        .flex()
        .items_center()
        .justify_center()
        .text_color(p.text_muted)
        .hover(|s| s.bg(p.surface_hover))
        .active(|s| s.bg(p.surface_pressed))
        .child(icon(name, 16., p.text_muted))
        .tooltip(tooltip(tip.to_string()))
}

/// A label in a pill, as the branch in the toolbar.
pub fn chip(name: &str, label: impl Into<SharedString>, color: Hsla, mono: bool, cx: &App) -> Div {
    let p = pal(cx);
    let d = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.))
        .px(px(8.))
        .py(px(3.))
        .rounded(px(12.))
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .text_color(color)
        .flex_none()
        .max_w(px(240.))
        .child(icon(name, 12., color))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(label.into()),
        );
    if mono { d.font_family(code_font(cx)) } else { d }
}

/// The track of a segmented control.
pub fn segment_track() -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(8.))
        .flex_none()
}

/// A segment of a segmented control, showing an icon.
pub fn segment(id: impl Into<ElementId>, name: &str, tip: &str, on: bool, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    let color = if on { p.text } else { p.text_muted };
    segment_with(id, icon(name, 15., color).into_any_element(), tip, on, cx)
}

/// A segment of a segmented control, showing what it is given.
pub fn segment_with(id: impl Into<ElementId>, content: AnyElement, tip: &str, on: bool, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    let mut s = div()
        .id(id.into())
        .w(px(30.))
        .h(px(24.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .child(content)
        .tooltip(tooltip(tip.to_string()));
    if on {
        s = s.bg(p.header_bg).shadow(vec![gpui::BoxShadow {
            color: rgba(0, 0, 0, 0.12),
            offset: gpui::point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
            inset: false,
        }]);
    } else {
        s = s.hover(|s| s.bg(p.surface_hover));
    }
    s
}

/// A button with the theme's look.
pub fn button(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    div()
        .id(id.into())
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .px(px(14.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .text_color(p.text)
        .text_size(px(14.))
        .flex_none()
        .hover(|s| s.bg(p.surface_hover))
        .active(|s| s.bg(p.surface_pressed))
}

/// A button in the accent color, for the main action.
pub fn primary_button(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    let hover = p.accent_hover;
    div()
        .id(id.into())
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .px(px(14.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(p.accent)
        .border_1()
        .border_color(gpui::transparent_black())
        .text_color(p.accent_text)
        .text_size(px(14.))
        .flex_none()
        .hover(move |s| s.bg(hover))
}

/// The initials of the first and last words of a name.
pub fn initials(name: &str) -> String {
    let mut first = None;
    let mut last = None;
    let mut in_word = false;
    for c in name.chars() {
        let letter = c.is_alphanumeric();
        if letter && !in_word {
            if first.is_none() {
                first = Some(c.to_uppercase().next().unwrap_or(c));
            } else {
                last = Some(c.to_uppercase().next().unwrap_or(c));
            }
        }
        in_word = letter;
    }
    let mut s = String::new();
    if let Some(f) = first {
        s.push(f);
    }
    if let Some(l) = last {
        s.push(l);
    }
    s
}

fn hsl(h: f32, s: f32, l: f32) -> Hsla {
    gpui::hsla(h / 360., s, l, 1.)
}

/// A round avatar with the initials of a name, in the hue of its hash.
pub fn avatar(name: &str, size: f32) -> Div {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    let hue = (h % 360) as f32;
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(hsl(hue, 0.45, 0.55))
        .flex()
        .items_center()
        .justify_center()
        .text_color(gpui::white())
        .text_size(px(size * 0.4))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials(name))
}

/// A checkbox's box: 15×15, filled and checked when on.
pub fn check_box(on: bool, color: Hsla, cx: &App) -> Div {
    let p = pal(cx);
    let b = div().size(px(15.)).flex_none().rounded(px(4.)).flex().items_center().justify_center();
    if on {
        b.bg(color).child(icon("check", 11., p.code_bg))
    } else {
        b.border(px(1.5)).border_color(p.text_muted)
    }
}

/// A spinning ring, for work under way.
pub fn spinner(id: &'static str, size: f32, color: Hsla) -> AnyElement {
    use gpui::AnimationExt;
    icon("spinner", size, color)
        .with_animation(
            id,
            gpui::Animation::new(std::time::Duration::from_millis(800)).repeat(),
            |s, t| s.with_transformation(gpui::Transformation::rotate(gpui::radians(t * std::f32::consts::TAU))),
        )
        .into_any_element()
}

/// A flexible space.
pub fn spacer() -> Div {
    div().flex_1()
}

pub fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// A count with separators: 1,234.
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A count written briefly: 999, 1.2k, 15k, 1.2m.
pub fn compact(n: usize) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 10000 {
        format!("{:.1}", n as f64 / 1000.).trim_end_matches(".0").to_string() + "k"
    } else if n < 1_000_000 {
        format!("{}k", (n + 500) / 1000)
    } else {
        format!("{:.1}", n as f64 / 1e6).trim_end_matches(".0").to_string() + "m"
    }
}

pub fn lines_text(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what} line") } else { format!("{} {what} lines", thousands(n)) }
}
