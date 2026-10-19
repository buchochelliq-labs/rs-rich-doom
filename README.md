# rs-rich-doom

Doom (id Software's GPL engine, through [doomgeneric](https://github.com/ozkl/doomgeneric))
in the terminal, drawn and driven by the [rs-rich](https://github.com/buchochelliq-labs/rs-rich-cli)
crates (rs-rich-art, rs-rich-interact, intuiTUIve).

## STATUS (WIP)

Works: engine builds via `build.rs`/`cc`; headless tics test; half-block, Braille,
Kitty, Sixel and iTerm2 modes; key mapping with kitty releases or synthesized
releases; sound effects through cpal; `fetch-freedoom`; `--fps`, `--stats`.
Left: full README (keys, terminal table, screenshot), CI workflow, final fps table.

## Licence

GPL-2.0-or-later (see `LICENSE`), because Doom's engine is GPL. rs-rich (MIT) is
used as an ordinary library. No game data is included: use `--iwad PATH` or
`rs-rich-doom fetch-freedoom` (Freedoom, BSD).
