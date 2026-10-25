//! The terminal side: an intuiTUIve app whose one node is the game.
//!
//! The engine runs on a thread of its own, on the app's clock: a
//! [`fixed_step`] at Doom's 35 Hz gives it each tic ([`engine::Pace`]),
//! with the keys held then, and takes the frame it drew. After a slow
//! frame the step hands over every tic owed (up to [`MAX_CATCH_UP`]), so
//! the game keeps to the clock and the picture shows the newest frame.
//!
//! The picture is intuiTUIve's: a [`pixels_fit`] node in half blocks or
//! Braille, or for real pixels (Kitty, Sixel, iTerm2) [`ImageView`] and the
//! app's graphics side channel ([`Pictures`]). Keys are the app's held
//! keys, with releases made up where the terminal does not report them
//! ([`App::synthesize_releases`]); [`Keyboard`] turns them into Doom's
//! downs and ups. The frame rate on screen is [`rate`] of the frames, and
//! `--stats` reads the app's [`StatsHandle`].

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use intuituive::pixels::{pixels_fit, CLEAR};
use intuituive::prelude::*;
use intuituive::widget::{Canvas, DrawCx};
use intuituive::{fixed_step, StatsHandle};
use rich_interact::{Key, KeyCode};

use crate::engine::{self, Clock, Exit, Log, TIC};
use crate::graphics::{kitty_placeholders, Pictures, Protocol};
use crate::keys::Keyboard;
use crate::render::{draw_into, CellMode, CellRect, Fit, Frame, Resampler};

/// The most tics one step hands the engine after the app fell behind:
/// a quarter second; past that the game slows instead of jumping.
pub const MAX_CATCH_UP: u64 = 9;

/// How long a step waits for the engine to run its tics.
const STEP_WAIT: Duration = Duration::from_millis(250);

/// How the view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draw {
    Cells(CellMode),
    Image(Protocol),
}

/// The game's picture: the newest frame in `frame`, fitted by `fit`.
pub fn view(
    frame: Signal<Option<Arc<Frame>>>,
    draw: Draw,
    fit: Fit,
    pictures: Option<Arc<Pictures>>,
) -> Node {
    match (draw, pictures) {
        (Draw::Image(protocol), Some(pictures)) => widget(ImageView {
            frame,
            fit,
            protocol,
            pictures,
        }),
        (Draw::Cells(mode), _) => cells(frame, mode, fit),
        // No graphics source: half blocks.
        (Draw::Image(_), None) => cells(frame, CellMode::HalfBlocks, fit),
    }
}

/// The frame in cells: intuiTUIve's pixel buffer, drawn in `mode`.
fn cells(frame: Signal<Option<Arc<Frame>>>, mode: CellMode, fit: Fit) -> Node {
    let mut resampler = Resampler::new();
    pixels_fit(move |buf| {
        frame.with(|frame| match frame {
            Some(frame) => {
                draw_into(buf, frame, &fit, mode.cell(), &mut resampler);
            }
            None => buf.clear(CLEAR),
        })
    })
    .pixel_mode(mode)
}

/// The frame in real pixels: Kitty's placeholder cells (or blank cells
/// for Sixel and iTerm2) where the picture goes, and the frame handed to
/// [`Pictures`], the app's graphics source, at the cells' place on the
/// screen.
pub struct ImageView {
    frame: Signal<Option<Arc<Frame>>>,
    fit: Fit,
    protocol: Protocol,
    pictures: Arc<Pictures>,
}

impl Widget for ImageView {
    fn name(&self) -> &'static str {
        "doom-image"
    }

    fn draw(&mut self, cx: &mut DrawCx, canvas: &mut Canvas) {
        let Some(frame) = self.frame.get() else {
            return;
        };
        let (cols, rows) = (canvas.width(), canvas.height());
        // Sixel can scroll the screen when it reaches the last row.
        let rows = match self.protocol {
            Protocol::Sixel => rows.saturating_sub(1),
            _ => rows,
        };
        let place = self.fit.place(cols, rows);
        if self.protocol == Protocol::Kitty {
            let lines = kitty_placeholders(place.cols as usize, place.rows as usize);
            canvas.lines_at(place.x, place.y, place.cols, place.rows, &lines);
        }
        let at = cx.rect();
        self.pictures.show(
            frame,
            CellRect {
                x: at.x + place.x,
                y: at.y + place.y,
                ..place
            },
        );
    }
}

/// Everything [`build`] needs.
pub struct Options {
    pub iwad: std::path::PathBuf,
    /// Doom's own arguments.
    pub args: Vec<String>,
    pub sound: bool,
    pub music: bool,
    pub music_gain: f32,
    pub draw: Draw,
    pub fit: Fit,
    pub pictures: Option<Arc<Pictures>>,
    pub show_fps: bool,
    pub quit_after: Option<Duration>,
    /// How long a key whose release the terminal does not report is held
    /// after its last press or repeat.
    pub hold: Duration,
}

/// Counters for `--stats`.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Frames the app drew ([`FrameStats::frames`](intuituive::FrameStats)).
    pub frames: u64,
    /// Frames it held back to keep to a frame rate.
    pub skipped: u64,
    /// Bytes the last frame wrote.
    pub bytes: usize,
    /// The newest engine frame number shown.
    pub engine_frames: u64,
    /// Tics the engine was given.
    pub tics: u64,
    /// How long the app ran.
    pub elapsed: Duration,
}

impl Stats {
    /// Frames drawn a second, over the whole run.
    pub fn fps(&self) -> f64 {
        match self.elapsed.as_secs_f64() {
            s if s > 0.0 => self.frames as f64 / s,
            _ => 0.0,
        }
    }
}

/// What a built app reports back.
#[derive(Clone)]
pub struct Handles {
    /// The engine's exit, once the game quit (or failed to start).
    pub exit: Arc<Mutex<Option<Exit>>>,
    /// The app's frame counters.
    pub stats: StatsHandle,
    /// The newest engine frame shown, and the tics given.
    pub engine: Rc<RefCell<(u64, u64)>>,
    /// The newest frame shown.
    pub frame: Rc<RefCell<Option<Arc<Frame>>>>,
    pub log: Log,
}

/// Build the app: the engine starts on its own thread when the app does.
pub fn build(options: Options) -> (App, Handles) {
    let exit_slot: Arc<Mutex<Option<Exit>>> = Arc::new(Mutex::new(None));
    let log = Log::default();
    let engine_counts = Rc::new(RefCell::new((0u64, 0u64)));
    let newest: Rc<RefCell<Option<Arc<Frame>>>> = Rc::default();
    let stats_slot: Rc<RefCell<Option<StatsHandle>>> = Rc::new(RefCell::new(None));
    let Options {
        iwad,
        args,
        sound,
        music,
        music_gain,
        draw,
        fit,
        pictures,
        show_fps,
        quit_after,
        hold,
    } = options;
    let graphics = pictures.clone();
    let (slot, counts, engine_log, stats_read, shown) = (
        exit_slot.clone(),
        engine_counts.clone(),
        log.clone(),
        stats_slot.clone(),
        newest.clone(),
    );
    let app = App::new(move || {
        let frame: Signal<Option<Arc<Frame>>> = signal(None);
        let keyboard = Rc::new(RefCell::new(Keyboard::new()));
        let (keys, input) = mpsc::channel();
        let (mut pace, pacer) = engine::pace();
        let drawn: Arc<Mutex<Option<Frame>>> = Arc::default();
        let config = engine::Config {
            iwad,
            args,
            clock: Clock::Stepped(pacer),
            sound,
            music,
            music_gain,
            headless_audio: false,
        };
        let (frames_out, exit_out) = (drawn.clone(), slot.clone());
        let failed = slot.clone();
        let spawned = std::thread::Builder::new()
            .name("doom-engine".into())
            .spawn(move || {
                let result = engine::run(
                    config,
                    input,
                    move |f| *frames_out.lock().unwrap_or_else(|e| e.into_inner()) = Some(f),
                    move |exit| *exit_out.lock().unwrap_or_else(|e| e.into_inner()) = Some(exit),
                    engine_log,
                    None,
                );
                // Only an engine that could not start returns.
                if let Err(error) = result {
                    *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Exit {
                        code: 1,
                        log: error,
                    });
                }
            });
        if let Err(error) = spawned {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Exit {
                code: 1,
                log: format!("the engine's thread: {error}"),
            });
        }
        let stepping = keyboard.clone();
        let _timer = fixed_step(TIC, move |cx, steps| {
            for event in stepping.borrow_mut().step(&cx.held_keys()) {
                let _ = keys.send(event);
            }
            let alive = pace.step(steps, STEP_WAIT);
            counts.borrow_mut().1 = pace.given();
            if let Some(new) = drawn.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let new = Arc::new(new);
                counts.borrow_mut().0 = new.seq;
                *shown.borrow_mut() = Some(new.clone());
                frame.set(Some(new));
            }
            if !alive {
                cx.quit();
            }
        })
        .max_catch_up(MAX_CATCH_UP);
        if let Some(wait) = quit_after {
            after(wait, |cx| cx.quit());
        }
        let mut layers_of = vec![view(frame, draw, fit, pictures)];
        // Overlays take only their text's cells, at the top left.
        layers_of.push(
            text(move || match frame.with(Option::is_none) {
                true => "Starting Doom…".to_string(),
                false => String::new(),
            })
            .place(Align::Start, Align::Start),
        );
        if show_fps {
            let fps = rate(frame);
            layers_of.push(
                text(move || {
                    let bytes = stats_read
                        .borrow()
                        .as_ref()
                        .map_or(0, |stats| stats.get().bytes);
                    let muted = if crate::sound::muted() {
                        " · muted"
                    } else {
                        ""
                    };
                    format!(
                        "[bold white on black] {} fps · {} kB{muted} ",
                        fps.get(),
                        bytes / 1000
                    )
                })
                .place(Align::Start, Align::Start),
            );
        }
        let pressed = keyboard.clone();
        layers(layers_of).on_any_key(move |cx, key| on_key(cx, key, &mut pressed.borrow_mut()))
    })
    // Doom's keys are its own: no mouse, no text selection; every key
    // reported where the terminal can (modifiers alone too), releases made
    // up where it cannot.
    .mouse(Some(false))
    .selectable(false)
    .all_keys(true)
    .synthesize_releases(Some(hold));
    let app = match graphics {
        Some(source) => app.graphics(source as Arc<dyn rich_ext::frame::PlacementSource>),
        None => app,
    };
    *stats_slot.borrow_mut() = Some(app.stats_handle());
    let handles = Handles {
        exit: exit_slot,
        stats: app.stats_handle(),
        engine: engine_counts,
        frame: newest,
        log,
    };
    (app, handles)
}

/// A key pressed: the app's own (Ctrl+C quits, Ctrl+S mutes) or Doom's.
fn on_key(cx: &mut Ctx, key: Key, keyboard: &mut Keyboard) -> bool {
    match key.code {
        KeyCode::Char('c') if key.modifiers.ctrl => {
            cx.quit();
            true
        }
        KeyCode::Char('s') if key.modifiers.ctrl => {
            crate::sound::set_muted(!crate::sound::muted());
            true
        }
        _ => keyboard.press(&key),
    }
}

/// How the game ended.
pub struct Outcome {
    /// The engine's exit, when the game quit (else the player closed the
    /// app: Ctrl+C, or `--quit-after`).
    pub exit: Option<Exit>,
    pub stats: Stats,
    pub log: Log,
}

/// The counters `handles` hold, after `elapsed`.
pub fn stats(handles: &Handles, elapsed: Duration) -> Stats {
    let frame = handles.stats.get();
    let (engine_frames, tics) = *handles.engine.borrow();
    Stats {
        frames: frame.frames,
        skipped: frame.skipped,
        bytes: frame.bytes,
        engine_frames,
        tics,
        elapsed,
    }
}

/// Run the game in the terminal until it quits.
pub fn run(options: Options) -> std::io::Result<Outcome> {
    let (app, handles) = build(options);
    let started = Instant::now();
    app.run()?;
    let stats = stats(&handles, started.elapsed());
    let exit = handles
        .exit
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    Ok(Outcome {
        exit,
        stats,
        log: handles.log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Arc<Frame> {
        // Red over blue, left half; green, right half.
        let mut frame = Frame::filled(4, 2, 0x0000_ff00);
        frame.pixels[0] = 0x00ff_0000;
        frame.pixels[1] = 0x00ff_0000;
        frame.pixels[4] = 0x0000_00ff;
        frame.pixels[5] = 0x0000_00ff;
        frame.seq = 1;
        Arc::new(frame)
    }

    #[test]
    fn the_view_draws_the_frame_in_pixel_cells() {
        let shown = frame();
        let app = App::new(move || {
            let frame = signal(Some(shown.clone()));
            view(
                frame,
                Draw::Cells(CellMode::HalfBlocks),
                Fit::default(),
                None,
            )
        });
        // 16x4 cells: the 4:3 picture is 11 columns from column 2.
        let mut driver = app.driver(16, 4);
        driver.settle();
        let screen = driver.screen().plain();
        assert_eq!(screen.len(), 4);
        for row in &screen {
            let cells: Vec<char> = row.chars().collect();
            assert_eq!(&cells[..2], &[' ', ' '], "{screen:?}");
            assert_eq!(&cells[13..], &[' ', ' ', ' '], "{screen:?}");
            assert!(cells[2..13].iter().all(|&c| c == '▀'), "{screen:?}");
        }
        // Braille: the same cells, two colours each.
        let shown = frame();
        let app = App::new(move || {
            let frame = signal(Some(shown.clone()));
            view(frame, Draw::Cells(CellMode::Braille), Fit::default(), None)
        });
        let mut driver = app.driver(16, 4);
        driver.settle();
        let screen = driver.screen().plain();
        assert!(
            screen[0].chars().skip(2).take(11).all(|c| c != ' '),
            "{screen:?}"
        );
    }

    #[test]
    fn ctrl_c_quits_and_ctrl_s_mutes() {
        let app = App::new(|| {
            let keyboard = Rc::new(RefCell::new(Keyboard::new()));
            label("").on_any_key(move |cx, key| on_key(cx, key, &mut keyboard.borrow_mut()))
        });
        let mut driver = app.driver(4, 1);
        let before = crate::sound::muted();
        driver.press("ctrl+s");
        assert_eq!(crate::sound::muted(), before != crate::sound::AVAILABLE);
        crate::sound::set_muted(false);
        driver.press("ctrl+c");
        assert!(driver.is_done());
    }
}
