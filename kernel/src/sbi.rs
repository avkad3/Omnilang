//! Calls into the Supervisor Binary Interface (OpenSBI on QEMU).

use core::arch::asm;

const EXT_TIME: usize = 0x5449_4D45; // "TIME"
const EXT_SRST: usize = 0x5352_5354; // "SRST" system reset

/// Returns (error, value) per the SBI calling convention.
fn call(ext: usize, func: usize, a0: usize, a1: usize) -> (isize, usize) {
    let (error, value): (usize, usize);
    // SAFETY: an SBI ecall only clobbers a0/a1 as declared.
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") a0 => error,
            inlateout("a1") a1 => value,
            in("a6") func,
            in("a7") ext,
        );
    }
    (error as isize, value)
}

/// Arms the supervisor timer interrupt for absolute time `stime`.
pub fn set_timer(stime: u64) {
    call(EXT_TIME, 0, stime as usize, 0);
}

/// Powers the machine off. `failure` reports an error exit status to QEMU.
pub fn shutdown(failure: bool) -> ! {
    // Reset type 0 = shutdown; reason 0 = none, 1 = system failure.
    call(EXT_SRST, 0, 0, failure as usize);
    loop {
        // SAFETY: wait for interrupt; nothing else to do.
        unsafe { asm!("wfi") };
    }
}
