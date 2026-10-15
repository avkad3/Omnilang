//! The framebuffer panic screen.

use crate::font::{text as t, Bitmap};
use crate::ramfb::{HEIGHT, WIDTH};
use crate::report::Cause;

const BACKGROUND: u32 = 0x0014_1f3d; // deep blue
const PANEL: u32 = 0x001b_2a52;
const ACCENT: u32 = 0x00e0_5a47; // warm red stripe
const TEXT: u32 = 0x00ff_ffff;
const MUTED: u32 = 0x00b8_c4e0;
const VALUE: u32 = 0x00ff_d27a; // addresses and numbers

struct Canvas<'a> {
    fb: &'a mut [u32],
}

impl Canvas<'_> {
    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        for row in y..(y + h).min(HEIGHT) {
            let start = row * WIDTH + x.min(WIDTH);
            let end = row * WIDTH + (x + w).min(WIDTH);
            self.fb[start..end].fill(color);
        }
    }

    /// Alpha-blends `bmp` in `color` with its baseline at `baseline_y`.
    /// Returns the x position after the bitmap.
    fn draw(&mut self, bmp: &Bitmap, x: usize, baseline_y: usize, color: u32) -> usize {
        let top = baseline_y.saturating_sub(bmp.baseline as usize);
        let (w, h) = (bmp.width as usize, bmp.height as usize);
        let mut px = bmp.pixels();
        for row in 0..h {
            for col in 0..w {
                let a = px.next().unwrap_or(0) as u32;
                let (fx, fy) = (x + col, top + row);
                if a == 0 || fx >= WIDTH || fy >= HEIGHT {
                    continue;
                }
                let dst = &mut self.fb[fy * WIDTH + fx];
                *dst = blend(*dst, color, a);
            }
        }
        x + w
    }

    /// Monospace ASCII (addresses, panic details). Returns the new x.
    fn mono(&mut self, s: &str, mut x: usize, baseline_y: usize, color: u32, max_x: usize) -> usize {
        for c in s.bytes() {
            let c = if (0x20..0x7f).contains(&c) { c } else { b'?' };
            let g = &t::MONO[(c - 0x20) as usize];
            if x + g.advance as usize > max_x {
                break;
            }
            self.draw(g, x, baseline_y, color);
            x += g.advance as usize;
        }
        x
    }

    /// A decimal number in the language's own digits.
    fn decimal(&mut self, mut n: u64, lang: usize, mut x: usize, baseline_y: usize, color: u32) {
        let mut digits = [0usize; 20];
        let mut len = 0;
        loop {
            digits[len] = (n % 10) as usize;
            len += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        for &d in digits[..len].iter().rev() {
            let g = &t::DIGITS[lang][d];
            self.draw(g, x, baseline_y, color);
            x += g.advance as usize;
        }
    }

    fn hex(&mut self, v: usize, x: usize, baseline_y: usize, color: u32) {
        let mut buf = [0u8; 18];
        buf[0] = b'0';
        buf[1] = b'x';
        for i in 0..16 {
            let nibble = (v >> ((15 - i) * 4)) & 0xf;
            buf[2 + i] = b"0123456789abcdef"[nibble];
        }
        // SAFETY of from_utf8: only ASCII bytes were written.
        let s = core::str::from_utf8(&buf).unwrap_or("?");
        self.mono(s, x, baseline_y, color, WIDTH);
    }
}

fn blend(dst: u32, src: u32, a: u32) -> u32 {
    let mix = |shift: u32| {
        let d = (dst >> shift) & 0xff;
        let s = (src >> shift) & 0xff;
        ((s * a + d * (255 - a)) / 255) << shift
    };
    mix(16) | mix(8) | mix(0)
}

pub struct Info<'a> {
    pub lang: usize,
    pub cause: Cause,
    pub pc: Option<usize>,
    pub addr: Option<usize>,
    pub ticks: u64,
    /// ASCII detail text (Rust panic message), wrapped across lines.
    pub detail: Option<&'a str>,
}

/// The boot screen: a localized "kernel started" message.
pub fn boot(fb: &mut [u32], lang: usize) {
    let mut c = Canvas { fb };
    c.fill(0, 0, WIDTH, HEIGHT, BACKGROUND);
    let msg = &t::BOOT[lang];
    let x = (WIDTH.saturating_sub(msg.width as usize)) / 2;
    c.draw(msg, x, HEIGHT / 2, TEXT);
}

/// Draws the whole panic screen.
pub fn draw(fb: &mut [u32], info: &Info) {
    let l = info.lang;
    let mut c = Canvas { fb };
    let margin = 56;

    c.fill(0, 0, WIDTH, HEIGHT, BACKGROUND);
    c.fill(0, 0, WIDTH, 8, ACCENT);

    let mut y = 96;
    c.draw(&t::TITLE[l], margin, y, TEXT);
    y += 44;
    c.draw(&t::SUBTITLE[l], margin, y, MUTED);

    // Panel with the cause and the machine state, sized to its rows.
    let value_rows = 1 + info.pc.is_some() as usize + info.addr.is_some() as usize;
    let panel_top = y + 30;
    let panel_h = 44 + 48 + (value_rows - 1) * 40 + 30;
    c.fill(margin - 20, panel_top, WIDTH - 2 * (margin - 20), panel_h, PANEL);
    c.fill(margin - 20, panel_top, 4, panel_h, ACCENT);

    y = panel_top + 44;
    let x = c.draw(&t::CAUSE_LABEL[l], margin, y, MUTED);
    c.draw(&info.cause.message_bitmap()[l], x + 12, y, TEXT);

    let value_x = margin + 260;
    y += 48;
    if let Some(pc) = info.pc {
        c.draw(&t::PC_LABEL[l], margin, y, MUTED);
        c.hex(pc, value_x, y, VALUE);
        y += 40;
    }
    if let Some(addr) = info.addr {
        c.draw(&t::ADDR_LABEL[l], margin, y, MUTED);
        c.hex(addr, value_x, y, VALUE);
        y += 40;
    }
    c.draw(&t::TICKS_LABEL[l], margin, y, MUTED);
    c.decimal(info.ticks, l, value_x, y, VALUE);

    // Free-form details (Rust panic messages are English/ASCII by nature).
    let mut y = panel_top + panel_h + 44;
    if let Some(detail) = info.detail {
        let x = c.draw(&t::DETAIL_LABEL[l], margin, y, MUTED) + 12;
        for line in wrap(detail, (WIDTH - margin - x) / t::MONO[0].advance as usize).take(3) {
            c.mono(line, x, y, TEXT, WIDTH - margin);
            y += 26;
        }
        y += 16;
    }

    c.draw(&t::FOOTER[l], margin, (y + 20).max(HEIGHT - 48), MUTED);
}

/// Splits `s` into lines of at most `width` characters (breaking at
/// newlines, then hard-wrapping).
fn wrap(s: &str, width: usize) -> impl Iterator<Item = &str> {
    let width = width.max(1);
    s.split('\n').flat_map(move |line| {
        let mut rest = line;
        core::iter::from_fn(move || {
            if rest.is_empty() {
                return None;
            }
            let cut = rest.char_indices().nth(width).map_or(rest.len(), |(i, _)| i);
            let (head, tail) = rest.split_at(cut);
            rest = tail;
            Some(head)
        })
    })
}
