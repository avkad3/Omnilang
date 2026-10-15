//! OmniLang kernel: a minimal RISC-V supervisor-mode kernel for QEMU `virt`
//! whose diagnostics are shown in the user's language.

#![no_std]
#![no_main]

mod bootargs;
mod font;
mod mm;
mod ramfb;
mod report;
mod sbi;
mod screen;
mod trap;
#[macro_use]
mod uart;

#[allow(dead_code)] // not every message is used by every build
mod messages {
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

use bootargs::{BootArgs, Demo};
use report::Cause;

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
    report::configure(args.lang, args.exit);
    let lang = args.lang;
    println!(
        "[omni] {} (hart {hart_id}, lang={})",
        messages::BOOT[lang],
        messages::LANGS[lang]
    );

    mm::init(args.ram.0, args.ram.1);
    println!(
        "[omni] paging on: {} MiB of RAM, kernel sections protected",
        args.ram.1 >> 20
    );

    match ramfb::init() {
        Some(fb) => {
            screen::boot(fb, lang);
            report::set_framebuffer(fb);
        }
        None => println!("[omni] no framebuffer (start QEMU with -device ramfb)"),
    }

    trap::init();
    // Let the timer run for a moment so the report has a real uptime.
    while trap::ticks() < 3 {
        // SAFETY: wait for the next interrupt.
        unsafe { asm!("wfi") };
    }

    match args.demo {
        Some(demo) => trigger(demo),
        None if args.exit => sbi::shutdown(false),
        None => {
            println!("[omni] idle");
            loop {
                unsafe { asm!("wfi") };
            }
        }
    }
}

/// Deliberately faults, to demonstrate the panic screen.
fn trigger(demo: Demo) -> ! {
    // SAFETY: none of this is safe; that is the point. Each access faults
    // and the trap handler never returns to it.
    unsafe {
        match demo {
            Demo::Illegal => asm!("unimp"),
            // Address 0 is never mapped.
            Demo::Load => {
                core::ptr::read_volatile(core::ptr::null::<u64>());
            }
            Demo::Store => core::ptr::write_volatile(core::ptr::null_mut::<u64>(), 0xdead),
            // Kernel code is mapped read/execute only. A byte write: with
            // compressed instructions, functions are only 2-byte aligned.
            Demo::ReadOnly => core::ptr::write_volatile(trigger as *const () as *mut u8, 0),
            // Atomic operations must be naturally aligned; firmware does not
            // emulate misaligned atomics, so this reaches the kernel.
            Demo::Misaligned => {
                let word = 0x8020_0001usize as *mut u32;
                asm!("amoadd.w zero, zero, ({})", in(reg) word);
            }
            Demo::Panic => {
                let ticks = trap::ticks();
                assert!(ticks == 0, "uptime must be zero, but is {ticks}");
            }
        }
    }
    unreachable!("demo fault did not trap")
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    report::fatal(Cause::KernelBug, None, None, Some(&format_args!("{info}")));
}
