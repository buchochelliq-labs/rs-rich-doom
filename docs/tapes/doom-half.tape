# rs-rich-doom playing Freedoom's first demo (`-playdemo demo1`), drawn in
# half blocks, recorded with rs-rich-record (`rich record`) at 35 frames a
# second. The demo is deterministic: the same footage on every run.
#
# Run from the directory that holds freedoom1.wad (`$REPO` in the tape),
# with the binary on PATH or under --bin-dir:
#
#     rich record --bin-dir target/release --output docs/media \
#         docs/tapes/doom-half.tape
Set Title "rs-rich-doom · Freedoom, half blocks"
Set Size 100x32
Set Framerate 35
Set FontSize 8
Set KeyOverlay off
Output gif apng
Mask /\d+(\.\d+)? fps/ "NN fps"
Hide
Type "clear; rs-rich-doom --iwad \"$REPO/freedoom1.wad\" --mode half --no-sound --quit-after 19 -- -playdemo demo1"
Enter
Sleep 1500ms
Show
Sleep 16s
Screenshot demo
Wait /❯\s*$/ 30s
