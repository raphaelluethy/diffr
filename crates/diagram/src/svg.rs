//! Post-processing of the renderer's SVG so it matches the app and stays
//! within what usvg/resvg supports.

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// Corner radius for node boxes.
const NODE_RADIUS: f32 = 6.0;

static FONT_FAMILY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"font-family="[^"]*""#).unwrap());
static RECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<rect\b[^>]*>").unwrap());
static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b([a-zA-Z-]+)="([^"]*)""#).unwrap());
static ROOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<svg\b[^>]*>").unwrap());

pub(crate) fn post_process(svg: &str, font_family: &str, accent: &str) -> String {
    let family = css_font_list(font_family);
    // The ER key badge (PK/FK) uses a hard-coded blue; use the app accent.
    let svg = svg.replace("fill=\"#1D4ED8\"", &format!("fill=\"{accent}\""));
    let svg = FONT_FAMILY.replace_all(&svg, |_: &Captures| format!("font-family=\"{family}\""));
    let svg = RECT.replace_all(&svg, |c: &Captures| round_rect(&c[0]));
    svg.into_owned()
}

/// Normalises a font-family list for an XML attribute: names are quoted with
/// single quotes (the attribute itself uses double quotes); generic and
/// keyword families stay bare.
fn css_font_list(list: &str) -> String {
    list.split(',')
        .map(|f| f.trim().trim_matches(['"', '\'']).trim())
        .filter(|f| !f.is_empty())
        .map(|f| {
            let bare = matches!(
                f,
                "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui"
            ) || (f.starts_with('-') && !f.contains(' '));
            if bare {
                f.to_string()
            } else {
                format!("'{}'", f.replace('\'', ""))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Gives outlined node boxes the app's corner radius. Edge-label pills,
/// unstroked fills and thin rows (ER/class compartments) keep theirs.
fn round_rect(tag: &str) -> String {
    let mut width = 0.0f32;
    let mut height = 0.0f32;
    let mut rx: Option<f32> = None;
    let mut stroked = false;
    let mut edge_label = false;
    for a in ATTR.captures_iter(tag) {
        match &a[1] {
            "width" => width = a[2].parse().unwrap_or(0.0),
            "height" => height = a[2].parse().unwrap_or(0.0),
            "rx" => rx = a[2].parse().ok(),
            "stroke" => stroked = &a[2] != "none",
            "data-edge-id" | "data-label-kind" => edge_label = true,
            _ => {}
        }
    }
    let Some(rx) = rx else { return tag.to_string() };
    if edge_label || !stroked || rx >= NODE_RADIUS || width < 24.0 || height < 24.0 {
        return tag.to_string();
    }
    let radius = NODE_RADIUS.min(height / 4.0).min(width / 4.0);
    let mut out = ATTR
        .replace_all(tag, |c: &Captures| match &c[1] {
            "rx" | "ry" => format!("{}=\"{radius}\"", &c[1]),
            _ => c[0].to_string(),
        })
        .into_owned();
    if !tag.contains(" ry=") {
        out = out.replacen(" rx=", &format!(" ry=\"{radius}\" rx="), 1);
    }
    out
}

/// How the renderer draws a status class (`added`, `changed`,
/// `removed`), and the labels of the nodes that have it.
pub(crate) struct StatusMark {
    pub fill: String,
    pub stroke: String,
    pub stroke_width: f32,
    pub dasharray: Option<String>,
    /// The class strikes its nodes' text through (`removed`).
    pub strike: bool,
    pub labels: Vec<String>,
}

static SHAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(?:rect|polygon|circle|ellipse)\b[^>]*>").unwrap());
static TEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<text\b([^>]*)>(.*?)</text>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());

/// An axis-aligned box.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Bounds {
    fn contains(&self, x: f32, y: f32) -> bool {
        const SLACK: f32 = 0.5;
        x >= self.x0 - SLACK && x <= self.x1 + SLACK && y >= self.y0 - SLACK && y <= self.y1 + SLACK
    }
}

fn attrs(tag: &str) -> Vec<(String, String)> {
    ATTR.captures_iter(tag)
        .map(|a| (a[1].to_string(), a[2].to_string()))
        .collect()
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn num(attrs: &[(String, String)], name: &str) -> f32 {
    attr(attrs, name)
        .and_then(|v| v.trim_end_matches("px").parse().ok())
        .unwrap_or(0.0)
}

/// The box of a `<rect>`, `<polygon>`, `<circle>` or `<ellipse>` tag.
fn bounds(tag: &str, a: &[(String, String)]) -> Option<Bounds> {
    if tag.starts_with("<rect") {
        let (x, y) = (num(a, "x"), num(a, "y"));
        return Some(Bounds {
            x0: x,
            y0: y,
            x1: x + num(a, "width"),
            y1: y + num(a, "height"),
        });
    }
    if tag.starts_with("<circle") || tag.starts_with("<ellipse") {
        let (cx, cy) = (num(a, "cx"), num(a, "cy"));
        let (rx, ry) = if tag.starts_with("<circle") {
            (num(a, "r"), num(a, "r"))
        } else {
            (num(a, "rx"), num(a, "ry"))
        };
        return Some(Bounds {
            x0: cx - rx,
            y0: cy - ry,
            x1: cx + rx,
            y1: cy + ry,
        });
    }
    let coords: Vec<f32> = attr(a, "points")?
        .split([',', ' '])
        .filter_map(|n| n.parse().ok())
        .collect();
    let xs = coords.iter().step_by(2);
    let ys = coords.iter().skip(1).step_by(2);
    Some(Bounds {
        x0: xs.clone().copied().fold(f32::INFINITY, f32::min),
        x1: xs.copied().fold(f32::NEG_INFINITY, f32::max),
        y0: ys.clone().copied().fold(f32::INFINITY, f32::min),
        y1: ys.copied().fold(f32::NEG_INFINITY, f32::max),
    })
    .filter(|b| b.x0 <= b.x1 && b.y0 <= b.y1)
}

/// Sets an attribute of a tag, adding it before its end when missing.
fn set_attr(tag: &str, name: &str, value: &str) -> String {
    let re = Regex::new(&format!(r#"\s{name}="[^"]*""#)).expect("an attribute's name");
    if re.is_match(tag) {
        return re
            .replace(tag, regex::NoExpand(&format!(" {name}=\"{value}\"")))
            .into_owned();
    }
    let end = if tag.ends_with("/>") {
        tag.len() - 2
    } else {
        tag.len() - 1
    };
    format!("{} {name}=\"{value}\"{}", &tag[..end], &tag[end..])
}

/// A `<text>`'s content as shown: its tspans' text, unescaped, with
/// whitespace collapsed.
fn text_content(inner: &str) -> String {
    let t = TAG.replace_all(inner, " ");
    let t = t
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Draws the status classes the renderer leaves out, and strikes through
/// the text of the nodes of those that strike (`removed`):
///
/// - in sequence diagrams (`label_boxes`), whose renderer ignores classes,
///   the boxes around a status node's label get the class' colors;
/// - shapes drawn with a striking class' fill and stroke get its dashes,
///   which the ER renderer leaves out, and the text within them a
///   `text-decoration="line-through"`, which usvg draws.
pub(crate) fn mark_statuses(svg: &str, marks: &[StatusMark], label_boxes: bool) -> String {
    let texts: Vec<(f32, f32, String)> = TEXT
        .captures_iter(svg)
        .map(|c| {
            let a = attrs(&c[1]);
            (num(&a, "x"), num(&a, "y"), text_content(&c[2]))
        })
        .collect();
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let mut struck: Vec<Bounds> = Vec::new();
    let svg = SHAPE.replace_all(svg, |c: &Captures| {
        let tag = &c[0];
        let a = attrs(tag);
        if attr(&a, "data-edge-id").is_some() || attr(&a, "data-label-kind").is_some() {
            return tag.to_string();
        }
        let Some(b) = bounds(tag, &a) else {
            return tag.to_string();
        };
        let fill = attr(&a, "fill").unwrap_or("");
        let stroke = attr(&a, "stroke").unwrap_or("none");
        for m in marks {
            let styled = same(fill, &m.fill) && same(stroke, &m.stroke);
            let labeled = label_boxes
                && stroke != "none"
                && !m.labels.is_empty()
                && texts
                    .iter()
                    .any(|(x, y, t)| b.contains(*x, *y) && m.labels.iter().any(|l| l == t));
            if !styled && !labeled {
                continue;
            }
            let mut out = tag.to_string();
            if labeled {
                out = set_attr(&out, "fill", &m.fill);
                out = set_attr(&out, "stroke", &m.stroke);
                out = set_attr(&out, "stroke-width", &m.stroke_width.to_string());
            }
            if let Some(d) = &m.dasharray
                && attr(&a, "stroke-dasharray").is_none()
            {
                out = set_attr(&out, "stroke-dasharray", d);
            }
            if m.strike {
                struck.push(b);
            }
            return out;
        }
        tag.to_string()
    });
    if struck.is_empty() {
        return svg.into_owned();
    }
    TEXT.replace_all(&svg, |c: &Captures| {
        let a = attrs(&c[1]);
        let (x, y) = (num(&a, "x"), num(&a, "y"));
        if attr(&a, "text-decoration").is_some() || !struck.iter().any(|b| b.contains(x, y)) {
            return c[0].to_string();
        }
        c[0].replacen("<text", "<text text-decoration=\"line-through\"", 1)
    })
    .into_owned()
}

/// Width/height of the root `<svg>` element.
pub(crate) fn dimensions(svg: &str) -> Option<(f32, f32)> {
    let root = ROOT.find(svg)?.as_str();
    let mut w = None;
    let mut h = None;
    for a in ATTR.captures_iter(root) {
        match &a[1] {
            "width" => w = a[2].trim_end_matches("px").parse().ok(),
            "height" => h = a[2].trim_end_matches("px").parse().ok(),
            _ => {}
        }
    }
    Some((w?, h?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_list_quotes_names() {
        assert_eq!(
            css_font_list("\"Noto Sans\", -apple-system, .SystemUIFont, sans-serif"),
            "'Noto Sans', -apple-system, '.SystemUIFont', sans-serif"
        );
    }

    #[test]
    fn rounds_node_rects_only() {
        let node = r##"<rect x="0" y="0" width="100" height="40" rx="3" ry="3" fill="#fff" stroke="#000"/>"##;
        assert!(round_rect(node).contains(r#"rx="6" ry="6""#));
        let label = r##"<rect data-edge-id="e" x="0" y="0" width="100" height="40" rx="2" ry="2" fill="#fff" stroke="#000"/>"##;
        assert_eq!(round_rect(label), label);
        let plain = r##"<rect x="0" y="0" width="100" height="40" fill="#fff" stroke="#000"/>"##;
        assert_eq!(round_rect(plain), plain);
    }

    #[test]
    fn sets_attributes() {
        assert_eq!(
            set_attr(r#"<rect fill="a"/>"#, "fill", "b"),
            r#"<rect fill="b"/>"#
        );
        assert_eq!(
            set_attr(r#"<rect fill="a">"#, "stroke", "c"),
            r#"<rect fill="a" stroke="c">"#
        );
        assert_eq!(
            text_content("<tspan>a &amp;</tspan>\n <tspan>b</tspan>"),
            "a & b"
        );
    }
}
