use anyhow::{Result, ensure};
use rayon::prelude::*;

/// Top-down packed BGRA pixels. Alpha is ignored for screen capture.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Frame {
    pub fn nv12(&self) -> Result<Vec<u8>> {
        let (w, h) = (self.width as usize, self.height as usize);
        ensure!(
            w >= 2 && h >= 2 && w % 2 == 0 && h % 2 == 0 && self.bgra.len() == w * h * 4,
            "Invalid BGRA frame"
        );
        let mut out = vec![0; w * h * 3 / 2];
        let (y, uv) = out.split_at_mut(w * h);
        y.par_chunks_mut(w).enumerate().for_each(|(row, line)| {
            for (x, value) in line.iter_mut().enumerate() {
                let p = &self.bgra[(row * w + x) * 4..];
                *value = (((66 * p[2] as i32 + 129 * p[1] as i32 + 25 * p[0] as i32 + 128) >> 8)
                    + 16)
                    .clamp(0, 255) as u8;
            }
        });
        uv.par_chunks_mut(w).enumerate().for_each(|(row, line)| {
            for x in (0..w).step_by(2) {
                let (mut r, mut g, mut b) = (0, 0, 0);
                for dy in 0..2 {
                    for dx in 0..2 {
                        let p = &self.bgra[((row * 2 + dy) * w + x + dx) * 4..];
                        r += p[2] as i32;
                        g += p[1] as i32;
                        b += p[0] as i32;
                    }
                }
                r /= 4;
                g /= 4;
                b /= 4;
                line[x] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
                line[x + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            }
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nv12_limited_range_black_white_and_chroma() {
        let f = |value| {
            Frame {
                width: 2,
                height: 2,
                bgra: vec![value; 16],
            }
            .nv12()
            .unwrap()
        };
        assert_eq!(f(0), [16, 16, 16, 16, 128, 128]);
        assert_eq!(f(255), [235, 235, 235, 235, 128, 128]);
    }
}
