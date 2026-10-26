//! A Doom frame to terminal cells: fitting it to the terminal keeping its
//! aspect ratio and resampling it into a pixel buffer, which intuiTUIve's
//! `pixels` node draws as half blocks or Braille.
//!
//! The engine's framebuffer is 320x200 XRGB (`0x00RRGGBB`), shown by the
//! original game at 4:3 (its pixels are 1.2 times taller than wide).

use intuituive::pixels::{PixelBuf, CLEAR};

/// A frame from the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// `width * height` pixels, `0x00RRGGBB`, row by row.
    pub pixels: Vec<u32>,
    /// Counts frames, from 1.
    pub seq: u64,
}

impl Frame {
    /// A frame of one colour.
    pub fn filled(width: usize, height: usize, xrgb: u32) -> Frame {
        Frame {
            width,
            height,
            pixels: vec![xrgb; width * height],
            seq: 0,
        }
    }

    /// The pixels as RGBA bytes, for the image encoders.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &p in &self.pixels {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]);
        }
        out
    }
}

/// How the picture fills the terminal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// Width over height of the picture as shown: 4:3 for Doom, or `None`
    /// to stretch over every cell.
    pub aspect: Option<f64>,
    /// A cell's height over its width (2.0 for most fonts).
    pub cell_aspect: f64,
}

impl Default for Fit {
    fn default() -> Self {
        Fit {
            aspect: Some(4.0 / 3.0),
            cell_aspect: 2.0,
        }
    }
}

/// A rectangle of cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CellRect {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

impl Fit {
    /// The cells the picture takes in a `cols` x `rows` area, centred.
    pub fn place(&self, cols: u16, rows: u16) -> CellRect {
        if cols == 0 || rows == 0 {
            return CellRect::default();
        }
        let Some(aspect) = self.aspect else {
            return CellRect {
                x: 0,
                y: 0,
                cols,
                rows,
            };
        };
        let cell = self.cell_aspect.max(0.1);
        // Cells wide for the full height, and rows tall for the full width.
        let want_cols = (rows as f64 * cell * aspect).round().max(1.0);
        let (c, r) = if want_cols <= cols as f64 {
            (want_cols as u16, rows)
        } else {
            let want_rows = (cols as f64 / (cell * aspect))
                .round()
                .clamp(1.0, rows as f64);
            (cols, want_rows as u16)
        };
        CellRect {
            x: (cols - c) / 2,
            y: (rows - r) / 2,
            cols: c,
            rows: r,
        }
    }
}

/// Resamples frames to one target size. By default each target pixel is
/// the source pixel at its centre (nearest neighbour), so the colours stay
/// the frame's own 256 (and a frame's cells use few styles).
/// [`averaging`](Self::averaging) averages the source pixels each target
/// pixel covers instead, for the image protocols. The tables are kept, so
/// a frame of the same size costs one pass.
#[derive(Clone, Debug, Default)]
pub struct Resampler {
    average: bool,
    key: (usize, usize, usize, usize),
    xs: Vec<(usize, usize)>,
    ys: Vec<(usize, usize)>,
    out: Vec<[u8; 3]>,
}

/// The source range each of `to` target pixels takes from `from`: its
/// centre pixel, or every pixel it covers when `average`.
fn spans(from: usize, to: usize, average: bool) -> Vec<(usize, usize)> {
    (0..to)
        .map(|i| {
            if average {
                let start = i * from / to;
                (start, ((i + 1) * from / to).max(start + 1).min(from))
            } else {
                let at = ((2 * i + 1) * from / (2 * to)).min(from.saturating_sub(1));
                (at, at + 1)
            }
        })
        .collect()
}

impl Resampler {
    /// Nearest neighbour.
    pub fn new() -> Resampler {
        Resampler::default()
    }

    /// Averaging the pixels each target pixel covers.
    pub fn averaging() -> Resampler {
        Resampler {
            average: true,
            ..Resampler::default()
        }
    }

    /// `frame` resampled to `width` x `height` RGB pixels, row by row.
    pub fn resample(&mut self, frame: &Frame, width: usize, height: usize) -> &[[u8; 3]] {
        let key = (frame.width, frame.height, width, height);
        if self.key != key {
            self.key = key;
            self.xs = spans(frame.width, width, self.average);
            self.ys = spans(frame.height, height, self.average);
            self.out = vec![[0; 3]; width * height];
        }
        if frame.width == 0 || frame.height == 0 {
            return &self.out;
        }
        for (ty, &(y0, y1)) in self.ys.iter().enumerate() {
            for (tx, &(x0, x1)) in self.xs.iter().enumerate() {
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for y in y0..y1 {
                    let row = &frame.pixels[y * frame.width..(y + 1) * frame.width];
                    for &p in &row[x0..x1] {
                        r += (p >> 16) & 0xff;
                        g += (p >> 8) & 0xff;
                        b += p & 0xff;
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as u32;
                self.out[ty * width + tx] = [(r / n) as u8, (g / n) as u8, (b / n) as u8];
            }
        }
        &self.out
    }
}

/// How cells show the picture: intuiTUIve's pixel modes.
pub use intuituive::pixels::PixelMode as CellMode;

/// Draw `frame` into `buf` (a [`pixels_fit`](intuituive::pixels::pixels_fit)
/// node's buffer, `cell` pixels a cell), fitted by `fit` in whole cells and
/// centred; the pixels around it are transparent, so the terminal's
/// background shows there. Where the frame lands, in cells.
pub fn draw_into(
    buf: &mut PixelBuf,
    frame: &Frame,
    fit: &Fit,
    cell: (u16, u16),
    resampler: &mut Resampler,
) -> CellRect {
    let (pw, ph) = (cell.0.max(1), cell.1.max(1));
    let place = fit.place(buf.width() / pw, buf.height() / ph);
    let (x0, y0) = (
        place.x as usize * pw as usize,
        place.y as usize * ph as usize,
    );
    let (w, h) = (
        place.cols as usize * pw as usize,
        place.rows as usize * ph as usize,
    );
    buf.clear(CLEAR);
    if w == 0 || h == 0 {
        return place;
    }
    let stride = buf.width() as usize;
    let pixels = resampler.resample(frame, w, h);
    let out = buf.pixels_mut();
    for (y, row) in pixels.chunks_exact(w).enumerate() {
        let at = (y0 + y) * stride + x0;
        for (slot, &[r, g, b]) in out[at..at + w].iter_mut().zip(row) {
            *slot = [r, g, b, 255];
        }
    }
    place
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_four_by_three_keeping_aspect() {
        let fit = Fit::default();
        // 80x24 with 1:2 cells: 24 rows are 48 squares tall, so 64 wide.
        assert_eq!(
            fit.place(80, 24),
            CellRect {
                x: 8,
                y: 0,
                cols: 64,
                rows: 24
            }
        );
        // A narrow terminal: full width (30 squares tall), fewer rows,
        // centred.
        assert_eq!(
            fit.place(40, 50),
            CellRect {
                x: 0,
                y: 17,
                cols: 40,
                rows: 15
            }
        );
        // 160x50: 50 rows want 133 columns.
        assert_eq!(fit.place(160, 50).cols, 133);
        let stretch = Fit {
            aspect: None,
            ..Fit::default()
        };
        assert_eq!(
            stretch.place(80, 24),
            CellRect {
                x: 0,
                y: 0,
                cols: 80,
                rows: 24
            }
        );
        assert_eq!(fit.place(0, 10), CellRect::default());
        // Square cells (a font's real size): twice as many rows' worth.
        let square = Fit {
            cell_aspect: 1.0,
            ..Fit::default()
        };
        assert_eq!(square.place(80, 24).cols, 32);
    }

    #[test]
    fn resampling_samples_averages_and_enlarges() {
        let mut frame = Frame::filled(4, 2, 0);
        frame.pixels[0] = 0x00ff_0000;
        frame.pixels[1] = 0x0000_0000;
        let mut r = Resampler::averaging();
        let out = r.resample(&frame, 2, 1).to_vec();
        // The first output pixel averages a 2x2 block with one red pixel.
        assert_eq!(out, vec![[63, 0, 0], [0, 0, 0]]);
        // Nearest takes each block's centre pixel, keeping the colours.
        frame.pixels[5] = 0x0000_ff00;
        let near = Resampler::new().resample(&frame, 2, 1).to_vec();
        assert_eq!(near, vec![[0, 255, 0], [0, 0, 0]]);
        let big = r.resample(&frame, 8, 4).to_vec();
        assert_eq!(big[0], [255, 0, 0]);
        assert_eq!(big[1], [255, 0, 0]);
        assert_eq!(big[2], [0, 0, 0]);
    }

    #[test]
    fn frames_land_centred_in_the_buffer() {
        // 16x4 cells of half blocks: a 16x8 buffer; 4:3 of 1:2 cells in 4
        // rows is 11 columns (rounded), centred from column 2.
        let mut buf = PixelBuf::new(16, 8);
        let mut frame = Frame::filled(4, 2, 0x0010_2030);
        frame.pixels[0] = 0x00ff_0000;
        let place = draw_into(
            &mut buf,
            &frame,
            &Fit::default(),
            CellMode::HalfBlocks.cell(),
            &mut Resampler::new(),
        );
        assert_eq!(
            place,
            CellRect {
                x: 2,
                y: 0,
                cols: 11,
                rows: 4
            }
        );
        assert_eq!(buf.get(1, 0), Some(CLEAR), "the margin is transparent");
        assert_eq!(buf.get(2, 0), Some([255, 0, 0, 255]));
        assert_eq!(buf.get(12, 7), Some([0x10, 0x20, 0x30, 255]));
        assert_eq!(buf.get(13, 7), Some(CLEAR));
        // Braille's cells are 2x4: the same cells hold 32x16 pixels.
        let mut fine = PixelBuf::new(32, 16);
        let place = draw_into(
            &mut fine,
            &frame,
            &Fit::default(),
            CellMode::Braille.cell(),
            &mut Resampler::new(),
        );
        assert_eq!((place.x, place.cols), (2, 11));
        assert_eq!(fine.get(4, 0), Some([255, 0, 0, 255]));
        assert_eq!(fine.get(3, 0), Some(CLEAR));
    }

    #[test]
    fn a_whole_frame_fills_the_fit() {
        let mut frame = Frame::filled(320, 200, 0x0020_4060);
        frame.pixels[0] = 0x00ff_ffff;
        let mut buf = PixelBuf::new(80, 48);
        let place = draw_into(
            &mut buf,
            &frame,
            &Fit::default(),
            (1, 2),
            &mut Resampler::new(),
        );
        assert_eq!((place.cols, place.rows), (64, 24));
        let opaque = buf.pixels().iter().filter(|p| p[3] == 255).count();
        assert_eq!(opaque, 64 * 48);
    }
}
