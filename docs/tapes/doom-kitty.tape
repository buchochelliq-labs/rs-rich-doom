# rs-rich-doom playing Freedoom's first demo (`-playdemo demo1`), drawn
# with the Kitty graphics protocol (Unicode placeholder cells), recorded
# with rs-rich-record (`rich record`), which decodes the images, at 12
# frames a second for 12 seconds in small text: every frame is a new
# picture, so this keeps the GIF and APNG under 10 MB. The demo is
# deterministic: the same footage on every run.
#
# Run from the directory that holds freedoom1.wad (`$REPO` in the tape),
# with the binary on PATH or under --bin-dir:
#
#     rich record --bin-dir target/release --output docs/media \
#         docs/tapes/doom-kitty.tape
Set Title "rs-rich-doom · Freedoom, Kitty graphics"
Set Size 100x32
Set Framerate 12
Set FontSize 6
Set KeyOverlay off
Output gif apng
Mask /\d+(\.\d+)? fps/ "NN fps"
Hide
Type "clear; rs-rich-doom --iwad \"$REPO/freedoom1.wad\" --mode kitty --no-sound --quit-after 15 -- -playdemo demo1"
Enter
Sleep 1500ms
Show
Sleep 12s
Screenshot demo
Wait /❯\s*$/ 30s
