//! End-to-end tests over `examples/`: every program must produce its
//! recorded output when run directly, after being translated into every
//! language, and (when a C compiler is available) as a native binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const LANGS: [&str; 3] = ["en", "hi", "ta"];

fn omni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_omni"))
}

fn examples() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut out = Vec::new();
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).expect("examples dir") {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "om") {
                out.push(path);
            }
        }
    }
    out.sort();
    assert!(out.len() >= 5, "expected example programs, found {out:?}");
    out
}

fn expected(example: &Path) -> String {
    fs::read_to_string(example.with_extension("out"))
        .expect("recorded .out file")
        .replace("\r\n", "\n")
}

/// Glossary next to the example, which must also be passed explicitly when
/// a translated copy lives elsewhere.
fn glossary_args(example: &Path) -> Vec<String> {
    let g = example.parent().unwrap().join("omni.glossary.toml");
    if g.exists() {
        vec!["--glossary".into(), g.display().to_string()]
    } else {
        vec![]
    }
}

fn run(file: &Path, extra: &[String]) -> String {
    let out = omni()
        .arg("run")
        .arg(file)
        .args(extra)
        .output()
        .expect("run omni");
    assert!(
        out.status.success(),
        "omni run {}: {}",
        file.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("examples");
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn examples_produce_recorded_output() {
    for ex in examples() {
        assert_eq!(
            run(&ex, &glossary_args(&ex)),
            expected(&ex),
            "{}",
            ex.display()
        );
    }
}

#[test]
fn translated_examples_behave_identically() {
    for ex in examples() {
        let stem = ex.file_stem().unwrap().to_string_lossy().to_string();
        for lang in LANGS {
            let view = omni()
                .arg("view")
                .arg(&ex)
                .args(["--to", lang])
                .args(glossary_args(&ex))
                .output()
                .unwrap();
            assert!(
                view.status.success(),
                "view {}: {}",
                ex.display(),
                String::from_utf8_lossy(&view.stderr)
            );
            let copy = scratch(&format!("{stem}.{lang}.om"));
            fs::write(&copy, &view.stdout).unwrap();
            assert_eq!(
                run(&copy, &glossary_args(&ex)),
                expected(&ex),
                "{} via {lang}",
                ex.display()
            );
        }
    }
}

fn c_compiler() -> Option<String> {
    let candidates = std::env::var("CC")
        .into_iter()
        .chain(["gcc".to_string(), "clang".to_string()]);
    candidates.into_iter().find(|cc| {
        Command::new(cc)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn native_builds_match_the_interpreter() {
    let Some(cc) = c_compiler() else {
        eprintln!("no C compiler found; skipping native build test");
        return;
    };
    for ex in examples() {
        let stem = ex.file_stem().unwrap().to_string_lossy().to_string();
        let exe = scratch(&format!("{stem}{}", std::env::consts::EXE_SUFFIX));
        let build = omni()
            .arg("build")
            .arg(&ex)
            .arg("-o")
            .arg(&exe)
            .args(["--cc", &cc])
            .args(glossary_args(&ex))
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "build {}: {}",
            ex.display(),
            String::from_utf8_lossy(&build.stderr)
        );
        let out = Command::new(&exe).output().unwrap();
        assert!(
            out.status.success(),
            "{} exited with {}",
            exe.display(),
            out.status
        );
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            expected(&ex),
            "native {}",
            ex.display()
        );
    }
}

#[test]
fn errors_are_reported_in_the_chosen_language() {
    let bad = scratch("bad.om");
    fs::write(&bad, "fn main() { let x: int = true; }\n").unwrap();
    for (lang, word) in [
        ("en", "type mismatch"),
        ("hi", "प्रकार मेल नहीं खाता"),
        ("ta", "வகை பொருந்தவில்லை"),
    ] {
        let out = omni()
            .arg("check")
            .arg(&bad)
            .args(["--lang", lang])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains(word), "{lang}: {stderr}");
    }
}

#[test]
fn runtime_errors_exit_with_101() {
    let f = scratch("div.om");
    fs::write(&f, "fn main() { var z = 0; print 1 / z; }\n").unwrap();
    let out = omni().arg("run").arg(&f).output().unwrap();
    assert_eq!(out.status.code(), Some(101));
    assert!(String::from_utf8(out.stderr)
        .unwrap()
        .contains("division by zero"));
}
