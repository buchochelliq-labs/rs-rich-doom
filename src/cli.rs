//! The command line.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// How the picture is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// The best the terminal supports: Kitty, Sixel or iTerm2 images, else
    /// half blocks.
    Auto,
    /// Half blocks in truecolour: works everywhere.
    Half,
    /// Braille dots, two colours a cell.
    Braille,
    /// The Kitty graphics protocol (kitty, Ghostty, WezTerm).
    Kitty,
    /// Sixel graphics (foot, WezTerm, mlterm, xterm -ti vt340, Windows
    /// Terminal).
    Sixel,
    /// iTerm2 inline images (iTerm2, WezTerm).
    Iterm,
}

/// Doom in the terminal, drawn with rs-rich.
#[derive(Debug, Parser)]
#[command(name = "rs-rich-doom", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// The IWAD to play: doom1.wad (shareware), doom.wad, doom2.wad, or
    /// Freedoom's. Default: Freedoom from `rs-rich-doom fetch-freedoom`.
    #[arg(long, value_name = "PATH", global = true)]
    pub iwad: Option<PathBuf>,

    /// How to draw the picture.
    #[arg(long, value_enum, default_value_t = Mode::Auto)]
    pub mode: Mode,

    /// Show the frame rate in the top left corner.
    #[arg(long)]
    pub fps: bool,

    /// Fill the terminal instead of keeping Doom's 4:3 picture.
    #[arg(long)]
    pub stretch: bool,

    /// Turn sound effects off.
    #[arg(long)]
    pub no_sound: bool,

    /// Turn music off (OPL FM synthesis from the IWAD's GENMIDI
    /// instruments, as Doom played it on a Sound Blaster).
    #[arg(long)]
    pub no_music: bool,

    /// Music loudness in percent, over the game's own music volume (its
    /// sound menu).
    #[arg(long, value_name = "PERCENT", default_value_t = 100,
          value_parser = clap::value_parser!(u32).range(0..=400))]
    pub music_volume: u32,

    /// How long a key whose release the terminal does not report is held
    /// after its first press, in milliseconds (longer than the keyboard's
    /// auto-repeat delay).
    #[arg(long, value_name = "MS", default_value_t = 400)]
    pub hold_ms: u64,

    /// Quit after this many seconds (for benchmarks).
    #[arg(long, value_name = "SECONDS")]
    pub quit_after: Option<f64>,

    /// Print the frame rate and draw times when the game ends.
    #[arg(long)]
    pub stats: bool,

    /// Widest image Sixel is sent at, in pixels.
    #[arg(long, value_name = "PIXELS", default_value_t = 640)]
    pub sixel_width: u32,

    /// Doom's own arguments, after `--`: `-warp 1 1`, `-skill 4`, `-file x.wad`.
    #[arg(last = true, value_name = "DOOM ARGS")]
    pub doom_args: Vec<String>,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Download Freedoom 0.13.0 (BSD licence) into the data directory.
    FetchFreedoom {
        /// Download again even if it is there.
        #[arg(long)]
        force: bool,
    },
    /// Print where the data directory and the IWAD found are.
    Where,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("rs-rich-doom").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn defaults() {
        let cli = parse(&[]);
        assert_eq!(cli.mode, Mode::Auto);
        assert!(cli.iwad.is_none() && !cli.fps && !cli.stretch && !cli.no_sound);
        assert!(!cli.no_music);
        assert_eq!(cli.music_volume, 100);
        assert_eq!(cli.hold_ms, 400);
        assert!(cli.command.is_none());
    }

    #[test]
    fn options_and_doom_arguments() {
        let cli = parse(&[
            "--iwad",
            "/games/doom1.wad",
            "--mode",
            "braille",
            "--fps",
            "--",
            "-warp",
            "1",
            "3",
        ]);
        assert_eq!(cli.iwad, Some(PathBuf::from("/games/doom1.wad")));
        assert_eq!(cli.mode, Mode::Braille);
        assert!(cli.fps);
        assert_eq!(cli.doom_args, ["-warp", "1", "3"]);
        for (name, mode) in [
            ("half", Mode::Half),
            ("kitty", Mode::Kitty),
            ("sixel", Mode::Sixel),
            ("iterm", Mode::Iterm),
            ("auto", Mode::Auto),
        ] {
            assert_eq!(parse(&["--mode", name]).mode, mode);
        }
        assert!(Cli::try_parse_from(["rs-rich-doom", "--mode", "ascii"]).is_err());
        let cli = parse(&["--no-music", "--music-volume", "40"]);
        assert!(cli.no_music && !cli.no_sound);
        assert_eq!(cli.music_volume, 40);
        assert!(Cli::try_parse_from(["rs-rich-doom", "--music-volume", "500"]).is_err());
    }

    #[test]
    fn subcommands() {
        assert_eq!(
            parse(&["fetch-freedoom"]).command,
            Some(Command::FetchFreedoom { force: false })
        );
        assert_eq!(
            parse(&["fetch-freedoom", "--force"]).command,
            Some(Command::FetchFreedoom { force: true })
        );
        assert_eq!(parse(&["where"]).command, Some(Command::Where));
    }
}
