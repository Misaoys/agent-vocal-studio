use crate::timeline::Pulse;

pub(crate) struct Click {
    pulses: Vec<Pulse>,
    samples: [Vec<f32>; 2],
    next: usize,
    active: Option<usize>,
    previous: Option<i64>,
    pub played: u64,
    pub last_frame: Option<i64>,
}

impl Click {
    pub fn new(pulses: Vec<Pulse>) -> Self {
        let rate = crate::clips::RATE as f32;
        let samples = [(1000., 0.12), (1640., 0.18)].map(|(hz, gain)| {
            (0..1152)
                .map(|frame| {
                    let t = frame as f32 / rate;
                    let attack = (t / 0.001).min(1.);
                    let release = (1. - t / 0.024).max(0.).powi(3);
                    (std::f32::consts::TAU * hz * t).sin() * attack * release * gain
                })
                .collect()
        });
        Self {
            pulses,
            samples,
            next: 0,
            active: None,
            previous: None,
            played: 0,
            last_frame: None,
        }
    }

    pub fn reset(&mut self) {
        self.previous = None;
        self.active = None;
    }
    pub fn seek(&mut self, frame: i64) {
        self.active = None;
        self.previous = Some(frame);
        self.next = self.pulses.partition_point(|pulse| pulse.frame < frame);
    }

    // Called with the same source position used by audio playback.
    pub fn sample(&mut self, frame: i64) -> f32 {
        if self.previous.is_none_or(|previous| frame < previous) {
            self.seek(frame);
        }
        self.previous = Some(frame);
        while self
            .pulses
            .get(self.next)
            .is_some_and(|pulse| pulse.frame <= frame)
        {
            self.active = Some(self.next);
            self.last_frame = Some(self.pulses[self.next].frame);
            self.played += 1;
            self.next += 1;
        }
        let Some(index) = self.active else { return 0. };
        let pulse = &self.pulses[index];
        self.samples[usize::from(pulse.accent)]
            .get((frame - pulse.frame) as usize)
            .copied()
            .unwrap_or(0.)
    }
}
