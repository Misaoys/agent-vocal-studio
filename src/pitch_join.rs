use crate::{
    engine::{curve_cents, curve_value},
    model::{Note, Project},
    timeline::Timeline,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub(crate) struct Tuning {
    pitch_points: Vec<[f64; 2]>,
    pitch_blend: Vec<[f64; 2]>,
}

impl From<&Note> for Tuning {
    fn from(note: &Note) -> Self {
        Self {
            pitch_points: note.pitch_points.clone(),
            pitch_blend: note.pitch_blend.clone(),
        }
    }
}

pub(crate) fn plan(project: &Project, notes: &[&Note], request: &Value) -> Result<[Tuning; 2]> {
    let [left, right] = notes else {
        anyhow::bail!("请选择两个首尾相接的歌声音符");
    };
    let boundary = left.start + left.duration;
    ensure!(
        (right.start - boundary).abs() <= 1e-7,
        "两个歌声音符必须首尾相接"
    );
    let clock = project.timeline();
    let join = clock.seconds(boundary);
    let lead = interval(&request["lead_ms"], join - clock.seconds(left.start))?;
    let trail = interval(
        &request["trail_ms"],
        clock.seconds(right.start + right.duration) - join,
    )?;
    ensure!(lead + trail > 0., "至少一侧需要有滑音时长");
    let start = join - lead;
    let end = join + trail;
    ensure!(
        (lead == 0. || start < join) && (trail == 0. || end > join),
        "滑音时长太短"
    );
    let anchor_left = clock.beat(start) - left.start;
    let anchor_right = clock.beat(end) - right.start;
    let from = f64::from(left.pitch) * 100. + curve_cents(left, anchor_left);
    let to = f64::from(right.pitch) * 100. + curve_cents(right, anchor_right);
    let target = |seconds: f64| {
        let x = ((seconds - start) / (end - start)).clamp(0., 1.);
        from + (to - from) * x * x * (3. - 2. * x)
    };
    let mut a = Tuning::from(*left);
    if lead > 0. {
        let start_beat = anchor_left.clamp(0., left.duration);
        a.pitch_points.retain(|point| point[0] < start_beat);
        a.pitch_points.extend(samples(
            &clock,
            left,
            [start, join],
            [start_beat, left.duration],
            target,
        ));
        let ramp = clock.beat(start + (lead * 0.5).min(0.02)) - left.start;
        a.pitch_blend.retain(|point| point[0] < start_beat);
        a.pitch_blend.extend([
            [start_beat, curve_value(&left.pitch_blend, start_beat)],
            [ramp, 1.],
            [left.duration, 1.],
        ]);
    }
    let mut b = Tuning::from(*right);
    if trail > 0. {
        let end_beat = anchor_right.clamp(0., right.duration);
        b.pitch_points.retain(|point| point[0] > end_beat);
        let mut window = samples(&clock, right, [join, end], [0., end_beat], target);
        window.append(&mut b.pitch_points);
        b.pitch_points = window;
        let ramp = clock.beat(end - (trail * 0.5).min(0.02)) - right.start;
        b.pitch_blend.retain(|point| point[0] > end_beat);
        let mut blend = vec![
            [0., 1.],
            [ramp, 1.],
            [end_beat, curve_value(&right.pitch_blend, end_beat)],
        ];
        blend.append(&mut b.pitch_blend);
        b.pitch_blend = blend;
    }
    ensure!(
        a.pitch_points
            .iter()
            .chain(&b.pitch_points)
            .all(|point| point[1].abs() <= 2400.),
        "衔接超出音高曲线的 ±2400 音分范围"
    );
    Ok([a, b])
}

pub(crate) fn edge(project: &Project, notes: &[&Note], request: &Value) -> Result<Tuning> {
    let [note] = notes else {
        anyhow::bail!("请选择一个歌声音符");
    };
    let start = match crate::model::field(request, "side")? {
        "start" => true,
        "end" => false,
        _ => anyhow::bail!("请选择起音或尾音"),
    };
    let ms = request["length_ms"].as_f64().context("请输入滑音时长")?;
    ensure!((20. ..=2000.).contains(&ms), "单音滑音时长为 20–2000 毫秒");
    let amount = request["offset_cents"].as_f64().context("请输入滑音音分")?;
    ensure!(
        amount != 0. && (-600. ..=600.).contains(&amount),
        "单音滑音须为 ±600 音分内的非零数值"
    );
    let clock = project.timeline();
    let begin = clock.seconds(note.start);
    let end = clock.seconds(note.start + note.duration);
    let length = interval(&request["length_ms"], end - begin)?;
    let time = if start {
        [begin, begin + length]
    } else {
        [end - length, end]
    };
    let relative = |seconds| (clock.beat(seconds) - note.start).clamp(0., note.duration);
    let beats = if start {
        [0., relative(time[1])]
    } else {
        [relative(time[0]), note.duration]
    };
    let steps = (length / 0.01).ceil() as usize;
    let mut positions: Vec<_> = note.pitch_points.iter().map(|point| point[0]).collect();
    positions.extend(beats);
    positions
        .extend((1..steps).map(|step| relative(time[0] + length * step as f64 / steps as f64)));
    positions.sort_unstable_by(f64::total_cmp);
    positions.dedup_by(|a, b| a == b);
    let pitch_points = positions
        .into_iter()
        .map(|beat| {
            let cents = curve_cents(note, beat);
            if beat < beats[0] || beat > beats[1] {
                return [beat, cents];
            }
            let phase = if beat == beats[0] {
                0.
            } else if beat == beats[1] {
                1.
            } else {
                ((clock.seconds(note.start + beat) - time[0]) / length).clamp(0., 1.)
            };
            let x = if start { 1. - phase } else { phase };
            [beat, cents + amount * x * x * (3. - 2. * x)]
        })
        .collect::<Vec<_>>();
    ensure!(
        pitch_points.iter().all(|point| point[1].abs() <= 2400.),
        "滑音叠加后超出 ±2400 音分"
    );
    Ok(Tuning {
        pitch_points,
        pitch_blend: note.pitch_blend.clone(),
    })
}

fn interval(value: &Value, max: f64) -> Result<f64> {
    let ms = value.as_f64().context("滑音时长需要毫秒数")?;
    ensure!((0. ..=4000.).contains(&ms), "滑音时长为 0–4000 毫秒");
    ensure!(
        ms / 1000. <= max + 1e-8,
        "滑音时长超过音符时值（最长 {:.1} ms）",
        max * 1000.
    );
    Ok((ms / 1000.).min(max))
}

fn samples(
    clock: &Timeline,
    note: &Note,
    time: [f64; 2],
    beats: [f64; 2],
    target: impl Fn(f64) -> f64,
) -> Vec<[f64; 2]> {
    let steps = ((time[1] - time[0]) / 0.01).ceil().max(1.) as usize;
    let base = f64::from(note.pitch) * 100.;
    (0..=steps)
        .map(|index| {
            let seconds = time[0] + (time[1] - time[0]) * index as f64 / steps as f64;
            let beat = match index {
                0 => beats[0],
                i if i == steps => beats[1],
                _ => (clock.beat(seconds) - note.start).clamp(beats[0], beats[1]),
            };
            [beat, target(seconds) - base]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Track;
    use serde_json::json;

    #[test]
    fn edge_layers_over_existing_curve_in_wall_time() -> Result<()> {
        let note = Note::from_value(&json!({
            "start": 1., "duration": 2., "pitch_points": [[0,14],[1,38],[2,4]],
            "pitch_blend": [[0,0.1],[2,0.35]]
        }))?;
        let mut voice = Track::new("Vocal");
        voice.notes.push(note.clone());
        let project: Project = serde_json::from_value(json!({
            "id":"edge_test","title":"technical test","format":"agent-vocal/5","revision":1,
            "bpm":100,"meter":[4,4],"tempo_map":[{"beat":3,"bpm":160,"curve":"linear"}],"tracks":[voice]
        }))?;
        let clock = project.timeline();
        let first = clock.seconds(note.start);
        let last = clock.seconds(note.start + note.duration);
        let tuning = |side| {
            edge(
                &project,
                &[&note],
                &json!({"side":side,"length_ms":200,"offset_cents":-120}),
            )
        };
        let start = tuning("start")?;
        let end = tuning("end")?;
        assert_eq!(start.pitch_points.first(), Some(&[0., -106.]));
        assert_eq!(end.pitch_points.last(), Some(&[2., -116.]));
        assert_eq!(start.pitch_blend, note.pitch_blend);
        assert_eq!(end.pitch_blend, note.pitch_blend);
        for (curve, half) in [
            (&start.pitch_points, first + 0.1),
            (&end.pitch_points, last - 0.1),
        ] {
            let beat = clock.beat(half) - note.start;
            assert!((curve_value(curve, beat) - curve_cents(&note, beat) + 60.).abs() < 0.01);
            assert_eq!(curve_value(curve, 1.), 38.);
        }
        assert!(edge(
            &project,
            &[&note],
            &json!({"side":"end","length_ms":1500,"offset_cents":-120})
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn join_uses_wall_time_and_preserves_curves_outside_the_requested_interval() -> Result<()> {
        let left = Note::from_value(&json!({
            "pitch": 60, "start": 0., "duration": 2., "pitch_points":[[0,12],[1,18],[2,-8]]
        }))?;
        let right = Note::from_value(&json!({
            "pitch": 67, "start": 2., "duration": 2., "pitch_points":[[0,-22],[1,17],[2,33]]
        }))?;
        let mut track = Track::new("主唱");
        track.notes.extend([left.clone(), right.clone()]);
        let project: Project = serde_json::from_value(json!({
            "id":"join_test", "title":"technical_test", "format":"agent-vocal/2", "revision":1,
            "bpm":72, "meter":[4,4], "tracks":[track],
            "tempo_map":[{"beat":3,"bpm":170,"curve":"linear"}]
        }))?;
        let clock = project.timeline();
        let join = clock.seconds(2.);
        let [a, b] = plan(
            &project,
            &[&left, &right],
            &json!({"lead_ms":90,"trail_ms":130}),
        )?;
        assert_eq!(a.pitch_blend.first().unwrap()[1], 0.);
        assert_eq!(a.pitch_blend.last().unwrap()[1], 1.);
        assert_eq!(b.pitch_blend.first().unwrap()[1], 1.);
        assert_eq!(b.pitch_blend.last().unwrap()[1], 0.);
        let [a, b] = [a.pitch_points, b.pitch_points];
        let left_edge = a.iter().find(|point| point[0] > 1.).unwrap();
        let right_edge = b.iter().rev().find(|point| point[0] < 1.).unwrap();
        assert!((clock.seconds(left_edge[0]) - (join - 0.09)).abs() < 1e-10);
        assert!((clock.seconds(2. + right_edge[0]) - (join + 0.13)).abs() < 1e-10);
        assert!((6000. + a.last().unwrap()[1] - 6700. - b[0][1]).abs() < 1e-10);
        assert_eq!(&a[..2], &left.pitch_points[..2]);
        assert_eq!(&b[b.len() - 2..], &right.pitch_points[1..]);
        let [original, after] = plan(
            &project,
            &[&left, &right],
            &json!({"lead_ms":0,"trail_ms":120}),
        )?;
        assert_eq!(original.pitch_points, left.pitch_points);
        assert_eq!(
            6700. + after.pitch_points[0][1],
            6000. + left.pitch_points.last().unwrap()[1]
        );
        let mut separated = right.clone();
        separated.start += 0.1;
        assert!(plan(
            &project,
            &[&left, &separated],
            &json!({"lead_ms":90,"trail_ms":130})
        )
        .is_err());
        Ok(())
    }
}
