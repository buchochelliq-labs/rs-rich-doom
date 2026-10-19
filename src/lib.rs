//! rs-rich-doom: Doom (the GPL engine, through doomgeneric) in the terminal,
//! drawn and driven by the rs-rich crates.
//!
//! - [`engine`]: the engine and its platform layer (frames out, keys in).
//! - [`keys`]: terminal keys to Doom keys, with key-ups real or synthesized.
//! - [`render`]: fitting, resampling, half blocks and Braille.
//! - [`graphics`]: Kitty, Sixel and iTerm2 images.
//! - [`app`]: the intuiTUIve app.
//! - [`sound`]: sound effects and OPL music through cpal.
//! - [`data`]: the data directory and Freedoom's download.
//! - [`cli`]: the command line.
//!
//! Copyright (C) 2026 rs-rich-doom contributors. GPL-2.0-or-later.

pub mod app;
pub mod cli;
pub mod data;
pub mod engine;
pub mod graphics;
pub mod keys;
pub mod render;
pub mod sound;
