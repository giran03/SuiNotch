//! Soft chimes synthesized at startup (no audio files shipped) and played from memory.

use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

const RATE: u32 = 44_100;

#[derive(Clone, Copy)]
pub enum Chime {
    /// Two rising notes: focus done, reminders.
    Bell,
    /// A single round drop: water.
    Drop,
    /// Gentle descending pair: break over.
    Soft,
}

static BELL: OnceLock<Vec<u8>> = OnceLock::new();
static DROP: OnceLock<Vec<u8>> = OnceLock::new();
static SOFT: OnceLock<Vec<u8>> = OnceLock::new();

pub fn play(chime: Chime) {
    let wav = match chime {
        Chime::Bell => BELL.get_or_init(|| wav(&[(784.0, 0.0), (1175.0, 0.16)], 1.3)),
        Chime::Drop => DROP.get_or_init(drop_sound),
        Chime::Soft => SOFT.get_or_init(|| wav(&[(988.0, 0.0), (740.0, 0.18)], 1.2)),
    };
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

/// Bell-like notes: a fundamental plus a quiet inharmonic partial with exponential decay.
fn wav(notes: &[(f32, f32)], seconds: f32) -> Vec<u8> {
    let n = (RATE as f32 * seconds) as usize;
    let mut samples = vec![0f32; n];
    for &(freq, start) in notes {
        let offset = (start * RATE as f32) as usize;
        for (i, s) in samples.iter_mut().enumerate().skip(offset) {
            let t = (i - offset) as f32 / RATE as f32;
            let attack = (t / 0.006).min(1.0);
            let env = attack * (-t * 4.2).exp();
            let tau = std::f32::consts::TAU;
            *s += env * (0.62 * (tau * freq * t).sin() + 0.18 * (tau * freq * 2.76 * t).sin() * (-t * 9.0).exp());
        }
    }
    encode(&samples, 0.42)
}

/// A water-drop "plip": a sine whose pitch sweeps upward quickly.
fn drop_sound() -> Vec<u8> {
    let n = (RATE as f32 * 0.35) as usize;
    let mut phase = 0f32;
    let samples: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let freq = 600.0 + 1300.0 * (1.0 - (-t * 30.0).exp());
            phase += std::f32::consts::TAU * freq / RATE as f32;
            let env = (t / 0.004).min(1.0) * (-t * 14.0).exp();
            env * phase.sin()
        })
        .collect();
    encode(&samples, 0.5)
}

fn encode(samples: &[f32], gain: f32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s * gain).clamp(-1.0, 1.0);
        out.extend_from_slice(&((v * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_consistent() {
        let w = wav(&[(440.0, 0.0)], 0.1);
        assert_eq!(&w[0..4], b"RIFF");
        let data_len = u32::from_le_bytes([w[40], w[41], w[42], w[43]]) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert_eq!(data_len, (RATE as f32 * 0.1) as usize * 2);
    }
}
