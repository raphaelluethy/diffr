//! Render a Mermaid file with the app theme.
//!
//! ```text
//! cargo run -p diffr-diagram --example render -- diagram.mmd [--dark] [-o out.svg] [--png out.png]
//! ```
//!
//! Without `-o` the SVG is written next to the input as `<name>.<light|dark>.svg`.

use std::path::PathBuf;
use std::process::ExitCode;

use diffr_diagram::{DiagramTheme, check_command_output, render_svg};

fn main() -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut dark = false;
    let mut out: Option<PathBuf> = None;
    let mut png: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dark" => dark = true,
            "--light" => dark = false,
            "-o" => out = args.next().map(PathBuf::from),
            "--png" => png = args.next().map(PathBuf::from),
            _ => input = Some(PathBuf::from(arg)),
        }
    }
    let Some(input) = input else {
        eprintln!("usage: render <file.mmd> [--dark] [-o out.svg] [--png out.png]");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };
    let (ok, message) = check_command_output(&src);
    eprintln!("{message}");
    if !ok {
        return ExitCode::FAILURE;
    }
    let theme = if dark {
        DiagramTheme::dark()
    } else {
        DiagramTheme::light()
    };
    let rendered = match render_svg(&src, &theme) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out =
        out.unwrap_or_else(|| input.with_extension(if dark { "dark.svg" } else { "light.svg" }));
    if let Err(e) = std::fs::write(&out, &rendered.svg) {
        eprintln!("{}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "wrote {} ({:.0}x{:.0})",
        out.display(),
        rendered.width,
        rendered.height
    );
    if let Some(png) = png {
        if let Err(e) = rasterize(&rendered.svg, &png) {
            eprintln!("{}: {e}", png.display());
            return ExitCode::FAILURE;
        }
        eprintln!("wrote {}", png.display());
    }
    ExitCode::SUCCESS
}

/// Rasterizes at 2x the way GPUI does (resvg with system fonts).
fn rasterize(svg: &str, path: &std::path::Path) -> Result<(), String> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let scale = 2.0;
    let size = tree
        .size()
        .to_int_size()
        .scale_by(scale)
        .ok_or("bad size")?;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("empty pixmap")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(path).map_err(|e| e.to_string())
}
