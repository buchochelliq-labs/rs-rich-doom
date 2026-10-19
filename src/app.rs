//! The terminal side: an intuiTUIve app whose one node is the game.
//!
//! The engine runs on a thread of its own (a `latest` thread), ticking at
//! 35 Hz and sending each frame it draws; the app keeps only the newest
//! (`Latest`), so a slow terminal skips frames instead of falling behind.
//! The [`DoomView`] widget draws the newest frame on each turn, as half
//! blocks or Braille, or as an image (Kitty, Sixel, iTerm2) through the
//! app's graphics side channel, and turns key events into Doom's key-downs
//! and key-ups.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use intuituive::prelude::*;
use intuituive::widget::{Canvas, DrawCx, EventCx, Used, WidgetEvent};
use rich::Style;

use crate::engine::{self, Exit, Log};
use crate::graphics::{kitty_placeholders, Pictures, Protocol};
use crate::keys::{Input, Translator};
use crate::render::{cells, CellMode, CellRect, Fit, Frame, Resampler};

/// What the engine thread sends the app.
#[derive(Clone)]
pub enum Update {
    Frame(Arc<Frame>),
    Exit(Exit),
}

/// How the view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draw {
    Cells(CellMode),
    Image(Protocol),
}

/// Counters for `--stats` and `--fps`.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Frames the view drew.
    pub drawn: u64,
    /// The newest engine frame number seen.
    pub engine_frames: u64,
    /// Time spent converting and writing frames into the screen.
    pub draw_time: Duration,
    pub first: Option<Instant>,
    pub last: Option<Instant>,
    /// Draws in the last second, for the counter on screen.
    recent: std::collections::VecDeque<Instant>,
}

impl Stats {
    /// Frames drawn a second, over the whole run.
    pub fn fps(&self) -> f64 {
        match (self.first, self.last) {
            (Some(a), Some(b)) if b > a && self.drawn > 1 => {
                (self.drawn - 1) as f64 / (b - a).as_secs_f64()
            }
            _ => 0.0,
        }
    }

    /// Mean time a draw took.
    pub fn mean_draw(&self) -> Duration {
        if self.drawn == 0 {
            Duration::ZERO
        } else {
            self.draw_time / self.drawn as u32
        }
    }

    fn record(&mut self, now: Instant, took: Duration, seq: u64) {
        self.drawn += 1;
        self.engine_frames = self.engine_frames.max(seq);
        self.draw_time += took;
        self.first.get_or_insert(now);
        self.last = Some(now);
        self.recent.push_back(now);
        while self
            .recent
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1))
        {
            self.recent.pop_front();
        }
    }
}

/// The game's view: draws the newest frame, takes the keys.
pub struct DoomView {
    update: Signal<Option<Update>>,
    draw: Draw,
    fit: Fit,
    keys: Sender<Input>,
    translator: Translator,
    resampler: Resampler,
    pictures: Option<Arc<Pictures>>,
    show_fps: bool,
    stats: Arc<Mutex<Stats>>,
    last_seq: u64,
}

impl DoomView {
    pub fn new(
        update: Signal<Option<Update>>,
        draw: Draw,
        fit: Fit,
        keys: Sender<Input>,
        pictures: Option<Arc<Pictures>>,
        show_fps: bool,
        stats: Arc<Mutex<Stats>>,
    ) -> DoomView {
        DoomView {
            update,
            draw,
            fit,
            keys,
            translator: Translator::new(),
            resampler: Resampler::new(),
            pictures,
            show_fps,
            stats,
            last_seq: 0,
        }
    }

    fn send(&self, inputs: Vec<Input>) {
        for input in inputs {
            // The engine thread gone means the game ended: nothing to do.
            let _ = self.keys.send(input);
        }
    }
}

impl Widget for DoomView {
    fn name(&self) -> &'static str {
        "doom"
    }

    fn focusable(&self) -> bool {
        true
    }

    fn access_text(&self) -> Option<String> {
        Some("Doom".into())
    }

    fn draw(&mut self, _cx: &mut DrawCx, canvas: &mut Canvas) {
        let frame = self.update.with(|u| match u {
            Some(Update::Frame(frame)) => Some(frame.clone()),
            _ => None,
        });
        let (cols, rows) = (canvas.width(), canvas.height());
        let Some(frame) = frame else {
            canvas.print(0, 0, "Starting Doom…", None);
            return;
        };
        let started = Instant::now();
        // Sixel can scroll the screen when it reaches the last row.
        let area_rows = match self.draw {
            Draw::Image(Protocol::Sixel) => rows.saturating_sub(1),
            _ => rows,
        };
        let place = self.fit.place(cols, area_rows);
        match self.draw {
            Draw::Cells(mode) => {
                let (dx, dy) = mode.density();
                let (c, r) = (place.cols as usize, place.rows as usize);
                let pixels = self.resampler.resample(&frame, c * dx, r * dy);
                let lines = cells(pixels, c, r, mode);
                canvas.lines_at(place.x, place.y, place.cols, place.rows, &lines);
            }
            Draw::Image(protocol) => {
                if protocol == Protocol::Kitty {
                    let lines = kitty_placeholders(place.cols as usize, place.rows as usize);
                    canvas.lines_at(place.x, place.y, place.cols, place.rows, &lines);
                }
                if let Some(pictures) = &self.pictures {
                    // The view fills the screen from its top left corner,
                    // so its cells are the screen's.
                    pictures.show(frame.clone(), CellRect { ..place });
                }
            }
        }
        let took = started.elapsed();
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        if frame.seq != self.last_seq {
            self.last_seq = frame.seq;
            stats.record(Instant::now(), took, frame.seq);
        }
        if self.show_fps {
            let text = format!(
                " {} fps · {:.1} ms ",
                stats.recent.len(),
                took.as_secs_f64() * 1000.0
            );
            let style = Style::parse("bold white on black").ok();
            canvas.print(0, 0, &text, style.as_ref());
        }
    }

    fn event(&mut self, _cx: &mut EventCx, event: &WidgetEvent) -> Used {
        match event {
            WidgetEvent::Key(key) => {
                let inputs = self.translator.press(key);
                let used = !inputs.is_empty();
                self.send(inputs);
                if used {
                    Used::Yes
                } else {
                    Used::No
                }
            }
            WidgetEvent::KeyUp(key) => {
                let inputs = self.translator.release(key);
                self.send(inputs);
                Used::Yes
            }
            _ => Used::No,
        }
    }
}

/// Everything [`run`] needs.
pub struct Options {
    pub engine: engine::Config,
    pub draw: Draw,
    pub fit: Fit,
    pub pictures: Option<Arc<Pictures>>,
    pub show_fps: bool,
    pub quit_after: Option<Duration>,
}

/// How the game ended.
pub struct Outcome {
    /// The engine's exit, when the game quit (else the player closed the
    /// app: Ctrl+C, or `--quit-after`).
    pub exit: Option<Exit>,
    pub stats: Stats,
    pub log: Log,
}

/// Build the app.
pub fn build(options: Options, stats: Arc<Mutex<Stats>>, log: Log) -> (App, Arc<Mutex<Option<Exit>>>) {
    let exit_slot: Arc<Mutex<Option<Exit>>> = Arc::new(Mutex::new(None));
    let slot = exit_slot.clone();
    let Options {
        engine: config,
        draw,
        fit,
        pictures,
        show_fps,
        quit_after,
    } = options;
    let graphics = pictures.clone();
    let mut app = App::new(move || {
        let (keys, input) = std::sync::mpsc::channel();
        let engine_log = log.clone();
        let update = latest(move |sender: intuituive::Latest<Update>| {
            let sender = std::rc::Rc::new(sender);
            let frames = sender.clone();
            let result = engine::run(
                config,
                input,
                move |frame| {
                    frames.send(Update::Frame(Arc::new(frame)));
                },
                move |exit| {
                    sender.send(Update::Exit(exit));
                },
                engine_log,
                None,
            );
            // Only an engine that could not start returns.
            if let Err(error) = result {
                eprintln!("{error}");
            }
        });
        on_write_ref(update, move |value: &Option<Update>, cx| {
            if let Some(Update::Exit(exit)) = value {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(exit.clone());
                cx.quit();
            }
        });
        if let Some(wait) = quit_after {
            after(wait, |cx| cx.quit());
        }
        widget(DoomView::new(update, draw, fit, keys, pictures, show_fps, stats))
    })
    // Doom's keys are its own: no mouse, no text selection.
    .mouse(Some(false))
    .selectable(false);
    if let Some(source) = graphics {
        app = app.graphics(source as Arc<dyn rich_ext::frame::PlacementSource>);
    }
    (app, exit_slot)
}

/// Run the game in the terminal until it quits.
pub fn run(options: Options) -> std::io::Result<Outcome> {
    let stats = Arc::new(Mutex::new(Stats::default()));
    let log = Log::default();
    let (app, exit) = build(options, stats.clone(), log.clone());
    app.run()?;
    let exit = exit.lock().unwrap_or_else(|e| e.into_inner()).take();
    let stats = stats.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(Outcome { exit, stats, log })
}
