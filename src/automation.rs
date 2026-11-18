use crate::{effects::db_gain, model::Project, timeline::Timeline};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const MAX_POINTS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Point {
    pub beat: f64,
    pub db: f64,
}

pub(crate) fn validate(points: &[Point]) -> Result<()> {
    ensure!(
        points.len() <= MAX_POINTS,
        "Maximum 2000 gain automation points per track"
    );
    ensure!(
        points
            .iter()
            .all(|point| (0.0..=50_000.).contains(&point.beat) && (-60.0..=12.).contains(&point.db))
            && points.windows(2).all(|span| span[0].beat < span[1].beat),
        "Gain automation needs increasing beats from 0 to 50000 and levels from -60 to 12 dB"
    );
    Ok(())
}

pub(crate) fn beat_range(value: &Value) -> Result<[f64; 2]> {
    let [start, end]: [f64; 2] =
        serde_json::from_value(value.clone()).context("Provide [start, end] beats")?;
    ensure!(
        start >= 0. && start < end && end <= 50_000.,
        "Invalid automation beat range"
    );
    Ok([start, end])
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Envelope(Vec<(f64, f64)>);

impl Envelope {
    pub(crate) fn compile(points: &[Point], clock: &Timeline) -> Self {
        Self(
            points
                .iter()
                .map(|p| (clock.seconds(p.beat) * 48_000., db_gain(p.db)))
                .collect(),
        )
    }

    pub(crate) fn cursor(&self) -> Cursor<'_> {
        Cursor {
            envelope: self,
            upper: 0,
        }
    }

    fn level_at(&self, beat: f64, clock: &Timeline) -> f64 {
        (20. * self.cursor().at(clock.seconds(beat) * 48_000.).log10()).clamp(-60., 12.)
    }
}

pub(crate) struct Cursor<'a> {
    envelope: &'a Envelope,
    upper: usize,
}

impl Cursor<'_> {
    pub(crate) fn at(&mut self, frame: f64) -> f64 {
        self.envelope.at(frame, &mut self.upper)
    }
}

impl Envelope {
    pub(crate) fn at(&self, frame: f64, upper: &mut usize) -> f64 {
        let points = &self.0;
        let Some(first) = points.first() else {
            return 1.;
        };
        let index = *upper;
        if index > points.len()
            || index > 0 && frame < points[index - 1].0
            || index < points.len() && frame >= points[index].0
        {
            *upper = points.partition_point(|p| p.0 <= frame);
        }
        if *upper == 0 {
            return first.1;
        }
        let low = points[*upper - 1];
        points.get(*upper).map_or(low.1, |high| {
            let fraction = (frame - low.0) / (high.0 - low.0);
            low.1 + (high.1 - low.1) * fraction
        })
    }
}

/// Pin both endpoints to the old curve, so the same interpolation remains outside the edit.
pub(crate) fn replace_range(
    points: &[Point],
    new: &[Point],
    range: [f64; 2],
    clock: &Timeline,
) -> Result<Vec<Point>> {
    validate(new)?;
    let [start, end] = range;
    ensure!(
        new.iter()
            .all(|point| point.beat > start && point.beat < end),
        "Automation points must be strictly inside the beat range"
    );
    let before = Envelope::compile(points, clock);
    let mut result = Vec::with_capacity(points.len() + new.len() + 2);
    result.extend(
        points
            .iter()
            .copied()
            .take_while(|point| point.beat < start),
    );
    result.push(Point {
        beat: start,
        db: before.level_at(start, clock),
    });
    result.extend_from_slice(new);
    result.push(Point {
        beat: end,
        db: before.level_at(end, clock),
    });
    result.extend(points.iter().copied().skip_while(|point| point.beat <= end));
    validate(&result)?;
    Ok(result)
}

pub(crate) fn query(project: &Project, track_id: &str, range: Option<[f64; 2]>) -> Result<Value> {
    let track = project
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .context("Unknown automation track")?;
    let clock = project.timeline();
    let points = &track.gain_automation;
    let mut value = json!({"track_id":track_id,"project_id":project.id,"revision":project.revision,"fader_db":track.gain_db});
    if let Some([start, end]) = range {
        let envelope = Envelope::compile(points, &clock);
        value["beat_range"] = json!([start, end]);
        value["boundary_db"] = json!([
            envelope.level_at(start, &clock),
            envelope.level_at(end, &clock)
        ]);
        value["points"] = json!(points
            .iter()
            .filter(|p| p.beat >= start && p.beat <= end)
            .collect::<Vec<_>>());
    } else {
        value["points"] = json!(points);
    }
    Ok(value)
}
