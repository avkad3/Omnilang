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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Demo {
    Illegal,
    Load,
    Store,
    Misaligned,
    Panic,
}

impl BootArgs {
    pub fn parse(cmdline: &str) -> BootArgs {
        let mut args = BootArgs { lang: 0, demo: None, exit: false };
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
        let cmdline = fdt::Fdt::from_ptr(dtb as *const u8).ok().and_then(|f| f.chosen().bootargs());
        BootArgs::parse(cmdline.unwrap_or(""))
    }
}
