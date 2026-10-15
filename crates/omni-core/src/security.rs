//! Source-level security checks for multi-script code.
//!
//! Allowing many scripts in one language opens the door to two known attack
//! classes, so OmniLang treats them as compile errors rather than lint hints:
//!
//! * **Trojan Source** (CVE-2021-42574): bidirectional control characters
//!   reorder how code is *displayed* without changing how it is *parsed*.
//! * **Homoglyphs**: `pаypal` with a Cyrillic `а` looks identical to the
//!   Latin spelling but is a different name. Names must be single-script, and
//!   names that look alike (same UTS #39 "skeleton") are reported.

use std::collections::HashMap;

use unicode_normalization::char::canonical_combining_class;
use unicode_script::{Script, UnicodeScript};
use unicode_security::{skeleton, MixedScript};

use crate::diag::{Diagnostic, Span};
use crate::pack::Packs;

pub const ZWNJ: char = '\u{200C}';
pub const ZWJ: char = '\u{200D}';

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' | '\u{180E}' | '\u{2061}'..='\u{2064}'
    )
}

pub fn codepoint(c: char) -> String {
    format!("U+{:04X}", c as u32)
}

/// Rejects bidi controls and invisible characters anywhere in the file,
/// including inside strings and comments, where Trojan Source attacks hide.
pub fn scan_source(src: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (i, c) in src.char_indices() {
        let span = Span::new(i, i + c.len_utf8());
        if is_bidi_control(c) {
            out.push(Diagnostic::error("E0001", span).arg("cp", codepoint(c)));
        } else if is_invisible(c) && !(i == 0 && c == '\u{FEFF}') {
            out.push(Diagnostic::error("E0002", span).arg("cp", codepoint(c)));
        }
    }
    out
}

/// True if `c` is a virama (halant / pulli): the only place a zero-width
/// joiner or non-joiner legitimately appears inside an Indic word.
pub fn is_virama(c: char) -> bool {
    canonical_combining_class(c) == 9
}

/// Checks a single identifier (already NFKC-normalised).
pub fn check_identifier(name: &str, span: Span) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if (c == ZWJ || c == ZWNJ) && !prev.is_some_and(is_virama) {
            out.push(Diagnostic::error("E0003", span).arg("cp", codepoint(c)));
        }
        prev = Some(c);
    }
    if !name.is_single_script() {
        let mut scripts: Vec<String> = Vec::new();
        for c in name.chars() {
            let s = c.script();
            if s != Script::Common && s != Script::Inherited {
                let n = s.full_name().to_string();
                if !scripts.contains(&n) {
                    scripts.push(n);
                }
            }
        }
        out.push(
            Diagnostic::error("E0004", span)
                .arg("name", name)
                .arg("scripts", scripts.join(" + ")),
        );
    }
    out
}

/// Reports distinct names in one file that render (nearly) identically, and
/// names that imitate a keyword.
pub fn check_confusables(idents: &[(String, Span)], packs: &Packs) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, &str> = HashMap::new();
    let mut keyword_skeletons: HashMap<String, String> = HashMap::new();
    for p in packs.iter() {
        for word in all_words(p) {
            keyword_skeletons.insert(skeleton(&word).collect(), word);
        }
    }

    let mut reported: Vec<(String, String)> = Vec::new();
    for (name, span) in idents {
        let sk: String = skeleton(name).collect();
        if let Some(&other) = seen.get(&sk) {
            if other != name && !reported.contains(&(other.to_string(), name.clone())) {
                reported.push((other.to_string(), name.clone()));
                out.push(
                    Diagnostic::warning("W0005", *span)
                        .arg("name", name)
                        .arg("other", other),
                );
            }
        } else {
            seen.insert(sk.clone(), name);
        }
        if let Some(kw) = keyword_skeletons.get(&sk) {
            if kw != name {
                out.push(
                    Diagnostic::warning("W0006", *span)
                        .arg("name", name)
                        .arg("keyword", kw),
                );
            }
        }
    }
    out
}

fn all_words(p: &crate::pack::Pack) -> Vec<String> {
    use crate::pack::Kw::*;
    [
        Fn, Let, Var, Else, Then, True, False, And, Or, Not, Int, Bool, Str, If, While, Return, Print,
    ]
    .into_iter()
    .filter_map(|k| p.word(k).map(str::to_string))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(d: &[Diagnostic]) -> Vec<&str> {
        d.iter().map(|d| d.code).collect()
    }

    #[test]
    fn trojan_source_bidi_controls_are_rejected_even_in_comments() {
        let src = "let x = 1; // \u{202E} } \u{2066}\n";
        assert_eq!(codes(&scan_source(src)), ["E0001", "E0001"]);
    }

    #[test]
    fn invisible_characters_are_rejected_but_leading_bom_is_fine() {
        assert_eq!(codes(&scan_source("\u{FEFF}let x = 1;")), Vec::<&str>::new());
        assert_eq!(codes(&scan_source("let a\u{200B}b = 1;")), ["E0002"]);
    }

    #[test]
    fn zwj_is_allowed_only_after_virama() {
        // Devanagari "क्‍ष" (ka + virama + ZWJ + ssa) is legitimate.
        assert!(check_identifier("क\u{94D}\u{200D}ष", Span::default()).is_empty());
        // ZWJ between two Latin letters is an invisible trick.
        assert_eq!(codes(&check_identifier("a\u{200D}b", Span::default())), ["E0003"]);
    }

    #[test]
    fn mixed_script_identifiers_are_rejected() {
        let d = check_identifier("p\u{430}ypal", Span::default()); // Cyrillic а
        assert_eq!(codes(&d), ["E0004"]);
        assert!(
            d[0].message("en").contains("Latin + Cyrillic"),
            "{}",
            d[0].message("en")
        );
        // Digits and underscores are script-neutral.
        assert!(check_identifier("गिनती_2", Span::default()).is_empty());
        assert!(check_identifier("எண்ணிக்கை", Span::default()).is_empty());
    }

    #[test]
    fn lookalike_names_are_reported() {
        let packs = Packs::builtin();
        let ids = vec![
            ("scope".to_string(), Span::default()),
            ("\u{455}\u{441}\u{43E}\u{440}\u{435}".to_string(), Span::default()), // Cyrillic "ѕсоре"
        ];
        assert_eq!(codes(&check_confusables(&ids, &packs)), ["W0005"]);
    }

    #[test]
    fn keyword_imitation_is_reported() {
        // "retum" is plain single-script Latin, but "rn" and "m" are a classic
        // lookalike pair, so it reads as the keyword `return`.
        let packs = Packs::builtin();
        let ids = vec![("retum".to_string(), Span::default())];
        assert_eq!(codes(&check_confusables(&ids, &packs)), ["W0006"]);
    }
}
