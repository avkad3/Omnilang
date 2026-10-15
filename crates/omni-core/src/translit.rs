//! Romanized typing for Devanagari and Tamil.
//!
//! Typing Indic scripts on a standard keyboard is slow, so editors offer a
//! transliteration: `agar` → `अगर`, `ninaivagam` → `நினைவகம்`. The schemes
//! here are deterministic and loosely follow ITRANS (Hindi) and common
//! Tamil romanization. Keywords don't depend on getting a spelling exactly
//! right: the language server also offers them by English name.
//!
//! Hindi rules:
//! * A consonant takes the inherent vowel `a` unless another vowel follows.
//! * A consonant followed by another consonant gets a virama, forming a
//!   conjunct (`sthir` → `स्थिर`).
//! * A word-final consonant keeps its inherent vowel, as Hindi is written
//!   (`agar` → `अगर`). `.h` forces a virama.
//!
//! Tamil has no inherent-vowel deletion in writing, so a word-final
//! consonant gets a pulli (`seyal` → `செயல்`).

/// A transliteration target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Devanagari,
    Tamil,
}

impl Script {
    /// Script for a language-pack code (`hi`, `ta`).
    pub fn for_lang(code: &str) -> Option<Script> {
        match code {
            "hi" => Some(Script::Devanagari),
            "ta" => Some(Script::Tamil),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Unit {
    /// Independent form, dependent sign (None for the inherent `a`).
    Vowel(&'static str, Option<&'static str>),
    Consonant(&'static str),
    /// Anusvara, visarga, etc.: attaches to the preceding syllable.
    Mark(&'static str),
    /// Explicit virama/pulli.
    Virama,
    /// Produces nothing; separates sequences (`a_i` → `अइ`).
    Break,
}

use Unit::*;

/// (romanization, unit), longest keys first so matching is greedy.
const HINDI: &[(&str, Unit)] = &[
    // vowels
    ("RRi", Vowel("ऋ", Some("ृ"))),
    ("aa", Vowel("आ", Some("ा"))),
    ("ai", Vowel("ऐ", Some("ै"))),
    ("au", Vowel("औ", Some("ौ"))),
    ("ii", Vowel("ई", Some("ी"))),
    ("ee", Vowel("ई", Some("ी"))),
    ("uu", Vowel("ऊ", Some("ू"))),
    ("oo", Vowel("ऊ", Some("ू"))),
    ("Ri", Vowel("ऋ", Some("ृ"))),
    ("a", Vowel("अ", None)),
    ("A", Vowel("आ", Some("ा"))),
    ("i", Vowel("इ", Some("ि"))),
    ("I", Vowel("ई", Some("ी"))),
    ("u", Vowel("उ", Some("ु"))),
    ("U", Vowel("ऊ", Some("ू"))),
    ("e", Vowel("ए", Some("े"))),
    ("o", Vowel("ओ", Some("ो"))),
    // marks
    (".n", Mark("ं")),
    (".N", Mark("ँ")),
    (".h", Virama),
    ("M", Mark("ं")),
    ("H", Mark("ः")),
    ("_", Break),
    // conjuncts with their own common spellings
    ("ksh", Consonant("क्ष")),
    ("j~n", Consonant("ज्ञ")),
    ("gy", Consonant("ज्ञ")),
    // aspirated and multi-letter consonants
    ("chh", Consonant("छ")),
    ("shh", Consonant("ष")),
    (".Dh", Consonant("ढ़")),
    ("kh", Consonant("ख")),
    ("gh", Consonant("घ")),
    ("~N", Consonant("ङ")),
    ("ch", Consonant("च")),
    ("Ch", Consonant("छ")),
    ("jh", Consonant("झ")),
    ("~n", Consonant("ञ")),
    ("Th", Consonant("ठ")),
    ("Dh", Consonant("ढ")),
    ("th", Consonant("थ")),
    ("dh", Consonant("ध")),
    ("ph", Consonant("फ")),
    ("bh", Consonant("भ")),
    ("sh", Consonant("श")),
    ("Sh", Consonant("ष")),
    (".D", Consonant("ड़")),
    // single consonants
    ("k", Consonant("क")),
    ("g", Consonant("ग")),
    ("j", Consonant("ज")),
    ("T", Consonant("ट")),
    ("D", Consonant("ड")),
    ("N", Consonant("ण")),
    ("t", Consonant("त")),
    ("d", Consonant("द")),
    ("n", Consonant("न")),
    ("p", Consonant("प")),
    ("b", Consonant("ब")),
    ("m", Consonant("म")),
    ("y", Consonant("य")),
    ("r", Consonant("र")),
    ("l", Consonant("ल")),
    ("v", Consonant("व")),
    ("w", Consonant("व")),
    ("s", Consonant("स")),
    ("h", Consonant("ह")),
    // nukta consonants for Perso-Arabic sounds
    ("q", Consonant("क़")),
    ("K", Consonant("ख़")),
    ("G", Consonant("ग़")),
    ("z", Consonant("ज़")),
    ("f", Consonant("फ़")),
    ("x", Consonant("क्ष")),
    ("|", Mark("।")),
];

const TAMIL: &[(&str, Unit)] = &[
    // vowels (Tamil convention: e/o short, E/ee and O/oo long)
    ("aa", Vowel("ஆ", Some("ா"))),
    ("ai", Vowel("ஐ", Some("ை"))),
    ("au", Vowel("ஔ", Some("ௌ"))),
    ("ii", Vowel("ஈ", Some("ீ"))),
    ("uu", Vowel("ஊ", Some("ூ"))),
    ("ee", Vowel("ஏ", Some("ே"))),
    ("oo", Vowel("ஓ", Some("ோ"))),
    ("a", Vowel("அ", None)),
    ("A", Vowel("ஆ", Some("ா"))),
    ("i", Vowel("இ", Some("ி"))),
    ("I", Vowel("ஈ", Some("ீ"))),
    ("u", Vowel("உ", Some("ு"))),
    ("U", Vowel("ஊ", Some("ூ"))),
    ("e", Vowel("எ", Some("ெ"))),
    ("E", Vowel("ஏ", Some("ே"))),
    ("o", Vowel("ஒ", Some("ொ"))),
    ("O", Vowel("ஓ", Some("ோ"))),
    (".h", Virama),
    ("_", Break),
    // clusters with conventional spellings
    ("ksh", Consonant("க்ஷ")),
    ("ndr", Consonant("ன்ற")), // endraal → என்றால்
    // multi-letter consonants
    ("ng", Consonant("ங")),
    ("nj", Consonant("ஞ")),
    ("ch", Consonant("ச")),
    ("zh", Consonant("ழ")),
    ("sh", Consonant("ஷ")),
    ("th", Consonant("த")),
    ("dh", Consonant("த")),
    ("Th", Consonant("ட")),
    ("Dh", Consonant("ட")),
    // single consonants (Tamil merges voiced/unvoiced in writing)
    ("k", Consonant("க")),
    ("g", Consonant("க")),
    ("c", Consonant("ச")),
    ("s", Consonant("ச")),
    ("j", Consonant("ஜ")),
    ("T", Consonant("ட")),
    ("D", Consonant("ட")),
    ("N", Consonant("ண")),
    ("t", Consonant("த")),
    ("d", Consonant("த")),
    ("n", Consonant("ன")), // see `tamil_n` for the word-initial form
    ("p", Consonant("ப")),
    ("b", Consonant("ப")),
    ("m", Consonant("ம")),
    ("y", Consonant("ய")),
    ("r", Consonant("ர")),
    ("R", Consonant("ற")),
    ("l", Consonant("ல")),
    ("L", Consonant("ள")),
    ("v", Consonant("வ")),
    ("w", Consonant("வ")),
    ("S", Consonant("ஸ")),
    ("h", Consonant("ஹ")),
];

const DEVANAGARI_VIRAMA: &str = "्";
const TAMIL_PULLI: &str = "்";

/// Transliterates every run of ASCII letters in `input`; everything else
/// (spaces, digits, punctuation, text already in another script) is kept.
pub fn transliterate(input: &str, script: Script) -> String {
    let table = match script {
        Script::Devanagari => HINDI,
        Script::Tamil => TAMIL,
    };
    let virama = match script {
        Script::Devanagari => DEVANAGARI_VIRAMA,
        Script::Tamil => TAMIL_PULLI,
    };

    let mut out = String::with_capacity(input.len() * 3);
    let mut pending = false; // last output was a consonant without a vowel
    let mut word_start = true;
    let mut i = 0;
    let bytes = input.as_bytes();

    let end_word = |out: &mut String, pending: &mut bool| {
        if *pending && script == Script::Tamil {
            out.push_str(TAMIL_PULLI);
        }
        *pending = false;
    };

    while i < input.len() {
        let rest = &input[i..];
        let matched = table.iter().find(|(key, _)| rest.starts_with(key));
        let Some(&(key, unit)) = matched else {
            // Not part of the scheme: ends the current word.
            end_word(&mut out, &mut pending);
            let c = rest.chars().next().expect("non-empty");
            out.push(c);
            i += c.len_utf8();
            word_start = !c.is_alphanumeric();
            continue;
        };
        i += key.len();
        match unit {
            Consonant(c) => {
                if pending {
                    out.push_str(virama);
                }
                let next_is_dental = matches!(bytes.get(i), Some(b't' | b'd'));
                if script == Script::Tamil && key == "n" && (word_start || next_is_dental) {
                    out.push('ந'); // dental n: word-initial or before t/d
                } else {
                    out.push_str(c);
                }
                pending = true;
            }
            Vowel(independent, sign) => {
                if pending {
                    if let Some(s) = sign {
                        out.push_str(s);
                    }
                } else {
                    out.push_str(independent);
                }
                pending = false;
            }
            Mark(m) => {
                if m == "।" {
                    end_word(&mut out, &mut pending);
                }
                out.push_str(m);
                pending = false;
            }
            Virama => {
                out.push_str(virama);
                pending = false;
            }
            Break => {}
        }
        word_start = false;
    }
    end_word(&mut out, &mut pending);
    out
}

/// True if `s` looks like romanized input (ASCII letters only).
pub fn is_romanized(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'.' || b == b'~')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hi(s: &str) -> String {
        transliterate(s, Script::Devanagari)
    }

    fn ta(s: &str) -> String {
        transliterate(s, Script::Tamil)
    }

    #[test]
    fn hindi_keywords_and_words() {
        assert_eq!(hi("agar"), "अगर");
        assert_eq!(hi("chhaapo"), "छापो");
        assert_eq!(hi("kaarya"), "कार्य");
        assert_eq!(hi("sthir"), "स्थिर");
        assert_eq!(hi("char"), "चर");
        assert_eq!(hi("jabatak"), "जबतक");
        assert_eq!(hi("mukhya"), "मुख्य");
        assert_eq!(hi("ginatii"), "गिनती");
        assert_eq!(hi("namaste duniyaa"), "नमस्ते दुनिया");
        assert_eq!(hi("kShatriya"), "क्षत्रिय");
        assert_eq!(hi("poorNaaMk"), "पूर्णांक");
    }

    #[test]
    fn hindi_marks_and_explicit_virama() {
        assert_eq!(hi("haaM"), "हां");
        assert_eq!(hi("duHkh"), "दुःख");
        assert_eq!(hi("vidvaan.h"), "विद्वान्");
        assert_eq!(hi("a_i"), "अइ");
        assert_eq!(hi("zarUrat"), "ज़रूरत");
    }

    #[test]
    fn tamil_keywords_and_words() {
        assert_eq!(ta("seyal"), "செயல்");
        assert_eq!(ta("ninaivagam"), "நினைவகம்");
        assert_eq!(ta("endraal"), "என்றால்");
        assert_eq!(ta("thiruppu"), "திருப்பு");
        assert_eq!(ta("maaRi"), "மாறி");
        assert_eq!(ta("muthanmai"), "முதன்மை");
        assert_eq!(ta("vaNakkam"), "வணக்கம்");
        assert_eq!(ta("pazham"), "பழம்");
        assert_eq!(ta("nandri"), "நன்றி");
    }

    #[test]
    fn non_letters_and_existing_script_pass_through() {
        assert_eq!(hi("x = 42;"), "क्ष = 42;");
        assert_eq!(hi("गिनती + ek"), "गिनती + एक");
        assert_eq!(ta("a, b"), "அ, ப்");
    }

    #[test]
    fn output_lexes_as_identifiers() {
        use crate::lexer::{lex, Tok};
        use crate::pack::Packs;
        for (s, script) in [("ginatii", Script::Devanagari), ("eNNikkai", Script::Tamil)] {
            let word = transliterate(s, script);
            let l = lex(&word, &Packs::builtin());
            assert!(l.diagnostics.is_empty(), "{word}: {:?}", l.diagnostics);
            assert!(matches!(l.tokens[0].tok, Tok::Ident(_)), "{word}");
        }
    }
}
