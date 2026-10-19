//! The Doom engine (doomgeneric, compiled by build.rs) and its platform
//! layer: the C shim (csrc/shim.c) forwards doomgeneric's `DG_*` calls to
//! the `rs_doom_*` functions here.
//!
//! The engine is one global C program, so it runs once per process, on the
//! thread that calls [`run`]. That thread keeps a [`Host`]: where frames go,
//! the keys Doom holds, the clock, and what to do when the game exits.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::keys::{HoldTimes, Input, KeyState};
use crate::render::Frame;

extern "C" {
    fn doomgeneric_Create(argc: c_int, argv: *mut *mut c_char);
    fn doomgeneric_Tick();
    fn rs_doom_resx() -> c_int;
    fn rs_doom_resy() -> c_int;
}

/// The engine's framebuffer size, as compiled.
pub fn resolution() -> (usize, usize) {
    // SAFETY: plain functions returning compile-time constants.
    unsafe { (rs_doom_resx() as usize, rs_doom_resy() as usize) }
}

/// How the engine tells time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    /// The wall clock: the game runs at 35 tics a second.
    Real,
    /// A clock that only moves when the engine sleeps: tics run as fast as
    /// the machine allows, the same on every run. For tests.
    Virtual,
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
#[derive(Clone, Debug)]
pub struct Config {
    pub iwad: PathBuf,
    /// More engine arguments (`-warp 1 1`, `-skill 3`, `-file x.wad`).
    pub args: Vec<String>,
    pub clock: Clock,
    pub hold: HoldTimes,
    /// Play sound effects (when built with the `sound` feature).
    pub sound: bool,
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
    keys: KeyState,
    clock: Clock,
    started: Instant,
    virtual_ms: u64,
    seq: u64,
    /// The last frame sent.
    last: Vec<u32>,
    log: Log,
    title: String,
}

impl Host {
    fn now_ms(&self) -> u64 {
        match self.clock {
            Clock::Real => self.started.elapsed().as_millis() as u64,
            Clock::Virtual => self.virtual_ms,
        }
    }
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

static STARTED: AtomicBool = AtomicBool::new(false);

fn with_host<T>(f: impl FnOnce(&mut Host) -> T) -> Option<T> {
    HOST.with(|host| host.borrow_mut().as_mut().map(f))
}

/// Run the engine on this thread: each frame goes to `on_frame`, keys come
/// from `input`, and when the game exits `on_exit` is told and this thread
/// parks for good (the process ends from elsewhere). Returns only if
/// `ticks` is `Some(n)`, after `n` engine ticks, or with an error before
/// the engine starts. Once per process.
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
    crate::sound::configure(config.sound);
    HOST.with(|host| {
        *host.borrow_mut() = Some(Host {
            on_frame: Box::new(on_frame),
            on_exit: Box::new(on_exit),
            input,
            keys: KeyState::new(config.hold),
            clock: config.clock,
            started: Instant::now(),
            virtual_ms: 0,
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
    if !config.sound {
        args.push("-nosound".into());
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

#[no_mangle]
extern "C" fn rs_doom_sleep_ms(ms: u32) {
    let real = with_host(|host| match host.clock {
        Clock::Real => true,
        Clock::Virtual => {
            host.virtual_ms += u64::from(ms.max(1));
            false
        }
    });
    if real != Some(false) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
}

#[no_mangle]
extern "C" fn rs_doom_ticks_ms() -> u32 {
    with_host(|host| host.now_ms() as u32).unwrap_or(0)
}

#[no_mangle]
extern "C" fn rs_doom_get_key(pressed: *mut c_int, key: *mut u8) -> c_int {
    let event = with_host(|host| {
        let now = host.now_ms();
        while let Ok(input) = host.input.try_recv() {
            host.keys.input(input, now);
        }
        host.keys.next_event(now)
    })
    .flatten();
    match event {
        Some((down, k)) if !pressed.is_null() && !key.is_null() => {
            // SAFETY: the engine passes two valid out-pointers.
            unsafe {
                *pressed = c_int::from(down);
                *key = k;
            }
            1
        }
        _ => 0,
    }
}

#[no_mangle]
extern "C" fn rs_doom_set_title(title: *const c_char) {
    if title.is_null() {
        return;
    }
    // SAFETY: a NUL-terminated string from the engine.
    let title = unsafe { CStr::from_ptr(title) }.to_string_lossy().into_owned();
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
        // Before the engine has a host: nowhere better.
        eprint!("{text}");
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
    // The engine cannot go on after exit(); the process ends from the
    // terminal side, which restores the terminal first.
    loop {
        std::thread::park();
    }
}
