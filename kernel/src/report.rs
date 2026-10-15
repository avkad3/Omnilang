//! Fatal error reporting in the user's language.
//!
//! Every fatal error, whether a CPU exception or a Rust panic, ends up in
//! [`fatal`], which writes a localized report to the serial console and then
//! halts (or powers off when booted with `exit`).

use core::fmt::{self, Write as _};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::messages as m;
use crate::{sbi, trap, uart::Uart};

static LANG: AtomicUsize = AtomicUsize::new(0);
static EXIT_ON_FATAL: AtomicBool = AtomicBool::new(false);
static IN_FATAL: AtomicBool = AtomicBool::new(false);

pub fn configure(lang: usize, exit: bool) {
    LANG.store(lang, Ordering::Relaxed);
    EXIT_ON_FATAL.store(exit, Ordering::Relaxed);
}

pub fn lang() -> usize {
    LANG.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    IllegalInstruction,
    LoadAccess,
    StoreAccess,
    Misaligned,
    PageFault,
    KernelBug,
    /// Raw `scause` value.
    Unknown(usize),
}

impl Cause {
    pub fn message(self) -> &'static [&'static str; 3] {
        match self {
            Cause::IllegalInstruction => &m::CAUSE_ILLEGAL_INSTRUCTION,
            Cause::LoadAccess => &m::CAUSE_LOAD_ACCESS,
            Cause::StoreAccess => &m::CAUSE_STORE_ACCESS,
            Cause::Misaligned => &m::CAUSE_MISALIGNED,
            Cause::PageFault => &m::CAUSE_PAGE_FAULT,
            Cause::KernelBug => &m::CAUSE_KERNEL_BUG,
            Cause::Unknown(_) => &m::CAUSE_UNKNOWN,
        }
    }
}

/// Writes a decimal number using the language's own digits.
pub struct Decimal(pub u64);

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = m::DIGITS[lang()];
        let mut buf = [0u8; 20];
        let mut n = self.0;
        let mut len = 0;
        loop {
            buf[len] = (n % 10) as u8;
            len += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        for &d in buf[..len].iter().rev() {
            f.write_char(digits.chars().nth(d as usize).unwrap_or('?'))?;
        }
        Ok(())
    }
}

/// Reports a fatal error and never returns.
pub fn fatal(cause: Cause, pc: Option<usize>, addr: Option<usize>, detail: Option<&fmt::Arguments>) -> ! {
    // A fault while reporting a fault must not recurse forever.
    if IN_FATAL.swap(true, Ordering::SeqCst) {
        sbi::shutdown(true);
    }
    let l = lang();
    let mut out = Uart;
    let _ = writeln!(out, "\n==================== {} ====================", m::TITLE[l]);
    let _ = writeln!(out, "{}", m::SUBTITLE[l]);
    let _ = write!(out, "{} {}", m::CAUSE_LABEL[l], cause.message()[l]);
    if let Cause::Unknown(code) = cause {
        let _ = write!(out, " (scause {code:#x})");
    }
    let _ = writeln!(out);
    if let Some(pc) = pc {
        let _ = writeln!(out, "{} {pc:#018x}", m::PC_LABEL[l]);
    }
    if let Some(a) = addr {
        let _ = writeln!(out, "{} {a:#018x}", m::ADDR_LABEL[l]);
    }
    let _ = writeln!(out, "{} {}", m::TICKS_LABEL[l], Decimal(trap::ticks()));
    if let Some(d) = detail {
        let _ = writeln!(out, "{} {d}", m::DETAIL_LABEL[l]);
    }
    let _ = writeln!(out, "{}", m::FOOTER[l]);
    let _ = writeln!(out, "[omni] fatal: {}", m::LANGS[l]);

    if EXIT_ON_FATAL.load(Ordering::Relaxed) {
        sbi::shutdown(true);
    }
    halt()
}

/// Stops this hart for good (interrupts off, so the report stays on screen).
pub fn halt() -> ! {
    // SAFETY: masking interrupts and waiting is always safe here.
    unsafe { core::arch::asm!("csrc sstatus, {}", in(reg) 1usize << 1) };
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}
