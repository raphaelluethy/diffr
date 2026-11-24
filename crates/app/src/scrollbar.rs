//! Overlay scrollbars for GPUI's lists: a thin thumb at the right edge that
//! shows while the pointer is over the list or the thumb is dragged, as the
//! scrollbars of MyGo (and macOS) do.

use gpui::{
    AnyElement, Context, ListState, MouseButton, MouseMoveEvent, ScrollHandle, SharedString, div,
    point, prelude::*, px,
};

use crate::theme::rgba;
use crate::ui::pal;
use crate::window::Diffr;

/// What a scrollbar scrolls.
#[derive(Clone)]
pub enum Scroll {
    List(ListState),
    Handle(ScrollHandle),
}

impl Scroll {
    /// The viewport's height, how far it scrolled, and how far it can.
    fn metrics(&self) -> (f32, f32, f32) {
        match self {
            Scroll::List(l) => (
                f32::from(l.viewport_bounds().size.height),
                -f32::from(l.scroll_px_offset_for_scrollbar().y),
                f32::from(l.max_offset_for_scrollbar().y),
            ),
            Scroll::Handle(h) => (
                f32::from(h.bounds().size.height),
                -f32::from(h.offset().y),
                f32::from(h.max_offset().y),
            ),
        }
    }

    fn set(&self, offset: f32) {
        match self {
            Scroll::List(l) => l.set_offset_from_scrollbar(point(px(0.), px(-offset))),
            Scroll::Handle(h) => h.set_offset(point(px(0.), px(-offset))),
        }
    }

    fn drag(&self, start: bool) {
        if let Scroll::List(l) = self {
            if start {
                l.scrollbar_drag_started()
            } else {
                l.scrollbar_drag_ended()
            }
        }
    }

    /// The thumb's top and height in a viewport, None when nothing scrolls.
    fn thumb(&self) -> Option<(f32, f32, f32)> {
        let (h, off, max) = self.metrics();
        if max <= 0.5 || h <= 0. {
            return None;
        }
        let thumb = (h * h / (h + max)).max(28.).min(h);
        let top = (h - thumb) * (off / max).clamp(0., 1.);
        Some((top, thumb, h))
    }
}

/// A scrollbar being dragged: what it scrolls, where the drag started and
/// the offset then.
pub struct Drag {
    scroll: Scroll,
    y0: f32,
    off0: f32,
}

impl Diffr {
    /// The scrollbar of a list, to lay over its right edge. `group` is the
    /// group name of the list's container, whose hover shows the thumb.
    pub fn scrollbar(&mut self, id: &'static str, scroll: Scroll, group: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx).clone();
        let Some((top, height, _)) = scroll.thumb() else {
            return div().into_any_element();
        };
        let dragging = self.scroll_drag.as_ref().is_some_and(|d| d.scroll_id == id);
        let color = if p.dark { rgba(255, 255, 255, 0.35) } else { rgba(0, 0, 0, 0.32) };
        let s2 = scroll.clone();
        let thumb = div()
            .id(SharedString::from(format!("{id}-thumb")))
            .absolute()
            .top(px(top))
            .right(px(2.))
            .w(px(7.))
            .h(px(height))
            .rounded(px(4.))
            .bg(color)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let (_, off, _) = s2.metrics();
                    s2.drag(true);
                    this.scroll_drag = Some(ScrollDrag {
                        scroll_id: id,
                        drag: Drag { scroll: s2.clone(), y0: f32::from(ev.position.y), off0: off },
                    });
                    cx.notify();
                }),
            );
        let track = div()
            .id(SharedString::from(id))
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(11.))
            .child(thumb);
        let track = if dragging { track } else { track.opacity(0.).group_hover(group, |s| s.opacity(1.)) };
        track.into_any_element()
    }

    /// Moves the scrollbar dragged, as the pointer moves.
    pub fn on_scroll_drag(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let Some(d) = self.scroll_drag.as_ref() else { return false };
        if !ev.dragging() {
            self.end_scroll_drag(cx);
            return true;
        }
        let Some((_, thumb, h)) = d.drag.scroll.thumb() else { return true };
        let (_, _, max) = d.drag.scroll.metrics();
        let dy = f32::from(ev.position.y) - d.drag.y0;
        let per_px = if h - thumb > 0. { max / (h - thumb) } else { 0. };
        d.drag.scroll.set((d.drag.off0 + dy * per_px).clamp(0., max));
        if d.scroll_id == "diff-sb" { self.scroll_selection_dirty = true; }
        cx.notify();
        true
    }

    pub fn end_scroll_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.scroll_drag.take() {
            d.drag.scroll.drag(false);
            cx.notify();
        }
    }
}

/// The scrollbar dragged, by its id.
pub struct ScrollDrag {
    pub scroll_id: &'static str,
    pub drag: Drag,
}
