//! QEMU `ramfb`: a framebuffer in guest RAM, configured through `fw_cfg`.
//!
//! The kernel reserves the pixel memory and tells QEMU where it is by
//! writing a `RAMFBCfg` record into the `etc/ramfb` fw_cfg file with a DMA
//! transfer. All fw_cfg fields are big-endian.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

const FW_CFG: usize = 0x1010_0000; // QEMU virt fw_cfg MMIO base
const FW_CFG_DATA: usize = FW_CFG;
const FW_CFG_SELECTOR: usize = FW_CFG + 0x08;
const FW_CFG_DMA_HIGH: usize = FW_CFG + 0x10;
const FW_CFG_DMA_LOW: usize = FW_CFG + 0x14;

const SELECT_SIGNATURE: u16 = 0x0000;
const SELECT_FILE_DIR: u16 = 0x0019;

const DMA_ERROR: u32 = 0x01;
const DMA_SELECT: u32 = 0x08;
const DMA_WRITE: u32 = 0x10;

/// DRM fourcc 'XR24': 32-bit pixels, 0x00RRGGBB.
const FORMAT_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");

pub const WIDTH: usize = 800;
pub const HEIGHT: usize = 600;

#[repr(C, align(4096))]
struct Pixels([u32; WIDTH * HEIGHT]);

static mut FRAMEBUFFER: Pixels = Pixels([0; WIDTH * HEIGHT]);

#[repr(C, packed)]
struct RamFbCfg {
    addr: u64,
    fourcc: u32,
    flags: u32,
    width: u32,
    height: u32,
    stride: u32,
}

#[repr(C, align(8))]
struct DmaAccess {
    control: u32,
    length: u32,
    address: u64,
}

fn select(key: u16) {
    // SAFETY: fw_cfg MMIO register on QEMU virt.
    unsafe { write_volatile(FW_CFG_SELECTOR as *mut u16, key.to_be()) };
}

fn read_bytes(buf: &mut [u8]) {
    for b in buf {
        // SAFETY: each read of the data register returns the next byte.
        *b = unsafe { read_volatile(FW_CFG_DATA as *const u8) };
    }
}

fn read_be32() -> u32 {
    let mut b = [0; 4];
    read_bytes(&mut b);
    u32::from_be_bytes(b)
}

/// Finds a fw_cfg file by name; returns its selector key.
fn find_file(name: &[u8]) -> Option<u16> {
    select(SELECT_FILE_DIR);
    let count = read_be32();
    for _ in 0..count {
        let mut entry = [0u8; 64]; // size (4), select (2), reserved (2), name (56)
        read_bytes(&mut entry);
        let key = u16::from_be_bytes([entry[4], entry[5]]);
        let entry_name = &entry[8..];
        let len = entry_name.iter().position(|&c| c == 0).unwrap_or(56);
        if &entry_name[..len] == name {
            return Some(key);
        }
    }
    None
}

/// Configures the display and returns the framebuffer, or `None` when QEMU
/// was started without `-device ramfb`.
pub fn init() -> Option<&'static mut [u32]> {
    select(SELECT_SIGNATURE);
    let mut sig = [0u8; 4];
    read_bytes(&mut sig);
    if &sig != b"QEMU" {
        return None;
    }
    let key = find_file(b"etc/ramfb")?;

    // SAFETY: the framebuffer is only handed out here, once, and the kernel
    // runs on a single hart without paging, so its address is physical.
    let fb: &'static mut [u32] = unsafe { &mut (*core::ptr::addr_of_mut!(FRAMEBUFFER)).0 };
    let cfg = RamFbCfg {
        addr: (fb.as_ptr() as u64).to_be(),
        fourcc: FORMAT_XRGB8888.to_be(),
        flags: 0,
        width: (WIDTH as u32).to_be(),
        height: (HEIGHT as u32).to_be(),
        stride: ((WIDTH * 4) as u32).to_be(),
    };
    let dma = DmaAccess {
        control: (((key as u32) << 16) | DMA_SELECT | DMA_WRITE).to_be(),
        length: (core::mem::size_of::<RamFbCfg>() as u32).to_be(),
        address: (&cfg as *const RamFbCfg as u64).to_be(),
    };
    let dma_addr = &dma as *const DmaAccess as u64;
    fence(Ordering::SeqCst);
    // SAFETY: starting a fw_cfg DMA transfer of `cfg`; writing the low half
    // of the address triggers it, and QEMU completes it synchronously.
    unsafe {
        write_volatile(FW_CFG_DMA_HIGH as *mut u32, ((dma_addr >> 32) as u32).to_be());
        write_volatile(FW_CFG_DMA_LOW as *mut u32, (dma_addr as u32).to_be());
        while u32::from_be(read_volatile(&dma.control)) & !DMA_ERROR != 0 {}
        if u32::from_be(read_volatile(&dma.control)) & DMA_ERROR != 0 {
            return None;
        }
    }
    Some(fb)
}
