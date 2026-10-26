#!/bin/sh
# Records docs/media/doom-play.mp4: rs-rich-doom playing Freedoom's first
# demo in half blocks in xterm on a virtual X display (Xvfb), captured by
# ffmpeg, with the game's own sound effects and OPL music.
#
# The sound goes through the game's normal path (intuiTUIve's audio
# output, cpal, ALSA) to a device defined in a temporary ~/.asoundrc: ALSA's
# `plug` (44.1 kHz stereo f32) over its `file` plugin over `null`. The file
# is a FIFO that paced_tap.py reads at the sample rate, keeping at most a
# minute; everything temporary is deleted on exit.
#
#     docs/media/play/record.sh BINARY FREEDOOM1_WAD [SECONDS]
#
# Needs Xvfb, xterm, ffmpeg (with libx264 and aac) and python3.
set -eu
BIN=$(realpath "$1")
WAD=$(realpath "$2")
SECONDS_LONG=${3:-30}
HERE=$(dirname "$(realpath "$0")")
OUT=$(realpath "$HERE/..")/doom-play.mp4
WORK=$(mktemp -d)
DISPLAY_NUM=:77
cleanup() {
    [ -n "${XVFB_PID:-}" ] && kill "$XVFB_PID" 2>/dev/null || true
    [ -n "${TAP_PID:-}" ] && kill "$TAP_PID" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

mkfifo "$WORK/tap.fifo"
mkdir -p "$WORK/home" "$WORK/data"
cat > "$WORK/home/.asoundrc" <<ALSA
pcm.rsdoomtapfile {
    type file
    slave.pcm "null"
    file "$WORK/tap.fifo"
    format "raw"
}
pcm.rsdoomtap {
    type plug
    slave { pcm "rsdoomtapfile"; rate 44100; channels 2; format FLOAT_LE }
    hint { show on description "rs-rich-doom recording tap" }
}
ALSA

python3 -I "$HERE/paced_tap.py" "$WORK/tap.fifo" "$WORK/audio.f32" &
TAP_PID=$!

Xvfb $DISPLAY_NUM -screen 0 1280x720x24 -nolisten tcp &
XVFB_PID=$!
sleep 1

# The game, in a 160x45 xterm filling the screen (8x16-pixel cells).
FONT_SIZE=${FONT_SIZE:-10}
DISPLAY=$DISPLAY_NUM HOME="$WORK/home" RS_RICH_DOOM_DATA="$WORK/data" COLORTERM=truecolor \
    LC_ALL=C.UTF-8 LANG=C.UTF-8 \
    xterm -u8 -geometry 160x45+0+0 -fa "DejaVu Sans Mono" -fs "$FONT_SIZE" -bg black -fg white \
    -xrm "XTerm*directColor: true" -xrm "XTerm*internalBorder: 0" \
    -e "$BIN" --iwad "$WAD" --mode half --audio-device rsdoomtap \
    --quit-after $((SECONDS_LONG + 2)) -- -playdemo demo1 &
TERM_PID=$!

date +%s.%N > "$WORK/video.start"
ffmpeg -loglevel error -y -f x11grab -draw_mouse 0 -framerate 35 -video_size 1280x720 \
    -i "$DISPLAY_NUM" -t "$SECONDS_LONG" -c:v libx264 -preset veryfast -crf 30 \
    -pix_fmt yuv420p "$WORK/video.mp4"
wait "$TERM_PID" || true
wait "$TAP_PID" || true

# The sound started when the game opened the device: put it there.
OFFSET=$(python3 -I -c "import sys; print(max(0.0, float(open(sys.argv[1]).read()) - float(open(sys.argv[2]).read())))" \
    "$WORK/audio.f32.start" "$WORK/video.start")
ffmpeg -loglevel error -y -i "$WORK/video.mp4" \
    -itsoffset "$OFFSET" -f f32le -ar 44100 -ac 2 -i "$WORK/audio.f32" \
    -map 0:v -map 1:a -c:v copy -c:a aac -b:a 128k -shortest -movflags +faststart "$OUT"
echo "$OUT (sound from ${OFFSET}s)"
