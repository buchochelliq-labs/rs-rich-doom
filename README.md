# rs-rich-doom

Real Doom in the terminal. The engine is id Software's GPL release of Doom,
through [doomgeneric](https://github.com/ozkl/doomgeneric) (Chocolate Doom's
code with a six-function platform layer); the picture, the keyboard and the
app loop are the [rs-rich](https://github.com/buchochelliq-labs/rs-rich-cli)
crates: rs-rich-art (Kitty, Sixel and iTerm2 encoders), rs-rich-ext
(capability detection, the graphics side channel), rs-rich-interact (the
terminal session, raw input, the kitty keyboard protocol, synchronized
output) and intuiTUIve (the app, the `latest` signal, a custom `Widget`).
Doom draws its own HUD; the app adds nothing over it but an optional fps
counter.

## Licence

**GPL-2.0-or-later** (see [`LICENSE`](LICENSE)). Doom's engine is GPL-2.0,
and this program links it, so the program is GPL too; that is why it lives
in its own repository. rs-rich (MIT) is used as an ordinary library
dependency and stays MIT. The vendored engine is in
[`doomgeneric/`](doomgeneric) with its licence and a [NOTE](doomgeneric/NOTE)
of the upstream commit and the one local change.

**No game data is included.** Play the shareware `doom1.wad`, an IWAD you
own (`doom.wad`, `doom2.wad`, `plutonia.wad`, `tnt.wad`), or
[Freedoom](https://freedoom.github.io/) (BSD licence), which
`rs-rich-doom fetch-freedoom` downloads for you. WADs are never committed.

## Install

```sh
# Linux needs ALSA's headers for sound: sudo apt install libasound2-dev
cargo install --git https://github.com/buchochelliq-labs/rs-rich-doom
# or, without sound (no ALSA needed):
cargo install --git https://github.com/buchochelliq-labs/rs-rich-doom --no-default-features
```

A C compiler is needed (the engine is built by `build.rs` with the `cc`
crate).

## Run

```sh
rs-rich-doom fetch-freedoom          # Freedoom 0.13.0 into the data directory (SHA-256 checked)
rs-rich-doom                         # plays freedoom1.wad (else freedoom2.wad)
rs-rich-doom --iwad ~/wads/doom1.wad # your own IWAD
rs-rich-doom --mode braille --fps
rs-rich-doom -- -warp 1 3 -skill 4   # Doom's own arguments after --
rs-rich-doom where                   # the data directory and the IWAD found
```

The data directory is `$XDG_DATA_HOME/rs-rich-doom` (`~/.local/share/rs-rich-doom`),
`~/Library/Application Support/rs-rich-doom` on macOS, `%APPDATA%\rs-rich-doom`
on Windows, or `$RS_RICH_DOOM_DATA`. Doom's `default.cfg` and saves go there too.

| Option | |
|---|---|
| `--iwad PATH` | the IWAD to play |
| `--mode auto\|half\|braille\|kitty\|sixel\|iterm` | how to draw (default `auto`) |
| `--fps` | frame rate and draw time in the top-left corner |
| `--stretch` | fill the terminal instead of Doom's 4:3 |
| `--no-sound` | no sound effects |
| `--hold-ms MS` | hold time for keys without release events (default 400) |
| `--stats` | print frames drawn, fps and draw time on exit |
| `--quit-after S` | quit after S seconds (benchmarks) |
| `--sixel-width PX` | widest Sixel image sent (default 640) |

## Keys

| Key | Doom |
|---|---|
| arrows | move and turn |
| Ctrl (with an arrow), `x`, `z` | fire |
| Space | use (doors, switches) |
| Shift (with an arrow) | run |
| Alt (with an arrow) | strafe (with left/right) |
| `,` `.` | strafe left / right |
| `1`–`7` | weapons |
| Enter, Esc, `y`, `n` | menus and prompts |
| Tab | automap |
| F1–F12 | help, save, load, options, …, F10 quit |
| `-` `=` | screen size |
| letters | cheats (`iddqd`, `idkfa`, …) |
| Ctrl+C | quit at once |

### Key releases

Doom needs a key-down and a key-up for every key. Terminals send presses
(and auto-repeat as more presses); only terminals with the **kitty keyboard
protocol** also send releases, which rs-rich-interact turns on when the
terminal answers its query.

- With kitty (kitty, foot, WezTerm, Ghostty, Alacritty, recent iTerm2): the
  first time a key's release arrives, that key is trusted from then on and
  held exactly as long as it is held. Turning and running feel like the
  original.
- Without it (most others, tmux, screen): releases are synthesized. A
  movement key stays down 400 ms after its first press (to bridge the
  keyboard's auto-repeat delay; `--hold-ms`) and 120 ms after each repeat;
  menu keys, weapons and Space are taps of 80 ms. A tap of an arrow turns
  a little too far; holding works.
- Modifier keys alone are never reported to a terminal program (rs-rich
  pushes the kitty flags 1 and 2, not 8), so Ctrl, Shift and Alt act only
  together with another key: hold Ctrl while turning to fire, or use `x`.
  Enter, Tab and Backspace never report releases and are always taps.

## Modes

| Mode | What | Needs |
|---|---|---|
| `half` | `▀` cells, top pixel foreground, bottom background: 1×2 pixels a cell in truecolour | any truecolour (or 256-colour) terminal |
| `braille` | Braille dots, 2×4 a cell, two of the frame's colours per cell | Unicode font with Braille |
| `kitty` | the real 320×200 frame, Kitty graphics protocol with Unicode placeholders, scaled by the terminal | kitty, Ghostty, WezTerm |
| `sixel` | the frame at the cells' pixel size (≤ 640 wide), Sixel with the frame's own palette | foot, WezTerm, mlterm, xterm `-ti vt340`, Windows Terminal 1.22+, Konsole |
| `iterm` | iTerm2 inline images | iTerm2, WezTerm |
| `auto` | Kitty, Sixel or iTerm2 when rs-rich-ext detects it (`RICH_GRAPHICS=…` overrides), else `half` | |

The picture keeps Doom's 4:3 aspect, from the cell size the terminal reports
(else 1:2 cells), centred; `--stretch` fills the screen. Cells are sampled
nearest-neighbour so they keep Doom's 256 colours (see the notes below).

### Terminal support

| Terminal | Best mode | Key releases |
|---|---|---|
| kitty | kitty | yes |
| Ghostty | kitty | yes |
| WezTerm | kitty (sixel, iterm also) | yes |
| foot | sixel | yes |
| iTerm2 | iterm | yes (3.5+, CSI u enabled) |
| Alacritty | half | yes |
| GNOME Terminal / VTE, Konsole | half (Konsole: sixel) | no: synthesized |
| xterm | half (sixel with `-ti vt340`) | no: synthesized |
| Windows Terminal | sixel (1.22+) | no: synthesized |
| macOS Terminal.app | half (256 colours) | no: synthesized |
| tmux, screen | half | no: synthesized |

Only half blocks, Braille and the escape output of the image modes were
exercised here (in a pty); the image modes have not been looked at in a
real Kitty or Sixel terminal yet.

### Measured

Freedoom's first demo (`-- -playdemo demo1`), 10 s in a pty, `--stats`:
frames the app drew per second (the engine makes 35; identical frames are
not sent), on a Linux VM.

| Size | Mode | release | debug |
|---|---|---:|---:|
| 80×24 | half | 34.8 fps (3.3 ms a draw) | 34.6 (12.5 ms) |
| 80×24 | braille | 34.8 (3.6 ms) | 33.7 (17.6 ms) |
| 80×24 | kitty | 34.8 | 13.0 |
| 80×24 | sixel | 34.7 | 11.7 |
| 160×50 | half | 33.4 (11.2 ms) | 18.7 (41.5 ms) |
| 160×50 | braille | 32.9 (13.3 ms) | 15.9 (53.9 ms) |
| 160×50 | kitty | 33.3 | 12.5 |
| 160×50 | sixel | 33.7 | 7.0 |

Half blocks cost about 34 kB a frame at 80×24 and 140 kB at 160×50
(about 1.2 MB/s and 5 MB/s); Kitty and Sixel about 180 kB a frame at any
size. `cargo run --release --example draw_bench -- IWAD COLS ROWS MODE`
measures the app side alone with the headless driver.

## Screenshot

80×24, half blocks, Freedoom's first demo. As colour cells:
`cat docs/screenshot-80x24.ans` (or `docs/screenshot-160x50.ans`) in a
truecolour terminal. By brightness:

```text
        ::.:...::+.::=:=.::..-.=..:..:==-===-=+=-=+++-===-===+*+++-###@=
        :::==+*+===++-=-=-=+===-+=======+===-====+=-+=-====*=+==@****@-=
        :.:---=:=--==+=-========+=====-=-++=+=-===++=+++=*+##@@@*@@*+*#=
        -:--==+=+=++=-:::-:-:::--------:=#---===-==-*:##::@*..#@+@*#@*+=
        .::.--+::===**.:.-.::=--:.::--:=#@::.=#@#****@#@*@:-.-%%::::--*=
        =:--=--+=+==**..::.-.@@@.:..@@:*#-:::*-*-*#@*:#:+=::.--.+.=-==-=
        ::-::=+-+=+:==-::=::.##+.:..==:+--=::*:--:==--:*::**-.= =:=--===
        :::---==:===**::::.:.+:+::::-=:*.==-:+---=%:=--=-=++-.+.--::-:+=
        *+*-*+=-====**:::-:-.:-.:::.--:==*:::#:+:=@:=:--*=-+--=:::-+*+*=
        ===-=+=++*+*#*:::-.:.-::.:::-=  -+:::=++++=+=+++=++*==+*+#++*++=
        +++==+=*=*++++.::--=-++=-==:-:.+=-::.===-====++=-=+=++*+*#*++++=
        *===+++++++======-====-==-+++-+@@*=-==-===-+=+=+==+++++++*+++++=
        ===+==++=-----==-=-==+=++:::=@@@@@#=+=======++++*+=-=--+*+=+++==
        ++-=--======+++++===*-==--=--@@==@=---==-----:-:------:-+==-++==
        -+-=-:-::-:=---=::% ==::---:.:=::::-=---:--.:::::-:::-=---.:=:-:
        :-:---------=--::-==--::-:::::==------:----:.----:--=--..-=:::::
        ---==-=--=----:::.:-:::--=--= :-:.==::::----:::--:::::--==-::::-
        -- :--:--:::::-------::----:++:---**-:::-==--=-:::--===---:---.:
        -:--===+==::----:::-:--==---+******=-:=---::.------:::----:-::--
        ====----::---==++=--:::--:: ==+***+-::--:-:.--==+=====:::.-:--:-
        ============----=--==-@----+:-::=.-======--=-==-=-==+=--------==
        ----:-:::------:::.--------=--@%+ .#-------:--:::.+++==-%-*-@@@=
        #****+*+***+****+*++*--=------*=*=-%--****+****=+==*+==---*:-@@=
        ::-=:==::-:.+:-:::=-:-==:==:.-=*------==:==:=--.::------------%-
```

## Sound

Sound effects play through cpal (ALSA, CoreAudio, WASAPI): the engine's
sound module (`csrc/sound.c`) reads each effect's DMX lump and a small Rust
mixer (`src/sound.rs`) resamples and pans it. **Music is not played** (MUS
needs a MIDI or OPL synthesizer). `--no-sound`, or build with
`--no-default-features`.

## How it works

```
engine thread (intuiTUIve `latest`)            app thread (intuiTUIve)
  doomgeneric_Tick() at 35 Hz                    DoomView widget
   DG_DrawFrame ─► rs_doom_draw_frame ─ Latest ─►  draw(): fit 4:3, sample,
                    (skips repeats)    (newest     half blocks / Braille cells,
                                        wins)      or placeholders + a graphic
   DG_GetKey ◄── KeyState ◄─ mpsc ◄──────────── event(): Key / KeyUp → Input
   (synthesized key-ups by hold time)            graphics: Pictures (PlacementSource)
   printf/exit ─► Rust log / clean quit             Kitty, Sixel, iTerm2 escapes
```

- `build.rs` compiles `doomgeneric/` and `csrc/` with `cc` at 320×200;
  `csrc/redirect.h` is force-included so the engine's `printf` and `exit()`
  go to Rust (a log, shown if the engine fails; a clean quit for F10 → Y)
  instead of the terminal the game is drawn on.
- `src/engine.rs` implements `DG_Init`, `DG_DrawFrame`, `DG_SleepMs`,
  `DG_GetTicksMs`, `DG_GetKey`, `DG_SetWindowTitle` (`extern "C"`), with a
  virtual clock for headless tests.
- `src/keys.rs` maps keys and turns presses, repeats and releases into
  Doom's downs and ups; `src/render.rs` fits, samples and converts;
  `src/graphics.rs` encodes images; `src/app.rs` is the app.

## Tests

`cargo test` covers the key mapping and key-up synthesis, fitting and
aspect, sampling, the half-block and Braille conversion of synthetic
framebuffers, the Sixel encoder, placements, the mixer, the CLI and the
Freedoom unpacking. With `RS_RICH_DOOM_IWAD=/path/freedoom1.wad`,
`tests/engine.rs` runs 400 engine tics headless on a virtual clock and
checks the frames; CI downloads Freedoom for it.

## Credits

- id Software, for Doom and its 1997/1999 GPL source release.
- Simon Howard and the Chocolate Doom authors; ozkl's doomgeneric.
- The Freedoom project (BSD licence) for free game data.
- rs-rich and rs-rich-cli (MIT).
