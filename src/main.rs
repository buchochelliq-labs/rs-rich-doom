//! `rs-rich-doom`: Doom in the terminal.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;

use rs_rich_doom::app::{self, Draw};
use rs_rich_doom::cli::{Cli, Command, Mode};
use rs_rich_doom::data;
use rs_rich_doom::graphics::{Pictures, Protocol};
use rs_rich_doom::render::{CellMode, Fit};
use rs_rich_doom::sound;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match &cli.command {
        Some(Command::FetchFreedoom { force }) => {
            let Some(dir) = data::data_dir() else {
                eprintln!("no data directory: set RS_RICH_DOOM_DATA");
                return ExitCode::FAILURE;
            };
            return match data::fetch_freedoom(&dir, *force) {
                Ok(files) => {
                    for file in files {
                        println!("{}", file.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("rs-rich-doom: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        Some(Command::Where) => {
            let dir = data::data_dir();
            match &dir {
                Some(d) => println!("data: {}", d.display()),
                None => println!("data: (none; set RS_RICH_DOOM_DATA)"),
            }
            match data::find_iwad(cli.iwad.as_deref(), dir.as_deref()) {
                Ok(iwad) => println!("iwad: {}", iwad.display()),
                Err(e) => println!("iwad: {e}"),
            }
            return ExitCode::SUCCESS;
        }
        None => {}
    }
    match play(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rs-rich-doom: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What `--mode` comes to on this terminal, and the cell size in pixels.
fn resolve(mode: Mode) -> (Draw, (u32, u32), f64) {
    let env = rich_ext::graphics::GraphicsEnvironment::system();
    let cell = env
        .cell_pixels
        .value
        .map(|c| (u32::from(c.width), u32::from(c.height)));
    let cell_aspect = cell.map_or(2.0, |(w, h)| h as f64 / w as f64);
    let cell = cell.unwrap_or((8, 16));
    use rich_ext::capabilities::Graphics;
    let draw = match mode {
        Mode::Half => Draw::Cells(CellMode::HalfBlocks),
        Mode::Braille => Draw::Cells(CellMode::Braille),
        Mode::Kitty => Draw::Image(Protocol::Kitty),
        Mode::Sixel => Draw::Image(Protocol::Sixel),
        Mode::Iterm => Draw::Image(Protocol::Iterm),
        Mode::Auto => match env.graphics.value {
            Graphics::Kitty => Draw::Image(Protocol::Kitty),
            Graphics::Sixel => Draw::Image(Protocol::Sixel),
            Graphics::Iterm => Draw::Image(Protocol::Iterm),
            Graphics::None => Draw::Cells(CellMode::HalfBlocks),
        },
    };
    (draw, cell, cell_aspect)
}

fn play(cli: Cli) -> Result<ExitCode, String> {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        return Err("standard output is not a terminal".into());
    }
    let dir = data::data_dir();
    let iwad = data::find_iwad(cli.iwad.as_deref(), dir.as_deref())?;
    let iwad = iwad
        .canonicalize()
        .map_err(|e| format!("{}: {e}", iwad.display()))?;
    // Files named in Doom's arguments are the caller's, relative to where
    // the game was started: made absolute before the directory changes.
    let here = std::env::current_dir().map_err(|e| format!("the current directory: {e}"))?;
    let args = data::resolve_doom_args(&cli.doom_args, &here);
    // Doom keeps default.cfg and its saves in the working directory: make
    // that the data directory, not wherever the game was started.
    if let Some(dir) = &dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::env::set_current_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let (draw, cell, cell_aspect) = resolve(cli.mode);
    let fit = Fit {
        aspect: (!cli.stretch).then_some(4.0 / 3.0),
        cell_aspect,
    };
    let pictures = match draw {
        Draw::Image(protocol) => Some(Arc::new(Pictures::new(protocol, cell, cli.sixel_width))),
        Draw::Cells(_) => None,
    };
    sound::choose_device(sound::Device {
        name: cli.audio_device.clone(),
        ..sound::Device::default()
    });
    sound::set_volume(cli.volume as f32 / 100.0);
    let options = app::Options {
        iwad,
        args,
        sound: !cli.no_sound,
        music: !cli.no_music,
        music_gain: cli.music_volume as f32 / 100.0,
        draw,
        fit,
        pictures,
        show_fps: cli.fps,
        quit_after: cli.quit_after.map(Duration::from_secs_f64),
        hold: Duration::from_millis(cli.hold_ms),
    };
    let outcome = app::run(options).map_err(|e| e.to_string())?;
    if cli.stats {
        let s = &outcome.stats;
        eprintln!(
            "mode {:?}: {} frames drawn in {:.1} s ({:.1} fps; {} tics, newest engine frame {}), \
             {} held back, last frame {} bytes",
            draw,
            s.frames,
            s.elapsed.as_secs_f64(),
            s.fps(),
            s.tics,
            s.engine_frames,
            s.skipped,
            s.bytes
        );
    }
    if let Some(note) = sound::note() {
        eprintln!("rs-rich-doom: {note}");
    }
    match outcome.exit {
        Some(exit) if exit.code != 0 => {
            eprintln!("{}", exit.log);
            Ok(ExitCode::from(exit.code.clamp(1, 255) as u8))
        }
        _ => Ok(ExitCode::SUCCESS),
    }
}
