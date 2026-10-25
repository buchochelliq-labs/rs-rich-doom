//! Sound through a real output device, with no sound card: ALSA's `file`
//! plugin over its `null` device, defined in a `.asoundrc` of a temporary
//! `HOME`, so every sample the device takes is also written to a file. The
//! file is a FIFO this test reads at the sample rate: the null device
//! alone takes samples as fast as the CPU allows.
//! The effect goes through this crate's mixer and intuiTUIve's audio
//! output (cpal, ALSA) like the game's; the file says what the device got:
//! how many samples, and silence while muted. Linux with the `sound`
//! feature only. One output per process, so one test in this file.

#![cfg(all(target_os = "linux", feature = "sound"))]

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rs_rich_doom::sound::{self, Audio, Device};

const RATE: u32 = 44_100;

/// Wait until `done`, at most five seconds.
fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// What the device took, read from the FIFO at the rate a sound card
/// would take it.
#[derive(Clone)]
struct Tap(Arc<Mutex<Vec<f32>>>);

impl Tap {
    fn read(fifo: std::path::PathBuf) -> Tap {
        let tap = Tap(Arc::default());
        let samples = tap.0.clone();
        std::thread::spawn(move || {
            // Blocks until ALSA opens it to write.
            let mut file = std::fs::File::open(&fifo).expect("the FIFO");
            let started = Instant::now();
            let (mut total, mut buffer, mut partial) = (0usize, [0u8; 4096], Vec::new());
            loop {
                let n = match file.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                partial.extend_from_slice(&buffer[..n]);
                let whole = partial.len() / 4 * 4;
                let floats = partial[..whole]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b));
                samples.lock().unwrap().extend(floats);
                partial.drain(..whole);
                total += n;
                // A stereo f32 frame is 8 bytes.
                let due = Duration::from_secs_f64(total as f64 / 8.0 / f64::from(RATE));
                if let Some(ahead) = due.checked_sub(started.elapsed()) {
                    std::thread::sleep(ahead);
                }
            }
        });
        tap
    }

    /// The samples the device has taken so far.
    fn samples(&self) -> Vec<f32> {
        self.0.lock().unwrap().clone()
    }
}

/// Left-channel samples that are not silence.
fn sounding(samples: &[f32]) -> usize {
    samples.iter().step_by(2).filter(|s| **s != 0.0).count()
}

#[test]
fn effects_reach_the_device_and_mute_silences_them() {
    let home = std::env::temp_dir().join(format!("rs-rich-doom-alsa-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let fifo = home.join("tap.raw");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    assert!(made.is_ok_and(|s| s.success()), "mkfifo");
    std::fs::write(
        home.join(".asoundrc"),
        format!(
            "pcm.rsdoomtap {{\n    type file\n    slave.pcm \"null\"\n    file \"{}\"\n    \
             format \"raw\"\n    hint {{ show on description \"rs-rich-doom test tap\" }}\n}}\n",
            fifo.display()
        ),
    )
    .unwrap();
    // ALSA reads ~/.asoundrc the first time it is used, which is below.
    // SAFETY: the only test in this process, before any other thread.
    unsafe { std::env::set_var("HOME", &home) };

    let tap = Tap::read(fifo);
    sound::configure(Audio {
        sfx: true,
        music: false,
        music_gain: 1.0,
        headless: false,
    });
    sound::choose_device(Device {
        name: Some("rsdoomtap".into()),
        rate: Some(RATE),
    });
    let Some(playing) = sound::start() else {
        panic!("no output: {:?}", sound::note());
    };
    eprintln!("opened {playing:?}");
    assert_eq!(playing.device, "rs-rich-doom test tap");
    assert_eq!((playing.rate, playing.channels), (RATE, 2));
    wait_for("the stream", || !tap.samples().is_empty());
    let float_format = tap
        .samples()
        .iter()
        .all(|s| s.is_finite() && s.abs() <= 1.0);
    assert!(float_format, "the stream's samples are f32");

    // A tenth of a second of a square wave at 11025 Hz, full volume, in
    // the middle: 1102 samples, 4408 frames at 44100 Hz, none silent.
    // The output fills any gap in what was mixed with silence: the
    // effect's frames are the frames that are not silent.
    let effect: Arc<[u8]> = (0..1102).map(|i| if i % 8 < 4 { 255 } else { 0 }).collect();
    let frames = 1102 * (RATE as usize / 11025);
    let play = |name: &str, sounded: usize| {
        let from = tap.samples().len();
        sound::play_effect(0, effect.clone(), 11025, 127, 128);
        wait_for(name, || !sound::effect_playing(0));
        // Mixed; then taken by the device: what was queued, and more.
        wait_for(name, || sounding(&tap.samples()) >= sounded);
        std::thread::sleep(Duration::from_millis(300));
        from
    };

    play("the first effect", frames);
    let first = sounding(&tap.samples());
    assert_eq!(first, frames, "every frame of the effect, once");

    sound::set_muted(true);
    let muted_from = play("the muted effect", 0);
    let after_mute = tap.samples();
    assert_eq!(sounding(&after_mute), first, "nothing sounded while muted");
    assert!(
        after_mute.len() - muted_from >= 2 * frames,
        "the device went on"
    );

    sound::set_muted(false);
    sound::set_volume(0.5);
    let last_from = play("the last effect", 2 * frames);
    let all = tap.samples();
    assert_eq!(
        sounding(&all),
        2 * frames,
        "the last effect, after unmuting"
    );
    // At half volume: the last effect's peak is half the first's.
    let peak = |samples: &[f32]| samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let (early, late) = (peak(&all[..muted_from]), peak(&all[last_from..]));
    assert!((late - early / 2.0).abs() < 1e-3, "{early} then {late}");

    let written = sound::playing().unwrap().frames;
    eprintln!(
        "the device took {} frames ({:.2} s); the mixer wrote {written}; \
         {} frames sounded: {first} before the mute, 0 while muted, {frames} after \
         (peak {early:.3}, then {late:.3} at half volume)",
        all.len() / 2,
        all.len() as f64 / 2.0 / f64::from(RATE),
        sounding(&all),
    );
    std::fs::remove_dir_all(&home).ok();
}
