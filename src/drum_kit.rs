// Original synthesized one-shots. No external samples or voicebanks.
use std::f32::consts::TAU;
use std::sync::OnceLock;

pub const BANK: u16 = 129;
pub const PROGRAM: u8 = 25; // Closest GM-style MIDI export fallback; AVPROJ keeps BANK.
pub const NAME: &str = "暖电子 · Warm Electronic";
pub const FINGERPRINT: &str = "studio-warm-electronic-v1";
pub const RATE: usize = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kit {
    pub program: u8,
    pub name: &'static str,
    pub fingerprint: &'static str,
    pub midi_fallback: u8,
}
pub const KITS: [Kit; 4] = [
    Kit {
        program: PROGRAM,
        name: NAME,
        fingerprint: FINGERPRINT,
        midi_fallback: 25,
    },
    Kit {
        program: 27,
        name: "干电子 · Tight Electronic",
        fingerprint: "studio-tight-electronic-v1",
        midi_fallback: 24,
    },
    Kit {
        program: 28,
        name: "软鼓 · Soft Pocket",
        fingerprint: "studio-soft-pocket-v1",
        midi_fallback: 32,
    },
    Kit {
        program: 29,
        name: "碎拍 · Broken Beat",
        fingerprint: "studio-broken-beat-v1",
        midi_fallback: 26,
    },
];

pub fn for_instrument(instrument: &crate::model::Instrument) -> Option<Kit> {
    if !instrument.drums || instrument.bank != BANK {
        return None;
    }
    KITS.into_iter()
        .find(|kit| kit.program == instrument.program)
}

pub fn is_kit(instrument: &crate::model::Instrument) -> bool {
    for_instrument(instrument).is_some()
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

// Each original kit uses different physical envelopes and source mixtures: a
// dry drum-machine transient, a rounded skin-like body, or a gated/stepped pulse.
fn variant(kit: Kit, pitch: u8) -> Vec<f32> {
    let flavor = kit.program;
    let cymbal = matches!(pitch, 42 | 44 | 46 | 49 | 51 | 52 | 53 | 55 | 57 | 59);
    let kick = matches!(pitch, 35 | 36);
    let snare = matches!(pitch, 38 | 40);
    let clap = pitch == 39;
    let drum = matches!(pitch, 41 | 43 | 45 | 47 | 48 | 50);
    let open = pitch == 46 || (cymbal && pitch >= 49);
    let (duration, decay) = match (flavor, pitch) {
        (27, 35 | 36) => (0.155, 0.043),
        (28, 35 | 36) => (0.245, 0.069),
        (29, 35 | 36) => (0.215, 0.055),
        (27, 38..=40) => (0.113, 0.024),
        (28, 38..=40) => (0.205, 0.051),
        (29, 38..=40) => (0.181, 0.039),
        (27, _) if open => (0.205, 0.045),
        (28, _) if open => (0.360, 0.100),
        (29, _) if open => (0.250, 0.062),
        (27, _) if cymbal => (0.057, 0.010),
        (28, _) if cymbal => (0.085, 0.021),
        (29, _) if cymbal => (0.093, 0.023),
        (27, _) if drum => (0.130, 0.035),
        (28, _) if drum => (0.240, 0.067),
        (29, _) if drum => (0.175, 0.050),
        (27, _) => (0.081, 0.019),
        (28, _) => (0.124, 0.033),
        _ => (0.142, 0.034),
    };
    let (seed_base, fast_coef, slow_coef, attack_secs) = match flavor {
        27 => (0x7382_ad49, 0.37, 0.108, 0.0013),
        28 => (0xd517_7023, 0.21, 0.065, 0.0034),
        _ => (0x9f84_e831, 0.28, 0.078, 0.0020),
    };
    let tone = if kick {
        (if flavor == 27 {
            55.
        } else if flavor == 28 {
            41.
        } else {
            48.
        }) + f32::from(pitch - 35) * 5.
    } else if snare {
        (if flavor == 28 { 134. } else { 174. }) + f32::from(pitch - 38) * 13.
    } else if drum {
        74. + f32::from(pitch - 41) * 10.
    } else {
        185. + f32::from(pitch % 23) * 26.
    };
    let (mut phase, mut fast, mut slow, mut held) = (0_f32, 0_f32, 0_f32, 0_f32);
    let mut seed = seed_base ^ u32::from(pitch);
    let nframes = (duration * RATE as f32) as usize;
    let mut out = Vec::with_capacity(nframes);
    for i in 0..nframes {
        let t = i as f32 / RATE as f32;
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        if flavor != 29 || i % 5 == 0 {
            held = (seed >> 8) as f32 / 8_388_608. - 1.;
        }
        fast += (held - fast) * fast_coef;
        slow += (held - slow) * slow_coef;
        let band = (fast - slow) * 2.0;
        let (sweep, speed) = if kick {
            match flavor {
                27 => (137., 0.010),
                28 => (58., 0.023),
                _ => (98., 0.014),
            }
        } else {
            match flavor {
                27 => (59., 0.009),
                28 => (24., 0.029),
                _ => (77., 0.015),
            }
        };
        phase += TAU / RATE as f32 * (tone + sweep * (-t / speed).exp());
        let sine = phase.sin();
        let body = match flavor {
            27 => 0.92 * sine + 0.08 * (2. * phase).sin() * (-t / 0.014).exp(),
            28 => 0.90 * sine + 0.10 * (phase * 1.52).sin() * (-t / 0.028).exp(),
            _ => (0.78 * sine + 0.22 * (phase * 2.07).sin() * (-t / 0.028).exp()).tanh() * 1.18,
        };
        let tail = (-t / decay).exp();
        let pulse = |offset: f32, width: f32| {
            if t < offset {
                0.
            } else {
                let age = t - offset;
                (age / 0.0012).min(1.) * (-age / width).exp()
            }
        };
        let (signal, gain) = if kick {
            let click = if flavor == 27 {
                0.18
            } else if flavor == 28 {
                0.056
            } else {
                0.11
            };
            (body * tail + click * band * (-t / 0.008).exp(), 0.65)
        } else if snare {
            match flavor {
                27 => (0.36 * body * (-t / 0.018).exp() + 0.80 * band * tail, 0.44),
                28 => (
                    0.65 * body * (-t / 0.042).exp() + 0.50 * (band * 0.50 + slow) * tail,
                    0.45,
                ),
                _ => (
                    0.47 * body * (-t / 0.027).exp()
                        + 0.78 * band * (tail * 0.64 + 0.47 * pulse(0.042, 0.016)),
                    0.43,
                ),
            }
        } else if clap {
            let env = match flavor {
                27 => pulse(0., 0.006) + 0.60 * pulse(0.011, 0.010),
                28 => 0.76 * pulse(0., 0.016) + 0.45 * pulse(0.024, 0.026),
                _ => pulse(0., 0.009) + 0.71 * pulse(0.018, 0.011) + 0.47 * pulse(0.061, 0.008),
            };
            (band * 0.72 * (env + tail * 0.24), 0.44)
        } else if cymbal {
            let metallic = (TAU * (1500. + f32::from(pitch) * 6.) * t).sin() * 0.61
                + (TAU * 2327. * t).sin() * 0.39;
            let gate = if flavor == 29 {
                0.56 * tail + 0.38 * pulse(0.035, 0.012)
            } else {
                tail
            };
            let blend = if flavor == 28 { 0.027 } else { 0.055 };
            (
                ((1. - blend) * band + blend * metallic) * gate,
                if open { 0.22 } else { 0.24 },
            )
        } else {
            let env = if flavor == 29 && !drum {
                0.70 * pulse(0., 0.017) + 0.56 * pulse(0.038, 0.020)
            } else {
                tail
            };
            (
                (0.79 * body + 0.25 * band) * env,
                if drum { 0.38 } else { 0.24 },
            )
        };
        let attack = (t / attack_secs).min(1.);
        let release = ((duration - t) / 0.012).clamp(0., 1.);
        out.push(gain * attack * release * signal);
    }
    out
}

pub fn kit_sample(kit: Kit, pitch: u8) -> &'static [f32] {
    static WAVES: [OnceLock<[Vec<f32>; 47]>; KITS.len()] = [const { OnceLock::new() }; KITS.len()];
    if !(35..=81).contains(&pitch) {
        return &[];
    }
    let Some(index) = KITS.iter().position(|known| known.program == kit.program) else {
        return &[];
    };
    &WAVES[index].get_or_init(|| {
        std::array::from_fn(|n| {
            let key = n as u8 + 35;
            if index == 0 {
                wave(key)
            } else {
                variant(KITS[index], key)
            }
        })
    })[usize::from(pitch - 35)]
}

#[cfg(test)]
fn sample(pitch: u8) -> &'static [f32] {
    kit_sample(KITS[0], pitch)
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

    #[test]
    fn extended_kits_are_independent_and_complete() {
        for (i, kit) in KITS.into_iter().enumerate() {
            assert_eq!(
                for_instrument(&crate::model::Instrument {
                    drums: true,
                    bank: BANK,
                    program: kit.program
                }),
                Some(kit)
            );
            for pitch in 35..=81 {
                let pcm = kit_sample(kit, pitch);
                assert_eq!(pcm[0], 0.);
                assert!(pcm.last().unwrap().abs() < 0.001);
                assert!(pcm.iter().all(|x| x.is_finite() && x.abs() < 0.80));
                assert!(pcm.iter().any(|x| x.abs() > 0.015), "{kit:?}: {pitch}");
                for previous in &KITS[..i] {
                    assert_ne!(pcm, kit_sample(*previous, pitch), "{kit:?}: {pitch}");
                }
            }
            assert!(kit_sample(kit, 46).len() > kit_sample(kit, 42).len() * 2);
            assert!(kit_sample(kit, 82).is_empty());
        }
        assert_eq!(
            for_instrument(&crate::model::Instrument {
                drums: true,
                bank: 0,
                program: 25
            }),
            None
        );
        assert_eq!(
            for_instrument(&crate::model::Instrument {
                drums: false,
                bank: BANK,
                program: 27
            }),
            None
        );
    }
}
