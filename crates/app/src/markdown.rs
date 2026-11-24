//! Markdown as the review's texts use it — summaries, notes, pull request
//! bodies: paragraphs, lists and code blocks, with inline code, emphasis,
//! strong text and links, drawn as GPUI styled text.

use std::ops::Range;

use gpui::{
    AnyElement, App, FontStyle, FontWeight, Hsla, InteractiveText, SharedString, StyledText, TextRun,
    UnderlineStyle, div, font, prelude::*, px,
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::theme::alpha;
use crate::ui::{self, pal};

/// A block of the text: a paragraph or a list item (inline text), or code.
enum Block {
    Text { text: String, runs: Vec<Span>, bullet: Option<String>, depth: usize, heading: bool },
    Code(String),
}

#[derive(Clone, Default)]
struct Style {
    strong: bool,
    emph: bool,
    code: bool,
    strike: bool,
    link: Option<String>,
}

struct Span {
    range: Range<usize>,
    style: Style,
}

fn parse(src: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Style::default();
    let mut stack: Vec<Style> = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut bullet: Option<String> = None;
    let mut heading = false;
    let mut code: Option<String> = None;
    let flush = |blocks: &mut Vec<Block>, text: &mut String, spans: &mut Vec<Span>, bullet: &mut Option<String>, depth: usize, heading: &mut bool| {
        if !text.trim().is_empty() {
            blocks.push(Block::Text {
                text: std::mem::take(text),
                runs: std::mem::take(spans),
                bullet: bullet.take(),
                depth,
                heading: std::mem::take(heading),
            });
        } else {
            text.clear();
            spans.clear();
        }
    };
    let push = |text: &mut String, spans: &mut Vec<Span>, s: &str, style: &Style| {
        let start = text.len();
        text.push_str(s);
        spans.push(Span { range: start..text.len(), style: style.clone() });
    };
    for ev in Parser::new_ext(src, Options::ENABLE_STRIKETHROUGH) {
        match ev {
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading);
                let _ = matches!(kind, CodeBlockKind::Fenced(_));
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(c) = code.take() {
                    blocks.push(Block::Code(c.trim_end().to_string()));
                }
            }
            Event::Text(t) if code.is_some() => code.as_mut().unwrap().push_str(&t),
            Event::Start(Tag::List(start)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading);
                bullet = Some(match lists.last_mut() {
                    Some(Some(n)) => {
                        let b = format!("{n}.");
                        *n += 1;
                        b
                    }
                    _ => "•".into(),
                });
            }
            Event::End(TagEnd::Item) | Event::End(TagEnd::Paragraph) => {
                flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading)
            }
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading);
                heading = true;
            }
            Event::End(TagEnd::Heading(_)) => flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading),
            Event::Start(Tag::Strong) => {
                stack.push(style.clone());
                style.strong = true;
            }
            Event::Start(Tag::Emphasis) => {
                stack.push(style.clone());
                style.emph = true;
            }
            Event::Start(Tag::Strikethrough) => {
                stack.push(style.clone());
                style.strike = true;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                stack.push(style.clone());
                style.link = Some(dest_url.to_string());
            }
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link) => {
                style = stack.pop().unwrap_or_default();
            }
            Event::Code(c) => {
                let mut s = style.clone();
                s.code = true;
                push(&mut text, &mut spans, &c, &s);
            }
            Event::Text(t) => push(&mut text, &mut spans, &t, &style),
            Event::SoftBreak => push(&mut text, &mut spans, " ", &style),
            Event::HardBreak => push(&mut text, &mut spans, "\n", &style),
            Event::Html(h) | Event::InlineHtml(h)
                // Comments of templates are left out.
                if !h.trim_start().starts_with("<!--") => {
                    push(&mut text, &mut spans, &h, &style);
                }
            _ => {}
        }
    }
    flush(&mut blocks, &mut text, &mut spans, &mut bullet, lists.len(), &mut heading);
    if let Some(c) = code {
        blocks.push(Block::Code(c));
    }
    blocks
}

/// Draws Markdown text in the inherited size, in a color.
pub fn markdown(id: impl Into<SharedString>, src: &str, color: Hsla, cx: &App) -> AnyElement {
    let p = pal(cx).clone();
    let ui_font = cx.global::<ui::Fonts>().ui.clone();
    let code_font = ui::code_font(cx);
    let id: SharedString = id.into();
    let mut col = div().flex().flex_col().gap(px(6.)).min_w_0();
    for (bi, b) in parse(src).into_iter().enumerate() {
        match b {
            Block::Code(c) => {
                col = col.child(
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .bg(p.surface)
                        .font_family(code_font.clone())
                        .text_size(px(12.))
                        .child(c),
                );
            }
            Block::Text { text, runs, bullet, depth, heading } => {
                let mut out_runs = Vec::with_capacity(runs.len());
                let mut links: Vec<(Range<usize>, String)> = Vec::new();
                for sp in &runs {
                    let st = &sp.style;
                    let mut f = font(if st.code { code_font.clone() } else { ui_font.clone() });
                    if st.strong || heading {
                        f.weight = FontWeight::SEMIBOLD;
                    }
                    if st.emph {
                        f.style = FontStyle::Italic;
                    }
                    let link_color = p.accent_foreground;
                    out_runs.push(TextRun {
                        len: sp.range.len(),
                        font: f,
                        color: if st.link.is_some() { link_color } else { color },
                        background_color: st.code.then_some(p.surface),
                        underline: st.link.as_ref().map(|_| UnderlineStyle {
                            color: Some(alpha(link_color, 0.5)),
                            thickness: px(1.),
                            wavy: false,
                        }),
                        strikethrough: st.strike.then(|| gpui::StrikethroughStyle { color: Some(color), thickness: px(1.) }),
                    });
                    if let Some(url) = &st.link {
                        links.push((sp.range.clone(), url.clone()));
                    }
                }
                let styled = StyledText::new(text).with_runs(out_runs);
                let body: AnyElement = if links.is_empty() {
                    styled.into_any_element()
                } else {
                    let ranges: Vec<Range<usize>> = links.iter().map(|l| l.0.clone()).collect();
                    let urls: Vec<String> = links.into_iter().map(|l| l.1).collect();
                    InteractiveText::new(SharedString::from(format!("{id}-{bi}")), styled)
                        .on_click(ranges, move |i, _, cx| cx.open_url(&urls[i]))
                        .into_any_element()
                };
                let mut row = div().flex().flex_row().gap(px(6.)).min_w_0().pl(px(depth.saturating_sub(1) as f32 * 14.));
                if let Some(b) = bullet {
                    row = row.child(div().flex_none().text_color(alpha(color, 0.7)).child(b));
                }
                col = col.child(row.child(div().flex_1().min_w_0().child(body)));
            }
        }
    }
    col.into_any_element()
}
