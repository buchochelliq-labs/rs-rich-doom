//! The Doom engine (doomgeneric, compiled by build.rs) and its platform
//! layer: the C shim (csrc/shim.c) forwards doomgeneric's `DG_*` calls to
//! the `rs_doom_*` functions here.
//!
//! The engine is one global C program, so it runs once per process, on the
//! thread that calls [`run`]. That thread keeps a [`Host`]: where frames go,
//! the key events for Doom, the clock, and what to do when the game exits.
//!
//! The clock is not the engine's own. In the app it is the app's: a
//! [`Pace`] moves it on a tic at a time from intuiTUIve's `fixed_step`,
//! and waits while the engine runs those tics, so the game keeps to the
//! app's clock (a headless driver's too) and draws once a tic. Tests run
//! it on a [`Clock::Virtual`] that moves when the engine sleeps.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::keys::Input;
use crate::render::Frame;

extern "C" {
    fn doomgeneric_Create(argc: c_int, argv: *mut *mut c_char);
    fn doomgeneric_Tick();
    fn rs_doom_resx() -> c_int;
    fn rs_doom_resy() -> c_int;
}

/// Doom's tics a second.
pub const TICRATE: u64 = 35;

/// The time between two tics.
pub const TIC: Duration = Duration::from_nanos(1_000_000_000 / TICRATE);

/// The engine's framebuffer size, as compiled.
pub fn resolution() -> (usize, usize) {
    // SAFETY: plain functions returning compile-time constants.
    unsafe { (rs_doom_resx() as usize, rs_doom_resy() as usize) }
}

/// How the engine tells time.
#[derive(Debug)]
pub enum Clock {
    /// A clock that only moves when the engine sleeps: tics run as fast as
    /// the machine allows, the same on every run. For tests.
    Virtual,
    /// Tics given by the app through the [`Pace`] that came with this
    /// [`Pacer`] ([`pace`]).
    Stepped(Pacer),
}

/// The app's side of a [`Clock::Stepped`]: moves the engine's clock on.
#[derive(Debug)]
pub struct Pace {
    steps: Sender<u64>,
    idle: Receiver<u64>,
    /// The tics given so far.
    given: u64,
}

/// The engine's side of a [`Clock::Stepped`].
#[derive(Debug)]
pub struct Pacer {
    steps: Receiver<u64>,
    idle: Sender<u64>,
}

/// A stepped clock: the [`Pace`] for the app, the [`Pacer`] for the
/// engine's [`Config::clock`].
pub fn pace() -> (Pace, Pacer) {
    let (steps_tx, steps_rx) = mpsc::channel();
    let (idle_tx, idle_rx) = mpsc::channel();
    (
        Pace {
            steps: steps_tx,
            idle: idle_rx,
            given: 0,
        },
        Pacer {
            steps: steps_rx,
            idle: idle_tx,
        },
    )
}

impl Pace {
    /// Move the engine's clock on by `tics`, and wait at most `wait` for
    /// the engine to run them and to wait for more (so their frame is
    /// drawn). False once the engine has gone (it exited, or never
    /// started).
    pub fn step(&mut self, tics: u64, wait: Duration) -> bool {
        self.given += tics;
        if self.steps.send(self.given).is_err() {
            return false;
        }
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.idle.recv_timeout(left) {
                Ok(at) if at >= self.given => return true,
                Ok(_) => {}
                // Slow (a level loading): it catches up on later steps.
                Err(RecvTimeoutError::Timeout) => return true,
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    /// The tics given so far.
    pub fn given(&self) -> u64 {
        self.given
    }
}

/// Why the engine stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    /// The engine's exit code: 0 when the player quit from the menu.
    pub code: i32,
    /// The last lines the engine printed (its error, for a failure).
    pub log: String,
}

/// What the engine needs to start.
#[derive(Debug)]
pub struct Config {
    pub iwad: PathBuf,
    /// More engine arguments (`-warp 1 1`, `-skill 3`, `-file x.wad`).
    pub args: Vec<String>,
    pub clock: Clock,
    /// Play sound effects (when built with the `sound` feature).
    pub sound: bool,
    /// Play music (OPL FM synthesis, with the `sound` feature).
    pub music: bool,
    /// The music's loudness over the game's own music volume (1.0 as is).
    pub music_gain: f32,
    /// No audio device: music is made only when
    /// [`crate::sound::render_music`] is called (tests).
    pub headless_audio: bool,
}

/// The engine's log: the lines it printed, newest last, bounded.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<LogLines>>);

#[derive(Default)]
struct LogLines {
    lines: VecDeque<String>,
    partial: String,
}

const LOG_LINES: usize = 200;

impl Log {
    fn push(&self, text: &str) {
        let mut log = self.0.lock().unwrap_or_else(|e| e.into_inner());
        log.partial.push_str(text);
        while let Some(end) = log.partial.find('\n') {
            let line: String = log.partial.drain(..=end).collect();
            log.lines.push_back(line.trim_end().to_string());
        }
        while log.lines.len() > LOG_LINES {
            log.lines.pop_front();
        }
    }

    /// The last `n` lines.
    pub fn tail(&self, n: usize) -> String {
        let log = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut lines: Vec<&str> = log.lines.iter().map(String::as_str).collect();
        if !log.partial.trim().is_empty() {
            lines.push(log.partial.trim_end());
        }
        let skip = lines.len().saturating_sub(n);
        lines[skip..].join("\n").trim().to_string()
    }
}

/// What the engine thread holds while the engine runs.
struct Host {
    on_frame: Box<dyn FnMut(Frame)>,
    on_exit: Box<dyn FnMut(Exit)>,
    input: Receiver<Input>,
    clock: Clock,
    /// The virtual clock's milliseconds, or the stepped clock's tics.
    time: u64,
    /// A timed demo's wall clock: `-timedemo` runs a tic per frame without
    /// sleeping, so the stepped clock never moves, and the engine measures
    /// the run's frame rate with `I_GetTime`.
    wall: Option<Instant>,
    /// The stepped clock's tics given by the app; `time` follows a tic at
    /// a time.
    given: u64,
    seq: u64,
    /// The last frame sent.
    last: Vec<u32>,
    log: Log,
    title: String,
}

impl Host {
    fn now_ms(&self) -> u64 {
        if let Some(start) = self.wall {
            return 1 + start.elapsed().as_millis() as u64;
        }
        match self.clock {
            Clock::Virtual => self.time,
            // Rounded up, so I_GetTime's `ms * 35 / 1000` is the tic itself;
            // from 1, as the engine reads 0 as "no base time yet".
            Clock::Stepped(_) => 1 + (self.time * 1000).div_ceil(TICRATE),
        }
    }
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// The running engine's log, for what C code prints from other threads
/// (the music's timer callbacks run on the audio thread).
static ENGINE_LOG: Mutex<Option<Log>> = Mutex::new(None);

fn with_host<T>(f: impl FnOnce(&mut Host) -> T) -> Option<T> {
    HOST.with(|host| host.borrow_mut().as_mut().map(f))
}

/// Run the engine on this thread: each frame goes to `on_frame`, key
/// events come from `input`, and when the game exits `on_exit` is told
/// and this thread parks for good (the process ends from elsewhere).
/// Returns only if `ticks` is `Some(n)`, after `n` engine ticks, or with
/// an error before the engine starts. Once per process.
pub fn run(
    config: Config,
    input: Receiver<Input>,
    on_frame: impl FnMut(Frame) + 'static,
    on_exit: impl FnMut(Exit) + 'static,
    log: Log,
    ticks: Option<u64>,
) -> Result<(), String> {
    if STARTED.swap(true, Ordering::SeqCst) {
        return Err("the Doom engine already ran in this process".into());
    }
    let iwad = config
        .iwad
        .canonicalize()
        .map_err(|e| format!("{}: {e}", config.iwad.display()))?;
    crate::sound::configure(crate::sound::Audio {
        sfx: config.sound,
        music: config.music,
        music_gain: config.music_gain,
        headless: config.headless_audio,
    });
    *ENGINE_LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(log.clone());
    let timed_demo = matches!(config.clock, Clock::Stepped(_))
        && config
            .args
            .iter()
            .any(|a| a.eq_ignore_ascii_case("-timedemo"));
    HOST.with(|host| {
        *host.borrow_mut() = Some(Host {
            on_frame: Box::new(on_frame),
            on_exit: Box::new(on_exit),
            input,
            clock: config.clock,
            time: 0,
            given: 0,
            wall: timed_demo.then(Instant::now),
            seq: 0,
            last: Vec::new(),
            log,
            title: String::new(),
        })
    });
    let mut args = vec![
        "rs-rich-doom".to_string(),
        "-iwad".to_string(),
        iwad.to_string_lossy().into_owned(),
    ];
    match (config.sound, config.music) {
        (false, false) => args.push("-nosound".into()),
        (false, true) => args.push("-nosfx".into()),
        (true, false) => args.push("-nomusic".into()),
        (true, true) => {}
    }
    args.extend(config.args);
    // The engine keeps argv for its whole life: leak it.
    let argv: Vec<*mut c_char> = args
        .into_iter()
        .map(|a| CString::new(a).unwrap_or_default().into_raw())
        .collect();
    let argc = argv.len() as c_int;
    let argv = Box::leak(argv.into_boxed_slice()).as_mut_ptr();
    // SAFETY: argv is argc valid C strings that live forever; the engine
    // runs on this thread only.
    unsafe { doomgeneric_Create(argc, argv) };
    let mut done = 0;
    loop {
        if ticks.is_some_and(|n| done >= n) {
            return Ok(());
        }
        // SAFETY: as above; the engine was created on this thread.
        unsafe { doomgeneric_Tick() };
        done += 1;
    }
}

// ---- The platform layer, called from csrc/shim.c on the engine thread ----

#[no_mangle]
extern "C" fn rs_doom_init() {}

#[no_mangle]
extern "C" fn rs_doom_draw_frame(pixels: *const u32, width: c_int, height: c_int) {
    let (w, h) = (width.max(0) as usize, height.max(0) as usize);
    if pixels.is_null() {
        return;
    }
    // SAFETY: the shim passes DG_ScreenBuffer, width * height pixels.
    let pixels = unsafe { std::slice::from_raw_parts(pixels, w * h) };
    // The engine redraws while it waits for a tic (up to twice a tic): a
    // frame like the last one is not sent again.
    if with_host(|host| host.last.as_slice() == pixels) == Some(true) {
        return;
    }
    let pixels = pixels.to_vec();
    with_host(|host| host.last.clone_from(&pixels));
    // Taken out while it runs, so a frame callback can never re-enter.
    let Some(mut sink) = with_host(|host| std::mem::replace(&mut host.on_frame, Box::new(|_| {})))
    else {
        return;
    };
    let seq = with_host(|host| {
        host.seq += 1;
        host.seq
    })
    .unwrap_or(0);
    sink(Frame {
        width: w,
        height: h,
        pixels,
        seq,
    });
    with_host(|host| host.on_frame = sink);
}

/// What a sleep comes to (with no host yet, the wall clock's).
enum Sleep {
    /// Done: the virtual clock moved, or the app gave more tics.
    Done,
    /// The app has gone: wait for the process to end.
    Orphaned,
}

#[no_mangle]
extern "C" fn rs_doom_sleep_ms(ms: u32) {
    let sleep = with_host(|host| match &host.clock {
        Clock::Virtual => {
            host.time += u64::from(ms.max(1));
            Sleep::Done
        }
        Clock::Stepped(pacer) => {
            // The clock moves one tic a sleep, never further: with the
            // classic sync, the engine builds at most five tic commands
            // ahead and drops the rest of a jump, so catching up after a
            // slow frame would lose tics.
            while let Ok(at) = pacer.steps.try_recv() {
                host.given = host.given.max(at);
            }
            if host.given > host.time {
                host.time += 1;
                return Sleep::Done;
            }
            // The engine has run every tic it was given: say so, and wait
            // for the next.
            let _ = pacer.idle.send(host.time);
            match pacer.steps.recv() {
                Ok(at) => {
                    host.given = host.given.max(at);
                    if host.given > host.time {
                        host.time += 1;
                    }
                    Sleep::Done
                }
                Err(_) => Sleep::Orphaned,
            }
        }
    });
    match sleep {
        None => std::thread::sleep(Duration::from_millis(u64::from(ms))),
        Some(Sleep::Done) => {}
        Some(Sleep::Orphaned) => loop {
            std::thread::park();
        },
    }
}

#[no_mangle]
extern "C" fn rs_doom_ticks_ms() -> u32 {
    with_host(|host| host.now_ms() as u32).unwrap_or(0)
}

#[no_mangle]
extern "C" fn rs_doom_get_key(pressed: *mut c_int, key: *mut u8) -> c_int {
    let event = with_host(|host| host.input.try_recv().ok()).flatten();
    let (down, k) = match event {
        Some(Input::Down(k)) => (true, k),
        Some(Input::Up(k)) => (false, k),
        None => return 0,
    };
    if pressed.is_null() || key.is_null() {
        return 0;
    }
    // SAFETY: the engine passes two valid out-pointers.
    unsafe {
        *pressed = c_int::from(down);
        *key = k;
    }
    1
}

#[no_mangle]
extern "C" fn rs_doom_set_title(title: *const c_char) {
    if title.is_null() {
        return;
    }
    // SAFETY: a NUL-terminated string from the engine.
    let title = unsafe { CStr::from_ptr(title) }
        .to_string_lossy()
        .into_owned();
    with_host(|host| host.title = title);
}

#[no_mangle]
extern "C" fn rs_doom_log(text: *const c_char, len: usize) {
    if text.is_null() {
        return;
    }
    // SAFETY: `len` bytes from the shim's buffer.
    let bytes = unsafe { std::slice::from_raw_parts(text as *const u8, len) };
    let text = String::from_utf8_lossy(bytes);
    if with_host(|host| host.log.push(&text)).is_none() {
        let log = ENGINE_LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match log {
            // Another thread of the running engine.
            Some(log) => log.push(&text),
            // Before the engine has a host: nowhere better.
            None => eprint!("{text}"),
        }
    }
}

#[no_mangle]
extern "C" fn rs_doom_exit(code: c_int) -> ! {
    let exit = with_host(|host| Exit {
        code,
        log: host.log.tail(12),
    });
    if let Some(exit) = exit {
        if let Some(mut tell) =
            with_host(|host| std::mem::replace(&mut host.on_exit, Box::new(|_| {})))
        {
            tell(exit);
        }
    }
    // The host goes, and with it the stepped clock: an app waiting on a
    // step learns at once that the engine has gone.
    let host = HOST.with(|host| host.borrow_mut().take());
    drop(host);
    // The engine cannot go on after exit(); the process ends from the
    // terminal side, which restores the terminal first.
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catch_up_moves_the_clock_a_tic_at_a_time() {
        // The classic sync builds at most five tic commands ahead and drops
        // the rest of a jump: nine tics given at once must reach the engine
        // as nine sleeps of one tic each, none of them waiting.
        let (pace, pacer) = pace();
        let (_keys, input) = mpsc::channel();
        HOST.with(|host| {
            *host.borrow_mut() = Some(Host {
                on_frame: Box::new(|_| {}),
                on_exit: Box::new(|_| {}),
                input,
                clock: Clock::Stepped(pacer),
                time: 0,
                given: 0,
                wall: None,
                seq: 0,
                last: Vec::new(),
                log: Log::default(),
                title: String::new(),
            })
        });
        pace.steps.send(9).unwrap();
        for tic in 1..=9 {
            rs_doom_sleep_ms(1);
            assert_eq!(with_host(|host| host.time), Some(tic));
        }
        assert!(
            pace.idle.try_recv().is_err(),
            "not idle while tics are left"
        );
        HOST.with(|host| host.borrow_mut().take());
    }

    #[test]
    fn a_timed_demo_reads_the_wall_clock() {
        // -timedemo never sleeps, so the stepped clock would stand still and
        // the engine's measured frame rate would divide by no time at all.
        let (_pace, pacer) = pace();
        let (_keys, input) = mpsc::channel();
        let host = Host {
            on_frame: Box::new(|_| {}),
            on_exit: Box::new(|_| {}),
            input,
            clock: Clock::Stepped(pacer),
            time: 0,
            given: 0,
            wall: Some(Instant::now()),
            seq: 0,
            last: Vec::new(),
            log: Log::default(),
            title: String::new(),
        };
        let before = host.now_ms();
        assert!(before >= 1, "0 reads as no base time yet");
        std::thread::sleep(Duration::from_millis(60));
        assert!(host.now_ms() >= before + 60);
    }

    #[test]
    fn a_stepped_clock_reads_whole_tics() {
        // I_GetTime is (ms - base) * 35 / 1000: each tic must read as
        // itself.
        let (_pace, pacer) = pace();
        let (_keys, input) = mpsc::channel();
        let mut host = Host {
            on_frame: Box::new(|_| {}),
            on_exit: Box::new(|_| {}),
            input,
            clock: Clock::Stepped(pacer),
            time: 0,
            given: 0,
            wall: None,
            seq: 0,
            last: Vec::new(),
            log: Log::default(),
            title: String::new(),
        };
        let base = host.now_ms();
        for tic in 0..10_000u64 {
            host.time = tic;
            assert_eq!((host.now_ms() - base) * TICRATE / 1000, tic);
        }
        assert_eq!(TIC.as_micros(), 28_571);
    }

    #[test]
    fn pace_waits_for_the_engine_and_learns_it_has_gone() {
        let (mut pace, pacer) = pace();
        let engine = std::thread::spawn(move || {
            // Two tics' work, reporting idle after each, then exit.
            for _ in 0..2 {
                let at = pacer.steps.recv().unwrap();
                pacer.idle.send(at).unwrap();
            }
        });
        assert!(pace.step(1, Duration::from_secs(5)));
        assert!(pace.step(2, Duration::from_secs(5)));
        assert_eq!(pace.given(), 3);
        engine.join().unwrap();
        assert!(!pace.step(1, Duration::from_secs(5)), "gone");
    }
}
