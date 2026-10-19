//! A Doom frame to terminal cells: fitting it to the terminal keeping its
//! aspect ratio, resampling it, and drawing it as half blocks or Braille.
//!
//! The engine's framebuffer is 320x200 XRGB (`0x00RRGGBB`), shown by the
//! original game at 4:3 (its pixels are 1.2 times taller than wide).

use rich::color::Color;
use rich::{Segment, Style};

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
/// the frame's own 256: a terminal screen interns every style it is given,
/// and averaged colours make a new one for nearly every cell of every
/// frame. [`averaging`](Self::averaging) averages the source pixels each
/// target pixel covers instead, for the image protocols. The tables are
/// kept, so a frame of the same size costs one pass.
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

/// How cells show the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellMode {
    /// `▀` with the upper pixel as foreground and the lower as background:
    /// one by two pixels a cell.
    HalfBlocks,
    /// Braille dots, two by four a cell, in two colours per cell.
    Braille,
}

impl CellMode {
    /// Pixels a cell holds, across and down.
    pub fn density(self) -> (usize, usize) {
        match self {
            CellMode::HalfBlocks => (1, 2),
            CellMode::Braille => (2, 4),
        }
    }
}

fn color([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgb(r, g, b)
}

/// Appends a cell to a line, merging it into the last segment when both
/// have the same colours (fewer segments, fewer style changes on the wire).
fn push_cell(
    line: &mut Vec<Segment>,
    last: &mut Option<(u32, u32)>,
    text: char,
    fg: [u8; 3],
    bg: [u8; 3],
) {
    let pack = |[r, g, b]: [u8; 3]| (r as u32) << 16 | (g as u32) << 8 | b as u32;
    let key = (pack(fg), pack(bg));
    if *last == Some(key) {
        if let Some(segment) = line.last_mut() {
            segment.text.push(text);
            return;
        }
    }
    *last = Some(key);
    line.push(Segment::new(
        text.to_string(),
        Some(Style::from_color(Some(color(fg)), Some(color(bg)))),
    ));
}

/// The lines of cells for `pixels` (`cols * density.0` by
/// `rows * density.1`, row by row) in `mode`.
pub fn cells(pixels: &[[u8; 3]], cols: usize, rows: usize, mode: CellMode) -> Vec<Vec<Segment>> {
    let (dx, dy) = mode.density();
    let width = cols * dx;
    assert_eq!(pixels.len(), width * rows * dy, "pixels for the cells");
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut line = Vec::new();
        let mut last = None;
        for col in 0..cols {
            match mode {
                CellMode::HalfBlocks => {
                    let top = pixels[(row * 2) * width + col];
                    let bottom = pixels[(row * 2 + 1) * width + col];
                    if top == bottom {
                        // A space shows the background alone: no glyph to
                        // draw, and runs of one colour merge.
                        push_cell(&mut line, &mut last, ' ', top, top);
                    } else {
                        push_cell(&mut line, &mut last, '▀', top, bottom);
                    }
                }
                CellMode::Braille => {
                    let (glyph, fg, bg) = braille_cell(pixels, width, col * 2, row * 4);
                    push_cell(&mut line, &mut last, glyph, fg, bg);
                }
            }
        }
        lines.push(line);
    }
    lines
}

/// Braille's dot bits, by (x, y) in the 2x4 cell.
const DOTS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

fn luma([r, g, b]: [u8; 3]) -> u32 {
    (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000
}

/// One Braille cell: the dots brighter than the cell's mean are raised.
/// Each side is drawn in one of its own pixels' colours (the one nearest
/// the side's mean), never a blend, so cells use the frame's colours only.
fn braille_cell(pixels: &[[u8; 3]], width: usize, x: usize, y: usize) -> (char, [u8; 3], [u8; 3]) {
    let mut samples = [[0u8; 3]; 8];
    let mut total = 0;
    for dy in 0..4 {
        for dx in 0..2 {
            let p = pixels[(y + dy) * width + x + dx];
            samples[dy * 2 + dx] = p;
            total += luma(p);
        }
    }
    let mean = total / 8;
    let mut bits = 0u32;
    let (mut on, mut off) = (Vec::with_capacity(8), Vec::with_capacity(8));
    for (i, &p) in samples.iter().enumerate() {
        if luma(p) > mean {
            bits |= DOTS[i / 2][i % 2];
            on.push(p);
        } else {
            off.push(p);
        }
    }
    let bg = representative(&off);
    if bits == 0 {
        return (' ', bg, bg);
    }
    let glyph = char::from_u32(0x2800 + bits).unwrap_or(' ');
    (glyph, representative(&on), bg)
}

/// The member of `group` nearest its mean colour.
fn representative(group: &[[u8; 3]]) -> [u8; 3] {
    let Some(&first) = group.first() else {
        return [0, 0, 0];
    };
    let n = group.len() as i32;
    let mut sum = [0i32; 3];
    for p in group {
        for c in 0..3 {
            sum[c] += p[c] as i32;
        }
    }
    let mean = [sum[0] / n, sum[1] / n, sum[2] / n];
    let distance = |p: &[u8; 3]| -> i32 { (0..3).map(|c| (p[c] as i32 - mean[c]).pow(2)).sum() };
    group.iter().copied().min_by_key(distance).unwrap_or(first)
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
    fn half_blocks_put_two_pixels_in_a_cell() {
        // A synthetic 2x4 framebuffer: red over blue in the first column,
        // all green in the second.
        let red = [255, 0, 0];
        let blue = [0, 0, 255];
        let green = [0, 255, 0];
        let pixels = vec![red, green, blue, green, red, green, red, green];
        let lines = cells(&pixels, 2, 2, CellMode::HalfBlocks);
        assert_eq!(lines.len(), 2);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert_eq!(text, vec!["▀ ", "  "]);
        let style = lines[0][0].style.as_ref().unwrap();
        assert_eq!(style.color(), Some(&Color::from_rgb(255, 0, 0)));
        assert_eq!(style.bgcolor(), Some(&Color::from_rgb(0, 0, 255)));
        // The second row: two cells of one colour each, two segments.
        assert_eq!(lines[1].len(), 2);
    }

    #[test]
    fn equal_cells_merge_into_one_segment() {
        let grey = [9, 9, 9];
        let pixels = vec![grey; 6 * 2];
        let lines = cells(&pixels, 6, 1, CellMode::HalfBlocks);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].text, "      ");
    }

    #[test]
    fn braille_raises_the_bright_dots() {
        let (w, b) = ([255, 255, 255], [0, 0, 0]);
        // Left column bright, right column dark: dots 1, 2, 3 and 7.
        let pixels = vec![w, b, w, b, w, b, w, b];
        let lines = cells(&pixels, 1, 1, CellMode::Braille);
        assert_eq!(lines[0][0].text, "\u{2847}");
        let style = lines[0][0].style.as_ref().unwrap();
        assert_eq!(style.color(), Some(&Color::from_rgb(255, 255, 255)));
        assert_eq!(style.bgcolor(), Some(&Color::from_rgb(0, 0, 0)));
        // A flat cell is a blank in its colour.
        let flat = cells(&[b; 8], 1, 1, CellMode::Braille);
        assert_eq!(flat[0][0].text, " ");
    }

    #[test]
    fn a_whole_frame_converts() {
        let mut frame = Frame::filled(320, 200, 0x0020_4060);
        frame.pixels[0] = 0x00ff_ffff;
        let fit = Fit::default().place(80, 24);
        let mut r = Resampler::new();
        let px = r.resample(&frame, fit.cols as usize, fit.rows as usize * 2);
        let lines = cells(
            px,
            fit.cols as usize,
            fit.rows as usize,
            CellMode::HalfBlocks,
        );
        assert_eq!(lines.len(), 24);
        let width: usize = lines[5].iter().map(|s| s.text.chars().count()).sum();
        assert_eq!(width, 64);
    }
}
