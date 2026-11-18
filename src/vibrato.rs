use crate::{
    model::{Note, Vibrato},
    timeline::Timeline,
};
use serde_json::{json, Value};
use std::f64::consts::TAU;

pub(crate) struct Envelope {
    start: f64,
    end: f64,
    fade_in: f64,
    fade_out: f64,
    depth: f64,
    rate: f64,
}

pub(crate) fn region(note: &Note, clock: &Timeline) -> [f64; 2] {
    [note.vibrato.start, note.vibrato.end]
        .map(|position| clock.seconds(note.start + note.duration * position))
}

impl Envelope {
    pub(crate) fn new(note: &Note, clock: &Timeline) -> Self {
        let settings = &note.vibrato;
        let [start, end] = region(note, clock);
        Self {
            start,
            // Older projects let the final cycle follow a vowel retimed beyond the score boundary.
            end: if settings.has_envelope() {
                end
            } else {
                f64::INFINITY
            },
            fade_in: settings.fade_in_ms / 1000.,
            fade_out: settings.fade_out_ms / 1000.,
            depth: if end > start { settings.depth } else { 0. },
            rate: settings.rate,
        }
    }

    pub(crate) fn cents(&self, time: f64) -> f64 {
        if self.depth == 0. || time <= self.start || time >= self.end {
            return 0.;
        }
        let elapsed = time - self.start;
        let rise = if self.fade_in > 0. {
            elapsed / self.fade_in
        } else {
            1.
        };
        let fall = if self.fade_out > 0. {
            (self.end - time) / self.fade_out
        } else {
            1.
        };
        self.depth * rise.min(fall).min(1.) * (TAU * self.rate * elapsed).sin()
    }
}

pub(crate) fn describe(note: &Note, clock: &Timeline) -> Value {
    let settings = &note.vibrato;
    let range = region(note, clock);
    let from = clock.seconds(note.start);
    let length = range[1] - range[0];
    let ramps = (settings.fade_in_ms + settings.fade_out_ms) / 1000.;
    let envelope_peak = if ramps > 0. {
        (length / ramps).min(1.)
    } else {
        1.
    };
    json!({
        "note_offset_ms":range.map(|time| (time - from) * 1000.),
        "beat_range":[note.start + note.duration * settings.start,
            note.start + note.duration * settings.end],
        "cycle_count": if settings.depth > 0. { length * settings.rate } else { 0. },
        "depth_limit_cents": if length > 0. { settings.depth * envelope_peak } else { 0. },
    })
}

pub(crate) fn settings(value: &Value) -> anyhow::Result<Vibrato> {
    let options = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("请选择颤音参数"))?;
    anyhow::ensure!(
        options.keys().all(
            |key| ["depth", "rate", "start", "end", "fade_in_ms", "fade_out_ms"]
                .contains(&key.as_str())
        ),
        "颤音参数含有未知字段"
    );
    let settings: Vibrato = serde_json::from_value(value.clone())?;
    settings.validate()?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{Curve, TempoPoint};

    #[test]
    fn bounded_envelope_uses_wall_time_and_legacy_timing_still_matches() -> anyhow::Result<()> {
        let clock = Timeline::new(
            120.,
            [4, 4],
            &[TempoPoint {
                beat: 2.,
                bpm: 60.,
                curve: Curve::Step,
            }],
            &[],
        );
        let mut note = Note::from_value(&json!({
            "start":1., "duration":3., "pitch":69,
            "vibrato":{"depth":48., "rate":5., "start":0.25}
        }))?;
        let legacy = Envelope::new(&note, &clock);
        let start = clock.seconds(1.75);
        assert_eq!(
            serde_json::to_value(&note.vibrato)?,
            json!({"depth":48., "rate":5., "start":0.25})
        );
        for time in [
            start - 0.001,
            start + 0.05,
            start + 0.25,
            clock.seconds(4.) + 0.023,
        ] {
            let elapsed = (time - start).max(0.);
            let expected = 48. * (elapsed / 0.08).clamp(0., 1.) * (TAU * 5. * elapsed).sin();
            assert!((legacy.cents(time) - expected).abs() < 1e-10);
        }
        note.vibrato.end = 0.75;
        note.vibrato.fade_in_ms = 100.;
        note.vibrato.fade_out_ms = 200.;
        let end = clock.seconds(3.25);
        let envelope = Envelope::new(&note, &clock);
        assert_eq!(region(&note, &clock), [start, end]);
        for (time, value) in [
            (start - 0.001, 0.),
            (start + 0.05, 24.),
            (start + 0.25, 48.),
            (end - 0.125, 30.),
            (end, 0.),
            (end + 0.03, 0.),
        ] {
            assert!((envelope.cents(time) - value).abs() < 1e-9);
        }
        let report = describe(&note, &clock);
        assert_eq!(report["note_offset_ms"], json!([375., 1750.]));
        assert_eq!(report["cycle_count"], json!(6.875));
        note.vibrato.fade_in_ms = 1000.;
        note.vibrato.fade_out_ms = 1000.;
        assert_eq!(describe(&note, &clock)["depth_limit_cents"], json!(33.));
        Ok(())
    }
}
