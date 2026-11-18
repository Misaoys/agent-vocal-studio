use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::f64::consts::TAU;

const RATE: f64 = 48_000.;
pub const CHAIN_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    HighPass,
    Bell,
    LowPass,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    Eq {
        #[serde(default)]
        bypass: bool,
        shape: Shape,
        hz: f64,
        q: f64,
        gain_db: f64,
    },
    Compressor {
        #[serde(default)]
        bypass: bool,
        threshold_db: f64,
        ratio: f64,
        attack_ms: f64,
        release_ms: f64,
        knee_db: f64,
        makeup_db: f64,
        mix: f64,
    },
    Expander {
        #[serde(default)]
        bypass: bool,
        threshold_db: f64,
        ratio: f64,
        range_db: f64,
        knee_db: f64,
        attack_ms: f64,
        hold_ms: f64,
        release_ms: f64,
    },
    Ducker {
        #[serde(default)]
        bypass: bool,
        source_track_id: String,
        threshold_db: f64,
        range_db: f64,
        ratio: f64,
        attack_ms: f64,
        release_ms: f64,
    },
    DeEsser {
        #[serde(default)]
        bypass: bool,
        hz: f64,
        threshold_db: f64,
        range_db: f64,
        ratio: f64,
        attack_ms: f64,
        release_ms: f64,
    },
    Delay {
        #[serde(default)]
        bypass: bool,
        time_ms: f64,
        feedback: f64,
        damping_hz: f64,
        mix: f64,
        ping_pong: bool,
    },
    Reverb {
        #[serde(default)]
        bypass: bool,
        decay_s: f64,
        damping_hz: f64,
        mix: f64,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeakMode {
    #[default]
    SamplePeak,
    TruePeak,
}

impl PeakMode {
    fn is_sample(value: &Self) -> bool {
        *value == Self::SamplePeak
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Limiter {
    pub ceiling_db: f64,
    pub input_db: f64,
    pub lookahead_ms: f64,
    pub release_ms: f64,
    #[serde(default, skip_serializing_if = "PeakMode::is_sample")]
    pub peak_mode: PeakMode,
}

fn range(name: &str, value: f64, min: f64, max: f64) -> Result<()> {
    ensure!((min..=max).contains(&value), "{name} must be {min}..{max}");
    Ok(())
}

impl Effect {
    pub fn bypassed(&self) -> bool {
        match self {
            Self::Eq { bypass, .. }
            | Self::Compressor { bypass, .. }
            | Self::Expander { bypass, .. }
            | Self::Ducker { bypass, .. }
            | Self::DeEsser { bypass, .. }
            | Self::Delay { bypass, .. }
            | Self::Reverb { bypass, .. } => *bypass,
        }
    }

    pub(crate) fn sidechain_source(&self) -> Option<&str> {
        match self {
            Self::Ducker {
                source_track_id, ..
            } => Some(source_track_id),
            _ => None,
        }
    }

    pub(crate) fn active_sidechain(&self) -> Option<&str> {
        match self {
            Self::Ducker {
                bypass: false,
                source_track_id,
                range_db,
                ratio,
                ..
            } if *range_db > 0. && *ratio > 1. => Some(source_track_id),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        match *self {
            Self::Eq {
                shape,
                hz,
                q,
                gain_db,
                ..
            } => {
                range("EQ Hz", hz, 20., 20_000.)?;
                range("EQ Q", q, 0.15, 12.)?;
                range("EQ gain", gain_db, -24., 24.)?;
                ensure!(
                    shape == Shape::Bell || gain_db == 0.,
                    "Cut filters require zero gain"
                );
            }
            Self::Compressor {
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                knee_db,
                makeup_db,
                mix,
                ..
            } => {
                range("Compression threshold", threshold_db, -60., 0.)?;
                range("Compression ratio", ratio, 1., 30.)?;
                range("Compression attack", attack_ms, 0.1, 200.)?;
                range("Compression release", release_ms, 5., 2000.)?;
                range("Compression knee", knee_db, 0., 24.)?;
                range("Compression makeup", makeup_db, -24., 24.)?;
                range("Compression mix", mix, 0., 1.)?;
            }
            Self::Expander {
                threshold_db,
                ratio,
                range_db,
                knee_db,
                attack_ms,
                hold_ms,
                release_ms,
                ..
            } => {
                range("Expansion threshold", threshold_db, -90., 0.)?;
                range("Expansion ratio", ratio, 1., 12.)?;
                range("Expansion range", range_db, 0., 60.)?;
                range("Expansion knee", knee_db, 0., 24.)?;
                range("Expansion attack", attack_ms, 0.1, 100.)?;
                range("Expansion hold", hold_ms, 0., 250.)?;
                range("Expansion release", release_ms, 10., 2000.)?;
            }
            Self::Ducker {
                ref source_track_id,
                threshold_db,
                range_db,
                ratio,
                attack_ms,
                release_ms,
                ..
            } => {
                crate::model::identifier(source_track_id)?;
                range("Sidechain threshold", threshold_db, -72., 0.)?;
                range("Sidechain range", range_db, 0., 36.)?;
                range("Sidechain ratio", ratio, 1., 30.)?;
                range("Sidechain attack", attack_ms, 0.1, 200.)?;
                range("Sidechain release", release_ms, 5., 2000.)?;
            }
            Self::Delay {
                time_ms,
                feedback,
                damping_hz,
                mix,
                ..
            } => {
                range("Delay time", time_ms, 1., 2000.)?;
                range("Delay feedback", feedback, 0., 0.90)?;
                range("Delay damping", damping_hz, 100., 20_000.)?;
                range("Delay mix", mix, 0., 1.)?;
            }
            Self::DeEsser {
                hz,
                threshold_db,
                range_db,
                ratio,
                attack_ms,
                release_ms,
                ..
            } => {
                range("De-esser frequency", hz, 2500., 14_000.)?;
                range("De-esser threshold", threshold_db, -72., 0.)?;
                range("De-esser range", range_db, 0., 24.)?;
                range("De-esser ratio", ratio, 1., 12.)?;
                range("De-esser attack", attack_ms, 0.1, 20.)?;
                range("De-esser release", release_ms, 10., 500.)?;
            }
            Self::Reverb {
                decay_s,
                damping_hz,
                mix,
                ..
            } => {
                range("Reverb decay", decay_s, 0.15, 8.)?;
                range("Reverb damping", damping_hz, 250., 20_000.)?;
                range("Reverb mix", mix, 0., 1.)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_sidechains(
    effects: &[Effect],
    tracks: &[crate::model::Track],
    destination: Option<&str>,
) -> Result<()> {
    for source in effects.iter().filter_map(Effect::sidechain_source) {
        ensure!(
            Some(source) != destination,
            "Sidechain source must be a different track"
        );
        ensure!(
            tracks.iter().any(|track| track.id == source),
            "Sidechain source track does not exist: {source}"
        );
    }
    Ok(())
}

impl Limiter {
    pub(crate) const TRUE_PEAK_MARGIN_DB: f64 = 0.2;

    pub(crate) fn is_true_peak(&self) -> bool {
        self.peak_mode == PeakMode::TruePeak
    }

    pub(crate) fn detection_ceiling(&self) -> f32 {
        let margin = if self.is_true_peak() {
            Self::TRUE_PEAK_MARGIN_DB
        } else {
            0.
        };
        db_gain(self.ceiling_db - margin) as f32
    }

    pub fn validate(&self) -> Result<()> {
        range("Limiter ceiling", self.ceiling_db, -24., -0.1)?;
        range("Limiter input", self.input_db, -12., 12.)?;
        range("Limiter lookahead", self.lookahead_ms, 0., 20.)?;
        range("Limiter release", self.release_ms, 10., 2000.)
    }

    pub fn gain_envelope(
        &self,
        mix: &[[f32; 2]],
        gains: &mut [f32],
        poll: impl Fn() -> Result<()>,
    ) -> Result<f32> {
        ensure!(
            mix.len() == gains.len(),
            "Limiter peak buffer length mismatch"
        );
        let input = db_gain(self.input_db) as f32;
        let ceiling = self.detection_ceiling();
        let true_mode = self.is_true_peak();
        if true_mode && !mix.is_empty() {
            let support = crate::loudness::FIR_TAPS - 1;
            let mut detector = crate::loudness::TruePeak::default();
            for (index, (frame, required)) in mix.iter().zip(gains.iter_mut()).enumerate() {
                if index % 8192 == 0 {
                    poll()?;
                }
                *required = (peak(*frame) * input / ceiling).max(*required * input / 0.999);
            }
            for (index, frame) in mix
                .iter()
                .copied()
                .chain(std::iter::repeat_n([0.; 2], support))
                .enumerate()
            {
                if index % 8192 == 0 {
                    poll()?;
                }
                let required = detector.push(frame) * input / ceiling;
                // Start the reduction at the first source sample contributing to the FIR window.
                let at = &mut gains[index.saturating_sub(support)];
                *at = at.max(required);
            }
        }
        let ramp = if self.lookahead_ms > 0. {
            (1. / (RATE * self.lookahead_ms / 1000.).round().max(1.)) as f32
        } else {
            1.
        };
        let mut next = 1_f32;
        // Prepare gain before each peak without shifting the timeline.
        for (index, (frame, stem_peak)) in mix.iter().zip(gains.iter_mut()).enumerate().rev() {
            if index % 8192 == 0 {
                poll()?;
            }
            let constraint = if true_mode {
                *stem_peak
            } else {
                (peak(*frame) * input / ceiling).max(*stem_peak * input / 0.999)
            };
            next = 1.0_f32.min(constraint.recip()).min(next + ramp);
            *stem_peak = next;
        }
        let coefficient = smooth_ms(self.release_ms, RATE) as f32;
        let mut current = 1_f32;
        let mut minimum = 1_f32;
        for (index, gain) in gains.iter_mut().enumerate() {
            if index % 8192 == 0 {
                poll()?;
            }
            current = gain.min(1. - (1. - current) * coefficient);
            minimum = minimum.min(current);
            *gain = current * input;
        }
        Ok(-20. * minimum.log10())
    }
}

pub fn catalog() -> Value {
    json!({"track_limit":CHAIN_LIMIT,"sample_rate":48000,"aux_routing":{
        "max_buses":crate::routing::AUX_LIMIT,"tap":"post_insert_post_fader_post_pan_and_automation",
        "send_gain_db":[-60,0],"return_gain_db":[-60,12],
        "bus_fields":{"id":"unique identifier","name":"display name","gain_db":"number; default 0","mute":"boolean; default false","effects":"ordered insert chain"},
        "send_fields":{"bus_id":"existing auxiliary return ID","gain_db":"number"},
        "wet_only_tip":"Use mix:1 on a shared delay or reverb to keep the dry signal from being doubled."
    },"mix_groups":{
        "max_groups":crate::routing::GROUP_LIMIT,"gain_db":[-60,12],
        "project_field":"mix_groups","track_field":"output_group",
        "signal":"Named track outputs sum once into the group before its effects and fader. A null track output goes straight to the master.",
        "aux_sends":"Track sends branch before the group effects and fader. Group mute blocks new sends from its members."
    },"sidechain":{"effect":"ducker","source_field":"source_track_id",
        "tap":"rendered track audio before inserts, fader, pan, automation, mute and solo",
        "destinations":["track","mix_group","aux_return"],"source":"existing track ID; use a different track for track inserts",
        "empty_source":"zero signal"
    },"effects":[
        {"kind":"eq","label":"参数均衡","defaults":{"kind":"eq","bypass":false,"shape":"bell","hz":250,"q":1,"gain_db":0},"fields":[["shape","滤波形状",[["high_pass","低切"],["bell","峰值"],["low_pass","高切"]]],["hz","频率 · Hz",20,20000,1],["q","Q",0.15,12,0.01],["gain_db","增益 · dB",-24,24,0.1]]},
        {"kind":"compressor","label":"压缩器","defaults":{"kind":"compressor","bypass":false,"threshold_db":-18,"ratio":3,"attack_ms":12,"release_ms":110,"knee_db":6,"makeup_db":0,"mix":1},"fields":[["threshold_db","阈值 · dB",-60,0,0.1],["ratio","比率",1,30,0.1],["attack_ms","启动 · ms",0.1,200,0.1],["release_ms","释放 · ms",5,2000,1],["knee_db","软拐点 · dB",0,24,0.1],["makeup_db","补偿 · dB",-24,24,0.1],["mix","混合",0,1,0.01]]},
        {"kind":"expander","label":"向下扩展","defaults":{"kind":"expander","bypass":false,"threshold_db":-44,"ratio":2,"range_db":12,"knee_db":6,"attack_ms":2,"hold_ms":45,"release_ms":135},"fields":[["threshold_db","阈值 · dBFS",-90,0,0.1],["ratio","比率",1,12,0.1],["range_db","最大衰减 · dB",0,60,0.1],["knee_db","软拐点 · dB",0,24,0.1],["attack_ms","开启 · ms",0.1,100,0.1],["hold_ms","保持 · ms",0,250,1],["release_ms","关闭 · ms",10,2000,1]]},
        {"kind":"ducker","label":"旁链闪避","defaults":{"kind":"ducker","bypass":false,"source_track_id":"","threshold_db":-28,"range_db":6,"ratio":4,"attack_ms":8,"release_ms":150},"fields":[["source_track_id","触发音轨",[]],["threshold_db","阈值 · dB",-72,0,0.1],["range_db","最大衰减 · dB",0,36,0.1],["ratio","比率",1,30,0.1],["attack_ms","启动 · ms",0.1,200,0.1],["release_ms","释放 · ms",5,2000,1]]},
        {"kind":"de_esser","label":"齿音控制","defaults":{"kind":"de_esser","bypass":false,"hz":6200,"threshold_db":-27,"range_db":6,"ratio":5,"attack_ms":1,"release_ms":65},"fields":[["hz","分频 · Hz",2500,14000,10],["threshold_db","高频阈值 · dB",-72,0,0.1],["range_db","最大衰减 · dB",0,24,0.1],["ratio","比率",1,12,0.1],["attack_ms","启动 · ms",0.1,20,0.1],["release_ms","释放 · ms",10,500,1]]},
        {"kind":"delay","label":"延迟","defaults":{"kind":"delay","bypass":false,"time_ms":240,"feedback":0.26,"damping_hz":4600,"mix":0.18,"ping_pong":true},"fields":[["time_ms","时长 · ms",1,2000,1],["feedback","反馈",0,0.9,0.01],["damping_hz","阻尼 · Hz",100,20000,1],["mix","混合",0,1,0.01],["ping_pong","左右交替"]]},
        {"kind":"reverb","label":"混响","defaults":{"kind":"reverb","bypass":false,"decay_s":1.4,"damping_hz":5100,"mix":0.13},"fields":[["decay_s","衰减 · 秒",0.15,8,0.01],["damping_hz","阻尼 · Hz",250,20000,1],["mix","混合",0,1,0.01]]}
    ],"master_limiter":{"label":"总线限制器","defaults":{"ceiling_db":-1,"input_db":0,"lookahead_ms":5,"release_ms":100},"fields":[["peak_mode","检测",[["sample_peak","采样峰值"],["true_peak","真峰值 · 导出"]]],["ceiling_db","上限 · dBFS",-24,-0.1,0.1],["input_db","输入 · dB",-12,12,0.1],["lookahead_ms","前瞻 · ms",0,20,0.1],["release_ms","释放 · ms",10,2000,1]],
        "peak_modes":{"sample_peak":"dBFS, default","true_peak":"dBTP, stereo 48 kHz offline render and A/B; live audition uses the rendered WAV"},
        "true_peak_margin_db":Limiter::TRUE_PEAK_MARGIN_DB}})
}

pub fn peak(frame: [f32; 2]) -> f32 {
    frame[0].abs().max(frame[1].abs())
}
pub(crate) fn db_gain(db: f64) -> f64 {
    10_f64.powf(db / 20.)
}
pub(crate) fn smooth_ms(ms: f64, rate: f64) -> f64 {
    (-1000. / (rate * ms)).exp()
}
fn lowpass_memory(hz: f64, rate: f64) -> f64 {
    (-TAU * hz / rate).exp()
}

pub(crate) struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    state: [[f64; 2]; 2],
}
impl Biquad {
    pub(crate) fn new(shape: Shape, hz: f64, q: f64, db: f64, rate: f64) -> Self {
        let (sin, cos) = (TAU * hz.min(0.49 * rate) / rate).sin_cos();
        let alpha = sin / (2. * q);
        let (b, a0, a2) = match shape {
            Shape::Bell => {
                let a = db_gain(db / 2.);
                (
                    [1. + alpha * a, -2. * cos, 1. - alpha * a],
                    1. + alpha / a,
                    1. - alpha / a,
                )
            }
            Shape::HighPass => {
                let b0 = (1. + cos) / 2.;
                ([b0, -2. * b0, b0], 1. + alpha, 1. - alpha)
            }
            Shape::LowPass => {
                let b0 = (1. - cos) / 2.;
                ([b0, 2. * b0, b0], 1. + alpha, 1. - alpha)
            }
        };
        Self {
            b: b.map(|v| v / a0),
            a: [-2. * cos / a0, a2 / a0],
            state: [[0.; 2]; 2],
        }
    }
    pub(crate) fn process(&mut self, frame: &mut [f32; 2]) {
        for (value, [first, last]) in frame.iter_mut().zip(&mut self.state) {
            let x = f64::from(*value);
            let y = self.b[0] * x + *first;
            *first = self.b[1] * x - self.a[0] * y + *last;
            *last = self.b[2] * x - self.a[1] * y;
            *value = y as f32;
        }
    }
}

struct DeEsser {
    detector: Biquad,
    split: f64,
    low: [f64; 2],
    peak_memory: f64,
    envelope: f64,
    threshold: f64,
    minimum: f64,
    slope: f64,
    attack: f64,
    release: f64,
    gain: f64,
    lowest_gain: f64,
}

impl DeEsser {
    fn process(&mut self, frame: &mut [f32; 2]) {
        let mut band = *frame;
        self.detector.process(&mut band);
        self.envelope = f64::from(peak(band)).max(self.envelope * self.peak_memory);
        let target = if self.envelope > self.threshold {
            (self.threshold / self.envelope)
                .powf(self.slope)
                .max(self.minimum)
        } else {
            1.
        };
        let time = if target < self.gain {
            self.attack
        } else {
            self.release
        };
        self.gain = target + (self.gain - target) * time;
        self.lowest_gain = self.lowest_gain.min(self.gain);

        // Complementary split: preserve the original PCM at unity and subtract
        // only a fraction of the high-pass signal when reduction is active.
        for (sample, state) in frame.iter_mut().zip(&mut self.low) {
            let dry = f64::from(*sample);
            let step = (dry - *state) * self.split;
            let low = *state + step;
            *state = low + step;
            *sample = (dry + (dry - low) * (self.gain - 1.)) as f32;
        }
    }

    #[cfg(windows)]
    fn reset(&mut self) {
        self.detector.state = [[0.; 2]; 2];
        self.low = [0.; 2];
        self.envelope = 0.;
        self.gain = 1.;
        self.lowest_gain = 1.;
    }
}

struct DelayLine {
    data: Vec<f64>,
    index: usize,
    low: f64,
    filled: usize,
}
impl DelayLine {
    fn new(samples: usize) -> Self {
        Self {
            data: vec![0.; samples.max(1)],
            index: 0,
            low: 0.,
            filled: 0,
        }
    }
    fn read(&self) -> f64 {
        if self.filled < self.data.len() {
            0.
        } else {
            self.data[self.index]
        }
    }
    fn write(&mut self, sample: f64) {
        self.data[self.index] = sample;
        self.index = (self.index + 1) % self.data.len();
        self.filled = (self.filled + 1).min(self.data.len());
    }
    fn filtered(&mut self, memory: f64) -> f64 {
        self.low = memory * self.low + (1. - memory) * self.read();
        self.low
    }
    #[cfg(windows)]
    fn reset(&mut self) {
        self.index = 0;
        self.filled = 0;
        self.low = 0.;
    }
}

struct Expander {
    threshold_db: f64,
    slope: f64,
    range_db: f64,
    knee_db: f64,
    open_level: f64,
    floor_level: f64,
    detector_memory: f64,
    attack: f64,
    release: f64,
    hold_frames: u32,
    hold_left: u32,
    envelope: f64,
    reduction_db: f64,
    maximum_db: f64,
}

impl Expander {
    fn process(&mut self, frame: &mut [f32; 2]) {
        let level = f64::from(peak(*frame));
        self.envelope = level.max(self.envelope * self.detector_memory);
        let target_db = if self.envelope >= self.open_level {
            self.hold_left = self.hold_frames;
            0.
        } else if self.hold_left > 0 {
            self.hold_left -= 1;
            0.
        } else if self.envelope <= self.floor_level {
            self.range_db
        } else {
            let below = self.threshold_db - 20. * self.envelope.log10();
            let knee = (below + self.knee_db * 0.5).max(0.);
            let amount = if knee < self.knee_db {
                knee * knee / (2. * self.knee_db)
            } else {
                below.max(0.)
            };
            (self.slope * amount).min(self.range_db)
        };
        let memory = if target_db < self.reduction_db {
            self.attack
        } else {
            self.release
        };
        self.reduction_db = target_db + (self.reduction_db - target_db) * memory;
        if level > 1e-8 {
            self.maximum_db = self.maximum_db.max(self.reduction_db);
        }
        let gain = db_gain(-self.reduction_db) as f32;
        *frame = frame.map(|sample| sample * gain);
    }

    #[cfg(windows)]
    fn reset(&mut self) {
        self.hold_left = 0;
        self.envelope = 0.;
        self.reduction_db = 0.;
        self.maximum_db = 0.;
    }
}

struct Ducker {
    key: Option<usize>,
    threshold: f64,
    floor: f64,
    slope: f64,
    attack: f64,
    release: f64,
    gain: f64,
    lowest_gain: f64,
}

impl Ducker {
    fn process(&mut self, frame: &mut [f32; 2], peaks: &[f32]) {
        let level = self
            .key
            .and_then(|index| peaks.get(index))
            .copied()
            .unwrap_or_default();
        let target = if f64::from(level) > self.threshold {
            (f64::from(level) / self.threshold)
                .powf(self.slope)
                .max(self.floor)
        } else {
            1.
        };
        let smooth = if target < self.gain {
            self.attack
        } else {
            self.release
        };
        self.gain = target + (self.gain - target) * smooth;
        self.lowest_gain = self.lowest_gain.min(self.gain);
        *frame = frame.map(|sample| sample * self.gain as f32);
    }
}

enum Insert {
    Eq(Biquad),
    Expander(Expander),
    DeEsser(DeEsser),
    Ducker(Ducker),
    Compressor {
        threshold_db: f64,
        slope: f64,
        attack: f64,
        release: f64,
        knee_db: f64,
        makeup: f64,
        mix: f64,
        reduction: f64,
    },
    Delay {
        lines: [DelayLine; 2],
        memory: f64,
        feedback: f64,
        mix: f64,
        ping_pong: bool,
    },
    Reverb {
        rooms: Box<[Room; 2]>,
        memory: f64,
        mix: f64,
    },
}

impl Insert {
    fn new(effect: &Effect, rate: f64, resolve: &impl Fn(&str) -> Option<usize>) -> Option<Self> {
        if effect.bypassed() {
            return None;
        }
        Some(match *effect {
            Effect::Eq {
                shape,
                hz,
                q,
                gain_db,
                ..
            } => {
                if shape == Shape::Bell && gain_db == 0. {
                    return None;
                }
                Self::Eq(Biquad::new(shape, hz, q, gain_db, rate))
            }
            Effect::Compressor {
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                knee_db,
                makeup_db,
                mix,
                ..
            } => Self::Compressor {
                threshold_db,
                slope: ratio.recip() - 1.,
                attack: smooth_ms(attack_ms, rate),
                release: smooth_ms(release_ms, rate),
                knee_db,
                makeup: db_gain(makeup_db),
                mix,
                reduction: 0.,
            },
            Effect::Expander {
                threshold_db,
                ratio,
                range_db,
                knee_db,
                attack_ms,
                hold_ms,
                release_ms,
                ..
            } => {
                if range_db == 0. || ratio == 1. {
                    return None;
                }
                let slope = ratio - 1.;
                Self::Expander(Expander {
                    threshold_db,
                    slope,
                    range_db,
                    knee_db,
                    open_level: db_gain(threshold_db + knee_db * 0.5),
                    floor_level: db_gain(threshold_db - (range_db / slope).max(knee_db * 0.5)),
                    detector_memory: smooth_ms(7., rate),
                    attack: smooth_ms(attack_ms, rate),
                    release: smooth_ms(release_ms, rate),
                    hold_frames: (hold_ms * rate / 1000.).round() as u32,
                    hold_left: 0,
                    envelope: 0.,
                    reduction_db: 0.,
                    maximum_db: 0.,
                })
            }
            Effect::Ducker {
                ref source_track_id,
                threshold_db,
                range_db,
                ratio,
                attack_ms,
                release_ms,
                ..
            } => {
                if range_db == 0. || ratio == 1. {
                    return None;
                }
                Self::Ducker(Ducker {
                    key: resolve(source_track_id),
                    threshold: db_gain(threshold_db),
                    floor: db_gain(-range_db),
                    slope: ratio.recip() - 1.,
                    attack: smooth_ms(attack_ms, rate),
                    release: smooth_ms(release_ms, rate),
                    gain: 1.,
                    lowest_gain: 1.,
                })
            }
            Effect::DeEsser {
                hz,
                threshold_db,
                range_db,
                ratio,
                attack_ms,
                release_ms,
                ..
            } => {
                if ratio == 1. || range_db == 0. {
                    return None;
                }
                let hz = hz.min(rate * 0.45);
                let split = (TAU * hz / (2. * rate)).tan();
                Self::DeEsser(DeEsser {
                    detector: Biquad::new(
                        Shape::HighPass,
                        hz,
                        std::f64::consts::FRAC_1_SQRT_2,
                        0.,
                        rate,
                    ),
                    split: split / (1. + split),
                    low: [0.; 2],
                    peak_memory: smooth_ms(3., rate),
                    envelope: 0.,
                    threshold: db_gain(threshold_db),
                    minimum: db_gain(-range_db),
                    slope: 1. - ratio.recip(),
                    attack: smooth_ms(attack_ms, rate),
                    release: smooth_ms(release_ms, rate),
                    gain: 1.,
                    lowest_gain: 1.,
                })
            }
            Effect::Delay {
                time_ms,
                feedback,
                damping_hz,
                mix,
                ping_pong,
                ..
            } => {
                let frames = (rate * time_ms / 1000.).round() as usize;
                Self::Delay {
                    lines: [DelayLine::new(frames), DelayLine::new(frames)],
                    memory: lowpass_memory(damping_hz, rate),
                    feedback,
                    mix,
                    ping_pong,
                }
            }
            Effect::Reverb {
                decay_s,
                damping_hz,
                mix,
                ..
            } => Self::Reverb {
                rooms: Box::new([Room::new(0, decay_s, rate), Room::new(43, decay_s, rate)]),
                memory: lowpass_memory(damping_hz, rate),
                mix,
            },
        })
    }

    fn process(&mut self, frame: &mut [f32; 2], peaks: &[f32]) {
        match self {
            Self::Eq(filter) => filter.process(frame),
            Self::Expander(processor) => processor.process(frame),
            Self::DeEsser(processor) => processor.process(frame),
            Self::Ducker(processor) => processor.process(frame, peaks),
            Self::Compressor {
                threshold_db,
                slope,
                attack,
                release,
                knee_db,
                makeup,
                mix,
                reduction,
            } => {
                let above = 20. * f64::from(peak(*frame).max(1e-12)).log10() - *threshold_db;
                let knee = above + *knee_db * 0.5;
                let desired = if knee <= 0. {
                    0.
                } else if knee < *knee_db {
                    *slope * knee * knee / (2. * *knee_db)
                } else {
                    *slope * above
                };
                let coefficient = if desired < *reduction {
                    *attack
                } else {
                    *release
                };
                *reduction = coefficient * *reduction + (1. - coefficient) * desired;
                let gain = ((1. - *mix) + *mix * *makeup * db_gain(*reduction)) as f32;
                *frame = frame.map(|sample| sample * gain);
            }
            Self::Delay {
                lines,
                memory,
                feedback,
                mix,
                ping_pong,
            } => {
                let wet = lines.each_mut().map(|line| line.filtered(*memory));
                for (side, (sample, line)) in frame.iter_mut().zip(lines).enumerate() {
                    let dry = f64::from(*sample);
                    line.write(dry + *feedback * wet[if *ping_pong { 1 - side } else { side }]);
                    *sample = ((1. - *mix) * dry + *mix * wet[side]) as f32;
                }
            }
            Self::Reverb { rooms, memory, mix } => {
                let dry = frame.map(f64::from);
                for (side, (sample, room)) in frame.iter_mut().zip(rooms.iter_mut()).enumerate() {
                    let wet = room.process(dry[side] * 0.88 + dry[1 - side] * 0.12, *memory);
                    *sample = ((1. - *mix) * dry[side] + *mix * wet) as f32;
                }
            }
        }
    }
    #[cfg(windows)]
    fn reset(&mut self) {
        match self {
            Self::Eq(eq) => eq.state = [[0.; 2]; 2],
            Self::Expander(processor) => processor.reset(),
            Self::DeEsser(processor) => processor.reset(),
            Self::Ducker(processor) => {
                processor.gain = 1.;
                processor.lowest_gain = 1.;
            }
            Self::Compressor { reduction, .. } => *reduction = 0.,
            Self::Delay { lines, .. } => lines.iter_mut().for_each(DelayLine::reset),
            Self::Reverb { rooms, .. } => {
                for room in rooms.iter_mut() {
                    room.combs.iter_mut().for_each(|(line, _)| line.reset());
                    room.diffusers.iter_mut().for_each(DelayLine::reset);
                }
            }
        }
    }
}

pub(crate) struct Chain(Vec<Insert>);
impl Chain {
    pub(crate) fn new(
        effects: &[Effect],
        rate: f64,
        resolve: impl Fn(&str) -> Option<usize>,
    ) -> Self {
        Self(
            effects
                .iter()
                .filter_map(|effect| Insert::new(effect, rate, &resolve))
                .collect(),
        )
    }
    pub(crate) fn process(&mut self, frame: &mut [f32; 2], peaks: &[f32]) {
        for insert in &mut self.0 {
            insert.process(frame, peaks)
        }
    }
    pub(crate) fn reduction_report(&self, report: &mut Value) {
        for (name, maximum) in [
            "de_esser_max_reduction_db",
            "ducking_max_reduction_db",
            "expander_max_reduction_db",
        ]
        .into_iter()
        .zip(self.reduction_maxima())
        {
            if let Some(db) = maximum {
                report[name] = json!(db);
            }
        }
    }
    fn reduction_maxima(&self) -> [Option<f64>; 3] {
        let mut maxima = [None; 3];
        for insert in &self.0 {
            let (index, db) = match insert {
                Insert::DeEsser(processor) => (0, -20. * processor.lowest_gain.log10()),
                Insert::Ducker(processor) => (1, -20. * processor.lowest_gain.log10()),
                Insert::Expander(processor) => (2, processor.maximum_db),
                _ => continue,
            };
            maxima[index] = Some(maxima[index].map_or(db, |previous| db.max(previous)));
        }
        maxima
    }
    #[cfg(windows)]
    pub(crate) fn reset(&mut self) {
        self.0.iter_mut().for_each(Insert::reset);
    }
}

#[cfg(test)]
fn process(
    chain: &[Effect],
    data: &mut [[f32; 2]],
    poll: impl Fn() -> Result<()>,
) -> Result<Option<f64>> {
    let mut chain = Chain::new(chain, RATE, |_| None);
    for chunk in data.chunks_mut(8192) {
        poll()?;
        for frame in chunk {
            chain.process(frame, &[])
        }
    }
    Ok(chain.reduction_maxima()[0])
}

struct Room {
    combs: [(DelayLine, f64); 4],
    diffusers: [DelayLine; 2],
}
impl Room {
    fn new(spread: usize, decay: f64, rate: f64) -> Self {
        let length = |size| ((size + spread) as f64 * rate / RATE).round().max(1.) as usize;
        Self {
            combs: [1429, 1607, 1777, 1997].map(|n| {
                let n = length(n);
                (
                    DelayLine::new(n),
                    10_f64.powf(-3. * n as f64 / (rate * decay)),
                )
            }),
            diffusers: [241, 379].map(|n| DelayLine::new(length(n))),
        }
    }
    fn process(&mut self, input: f64, memory: f64) -> f64 {
        let mut sum = 0.;
        for (line, gain) in &mut self.combs {
            let delayed = line.filtered(memory);
            line.write(input + delayed * *gain);
            sum += delayed * 0.25;
        }
        for line in &mut self.diffusers {
            let past = line.read();
            let output = past - 0.55 * sum;
            line.write(sum + 0.55 * output);
            sum = output;
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(signal: &[[f32; 2]]) -> f64 {
        (signal
            .iter()
            .map(|frame| f64::from(frame[0]).powi(2))
            .sum::<f64>()
            / signal.len() as f64)
            .sqrt()
    }

    #[test]
    fn eq_gain_and_low_cut_are_measurable() -> Result<()> {
        let signal = |hz: f64| {
            (0..4800)
                .map(|i| [(TAU * hz * i as f64 / RATE).sin() as f32; 2])
                .collect::<Vec<_>>()
        };
        let mut low = signal(100.);
        let mut high = signal(5_000.);
        let cutoff = Effect::Eq {
            bypass: false,
            shape: Shape::HighPass,
            hz: 600.,
            q: 0.707,
            gain_db: 0.,
        };
        process(std::slice::from_ref(&cutoff), &mut low, || Ok(()))?;
        process(&[cutoff], &mut high, || Ok(()))?;
        assert!(rms(&low[2400..]) < 0.024);
        assert!((rms(&high[2400..]) - 2f64.sqrt() / 2.).abs() < 0.004);
        let mut bell = signal(600.);
        process(
            &[Effect::Eq {
                bypass: false,
                shape: Shape::Bell,
                hz: 600.,
                q: 1.,
                gain_db: -12.,
            }],
            &mut bell,
            || Ok(()),
        )?;
        assert!((rms(&bell[2400..]) * 2f64.sqrt() - db_gain(-12.)).abs() < 1e-4);
        Ok(())
    }

    #[test]
    fn soft_knee_compression_preserves_stereo_image() -> Result<()> {
        let mut signal = vec![[1., 0.25]; 4800];
        process(
            &[Effect::Compressor {
                bypass: false,
                threshold_db: -20.,
                ratio: 4.,
                attack_ms: 1.,
                release_ms: 100.,
                knee_db: 6.,
                makeup_db: 0.,
                mix: 1.,
            }],
            &mut signal,
            || Ok(()),
        )?;
        assert!(signal[0][0] > 0.9);
        assert!((f64::from(signal[4799][0]) - db_gain(-15.)).abs() < 1e-5);
        assert!(signal.iter().all(|sample| sample[1] == sample[0] / 4.));
        Ok(())
    }

    #[test]
    fn expansion_hold_knee_and_reopen_are_linked_at_device_rates() {
        let effect = Effect::Expander {
            bypass: false,
            threshold_db: -36.,
            ratio: 2.5,
            range_db: 14.,
            knee_db: 6.,
            attack_ms: 2.,
            hold_ms: 65.,
            release_ms: 45.,
        };
        for rate in [8_000., 44_100., 48_000., 96_000.] {
            let mut chain = Chain::new(std::slice::from_ref(&effect), rate, |_| None);
            let mut play = |level: f32, seconds: f64| {
                let mut output = level;
                for index in 0..(rate * seconds).round() as usize {
                    let sign = if index % 2 == 0 { 1. } else { -1. };
                    let mut frame = [sign * level, sign * level * 0.25];
                    chain.process(&mut frame, &[]);
                    assert_eq!(frame[0] * 0.25, frame[1]);
                    output = frame[0].abs();
                }
                20. * f64::from(output / level).log10()
            };
            assert_eq!(play(0.1, 0.06), 0.);
            assert_eq!(play(0.003, 0.04), 0.);
            assert!((play(0.003, 0.45) + 14.).abs() < 0.01);
            let knee = db_gain(-36.) as f32;
            assert!((play(knee, 0.4) + 1.125).abs() < 0.01);
            let first = play(0.1, rate.recip());
            assert!(first > -1.125 && first < -0.8, "{rate}: {first}");
            assert!(play(0.1, 0.025).abs() < 1e-5);
            assert!(chain.reduction_maxima()[2].unwrap() > 13.99);
            #[cfg(windows)]
            {
                chain.reset();
                let mut frame = [0.1, 0.025];
                chain.process(&mut frame, &[]);
                assert_eq!(frame, [0.1, 0.025]);
                assert_eq!(chain.reduction_maxima()[2], Some(0.));
            }
        }
    }

    #[test]
    fn de_esser_is_transparent_before_triggering_and_linked_at_device_rates() {
        let effect = Effect::DeEsser {
            bypass: false,
            hz: 6200.,
            threshold_db: -28.,
            range_db: 8.,
            ratio: 5.,
            attack_ms: 1.,
            release_ms: 65.,
        };
        for rate in [8_000., 44_100., 48_000., 96_000.] {
            let mut chain = Chain::new(std::slice::from_ref(&effect), rate, |_| None);
            for i in 0..(rate as usize / 10) {
                let low = (0.18 * (TAU * 180. * i as f64 / rate).sin()) as f32;
                let mut frame = [low, low * 0.25];
                let dry = frame;
                chain.process(&mut frame, &[]);
                assert_eq!(frame, dry);
            }
            let mut dry = Vec::new();
            let mut wet = Vec::new();
            for i in 0..(rate as usize / 10) {
                let high = (0.6 * (TAU * 0.46 * i as f64).sin()) as f32;
                let mut frame = [high, high * 0.25];
                dry.push(frame);
                chain.process(&mut frame, &[]);
                assert_eq!(frame[0] * 0.25, frame[1]);
                wet.push(frame);
            }
            let last = rate as usize / 20;
            let level = rms(&wet[last..]) / rms(&dry[last..]);
            assert!(
                level.is_finite() && level > 0.39 && level < 0.76,
                "{rate}: {level}"
            );
        }
    }

    #[test]
    fn delay_alternates_and_reverb_generates_a_stereo_tail() -> Result<()> {
        let mut echo = vec![[0.; 2]; 2400];
        echo[0][0] = 1.;
        let mut room = echo.clone();
        process(
            &[Effect::Delay {
                bypass: false,
                time_ms: 10.,
                feedback: 0.5,
                damping_hz: 10_000.,
                mix: 1.,
                ping_pong: true,
            }],
            &mut echo,
            || Ok(()),
        )?;
        assert_eq!(echo[479], [0.; 2]);
        assert!(echo[480][0] > 0.6 && echo[480][1] == 0.);
        assert!(echo[960][1] > 0.2);
        process(
            &[Effect::Reverb {
                bypass: false,
                decay_s: 2.,
                damping_hz: 5_000.,
                mix: 1.,
            }],
            &mut room,
            || Ok(()),
        )?;
        assert!(room[1429][0] > 0.01 && room[1429][1] == 0.);
        assert!(room[1472][1] > 0.003);
        Ok(())
    }

    #[test]
    fn limiter_respects_peak_and_stem_headroom_without_changing_sample_positions() -> Result<()> {
        let limiter = Limiter {
            ceiling_db: -3.,
            input_db: 3.,
            lookahead_ms: 2.,
            release_ms: 80.,
            peak_mode: PeakMode::SamplePeak,
        };
        let mut mix = vec![[0.2, 0.1]; 600];
        let mut gains = vec![0.2; 600];
        mix[220] = [2., -1.];
        gains[320] = 3.;
        let reduction = limiter.gain_envelope(&mix, &mut gains, || Ok(()))?;
        assert!(reduction > 9.);
        assert!((gains[100] - db_gain(3.) as f32).abs() < 1e-5);
        assert!(gains[219] < gains[100] && gains[221] >= gains[220]);
        assert!((2. * gains[220] - db_gain(-3.) as f32).abs() < 1e-5);
        assert!(3. * gains[320] <= 1.);
        Ok(())
    }
}
