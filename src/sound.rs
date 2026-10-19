//! Sound effects and music through cpal.
//!
//! Effects: the engine's sound module (csrc/sound.c) hands each effect to
//! this mixer. They are 8-bit unsigned PCM at their own rate (usually
//! 11025 Hz), resampled here to the device's rate and panned by Doom's
//! stereo separation.
//!
//! Music: the engine's music module (csrc/music.c, Chocolate Doom's OPL
//! module in opl/) plays MUS through an emulated OPL3 FM chip with the
//! IWAD's GENMIDI instruments, as Doom did on a Sound Blaster. The audio
//! callback here pulls its samples at the device's rate
//! ([`render_music`]) and adds them to the effects.

#[cfg(feature = "sound")]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[cfg(feature = "sound")]
static SFX: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "sound")]
static MUSIC: AtomicBool = AtomicBool::new(false);
/// The music's gain, as `f32` bits.
#[cfg(feature = "sound")]
static MUSIC_GAIN: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// Music without a device, at this rate (0: through the device).
#[cfg(feature = "sound")]
static HEADLESS_RATE: AtomicU32 = AtomicU32::new(0);

/// The rate music is made at when there is no device ([`Audio::headless`]).
pub const HEADLESS_RATE_HZ: u32 = 44_100;

/// What the engine may play.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Audio {
    /// Sound effects (`--no-sound` turns them off).
    pub sfx: bool,
    /// Music (`--no-music` turns it off).
    pub music: bool,
    /// The music's loudness over the game's own music volume, 1.0 as is
    /// (`--music-volume`, in percent).
    pub music_gain: f32,
    /// No device: music is made only when [`render_music`] asks for it, at
    /// [`HEADLESS_RATE_HZ`], and effects are not played. For tests.
    pub headless: bool,
}

/// Set what the engine may play, before it starts.
pub fn configure(audio: Audio) {
    #[cfg(feature = "sound")]
    {
        SFX.store(audio.sfx && !audio.headless, Ordering::SeqCst);
        MUSIC.store(audio.music, Ordering::SeqCst);
        MUSIC_GAIN.store(audio.music_gain.max(0.0).to_bits(), Ordering::SeqCst);
        let rate = if audio.headless { HEADLESS_RATE_HZ } else { 0 };
        HEADLESS_RATE.store(rate, Ordering::SeqCst);
    }
    #[cfg(not(feature = "sound"))]
    let _ = audio;
}

/// Whether this build plays sound at all.
pub const AVAILABLE: bool = cfg!(feature = "sound");

/// Why sound or music that was asked for is not playing (no output
/// device, say), for a note after the game ends.
pub fn note() -> Option<String> {
    #[cfg(feature = "sound")]
    return device::note();
    #[cfg(not(feature = "sound"))]
    None
}

/// Make `out.len() / 2` stereo frames of music (interleaved, 16-bit) at
/// the device's rate, or [`HEADLESS_RATE_HZ`] when headless, advancing the
/// song by that much. False, and silence, when no music is playing (or
/// this build has no sound).
pub fn render_music(out: &mut [i16]) -> bool {
    #[cfg(feature = "sound")]
    {
        extern "C" {
            fn rs_doom_music_render(out: *mut i16, frames: std::ffi::c_uint) -> std::ffi::c_int;
        }
        let frames = out.len() / 2;
        // SAFETY: `out` holds `frames` stereo frames; the C side takes its
        // own lock against the engine thread.
        let ran = unsafe { rs_doom_music_render(out.as_mut_ptr(), frames as std::ffi::c_uint) };
        out[frames * 2..].fill(0);
        ran != 0
    }
    #[cfg(not(feature = "sound"))]
    {
        out.fill(0);
        false
    }
}

/// Add stereo 16-bit music to `out` (interleaved frames of `channels`
/// samples) at `gain`: both sides averaged for mono, the first two
/// channels of more. Frames past the music's are left alone.
pub fn add_music(out: &mut [f32], music: &[i16], channels: usize, gain: f32) {
    let channels = channels.max(1);
    let scale = gain / 32768.0;
    for (frame, lr) in out.chunks_mut(channels).zip(music.as_chunks::<2>().0) {
        let (l, r) = (lr[0] as f32 * scale, lr[1] as f32 * scale);
        match frame {
            [mono] => *mono += (l + r) * 0.5,
            [left, right, ..] => {
                *left += l;
                *right += r;
            }
            [] => {}
        }
    }
}

/// Hard-clip samples to -1..1.
pub fn clip(out: &mut [f32]) {
    out.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));
}

/// A playing effect.
#[derive(Clone, Debug)]
pub struct Voice {
    samples: std::sync::Arc<[u8]>,
    /// Where it is, in source samples.
    position: f64,
    /// Source samples per output frame.
    step: f64,
    left: f32,
    right: f32,
}

/// Doom's volume (0..=127) and separation (0 left ..= 254 right) as gains.
pub fn gains(vol: i32, sep: i32) -> (f32, f32) {
    let vol = vol.clamp(0, 127) as f32 / 127.0;
    let sep = sep.clamp(0, 254) as f32;
    // Chocolate Doom's pan: full in one ear at the edges, both at 128.
    let left = ((254.0 - sep) / 127.0).min(1.0) * vol;
    let right = (sep / 127.0).min(1.0) * vol;
    (left, right)
}

/// Mixes the channels Doom plays effects on.
#[derive(Debug, Default)]
pub struct Mixer {
    voices: Vec<Option<Voice>>,
    rate: u32,
}

impl Mixer {
    pub fn new(rate: u32) -> Mixer {
        Mixer {
            voices: Vec::new(),
            rate: rate.max(1),
        }
    }

    pub fn start(
        &mut self,
        channel: usize,
        samples: std::sync::Arc<[u8]>,
        rate: u32,
        vol: i32,
        sep: i32,
    ) {
        if self.voices.len() <= channel {
            self.voices.resize(channel + 1, None);
        }
        let (left, right) = gains(vol, sep);
        self.voices[channel] = Some(Voice {
            samples,
            position: 0.0,
            step: rate.max(1) as f64 / self.rate as f64,
            left,
            right,
        });
    }

    pub fn params(&mut self, channel: usize, vol: i32, sep: i32) {
        if let Some(Some(voice)) = self.voices.get_mut(channel) {
            (voice.left, voice.right) = gains(vol, sep);
        }
    }

    pub fn stop(&mut self, channel: usize) {
        if let Some(slot) = self.voices.get_mut(channel) {
            *slot = None;
        }
    }

    pub fn playing(&self, channel: usize) -> bool {
        matches!(self.voices.get(channel), Some(Some(_)))
    }

    /// Mix into `out`, interleaved frames of `channels` samples in -1..1.
    pub fn mix(&mut self, out: &mut [f32], channels: usize) {
        self.mix_unclipped(out, channels);
        clip(out);
    }

    /// [`Mixer::mix`] without the clipping, for more to be added.
    pub fn mix_unclipped(&mut self, out: &mut [f32], channels: usize) {
        out.iter_mut().for_each(|s| *s = 0.0);
        let channels = channels.max(1);
        for slot in &mut self.voices {
            let Some(voice) = slot else { continue };
            for frame in out.chunks_mut(channels) {
                let index = voice.position as usize;
                let Some(&byte) = voice.samples.get(index) else {
                    *slot = None;
                    break;
                };
                let sample = (byte as f32 - 128.0) / 128.0 * 0.5;
                match frame {
                    [mono] => *mono += sample * (voice.left + voice.right) * 0.5,
                    [l, r, ..] => {
                        *l += sample * voice.left;
                        *r += sample * voice.right;
                    }
                    [] => {}
                }
                voice.position += voice.step;
            }
        }
    }
}

#[cfg(feature = "sound")]
mod device {
    use std::collections::HashMap;
    use std::ffi::{c_int, c_uint};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, OnceLock};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::{add_music, clip, render_music, Mixer, HEADLESS_RATE, MUSIC, MUSIC_GAIN, SFX};

    struct Output {
        mixer: Arc<Mutex<Mixer>>,
        cache: Mutex<HashMap<c_int, Arc<[u8]>>>,
        rate: u32,
    }

    static OUTPUT: OnceLock<Option<Output>> = OnceLock::new();
    static NOTE: Mutex<Option<String>> = Mutex::new(None);

    pub(super) fn note() -> Option<String> {
        NOTE.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    extern "C" {
        fn rs_doom_quiet_alsa();
    }

    fn open() -> Option<Output> {
        let sfx = SFX.load(Ordering::SeqCst);
        let music = MUSIC.load(Ordering::SeqCst);
        if !(sfx || music) || HEADLESS_RATE.load(Ordering::SeqCst) != 0 {
            return None;
        }
        let what = match (sfx, music) {
            (true, true) => "sound and music are off",
            (true, false) => "sound is off",
            _ => "music is off",
        };
        match open_device(music) {
            Ok(output) => Some(output),
            Err(why) => {
                *NOTE.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{why}: {what}"));
                None
            }
        }
    }

    fn open_device(music: bool) -> Result<Output, String> {
        // SAFETY: installs a no-op ALSA error handler (nothing on other
        // platforms), so ALSA's messages stay off the game's screen.
        unsafe { rs_doom_quiet_alsa() };
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no audio output device".to_string())?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("no audio output ({e})"))?;
        let config = supported.config();
        let channels = (config.channels as usize).max(1);
        let rate = config.sample_rate;
        let mixer = Arc::new(Mutex::new(Mixer::new(rate)));
        let shared = mixer.clone();
        let mut scratch = Vec::new();
        let mut tune = Vec::new();
        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |out: &mut [f32], _| {
                    scratch.resize(out.len(), 0.0);
                    match shared.lock() {
                        Ok(mut mixer) => mixer.mix_unclipped(&mut scratch, channels),
                        Err(_) => scratch.fill(0.0),
                    }
                    // The music's lock is taken after the effects' is let
                    // go: the two are never held together.
                    if music {
                        tune.resize(out.len() / channels * 2, 0);
                        if render_music(&mut tune) {
                            let gain = f32::from_bits(MUSIC_GAIN.load(Ordering::Relaxed));
                            add_music(&mut scratch, &tune, channels, gain);
                        }
                    }
                    clip(&mut scratch);
                    out.copy_from_slice(&scratch);
                },
                |_| {},
                None,
            )
            .map_err(|e| format!("no audio output ({e})"))?;
        stream
            .play()
            .map_err(|e| format!("no audio output ({e})"))?;
        // The stream plays for the rest of the process.
        std::mem::forget(stream);
        Ok(Output {
            mixer,
            cache: Mutex::new(HashMap::new()),
            rate,
        })
    }

    fn output() -> Option<&'static Output> {
        OUTPUT.get_or_init(open).as_ref()
    }

    #[no_mangle]
    extern "C" fn rs_doom_sound_init() -> c_int {
        c_int::from(SFX.load(Ordering::SeqCst) && output().is_some())
    }

    /// The rate the music module synthesizes at: the device's, or the
    /// headless rate; 0 for no music.
    #[no_mangle]
    extern "C" fn rs_doom_music_rate() -> c_uint {
        if !MUSIC.load(Ordering::SeqCst) {
            return 0;
        }
        match HEADLESS_RATE.load(Ordering::SeqCst) {
            0 => output().map_or(0, |out| out.rate),
            rate => rate,
        }
    }

    #[no_mangle]
    extern "C" fn rs_doom_sound_shutdown() {
        if let Some(out) = output() {
            if let Ok(mut mixer) = out.mixer.lock() {
                *mixer = Mixer::new(mixer.rate);
            }
        }
    }

    #[no_mangle]
    extern "C" fn rs_doom_sound_start(
        channel: c_int,
        lump: c_int,
        samples: *const u8,
        len: usize,
        rate: c_int,
        vol: c_int,
        sep: c_int,
    ) {
        let Some(out) = output() else { return };
        if samples.is_null() || channel < 0 {
            return;
        }
        let data = {
            let mut cache = out.cache.lock().unwrap_or_else(|e| e.into_inner());
            cache
                .entry(lump)
                .or_insert_with(|| {
                    // SAFETY: `len` bytes of the lump the engine cached.
                    Arc::from(unsafe { std::slice::from_raw_parts(samples, len) })
                })
                .clone()
        };
        if let Ok(mut mixer) = out.mixer.lock() {
            mixer.start(channel as usize, data, rate.max(1) as u32, vol, sep);
        }
    }

    #[no_mangle]
    extern "C" fn rs_doom_sound_params(channel: c_int, vol: c_int, sep: c_int) {
        if let (Some(out), Ok(channel)) = (output(), usize::try_from(channel)) {
            if let Ok(mut mixer) = out.mixer.lock() {
                mixer.params(channel, vol, sep);
            }
        }
    }

    #[no_mangle]
    extern "C" fn rs_doom_sound_stop(channel: c_int) {
        if let (Some(out), Ok(channel)) = (output(), usize::try_from(channel)) {
            if let Ok(mut mixer) = out.mixer.lock() {
                mixer.stop(channel);
            }
        }
    }

    #[no_mangle]
    extern "C" fn rs_doom_sound_playing(channel: c_int) -> c_int {
        let playing = match (output(), usize::try_from(channel)) {
            (Some(out), Ok(channel)) => out
                .mixer
                .lock()
                .map(|m| m.playing(channel))
                .unwrap_or(false),
            _ => false,
        };
        c_int::from(playing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pans_like_doom() {
        assert_eq!(gains(127, 128), (126.0 / 127.0, 1.0));
        assert_eq!(gains(127, 0), (1.0, 0.0));
        assert_eq!(gains(127, 254), (0.0, 1.0));
        assert_eq!(gains(0, 128), (0.0, 0.0));
    }

    #[test]
    fn mixes_resamples_and_ends() {
        let mut mixer = Mixer::new(22050);
        // Four samples at 11025 Hz: eight output frames.
        let samples: std::sync::Arc<[u8]> = vec![255u8, 255, 0, 0].into();
        mixer.start(2, samples, 11025, 127, 254);
        assert!(mixer.playing(2));
        let mut out = vec![0.0f32; 20];
        mixer.mix(&mut out, 2);
        // Right ear only; each source sample twice.
        assert_eq!(out[0], 0.0);
        assert!([1, 3, 5, 7].iter().all(|&i| out[i] > 0.4));
        assert!([9, 11, 13, 15].iter().all(|&i| out[i] < -0.4));
        assert!(!mixer.playing(2), "ended after its samples");
        mixer.stop(9);
    }

    #[test]
    fn music_is_added_and_clipped() {
        // Stereo: left and right in place, at the gain, over the effects.
        let mut out = vec![0.25f32, -0.25, 0.9, -0.9];
        add_music(&mut out, &[16384, -16384, 16384, -16384], 2, 1.0);
        assert_eq!(&out[..2], &[0.75, -0.75]);
        clip(&mut out);
        assert_eq!(&out[2..], &[1.0, -1.0], "clipped");
        // Mono: the two sides averaged; half gain.
        let mut mono = vec![0.0f32; 2];
        add_music(&mut mono, &[16384, 0, -32768, -32768], 1, 0.5);
        assert_eq!(mono, [0.125, -0.5]);
        // More channels: the first two only; music shorter than the buffer.
        let mut surround = vec![0.0f32; 8];
        add_music(&mut surround, &[32767, 32767], 4, 1.0);
        assert!(surround[0] > 0.99 && surround[1] > 0.99);
        assert!(surround[2..].iter().all(|&s| s == 0.0));
        // Effects past full scale are clipped only once the music is in.
        let mut mixer = Mixer::new(11025);
        let loud: std::sync::Arc<[u8]> = vec![255u8; 4].into();
        mixer.start(0, loud.clone(), 11025, 127, 128);
        mixer.start(1, loud.clone(), 11025, 127, 128);
        mixer.start(2, loud, 11025, 127, 128);
        let mut out = vec![0.0f32; 2];
        mixer.mix_unclipped(&mut out, 2);
        assert!(out[1] > 1.0, "{out:?}");
        add_music(&mut out, &[-32768, -32768], 2, 1.0);
        clip(&mut out);
        assert!(out[1] > 0.4 && out[1] < 1.0, "{out:?}");
    }

    /// The OPL music module without the engine: a synthetic GENMIDI lump
    /// and MUS song through Chocolate Doom's code and Nuked OPL3.
    #[cfg(feature = "sound")]
    mod music {
        use super::super::*;

        extern "C" {
            fn rs_doom_music_test_start(
                genmidi: *const u8,
                song: *const u8,
                len: std::ffi::c_int,
                volume: std::ffi::c_int,
            ) -> std::ffi::c_int;
            fn rs_doom_music_test_stop();
            fn rs_doom_mus2mid(
                mus: *const u8,
                len: usize,
                out: *mut u8,
                cap: usize,
            ) -> std::ffi::c_long;
        }

        /// The C side is one global module: one test at a time.
        static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

        /// A GENMIDI lump (175 instruments, then 175 names) whose
        /// instrument 0 is a pure sine: the modulator silent, the carrier
        /// added, attacking at once and released fast, a note offset of
        /// -12. `attack` 0 makes it
        /// never sound.
        fn genmidi(attack: u8) -> Vec<u8> {
            let mut lump = b"#OPL_II#".to_vec();
            for instrument in 0..175 {
                let mut instr = [0u8; 36];
                if instrument == 0 {
                    // flags, fine tuning, fixed note; then voice 0:
                    // modulator (tremolo, attack, sustain, waveform,
                    // scale, level), feedback, carrier, unused, offset.
                    instr[2] = 128;
                    instr[4..10].copy_from_slice(&[0x21, 0xf0, 0x0f, 0, 0, 0x3f]);
                    instr[10] = 0x01;
                    instr[11..17].copy_from_slice(&[0x21, attack, 0x0f, 0, 0, 0]);
                    // DMX's frequency table runs an octave high: patches
                    // take it back down with their base note offset.
                    instr[18..20].copy_from_slice(&(-12i16).to_le_bytes());
                }
                lump.extend_from_slice(&instr);
            }
            for _ in 0..175 {
                lump.extend_from_slice(&[0u8; 32]);
            }
            assert_eq!(lump.len(), 11908, "Doom's GENMIDI size");
            lump
        }

        /// A MUS song: instrument 0 on channel 0, A4 (note 69) at full
        /// volume for one second (140 MUS ticks), released, then a second
        /// of nothing, then the end.
        fn mus() -> Vec<u8> {
            let score: &[u8] = &[
                0x40, 0x00, 0x00, // controller 0 (instrument) = 0
                0x90, 0xc5, 0x7f, // play note 69, volume 127; then a delay
                0x81, 0x0c, // 140 ticks
                0x80, 0x45, // release note 69; then a delay
                0x81, 0x0c, // 140 ticks
                0x60, // end of score
            ];
            let mut song = b"MUS\x1a".to_vec();
            let start = 16u16;
            // Score length and start, primary and secondary channels, the
            // number of instruments; then the instrument list: 0.
            for field in [score.len() as u16, start, 1, 0, 1, 0] {
                song.extend_from_slice(&field.to_le_bytes());
            }
            assert_eq!(song.len(), start as usize);
            song.extend_from_slice(score);
            song
        }

        fn play(genmidi: &[u8], seconds: f64) -> Option<Vec<i16>> {
            configure(Audio {
                sfx: false,
                music: true,
                music_gain: 1.0,
                headless: true,
            });
            let song = mus();
            // SAFETY: both buffers outlive the song (stopped below).
            let started = unsafe {
                rs_doom_music_test_start(genmidi.as_ptr(), song.as_ptr(), song.len() as _, 127)
            };
            if started == 0 {
                return None;
            }
            let mut out = vec![0i16; (HEADLESS_RATE_HZ as f64 * seconds) as usize * 2];
            // In 10 ms blocks, as an audio callback asks.
            for block in out.chunks_mut(HEADLESS_RATE_HZ as usize / 100 * 2) {
                assert!(render_music(block));
            }
            // SAFETY: the song started above.
            unsafe { rs_doom_music_test_stop() };
            assert!(!render_music(&mut [1, 1]), "stopped");
            Some(out)
        }

        fn rms(samples: &[i16]) -> f64 {
            let energy: f64 = samples.iter().map(|&s| f64::from(s).powi(2)).sum();
            (energy / samples.len().max(1) as f64).sqrt()
        }

        #[test]
        fn mus_converts_to_midi() {
            let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
            let song = mus();
            let mut midi = vec![0u8; 4096];
            // SAFETY: input and output buffers of the lengths given.
            let len = unsafe {
                rs_doom_mus2mid(song.as_ptr(), song.len(), midi.as_mut_ptr(), midi.len())
            };
            assert!(len > 22, "{len}");
            let midi = &midi[..len as usize];
            assert_eq!(&midi[..4], b"MThd");
            assert_eq!(&midi[14..18], b"MTrk");
            // Program change 0 and note on 69 at velocity 127, channel 0;
            // the release as a note off (or note on at 0).
            let has = |needle: &[u8]| midi.windows(needle.len()).any(|w| w == needle);
            assert!(has(&[0xc0, 0x00]), "{midi:02x?}");
            assert!(has(&[0x90, 69, 127]), "{midi:02x?}");
            assert!(has(&[0x80, 69]) || has(&[69, 0]), "{midi:02x?}");
            // The end of track.
            assert_eq!(&midi[midi.len() - 3..], &[0xff, 0x2f, 0x00]);
            // Not a MUS: refused.
            let bad = b"NOPE and more bytes";
            let len =
                unsafe { rs_doom_mus2mid(bad.as_ptr(), bad.len(), midi.as_ptr() as *mut u8, 0) };
            assert_eq!(len, -1);
        }

        #[test]
        fn a_genmidi_patch_plays_a_note_through_the_opl() {
            let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
            let rate = HEADLESS_RATE_HZ as usize;
            let out = play(&genmidi(0xf0), 2.0).expect("the music module started");
            let (left, right): (Vec<i16>, Vec<i16>) =
                out.as_chunks::<2>().0.iter().map(|&[l, r]| (l, r)).unzip();
            // Sounding for the first second (skip the attack), in both ears.
            let held = &left[rate / 10..rate * 9 / 10];
            assert!(rms(held) > 500.0, "rms {}", rms(held));
            assert!(rms(&right[rate / 10..rate * 9 / 10]) > 500.0);
            // At A4's pitch: zero crossings over 0.8 s.
            let crossings = held.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
            let hz = crossings as f64 / 2.0 / 0.8;
            assert!((hz - 440.0).abs() < 440.0 * 0.03, "{hz} Hz");
            // Released: quiet in the second second.
            let after = &left[rate * 5 / 4..rate * 2];
            assert!(rms(after) < 20.0, "rms after release {}", rms(after));

            // The same song with an instrument that never attacks: the
            // patch really comes from the lump.
            let silent = play(&genmidi(0x00), 1.0).expect("started");
            // (A fully attenuated OPL operator still leaves a few LSBs.)
            let loudest = silent.iter().map(|s| s.unsigned_abs()).max();
            assert!(loudest < Some(64), "not silent: {loudest:?}");
            assert!(rms(&silent) < 20.0, "rms {}", rms(&silent));

            // With music off the module has no rate, so no chip.
            configure(Audio {
                sfx: false,
                music: false,
                music_gain: 1.0,
                headless: true,
            });
            let lump = genmidi(0xf0);
            let song = mus();
            // SAFETY: as in `play`; music is off, so the start fails.
            let started = unsafe {
                rs_doom_music_test_start(lump.as_ptr(), song.as_ptr(), song.len() as _, 127)
            };
            assert_eq!(started, 0, "started with music off");
        }
    }
}
