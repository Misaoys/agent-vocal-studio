//! Stereo 48 kHz output metering. Coefficients are from ITU-R BS.1770-5 Annexes 1 and 2.
use anyhow::Result;
use serde::Serialize;

const HOP: usize = 4_800;
const MOMENTARY: usize = 4;
const SHORT: usize = 30;
const OFFSET: f64 = -0.691;

#[derive(Default)]
struct Filter {
    history: [[f64; 2]; 2],
}

impl Filter {
    fn step(&mut self, input: [f64; 2], b: [f64; 3], a: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|ch| {
            let [z1, z2] = self.history[ch];
            let out = b[0] * input[ch] + z1;
            self.history[ch] = [
                b[1] * input[ch] - a[0] * out + z2,
                b[2] * input[ch] - a[1] * out,
            ];
            out
        })
    }
}

#[derive(Default)]
pub(crate) struct Loudness {
    shelf: Filter,
    high_pass: Filter,
    hops: [f64; SHORT],
    blocks: Vec<f64>,
    cursor: usize,
    hops_seen: usize,
    count: usize,
    energy: f64,
    short_energy: f64,
    max_momentary: f64,
    max_short: f64,
}

fn loudness(energy: f64) -> Option<f64> {
    (energy > 0.).then(|| OFFSET + 10. * energy.log10())
}

impl Loudness {
    pub(crate) fn push(&mut self, input: [f32; 2]) {
        let filtered = self.shelf.step(
            input.map(f64::from),
            [1.53512485958697, -2.69169618940638, 1.19839281085285],
            [-1.69065929318241, 0.73248077421585],
        );
        let filtered = self.high_pass.step(
            filtered,
            [1., -2., 1.],
            [-1.99004745483398, 0.99007225036621],
        );
        self.energy += filtered[0] * filtered[0] + filtered[1] * filtered[1];
        self.count += 1;
        if self.count < HOP {
            return;
        }
        if self.energy < 1e-24 {
            // Clear effectively silent IIR state before it decays into subnormal values.
            self.shelf = Filter::default();
            self.high_pass = Filter::default();
        }
        self.short_energy += self.energy - self.hops[self.cursor];
        self.hops[self.cursor] = self.energy;
        self.cursor = (self.cursor + 1) % SHORT;
        self.hops_seen += 1;
        self.energy = 0.;
        self.count = 0;
        if self.hops_seen >= MOMENTARY {
            let sum = (1..=MOMENTARY)
                .map(|offset| self.hops[(SHORT + self.cursor - offset) % SHORT])
                .sum::<f64>();
            let mean = sum / (HOP * MOMENTARY) as f64;
            self.max_momentary = self.max_momentary.max(mean);
            self.blocks.push(mean);
        }
        if self.hops_seen >= SHORT {
            self.max_short = self.max_short.max(self.short_energy / (HOP * SHORT) as f64);
        }
    }

    pub(crate) fn scan(frames: &[[f32; 2]], poll: impl Fn() -> Result<()>) -> Result<Self> {
        let mut meter = Self::default();
        for block in frames.chunks(8192) {
            poll()?;
            for frame in block {
                meter.push(*frame);
            }
        }
        Ok(meter)
    }

    pub(crate) fn integrated(&self, gain: f32) -> Option<f64> {
        let power = f64::from(gain).powi(2);
        let average_above = |threshold| {
            let (sum, count) = self
                .blocks
                .iter()
                .map(|block| block * power)
                .filter(|&block| block > threshold)
                .fold((0., 0), |(sum, count), block| (sum + block, count + 1));
            (count > 0).then(|| sum / f64::from(count))
        };
        let absolute = 10_f64.powf((-70. - OFFSET) / 10.);
        let relative = average_above(absolute)? * 0.1;
        loudness(average_above(absolute.max(relative))?)
    }
}

// Annex 2 FIR coefficients scaled by 8192. Phase 2 and 3 are the reverse of 1 and 0.
pub(crate) const FIR_TAPS: usize = 12;
const PHASES: [[f32; FIR_TAPS]; 4] = [
    [
        14., 90., -161., 272., -487., 1125., 7964., -838., 390., -218., 122., -68.,
    ],
    [
        -239., 240., -424., 730., -1364., 3810., 6388., -1641., 832., -477., 271., -155.,
    ],
    [
        -155., 271., -477., 832., -1641., 6388., 3810., -1364., 730., -424., 240., -239.,
    ],
    [
        -68., 122., -218., 390., -838., 7964., 1125., -487., 272., -161., 90., 14.,
    ],
];

#[derive(Default)]
pub(crate) struct TruePeak {
    history: [[f32; 2]; 2 * FIR_TAPS],
    cursor: usize,
    frame: usize,
    peak: f32,
    peak_frame: f64,
    channel: usize,
}

impl TruePeak {
    pub(crate) fn push(&mut self, input: [f32; 2]) -> f32 {
        self.cursor = (self.cursor + FIR_TAPS - 1) % FIR_TAPS;
        self.history[self.cursor] = input;
        self.history[self.cursor + FIR_TAPS] = input;
        let mut highest = crate::effects::peak(input);
        for (ch, input) in input.into_iter().enumerate() {
            if input.abs() > self.peak {
                self.peak = input.abs();
                self.peak_frame = self.frame as f64;
                self.channel = ch;
            }
        }
        let history = &self.history[self.cursor..][..FIR_TAPS];
        for (phase, weights) in PHASES.iter().enumerate() {
            let mut left = 0.;
            let mut right = 0.;
            for (sample, weight) in history.iter().zip(weights) {
                left += sample[0] * weight;
                right += sample[1] * weight;
            }
            for (ch, value) in [left, right].into_iter().enumerate() {
                let peak = value.abs() / 8192.;
                highest = highest.max(peak);
                if peak > self.peak {
                    self.peak = peak;
                    self.peak_frame = self.frame as f64 + phase as f64 / 4. - 23.5 / 4.;
                    self.channel = ch;
                }
            }
        }
        self.frame += 1;
        highest
    }

    pub(crate) fn flush(&mut self) -> f32 {
        for _ in 1..FIR_TAPS {
            self.push([0.; 2]);
        }
        self.peak
    }
}

#[derive(Default)]
pub(crate) struct Meter {
    loudness: Loudness,
    peak: TruePeak,
}

#[derive(Serialize)]
pub(crate) struct Report {
    standard: &'static str,
    integrated_lufs: Option<f64>,
    max_momentary_lufs: Option<f64>,
    max_short_term_lufs: Option<f64>,
    pub(crate) true_peak_dbtp: Option<f64>,
    true_peak_seconds: Option<f64>,
    true_peak_channel: Option<&'static str>,
}

impl Meter {
    pub(crate) fn push(&mut self, frame: [f32; 2]) {
        self.loudness.push(frame);
        self.peak.push(frame);
    }

    pub(crate) fn finish(mut self) -> Report {
        let frames = self.peak.frame;
        // Flush the interpolation delay at EOF. The loudness windows use only file samples.
        let has_peak = self.peak.flush() > 0.;
        Report {
            standard: "ITU-R BS.1770-5 stereo 48 kHz",
            integrated_lufs: self.loudness.integrated(1.),
            max_momentary_lufs: loudness(self.loudness.max_momentary),
            max_short_term_lufs: loudness(self.loudness.max_short),
            true_peak_dbtp: has_peak.then(|| 20. * f64::from(self.peak.peak).log10()),
            true_peak_seconds: has_peak
                .then(|| self.peak.peak_frame.clamp(0., frames as f64) / 48_000.),
            true_peak_channel: has_peak.then_some(["L", "R"][self.peak.channel]),
        }
    }
}
