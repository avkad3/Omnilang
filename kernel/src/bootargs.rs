//! Kernel command line, read from the device tree's `/chosen/bootargs`
//! (QEMU: `-append "lang=hi demo=illegal"`).

/// Options recognised on the command line.
#[derive(Clone, Copy)]
pub struct BootArgs {
    /// Index into `messages::LANGS`.
    pub lang: usize,
    /// Deliberately crash after boot, to demonstrate the panic screen.
    pub demo: Option<Demo>,
    /// Power off after the panic screen instead of halting (for tests).
    pub exit: bool,
    /// First RAM region from the device tree: (start, size).
    pub ram: (usize, usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Demo {
    Illegal,
    Load,
    Store,
    Misaligned,
    ReadOnly,
    Panic,
}

impl BootArgs {
    pub fn parse(cmdline: &str) -> BootArgs {
        let mut args = BootArgs {
            lang: 0,
            demo: None,
            exit: false,
            ram: (0x8000_0000, 128 << 20),
        };
        for word in cmdline.split_ascii_whitespace() {
            match word.split_once('=') {
                Some(("lang", v)) => {
                    if let Some(i) = crate::messages::LANGS.iter().position(|&l| l == v) {
                        args.lang = i;
                    }
                }
                Some(("demo", v)) => {
                    args.demo = match v {
                        "illegal" => Some(Demo::Illegal),
                        "load" => Some(Demo::Load),
                        "store" => Some(Demo::Store),
                        "misaligned" => Some(Demo::Misaligned),
                        "readonly" => Some(Demo::ReadOnly),
                        "panic" => Some(Demo::Panic),
                        _ => None,
                    }
                }
                None if word == "exit" => args.exit = true,
                _ => {}
            }
        }
        args
    }

    /// Reads `/chosen/bootargs` from the flattened device tree at `dtb`.
    ///
    /// # Safety
    /// `dtb` must be the device tree pointer passed in by the firmware.
    pub unsafe fn from_device_tree(dtb: usize) -> BootArgs {
        let Ok(tree) = fdt::Fdt::from_ptr(dtb as *const u8) else {
            return BootArgs::parse("");
        };
        let mut args = BootArgs::parse(tree.chosen().bootargs().unwrap_or(""));
        if let Some(region) = tree.memory().regions().next() {
            if let Some(size) = region.size {
                args.ram = (region.starting_address as usize, size);
            }
        }
        args
    }
}
