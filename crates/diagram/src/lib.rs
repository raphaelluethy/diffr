//! Mermaid diagrams for diffr.
//!
//! An agent reviewing a change can emit Mermaid source explaining the
//! feature. This crate
//!
//! * [`validate`]s it with precise, line/column error messages (so the agent
//!   can fix its syntax; see [`check_command_output`] for the CLI text),
//! * [`render_svg`]s it to a plain SVG (no CSS, no `foreignObject`, plain
//!   `<text>`) themed like the app via [`DiagramTheme`], suitable for GPUI's
//!   resvg-based SVG rasterizer,
//! * [`render_png`]s it, rasterized with resvg ("Copy Image").
//!
//! Each first [`clean`]s the source: ``` fences and
//! `%%{init}%%` directives go, and sequence messages lose their quotes.
//!
//! Status classes: nodes tagged `added`, `changed` or `removed`
//! (`class A added`, `A:::added`, `cssClass "A" added`) get green / amber /
//! red tinted fills with matching borders in every diagram kind; `removed`
//! ones a dashed border and struck-through text. A
//! `classDef added fill:...` in the source overrides individual properties.
//!
//! Rendering is done by the pure-Rust `mermaid-rs-renderer` (parse + layout
//! + SVG), with a styling pre-pass and an SVG post-pass in this crate.

mod clean;
mod error;
mod lint;
mod style;
mod svg;
mod theme;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, LazyLock};

use mermaid_rs_renderer::{
    DiagramKind, Graph, LayoutConfig, ParseError, ParseOutput, compute_layout, parse_mermaid,
    render_svg as upstream_render, validator,
};

pub use clean::clean;
pub use style::STATUS_CLASSES;
pub use theme::{DiagramTheme, font_family_for, hex, mix};

/// A rendered diagram. `width` / `height` are the SVG's intrinsic size in
/// CSS pixels (the `<svg>` `width`/`height` attributes).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedDiagram {
    pub svg: String,
    pub width: f32,
    pub height: f32,
}

/// Summary of a successfully parsed diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramInfo {
    /// Human-readable kind, e.g. `"flowchart"`, `"sequence diagram"`.
    pub kind: &'static str,
    /// User-visible nodes (participants, classes, states, entities...).
    pub nodes: usize,
    pub edges: usize,
}

/// [`clean`]s, parses and lays out `src`. On failure returns a message that
/// starts with `line L, column C:` when the location is known (lines of
/// `src`, cleaned or not), followed by the offending source line and a
/// caret.
pub fn validate(src: &str) -> Result<(), String> {
    inspect(src).map(|_| ())
}

/// Like [`validate`], returning what was parsed.
pub fn inspect(src: &str) -> Result<DiagramInfo, String> {
    let src = clean::clean_in_place(src);
    let theme = DiagramTheme::light();
    let graph = parse(&src, &theme)?;
    let info = DiagramInfo {
        kind: style::kind_name(graph.kind),
        nodes: style::visible_node_count(&graph),
        edges: graph.edges.len(),
    };
    layout_and_render(&graph, &theme)?;
    Ok(info)
}

/// [`clean`]s `src` and renders it to a themed SVG.
pub fn render_svg(src: &str, theme: &DiagramTheme) -> Result<RenderedDiagram, String> {
    let src = clean::clean_in_place(src);
    let graph = parse(&src, theme)?;
    let raw = layout_and_render(&graph, theme)?;
    let svg = svg::post_process(&raw, &theme.font_family, &hex(theme.accent));
    let svg = svg::mark_statuses(
        &svg,
        &status_marks(&graph),
        graph.kind == DiagramKind::Sequence,
    );
    let (width, height) = svg::dimensions(&svg)
        .ok_or_else(|| "internal error: rendered SVG has no width/height".to_string())?;
    Ok(RenderedDiagram { svg, width, height })
}

/// Renders `src` as [`render_svg`] does and rasterizes it with resvg, as
/// GPUI draws it, `scale` device pixels per SVG pixel (2.0 for a Retina
/// copy): a PNG, e.g. for the clipboard.
pub fn render_png(src: &str, theme: &DiagramTheme, scale: f32) -> Result<Vec<u8>, String> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("invalid scale {scale}"));
    }
    let rendered = render_svg(src, theme)?;
    rasterize(&rendered.svg, scale)
}

/// The system's fonts, loaded once.
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    Arc::new(db)
});

fn rasterize(svg: &str, scale: f32) -> Result<Vec<u8>, String> {
    let options = usvg::Options {
        fontdb: FONTS.clone(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let size = tree
        .size()
        .to_int_size()
        .scale_by(scale)
        .ok_or_else(|| "the diagram is too large to draw".to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| "the diagram is empty".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| e.to_string())
}

/// How the status classes are drawn, with the labels of their nodes.
fn status_marks(graph: &Graph) -> Vec<svg::StatusMark> {
    STATUS_CLASSES
        .iter()
        .filter_map(|&name| {
            let style = graph.class_defs.get(name)?;
            let mut labels: Vec<String> = graph
                .node_classes
                .iter()
                .filter(|(_, classes)| classes.iter().any(|c| c == name))
                .filter_map(|(id, _)| graph.nodes.get(id))
                .map(|n| n.label.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|l| !l.is_empty())
                .collect();
            labels.sort();
            Some(svg::StatusMark {
                fill: style.fill.clone()?,
                stroke: style.stroke.clone()?,
                stroke_width: style.stroke_width.unwrap_or(1.5),
                dasharray: style.stroke_dasharray.clone(),
                strike: name == "removed",
                labels,
            })
        })
        .collect()
}

/// Text for `diffr check-diagram`: `(true, "ok: flowchart, 5 nodes")` or
/// `(false, "error: line 3, column 5: ...")`.
pub fn check_command_output(src: &str) -> (bool, String) {
    match inspect(src) {
        Ok(info) => {
            let noun = if info.nodes == 1 { "node" } else { "nodes" };
            (true, format!("ok: {}, {} {noun}", info.kind, info.nodes))
        }
        Err(e) => (false, format!("error: {e}")),
    }
}

fn parse(src: &str, theme: &DiagramTheme) -> Result<Graph, String> {
    if src.trim().is_empty() {
        return Err(
            "empty diagram: start with a diagram type, e.g. `flowchart TD` or `sequenceDiagram`"
                .to_string(),
        );
    }
    let header = style::detect_header(src);
    let (cleaned, extracted) = style::extract(src, header);
    let parsed = catch_unwind(AssertUnwindSafe(|| parse_strict(&cleaned)))
        .map_err(|_| "internal error: the Mermaid parser crashed on this input".to_string())?
        .map_err(|e| error::describe(src, &e))?;
    lint::check(src, header)?;
    let mut graph = parsed.graph;
    style::apply(src, &mut graph, &extracted)?;
    lint::check_classes(src, &graph)?;
    style::install_status_classes(&mut graph, theme);
    check_not_empty(&graph)?;
    Ok(graph)
}

/// `mermaid_rs_renderer::parse_mermaid_strict`, except that the
/// pre-validator does not see class annotation lines (`<<interface>>`),
/// which it mistakes for a leading arrow although the parser supports them.
fn parse_strict(src: &str) -> Result<ParseOutput, ParseError> {
    let masked: String = src
        .lines()
        .map(|line| {
            let t = line.trim_start();
            let annotation = t.starts_with("<<")
                && t[2..].starts_with(|c: char| c.is_alphabetic())
                && t.contains(">>");
            if annotation { "" } else { line }
        })
        .collect::<Vec<_>>()
        .join("\n");
    validator::validate(&masked)?;
    parse_mermaid(src).map_err(|err| ParseError::UnexpectedToken {
        line: 1,
        col: 1,
        found: src
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .unwrap_or("<empty>")
            .to_string(),
        expected: err.to_string(),
    })
}

/// The parser is lenient and accepts headers with nothing after them;
/// an empty diagram is almost certainly a mistake on the agent's side.
fn check_not_empty(graph: &Graph) -> Result<(), String> {
    let has_content = match graph.kind {
        DiagramKind::Pie => !graph.pie_slices.is_empty(),
        DiagramKind::Gantt => !graph.gantt_tasks.is_empty(),
        DiagramKind::GitGraph | DiagramKind::Quadrant | DiagramKind::XYChart => true,
        DiagramKind::Timeline | DiagramKind::C4 | DiagramKind::Mindmap => true,
        _ => style::visible_node_count(graph) > 0 || !graph.subgraphs.is_empty(),
    };
    if has_content {
        Ok(())
    } else {
        Err(format!(
            "the {} has no nodes: add at least one node or edge after the header line",
            style::kind_name(graph.kind)
        ))
    }
}

fn layout_and_render(graph: &Graph, theme: &DiagramTheme) -> Result<String, String> {
    let renderer_theme = theme.to_renderer_theme();
    let config = LayoutConfig::default();
    catch_unwind(AssertUnwindSafe(|| {
        let layout = compute_layout(graph, &renderer_theme, &config);
        upstream_render(&layout, &renderer_theme, &config)
    }))
    .map_err(|_| "internal error: laying out this diagram failed".to_string())
}
