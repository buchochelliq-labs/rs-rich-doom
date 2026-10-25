//! The whole app headless, with the real engine: intuiTUIve's driver runs
//! it on its own clock, so `fixed_step` gives the engine exactly 35 tics a
//! second of driver time, and keys sent to the app reach Doom through the
//! app's held keys. Then a long run in Braille (colours averaged, the
//! most styles) measures whether the screen's style table settles.
//!
//! Needs `RS_RICH_DOOM_IWAD` (Freedoom's freedoom1.wad); skipped without
//! it. One engine per process, so one test in this file.

use std::time::Duration;

use rs_rich_doom::app::{self, Draw, Options};
use rs_rich_doom::render::{CellMode, Fit, Frame};

use intuituive::interact::{Event, Key, Releases};

/// The share of pixels that differ between two frames.
fn changed(a: &Frame, b: &Frame) -> f64 {
    let differ = a
        .pixels
        .iter()
        .zip(&b.pixels)
        .filter(|(x, y)| x != y)
        .count();
    differ as f64 / a.pixels.len() as f64
}

/// Resident memory, in kB (Linux), for a coarse look at growth.
fn rss_kb() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4)
}

#[test]
fn the_app_runs_doom_on_its_clock_with_held_keys() {
    let Some(iwad) = std::env::var_os("RS_RICH_DOOM_IWAD") else {
        assert!(
            std::env::var_os("RS_RICH_DOOM_REQUIRE_IWAD").is_none(),
            "RS_RICH_DOOM_IWAD is not set"
        );
        eprintln!("RS_RICH_DOOM_IWAD is not set: skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rs-rich-doom-app-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let iwad = std::path::PathBuf::from(iwad).canonicalize().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let (app, handles) = app::build(Options {
        iwad,
        args: ["-warp", "1", "1", "-skill", "1"]
            .map(String::from)
            .to_vec(),
        sound: false,
        music: false,
        music_gain: 1.0,
        draw: Draw::Cells(CellMode::Braille),
        fit: Fit::default(),
        pictures: None,
        show_fps: true,
        quit_after: None,
        hold: Duration::from_millis(400),
    });
    let mut driver = app.driver(80, 24);
    driver.set_releases(Releases::All);
    driver.settle();
    let frame = || handles.frame.borrow().clone().expect("a frame");
    let tics = || handles.engine.borrow().1;

    // On the app's clock: one second of driver time is 35 tics.
    driver.advance(Duration::from_secs(3));
    assert_eq!(tics(), 105, "tics in 3 s");
    assert!(
        handles.exit.lock().unwrap().is_none(),
        "{}",
        handles.log.tail(20)
    );

    // Standing still, the view barely changes (lights, the status bar).
    let a = frame();
    driver.advance(Duration::from_secs(1));
    let b = frame();
    let still = changed(&a, &b);

    // Up held (its release reported): the player walks, the view moves.
    let up = Key::parse("up").unwrap();
    driver.send(Event::Key(up));
    driver.advance(Duration::from_secs(1));
    let c = frame();
    let walking = changed(&b, &c);
    driver.send(Event::KeyUp(up));
    // The player slides on a little (Doom's momentum).
    driver.advance(Duration::from_millis(2500));
    let d = frame();
    driver.advance(Duration::from_secs(1));
    let e = frame();
    let stopped = changed(&d, &e);
    eprintln!(
        "pixels changed in 1 s: standing {:.1}%, walking {:.1}%, after letting go {:.1}%",
        still * 100.0,
        walking * 100.0,
        stopped * 100.0
    );
    assert!(walking > 0.2, "walking changed {walking}");
    assert!(
        still < 0.1 && stopped < 0.1,
        "still {still}, stopped {stopped}"
    );

    // A tap: Escape opens the menu over the game, Escape closes it.
    driver.send(Event::Key(Key::parse("escape").unwrap()));
    driver.advance(Duration::from_millis(300));
    let menu = frame();
    let opened = changed(&e, &menu);
    driver.send(Event::Key(Key::parse("escape").unwrap()));
    driver.advance(Duration::from_millis(600));
    let closed = changed(&e, &frame());
    eprintln!(
        "the menu: {:.1}% of pixels when open, {:.1}% after it closed",
        opened * 100.0,
        closed * 100.0
    );
    assert!(opened > 0.05 && closed < opened, "{opened} {closed}");

    // Drawn once a tic: about as many frames as tics, each in Braille.
    let stats = driver.stats();
    let screen = driver.screen().plain();
    assert!(
        screen[5]
            .chars()
            .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "{screen:#?}"
    );
    assert!(screen[0].contains("fps"), "{:?}", screen[0]);
    eprintln!("{} frames for {} tics", stats.frames, tics());
    assert!(stats.frames >= tics() / 2, "{} frames", stats.frames);

    // A long run, walking about: the style table settles.
    let mut counts = Vec::new();
    let rss_start = rss_kb();
    let turn = Key::parse("left").unwrap();
    for second in 0..120u64 {
        if second % 10 == 0 {
            driver.send(Event::Key(up));
        }
        if second % 10 == 5 {
            driver.send(Event::KeyUp(up));
            driver.send(Event::Key(turn));
        }
        if second % 10 == 7 {
            driver.send(Event::KeyUp(turn));
        }
        driver.advance(Duration::from_secs(1));
        counts.push(driver.screen().style_count());
    }
    let rss_end = rss_kb();
    let peak = counts.iter().copied().max().unwrap_or(0);
    let late_peak = counts[60..].iter().copied().max().unwrap_or(0);
    eprintln!(
        "styles kept over 120 s of Braille: first {}, peak {peak}, peak in the last minute \
         {late_peak}, last {}; resident {:?} kB -> {:?} kB; {} tics",
        counts[0],
        counts[counts.len() - 1],
        rss_start,
        rss_end,
        tics()
    );
    // The table is compacted once it holds more than twice the styles its
    // cells used at the last compaction (or 1024): at most twice the
    // screen's cells. A table that never shrank would hold every colour
    // pair the run made.
    assert!(late_peak <= 2 * 80 * 24, "{counts:?}");
    // 129.4 s of driver time in all: every tic of it, none skipped.
    assert_eq!(tics(), 129_400 * 35 / 1000, "on the clock throughout");
    std::fs::remove_dir_all(&dir).ok();
}
