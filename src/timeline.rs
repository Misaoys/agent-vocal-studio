use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    #[default]
    Step,
    Linear,
}
/// Curve describes the transition FROM the preceding tempo TO this point.
/// The implicit point at beat zero is Project.bpm, so there is no duplicated base.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoPoint {
    pub beat: f64,
    pub bpm: f64,
    #[serde(default)]
    pub curve: Curve,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterPoint {
    pub beat: f64,
    pub meter: [u32; 2],
}
#[derive(Clone, Debug)]
struct Segment {
    beat: f64,
    seconds: f64,
    bpm: f64,
    slope: f64,
}
#[derive(Clone, Debug)]
struct MeterSegment {
    beat: f64,
    bar: u64,
    meter: [u32; 2],
}
#[derive(Clone, Debug)]
pub struct Timeline {
    segments: Vec<Segment>,
    meters: Vec<MeterSegment>,
}
pub fn bar_length(meter: [u32; 2]) -> f64 {
    meter[0] as f64 * 4. / meter[1] as f64
}

#[derive(Serialize)]
pub(crate) struct Pulse {
    pub beat: f64,
    pub frame: i64,
    pub accent: bool,
}
fn integral(delta: f64, bpm: f64, slope: f64) -> f64 {
    if slope.abs() < 1e-10 {
        60. * delta / bpm
    } else {
        60. / slope * (slope * delta / bpm).ln_1p()
    }
}
fn inverse(seconds: f64, bpm: f64, slope: f64) -> f64 {
    if slope.abs() < 1e-10 {
        seconds * bpm / 60.
    } else {
        bpm / slope * (seconds * slope / 60.).exp_m1()
    }
}
pub fn validate(
    bpm: f64,
    meter: [u32; 2],
    tempos: &[TempoPoint],
    meters: &[MeterPoint],
) -> Result<()> {
    ensure!(
        bpm.is_finite() && (30. ..=300.).contains(&bpm),
        "BPM must be 30..300"
    );
    let valid_meter =
        |m: [u32; 2]| (1..=32).contains(&m[0]) && [1, 2, 4, 8, 16, 32].contains(&m[1]);
    ensure!(valid_meter(meter), "Invalid time signature");
    ensure!(
        tempos.len() <= 20000 && meters.len() <= 1024,
        "Timeline exceeds event budget"
    );
    let mut previous = 0.;
    for p in tempos {
        ensure!(
            p.beat.is_finite() && p.beat > previous && p.beat <= 50000.,
            "Tempo points must have strictly increasing positive quarter-note beats"
        );
        ensure!(
            p.bpm.is_finite() && (30. ..=300.).contains(&p.bpm),
            "Tempo BPM must be 30..300"
        );
        previous = p.beat;
    }
    let mut previous = 0.;
    for p in meters {
        ensure!(
            p.beat.is_finite() && p.beat > previous && p.beat <= 50000. && valid_meter(p.meter),
            "Invalid meter change"
        );
        // A change starts a new bar; an incomplete preceding bar is preserved.
        previous = p.beat;
    }
    Ok(())
}
impl Timeline {
    /// Call only with validated model data. A single compiled map is reused per operation.
    pub fn new(bpm: f64, meter: [u32; 2], tempos: &[TempoPoint], meters: &[MeterPoint]) -> Self {
        let mut segments = vec![];
        let mut beat = 0.;
        let mut seconds = 0.;
        let mut start_bpm = bpm;
        for p in tempos {
            let slope = if p.curve == Curve::Linear {
                (p.bpm - start_bpm) / (p.beat - beat)
            } else {
                0.
            };
            segments.push(Segment {
                beat,
                seconds,
                bpm: start_bpm,
                slope,
            });
            seconds += integral(p.beat - beat, start_bpm, slope);
            beat = p.beat;
            start_bpm = p.bpm;
        }
        segments.push(Segment {
            beat,
            seconds,
            bpm: start_bpm,
            slope: 0.,
        });
        let mut m = vec![MeterSegment {
            beat: 0.,
            bar: 1,
            meter,
        }];
        for p in meters {
            let prev = m.last().unwrap();
            let bar =
                prev.bar + ((p.beat - prev.beat) / bar_length(prev.meter) - 1e-9).ceil() as u64;
            m.push(MeterSegment {
                beat: p.beat,
                bar,
                meter: p.meter,
            });
        }
        Self {
            segments,
            meters: m,
        }
    }
    pub fn seconds(&self, beat: f64) -> f64 {
        let i = self
            .segments
            .partition_point(|s| s.beat <= beat)
            .saturating_sub(1);
        let s = &self.segments[i];
        s.seconds + integral((beat - s.beat).max(0.), s.bpm, s.slope)
    }
    pub fn beat(&self, seconds: f64) -> f64 {
        let i = self
            .segments
            .partition_point(|s| s.seconds <= seconds)
            .saturating_sub(1);
        let s = &self.segments[i];
        s.beat + inverse((seconds - s.seconds).max(0.), s.bpm, s.slope)
    }
    pub fn tempo(&self, beat: f64) -> f64 {
        let i = self
            .segments
            .partition_point(|s| s.beat <= beat)
            .saturating_sub(1);
        let s = &self.segments[i];
        s.bpm + s.slope * (beat - s.beat).max(0.)
    }
    pub(crate) fn constant_tempo(&self, start: f64, end: f64) -> bool {
        let index = self
            .segments
            .partition_point(|segment| segment.beat <= start)
            .saturating_sub(1);
        let bpm = self.segments[index].bpm;
        self.segments[index..]
            .iter()
            .take_while(|segment| segment.beat < end)
            .all(|segment| segment.slope.abs() < 1e-10 && segment.bpm == bpm)
    }
    pub fn bar_start(&self, beat: f64) -> (u64, f64, [u32; 2]) {
        let i = self
            .meters
            .partition_point(|m| m.beat <= beat + 1e-9)
            .saturating_sub(1);
        let m = &self.meters[i];
        let n = ((beat - m.beat).max(0.) / bar_length(m.meter) + 1e-9).floor();
        (m.bar + n as u64, m.beat + n * bar_length(m.meter), m.meter)
    }
    pub(crate) fn bar_bounds(&self, beat: f64) -> [f64; 2] {
        let (_, start, meter) = self.bar_start(beat);
        let end = start + bar_length(meter);
        let next = self.meters.partition_point(|m| m.beat <= start);
        [
            start,
            self.meters.get(next).map_or(end, |m| end.min(m.beat)),
        ]
    }
    pub fn position(&self, beat: f64) -> Value {
        let (bar, start, meter) = self.bar_start(beat);
        let unit = (beat - start).max(0.) * meter[1] as f64 / 4.;
        json!({"bar":bar,"beat":unit.floor() as u64+1,"fraction":unit-unit.floor(),"meter":meter})
    }
    pub fn query(&self, beat: f64) -> Value {
        json!({"quarter_beat":beat,"seconds":self.seconds(beat),"bpm":self.tempo(beat),"position":self.position(beat)})
    }

    pub(crate) fn seconds_with_preroll(&self, beat: f64) -> f64 {
        if beat < 0. {
            beat * 60. / self.segments[0].bpm
        } else {
            self.seconds(beat)
        }
    }

    pub(crate) fn pulses(&self, [start, end]: [f64; 2]) -> Vec<Pulse> {
        let mut pulses = Vec::new();
        let rate = f64::from(crate::clips::RATE);
        for (index, segment) in self.meters.iter().enumerate() {
            let [numerator, denominator] = segment.meter;
            let compound = numerator >= 6 && numerator % 3 == 0 && denominator >= 8;
            let grouping = if compound { 3 } else { 1 };
            let interval = 4. * f64::from(grouping) / f64::from(denominator);
            let stop = self
                .meters
                .get(index + 1)
                .map_or(end, |next| end.min(next.beat));
            let first = if index == 0 {
                start
            } else {
                start.max(segment.beat)
            };
            if first >= stop {
                continue;
            }
            let mut pulse = ((first - segment.beat) / interval - 1e-9).ceil() as i64;
            loop {
                let beat = segment.beat + pulse as f64 * interval;
                if beat >= stop - 1e-9 {
                    break;
                }
                let seconds = self.seconds_with_preroll(beat);
                pulses.push(Pulse {
                    beat,
                    frame: (seconds * rate).round() as i64,
                    accent: pulse % i64::from(numerator / grouping) == 0,
                });
                pulse += 1;
            }
        }
        pulses
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stepped_and_linear_time_maps_invert_and_count_compound_bars() -> Result<()> {
        let tempos = vec![
            TempoPoint {
                beat: 4.,
                bpm: 60.,
                curve: Curve::Step,
            },
            TempoPoint {
                beat: 8.,
                bpm: 120.,
                curve: Curve::Linear,
            },
        ];
        let meters = vec![
            MeterPoint {
                beat: 8.,
                meter: [6, 8],
            },
            MeterPoint {
                beat: 14.,
                meter: [3, 4],
            },
        ];
        validate(120., [4, 4], &tempos, &meters)?;
        let t = Timeline::new(120., [4, 4], &tempos, &meters);
        assert!((t.seconds(4.) - 2.).abs() < 1e-12);
        assert!((t.seconds(8.) - (2. + 4. * 2f64.ln())).abs() < 1e-12);
        for i in 0..20001 {
            let beat = i as f64 / 997.;
            assert!((t.beat(t.seconds(beat)) - beat).abs() < 1e-10);
        }
        assert_eq!(t.position(8.)["bar"], 3);
        assert_eq!(t.position(8.5)["beat"], 2);
        assert_eq!(t.position(14.)["bar"], 5);
        assert!(validate(
            120.,
            [4, 4],
            &[],
            &[MeterPoint {
                beat: 7.,
                meter: [3, 3]
            }]
        )
        .is_err());
        let pickup = Timeline::new(
            120.,
            [4, 4],
            &[],
            &[MeterPoint {
                beat: 1.,
                meter: [3, 4],
            }],
        );
        assert_eq!(pickup.position(1.)["bar"], 2);
        assert!(validate(
            120.,
            [4, 4],
            &[TempoPoint {
                beat: 0.,
                bpm: 90.,
                curve: Curve::Step
            }],
            &[]
        )
        .is_err());
        // A falling ramp, a near-flat ramp and an explicit base tempo all invert.
        for end in [30., 119.9999999999, 300.] {
            let t = Timeline::new(
                120.,
                [4, 4],
                &[TempoPoint {
                    beat: 4.,
                    bpm: end,
                    curve: Curve::Linear,
                }],
                &[],
            );
            for beat in [0., 0.01, 2., 3.99, 4., 20.] {
                assert!((t.beat(t.seconds(beat)) - beat).abs() < 1e-8);
            }
        }
        Ok(())
    }
}
