//! Token classes and the mapping from tree-sitter capture names to them.

/// The kind of a token, which picks its color. The classes of a
/// Chroma-based highlighter, so codiff's Licht and Dunkel palettes apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Class {
    #[default]
    Plain,
    Comment,
    Preproc,
    Keyword,
    Type,
    LangConst,
    Function,
    ClassName,
    Exception,
    Number,
    String,
    Escape,
    Regexp,
    Tag,
    Attribute,
    Property,
    Heading,
    Inserted,
    Deleted,
}

impl Class {
    /// The number of classes, for tables indexed by `class as usize`.
    pub const COUNT: usize = 19;

    /// Every class, in order of their discriminants.
    pub const ALL: [Class; Class::COUNT] = [
        Class::Plain,
        Class::Comment,
        Class::Preproc,
        Class::Keyword,
        Class::Type,
        Class::LangConst,
        Class::Function,
        Class::ClassName,
        Class::Exception,
        Class::Number,
        Class::String,
        Class::Escape,
        Class::Regexp,
        Class::Tag,
        Class::Attribute,
        Class::Property,
        Class::Heading,
        Class::Inserted,
        Class::Deleted,
    ];
}

/// Capture names the highlighter recognizes, with their classes.
///
/// tree-sitter-highlight gives a capture the recognized name with the most
/// dot-separated parts all found in the capture's name (`function.method.call`
/// → `function`), the first listed on a tie (so `keyword.function` →
/// `keyword`). Captures matching no name (`variable`, `punctuation.*`,
/// `operator`'s siblings in brackets, …) do not change the class of the text
/// they cover. `none`, `embedded` and `variable` reset it to plain, as for
/// the code of an interpolation inside a string.
///
/// The mapping follows Chroma's token types:
/// operators are keywords, constants are numbers, `self` and booleans are
/// language constants, builtins are functions, punctuation is plain.
pub(crate) static NAMES: &[(&str, Class)] = &[
    ("none", Class::Plain),
    ("embedded", Class::Plain),
    // Variables are plain; naming them (rather than leaving them unknown)
    // makes local-variable tracking resolve references to plain too.
    ("variable", Class::Plain),
    ("variable.parameter", Class::Plain),
    // Comments.
    ("comment", Class::Comment),
    ("preproc", Class::Preproc),
    ("keyword.directive", Class::Preproc),
    ("keyword.preproc", Class::Preproc),
    // Keywords and operators (Chroma: Keyword and Operator categories).
    ("keyword", Class::Keyword),
    ("operator", Class::Keyword),
    ("conditional", Class::Keyword),
    ("repeat", Class::Keyword),
    ("include", Class::Keyword),
    ("import", Class::Keyword),
    // Types.
    ("type", Class::Type),
    ("type.builtin", Class::Type),
    ("constructor", Class::ClassName),
    ("class", Class::ClassName),
    ("exception", Class::Exception),
    // Constants.
    ("boolean", Class::LangConst),
    ("constant.builtin", Class::LangConst),
    ("variable.builtin", Class::LangConst),
    ("constant", Class::Number),
    ("number", Class::Number),
    ("float", Class::Number),
    // Functions.
    ("function", Class::Function),
    ("method", Class::Function),
    // Strings.
    ("string", Class::String),
    ("character", Class::String),
    ("string.special.symbol", Class::Number),
    ("string.special.key", Class::Tag),
    ("string.escape", Class::Escape),
    ("escape", Class::Escape),
    ("string.regex", Class::Regexp),
    ("string.regexp", Class::Regexp),
    ("string.special.regex", Class::Regexp),
    // Markup.
    ("tag", Class::Tag),
    ("attribute", Class::Attribute),
    ("property", Class::Property),
    ("field", Class::Property),
    ("variable.member", Class::Property),
    ("label", Class::Property),
    ("text.title", Class::Heading),
    ("markup.heading", Class::Heading),
    ("title", Class::Heading),
    ("text.literal", Class::String),
    ("markup.raw", Class::String),
    ("text.uri", Class::Attribute),
    ("markup.link.url", Class::Attribute),
    ("text.reference", Class::Tag),
    ("markup.link.text", Class::Tag),
    ("diff.plus", Class::Inserted),
    ("markup.inserted", Class::Inserted),
    ("diff.minus", Class::Deleted),
    ("markup.deleted", Class::Deleted),
];

/// The class of the recognized name at `index` in [`NAMES`].
pub(crate) fn class_of(index: usize) -> Class {
    NAMES.get(index).map_or(Class::Plain, |&(_, c)| c)
}

/// The class a capture name maps to, by the same rule as the highlighter.
pub fn class_for_capture(name: &str) -> Class {
    let parts: Vec<&str> = name.split('.').collect();
    let mut best: Option<(usize, Class)> = None;
    for &(recognized, class) in NAMES {
        let mut len = 0;
        if recognized.split('.').all(|p| {
            len += 1;
            parts.contains(&p)
        }) && best.is_none_or(|(l, _)| len > l)
        {
            best = Some((len, class));
        }
    }
    best.map_or(Class::Plain, |(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_in_order() {
        for (i, c) in Class::ALL.iter().enumerate() {
            assert_eq!(*c as usize, i);
        }
    }

    #[test]
    fn captures() {
        for (name, want) in [
            ("keyword", Class::Keyword),
            ("keyword.function", Class::Keyword),
            ("keyword.directive", Class::Preproc),
            ("operator", Class::Keyword),
            ("type.builtin", Class::Type),
            ("constant.builtin", Class::LangConst),
            ("constant.macro", Class::Number),
            ("boolean", Class::LangConst),
            ("function.method.builtin", Class::Function),
            ("function.macro", Class::Function),
            ("constructor", Class::ClassName),
            ("string.escape", Class::Escape),
            ("string.special.regex", Class::Regexp),
            ("string.special.key", Class::Tag),
            ("string.special", Class::String),
            ("comment.documentation", Class::Comment),
            ("punctuation.bracket", Class::Plain),
            ("variable", Class::Plain),
            ("variable.builtin", Class::LangConst),
            ("text.title", Class::Heading),
            ("diff.plus", Class::Inserted),
        ] {
            assert_eq!(class_for_capture(name), want, "{name}");
        }
    }
}
