//! Real pixels: the frame drawn with a terminal graphics protocol (Kitty,
//! Sixel or iTerm2) beside intuiTUIve's cells, through its
//! [`PlacementSource`] side channel.
//!
//! The widget tells [`Pictures`] which frame shows where; after each frame's
//! cells are written, the app asks it for the placements and draws those
//! that changed. Kitty's image lives in its cells (Unicode placeholders the
//! widget prints), so only the image data is sent again; Sixel and iTerm2
//! images are overlays drawn over blank cells.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use rich_art::image::RgbaImage;
use rich_ext::frame::{Graphic, Placement, PlacementSource};

use crate::render::{CellRect, Frame, Resampler};

/// A graphics protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Kitty,
    Sixel,
    Iterm,
}

/// The Kitty image id this program uses.
pub const KITTY_ID: u32 = 0x00d0_0d01;

/// The pixel size a frame is encoded at for `rect`, `cell` pixels a cell:
/// the cells' shape, at most `max_width` wide.
pub fn pixel_size(rect: CellRect, cell: (u32, u32), max_width: u32) -> (u32, u32) {
    let w = (rect.cols as u32 * cell.0).max(1);
    let h = (rect.rows as u32 * cell.1).max(1);
    if w <= max_width {
        (w, h)
    } else {
        (max_width, ((h as u64 * max_width as u64) / w as u64).max(1) as u32)
    }
}

/// `frame` resampled to `width` x `height` RGBA.
pub fn rgba(frame: &Frame, width: u32, height: u32, resampler: &mut Resampler) -> RgbaImage {
    let px = resampler.resample(frame, width as usize, height as usize);
    let mut out = Vec::with_capacity(px.len() * 4);
    for p in px {
        out.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    RgbaImage::from_raw(width, height, out).unwrap_or_default()
}

/// A Sixel image of `pixels` (`0x00RRGGBB`, `width` x `height`) with a
/// palette of exactly its colours: a Doom frame has at most 256, so nothing
/// is quantized. `None` past 256 colours.
pub fn sixel_indexed(pixels: &[u32], width: usize, height: usize) -> Option<String> {
    if pixels.len() != width * height || width == 0 || height == 0 {
        return None;
    }
    let mut palette: Vec<u32> = Vec::new();
    let mut index_of = std::collections::HashMap::new();
    let mut indices = Vec::with_capacity(pixels.len());
    for &p in pixels {
        let p = p & 0x00ff_ffff;
        let i = match index_of.get(&p) {
            Some(&i) => i,
            None => {
                if palette.len() == 256 {
                    return None;
                }
                let i = palette.len() as u8;
                palette.push(p);
                index_of.insert(p, i);
                i
            }
        };
        indices.push(i);
    }
    let mut out = String::with_capacity(pixels.len() / 2);
    // P2=1: pixels left unset stay as they were; raster attributes give
    // the size and square pixels.
    let _ = write!(out, "\x1bP0;1;0q\"1;1;{width};{height}");
    let pct = |v: u32| (v * 100 + 127) / 255;
    for (i, &p) in palette.iter().enumerate() {
        let _ = write!(
            out,
            "#{i};2;{};{};{}",
            pct((p >> 16) & 0xff),
            pct((p >> 8) & 0xff),
            pct(p & 0xff)
        );
    }
    let bands = height.div_ceil(6);
    let mut bits = vec![0u8; width * palette.len()];
    let mut used = vec![false; palette.len()];
    for band in 0..bands {
        bits.iter_mut().for_each(|b| *b = 0);
        used.iter_mut().for_each(|u| *u = false);
        for dy in 0..6 {
            let y = band * 6 + dy;
            if y >= height {
                break;
            }
            for x in 0..width {
                let c = indices[y * width + x] as usize;
                bits[c * width + x] |= 1 << dy;
                used[c] = true;
            }
        }
        let mut first = true;
        for c in 0..palette.len() {
            if !used[c] {
                continue;
            }
            if !first {
                out.push('$');
            }
            first = false;
            let _ = write!(out, "#{c}");
            let row = &bits[c * width..(c + 1) * width];
            // The columns after the colour's last one need nothing.
            let end = row.iter().rposition(|&b| b != 0).map_or(0, |e| e + 1);
            let mut x = 0;
            while x < end {
                let b = row[x];
                let mut run = 1;
                while x + run < end && row[x + run] == b {
                    run += 1;
                }
                let glyph = char::from(63 + b);
                if run > 3 {
                    let _ = write!(out, "!{run}{glyph}");
                } else {
                    (0..run).for_each(|_| out.push(glyph));
                }
                x += run;
            }
        }
        if band + 1 < bands {
            out.push('-');
        }
    }
    out.push_str("\x1b\\");
    Some(out)
}

/// What the widget shows now.
#[derive(Clone)]
struct Shown {
    frame: Arc<Frame>,
    rect: CellRect,
}

/// The graphics source for the app: one picture, the newest frame.
pub struct Pictures {
    protocol: Protocol,
    /// A cell's size in pixels.
    cell: (u32, u32),
    /// The widest image Sixel and iTerm2 are sent at.
    max_width: u32,
    shown: Mutex<Option<Shown>>,
}

impl Pictures {
    pub fn new(protocol: Protocol, cell: (u32, u32), max_width: u32) -> Pictures {
        Pictures {
            protocol,
            cell: (cell.0.max(1), cell.1.max(1)),
            max_width: max_width.max(64),
            shown: Mutex::new(None),
        }
    }

    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// Show `frame` on `rect` (screen cells) from the next placement on.
    pub fn show(&self, frame: Arc<Frame>, rect: CellRect) {
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = Some(Shown { frame, rect });
    }

    /// The cells an overlay of `rect` really covers: Sixel images are sent
    /// at most `max_width` pixels wide, so they may take fewer cells.
    pub fn covered(&self, rect: CellRect) -> CellRect {
        match self.protocol {
            Protocol::Kitty | Protocol::Iterm => rect,
            Protocol::Sixel => {
                let (w, h) = pixel_size(rect, self.cell, self.max_width);
                CellRect {
                    cols: w.div_ceil(self.cell.0).min(rect.cols as u32) as u16,
                    rows: h.div_ceil(self.cell.1).min(rect.rows as u32) as u16,
                    ..rect
                }
            }
        }
    }
}

impl PlacementSource for Pictures {
    fn placements(&self, _frame: &rich_ext::frame::Frame) -> Vec<Placement> {
        let shown = self.shown.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(Shown { frame, rect }) = shown else {
            return Vec::new();
        };
        if rect.cols == 0 || rect.rows == 0 {
            return Vec::new();
        }
        let covered = self.covered(rect);
        vec![Placement {
            row: covered.y as usize,
            column: covered.x as usize,
            cols: covered.cols as usize,
            rows: covered.rows as usize,
            graphic: Arc::new(Picture {
                frame,
                protocol: self.protocol,
                rect: covered,
                cell: self.cell,
                max_width: self.max_width,
            }),
            frame: 0,
        }]
    }

    fn release(&self, _retained: &[Placement]) -> String {
        match self.protocol {
            Protocol::Kitty => rich_art::kitty::delete(KITTY_ID),
            _ => String::new(),
        }
    }
}

/// One frame, as a protocol draws it.
#[derive(Debug)]
struct Picture {
    frame: Arc<Frame>,
    protocol: Protocol,
    rect: CellRect,
    cell: (u32, u32),
    max_width: u32,
}

impl Graphic for Picture {
    fn key(&self) -> u64 {
        self.frame.seq ^ (u64::from(self.rect.cols) << 40) ^ (u64::from(self.rect.rows) << 52)
    }

    fn draw(&self, _frame: usize) -> String {
        let mut resampler = Resampler::averaging();
        let (cols, rows) = (self.rect.cols as usize, self.rect.rows as usize);
        match self.protocol {
            Protocol::Kitty | Protocol::Iterm => {
                // The frame at the cells' shape; the terminal scales it.
                let (w, h) = pixel_size(self.rect, self.cell, u32::MAX);
                let height = ((self.frame.width as u64 * h as u64) / w.max(1) as u64).max(1) as u32;
                let image = rgba(&self.frame, self.frame.width as u32, height, &mut resampler);
                let Some(png) = rich_art::graphics::encode_png(&image) else {
                    return String::new();
                };
                if self.protocol == Protocol::Kitty {
                    rich_art::kitty::transmit(KITTY_ID, &png, cols, rows)
                } else {
                    let image = rich_art::iterm::inline_image(&png, cols, rows);
                    format!("\x1b7{image}\x1b8")
                }
            }
            Protocol::Sixel => {
                let (w, h) = pixel_size(self.rect, self.cell, self.max_width);
                // Nearest neighbour keeps the frame's own 256 colours.
                let px = Resampler::new().resample(&self.frame, w as usize, h as usize).to_vec();
                let packed: Vec<u32> = px
                    .iter()
                    .map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
                    .collect();
                // Averaging may make more than 256 colours: fall back to
                // rs-rich-art's quantizing encoder then.
                let sixel = sixel_indexed(&packed, w as usize, h as usize).or_else(|| {
                    let image = rgba(&self.frame, w, h, &mut Resampler::averaging());
                    rich_art::SixelArt::new(rich_art::image::DynamicImage::ImageRgba8(image))
                        .cell_px(self.cell.0, self.cell.1)
                        .encode_cells(cols, rows)
                });
                match sixel {
                    Some(s) => format!("\x1b7{s}\x1b8"),
                    None => String::new(),
                }
            }
        }
    }

    fn in_cells(&self) -> bool {
        self.protocol == Protocol::Kitty
    }
}

/// The placeholder cells Kitty shows its image through, one line a row,
/// in the colour that names the image.
pub fn kitty_placeholders(cols: usize, rows: usize) -> Vec<Vec<rich::Segment>> {
    let color = rich_art::kitty::id_color(KITTY_ID, true);
    let style = rich::Style::from_color(color, None);
    (0..rows)
        .map(|row| {
            let text = rich_art::kitty::placeholder_row(row, cols).unwrap_or_default();
            vec![rich::Segment::new(text, Some(style.clone()))]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixel_keeps_the_frames_own_colours() {
        // 3x7: two bands, two colours.
        let red = 0x00ff_0000;
        let blue = 0x0000_00ff;
        let mut px = vec![red; 21];
        px[0] = blue;
        let s = sixel_indexed(&px, 3, 7).unwrap();
        assert!(s.starts_with("\x1bP0;1;0q\"1;1;3;7#0;2;0;0;100#1;2;100;0;0"));
        assert!(s.ends_with("\x1b\\"));
        // Band one: blue in the first column's top pixel; red the rest.
        assert!(s.contains("#0@$#1}~~-"), "{s:?}");
        // 300 colours do not fit a palette.
        let many: Vec<u32> = (0..300).collect();
        assert!(sixel_indexed(&many, 300, 1).is_none());
    }

    #[test]
    fn pixel_sizes_follow_the_cells() {
        let rect = CellRect {
            x: 0,
            y: 0,
            cols: 64,
            rows: 24,
        };
        assert_eq!(pixel_size(rect, (10, 20), 2000), (640, 480));
        assert_eq!(pixel_size(rect, (10, 20), 320), (320, 240));
        let p = Pictures::new(Protocol::Sixel, (10, 20), 320);
        assert_eq!(p.covered(rect).cols, 32);
        assert_eq!(p.covered(rect).rows, 12);
    }

    #[test]
    fn placements_follow_what_is_shown() {
        let p = Pictures::new(Protocol::Kitty, (8, 16), 640);
        let empty = rich_ext::frame::Frame::default();
        assert!(p.placements(&empty).is_empty());
        let rect = CellRect {
            x: 8,
            y: 0,
            cols: 64,
            rows: 24,
        };
        let mut frame = Frame::filled(320, 200, 0x0010_2030);
        frame.seq = 7;
        p.show(Arc::new(frame), rect);
        let placed = p.placements(&empty);
        assert_eq!((placed[0].row, placed[0].column, placed[0].cols), (0, 8, 64));
        assert!(placed[0].graphic.in_cells());
        let escape = placed[0].graphic.draw(0);
        assert!(escape.starts_with("\x1b_Ga=T,U=1,f=100,t=d,i="), "{escape:.40}");
        assert!(p.release(&[]).contains("a=d"));
    }

    #[test]
    fn placeholders_are_one_cell_each() {
        let lines = kitty_placeholders(5, 2);
        assert_eq!(lines.len(), 2);
        assert_eq!(rich::cells::cell_len(&lines[1][0].text), 5);
    }
}
