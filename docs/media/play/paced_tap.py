"""Read the game's audio from a FIFO at the rate a sound card would.

ALSA's `file` plugin over its `null` device writes every sample the game
plays into a FIFO; `null` alone takes samples as fast as the CPU allows,
so this reader keeps the time: 44100 stereo f32 frames a second. It keeps
at most MAX_SECONDS of it in OUT and throws the rest away, so nothing grows
without bound, and writes when the first sample came (for the video's
sync) to OUT.start.

    python3 -I paced_tap.py FIFO OUT
"""
import sys
import time

RATE, FRAME = 44100, 8  # stereo f32le
MAX_SECONDS = 60

fifo, out = sys.argv[1], sys.argv[2]
kept, cap = 0, MAX_SECONDS * RATE * FRAME
with open(fifo, "rb") as source, open(out, "wb") as sink:
    started = None
    total = 0
    while True:
        chunk = source.read(4096)
        if not chunk:
            break
        if started is None:
            started = time.time()
            with open(out + ".start", "w") as stamp:
                stamp.write(f"{started:.6f}\n")
        if kept < cap:
            keep = chunk[: cap - kept]
            sink.write(keep)
            kept += len(keep)
        total += len(chunk)
        ahead = total / (RATE * FRAME) - (time.time() - started)
        if ahead > 0:
            time.sleep(ahead)
