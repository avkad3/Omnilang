//! Editor features as pure functions over a document's text.

use std::collections::{BTreeMap, BTreeSet};

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Diagnostic, DiagnosticSeverity, NumberOrString,
    Position, TextEdit,
};
use omni_core::check::check;
use omni_core::diag::Severity;
use omni_core::glossary::Glossary;
use omni_core::lexer::{lex, Tok};
use omni_core::pack::{Kw, Pack, Packs, Position as WordOrder};
use omni_core::parser::parse;
use omni_core::translit::{is_romanized, transliterate, Script};
use omni_core::view::{render, ViewOptions};
use unicode_ident::is_xid_continue;

use crate::position::LineIndex;

/// Everything about a document the features need.
pub struct Doc<'a> {
    pub text: &'a str,
    pub packs: &'a Packs,
    pub glossary: &'a Glossary,
}

impl Doc<'_> {
    /// Language packs whose keywords appear in the document, most used first.
    pub fn languages(&self) -> Vec<&str> {
        let lexed = lex(self.text, self.packs);
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for t in &lexed.tokens {
            if let Tok::Kw(kw) = t.tok {
                let word = &self.text[t.span.start..t.span.end];
                for p in self.packs.iter() {
                    if p.word(kw) == Some(word) {
                        *counts.entry(p.code.as_str()).or_default() += 1;
                    }
                }
            }
        }
        let mut langs: Vec<(&str, usize)> = counts.into_iter().collect();
        langs.sort_by(|a, b| b.1.cmp(&a.1));
        langs.into_iter().map(|(l, _)| l).collect()
    }

    /// The document's language: its dominant keyword language, or `fallback`
    /// for a document without keywords yet.
    pub fn language<'p>(&'p self, fallback: &'p str) -> &'p str {
        self.languages().first().copied().unwrap_or(fallback)
    }

    fn pack(&self, code: &str) -> &Pack {
        self.packs
            .get(code)
            .or_else(|| self.packs.iter().next())
            .expect("at least one pack")
    }
}

// ---------------------------------------------------------------- diagnostics

pub fn diagnostics(doc: &Doc, message_lang: &str) -> Vec<Diagnostic> {
    let parsed = parse(doc.text, doc.packs);
    let mut diags = parsed.diagnostics;
    if !diags.iter().any(|d| d.severity == Severity::Error) {
        diags.extend(check(&parsed.program, doc.packs, doc.glossary, false).diagnostics);
    }
    let idx = LineIndex::new(doc.text);
    diags
        .iter()
        .map(|d| Diagnostic {
            range: idx.range(d.span.start, d.span.end.max(d.span.start)),
            severity: Some(match d.severity {
                Severity::Error => DiagnosticSeverity::ERROR,
                Severity::Warning => DiagnosticSeverity::WARNING,
            }),
            code: Some(NumberOrString::String(d.code.to_string())),
            source: Some("omni".into()),
            message: d.message(message_lang),
            ..Default::default()
        })
        .collect()
}

// ---------------------------------------------------------------- completion

/// The partial word before the cursor: (byte start, text).
fn word_before(text: &str, offset: usize) -> (usize, &str) {
    let before = &text[..offset];
    let start = before
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_xid_continue(c) || c == '\u{200C}' || c == '\u{200D}')
        .last()
        .map_or(offset, |(i, _)| i);
    (start, &text[start..offset])
}

fn starts_with_ci(candidate: &str, prefix: &str) -> bool {
    candidate.len() >= prefix.len()
        && candidate.is_char_boundary(prefix.len())
        && candidate[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Completion for the word being typed at `pos`, in the document's language
/// (or `typing_lang` for an empty document).
///
/// * Keywords match their native word, their English name, their word in
///   any other language, and common romanized spellings: in a Hindi file,
///   `return`, `lautao` and `திருப்பு` all complete to `लौटाओ`.
/// * Glossary terms match any spelling and insert the document language's.
/// * Names already used in the document match directly or after
///   transliteration (`gin` finds `गिनती`).
/// * A romanized word is offered transliterated (`ginatii` → `गिनती`).
pub fn completion(doc: &Doc, pos: Position, typing_lang: &str) -> Vec<CompletionItem> {
    let idx = LineIndex::new(doc.text);
    let offset = idx.offset(pos);
    let (start, prefix) = word_before(doc.text, offset);
    if prefix.is_empty() {
        return Vec::new();
    }
    let lang = doc.language(typing_lang);
    let pack = doc.pack(lang);
    let script = Script::for_lang(&pack.code);
    let romanized = is_romanized(prefix);
    let transliterated = script.filter(|_| romanized).map(|s| transliterate(prefix, s));
    let range = idx.range(start, offset);

    let mut items: Vec<CompletionItem> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push = |label: String, kind, detail: String, rank: char| {
        if label.is_empty() || label == prefix || !seen.insert(label.clone()) {
            return;
        }
        items.push(CompletionItem {
            label: label.clone(),
            kind: Some(kind),
            detail: Some(detail),
            // The client filters by this; the server has already matched.
            filter_text: Some(prefix.to_string()),
            sort_text: Some(format!("{rank}{label}")),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, label))),
            ..Default::default()
        });
    };

    // Keywords and the entry-point name.
    for kw in Kw::ALL {
        let Some(word) = pack.word(kw) else { continue };
        let mut spellings: Vec<&str> = doc.packs.iter().filter_map(|p| p.word(kw)).collect();
        spellings.extend(pack.aliases(kw).iter().map(String::as_str));
        if spellings.iter().any(|s| starts_with_ci(s, prefix)) {
            push(
                word.to_string(),
                CompletionItemKind::KEYWORD,
                keyword_summary(doc.packs, kw),
                '1',
            );
        }
    }
    let mut main_spellings: Vec<&str> = doc.packs.iter().map(|p| p.main.as_str()).collect();
    main_spellings.extend(pack.main_aliases.iter().map(String::as_str));
    if main_spellings.iter().any(|s| starts_with_ci(s, prefix)) {
        push(
            pack.main.clone(),
            CompletionItemKind::FUNCTION,
            "entry point".into(),
            '1',
        );
    }

    // Names already in the document.
    let lexed = lex(doc.text, doc.packs);
    let names: BTreeSet<&str> = lexed
        .tokens
        .iter()
        .filter(|t| t.span.end != offset) // not the word being typed
        .filter_map(|t| match &t.tok {
            Tok::Ident(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    for name in names {
        let direct = starts_with_ci(name, prefix);
        let via_translit = transliterated
            .as_deref()
            .is_some_and(|t| !t.is_empty() && name.starts_with(t));
        if direct || via_translit {
            push(
                name.to_string(),
                CompletionItemKind::VARIABLE,
                "name in this file".into(),
                '0',
            );
        }
    }

    // Glossary terms in the document's language.
    for spelling in doc.glossary.spellings() {
        if starts_with_ci(spelling, prefix) {
            if let Some(local) = doc.glossary.translate(spelling, lang) {
                push(
                    local.to_string(),
                    CompletionItemKind::VARIABLE,
                    format!("glossary: {spelling}"),
                    '0',
                );
            }
        }
    }

    // The romanized word itself, transliterated.
    if let Some(t) = transliterated {
        push(
            t,
            CompletionItemKind::TEXT,
            format!("{prefix} → {}", pack.name),
            '2',
        );
    }
    items
}

/// "if · अगर · … என்றால்" (postfix words marked with an ellipsis).
fn keyword_summary(packs: &Packs, kw: Kw) -> String {
    packs
        .iter()
        .filter_map(|p| {
            let w = p.word(kw)?;
            let after = is_statement(kw) && p.position(kw) == WordOrder::Postfix;
            Some(if after { format!("… {w}") } else { w.to_string() })
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn is_statement(kw: Kw) -> bool {
    matches!(kw, Kw::If | Kw::While | Kw::Return | Kw::Print)
}

// ---------------------------------------------------------------- hover

/// Hover text for the keyword or name at `pos`.
pub fn hover(doc: &Doc, pos: Position) -> Option<String> {
    let idx = LineIndex::new(doc.text);
    let offset = idx.offset(pos);
    let lexed = lex(doc.text, doc.packs);
    let tok = lexed
        .tokens
        .iter()
        .find(|t| t.span.start <= offset && offset < t.span.end)?;
    match &tok.tok {
        Tok::Kw(kw) => Some(keyword_hover(doc.packs, *kw)),
        Tok::Ident(name) => {
            let mut lines = Vec::new();
            let program = parse(doc.text, doc.packs).program;
            if let Some(f) = program
                .funcs
                .iter()
                .find(|f| doc.glossary.key(&f.name.text) == doc.glossary.key(name))
            {
                let params: Vec<String> = f
                    .params
                    .iter()
                    .map(|p| format!("{}: {}", p.name.text, type_name(p.ty)))
                    .collect();
                let ret = match f.ret {
                    omni_core::ast::Type::Unit => String::new(),
                    t => format!(" -> {}", type_name(t)),
                };
                lines.push(format!(
                    "```omnilang\nfn {}({}){ret}\n```",
                    f.name.text,
                    params.join(", ")
                ));
            }
            let translations: Vec<String> = doc
                .packs
                .iter()
                .filter_map(|p| {
                    doc.glossary
                        .translate(name, &p.code)
                        .map(|t| format!("{}: {t}", p.name))
                })
                .collect();
            if !translations.is_empty() {
                lines.push(format!("Glossary: {}", translations.join(" · ")));
            }
            (!lines.is_empty()).then(|| lines.join("\n\n"))
        }
        _ => None,
    }
}

fn type_name(t: omni_core::ast::Type) -> &'static str {
    match t {
        omni_core::ast::Type::Int => "int",
        omni_core::ast::Type::Bool => "bool",
        omni_core::ast::Type::Str => "str",
        omni_core::ast::Type::Unit => "()",
    }
}

fn keyword_hover(packs: &Packs, kw: Kw) -> String {
    let mut out = format!(
        "**{}**: keyword\n\n| language | written as |\n|---|---|\n",
        packs_word(packs, "en", kw)
    );
    for p in packs.iter() {
        let Some(w) = p.word(kw) else { continue };
        let example = match (kw, p.position(kw)) {
            (Kw::If, WordOrder::Prefix) => match p.word(Kw::Then) {
                Some(t) => format!("{w} x > 0 {t} {{ … }}"),
                None => format!("{w} x > 0 {{ … }}"),
            },
            (Kw::While, WordOrder::Prefix) => format!("{w} x > 0 {{ … }}"),
            (Kw::If | Kw::While, WordOrder::Postfix) => format!("x > 0 {w} {{ … }}"),
            (Kw::Return | Kw::Print, WordOrder::Prefix) => format!("{w} x{}", p.terminator),
            (Kw::Return | Kw::Print, WordOrder::Postfix) => format!("x {w}{}", p.terminator),
            _ => w.to_string(),
        };
        out.push_str(&format!("| {} | `{example}` |\n", p.name));
    }
    out
}

fn packs_word<'a>(packs: &'a Packs, code: &str, kw: Kw) -> &'a str {
    packs.get(code).and_then(|p| p.word(kw)).unwrap_or("?")
}

// ---------------------------------------------------------------- formatting

/// Canonical formatting in the document's own language. Documents that
/// deliberately mix languages, or don't parse, are left alone.
pub fn format(doc: &Doc, fallback_lang: &str) -> Option<Vec<TextEdit>> {
    let parsed = parse(doc.text, doc.packs);
    if parsed.diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return None;
    }
    let langs = doc.languages();
    if langs.len() > 1 {
        return None;
    }
    let pack = doc.pack(langs.first().copied().unwrap_or(fallback_lang));
    let opts = ViewOptions {
        pack,
        packs: doc.packs,
        glossary: doc.glossary,
        native_digits: false,
        translate_names: false,
    };
    let formatted = render(&parsed.program, &opts);
    if formatted == doc.text {
        return Some(Vec::new());
    }
    let idx = LineIndex::new(doc.text);
    Some(vec![TextEdit::new(
        lsp_types::Range::new(Position::new(0, 0), idx.end()),
        formatted,
    )])
}

/// The whole program rendered in `lang`, names translated.
pub fn view(doc: &Doc, lang: &str) -> Option<String> {
    let parsed = parse(doc.text, doc.packs);
    if parsed.diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return None;
    }
    let opts = ViewOptions {
        pack: doc.packs.get(lang)?,
        packs: doc.packs,
        glossary: doc.glossary,
        native_digits: false,
        translate_names: true,
    };
    Some(render(&parsed.program, &opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc<'a>(text: &'a str, packs: &'a Packs, glossary: &'a Glossary) -> Doc<'a> {
        Doc {
            text,
            packs,
            glossary,
        }
    }

    fn at_end(text: &str) -> Position {
        LineIndex::new(text).end()
    }

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn detects_document_language() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        assert_eq!(doc("कार्य मुख्य() { 1 छापो। }", &packs, &g).language("en"), "hi");
        assert_eq!(doc("x = 1;", &packs, &g).language("ta"), "ta");
    }

    #[test]
    fn english_name_completes_to_native_keyword() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        let text = "कार्य मुख्य() { 5 ret";
        let items = completion(&doc(text, &packs, &g), at_end(text), "en");
        assert_eq!(labels(&items)[0], "लौटाओ", "{:?}", labels(&items));
    }

    #[test]
    fn romanized_spelling_completes_keyword_and_transliterates() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        let text = "செயல்பாடு முதன்மை() { x > 1 endr";
        let l = completion(&doc(text, &packs, &g), at_end(text), "en");
        assert!(labels(&l).contains(&"என்றால்"), "{:?}", labels(&l));

        let text = "कार्य मुख्य() { ginatii";
        let l = completion(&doc(text, &packs, &g), at_end(text), "en");
        assert!(labels(&l).contains(&"गिनती"), "{:?}", labels(&l));
    }

    #[test]
    fn existing_names_match_after_transliteration() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        let text = "कार्य मुख्य() { चर गिनती = 1। gin";
        let l = completion(&doc(text, &packs, &g), at_end(text), "en");
        assert_eq!(labels(&l)[0], "गिनती", "{:?}", labels(&l));
    }

    #[test]
    fn glossary_terms_complete_in_document_language() {
        let packs = Packs::builtin();
        let g = Glossary::from_toml("[[term]]\nen = \"total\"\nhi = \"कुल\"\n").unwrap();
        let text = "कार्य मुख्य() { tot";
        let l = completion(&doc(text, &packs, &g), at_end(text), "en");
        assert!(labels(&l).contains(&"कुल"), "{:?}", labels(&l));
    }

    #[test]
    fn completion_replaces_exactly_the_typed_word() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        let text = "कार्य मुख्य() { 5 ret";
        let item = &completion(&doc(text, &packs, &g), at_end(text), "en")[0];
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        // "कार्य मुख्य() { 5 " is 18 UTF-16 units (each word is 5 code points)
        // but 36 bytes: the edit must use the former.
        assert_eq!(edit.range.start, Position::new(0, 18));
        assert_eq!(edit.range.end, Position::new(0, 21));
    }

    #[test]
    fn hover_shows_every_language_with_word_order() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        let text = "fn main() { if true { } }";
        let h = hover(&doc(text, &packs, &g), Position::new(0, 13)).unwrap();
        assert!(h.contains("`अगर x > 0 तो { … }`"), "{h}");
        assert!(h.contains("`x > 0 என்றால் { … }`"), "{h}");
    }

    #[test]
    fn diagnostics_are_translated_and_positioned() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        let text = "कार्य मुख्य() {\n    गिनती छापो।\n}\n";
        let d = diagnostics(&doc(text, &packs, &g), "hi");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("परिभाषित नहीं"), "{}", d[0].message);
        assert_eq!(d[0].range.start, Position::new(1, 4));
        assert_eq!(d[0].range.end, Position::new(1, 9));
    }

    #[test]
    fn formatting_normalizes_but_never_mixes_or_renames() {
        let packs = Packs::builtin();
        let g = Glossary::default();
        let messy = "fn main(){print 1;}";
        let edits = format(&doc(messy, &packs, &g), "en").unwrap();
        assert_eq!(edits[0].new_text, "fn main() {\n    print 1;\n}\n");

        let mixed = "fn main() { 1 छापो। }";
        assert!(format(&doc(mixed, &packs, &g), "en").is_none());
    }
}
