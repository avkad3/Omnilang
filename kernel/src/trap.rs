//! Trap entry, timer interrupts and exception dispatch.

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU64, Ordering};

use crate::report::{self, Cause};
use crate::sbi;

/// QEMU `virt` timer frequency.
const TIMEBASE_HZ: u64 = 10_000_000;
pub const TICK_HZ: u64 = 10;

static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Registers saved on entry to a trap.
#[repr(C)]
pub struct TrapFrame {
    /// x0..x31 (x0 slot unused; x2 holds the interrupted sp).
    pub regs: [usize; 32],
    pub sepc: usize,
    pub sstatus: usize,
}

const FRAME_SIZE: usize = core::mem::size_of::<TrapFrame>();

// Saves every register on the current stack, calls `trap_handler`, and
// restores them. Kernel-only, so the kernel stack is used directly.
global_asm!(
    r#"
    .altmacro
    .macro SAVE_REG n
        sd x\n, \n*8(sp)
    .endm
    .macro LOAD_REG n
        ld x\n, \n*8(sp)
    .endm

    .section .text
    .globl trap_entry
    .align 4
trap_entry:
    addi    sp, sp, -{frame}
    sd      x1, 1*8(sp)
    .set n, 3
    .rept 29
        SAVE_REG %n
        .set n, n + 1
    .endr
    addi    t0, sp, {frame}
    sd      t0, 2*8(sp)
    csrr    t0, sepc
    sd      t0, 32*8(sp)
    csrr    t1, sstatus
    sd      t1, 33*8(sp)

    mv      a0, sp
    call    trap_handler

    ld      t0, 32*8(sp)
    csrw    sepc, t0
    ld      t1, 33*8(sp)
    csrw    sstatus, t1
    ld      x1, 1*8(sp)
    .set n, 3
    .rept 29
        LOAD_REG %n
        .set n, n + 1
    .endr
    addi    sp, sp, {frame}
    sret
"#,
    frame = const FRAME_SIZE,
);

extern "C" {
    fn trap_entry();
}

fn read_time() -> u64 {
    let t: u64;
    // SAFETY: reading the time CSR has no side effects.
    unsafe { asm!("rdtime {}", out(reg) t) };
    t
}

fn arm_timer() {
    sbi::set_timer(read_time() + TIMEBASE_HZ / TICK_HZ);
}

/// Installs the trap vector and starts the periodic timer interrupt.
pub fn init() {
    // SAFETY: trap_entry is a valid, 4-byte aligned direct-mode vector;
    // enabling STIE/SIE only lets timer interrupts reach trap_entry.
    unsafe {
        asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize);
        arm_timer();
        asm!("csrs sie, {}", in(reg) 1usize << 5); // STIE
        asm!("csrs sstatus, {}", in(reg) 1usize << 1); // SIE
    }
}

const INTERRUPT: usize = 1 << 63;
const SUPERVISOR_TIMER: usize = 5;

#[no_mangle]
extern "C" fn trap_handler(frame: &mut TrapFrame) {
    let (scause, stval): (usize, usize);
    // SAFETY: reading trap CSRs.
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
    }

    if scause & INTERRUPT != 0 {
        if scause & !INTERRUPT == SUPERVISOR_TIMER {
            TICKS.fetch_add(1, Ordering::Relaxed);
            arm_timer();
            return;
        }
        report::fatal(Cause::Unknown(scause), Some(frame.sepc), None, None);
    }

    let cause = match scause {
        2 => Cause::IllegalInstruction,
        1 | 5 => Cause::LoadAccess,
        7 => Cause::StoreAccess,
        0 | 4 | 6 => Cause::Misaligned,
        12 | 13 | 15 => Cause::PageFault,
        other => Cause::Unknown(other),
    };
    let addr = match cause {
        Cause::IllegalInstruction | Cause::Unknown(_) => None,
        _ => Some(stval),
    };
    report::fatal(cause, Some(frame.sepc), addr, None);
}
