//! `cargo xtask`: build the kernel and run it in QEMU.
//!
//!   cargo xtask build      [--release]
//!   cargo xtask run        [--lang en|hi|ta] [--demo illegal|load|store|misaligned|panic] [--headless]
//!   cargo xtask screenshot [--lang ..] [--demo ..] [-o file.png]
//!   cargo xtask test       [--release]   boot every language × fault and check the reports
//!   cargo xtask grammar    [--check]     regenerate the VS Code syntax grammar from the packs

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const TARGET: &str = "riscv64gc-unknown-none-elf";

struct Opts {
    check: bool,
    release: bool,
    lang: String,
    demo: Option<String>,
    headless: bool,
    output: Option<PathBuf>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn kernel_dir() -> PathBuf {
    root().join("kernel")
}

fn fail(msg: impl AsRef<str>) -> ! {
    eprintln!("xtask: {}", msg.as_ref());
    exit(1)
}

/// Builds the kernel and returns the path of the ELF image.
fn build(release: bool) -> PathBuf {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    // Run inside kernel/ so its .cargo/config.toml (target) applies.
    cmd.current_dir(kernel_dir()).arg("build");
    if release {
        cmd.arg("--release");
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| fail(format!("cannot run cargo: {e}")));
    if !status.success() {
        fail("kernel build failed");
    }
    let profile = if release { "release" } else { "debug" };
    kernel_dir()
        .join("target")
        .join(TARGET)
        .join(profile)
        .join("omni-kernel")
}

/// Finds qemu-system-riscv64: $QEMU, then PATH, then the Windows installer location.
pub fn qemu() -> PathBuf {
    if let Ok(q) = env::var("QEMU") {
        return q.into();
    }
    let exe = format!("qemu-system-riscv64{}", env::consts::EXE_SUFFIX);
    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            let p = dir.join(&exe);
            if p.is_file() {
                return p;
            }
        }
    }
    let windows = Path::new(r"C:\Program Files\qemu").join(&exe);
    if windows.is_file() {
        return windows;
    }
    fail("qemu-system-riscv64 not found; install QEMU or set $QEMU")
}

/// QEMU command for the `virt` machine with a ramfb framebuffer.
fn qemu_command(kernel: &Path, cmdline: &str, headless: bool) -> Command {
    let mut cmd = Command::new(qemu());
    cmd.args(["-machine", "virt", "-bios", "default", "-m", "128M", "-smp", "1"])
        .arg("-kernel")
        .arg(kernel)
        .args(["-append", cmdline])
        .args(["-device", "ramfb"])
        .args(["-serial", "stdio"]);
    if headless {
        cmd.args(["-display", "none"]);
    }
    cmd
}

fn cmdline(o: &Opts) -> String {
    let mut s = format!("lang={}", o.lang);
    if let Some(d) = &o.demo {
        s.push_str(&format!(" demo={d}"));
    }
    s
}

fn run(o: &Opts) -> ExitStatus {
    let kernel = build(o.release);
    qemu_command(&kernel, &cmdline(o), o.headless)
        .status()
        .unwrap_or_else(|e| fail(format!("cannot start QEMU: {e}")))
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        check: false,
        release: false,
        lang: "en".into(),
        demo: None,
        headless: false,
        output: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .unwrap_or_else(|| fail(format!("{a} needs a value")))
        };
        match a.as_str() {
            "--release" => o.release = true,
            "--check" => o.check = true,
            "--lang" => o.lang = value(),
            "--demo" => o.demo = Some(value()),
            "--headless" => o.headless = true,
            "-o" | "--output" => o.output = Some(value().into()),
            other => fail(format!("unknown option '{other}'")),
        }
    }
    o
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        fail("usage: cargo xtask <build|run> [options]");
    };
    let o = parse(rest);
    match cmd.as_str() {
        "build" => {
            println!("{}", build(o.release).display());
        }
        "run" => exit(run(&o).code().unwrap_or(1)),
        "screenshot" => screenshot(&o),
        "test" => test(&o),
        "grammar" => grammar(&o),
        other => fail(format!("unknown command '{other}'")),
    }
}

/// Boots headless, waits for the panic screen, and saves it as PNG via QMP.
fn screenshot(o: &Opts) {
    let demo = o.demo.clone().unwrap_or_else(|| "illegal".into());
    let out = o
        .output
        .clone()
        .unwrap_or_else(|| root().join(format!("docs/panic-{}-{demo}.png", o.lang)));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(format!("{}: {e}", dir.display())));
    }
    let kernel = build(o.release);
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap();
    let cmdline = format!("lang={} demo={demo}", o.lang);
    let mut child = qemu_command(&kernel, &cmdline, true)
        .args(["-qmp", &format!("tcp:127.0.0.1:{port},server=on,wait=off")])
        .stdout(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| fail(format!("cannot start QEMU: {e}")));

    // Watch the serial console for the kernel's "drawn" marker.
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            println!("  | {line}");
            if line.contains("[omni] panic screen drawn") {
                let _ = tx.send(());
            }
        }
    });
    if rx.recv_timeout(Duration::from_secs(60)).is_err() {
        let _ = child.kill();
        fail("timed out waiting for the panic screen");
    }

    let result = qmp_screendump(port, &out);
    let _ = child.kill();
    let _ = child.wait();
    match result {
        Ok(()) => println!("saved {}", out.display()),
        Err(e) => fail(format!("screendump failed: {e}")),
    }
}

/// Reads QMP messages until the reply to the last command arrives.
fn qmp_reply(reader: &mut BufReader<TcpStream>) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::Error::other("QMP connection closed"));
        }
        if line.contains("\"return\"") {
            return Ok(());
        }
        if line.contains("\"error\"") {
            return Err(std::io::Error::other(line.trim().to_string()));
        }
        // Anything else is an asynchronous event; keep reading.
    }
}

fn qmp_screendump(port: u16, out: &Path) -> std::io::Result<()> {
    let stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut greeting = String::new();
    reader.read_line(&mut greeting)?;
    writeln!(writer, r#"{{"execute":"qmp_capabilities"}}"#)?;
    qmp_reply(&mut reader)?;
    // JSON string: forward slashes avoid escaping Windows backslashes.
    let path = std::path::absolute(out)?.display().to_string().replace('\\', "/");
    writeln!(
        writer,
        r#"{{"execute":"screendump","arguments":{{"filename":"{path}","format":"png"}}}}"#
    )?;
    qmp_reply(&mut reader)?;
    Ok(())
}

/// Demo fault → the messages.toml key of the cause the kernel must report.
const FAULTS: [(&str, &str); 5] = [
    ("illegal", "cause_illegal_instruction"),
    ("load", "cause_page_fault"),
    ("readonly", "cause_write_protected"),
    ("misaligned", "cause_misaligned"),
    ("panic", "cause_kernel_bug"),
];

/// Boots the kernel headless once per language and fault, checking the
/// exit status and that the serial report names the right cause in the
/// right language.
fn test(o: &Opts) {
    let kernel = build(o.release);
    let messages: toml::Table = std::fs::read_to_string(kernel_dir().join("messages.toml"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| fail("cannot read kernel/messages.toml"));
    let text = |key: &str, lang: &str| -> String {
        messages[key][lang]
            .as_str()
            .unwrap_or_else(|| fail(format!("{key}.{lang} missing")))
            .to_string()
    };

    let mut failures = 0;
    let mut runs = 0;
    for lang in ["en", "hi", "ta"] {
        let mut cases: Vec<(Option<&str>, Option<&str>)> = vec![(None, None)];
        cases.extend(FAULTS.iter().map(|(d, k)| (Some(*d), Some(*k))));
        for (demo, cause) in cases {
            runs += 1;
            let mut cmdline = format!("lang={lang} exit");
            if let Some(d) = demo {
                cmdline.push_str(&format!(" demo={d}"));
            }
            let out = qemu_command(&kernel, &cmdline, true)
                .stdin(Stdio::null())
                .output()
                .unwrap_or_else(|e| fail(format!("cannot start QEMU: {e}")));
            let serial = String::from_utf8_lossy(&out.stdout);
            let mut problems = Vec::new();
            let expected_code = if demo.is_some() { 1 } else { 0 };
            if out.status.code() != Some(expected_code) {
                problems.push(format!(
                    "exit status {:?}, expected {expected_code}",
                    out.status.code()
                ));
            }
            if !serial.contains(&text("boot", lang)) {
                problems.push("boot message missing".into());
            }
            if let Some(key) = cause {
                let want = format!("{} {}", text("cause_label", lang), text(key, lang));
                if !serial.contains(&want) {
                    problems.push(format!("expected `{want}` in the report"));
                }
                if !serial.contains("[omni] panic screen drawn") {
                    problems.push("panic screen not drawn".into());
                }
            }
            let label = format!("lang={lang} demo={}", demo.unwrap_or("none"));
            if problems.is_empty() {
                println!("ok   {label}");
            } else {
                failures += 1;
                println!("FAIL {label}: {}", problems.join("; "));
                for line in serial
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .rev()
                    .take(12)
                    .collect::<Vec<_>>()
                    .iter()
                    .rev()
                {
                    println!("     | {line}");
                }
            }
        }
    }
    println!("\n{} of {runs} boots passed", runs - failures);
    if failures > 0 {
        exit(1);
    }
}

/// Keyword groups for highlighting: (TextMate scope, keyword names).
const GRAMMAR_GROUPS: [(&str, &[&str]); 6] = [
    (
        "keyword.control.omnilang",
        &["if", "else", "then", "while", "return"],
    ),
    ("storage.type.function.omnilang", &["fn"]),
    ("storage.modifier.omnilang", &["let", "var"]),
    ("support.function.builtin.omnilang", &["print"]),
    ("constant.language.omnilang", &["true", "false"]),
    ("keyword.operator.word.omnilang", &["and", "or", "not"]),
];
const GRAMMAR_TYPES: &[&str] = &["int", "bool", "str"];

/// Generates editors/vscode/syntaxes/omnilang.tmLanguage.json from the
/// language packs, so every language's keywords are highlighted.
fn grammar(o: &Opts) {
    let packs_dir = root().join("crates/omni-core/packs");
    let mut packs: Vec<toml::Table> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&packs_dir)
        .unwrap_or_else(|e| fail(e.to_string()))
        .collect();
    entries.sort_by_key(|e| e.as_ref().map(|e| e.path()).ok());
    for e in entries {
        let path = e.unwrap().path();
        if path.extension().is_some_and(|x| x == "toml") {
            let text = std::fs::read_to_string(&path).unwrap();
            packs.push(
                text.parse()
                    .unwrap_or_else(|e| fail(format!("{}: {e}", path.display()))),
            );
        }
    }
    let words = |names: &[&str]| -> String {
        let mut all: Vec<String> = Vec::new();
        for p in &packs {
            for name in names {
                let w = p["keywords"].get(*name).and_then(|v| v.as_str()).or_else(|| {
                    p["statements"]
                        .get(*name)
                        .and_then(|s| s.get("word"))
                        .and_then(|v| v.as_str())
                });
                if let Some(w) = w {
                    if !all.iter().any(|x| x == w) {
                        all.push(w.to_string());
                    }
                }
            }
        }
        // Longest first so alternation prefers whole words.
        all.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
        all.join("|")
    };
    // Unicode-aware word boundaries: Indic vowel signs are marks (\p{M}),
    // which a plain \b would treat as a boundary.
    let word = |alts: String| format!(r"(?<![\p{{L}}\p{{M}}\p{{N}}_])(?:{alts})(?![\p{{L}}\p{{M}}\p{{N}}_])");

    let mut patterns = vec![
        serde_json::json!({ "include": "#comment" }),
        serde_json::json!({ "include": "#string" }),
    ];
    for (scope, names) in GRAMMAR_GROUPS {
        patterns.push(serde_json::json!({ "name": scope, "match": word(words(names)) }));
    }
    patterns
        .push(serde_json::json!({ "name": "storage.type.omnilang", "match": word(words(GRAMMAR_TYPES)) }));
    patterns.push(serde_json::json!({
        "name": "constant.numeric.omnilang",
        "match": r"(?<![\p{L}\p{M}_])[0-9०-९௦-௯]+(?![\p{L}\p{M}_])"
    }));
    patterns.push(serde_json::json!({
        "name": "entity.name.function.omnilang",
        "match": r"[\p{L}_][\p{L}\p{M}\p{N}_\x{200C}\x{200D}]*(?=\s*\()"
    }));
    patterns.push(serde_json::json!({ "name": "punctuation.terminator.omnilang", "match": "[;।॥]" }));
    patterns.push(serde_json::json!({ "name": "keyword.operator.omnilang", "match": r"->|==|!=|<=|>=|&&|\|\||[-+*/%<>=!]" }));

    let grammar = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
        "name": "OmniLang",
        "scopeName": "source.omnilang",
        "comment": "Generated by `cargo xtask grammar` from crates/omni-core/packs. Do not edit by hand.",
        "patterns": patterns,
        "repository": {
            "comment": { "name": "comment.line.double-slash.omnilang", "match": "//.*$" },
            "string": {
                "name": "string.quoted.double.omnilang",
                "begin": "\"",
                "end": "\"",
                "patterns": [{ "name": "constant.character.escape.omnilang", "match": r#"\[nt"\]"# }]
            }
        }
    });
    let text = serde_json::to_string_pretty(&grammar).unwrap() + "\n";
    let out = root().join("editors/vscode/syntaxes/omnilang.tmLanguage.json");
    if o.check {
        let current = std::fs::read_to_string(&out)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if current != text {
            fail("editors/vscode grammar is out of date: run `cargo xtask grammar`");
        }
        println!("grammar is up to date");
        return;
    }
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(&out, text).unwrap();
    println!("wrote {}", out.display());
}
