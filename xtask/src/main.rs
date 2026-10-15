//! `cargo xtask`: build the kernel and run it in QEMU.
//!
//!   cargo xtask build      [--release]
//!   cargo xtask run        [--lang en|hi|ta] [--demo illegal|load|store|misaligned|panic] [--headless]
//!   cargo xtask screenshot [--lang ..] [--demo ..] [-o file.png]

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
