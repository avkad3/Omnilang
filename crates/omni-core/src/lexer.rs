//! Script-agnostic lexer.
//!
//! Identifiers follow Unicode UAX #31 (XID_Start / XID_Continue), are
//! NFKC-normalised (as in Python) so that equivalent spellings, including
//! fullwidth and ligature forms, are the same name, and are checked by
//! [`crate::security`]. A word that any active language
//! pack defines becomes a keyword token, whatever its script. Numbers may be
//! written with ASCII or native (Devanagari, Tamil) digits.

use unicode_ident::{is_xid_continue, is_xid_start};
use unicode_normalization::UnicodeNormalization;

use crate::diag::{Diagnostic, Span};
use crate::pack::{Kw, Packs};
use crate::security::{self, ZWJ, ZWNJ};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Kw(Kw),
    Int(i64),
    Str(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Arrow,
    Semi,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub span: Span,
    /// Comment text without the leading `//`.
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
    pub diagnostics: Vec<Diagnostic>,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    packs: &'a Packs,
    out: Lexed,
    idents: Vec<(String, Span)>,
}

pub fn lex(src: &str, packs: &Packs) -> Lexed {
    let mut lx = Lexer {
        src,
        pos: 0,
        packs,
        out: Lexed::default(),
        idents: Vec::new(),
    };
    lx.out.diagnostics.extend(security::scan_source(src));
    if src.starts_with('\u{FEFF}') {
        lx.pos = '\u{FEFF}'.len_utf8();
    }
    lx.run();
    let conf = security::check_confusables(&lx.idents, packs);
    lx.out.diagnostics.extend(conf);
    lx.out
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek2(&self) -> Option<char> {
        let mut it = self.src[self.pos..].chars();
        it.next();
        it.next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        self.out.tokens.push(Token {
            tok,
            span: Span::new(start, self.pos),
        });
    }

    fn error(&mut self, d: Diagnostic) {
        self.out.diagnostics.push(d);
    }

    fn run(&mut self) {
        while let Some(c) = self.peek() {
            let start = self.pos;
            if c.is_whitespace() {
                self.bump();
            } else if c == '/' && self.peek2() == Some('/') {
                self.comment(start);
            } else if c == '_' || is_xid_start(c) {
                self.ident(start);
            } else if self.packs.digit_value(c).is_some() {
                self.number(start);
            } else if c == '"' {
                self.string(start);
            } else {
                self.bump();
                let two = |lx: &mut Self, next: char, yes: Tok, no: Tok| {
                    if lx.peek() == Some(next) {
                        lx.bump();
                        yes
                    } else {
                        no
                    }
                };
                let tok = match c {
                    '(' => Tok::LParen,
                    ')' => Tok::RParen,
                    '{' => Tok::LBrace,
                    '}' => Tok::RBrace,
                    ',' => Tok::Comma,
                    ':' => Tok::Colon,
                    // Danda (।) and double danda (॥) end statements in Hindi.
                    ';' | '।' | '॥' => Tok::Semi,
                    '+' => Tok::Plus,
                    '-' => two(self, '>', Tok::Arrow, Tok::Minus),
                    '*' => Tok::Star,
                    '/' => Tok::Slash,
                    '%' => Tok::Percent,
                    '=' => two(self, '=', Tok::EqEq, Tok::Assign),
                    '!' => two(self, '=', Tok::NotEq, Tok::Bang),
                    '<' => two(self, '=', Tok::Le, Tok::Lt),
                    '>' => two(self, '=', Tok::Ge, Tok::Gt),
                    '&' if self.peek() == Some('&') => {
                        self.bump();
                        Tok::AndAnd
                    }
                    '|' if self.peek() == Some('|') => {
                        self.bump();
                        Tok::OrOr
                    }
                    _ => {
                        // Bidi/invisible characters were already reported by
                        // the security scan; don't report them twice.
                        if security::scan_source(&c.to_string()).is_empty() {
                            let shown = if c.is_control() || c == ZWJ || c == ZWNJ {
                                security::codepoint(c)
                            } else {
                                c.to_string()
                            };
                            self.error(
                                Diagnostic::error("E0101", Span::new(start, self.pos)).arg("ch", shown),
                            );
                        }
                        continue;
                    }
                };
                self.push(tok, start);
            }
        }
        let end = self.src.len();
        self.out.tokens.push(Token {
            tok: Tok::Eof,
            span: Span::new(end, end),
        });
    }

    fn comment(&mut self, start: usize) {
        let end = self.src[start..].find('\n').map_or(self.src.len(), |i| start + i);
        let text = self.src[start + 2..end].trim_end_matches('\r').to_string();
        self.pos = end;
        self.out.comments.push(Comment {
            span: Span::new(start, end),
            text,
        });
    }

    fn ident(&mut self, start: usize) {
        while let Some(c) = self.peek() {
            if is_xid_continue(c) || c == ZWJ || c == ZWNJ {
                self.bump();
            } else {
                break;
            }
        }
        let span = Span::new(start, self.pos);
        let raw = &self.src[start..self.pos];
        let name: String = raw.nfkc().collect();
        if let Some(kw) = self.packs.keyword(&name) {
            self.push(Tok::Kw(kw), start);
            return;
        }
        // Check each distinct name once, at its first occurrence.
        if !self.idents.iter().any(|(n, _)| *n == name) {
            for d in security::check_identifier(&name, span) {
                self.error(d);
            }
        }
        self.idents.push((name.clone(), span));
        self.push(Tok::Ident(name), start);
    }

    fn number(&mut self, start: usize) {
        let mut value: Option<i64> = Some(0);
        let mut kind: Option<bool> = None; // Some(true) = ASCII digits
        let mut mixed = false;
        while let Some(c) = self.peek() {
            let Some(d) = self.packs.digit_value(c) else {
                break;
            };
            let ascii = c.is_ascii_digit();
            match kind {
                None => kind = Some(ascii),
                Some(k) if k != ascii => mixed = true,
                _ => {}
            }
            self.bump();
            value = value
                .and_then(|v| v.checked_mul(10))
                .and_then(|v| v.checked_add(d as i64));
        }
        // A number glued to letters ("12ab") is a typo, not two tokens.
        while let Some(c) = self.peek() {
            if is_xid_continue(c) {
                self.bump();
            } else {
                break;
            }
        }
        let span = Span::new(start, self.pos);
        let text = &self.src[start..self.pos];
        if text.chars().any(|c| self.packs.digit_value(c).is_none()) {
            let bad = text
                .chars()
                .find(|&c| self.packs.digit_value(c).is_none())
                .unwrap();
            self.error(Diagnostic::error("E0101", span).arg("ch", bad.to_string()));
            self.push(Tok::Int(0), start);
            return;
        }
        if mixed {
            self.error(Diagnostic::error("E0105", span).arg("text", text));
        }
        match value {
            Some(v) => self.push(Tok::Int(v), start),
            None => {
                self.error(Diagnostic::error("E0104", span).arg("text", text));
                self.push(Tok::Int(0), start);
            }
        }
    }

    fn string(&mut self, start: usize) {
        self.bump(); // opening quote
        let mut value = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => {
                    self.error(Diagnostic::error("E0102", Span::new(start, self.pos)));
                    break;
                }
                Some('"') => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    let esc_start = self.pos;
                    self.bump();
                    match self.bump() {
                        Some('n') => value.push('\n'),
                        Some('t') => value.push('\t'),
                        Some('\\') => value.push('\\'),
                        Some('"') => value.push('"'),
                        Some(other) => {
                            self.error(
                                Diagnostic::error("E0103", Span::new(esc_start, self.pos))
                                    .arg("ch", other.to_string()),
                            );
                        }
                        None => {}
                    }
                }
                Some(c) => {
                    self.bump();
                    value.push(c);
                }
            }
        }
        self.push(Tok::Str(value), start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let l = lex(src, &Packs::builtin());
        assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
        l.tokens.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn same_program_three_scripts_same_keywords() {
        let en = toks("if x > 1 { print x; }");
        assert_eq!(en[0], Tok::Kw(Kw::If));
        let hi = toks("अगर x > 1 तो { x छापो। }");
        assert_eq!(hi[0], Tok::Kw(Kw::If));
        assert!(hi.contains(&Tok::Kw(Kw::Then)) && hi.contains(&Tok::Kw(Kw::Print)));
        assert!(hi.contains(&Tok::Semi));
        let ta = toks("x > 1 என்றால் { x அச்சிடு; }");
        assert_eq!(ta[3], Tok::Kw(Kw::If));
    }

    #[test]
    fn indic_identifiers_and_native_digits() {
        assert_eq!(
            toks("गिनती = ४२;"),
            vec![
                Tok::Ident("गिनती".into()),
                Tok::Assign,
                Tok::Int(42),
                Tok::Semi,
                Tok::Eof
            ]
        );
        assert_eq!(toks("௧௦"), vec![Tok::Int(10), Tok::Eof]);
    }

    #[test]
    fn nfc_makes_equivalent_spellings_one_name() {
        // U+0958 (क़ precomposed) and U+0915 U+093C (क + nukta) are canonically
        // equivalent; both must lex to the same identifier.
        let a = toks("\u{958}");
        let b = toks("\u{915}\u{93C}");
        assert_eq!(a, b);
    }

    #[test]
    fn compatibility_forms_fold_to_plain_letters() {
        // Fullwidth "ｌｅｔ" is the keyword `let`: it behaves exactly as it looks.
        assert_eq!(toks("\u{FF4C}\u{FF45}\u{FF54}")[0], Tok::Kw(Kw::Let));
    }

    #[test]
    fn operators_strings_and_comments() {
        let l = lex(
            "a -> b == c != d <= e >= f && g || !h // note\n\"x\\n\"",
            &Packs::builtin(),
        );
        assert!(l.diagnostics.is_empty());
        assert_eq!(l.comments.len(), 1);
        assert_eq!(l.comments[0].text, " note");
        assert!(l.tokens.iter().any(|t| t.tok == Tok::Str("x\n".into())));
        assert!(l.tokens.iter().any(|t| t.tok == Tok::Arrow));
    }

    #[test]
    fn lexing_errors() {
        let codes = |src: &str| -> Vec<&'static str> {
            lex(src, &Packs::builtin())
                .diagnostics
                .iter()
                .map(|d| d.code)
                .collect()
        };
        assert_eq!(codes("\"open"), ["E0102"]);
        assert_eq!(codes("\"\\q\""), ["E0103"]);
        assert_eq!(codes("99999999999999999999"), ["E0104"]);
        assert_eq!(codes("1२"), ["E0105"]);
        assert_eq!(codes("a $ b"), ["E0101"]);
        assert_eq!(codes("p\u{430}ypal"), ["E0004"]);
        assert_eq!(codes("p\u{430}ypal + p\u{430}ypal"), ["E0004"]); // once per name
        assert_eq!(codes("x = 1; // \u{202E}"), ["E0001"]);
    }
}
