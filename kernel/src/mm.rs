//! Physical frames and Sv39 paging.
//!
//! The kernel is identity-mapped (virtual = physical) with per-section
//! permissions: code is read/execute, read-only data is read-only, and data,
//! .bss, the stack and the rest of RAM are read/write. Device registers are
//! mapped read/write. Address 0 and everything else stays unmapped, so a
//! null pointer or a write into the kernel's own code is caught as a page
//! fault instead of silently corrupting memory.

use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const PAGE_SIZE: usize = 4096;

const V: u64 = 1 << 0;
const R: u64 = 1 << 1;
const W: u64 = 1 << 2;
const X: u64 = 1 << 3;
const G: u64 = 1 << 5;
// Accessed/Dirty are set up front: hardware need not maintain them.
const A: u64 = 1 << 6;
const D: u64 = 1 << 7;

pub const READ_ONLY: u64 = R | A;
pub const READ_EXEC: u64 = R | X | A;
pub const READ_WRITE: u64 = R | W | A | D;

/// Device MMIO regions used by the kernel: (base, size).
const DEVICES: [(usize, usize); 3] = [
    (0x0010_0000, PAGE_SIZE), // QEMU test device (power off)
    (0x1000_0000, PAGE_SIZE), // UART
    (0x1010_0000, PAGE_SIZE), // fw_cfg
];

extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __kernel_end: u8;
}

fn addr_of(sym: &u8) -> usize {
    sym as *const u8 as usize
}

// ---------------------------------------------------------------- frames

/// Bump allocator over the RAM after the kernel image.
static NEXT_FRAME: AtomicUsize = AtomicUsize::new(0);
static RAM_END: AtomicUsize = AtomicUsize::new(0);

/// Returns a zeroed, page-aligned physical frame.
pub fn alloc_frame() -> usize {
    let frame = NEXT_FRAME.fetch_add(PAGE_SIZE, Ordering::Relaxed);
    assert!(frame + PAGE_SIZE <= RAM_END.load(Ordering::Relaxed), "out of physical memory");
    // SAFETY: the frame lies in free RAM above the kernel and is handed out once.
    unsafe { core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE) };
    frame
}

// ---------------------------------------------------------------- page tables

#[repr(C, align(4096))]
struct Table([u64; 512]);

static ROOT: AtomicUsize = AtomicUsize::new(0);

fn table(addr: usize) -> &'static mut Table {
    // SAFETY: page tables are frames from alloc_frame, identity-mapped.
    unsafe { &mut *(addr as *mut Table) }
}

fn vpn(va: usize, level: usize) -> usize {
    (va >> (12 + 9 * level)) & 0x1ff
}

fn pte_addr(pte: u64) -> usize {
    ((pte >> 10) << 12) as usize
}

/// Maps one 4 KiB page `va` → `pa` with permission `flags`.
fn map_page(root: usize, va: usize, pa: usize, flags: u64) {
    let mut t = root;
    for level in [2, 1] {
        let entry = &mut table(t).0[vpn(va, level)];
        if *entry & V == 0 {
            let next = alloc_frame();
            *entry = ((next as u64 >> 12) << 10) | V;
        }
        t = pte_addr(*entry);
    }
    table(t).0[vpn(va, 0)] = ((pa as u64 >> 12) << 10) | flags | G | V;
}

fn map_range(root: usize, start: usize, end: usize, flags: u64) {
    let mut va = start & !(PAGE_SIZE - 1);
    while va < end {
        map_page(root, va, va, flags);
        va += PAGE_SIZE;
    }
}

/// Leaf PTE for `va`, if mapped.
fn lookup(va: usize) -> Option<u64> {
    let mut t = ROOT.load(Ordering::Relaxed);
    if t == 0 {
        return None;
    }
    for level in [2, 1, 0] {
        let entry = table(t).0[vpn(va, level)];
        if entry & V == 0 {
            return None;
        }
        if entry & (R | X) != 0 || level == 0 {
            return Some(entry);
        }
        t = pte_addr(entry);
    }
    None
}

/// True if `va` is mapped but not writable (for fault reports).
pub fn is_read_only(va: usize) -> bool {
    lookup(va).is_some_and(|pte| pte & W == 0)
}

/// Builds the kernel page table and turns on Sv39 translation.
pub fn init(ram_start: usize, ram_size: usize) {
    // SAFETY: linker-provided symbols; only their addresses are used.
    let (text, text_end, ro, ro_end, data, kernel_end) = unsafe {
        (
            addr_of(&__text_start),
            addr_of(&__text_end),
            addr_of(&__rodata_start),
            addr_of(&__rodata_end),
            addr_of(&__data_start),
            addr_of(&__kernel_end),
        )
    };
    let ram_end = ram_start + ram_size;
    NEXT_FRAME.store((kernel_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1), Ordering::Relaxed);
    RAM_END.store(ram_end, Ordering::Relaxed);

    let root = alloc_frame();
    map_range(root, text, text_end, READ_EXEC);
    map_range(root, ro, ro_end, READ_ONLY);
    // .data, .bss (including the framebuffer) and the boot stack.
    map_range(root, data, kernel_end, READ_WRITE);
    // Free RAM, used for page tables and future allocations.
    map_range(root, kernel_end, ram_end, READ_WRITE);
    // OpenSBI owns the RAM below the kernel; leave it unmapped.
    for (base, size) in DEVICES {
        map_range(root, base, base + size, READ_WRITE);
    }
    ROOT.store(root, Ordering::Relaxed);

    let satp = (8usize << 60) | (root >> 12); // mode 8 = Sv39
    // SAFETY: the new table identity-maps everything the kernel is
    // currently executing and touching, so execution continues seamlessly.
    unsafe {
        asm!("csrw satp, {}", "sfence.vma", in(reg) satp);
    }
}
