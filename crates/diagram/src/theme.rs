//! App-matching diagram theme and its mapping onto the renderer's theme.

use mermaid_rs_renderer::Theme;

/// Colors (`0xRRGGBB`) and typography used to render diagrams so they look
/// like the rest of diffr.
#[derive(Debug, Clone, PartialEq)]
pub struct DiagramTheme {
    pub dark: bool,
    /// Canvas background.
    pub background: u32,
    /// Node fill.
    pub surface: u32,
    /// Node / cluster border.
    pub border: u32,
    /// Primary text.
    pub text: u32,
    /// Edges, arrowheads and secondary labels.
    pub muted: u32,
    pub accent: u32,
    /// `added` class (green).
    pub added: u32,
    /// `changed` class (amber).
    pub changed: u32,
    /// `removed` class (red).
    pub removed: u32,
    /// CSS-style font-family list, e.g. `"Noto Sans", sans-serif`.
    pub font_family: String,
    pub font_size: f32,
}

impl DiagramTheme {
    pub fn light() -> Self {
        Self {
            dark: false,
            background: 0xffffff,
            surface: 0xf8f8f6,
            border: 0xd9d9d9,
            text: 0x18181b,
            muted: 0x71717a,
            accent: 0x2563eb,
            added: 0x0dbe4e,
            changed: 0xd4a72c,
            removed: 0xff2e3f,
            font_family: default_font_family(),
            font_size: 13.0,
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            background: 0x1c1c1c,
            surface: 0x232323,
            border: 0x3f3f46,
            text: 0xf4f4f5,
            muted: 0xa1a1aa,
            accent: 0x3b82f6,
            added: 0x5ecc71,
            changed: 0xeb9a3d,
            removed: 0xff6762,
            font_family: default_font_family(),
            font_size: 13.0,
        }
    }

    /// Fill used for nodes tagged with a status class (`added`, ...): the
    /// status color blended into the background.
    pub fn tint(&self, color: u32) -> u32 {
        mix(self.background, color, if self.dark { 0.22 } else { 0.14 })
    }

    /// The renderer theme matching this app theme.
    pub(crate) fn to_renderer_theme(&self) -> Theme {
        let mut t = Theme::modern();
        let hex = |c: u32| hex(c);
        let subtle = mix(self.background, self.surface, 0.5);
        t.font_family = self.font_family.clone();
        t.font_size = self.font_size;
        t.primary_color = hex(self.surface);
        t.primary_text_color = hex(self.text);
        t.primary_border_color = hex(self.border);
        t.line_color = hex(self.muted);
        t.secondary_color = hex(mix(self.surface, self.border, 0.35));
        t.tertiary_color = hex(self.background);
        t.edge_label_background = hex(self.background);
        t.cluster_background = hex(subtle);
        t.cluster_border = hex(self.border);
        t.background = hex(self.background);
        t.sequence_actor_fill = hex(self.surface);
        t.sequence_actor_border = hex(self.border);
        t.sequence_actor_line = hex(self.muted);
        t.sequence_note_fill = hex(self.tint(self.changed));
        t.sequence_note_border = hex(self.changed);
        t.sequence_activation_fill = hex(mix(self.surface, self.border, 0.5));
        t.sequence_activation_border = hex(self.border);
        t.text_color = hex(self.text);

        let palette = [
            self.accent,
            self.added,
            self.changed,
            self.removed,
            mix(self.accent, self.added, 0.5),
            mix(self.changed, self.removed, 0.5),
            self.muted,
            mix(self.accent, self.removed, 0.5),
        ];
        for (i, slot) in t.git_colors.iter_mut().enumerate() {
            *slot = hex(palette[i % palette.len()]);
        }
        for slot in t.git_inv_colors.iter_mut() {
            *slot = hex(self.background);
        }
        for slot in t.git_branch_label_colors.iter_mut() {
            *slot = hex(self.background);
        }
        t.git_commit_label_color = hex(self.text);
        t.git_commit_label_background = hex(self.surface);
        t.git_tag_label_color = hex(self.text);
        t.git_tag_label_background = hex(self.surface);
        t.git_tag_label_border = hex(self.border);

        for (i, slot) in t.pie_colors.iter_mut().enumerate() {
            let base = palette[i % palette.len()];
            // Second round of the palette gets a lighter/darker variant.
            *slot = hex(if i < palette.len() {
                base
            } else {
                mix(base, self.background, 0.4)
            });
        }
        t.pie_title_text_color = hex(self.text);
        t.pie_section_text_color = hex(self.text);
        t.pie_legend_text_color = hex(self.text);
        t.pie_stroke_color = hex(self.background);
        t.pie_outer_stroke_color = hex(self.border);
        t.pie_title_text_size = self.font_size * 1.4;
        t.pie_section_text_size = self.font_size;
        t.pie_legend_text_size = self.font_size;
        t
    }
}

impl Default for DiagramTheme {
    fn default() -> Self {
        Self::light()
    }
}

fn default_font_family() -> String {
    font_family_for("system-ui")
}

/// The font-family list for diagram text set in the UI font `ui`, as resvg
/// and the layout's text measurer resolve it: by the family names in the
/// font files. The system UI font's aliases (`.SystemUIFont`, `system-ui`,
/// `-apple-system`) are in none, and an unknown family falls back to Arial.
/// On macOS resvg draws no glyphs of SF Pro (SFNS.ttf), so the system font
/// there is Helvetica Neue, the nearest to it.
pub fn font_family_for(ui: &str) -> String {
    let system = matches!(ui, ".SystemUIFont" | ".AppleSystemUIFont" | "system-ui" | "-apple-system");
    let platform = if cfg!(target_os = "macos") {
        "\"Helvetica Neue\""
    } else if cfg!(target_os = "windows") {
        "\"Segoe UI\""
    } else {
        "\"Noto Sans\", \"DejaVu Sans\""
    };
    if system { format!("{platform}, sans-serif") } else { format!("\"{ui}\", {platform}, sans-serif") }
}

/// `#rrggbb` for a `0xRRGGBB` color.
pub fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xff_ffff)
}

/// Linear blend from `a` (t = 0) to `b` (t = 1).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as f32;
    let blend = |s: u32| {
        let v = ch(a, s) + (ch(b, s) - ch(a, s)) * t;
        (v.round().clamp(0.0, 255.0) as u32) << s
    };
    blend(16) | blend(8) | blend(0)
}
