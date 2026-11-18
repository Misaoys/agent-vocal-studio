//! Find short energy rises in three frequency bands of an audio source.
const HOP: u32 = 120; // 2.5 ms at 48 kHz
const HISTORY: usize = 8;
const SEPARATION: u32 = 2160; // 45 ms
const LIMIT: usize = 96;
// High percussion gives less RMS per band than a bass or midrange attack.
const RMS_FLOOR: [f32; 3] = [0.006, 0.004, 0.0025];

pub(crate) fn analysis_start(first: u32) -> u32 {
    (first / HOP).saturating_sub(HISTORY as u32) * HOP
}

#[derive(Default)]
pub(crate) struct Detector {
    low: [f32; 2],
    mid: [f32; 2],
    energy: [f32; 3],
    count: u32,
    bands: Vec<[f32; 3]>,
}

pub(crate) struct Attack {
    pub frame: u32,
    pub contrast: f32,
}

impl Detector {
    pub fn push(&mut self, samples: [f32; 2]) {
        for (side, sample) in samples.into_iter().enumerate() {
            // One-pole crossovers at about 240 Hz and 2.6 kHz.
            self.low[side] += 0.031 * (sample - self.low[side]);
            self.mid[side] += 0.288 * (sample - self.mid[side]);
            let bands = [
                self.low[side],
                self.mid[side] - self.low[side],
                sample - self.mid[side],
            ];
            for (power, band) in self.energy.iter_mut().zip(bands) {
                *power += band * band;
            }
        }
        self.count += 1;
        if self.count == HOP {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let scale = 1. / (2. * self.count as f32);
        self.bands
            .push(self.energy.map(|power| (power * scale).sqrt()));
        self.energy = [0.; 3];
        self.count = 0;
    }

    pub fn finish(mut self, source_start: u32, [first, end]: [u32; 2]) -> (Vec<Attack>, bool) {
        if self.count > 0 {
            self.flush();
        }
        let mut strengths = vec![0.; self.bands.len()];
        for (i, current) in self.bands.iter().enumerate().skip(1) {
            let history = &self.bands[i.saturating_sub(HISTORY)..i];
            for (band, &rms) in current.iter().enumerate() {
                let previous = history
                    .iter()
                    .map(|levels| levels[band])
                    .fold(RMS_FLOOR[band], f32::max);
                if rms > previous {
                    strengths[i] = f32::max(strengths[i], (rms / previous).ln());
                }
            }
        }
        let mut candidates = Vec::new();
        for (i, &strength) in strengths.iter().enumerate().skip(1) {
            if strength < 0.7
                || strength <= strengths[i - 1]
                || strengths.get(i + 1).is_some_and(|&next| strength < next)
            {
                continue;
            }
            // Locate the leading edge instead of the loudest point of the attack.
            let mut lead = i;
            while lead > i.saturating_sub(4) && strengths[lead - 1] >= strength * 0.25 {
                lead -= 1;
            }
            let frame = source_start + lead as u32 * HOP;
            if first <= frame && frame < end {
                candidates.push(Attack {
                    frame,
                    contrast: strength,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.contrast
                .total_cmp(&a.contrast)
                .then(a.frame.cmp(&b.frame))
        });
        let mut attacks: Vec<Attack> = Vec::new();
        let mut limited = false;
        for candidate in candidates {
            if attacks
                .iter()
                .any(|attack| attack.frame.abs_diff(candidate.frame) < SEPARATION)
            {
                continue;
            }
            if attacks.len() == LIMIT {
                limited = true;
                break;
            }
            attacks.push(candidate);
        }
        attacks.sort_by_key(|attack| attack.frame);
        (attacks, limited)
    }
}
