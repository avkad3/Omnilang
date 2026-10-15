//! Language packs: the per-language keyword tables and word order.
//!
//! A pack maps every language-neutral keyword ([`Kw`]) to a word in one human
//! language. Several packs are active at once, so a single source file may mix
//! English, Hindi and Tamil keywords; the lexer resolves each word to the same
//! `Kw` regardless of script.

use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

/// Language-neutral keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kw {
    Fn,
    Let,
    Var,
    Else,
    Then,
    True,
    False,
    And,
    Or,
    Not,
    Int,
    Bool,
    Str,
    If,
    While,
    Return,
    Print,
}

impl Kw {
    /// Keywords every pack must define (`Then` is optional).
    const REQUIRED: [(Kw, &'static str); 13] = [
        (Kw::Fn, "fn"),
        (Kw::Let, "let"),
        (Kw::Var, "var"),
        (Kw::Else, "else"),
        (Kw::True, "true"),
        (Kw::False, "false"),
        (Kw::And, "and"),
        (Kw::Or, "or"),
        (Kw::Not, "not"),
        (Kw::Int, "int"),
        (Kw::Bool, "bool"),
        (Kw::Str, "str"),
        (Kw::Then, "then"),
    ];
    const STATEMENTS: [(Kw, &'static str); 4] = [
        (Kw::If, "if"),
        (Kw::While, "while"),
        (Kw::Return, "return"),
        (Kw::Print, "print"),
    ];
}

/// Where a statement keyword sits relative to its operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    Prefix,
    Postfix,
}

#[derive(Debug, Clone)]
pub struct Pack {
    pub code: String,
    pub name: String,
    pub script: String,
    /// Native digits 0-9, if the script has them.
    pub digits: Option<[char; 10]>,
    /// Statement terminator used when rendering (`;` is always accepted).
    pub terminator: String,
    /// Conventional name of the entry-point function in this language.
    pub main: String,
    words: HashMap<Kw, String>,
    positions: HashMap<Kw, Position>,
}

impl Pack {
    pub fn word(&self, kw: Kw) -> Option<&str> {
        self.words.get(&kw).map(String::as_str)
    }

    /// Word order of a statement keyword (`If`, `While`, `Return`, `Print`).
    pub fn position(&self, kw: Kw) -> Position {
        self.positions.get(&kw).copied().unwrap_or(Position::Prefix)
    }
}

#[derive(Debug)]
pub struct PackError(pub String);

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackError {}

#[derive(Deserialize)]
struct RawPack {
    pack: RawMeta,
    keywords: HashMap<String, String>,
    statements: HashMap<String, RawStatement>,
}

#[derive(Deserialize)]
struct RawMeta {
    code: String,
    name: String,
    script: String,
    #[serde(default)]
    digits: Option<String>,
    terminator: String,
}

#[derive(Deserialize)]
struct RawStatement {
    word: String,
    position: Position,
}

fn normalize(s: &str) -> String {
    s.nfkc().collect()
}

impl Pack {
    /// Parses a pack from its TOML source.
    pub fn from_toml(src: &str) -> Result<Pack, PackError> {
        let raw: RawPack = toml::from_str(src).map_err(|e| PackError(e.to_string()))?;
        let code = raw.pack.code.clone();
        let err = |msg: String| PackError(format!("pack '{code}': {msg}"));

        let mut keywords = raw.keywords;
        let main = keywords
            .remove("main")
            .ok_or_else(|| err("missing keywords.main".into()))?;
        let mut words = HashMap::new();
        for (kw, key) in Kw::REQUIRED {
            match keywords.remove(key) {
                Some(w) => {
                    words.insert(kw, normalize(w.trim()));
                }
                None if kw == Kw::Then => {}
                None => return Err(err(format!("missing keywords.{key}"))),
            }
        }
        if let Some(extra) = keywords.keys().next() {
            return Err(err(format!("unknown keyword '{extra}'")));
        }

        let mut statements = raw.statements;
        let mut positions = HashMap::new();
        for (kw, key) in Kw::STATEMENTS {
            let st = statements
                .remove(key)
                .ok_or_else(|| err(format!("missing statements.{key}")))?;
            words.insert(kw, normalize(st.word.trim()));
            positions.insert(kw, st.position);
        }
        if let Some(extra) = statements.keys().next() {
            return Err(err(format!("unknown statement '{extra}'")));
        }

        for w in words.values() {
            if w.is_empty() || w.chars().any(char::is_whitespace) {
                return Err(err(format!("keyword {w:?} must be a single word")));
            }
        }

        let digits = match raw.pack.digits {
            None => None,
            Some(d) => {
                let v: Vec<char> = d.chars().collect();
                let arr: [char; 10] = v
                    .try_into()
                    .map_err(|_| err("digits must list exactly 10 characters".into()))?;
                Some(arr)
            }
        };

        Ok(Pack {
            code: raw.pack.code,
            name: raw.pack.name,
            script: raw.pack.script,
            digits,
            terminator: raw.pack.terminator,
            main: normalize(main.trim()),
            words,
            positions,
        })
    }
}

/// The set of active packs plus a reverse index from word to keyword.
#[derive(Debug, Clone)]
pub struct Packs {
    packs: Vec<Pack>,
    lookup: HashMap<String, (Kw, Option<Position>)>,
}

const BUILTIN: [&str; 3] = [
    include_str!("../packs/en.toml"),
    include_str!("../packs/hi.toml"),
    include_str!("../packs/ta.toml"),
];

impl Packs {
    /// All packs that ship with OmniLang.
    pub fn builtin() -> Packs {
        let packs = BUILTIN
            .iter()
            .map(|src| Pack::from_toml(src).expect("built-in pack is valid"))
            .collect();
        Packs::new(packs).expect("built-in packs are consistent")
    }

    /// Builds a registry, rejecting a word that means two different things.
    pub fn new(packs: Vec<Pack>) -> Result<Packs, PackError> {
        let mut lookup: HashMap<String, (Kw, Option<Position>)> = HashMap::new();
        let mut owner: HashMap<String, String> = HashMap::new();
        for p in &packs {
            for (&kw, w) in &p.words {
                let pos = p.positions.get(&kw).copied();
                if let Some(&prev) = lookup.get(w) {
                    if prev != (kw, pos) {
                        return Err(PackError(format!(
                            "word {w:?} means {prev:?} in pack '{}' but {:?} in pack '{}'",
                            owner[w],
                            (kw, pos),
                            p.code
                        )));
                    }
                }
                lookup.insert(w.clone(), (kw, pos));
                owner.entry(w.clone()).or_insert_with(|| p.code.clone());
            }
        }
        Ok(Packs { packs, lookup })
    }

    /// Restricts the registry to the given pack codes (e.g. `["en", "hi"]`).
    pub fn only(&self, codes: &[&str]) -> Result<Packs, PackError> {
        let mut chosen = Vec::new();
        for c in codes {
            let p = self
                .get(c)
                .ok_or_else(|| PackError(format!("unknown language pack '{c}'")))?;
            chosen.push(p.clone());
        }
        Packs::new(chosen)
    }

    pub fn keyword(&self, word: &str) -> Option<Kw> {
        self.lookup.get(word).map(|&(kw, _)| kw)
    }

    /// Word order of this particular keyword word, for statement keywords.
    pub fn keyword_position(&self, word: &str) -> Option<Position> {
        self.lookup.get(word).and_then(|&(_, pos)| pos)
    }

    pub fn get(&self, code: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.code == code)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Pack> {
        self.packs.iter()
    }

    /// Value of a native digit from any active pack.
    pub fn digit_value(&self, c: char) -> Option<u32> {
        if let Some(d) = c.to_digit(10) {
            return Some(d);
        }
        self.packs
            .iter()
            .filter_map(|p| p.digits)
            .find_map(|ds| ds.iter().position(|&d| d == c).map(|i| i as u32))
    }

    /// True if `name` is the entry-point name in any active pack.
    pub fn is_main(&self, name: &str) -> bool {
        self.packs.iter().any(|p| p.main == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_load_and_agree() {
        let p = Packs::builtin();
        assert_eq!(p.keyword("if"), Some(Kw::If));
        assert_eq!(p.keyword("अगर"), Some(Kw::If));
        assert_eq!(p.keyword("என்றால்"), Some(Kw::If));
        assert_eq!(p.keyword("छापो"), Some(Kw::Print));
        assert_eq!(p.keyword("तो"), Some(Kw::Then));
        assert_eq!(p.keyword("hello"), None);
    }

    #[test]
    fn word_order_is_per_pack() {
        let p = Packs::builtin();
        assert_eq!(p.get("en").unwrap().position(Kw::Return), Position::Prefix);
        assert_eq!(p.get("hi").unwrap().position(Kw::Return), Position::Postfix);
        assert_eq!(p.get("ta").unwrap().position(Kw::If), Position::Postfix);
    }

    #[test]
    fn native_digits() {
        let p = Packs::builtin();
        assert_eq!(p.digit_value('७'), Some(7));
        assert_eq!(p.digit_value('௩'), Some(3));
        assert_eq!(p.digit_value('5'), Some(5));
        assert_eq!(p.digit_value('x'), None);
    }

    #[test]
    fn entry_point_names() {
        let p = Packs::builtin();
        assert!(p.is_main("main") && p.is_main("मुख्य") && p.is_main("முதன்மை"));
    }

    #[test]
    fn conflicting_words_are_rejected() {
        let en = Packs::builtin().get("en").unwrap().clone();
        let mut bad = en.clone();
        bad.code = "xx".into();
        bad.words.insert(Kw::Let, "if".into()); // "if" already means If
        assert!(Packs::new(vec![en, bad]).is_err());
    }

    #[test]
    fn missing_keyword_is_reported() {
        let src = include_str!("../packs/en.toml").replace("let = \"let\"\n", "");
        let e = Pack::from_toml(&src).unwrap_err();
        assert!(e.0.contains("keywords.let"), "{e}");
    }
}
