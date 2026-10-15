# OmniLang

![ci](https://github.com/avkad3/Omnilang/actions/workflows/ci.yml/badge.svg)

**A programming language you can write in your own language.** OmniLang source can use English, Hindi (Devanagari) or Tamil keywords, or a mix of them in one file. Every form compiles to the same program, and any program can be shown in any of the languages.

The same function in three languages:

```text
fn square(x: int) -> int {          // English
    return x * x;
}

कार्य वर्ग(क: पूर्णांक) -> पूर्णांक {      // Hindi
    क * क लौटाओ।
}

செயல்பாடு வர்க்கம்(எ: முழுஎண்) -> முழுஎண் {   // Tamil
    எ * எ திருப்பு;
}
```

This isn't just keyword substitution. Hindi and Tamil put the verb at the end of a sentence, so OmniLang lets each language keep its own **word order**. In Hindi you write `x लौटाओ।` ("x return"), and in Tamil `x > 5 என்றால் { … }` ("x > 5 if"). The Hindi version even ends statements with the danda `।`.

## Why

Most people learning to program in India learn the ideas in their own language and the tools in English. Translating keywords has been tried before (Ezhil for Tamil, Hedy for many languages). OmniLang goes after the problems that usually make such languages a dead end.

| Problem | What OmniLang does |
|---|---|
| Code written in Tamil can't be read by anyone else | **Multilingual views:** `omni view --to hi` shows any program in Hindi, translating keywords, word order, type names, the entry point and (with a glossary) your own names. Translating to another language and back gives back the original source exactly. |
| Keywords are a few dozen words; real programs have hundreds of names | **Glossaries:** one file lists `total` / `कुल` / `மொத்தம்` as the same name. Different team members can use different spellings for the same variable. |
| Error messages are the real language barrier | **Errors in your language:** every compiler and runtime error comes in English, Hindi or Tamil, including word-order hints like ``'लौटाओ' … इसे अंत में लिखें, जैसे `x लौटाओ` ``. |
| Mixing scripts opens security holes | **Lookalike and hidden-character checks:** see [Security](#security). |
| Localized languages are toys | OmniLang compiles to **readable C**, has static types and overflow-checked arithmetic, and is being built toward a RISC-V kernel (see [Roadmap](#roadmap)). |

## Quick start

```bash
cargo build --release
./target/release/omni run examples/primes.om              # Hindi program
./target/release/omni view examples/primes.om --to ta     # …shown in Tamil
./target/release/omni build examples/squares.om           # native binary (needs gcc or clang)
./target/release/omni packs                               # keyword table
```

| Command | What it does |
|---|---|
| `omni run <file>` | type-check and run with the interpreter |
| `omni check <file>` | report errors only |
| `omni view <file> --to <en\|hi\|ta>` | print the program in another language |
| `omni c <file> [-o out.c]` | translate to C |
| `omni build <file> [-o prog]` | compile to a native executable with gcc or clang |
| `omni packs` | show every keyword in every language |

| Option | Effect |
|---|---|
| `--lang hi` | errors and printed output (`सत्य` / `असत्य`) in Hindi. You can also set `OMNI_LANG`. |
| `--native-digits` | print numbers as `२०२६` or `௨௦௨௬` |
| `--glossary <file>` | name translations. By default, `omni.glossary.toml` next to the source is used. |
| `--packs en,hi` | allow only some languages in the source |

## A program written by three people

[`examples/team/shop.om`](examples/team/shop.om) was written in three languages. Its [glossary](examples/team/omni.glossary.toml) declares `total` = `कुल` = `மொத்தம்`, so they are one variable:

```text
செயல்பாடு கட்டணம்(விலை: முழுஎண், அளவு: முழுஎண்) -> முழுஎண் {     // Tamil author
    விலை * அளவு திருப்பு;
}

fn main() {
    चर कुल = 0।                              // Hindi
    total = total + bill(40, 3);             // English
    कुल = कुल + बिल(25, 2)।                   // Hindi
    "Total:", மொத்தம் அச்சிடு;               // Tamil
}
```

`omni view examples/team/shop.om --to hi` shows the whole program, names included, in Hindi:

```text
कार्य बिल(मूल्य: पूर्णांक, मात्रा: पूर्णांक) -> पूर्णांक {
    मूल्य * मात्रा लौटाओ।
}

कार्य मुख्य() {
    चर कुल = 0।
    कुल = कुल + बिल(40, 3)।
    कुल = कुल + बिल(25, 2)।
    "Total:", कुल छापो।
}
```

## The language (v0.1)

- **Types:** `int` (64-bit), `bool`, `str`.
- **Statements:** `let` (fixed) and `var` (changeable) bindings, assignment, `if` / `else if` / `else`, `while`, `return`, and `print a, b, c`.
- **Functions:** typed parameters and return values, recursion.
- **Operators:** `+ - * / %`, comparisons, and `and` / `or` / `not`, which are also words in each language.
- **Numbers:** ASCII, Devanagari (`४२`) or Tamil (`௪௨`) digits.
- **Entry point:** a function named `main`, `मुख्य` or `முதன்மை`.
- **Arithmetic:** overflow and division by zero are errors, not undefined behaviour, in both the interpreter and compiled C.

| | English | Hindi | Tamil |
|---|---|---|---|
| function | `fn` | `कार्य` | `செயல்பாடு` |
| if | `if x { }` | `अगर x तो { }` | `x என்றால் { }` |
| while | `while x { }` | `जबतक x { }` | `x இருக்கும்வரை { }` |
| return | `return x;` | `x लौटाओ।` | `x திருப்பு;` |
| print | `print x;` | `x छापो।` | `x அச்சிடு;` |

Run `omni packs` for the full table. Language packs are plain [TOML files](crates/omni-core/packs/), so adding a language means adding one file.

## Security

A language that accepts many scripts must prevent code that *looks* different from what it *does*. These are compile errors, not lint hints:

- **Trojan Source** ([CVE-2021-42574](https://trojansource.codes/)): bidirectional control characters are rejected anywhere in the file, including comments and strings.
- **Invisible characters:** zero-width spaces and similar characters are rejected. Zero-width joiners are allowed only after a virama (halant or pulli), where Indic scripts legitimately need them.
- **Mixed-script names:** `pаypal` with a Cyrillic `а` is an error. Each name must use one script, following [Unicode UTS #39](https://www.unicode.org/reports/tr39/).
- **Lookalikes:** two names that render alike, or a name like `retum` that imitates the keyword `return`, are reported.
- **Normalization:** names are NFKC-normalized, as in Python. Canonically equivalent spellings, such as `क़` written as one or two code points, and fullwidth letters therefore mean exactly what they look like.

## How it works

```
source (any mix of packs)
   │  lexer: UAX #31 identifiers, NFKC, keyword lookup across packs, security checks
   ▼
language-neutral AST          ◄── nothing here records the human language
   │  checker: types, scopes, glossary-aware name resolution
   ├──► interpreter (omni run)
   ├──► view renderer (omni view --to xx)
   └──► C99 backend (omni c / omni build)
```

```
crates/omni-core/   compiler library: pack, lexer, security, parser, ast,
                    check, glossary, interp, view, cgen, diag
crates/omni-core/packs/     en.toml, hi.toml, ta.toml
crates/omni-core/messages.toml   every diagnostic in every language
crates/omni-cli/    the `omni` binary and end-to-end tests
examples/           programs in each language, with recorded output
```

## Testing

`cargo test` runs about 60 unit tests and end-to-end tests. Every example in `examples/`:

1. produces its recorded output;
2. still produces it after being translated into each of the three languages;
3. produces the same bytes as a native binary built with gcc or clang.

CI runs all of this on Linux and Windows.

## Roadmap

- [x] **Phase 1: multilingual compiler.** Language packs, word-order-aware parser, glossaries, multilingual views, localized errors, interpreter and C backend.
- [ ] **Phase 2: bare-metal RISC-V.** A minimal kernel in QEMU whose panic screen shows the error in Hindi or Tamil, using text shaped at build time.
- [ ] **Phase 3: editor support.** A language server and VS Code extension with transliteration input (`agar` → `अगर`).
- [ ] **Phase 4: offline assistant.** A small local model that explains errors in your language and drafts code that the compiler then checks.
- [ ] More languages (Marathi, Bengali, Arabic…) and a browser playground (WebAssembly).

## Known limitations

- The Hindi and Tamil keywords and messages need review by native speakers and teachers. They live in data files ([packs](crates/omni-core/packs/), [messages](crates/omni-core/messages.toml)), so corrections don't require touching the compiler.
- The C backend relies on GCC or Clang overflow builtins, so MSVC isn't supported yet.
- Deep recursion is limited to 5,000 calls in the interpreter. Compiled programs use the native stack.

## License

[MIT](LICENSE)
