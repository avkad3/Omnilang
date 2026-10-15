//! Pre-shaped text produced by `build.rs`.
//!
//! Each [`Bitmap`] is a finished line of text (or one atlas glyph) as 8-bit
//! coverage, run-length encoded: `0x00 n` = `n` transparent pixels, any
//! other byte = one pixel's coverage.

static BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/glyphs.bin"));

#[derive(Clone, Copy)]
pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    /// Distance from the top of the bitmap to the text baseline.
    pub baseline: u16,
    /// Horizontal distance to the next glyph (atlas glyphs).
    pub advance: u16,
    offset: u32,
    len: u32,
}

impl Bitmap {
    pub const fn new(width: u16, height: u16, baseline: u16, advance: u16, offset: u32, len: u32) -> Bitmap {
        Bitmap {
            width,
            height,
            baseline,
            advance,
            offset,
            len,
        }
    }

    /// Coverage values in row-major order.
    pub fn pixels(&self) -> Pixels {
        let start = self.offset as usize;
        Pixels {
            data: &BLOB[start..start + self.len as usize],
            pos: 0,
            zeros: 0,
        }
    }
}

pub struct Pixels {
    data: &'static [u8],
    pos: usize,
    zeros: u8,
}

impl Iterator for Pixels {
    type Item = u8;

    fn next(&mut self) -> Option<u8> {
        if self.zeros > 0 {
            self.zeros -= 1;
            return Some(0);
        }
        let b = *self.data.get(self.pos)?;
        if b == 0 {
            self.zeros = self.data[self.pos + 1] - 1;
            self.pos += 2;
            Some(0)
        } else {
            self.pos += 1;
            Some(b)
        }
    }
}

#[allow(dead_code)] // the screen uses a subset of the messages
pub mod text {
    use super::Bitmap;
    include!(concat!(env!("OUT_DIR"), "/glyphs.rs"));
}
