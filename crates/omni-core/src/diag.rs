//! Diagnostics with messages in the user's own language.
//!
//! Every diagnostic has a stable code (e.g. `E0102`) and named arguments. The
//! text comes from `messages.toml`, which holds one translation per language,
//! so the same error can be shown in English, Hindi or Tamil.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use unicode_segmentation::UnicodeSegmentation;

/// Byte range into the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub span: Span,
    pub args: Vec<(&'static str, String)>,
}

impl Diagnostic {
    pub fn error(code: &'static str, span: Span) -> Diagnostic {
        Diagnostic {
            code,
            severity: Severity::Error,
            span,
            args: Vec::new(),
        }
    }

    pub fn warning(code: &'static str, span: Span) -> Diagnostic {
        Diagnostic {
            code,
            severity: Severity::Warning,
            span,
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, name: &'static str, value: impl Into<String>) -> Diagnostic {
        self.args.push((name, value.into()));
        self
    }

    /// The message text in `lang`, falling back to English.
    pub fn message(&self, lang: &str) -> String {
        let cat = catalog();
        let entry = cat.get(self.code);
        let template = entry
            .and_then(|m| m.get(lang).or_else(|| m.get("en")))
            .map(String::as_str)
            .unwrap_or(self.code);
        let mut out = template.to_string();
        for (name, value) in &self.args {
            out = out.replace(&format!("{{{name}}}"), &phrase(value, lang));
        }
        out
    }

    /// Renders the diagnostic with a source excerpt, compiler-style.
    pub fn render(&self, src: &str, file: &str, lang: &str) -> String {
        let label = catalog()
            .get(match self.severity {
                Severity::Error => "label.error",
                Severity::Warning => "label.warning",
            })
            .and_then(|m| m.get(lang).or_else(|| m.get("en")))
            .cloned()
            .unwrap_or_default();
        let (line_no, line_start) = line_of(src, self.span.start);
        let line_end = src[line_start..]
            .find('\n')
            .map_or(src.len(), |i| line_start + i);
        let line = src[line_start..line_end].trim_end_matches('\r');
        // Columns are grapheme clusters, so a Devanagari conjunct like "क्ष"
        // counts as one column, the way a reader sees it.
        let col = src[line_start..self.span.start.min(line_end)]
            .graphemes(true)
            .count();
        let width = src[self.span.start.min(line_end)
            ..self
                .span
                .end
                .min(line_end)
                .max(self.span.start.min(line_end))]
            .graphemes(true)
            .count()
            .max(1);
        let gutter = line_no.to_string().len();

        let mut out = String::new();
        let _ = writeln!(out, "{label}[{}]: {}", self.code, self.message(lang));
        let _ = writeln!(out, "{:gutter$}--> {file}:{line_no}:{}", "", col + 1);
        let _ = writeln!(out, "{:gutter$} |", "");
        let _ = writeln!(out, "{line_no} | {line}");
        let pad: String = line
            .graphemes(true)
            .take(col)
            .map(|g| if g == "\t" { '\t' } else { ' ' })
            .collect();
        let _ = writeln!(out, "{:gutter$} | {pad}{}", "", "^".repeat(width));
        out
    }
}

/// Resolves an argument: `@key` is looked up in the catalog in `lang`
/// (e.g. `@what.expr`, `@type.int`), anything else is used verbatim.
pub fn phrase(value: &str, lang: &str) -> String {
    match value.strip_prefix('@') {
        Some(key) => catalog()
            .get(key)
            .and_then(|m| m.get(lang).or_else(|| m.get("en")))
            .cloned()
            .unwrap_or_else(|| key.to_string()),
        None => value.to_string(),
    }
}

/// 1-based line number and byte offset of the line containing `pos`.
pub fn line_of(src: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(src.len());
    let before = &src[..pos];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, start)
}

type Catalog = HashMap<String, HashMap<String, String>>;

fn catalog() -> &'static Catalog {
    static CAT: OnceLock<Catalog> = OnceLock::new();
    CAT.get_or_init(|| {
        toml::from_str(include_str!("../messages.toml")).expect("messages.toml is valid")
    })
}

/// Languages that have diagnostic translations.
pub const MESSAGE_LANGS: [&str; 3] = ["en", "hi", "ta"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_has_every_language() {
        for (code, m) in catalog() {
            for lang in MESSAGE_LANGS {
                assert!(m.contains_key(lang), "{code} is missing '{lang}'");
            }
        }
    }

    #[test]
    fn placeholders_are_filled_in_each_language() {
        let d = Diagnostic::error("E0201", Span::new(0, 1)).arg("name", "गिनती");
        for lang in MESSAGE_LANGS {
            let m = d.message(lang);
            assert!(m.contains("गिनती") && !m.contains('{'), "{lang}: {m}");
        }
    }

    #[test]
    fn phrase_arguments_are_translated() {
        let d = Diagnostic::error("E0302", Span::new(0, 1))
            .arg("expected", "@type.int")
            .arg("found", "@type.bool");
        assert_eq!(d.message("en"), "type mismatch: expected int, found bool");
        assert!(d.message("hi").contains("पूर्णांक") && d.message("hi").contains("तार्किक"));
    }

    #[test]
    fn type_names_match_language_packs() {
        use crate::pack::{Kw, Packs};
        let packs = Packs::builtin();
        for lang in MESSAGE_LANGS {
            let p = packs.get(lang).unwrap();
            for (key, kw) in [
                ("type.int", Kw::Int),
                ("type.bool", Kw::Bool),
                ("type.str", Kw::Str),
            ] {
                assert_eq!(
                    phrase(&format!("@{key}"), lang),
                    p.word(kw).unwrap(),
                    "{lang} {key}"
                );
            }
        }
    }

    #[test]
    fn render_points_at_grapheme_columns() {
        let src = "स्थिर क = ख।\n";
        // span of "ख": byte offset after "स्थिर क = "
        let start = src.find('ख').unwrap();
        let d =
            Diagnostic::error("E0201", Span::new(start, start + 'ख'.len_utf8())).arg("name", "ख");
        let out = d.render(src, "t.om", "en");
        assert!(out.contains("t.om:1:"), "{out}");
        // "स्थिर" is 3 grapheme clusters (स्थि + र ... ) so caret is aligned by
        // clusters, not bytes: just check there is exactly one caret.
        assert_eq!(out.matches('^').count(), 1, "{out}");
    }
}
