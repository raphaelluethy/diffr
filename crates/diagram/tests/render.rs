use diffr_diagram::{
    DiagramTheme, RenderedDiagram, check_command_output, clean, hex, inspect, render_png,
    render_svg, validate,
};

const FLOWCHART: &str = "\
flowchart LR
  subgraph App[diffr]
    Review[Review panel] --> Render[Diagram view]
  end
  Agent([AI agent]) -->|emits| Check{valid?}
  Check -->|yes| Render
  Check -->|no| Agent
  class Render added
";

const SEQUENCE: &str = "\
sequenceDiagram
  participant A as Agent
  participant D as diffr
  A->>D: diagram source
  alt invalid
    D-->>A: line 2, column 3: ...
  else ok
    D-->>A: rendered
  end
  Note over A,D: loop until valid
";

const CLASS: &str = "\
classDiagram
  class DiagramTheme {
    +bool dark
    +light() DiagramTheme
  }
  class Renderer {
    <<interface>>
    +render(src) Svg
  }
  Renderer ..> DiagramTheme : uses
  cssClass \"Renderer\" changed
";

const STATE: &str = "\
stateDiagram-v2
  [*] --> Idle
  Idle --> Validating : emit
  Validating --> Rendered : ok
  Validating --> Idle : error
  Rendered --> [*]
  Validating:::added
";

const ER: &str = "\
erDiagram
  REVIEW ||--o{ DIAGRAM : contains
  REVIEW {
    string id PK
  }
  DIAGRAM {
    string source
  }
  class DIAGRAM removed
";

const ALL: [(&str, &str, usize); 5] = [
    ("flowchart", FLOWCHART, 4),
    ("sequence diagram", SEQUENCE, 2),
    ("class diagram", CLASS, 2),
    ("state diagram", STATE, 3),
    ("ER diagram", ER, 2),
];

/// Parses with usvg (as GPUI does) and rasterizes with resvg; returns the
/// number of pixels that differ from the background.
fn rasterize(rendered: &RenderedDiagram, background: u32) -> usize {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&rendered.svg, &options).expect("usvg parses the SVG");
    assert!((tree.size().width() - rendered.width).abs() < 1.0);
    assert!((tree.size().height() - rendered.height).abs() < 1.0);
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
    resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
    let [r, g, b] = [
        (background >> 16) as u8,
        (background >> 8) as u8,
        background as u8,
    ];
    pixmap
        .pixels()
        .iter()
        .filter(|p| p.alpha() > 0 && (p.red(), p.green(), p.blue()) != (r, g, b))
        .count()
}

fn has_text(rendered: &RenderedDiagram) -> bool {
    let options = {
        let mut o = usvg::Options::default();
        o.fontdb_mut().load_system_fonts();
        o
    };
    let tree = usvg::Tree::from_str(&rendered.svg, &options).unwrap();
    fn walk(group: &usvg::Group) -> bool {
        group.children().iter().any(|node| match node {
            usvg::Node::Text(_) => true,
            usvg::Node::Group(g) => walk(g),
            _ => false,
        })
    }
    walk(tree.root())
}

#[test]
fn every_kind_validates_and_renders() {
    for (kind, src, nodes) in ALL {
        validate(src).unwrap_or_else(|e| panic!("{kind} should validate: {e}"));
        let info = inspect(src).unwrap();
        assert_eq!(info.kind, kind);
        assert_eq!(info.nodes, nodes, "{kind} node count");
        for theme in [DiagramTheme::light(), DiagramTheme::dark()] {
            let rendered = render_svg(src, &theme).unwrap();
            assert!(rendered.width > 10.0 && rendered.height > 10.0, "{kind}");
            assert!(!rendered.svg.contains("foreignObject"), "{kind}");
            let painted = rasterize(&rendered, theme.background);
            assert!(
                painted > 500,
                "{kind} rasterized almost empty ({painted} px)"
            );
            assert!(has_text(&rendered), "{kind} has no text nodes");
            // Theme applied: background, node text and edge colors.
            assert!(
                rendered
                    .svg
                    .contains(&format!("fill=\"{}\"", hex(theme.background)))
            );
            assert!(
                rendered
                    .svg
                    .contains(&format!("fill=\"{}\"", hex(theme.text)))
            );
            assert!(
                rendered.svg.contains(&hex(theme.muted)),
                "{kind} edge color"
            );
        }
    }
}

#[test]
fn check_command_output_reports_kind_and_count() {
    assert_eq!(
        check_command_output(FLOWCHART),
        (true, "ok: flowchart, 4 nodes".to_string())
    );
    assert_eq!(
        check_command_output(SEQUENCE),
        (true, "ok: sequence diagram, 2 nodes".to_string())
    );
    let (ok, msg) = check_command_output("flowchart LR\n  A --> B[oops\n");
    assert!(!ok);
    assert!(msg.starts_with("error: line 2, column 10:"), "{msg}");
}

fn err(src: &str) -> String {
    validate(src).expect_err("should be rejected")
}

#[test]
fn invalid_syntax_reports_location() {
    let cases: &[(&str, &str)] = &[
        ("flowchart LR\n  A --> B\n  B -->\n", "line 3, column 3"),
        ("flowchart LR\n  A --> B\n  --> C\n", "line 3, column 3"),
        (
            "flowchart TD\n  A --> B\n  subgraph S\n    C\n",
            "line 3, column 3",
        ),
        ("flowchart TD\n  A[open --> B\n", "line 2, column 4"),
        (
            "sequenceDiagram\n  participant Alice\n  participant Bob\n  Alice->>Bobb: hi\n",
            "line 4, column 11",
        ),
        (
            "sequenceDiagram\n  Alice->>Bob hello\n",
            "line 2, column 11",
        ),
        (
            "sequenceDiagram\n  A->>B: x\n  loop forever\n",
            "line 3, column 3",
        ),
        (
            "classDiagram\n  class A {\n    +x int\n",
            "line 2, column 3",
        ),
        ("classDiagram\n  A <|-- B\n  B ..|>\n", "line 3, column 5"),
        (
            "stateDiagram-v2\n  [*] --> A\n  A -->\n",
            "line 3, column 5",
        ),
        ("erDiagram\n  A ||--o{ B\n", "line 2, column 11"),
        ("erDiagram\n  A {\n    string x\n", "line 2, column 3"),
        ("flowchrt LR\n  A --> B\n", "line 1, column 1"),
        (
            "flowchart LR\n  A --> B\n  class A hot\n",
            "line 3, column 11",
        ),
        (
            "flowchart LR\n  A --> B\n  class C added\n",
            "line 3, column 9",
        ),
        ("stateDiagram-v2\n  [*] --> A\n  Nope:::added\n", "line 3"),
    ];
    for (src, location) in cases {
        let e = err(src);
        assert!(e.starts_with(location), "{src:?}\n=> {e}");
        // The offending line is quoted with a caret under it.
        assert!(e.contains(" | ") && e.contains('^'), "{e}");
    }
    assert!(err("").contains("empty diagram"));
    assert!(err("flowchart TD\n").contains("no nodes"));
}

#[test]
fn unknown_participant_suggests_candidates() {
    let e = err("sequenceDiagram\n  participant Alice\n  participant Bob\n  Alice->>Bobb: hi\n");
    assert!(e.contains("did you mean Bob"), "{e}");
}

/// The `<rect>`/`<polygon>` tags carrying `fill`.
fn shapes_with_fill<'a>(svg: &'a str, fill: &str) -> Vec<&'a str> {
    let needle = format!("fill=\"{fill}\"");
    svg.split('<')
        .filter(|tag| {
            (tag.starts_with("rect ") || tag.starts_with("polygon ") || tag.starts_with("path "))
                && tag.contains(&needle)
        })
        .collect()
}

#[test]
fn status_classes_get_tinted_fills_in_every_kind() {
    let theme = DiagramTheme::light();
    let cases = [
        (FLOWCHART, theme.added),
        (CLASS, theme.changed),
        (STATE, theme.added),
        (ER, theme.removed),
    ];
    for (src, color) in cases {
        let svg = render_svg(src, &theme).unwrap().svg;
        let fill = hex(theme.tint(color));
        let shapes = shapes_with_fill(&svg, &fill);
        assert!(!shapes.is_empty(), "no {fill} fill in:\n{src}");
        assert!(
            shapes
                .iter()
                .any(|s| s.contains(&format!("stroke=\"{}\"", hex(color)))),
            "border should match the status color in:\n{src}"
        );
    }
    // Inline `:::` and `class` statement forms, flowchart.
    for src in [
        "flowchart LR\n  A:::added --> B\n",
        "flowchart LR\n  A --> B\n  class A added\n",
        "flowchart LR\n  A --> B\n  A:::added\n",
    ] {
        let svg = render_svg(src, &theme).unwrap().svg;
        assert_eq!(
            shapes_with_fill(&svg, &hex(theme.tint(theme.added))).len(),
            1,
            "{src}"
        );
    }
}

#[test]
fn added_is_green() {
    let theme = DiagramTheme::light();
    let tint = theme.tint(theme.added);
    let (r, g, b) = ((tint >> 16) & 0xff, (tint >> 8) & 0xff, tint & 0xff);
    assert!(g > r && g > b, "added tint {tint:06x} is not green");
    let svg = render_svg("flowchart LR\n  A[New] --> B\n  class A added\n", &theme)
        .unwrap()
        .svg;
    assert!(svg.contains(&format!("fill=\"{}\"", hex(tint))));
    assert!(svg.contains("stroke=\"#0dbe4e\""));
}

#[test]
fn class_def_overrides_status_defaults() {
    let theme = DiagramTheme::light();
    let src = "flowchart LR\n  classDef added fill:#123456\n  A:::added --> B\n";
    let svg = render_svg(src, &theme).unwrap().svg;
    let shapes = shapes_with_fill(&svg, "#123456");
    assert_eq!(shapes.len(), 1);
    // Properties the user did not override keep the theme value.
    assert!(shapes[0].contains(&format!("stroke=\"{}\"", hex(theme.added))));

    let src = "classDiagram\n  class A\n  class B\n  A --> B\n  classDef hot fill:#abcdef\n  cssClass \"A\" hot\n";
    let svg = render_svg(src, &theme).unwrap().svg;
    assert_eq!(shapes_with_fill(&svg, "#abcdef").len(), 1);
}

#[test]
fn dark_and_light_differ() {
    for (kind, src, _) in ALL {
        let light = render_svg(src, &DiagramTheme::light()).unwrap();
        let dark = render_svg(src, &DiagramTheme::dark()).unwrap();
        assert_ne!(light.svg, dark.svg, "{kind}");
        assert!(dark.svg.contains("#1c1c1c"), "{kind} dark background");
        assert!(light.svg.contains("#ffffff"), "{kind} light background");
        assert!(
            !dark.svg.contains("#ffffff\""),
            "{kind} light color leaked into dark"
        );
    }
}

#[test]
fn nodes_have_rounded_corners_and_theme_font() {
    let theme = DiagramTheme::light();
    let svg = render_svg("flowchart LR\n  A[Box] --> B[Other]\n", &theme)
        .unwrap()
        .svg;
    assert!(svg.contains("rx=\"6\" ry=\"6\""), "{svg}");
    assert!(svg.contains("font-family=\"'Noto Sans'") || !cfg!(target_os = "linux"));
    assert!(svg.contains(&format!("font-size=\"{}\"", theme.font_size)));
}

#[test]
fn rendering_is_deterministic() {
    let theme = DiagramTheme::dark();
    assert_eq!(
        render_svg(FLOWCHART, &theme).unwrap(),
        render_svg(FLOWCHART, &theme).unwrap()
    );
}

#[test]
fn sources_are_cleaned_first() {
    let theme = DiagramTheme::light();
    let plain = "flowchart LR\n  A[Box] --> B[Other]";
    let wrapped =
        "```mermaid\n%%{init: {'theme': 'forest'}}%%\nflowchart LR\n  A[Box] --> B[Other]\n```\n";
    assert_eq!(clean(wrapped), "flowchart LR\n  A[Box] --> B[Other]\n");
    validate(wrapped).unwrap();
    assert_eq!(
        render_svg(wrapped, &theme).unwrap().svg,
        render_svg(plain, &theme).unwrap().svg
    );
    assert_eq!(
        check_command_output(wrapped),
        (true, "ok: flowchart, 2 nodes".to_string())
    );
    // Errors keep the lines of the source as given, fence included.
    let e = err("```mermaid\nflowchart LR\n  A --> B[oops\n```");
    assert!(e.starts_with("line 3, column 10:"), "{e}");
    assert!(err("```mermaid\n```").contains("empty diagram"));

    // A sequence message's quotes go.
    let quoted = "sequenceDiagram\n  A->>B: \"parse(args)\"\n  Note over A: \"a note\"";
    let svg = render_svg(quoted, &theme).unwrap().svg;
    assert!(svg.contains(">parse(args)<"), "{svg}");
    assert!(svg.contains(">a note<"), "{svg}");
    assert!(!svg.contains("&quot;") && !svg.contains("\"parse"), "{svg}");
}

/// The texts of the SVG as usvg reads them, with whether they are struck
/// through.
fn texts(svg: &str) -> Vec<(String, bool)> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &options).unwrap();
    fn walk(group: &usvg::Group, out: &mut Vec<(String, bool)>) {
        for node in group.children() {
            match node {
                usvg::Node::Text(t) => {
                    for chunk in t.chunks() {
                        let struck = chunk
                            .spans()
                            .iter()
                            .any(|s| s.decoration().line_through().is_some());
                        out.push((chunk.text().to_string(), struck));
                    }
                }
                usvg::Node::Group(g) => walk(g, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(tree.root(), &mut out);
    out
}

#[test]
fn removed_nodes_are_dashed_and_struck_through() {
    let cases = [
        (
            "flowchart LR\n  A[Keep] --> B[Gone]:::removed\n  B --> C[New]:::added\n",
            "Gone",
        ),
        (
            "flowchart TD\n  A{Gone?}:::removed --> B((Keep))\n",
            "Gone?",
        ),
        (
            "classDiagram\n  class Keep\n  class Gone\n  Keep --> Gone\n  cssClass \"Gone\" removed\n",
            "Gone",
        ),
        (
            "stateDiagram-v2\n  [*] --> Keep\n  Keep --> Gone\n  Gone:::removed\n",
            "Gone",
        ),
        (
            "erDiagram\n  KEEP ||--o{ GONE : has\n  class GONE removed\n",
            "GONE",
        ),
        (
            "sequenceDiagram\n  participant Keep\n  participant Gone\n  Keep->>Gone: hi\n  class Gone removed\n",
            "Gone",
        ),
    ];
    for theme in [DiagramTheme::light(), DiagramTheme::dark()] {
        for (src, gone) in cases {
            let svg = render_svg(src, &theme).unwrap().svg;
            // A dashed border in the removed color.
            let stroke = format!("stroke=\"{}\"", hex(theme.removed));
            assert!(
                svg.split('<')
                    .any(|t| t.contains(&stroke) && t.contains("stroke-dasharray=\"4 3\"")),
                "no dashed removed border:\n{src}\n{svg}"
            );
            // Its text struck through, and no other.
            let texts = texts(&svg);
            assert!(
                texts.iter().any(|(t, s)| t == gone && *s),
                "{gone} not struck in {texts:?}:\n{src}"
            );
            assert!(
                texts.iter().all(|(t, s)| !*s || t == gone),
                "other text struck in {texts:?}:\n{src}"
            );
        }
    }
    // The sequence diagram's participant takes the tint too.
    let theme = DiagramTheme::light();
    let svg = render_svg(cases[5].0, &theme).unwrap().svg;
    assert!(svg.contains(&format!("fill=\"{}\"", hex(theme.tint(theme.removed)))));
    let svg = render_svg(
        "sequenceDiagram\n  participant A\n  participant B\n  A->>B: hi\n  class B added\n",
        &theme,
    )
    .unwrap()
    .svg;
    assert!(svg.contains(&format!("fill=\"{}\"", hex(theme.tint(theme.added)))));
    assert!(!svg.contains("line-through"));
}

#[test]
fn renders_png() {
    let theme = DiagramTheme::dark();
    let svg = render_svg(FLOWCHART, &theme).unwrap();
    let png = render_png(FLOWCHART, &theme, 2.0).unwrap();
    assert!(png.len() > 1000, "{} bytes", png.len());
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    // IHDR: the width and height, scaled.
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert!(
        (w as f32 - 2.0 * svg.width).abs() <= 2.0,
        "{w} vs {}",
        svg.width
    );
    assert!(
        (h as f32 - 2.0 * svg.height).abs() <= 2.0,
        "{h} vs {}",
        svg.height
    );
    assert!(render_png(FLOWCHART, &theme, 0.0).is_err());
    assert!(render_png("flowchart LR\n  A --> B[oops\n", &theme, 1.0).is_err());
}
