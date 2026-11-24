//! Colors: codiff's Licht and Dunkel palettes, and the theme of
//! the controls around them.

use gpui::{Hsla, Rgba};

/// A color from 0xRRGGBB.
pub fn hex(v: u32) -> Hsla {
    gpui::rgb(v).into()
}

/// A color from 0-255 channels and an alpha.
pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a }.into()
}

/// Lays a translucent color over another.
pub fn over(base: Hsla, top: Hsla) -> Hsla {
    let b: Rgba = base.into();
    let t: Rgba = top.into();
    let a = t.a + b.a * (1. - t.a);
    if a <= 0. {
        return Rgba { r: 0., g: 0., b: 0., a: 0. }.into();
    }
    let mix = |x: f32, y: f32| (y * t.a + x * b.a * (1. - t.a)) / a;
    Rgba { r: mix(b.r, t.r), g: mix(b.g, t.g), b: mix(b.b, t.b), a }.into()
}

/// Mixes two opaque colors: t of the way from a to b.
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let x: Rgba = a.into();
    let y: Rgba = b.into();
    let l = |p: f32, q: f32| p + (q - p) * t;
    Rgba { r: l(x.r, y.r), g: l(x.g, y.g), b: l(x.b, y.b), a: l(x.a, y.a) }.into()
}

pub fn alpha(c: Hsla, a: f32) -> Hsla {
    Hsla { a: c.a * a, ..c }
}

/// The kinds of tokens, as diffr-highlight's classes.
pub const NUM_CLASSES: usize = 19;

#[derive(Clone)]
#[allow(dead_code)]
pub struct Palette {
    pub dark: bool,
    pub app_bg: Hsla,
    pub code: Hsla,
    pub line_number: Hsla,
    pub code_bg: Hsla,
    pub add_bg: Hsla,
    pub add_gutter: Hsla,
    pub add_word: Hsla,
    pub add_bar: Hsla,
    pub add_text: Hsla,
    pub del_bg: Hsla,
    pub del_gutter: Hsla,
    pub del_word: Hsla,
    pub del_bar: Hsla,
    pub del_text: Hsla,
    pub empty_side: Hsla,
    pub gap_bg: Hsla,
    pub gap_text: Hsla,
    pub card_border: Hsla,
    pub header_bg: Hsla,
    pub matched: Hsla,
    pub match_now: Hsla,
    pub selected: Hsla,
    pub viewed: Hsla,
    pub reference: Hsla,
    pub syntax: [Hsla; NUM_CLASSES],

    // The theme of the controls (MyGo's light and dark themes).
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub surface_pressed: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    /// Text on a filled accent button or selection.
    pub accent_text: Hsla,
    /// Colored action text on neutral or tinted surfaces.
    pub accent_foreground: Hsla,
    pub danger: Hsla,
    /// Critical labels and error text; distinct from destructive fills.
    pub danger_foreground: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub focus: Hsla,
    pub sidebar_bg: Hsla,
    pub modified: Hsla,
    pub renamed: Hsla,
}

// Class order: Plain, Comment, Preproc, Keyword, Type, LangConst, Function,
// ClassName, Exception, Number, String, Escape, Regexp, Tag, Attribute,
// Property, Heading, Inserted, Deleted.
fn syntax(code: u32, l: [u32; 18]) -> [Hsla; NUM_CLASSES] {
    let mut out = [hex(code); NUM_CLASSES];
    for (i, v) in l.iter().enumerate() {
        out[i + 1] = hex(*v);
    }
    out
}

impl Palette {
    pub fn light() -> Self {
        Self {
            dark: false,
            app_bg: hex(0xf7f7f8),
            code: hex(0x111111),
            line_number: hex(0x62626b),
            code_bg: hex(0xffffff),
            add_bg: hex(0xe2f7ea),
            add_gutter: hex(0xd3f3df),
            add_word: rgba(13, 190, 78, 0.20),
            add_bar: hex(0x16a34a),
            add_text: rgba(25, 105, 54, 1.),
            del_bg: hex(0xffe6e8),
            del_gutter: hex(0xffd9dc),
            del_word: rgba(255, 46, 63, 0.17),
            del_bar: hex(0xe5484d),
            del_text: rgba(172, 42, 40, 1.),
            empty_side: hex(0xfafafa),
            gap_bg: hex(0xf4f4f4),
            gap_text: hex(0x62626b),
            card_border: rgba(17, 17, 17, 0.10),
            header_bg: hex(0xffffff),
            matched: rgba(255, 216, 92, 0.65),
            match_now: rgba(255, 176, 46, 0.96),
            selected: rgba(61, 135, 245, 0.13),
            viewed: hex(0x62626b),
            reference: hex(0x89500a),
            syntax: syntax(
                0x111111,
                [
                    0x55555e, 0x55555e, 0x4432a8, 0x805008, 0x4d55a5, 0x284181, 0x86318e, 0x9c2525,
                    0x9c352b, 0x176535, 0x176535, 0x496329, 0x175a91, 0x175a91, 0x444444, 0x111111,
                    0x176535, 0x98232b,
                ],
            ),
            background: hex(0xffffff),
            surface: hex(0xf4f4f5),
            surface_hover: hex(0xe9e9ec),
            surface_pressed: hex(0xdddde1),
            border: hex(0xd9d9de),
            text: hex(0x18181b),
            text_muted: hex(0x62626b),
            accent: hex(0x2563eb),
            accent_hover: hex(0x1d4ed8),
            accent_text: hex(0xffffff),
            accent_foreground: hex(0x1d4ed8),
            danger: hex(0xdc2626),
            danger_foreground: hex(0xb91c1c),
            warning: hex(0x89500a),
            success: hex(0x18733c),
            focus: rgba(37, 99, 235, 0.55),
            sidebar_bg: hex(0xebebed),
            modified: hex(0x705008),
            renamed: hex(0x7641a1),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            app_bg: hex(0x18181b),
            code: hex(0xc8c8c8),
            line_number: hex(0x9999a2),
            code_bg: hex(0x1c1c20),
            add_bg: hex(0x22382a),
            add_gutter: hex(0x2a4632),
            add_word: rgba(94, 204, 113, 0.22),
            add_bar: hex(0x5ecc71),
            add_text: rgba(111, 208, 148, 1.),
            del_bg: hex(0x3d1f25),
            del_gutter: hex(0x50242d),
            del_word: rgba(255, 103, 98, 0.24),
            del_bar: hex(0xff6762),
            del_text: rgba(238, 126, 126, 1.),
            empty_side: hex(0x202020),
            gap_bg: hex(0x262626),
            gap_text: hex(0x9999a2),
            card_border: rgba(230, 230, 230, 0.10),
            header_bg: hex(0x232327),
            matched: rgba(255, 216, 92, 0.38),
            match_now: rgba(255, 176, 46, 0.80),
            selected: rgba(90, 150, 255, 0.12),
            viewed: hex(0xb0b0ba),
            reference: hex(0xeb9a3d),
            syntax: syntax(
                0xc8c8c8,
                [
                    0xc6c6c6, 0xc6c6c6, 0xb2c7e6, 0xf2bc7e, 0xb9c2fe, 0xb9c2fe, 0xf6acfc, 0xffb2ac,
                    0xf3b8b0, 0x7bdd9c, 0x7bdd9c, 0xb2d08c, 0xb2c7e6, 0xb2c7e6, 0xc8c8c8, 0xe8e8e8,
                    0x76db82, 0xffb2ad,
                ],
            ),
            background: hex(0x18181b),
            surface: hex(0x27272a),
            surface_hover: hex(0x323236),
            surface_pressed: hex(0x3c3c41),
            border: hex(0x3f3f46),
            text: hex(0xf4f4f5),
            text_muted: hex(0xa1a1aa),
            accent: hex(0x2563eb),
            accent_hover: hex(0x1d4ed8),
            accent_text: hex(0xffffff),
            accent_foreground: hex(0x8ab4ff),
            danger: hex(0xdc2626),
            danger_foreground: hex(0xff8f8f),
            warning: hex(0xf59e0b),
            success: hex(0x22c55e),
            focus: rgba(96, 165, 250, 0.6),
            sidebar_bg: hex(0x262628),
            modified: hex(0xd4a72c),
            renamed: hex(0xc29aef),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |c: Hsla| {
            let c: Rgba = c.into();
            let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn action_and_critical_text_is_readable_on_its_surfaces() {
        let mut failures = Vec::new();
        for p in [Palette::light(), Palette::dark()] {
            for (name, foreground, background) in [
                ("diagram link", p.accent_foreground, p.code_bg),
                ("AI action", p.accent_foreground, p.header_bg),
                ("hovered link", p.accent_foreground, p.surface_hover),
                ("note label", p.accent_foreground, over(p.code_bg, alpha(p.accent, 0.07))),
                ("critical count", p.danger_foreground, p.header_bg),
                ("critical badge", p.danger_foreground, over(p.app_bg, alpha(p.danger, 0.14))),
                ("critical note label", p.danger_foreground, over(p.code_bg, alpha(p.danger, 0.07))),
            ] {
                let ratio = contrast(foreground, background);
                if ratio < 4.5 { failures.push(format!("dark={} {name}: {ratio:.2}", p.dark)); }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn informational_text_is_readable_on_diff_surfaces() {
        for p in [Palette::light(), Palette::dark()] {
            for (name, fg, bg) in [
                ("line numbers", p.line_number, p.code_bg),
                ("context", p.gap_text, p.gap_bg),
                ("added numbers", p.add_text, p.add_gutter),
                ("deleted numbers", p.del_text, p.del_gutter),
                ("modified status", p.modified, p.sidebar_bg),
                ("selected modified status", p.modified, over(p.sidebar_bg, p.selected)),
                ("selected added status", p.add_text, over(p.sidebar_bg, p.selected)),
                ("selected deleted status", p.del_text, over(p.sidebar_bg, p.selected)),
                ("selected renamed status", p.renamed, over(p.sidebar_bg, p.selected)),
                ("command shortcut", p.accent_text, p.accent),
                ("primary button", p.accent_text, p.accent),
                ("destructive button", p.accent_text, p.danger),
            ] {
                assert!(contrast(fg, bg) >= 4.5, "dark={} {name}: {}", p.dark, contrast(fg, bg));
            }
            for (i, fg) in p.syntax.iter().enumerate() {
                for bg in [p.code_bg, p.add_bg, p.del_bg, over(p.add_bg, p.add_word), over(p.del_bg, p.del_word)] {
                    assert!(contrast(*fg, bg) >= 4.5, "dark={} syntax {i}: {}", p.dark, contrast(*fg, bg));
                }
            }
        }
    }
}
