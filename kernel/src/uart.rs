//! NS16550A serial port driver (QEMU `virt` puts one at 0x1000_0000).

use core::fmt;

const BASE: usize = 0x1000_0000;
const THR: usize = 0; // transmit holding register
const LSR: usize = 5; // line status register
const LSR_THR_EMPTY: u8 = 1 << 5;

pub struct Uart;

impl Uart {
    pub fn put_byte(&self, b: u8) {
        // SAFETY: fixed MMIO addresses of the virt machine's UART.
        unsafe {
            let lsr = (BASE + LSR) as *const u8;
            while lsr.read_volatile() & LSR_THR_EMPTY == 0 {}
            ((BASE + THR) as *mut u8).write_volatile(b);
        }
    }
}

impl fmt::Write for Uart {
    /// Strings go out as raw UTF-8: a modern terminal on the host shapes
    /// Devanagari and Tamil correctly, so serial logs are readable as-is.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.put_byte(b'\r');
            }
            self.put_byte(b);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = write!($crate::uart::Uart, $($arg)*);
    }};
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}
