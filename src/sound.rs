//! Soft interface clicks, as tvOS plays on focus changes, selection and Back.
//!
//! The clicks are synthesized once at startup (short, filtered sine and noise bursts), so no
//! audio files ship with Luma. Playback goes through an SDL audio queue: SDL mixes it on its own
//! audio thread, while input and rendering stay on the SDL thread. The device opens on the first
//! click and closes in the background; without an audio device the clicks are silently skipped.
use crate::model::Preferences;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    /// Focus moved to another element.
    Focus,
    /// OK activated the focused element.
    Select,
    /// Back navigated up or dismissed something.
    Back,
}

/// Whether a click should sound with these preferences (Settings › General › Sounds).
pub fn enabled(preferences: &Preferences) -> bool {
    preferences.sounds
}

pub struct Sounds {
    audio: Option<sdl2::AudioSubsystem>,
    /// Opened on the first click and closed while Luma is in the background, so Luma never
    /// holds the TV's audio output while another app plays.
    queue: Option<sdl2::audio::AudioQueue<f32>>,
    /// Opening failed: no device (common on TVs whose audio SDL cannot reach). Stay silent.
    unavailable: bool,
    clips: Option<[Vec<f32>; 3]>,
}

impl Sounds {
    pub fn new(sdl: &sdl2::Sdl) -> Self {
        let audio = sdl
            .audio()
            .map_err(|error| eprintln!("Luma sounds unavailable: {error}"))
            .ok();
        Self {
            unavailable: audio.is_none(),
            audio,
            queue: None,
            clips: None,
        }
    }

    fn open(&mut self) -> Option<&sdl2::audio::AudioQueue<f32>> {
        if self.queue.is_none() && !self.unavailable {
            let desired = sdl2::audio::AudioSpecDesired {
                freq: Some(48_000),
                channels: Some(1),
                samples: Some(512),
            };
            match self.audio.as_ref()?.open_queue::<f32, _>(None, &desired) {
                Ok(queue) => {
                    let rate = queue.spec().freq;
                    self.clips.get_or_insert_with(|| {
                        [Click::Focus, Click::Select, Click::Back]
                            .map(|click| synthesize(click, rate))
                    });
                    queue.resume();
                    self.queue = Some(queue);
                }
                Err(error) => {
                    eprintln!("Luma sounds unavailable: {error}");
                    self.unavailable = true;
                }
            }
        }
        self.queue.as_ref()
    }

    /// Plays a click, replacing any click still sounding so rapid focus moves never pile up.
    pub fn play(&mut self, click: Click, preferences: &Preferences) {
        if !enabled(preferences) {
            return;
        }
        if self.open().is_none() {
            return;
        }
        if let (Some(queue), Some(clips)) = (&self.queue, &self.clips) {
            queue.clear();
            let _ = queue.queue_audio(&clips[click as usize]);
        }
    }

    /// Closes the audio device while Luma is hidden or unfocused; the next click reopens it.
    pub fn release(&mut self) {
        self.queue = None;
    }
}

/// A short click: a decaying tone (sweeping for Back) with a little filtered noise at its start.
pub fn synthesize(click: Click, rate: i32) -> Vec<f32> {
    let rate = rate.max(8_000) as f32;
    // (length in seconds, start and end frequency, decay time constant, loudness)
    let (length, from, to, decay, gain) = match click {
        Click::Focus => (0.012, 2_600., 2_600., 0.0025, 0.12),
        Click::Select => (0.028, 1_150., 1_150., 0.006, 0.16),
        Click::Back => (0.024, 1_400., 820., 0.006, 0.14),
    };
    let samples = (length * rate) as usize;
    let mut phase = 0f32;
    let mut noise_state = 0x2545_f491_u32;
    let mut noise = 0f32;
    (0..samples)
        .map(|index| {
            let t = index as f32 / rate;
            let progress = t / length;
            let frequency = from + (to - from) * progress;
            phase += std::f32::consts::TAU * frequency / rate;
            // Deterministic white noise through a one-pole low-pass filter.
            noise_state = noise_state
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let white = (noise_state >> 8) as f32 / (1u32 << 24) as f32 * 2. - 1.;
            noise += (white - noise) * 0.35;
            let envelope = (-t / decay).exp();
            // A 1 ms fade-in and a fade-out to the end avoid clicks of their own.
            let fade = (t / 0.001).min(1.) * (1. - progress).min(0.1) * 10.;
            let body = phase.sin() + 0.35 * (2. * phase).sin();
            let transient = noise * (-t / 0.0015).exp();
            gain * fade * envelope * (0.8 * body + 0.5 * transient)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_are_short_quiet_and_silent_at_both_ends() {
        for click in [Click::Focus, Click::Select, Click::Back] {
            let samples = synthesize(click, 48_000);
            assert!(!samples.is_empty() && samples.len() < 48_000 / 20);
            assert!(samples.iter().all(|sample| sample.abs() <= 0.3));
            assert!(samples[0].abs() < 0.01);
            assert!(samples.last().unwrap().abs() < 0.01);
        }
    }

    #[test]
    fn sounds_default_on_and_are_inert_when_switched_off() {
        let mut preferences = Preferences::default();
        assert!(enabled(&preferences));
        preferences.sounds = false;
        assert!(!enabled(&preferences));
        // Without an audio device (as in tests) playing is a silent no-op either way, and a
        // switched-off click never opens a device.
        let mut sounds = Sounds {
            audio: None,
            queue: None,
            unavailable: false,
            clips: None,
        };
        sounds.play(Click::Focus, &preferences);
        assert!(sounds.queue.is_none() && !sounds.unavailable);
        preferences.sounds = true;
        sounds.play(Click::Select, &preferences);
        assert!(sounds.queue.is_none());
    }
}
