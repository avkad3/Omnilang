//! `omni`: the OmniLang command-line tool.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::{env, fs};

use omni_core::ast::Program;
use omni_core::cgen::{self, COptions};
use omni_core::check::check;
use omni_core::diag::{Diagnostic, Severity, MESSAGE_LANGS};
use omni_core::glossary::Glossary;
use omni_core::interp::{run_program, OutputStyle};
use omni_core::pack::{Kw, Packs, Position};
use omni_core::parser::parse;
use omni_core::view::{render, ViewOptions};

const USAGE: &str = "\
omni: the OmniLang compiler

USAGE:
    omni run   <file>                 compile and run with the interpreter
    omni check <file>                 report errors without running
    omni view  <file> --to <lang>     show the program in another language
    omni c     <file> [-o out.c]      translate to C
    omni build <file> [-o program]    compile to a native executable (needs gcc or clang)
    omni packs                        list language packs and their keywords
    omni translit --to <hi|ta> <text> romanized typing: agar -> अगर

OPTIONS:
    --lang <en|hi|ta>     language for errors and printed output (default: $OMNI_LANG or en)
    --native-digits       print numbers in the output language's digits (०१२ / ௦௧௨)
    --glossary <file>     name translations (default: omni.glossary.toml next to the source)
    --packs <en,hi,ta>    language packs allowed in the source (default: all)
    --cc <compiler>       C compiler for `build` (default: $CC, then gcc, then clang)
";

struct Opts {
    command: String,
    file: Option<PathBuf>,
    lang: String,
    to: Option<String>,
    output: Option<PathBuf>,
    native_digits: bool,
    glossary: Option<PathBuf>,
    packs: Option<String>,
    cc: Option<String>,
}

fn parse_args() -> Result<Opts, String> {
    let mut args = env::args().skip(1);
    let command = args.next().ok_or("missing command")?;
    let mut o = Opts {
        command,
        file: None,
        lang: env::var("OMNI_LANG").unwrap_or_else(|_| "en".into()),
        to: None,
        output: None,
        native_digits: false,
        glossary: None,
        packs: None,
        cc: None,
    };
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--lang" => o.lang = value("--lang")?,
            "--to" => o.to = Some(value("--to")?),
            "-o" | "--output" => o.output = Some(value("-o")?.into()),
            "--native-digits" => o.native_digits = true,
            "--glossary" => o.glossary = Some(value("--glossary")?.into()),
            "--packs" => o.packs = Some(value("--packs")?),
            "--cc" => o.cc = Some(value("--cc")?),
            "-h" | "--help" => o.command = "help".into(),
            s if s.starts_with('-') => return Err(format!("unknown option '{s}'")),
            s if o.file.is_none() => o.file = Some(s.into()),
            s => return Err(format!("unexpected argument '{s}'")),
        }
    }
    if !MESSAGE_LANGS.contains(&o.lang.as_str()) {
        return Err(format!("--lang must be one of {}", MESSAGE_LANGS.join(", ")));
    }
    Ok(o)
}

/// Everything loaded and checked for one source file.
struct Unit {
    src: String,
    file: String,
    glossary: Glossary,
    program: Program,
    entry: Option<usize>,
}

enum Failure {
    /// Diagnostics were already printed.
    Reported,
    Message(String),
}

impl From<String> for Failure {
    fn from(s: String) -> Failure {
        Failure::Message(s)
    }
}

impl From<&str> for Failure {
    fn from(s: &str) -> Failure {
        Failure::Message(s.to_string())
    }
}

/// Prints diagnostics; returns true if any of them is an error.
fn print_diagnostics(diags: &[Diagnostic], src: &str, file: &str, lang: &str) -> bool {
    let mut errors = false;
    for d in diags {
        eprintln!("{}", d.render(src, file, lang));
        errors |= d.severity == Severity::Error;
    }
    errors
}

fn load(o: &Opts, require_entry: bool) -> Result<Unit, Failure> {
    let path = o.file.as_ref().ok_or("missing source file")?;
    let src = fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let file = path.display().to_string();

    let all = Packs::builtin();
    let packs = match &o.packs {
        Some(list) => {
            let codes: Vec<&str> = list.split(',').map(str::trim).collect();
            all.only(&codes).map_err(|e| e.to_string())?
        }
        None => all,
    };

    let glossary_path = o.glossary.clone().or_else(|| {
        let p = path.parent().unwrap_or(Path::new(".")).join("omni.glossary.toml");
        p.exists().then_some(p)
    });
    let glossary = match glossary_path {
        Some(p) => {
            let text = fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
            Glossary::from_toml(&text).map_err(|e| format!("{}: {e}", p.display()))?
        }
        None => Glossary::default(),
    };

    let parsed = parse(&src, &packs);
    if print_diagnostics(&parsed.diagnostics, &src, &file, &o.lang) {
        return Err(Failure::Reported);
    }
    let checked = check(&parsed.program, &packs, &glossary, require_entry);
    if print_diagnostics(&checked.diagnostics, &src, &file, &o.lang) {
        return Err(Failure::Reported);
    }
    Ok(Unit {
        src,
        file,
        glossary,
        program: parsed.program,
        entry: checked.entry,
    })
}

/// Output words (true/false, digits) come from the --lang pack, even if
/// --packs restricts which keywords the source may use.
fn output_style<'a>(o: &Opts, all: &'a Packs) -> OutputStyle<'a> {
    let pack = all.get(&o.lang).expect("--lang was validated");
    OutputStyle {
        pack,
        native_digits: o.native_digits,
    }
}

fn cmd_run(o: &Opts) -> Result<ExitCode, Failure> {
    let u = load(o, true)?;
    let all = Packs::builtin();
    let mut stdout = std::io::stdout();
    let entry = u.entry.expect("checked entry point");
    match run_program(&u.program, &u.glossary, output_style(o, &all), entry, &mut stdout) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(d) => {
            eprintln!("{}", d.render(&u.src, &u.file, &o.lang));
            Ok(ExitCode::from(101))
        }
    }
}

fn cmd_check(o: &Opts) -> Result<ExitCode, Failure> {
    load(o, false)?;
    Ok(ExitCode::SUCCESS)
}

fn cmd_view(o: &Opts) -> Result<ExitCode, Failure> {
    let to = o.to.as_deref().ok_or("view needs --to <lang>")?;
    let u = load(o, false)?;
    let all = Packs::builtin();
    let pack = all.get(to).ok_or(format!("unknown language '{to}'"))?;
    let opts = ViewOptions {
        pack,
        packs: &all,
        glossary: &u.glossary,
        native_digits: o.native_digits,
        translate_names: true,
    };
    print!("{}", render(&u.program, &opts));
    Ok(ExitCode::SUCCESS)
}

fn c_source(o: &Opts, u: &Unit) -> String {
    let all = Packs::builtin();
    let opts = COptions {
        glossary: &u.glossary,
        style: output_style(o, &all),
        message_lang: &o.lang,
    };
    cgen::generate(&u.program, u.entry.expect("checked entry point"), &opts)
}

fn cmd_c(o: &Opts) -> Result<ExitCode, Failure> {
    let u = load(o, true)?;
    let c = c_source(o, &u);
    match &o.output {
        Some(p) => fs::write(p, c).map_err(|e| format!("cannot write {}: {e}", p.display()))?,
        None => print!("{c}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_build(o: &Opts) -> Result<ExitCode, Failure> {
    let u = load(o, true)?;
    let c = c_source(o, &u);
    let src_path = o.file.as_ref().expect("checked by load");
    let exe = o.output.clone().unwrap_or_else(|| {
        let stem = src_path.with_extension("");
        if cfg!(windows) {
            stem.with_extension("exe")
        } else {
            stem
        }
    });
    let c_path = exe.with_extension("c");
    fs::write(&c_path, &c).map_err(|e| format!("cannot write {}: {e}", c_path.display()))?;

    let candidates: Vec<String> = match (&o.cc, env::var("CC")) {
        (Some(cc), _) => vec![cc.clone()],
        (None, Ok(cc)) if !cc.is_empty() => vec![cc],
        _ => vec!["gcc".into(), "clang".into()],
    };
    for cc in &candidates {
        let status = Command::new(cc)
            .args(["-std=c99", "-O2", "-o"])
            .arg(&exe)
            .arg(&c_path)
            .status();
        match status {
            Ok(s) if s.success() => {
                let _ = fs::remove_file(&c_path);
                eprintln!("built {}", exe.display());
                return Ok(ExitCode::SUCCESS);
            }
            Ok(_) => return Err(format!("{cc} failed; the generated C is in {}", c_path.display()).into()),
            Err(_) => continue, // not installed: try the next compiler
        }
    }
    Err(format!(
        "no C compiler found (tried {}); install gcc or clang, or pass --cc. The C source is in {}",
        candidates.join(", "),
        c_path.display()
    )
    .into())
}

fn cmd_packs() -> ExitCode {
    let packs = Packs::builtin();
    let rows = [
        ("function", Kw::Fn),
        ("fixed", Kw::Let),
        ("variable", Kw::Var),
        ("if", Kw::If),
        ("then", Kw::Then),
        ("else", Kw::Else),
        ("while", Kw::While),
        ("return", Kw::Return),
        ("print", Kw::Print),
        ("true", Kw::True),
        ("false", Kw::False),
        ("and", Kw::And),
        ("or", Kw::Or),
        ("not", Kw::Not),
        ("integer", Kw::Int),
        ("boolean", Kw::Bool),
        ("text", Kw::Str),
    ];
    let list: Vec<_> = packs.iter().collect();
    let cell = |s: &str| format!("{s:<24}");
    let mut header = cell("");
    for p in &list {
        header.push_str(&cell(&format!("{} ({})", p.name, p.code)));
    }
    println!("{}", header.trim_end());
    for (label, kw) in rows {
        let mut line = cell(label);
        for p in &list {
            let w = p.word(kw).unwrap_or("-");
            let after = matches!(kw, Kw::If | Kw::While | Kw::Return | Kw::Print)
                && p.position(kw) == Position::Postfix;
            line.push_str(&cell(&if after { format!("… {w}") } else { w.to_string() }));
        }
        println!("{}", line.trim_end());
    }
    let mut line = cell("entry point");
    for p in &list {
        line.push_str(&cell(&p.main));
    }
    println!("{}", line.trim_end());
    println!("\n\"… word\" means the word comes after what it applies to.");
    ExitCode::SUCCESS
}

fn cmd_translit(o: &Opts) -> Result<ExitCode, Failure> {
    use omni_core::translit::{transliterate, Script};
    let to = o.to.as_deref().ok_or("translit needs --to <hi|ta>")?;
    let script = Script::for_lang(to).ok_or(format!("no transliteration for '{to}' (use hi or ta)"))?;
    // The text is the positional argument (parsed into `file`).
    let text = o.file.as_ref().ok_or("translit needs the text to convert")?;
    println!("{}", transliterate(&text.to_string_lossy(), script));
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("omni: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match o.command.as_str() {
        "run" => cmd_run(&o),
        "check" => cmd_check(&o),
        "view" => cmd_view(&o),
        "c" => cmd_c(&o),
        "build" => cmd_build(&o),
        "packs" => return cmd_packs(),
        "translit" => cmd_translit(&o),
        "help" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("omni: unknown command '{other}'\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => code,
        Err(Failure::Reported) => ExitCode::from(1),
        Err(Failure::Message(m)) => {
            eprintln!("omni: {m}");
            ExitCode::from(1)
        }
    }
}
