//! What a frame costs on the terminal side, without a terminal: real frames
//! from the engine (headless, on its virtual clock) drawn by the app's
//! headless driver, at a size and in a mode.
//!
//! ```text
//! cargo run --release --example draw_bench -- IWAD [COLS ROWS] [half|braille|kitty|sixel]
//! ```

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use intuituive::prelude::*;
use rs_rich_doom::app::{DoomView, Draw, Stats, Update};
use rs_rich_doom::engine::{self, Clock, Config, Log};
use rs_rich_doom::graphics::{Pictures, Protocol};
use rs_rich_doom::keys::HoldTimes;
use rs_rich_doom::render::{CellMode, Fit, Frame};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let iwad = args
        .first()
        .expect("usage: draw_bench IWAD [COLS ROWS] [MODE]");
    let cols: u16 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(80);
    let rows: u16 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(24);
    let draw = match args.get(3).map(String::as_str).unwrap_or("half") {
        "braille" => Draw::Cells(CellMode::Braille),
        "kitty" => Draw::Image(Protocol::Kitty),
        "sixel" => Draw::Image(Protocol::Sixel),
        _ => Draw::Cells(CellMode::HalfBlocks),
    };

    // Frames of the first demo.
    let frames: Arc<Mutex<Vec<Frame>>> = Arc::default();
    let seen = frames.clone();
    let (_keys, input) = mpsc::channel();
    let config = Config {
        iwad: iwad.into(),
        args: vec!["-playdemo".into(), "demo1".into()],
        clock: Clock::Virtual,
        hold: HoldTimes::default(),
        sound: false,
        music: false,
        music_gain: 1.0,
        headless_audio: false,
    };
    let engine_run = std::thread::spawn(move || {
        let _ = engine::run(
            config,
            input,
            move |f| seen.lock().unwrap().push(f),
            |_| {},
            Log::default(),
            Some(700),
        );
    });
    engine_run.join().unwrap();
    let frames: Vec<Arc<Frame>> = frames.lock().unwrap().drain(..).map(Arc::new).collect();
    let count = frames.len();
    eprintln!("{count} frames from the engine");

    let pictures = match draw {
        Draw::Image(p) => Some(Arc::new(Pictures::new(p, (8, 16), 640))),
        Draw::Cells(_) => None,
    };
    let stats = Arc::new(Mutex::new(Stats::default()));
    let (keys, _input) = mpsc::channel();
    let source = pictures.clone();
    let shared = Arc::new(Mutex::new(frames));
    let mut app = App::new(move || {
        let update = signal(None);
        let mut next = 0usize;
        let list = shared.clone();
        every(Duration::from_millis(28), move |_| {
            let list = list.lock().unwrap();
            let frame = list[next % list.len()].clone();
            update.update(|u| *u = Some(Update::Frame(frame)));
            next += 1;
        });
        widget(DoomView::new(
            update,
            draw,
            Fit::default(),
            keys,
            source,
            false,
            stats,
        ))
    });
    if let Some(p) = pictures {
        app = app.graphics(p as Arc<dyn rich_ext::frame::PlacementSource>);
    }
    let mut driver = app.driver(cols, rows);
    driver.settle();
    let (mut total, mut bytes, mut n) = (Duration::ZERO, 0usize, 0u32);
    for _ in 0..count.min(300) {
        let start = Instant::now();
        driver.update(driver_now(&mut n));
        if let Some(out) = driver.render() {
            bytes += out.len();
        }
        total += start.elapsed();
    }
    let frames = n.max(1);
    println!(
        "{cols}x{rows} {draw:?}: {:.2} ms a frame (update + render), {} bytes a frame",
        total.as_secs_f64() * 1000.0 / frames as f64,
        bytes / frames as usize
    );
}

/// The driver's clock for the next frame.
fn driver_now(n: &mut u32) -> Duration {
    *n += 1;
    Duration::from_millis(28 * u64::from(*n) + 1)
}
