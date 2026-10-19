//! The engine, headless: runs real tics on an IWAD and checks the frames,
//! then the music it plays (OPL, with no audio device). Needs
//! `RS_RICH_DOOM_IWAD` (say, Freedoom's freedoom1.wad); skipped without
//! it. One engine per process, so one test in this file.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rs_rich_doom::engine::{self, Clock, Config, Log};
use rs_rich_doom::keys::{doom, Hold, HoldTimes, Input};
use rs_rich_doom::render::{cells, CellMode, Fit, Frame, Resampler};
use rs_rich_doom::sound;

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
                music: true,
                music_gain: 1.0,
                headless_audio: true,
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

    // The music the engine started (the first demo's level by now), made
    // without a device: three seconds of it, in 10 ms blocks as an audio
    // callback would ask.
    if sound::AVAILABLE {
        let rate = sound::HEADLESS_RATE_HZ as usize;
        let mut block = vec![0i16; rate / 100 * 2];
        let mut peak = 0i32;
        let mut energy = 0f64;
        let mut samples = 0usize;
        let started = std::time::Instant::now();
        for _ in 0..300 {
            assert!(sound::render_music(&mut block), "no music is playing");
            for &s in &block {
                peak = peak.max(i32::from(s).abs());
                energy += f64::from(s) * f64::from(s);
            }
            samples += block.len();
        }
        let took = started.elapsed();
        let rms = (energy / samples as f64).sqrt();
        eprintln!(
            "music: 3 s made in {:.1} ms ({:.2}% of real time), peak {peak}, rms {rms:.0}",
            took.as_secs_f64() * 1000.0,
            took.as_secs_f64() / 3.0 * 100.0
        );
        assert!(
            peak > 1000 && rms > 100.0,
            "silent music: peak {peak}, rms {rms}"
        );
        assert!(
            log.tail(200)
                .contains("OPL_Init: Using driver 'rs-rich-doom'"),
            "{}",
            log.tail(200)
        );
    }
}
