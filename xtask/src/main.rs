//! `cargo xtask`: build the kernel and run it in QEMU.
//!
//!   cargo xtask build [--release]
//!   cargo xtask run   [--lang en|hi|ta] [--demo illegal|load|store|misaligned|panic] [--headless]

use std::env;
use std::path::{Path, PathBuf};
use std::process::{exit, Command, ExitStatus};

const TARGET: &str = "riscv64gc-unknown-none-elf";

struct Opts {
    release: bool,
    lang: String,
    demo: Option<String>,
    headless: bool,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
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
    let status = cmd.status().unwrap_or_else(|e| fail(format!("cannot run cargo: {e}")));
    if !status.success() {
        fail("kernel build failed");
    }
    let profile = if release { "release" } else { "debug" };
    kernel_dir().join("target").join(TARGET).join(profile).join("omni-kernel")
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
    let mut o = Opts { release: false, lang: "en".into(), demo: None, headless: false };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().unwrap_or_else(|| fail(format!("{a} needs a value")));
        match a.as_str() {
            "--release" => o.release = true,
            "--lang" => o.lang = value(),
            "--demo" => o.demo = Some(value()),
            "--headless" => o.headless = true,
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
        other => fail(format!("unknown command '{other}'")),
    }
}
