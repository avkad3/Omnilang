//! OmniLang kernel: a minimal RISC-V supervisor-mode kernel for QEMU `virt`
//! whose diagnostics are shown in the user's language.

#![no_std]
#![no_main]

mod bootargs;
mod sbi;
#[macro_use]
mod uart;

#[allow(dead_code)] // not every message is used by every build
mod messages {
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}

use core::arch::global_asm;
use core::panic::PanicInfo;

use bootargs::BootArgs;

// Entry point: set up the stack, clear .bss, and call `kmain`.
// OpenSBI passes the hart id in a0 and the device tree address in a1;
// neither register is touched before the call.
global_asm!(
    r#"
    .section .text.entry
    .globl _start
_start:
    la      sp, __stack_top
    la      t0, __bss_start
    la      t1, __bss_end
1:  bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:  call    kmain
3:  wfi
    j       3b
"#
);

#[no_mangle]
extern "C" fn kmain(hart_id: usize, dtb: usize) -> ! {
    // SAFETY: `dtb` comes straight from the firmware.
    let args = unsafe { BootArgs::from_device_tree(dtb) };
    let lang = args.lang;
    println!("[omni] {} (hart {hart_id}, lang={})", messages::BOOT[lang], messages::LANGS[lang]);
    sbi::shutdown(false);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[omni] panic: {info}");
    sbi::shutdown(true);
}
