//! Sound effects: the engine's sound module (csrc/sound.c) hands each effect
//! to this mixer, which plays it through cpal. Effects are 8-bit unsigned
//! PCM at their own rate (usually 11025 Hz), resampled here to the device's
//! rate and panned by Doom's stereo separation. Music is not played.

#[cfg(feature = "sound")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "sound")]
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether the engine may open the sound device (`--no-sound` says no).
pub fn configure(enabled: bool) {
    #[cfg(feature = "sound")]
    ENABLED.store(enabled, Ordering::SeqCst);
    #[cfg(not(feature = "sound"))]
    let _ = enabled;
}

/// Whether this build plays sound at all.
pub const AVAILABLE: bool = cfg!(feature = "sound");

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

    pub fn start(&mut self, channel: usize, samples: std::sync::Arc<[u8]>, rate: u32, vol: i32, sep: i32) {
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
        out.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));
    }
}

#[cfg(feature = "sound")]
mod device {
    use std::collections::HashMap;
    use std::ffi::c_int;
    use std::sync::{Arc, Mutex, OnceLock};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::{Mixer, ENABLED};

    struct Output {
        mixer: Arc<Mutex<Mixer>>,
        cache: Mutex<HashMap<c_int, Arc<[u8]>>>,
    }

    static OUTPUT: OnceLock<Option<Output>> = OnceLock::new();

    extern "C" {
        fn rs_doom_quiet_alsa();
    }

    fn open() -> Option<Output> {
        if !ENABLED.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        // SAFETY: installs a no-op ALSA error handler (nothing on other
        // platforms), so ALSA's messages stay off the game's screen.
        unsafe { rs_doom_quiet_alsa() };
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let config = supported.config();
        let channels = config.channels as usize;
        let mixer = Arc::new(Mutex::new(Mixer::new(config.sample_rate)));
        let shared = mixer.clone();
        let mut scratch = Vec::new();
        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |out: &mut [f32], _| {
                    scratch.resize(out.len(), 0.0);
                    if let Ok(mut mixer) = shared.lock() {
                        mixer.mix(&mut scratch, channels);
                    }
                    out.copy_from_slice(&scratch);
                },
                |_| {},
                None,
            )
            .ok()?;
        stream.play().ok()?;
        // The stream plays for the rest of the process.
        std::mem::forget(stream);
        Some(Output {
            mixer,
            cache: Mutex::new(HashMap::new()),
        })
    }

    fn output() -> Option<&'static Output> {
        OUTPUT.get_or_init(open).as_ref()
    }

    #[no_mangle]
    extern "C" fn rs_doom_sound_init() -> c_int {
        c_int::from(output().is_some())
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
            (Some(out), Ok(channel)) => out.mixer.lock().map(|m| m.playing(channel)).unwrap_or(false),
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
}
