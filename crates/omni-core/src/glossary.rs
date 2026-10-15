//! Glossaries: translations of *names*, not just keywords.
//!
//! Keywords are a few dozen words; a real program has hundreds of names. A
//! project glossary lists each concept once with its name in every language:
//!
//! ```toml
//! [[term]]
//! en = "count"
//! hi = "गिनती"
//! ta = "எண்ணிக்கை"
//! ```
//!
//! The compiler treats every spelling of a term as the same symbol, so a
//! variable declared as `गिनती` can be used as `count`, and the multilingual
//! view renders each name in the reader's language.

use std::collections::HashMap;

use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use crate::pack::PackError;

#[derive(Debug, Clone, Default)]
pub struct Glossary {
    terms: Vec<HashMap<String, String>>,
    index: HashMap<String, usize>,
}

#[derive(Deserialize)]
struct RawGlossary {
    #[serde(default)]
    term: Vec<HashMap<String, String>>,
}

impl Glossary {
    pub fn from_toml(src: &str) -> Result<Glossary, PackError> {
        let raw: RawGlossary = toml::from_str(src).map_err(|e| PackError(e.to_string()))?;
        let mut g = Glossary::default();
        for (i, term) in raw.term.into_iter().enumerate() {
            let mut norm = HashMap::new();
            for (lang, name) in term {
                let name: String = name.trim().nfkc().collect();
                if let Some(&other) = g.index.get(&name) {
                    if other != i {
                        return Err(PackError(format!(
                            "glossary: '{name}' appears in two different terms"
                        )));
                    }
                }
                g.index.insert(name.clone(), i);
                norm.insert(lang, name);
            }
            if norm.is_empty() {
                return Err(PackError(format!("glossary: term {} is empty", i + 1)));
            }
            g.terms.push(norm);
        }
        Ok(g)
    }

    /// The language-independent key for a name: `#<term>` for glossary
    /// names, the name itself otherwise. Two spellings of one term share a key.
    pub fn key(&self, name: &str) -> String {
        match self.index.get(name) {
            Some(i) => format!("#{i}"),
            None => name.to_string(),
        }
    }

    /// The spelling of `name` in `lang`, if the glossary has one.
    pub fn translate(&self, name: &str, lang: &str) -> Option<&str> {
        let i = *self.index.get(name)?;
        self.terms[i].get(lang).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
        [[term]]
        en = "count"
        hi = "गिनती"
        ta = "எண்ணிக்கை"

        [[term]]
        en = "total"
        hi = "कुल"
    "#;

    #[test]
    fn spellings_share_a_key() {
        let g = Glossary::from_toml(SRC).unwrap();
        assert_eq!(g.key("count"), g.key("गिनती"));
        assert_eq!(g.key("गिनती"), g.key("எண்ணிக்கை"));
        assert_ne!(g.key("count"), g.key("total"));
        assert_eq!(g.key("other"), "other");
    }

    #[test]
    fn translates_between_languages() {
        let g = Glossary::from_toml(SRC).unwrap();
        assert_eq!(g.translate("count", "ta"), Some("எண்ணிக்கை"));
        assert_eq!(g.translate("कुल", "en"), Some("total"));
        assert_eq!(g.translate("कुल", "ta"), None);
        assert_eq!(g.translate("unknown", "hi"), None);
    }

    #[test]
    fn a_name_in_two_terms_is_rejected() {
        let bad = "[[term]]\nen = \"x\"\n[[term]]\nhi = \"x\"\n";
        assert!(Glossary::from_toml(bad).is_err());
    }
}
