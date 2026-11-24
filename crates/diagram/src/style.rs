//! Styling pre-pass.
//!
//! The upstream parser only understands `classDef` / `class` / `:::` in a
//! few diagram kinds (flowchart, state, requirement, block). For the others
//! (class, ER, sequence, and the `X:::cls` shorthand in state diagrams) we
//! extract the styling statements ourselves, blank those lines out (keeping
//! line numbers stable for error messages) and apply them to the parsed IR.
//!
//! On top of that, the status classes `added` / `changed` / `removed` get
//! theme-colored defaults in every diagram kind; a user `classDef` with the
//! same name overrides individual properties.

use std::collections::HashMap;
use std::sync::LazyLock;

use mermaid_rs_renderer::{DiagramKind, Graph, ir::NodeStyle};
use regex::Regex;

use crate::error;
use crate::theme::{DiagramTheme, hex};

/// Class names with built-in theme colors.
pub const STATUS_CLASSES: [&str; 3] = ["added", "changed", "removed"];

/// Header keyword family, detected before parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Header {
    /// Kinds whose parser handles styling natively.
    Native,
    /// Kinds that get the pre-pass.
    Class,
    State,
    Er,
    Sequence,
}

pub(crate) fn detect_header(src: &str) -> Header {
    let mut in_front_matter = false;
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        if i == 0 && t == "---" {
            in_front_matter = true;
            continue;
        }
        if in_front_matter {
            if t == "---" {
                in_front_matter = false;
            }
            continue;
        }
        if t.is_empty() || t.starts_with("%%") {
            continue;
        }
        let kw = t
            .split(|c: char| c.is_whitespace() || c == ';')
            .next()
            .unwrap_or("");
        return match kw {
            "classDiagram" | "classDiagram-v2" => Header::Class,
            "stateDiagram" | "stateDiagram-v2" => Header::State,
            "erDiagram" => Header::Er,
            "sequenceDiagram" => Header::Sequence,
            _ => Header::Native,
        };
    }
    Header::Native
}

/// A class assignment collected from the source.
#[derive(Debug, Clone)]
struct Assignment {
    id: String,
    class: String,
    line: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Extracted {
    class_defs: Vec<(String, NodeStyle)>,
    assignments: Vec<Assignment>,
    styles: Vec<(String, NodeStyle, usize)>,
}

static CLASS_DEF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*classDef\s+(\S+)\s+(.*?);?\s*$").unwrap());
pub(crate) static CLASS_STMT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*class\s+([^\s{\[<]+)\s+([A-Za-z_][\w-]*)\s*;?\s*$").unwrap());
static CSS_CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*cssClass\s+"([^"]+)"\s+([A-Za-z_][\w-]*)\s*;?\s*$"#).unwrap()
});
static STYLE_STMT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*style\s+(\S+)\s+(.*?);?\s*$").unwrap());
static INLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([A-Za-z0-9_.$~\-]+|"[^"]*"):::([A-Za-z_][\w-]*)"#).unwrap());

/// Removes styling statements the upstream parser would choke on or
/// ignore. Returns the rewritten source (same number of lines) and what was
/// extracted. `Native` kinds are returned unchanged.
pub(crate) fn extract(src: &str, header: Header) -> (String, Extracted) {
    let mut ex = Extracted::default();
    if header == Header::Native {
        return (src.to_string(), ex);
    }
    let mut out = Vec::new();
    for (idx, line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("%%") {
            out.push(line.to_string());
            continue;
        }
        if let Some(c) = CLASS_DEF.captures(line) {
            let style = parse_style(&c[2]);
            for name in c[1].split(',').filter(|n| !n.is_empty()) {
                ex.class_defs.push((name.to_string(), style.clone()));
            }
            out.push(String::new());
            continue;
        }
        if let Some(c) = CSS_CLASS
            .captures(line)
            .or_else(|| CLASS_STMT.captures(line))
        {
            for id in c[1].split(',').map(str::trim).filter(|n| !n.is_empty()) {
                ex.assignments.push(Assignment {
                    id: id.to_string(),
                    class: c[2].to_string(),
                    line: lineno,
                });
            }
            out.push(String::new());
            continue;
        }
        if let Some(c) = STYLE_STMT.captures(line) {
            ex.styles
                .push((c[1].to_string(), parse_style(&c[2]), lineno));
            out.push(String::new());
            continue;
        }
        if line.contains(":::") {
            let mut rewritten = String::with_capacity(line.len());
            let mut last = 0;
            for c in INLINE.captures_iter(line) {
                let m = c.get(0).unwrap();
                let id = c.get(1).unwrap();
                rewritten.push_str(&line[last..m.start()]);
                rewritten.push_str(id.as_str());
                last = m.end();
                ex.assignments.push(Assignment {
                    id: id.as_str().trim_matches('"').to_string(),
                    class: c[2].to_string(),
                    line: lineno,
                });
            }
            rewritten.push_str(&line[last..]);
            out.push(rewritten);
            continue;
        }
        out.push(line.to_string());
    }
    let mut joined = out.join("\n");
    if src.ends_with('\n') {
        joined.push('\n');
    }
    (joined, ex)
}

/// Parses `fill:#f00,stroke:#000,color:#fff,stroke-width:2px`.
pub(crate) fn parse_style(spec: &str) -> NodeStyle {
    let mut style = NodeStyle::default();
    for part in spec.split([',', ';']) {
        let Some((k, v)) = part.split_once(':') else {
            continue;
        };
        let v = v.trim().trim_end_matches("!important").trim().to_string();
        match k.trim() {
            "fill" => style.fill = Some(v),
            "stroke" => style.stroke = Some(v),
            "color" => style.text_color = Some(v),
            "stroke-width" => style.stroke_width = v.trim_end_matches("px").trim().parse().ok(),
            "stroke-dasharray" => style.stroke_dasharray = Some(v),
            _ => {}
        }
    }
    style
}

/// Applies extracted styling to the parsed graph. Errors when a class or
/// style statement targets an id that is not in the diagram, so an agent
/// gets told about typos.
pub(crate) fn apply(src: &str, graph: &mut Graph, ex: &Extracted) -> Result<(), String> {
    for (name, style) in &ex.class_defs {
        let entry = graph.class_defs.entry(name.clone()).or_default();
        overlay(entry, style);
    }
    for a in &ex.assignments {
        let id = resolve_id(graph, &a.id).ok_or_else(|| {
            error::at_text(
                src,
                a.line,
                &a.id,
                &format!(
                    "class '{}' is assigned to '{}', which is not a node in this diagram{}",
                    a.class,
                    a.id,
                    suggest(graph, &a.id)
                ),
            )
        })?;
        let classes = graph.node_classes.entry(id).or_default();
        if !classes.contains(&a.class) {
            classes.push(a.class.clone());
        }
    }
    for (target, style, line) in &ex.styles {
        let id = resolve_id(graph, target).ok_or_else(|| {
            error::at_text(
                src,
                *line,
                target,
                &format!(
                    "style targets '{target}', which is not a node in this diagram{}",
                    suggest(graph, target)
                ),
            )
        })?;
        overlay(graph.node_styles.entry(id).or_default(), style);
    }
    Ok(())
}

fn resolve_id(graph: &Graph, id: &str) -> Option<String> {
    if graph.nodes.contains_key(id) {
        return Some(id.to_string());
    }
    // `Foo~T~` (class generics) is stored as `Foo`.
    let base = id.split('~').next().unwrap_or(id);
    if graph.nodes.contains_key(base) {
        return Some(base.to_string());
    }
    // Match by display label as a last resort (ER/sequence aliases).
    graph
        .nodes
        .values()
        .find(|n| n.label == id)
        .map(|n| n.id.clone())
}

fn suggest(graph: &Graph, id: &str) -> String {
    let lower = id.to_lowercase();
    let mut close: Vec<&str> = graph
        .nodes
        .keys()
        .filter(|k| !k.starts_with("__"))
        .filter(|k| {
            let kl = k.to_lowercase();
            kl.contains(&lower) || lower.contains(&kl)
        })
        .map(String::as_str)
        .collect();
    close.truncate(3);
    if close.is_empty() {
        let mut all: Vec<&str> = graph
            .nodes
            .keys()
            .filter(|k| !k.starts_with("__"))
            .map(String::as_str)
            .collect();
        all.truncate(8);
        format!(" (nodes: {})", all.join(", "))
    } else {
        format!(" (did you mean: {}?)", close.join(", "))
    }
}

/// Copies every property set in `src` onto `dst`.
fn overlay(dst: &mut NodeStyle, src: &NodeStyle) {
    if src.fill.is_some() {
        dst.fill.clone_from(&src.fill);
    }
    if src.stroke.is_some() {
        dst.stroke.clone_from(&src.stroke);
    }
    if src.text_color.is_some() {
        dst.text_color.clone_from(&src.text_color);
    }
    if src.stroke_width.is_some() {
        dst.stroke_width = src.stroke_width;
    }
    if src.stroke_dasharray.is_some() {
        dst.stroke_dasharray.clone_from(&src.stroke_dasharray);
    }
    if src.line_color.is_some() {
        dst.line_color.clone_from(&src.line_color);
    }
}

/// Installs the themed `added` / `changed` / `removed` classes; properties a
/// user `classDef` already set win.
pub(crate) fn install_status_classes(graph: &mut Graph, theme: &DiagramTheme) {
    let defaults: HashMap<&str, u32> = HashMap::from([
        ("added", theme.added),
        ("changed", theme.changed),
        ("removed", theme.removed),
    ]);
    for name in STATUS_CLASSES {
        let color = defaults[name];
        let mut style = NodeStyle {
            fill: Some(hex(theme.tint(color))),
            stroke: Some(hex(color)),
            text_color: Some(hex(theme.text)),
            stroke_width: Some(1.5),
            stroke_dasharray: None,
            line_color: None,
        };
        if name == "removed" {
            style.stroke_dasharray = Some("4 3".to_string());
        }
        if let Some(user) = graph.class_defs.get(name) {
            overlay(&mut style, user);
        }
        graph.class_defs.insert(name.to_string(), style);
    }
}

/// Number of user-visible nodes (excludes synthetic start/end pseudo states).
pub(crate) fn visible_node_count(graph: &Graph) -> usize {
    graph.nodes.keys().filter(|k| !k.starts_with("__")).count()
}

pub(crate) fn kind_name(kind: DiagramKind) -> &'static str {
    match kind {
        DiagramKind::Flowchart => "flowchart",
        DiagramKind::Class => "class diagram",
        DiagramKind::State => "state diagram",
        DiagramKind::Sequence => "sequence diagram",
        DiagramKind::Er => "ER diagram",
        DiagramKind::Pie => "pie chart",
        DiagramKind::Mindmap => "mindmap",
        DiagramKind::Journey => "user journey",
        DiagramKind::Timeline => "timeline",
        DiagramKind::Gantt => "gantt chart",
        DiagramKind::Requirement => "requirement diagram",
        DiagramKind::GitGraph => "git graph",
        DiagramKind::C4 => "C4 diagram",
        DiagramKind::Sankey => "sankey diagram",
        DiagramKind::Quadrant => "quadrant chart",
        DiagramKind::ZenUML => "ZenUML diagram",
        DiagramKind::Block => "block diagram",
        DiagramKind::Packet => "packet diagram",
        DiagramKind::Kanban => "kanban board",
        DiagramKind::Architecture => "architecture diagram",
        DiagramKind::Radar => "radar chart",
        DiagramKind::Treemap => "treemap",
        DiagramKind::XYChart => "xy chart",
    }
}
