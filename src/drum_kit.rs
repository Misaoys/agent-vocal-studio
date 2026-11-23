// Original synthesized one-shots. No external samples or voicebanks.
use std::f32::consts::TAU;
use std::sync::OnceLock;

pub const BANK: u16 = 129;
pub const PROGRAM: u8 = 25; // Closest GM-style MIDI export fallback; AVPROJ keeps BANK.
pub const NAME: &str = "暖电子 · Warm Electronic";
pub const FINGERPRINT: &str = "studio-warm-electronic-v1";
pub const RATE: usize = 48_000;

pub fn is_kit(instrument: &crate::model::Instrument) -> bool {
    instrument.drums && instrument.bank == BANK && instrument.program == PROGRAM
}

fn wave(pitch: u8) -> Vec<f32> {
    let (duration, decay, tone, amp) = match pitch {
        35 | 36 => (0.29, 0.076, if pitch == 35 { 41. } else { 49. }, 0.69),
        38 | 40 => (0.21, 0.046, if pitch == 38 { 168. } else { 205. }, 0.46),
        39 => (0.19, 0.037, 0., 0.44),
        42 => (0.064, 0.014, 0., 0.26),
        44 => (0.092, 0.020, 0., 0.22),
        46 => (0.31, 0.080, 0., 0.24),
        41 | 43 | 45 | 47 | 48 | 50 => (0.25, 0.055, 70. + (pitch - 41) as f32 * 10., 0.42),
        49 | 51 | 52 | 53 | 55 | 57 | 59 => (0.40, 0.102, 0., 0.23),
        75 | 76 | 77 => (0.100, 0.019, 450. + (pitch - 75) as f32 * 160., 0.26),
        _ => (0.145, 0.029, 130. + (pitch % 13) as f32 * 35., 0.26),
    };
    let mut seed = 0xca5e_7bd1_u32 ^ u32::from(pitch);
    let (mut fast, mut slow, mut phase) = (0., 0., 0.);
    let frames = (duration * RATE as f32) as usize;
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = i as f32 / RATE as f32;
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = (seed >> 8) as f32 / 8_388_608. - 1.;
        fast += 0.32 * (noise - fast);
        slow += 0.086 * (noise - slow);
        let band = (fast - slow) * 2.0;
        let kick = matches!(pitch, 35 | 36);
        let frequency = tone
            + if kick {
                105. * (-t / 0.013).exp()
            } else {
                42. * (-t / 0.025).exp()
            };
        phase += TAU * frequency / RATE as f32;
        let body = phase.sin();
        let tail = (-t / decay).exp();
        let hit = match pitch {
            35 | 36 => 0.97 * body * tail + 0.13 * band * (-t / 0.009).exp(),
            38 | 40 => 0.42 * body * (-t / 0.032).exp() + 0.84 * band * tail,
            39 => {
                let pulses = [0., 0.012, 0.025]
                    .into_iter()
                    .map(|offset| {
                        if t >= offset {
                            (-(t - offset) / 0.009).exp()
                        } else {
                            0.
                        }
                    })
                    .sum::<f32>();
                band * (0.54 * pulses + 0.20 * tail)
            }
            42 | 44 | 46 | 49 | 51 | 52 | 53 | 55 | 57 | 59 => {
                (0.93 * band + 0.07 * (TAU * (1126. + f32::from(pitch) * 9.) * t).sin()) * tail
            }
            75 | 76 | 77 => {
                // A rounded, two-part electronic tap can be used for repeat/glitch fills.
                let pulse = if t < 0.021 {
                    1.
                } else if t < 0.030 {
                    0.
                } else {
                    0.62
                };
                (body * 0.78 + band * 0.24) * tail * pulse
            }
            _ => (0.72 * body + 0.29 * band) * tail,
        };
        let attack = (t / if kick { 0.0024 } else { 0.0016 }).min(1.);
        let release = ((duration - t) / 0.018).clamp(0., 1.);
        out.push(amp * attack * release * hit);
    }
    out
}

pub fn sample(pitch: u8) -> &'static [f32] {
    static WAVES: OnceLock<[Vec<f32>; 47]> = OnceLock::new();
    if !(35..=81).contains(&pitch) {
        return &[];
    }
    &WAVES.get_or_init(|| std::array::from_fn(|n| wave(n as u8 + 35)))[usize::from(pitch - 35)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_are_distinct_bounded_and_have_smooth_ends() {
        for pitch in 35..=81 {
            let pcm = sample(pitch);
            assert_eq!(pcm[0], 0.);
            assert!(pcm.last().unwrap().abs() < 0.001);
            assert!(pcm.iter().all(|x| x.is_finite() && x.abs() <= 0.75));
            assert!(pcm.iter().any(|x| x.abs() > 0.035));
        }
        assert!(sample(46).len() > sample(42).len() * 3);
        assert_ne!(sample(38), sample(39));
        assert_eq!(sample(36), wave(36));
        assert!(sample(34).is_empty());
    }
}
