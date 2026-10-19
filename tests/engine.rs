//! The engine, headless: runs real tics on an IWAD and checks the frames.
//! Needs `RS_RICH_DOOM_IWAD` (say, Freedoom's freedoom1.wad); skipped
//! without it. One engine per process, so one test in this file.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rs_rich_doom::engine::{self, Clock, Config, Log};
use rs_rich_doom::keys::{doom, Hold, HoldTimes, Input};
use rs_rich_doom::render::{cells, CellMode, Fit, Frame, Resampler};

/// Enough for the title, the menu and the first demo's start.
const TICS: u64 = 400;

#[test]
fn runs_tics_headless_and_draws_frames() {
    let Some(iwad) = std::env::var_os("RS_RICH_DOOM_IWAD") else {
        eprintln!("RS_RICH_DOOM_IWAD is not set: skipped");
        return;
    };
    let frames: Arc<Mutex<Vec<Frame>>> = Arc::default();
    let (done_tx, done_rx) = mpsc::channel();
    let seen = frames.clone();
    let log = Log::default();
    let engine_log = log.clone();
    // A data directory of its own for default.cfg.
    let dir = std::env::temp_dir().join(format!("rs-rich-doom-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let iwad = std::path::PathBuf::from(iwad).canonicalize().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    std::thread::spawn(move || {
        let (keys, input) = mpsc::channel();
        // Press Escape a little in: the menu opens over the demo.
        keys.send(Input::Down {
            key: doom::ESCAPE,
            hold: Hold::Tap,
        })
        .unwrap();
        let result = engine::run(
            Config {
                iwad,
                args: Vec::new(),
                clock: Clock::Virtual,
                hold: HoldTimes::default(),
                sound: false,
            },
            input,
            move |frame| seen.lock().unwrap().push(frame),
            |exit| panic!("the engine exited: {exit:?}"),
            engine_log,
            Some(TICS),
        );
        drop(keys);
        done_tx.send(result).unwrap();
    });
    let result = done_rx
        .recv_timeout(Duration::from_secs(120))
        .unwrap_or_else(|_| {
            panic!(
                "the tics did not finish; the engine said:\n{}",
                log.tail(30)
            )
        });
    result.unwrap();

    let frames = frames.lock().unwrap();
    // Frames like the one before are not sent: the title screen is one.
    assert!(frames.len() >= 20, "{} frames", frames.len());
    let (w, h) = engine::resolution();
    let last = frames.last().unwrap();
    assert_eq!((last.width, last.height), (w, h));
    assert_eq!((w, h), (320, 200));
    assert_eq!(last.seq as usize, frames.len());
    // A real picture: many colours, and the frames change over time.
    let colours: std::collections::HashSet<u32> = last.pixels.iter().copied().collect();
    assert!(colours.len() > 16, "{} colours", colours.len());
    let distinct: std::collections::HashSet<&[u32]> =
        frames.iter().map(|f| f.pixels.as_slice()).collect();
    assert!(distinct.len() > 10, "{} distinct frames", distinct.len());
    assert!(log.tail(50).contains("R_Init"), "{}", log.tail(50));

    // And it converts to cells.
    let place = Fit::default().place(80, 24);
    let mut resampler = Resampler::new();
    let px = resampler.resample(last, place.cols as usize, place.rows as usize * 2);
    let lines = cells(
        px,
        place.cols as usize,
        place.rows as usize,
        CellMode::HalfBlocks,
    );
    assert_eq!(lines.len(), 24);
}
