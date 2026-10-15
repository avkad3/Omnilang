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

/// QEMU `virt` "sifive_test" device: writing to it ends the emulator with
/// a chosen exit status, which lets automated tests tell pass from fail.
const QEMU_TEST_DEVICE: usize = 0x10_0000;
const QEMU_PASS: u32 = 0x5555;
const QEMU_FAIL: u32 = 0x3333;

/// Powers the machine off. `failure` makes QEMU exit with status 1.
pub fn shutdown(failure: bool) -> ! {
    let code = if failure { (1 << 16) | QEMU_FAIL } else { QEMU_PASS };
    // SAFETY: fixed MMIO address of the test device on QEMU virt.
    unsafe { (QEMU_TEST_DEVICE as *mut u32).write_volatile(code) };
    // Fallback if the test device is absent: ask the firmware. Reset type 0
    // = shutdown; reason 1 = system failure.
    call(EXT_SRST, 0, 0, failure as usize);
    loop {
        // SAFETY: wait for interrupt; nothing else to do.
        unsafe { asm!("wfi") };
    }
}
