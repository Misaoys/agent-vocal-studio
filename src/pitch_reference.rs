use crate::{
    clips::{ClipRef, RATE},
    engine::curve_value,
    model::{Project, Store},
    selection::{selected_vocal_notes, Selection},
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;

pub(crate) const MAX_NOTES: usize = 32;
const MAX_SECONDS: f64 = 15.;
const DECIMATION: usize = 4;
const ANALYSIS_RATE: usize = RATE as usize / DECIMATION;
const COMPARISON: usize = 200;
const MAX_LAG: usize = 160;
const WINDOW: usize = COMPARISON + MAX_LAG;
const STEP: usize = ANALYSIS_RATE / 100;

// YIN's cumulative mean normalized difference, with abstention for unreliable frames.
fn estimate(samples: &[[f32; 2]], channel: usize) -> Option<f64> {
    let energy = samples
        .iter()
        .map(|frame| frame[channel].powi(2))
        .sum::<f32>();
    if energy < samples.len() as f32 * 0.0015_f32.powi(2) {
        return None;
    }
    let mut difference = [0.; MAX_LAG + 1];
    let mut normalized = [1.; MAX_LAG + 1];
    let mut total = 0.;
    for lag in 1..=MAX_LAG {
        let distance = samples[..COMPARISON]
            .iter()
            .zip(&samples[lag..])
            .map(|(a, b)| (a[channel] - b[channel]).powi(2))
            .sum::<f32>();
        difference[lag] = f64::from(distance);
        total += difference[lag];
        if total > 0. {
            normalized[lag] = difference[lag] * lag as f64 / total;
        }
    }
    let lag = (8..MAX_LAG).find(|&i| {
        normalized[i] < 0.15
            && normalized[i] < normalized[i - 1]
            && normalized[i] <= normalized[i + 1]
    })?;
    let [left, middle, right] = [difference[lag - 1], difference[lag], difference[lag + 1]];
    let curvature = left - 2. * middle + right;
    let delta = if curvature > 0. {
        (0.5 * (left - right) / curvature).clamp(-0.5, 0.5)
    } else {
        0.
    };
    Some(ANALYSIS_RATE as f64 / (lag as f64 + delta))
}

#[derive(Serialize)]
struct VoicedNote<'a> {
    note_id: &'a str,
    beat_range: [f64; 2],
    nominal_midi: u8,
    analyzed_frames: usize,
    voiced_frames: usize,
    matched_pitch_frames: usize,
    first_offset_cents: Option<f64>,
    median_offset_cents: Option<f64>,
    tail_offset_cents: Option<f64>,
    offset_percentiles_cents: Option<[f64; 2]>,
    voiced_within_note_ms: Option<[f64; 2]>,
}

fn median_offset(values: &[f64]) -> Option<f64> {
    if values.len() < 3 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    Some(((sorted[(sorted.len() - 1) / 2] + sorted[middle]) * 5.).round() / 10.)
}

fn inspect_voice_notes<'a>(
    project: &'a Project,
    track_id: &str,
    [start, end]: [f64; 2],
    first: usize,
    samples: &[[f32; 2]],
    control: &crate::jobs::Control,
) -> Result<(Vec<VoicedNote<'a>>, bool)> {
    let track = project
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .context("找不到实音分析歌声轨")?;
    let clock = project.timeline();
    let decimated = samples
        .as_chunks::<DECIMATION>()
        .0
        .iter()
        .map(|frames| [frames.iter().map(|frame| frame[0]).sum::<f32>() / DECIMATION as f32; 2])
        .collect::<Vec<_>>();
    let mut reports = Vec::new();
    let mut intersecting = track
        .notes
        .iter()
        .filter(|note| note.start < end && note.start + note.duration > start);
    for note in intersecting.by_ref().take(MAX_NOTES) {
        control.check()?;
        let beat_range = [note.start.max(start), (note.start + note.duration).min(end)];
        let frame =
            |beat| ((clock.seconds(beat) * f64::from(RATE)).round() as usize).saturating_sub(first);
        let begin = frame(beat_range[0])
            .div_ceil(DECIMATION)
            .min(decimated.len());
        let finish = (frame(beat_range[1]) / DECIMATION)
            .min(decimated.len())
            .max(begin);
        let spaced = decimated[begin..finish].windows(WINDOW).step_by(STEP);
        let analyzed_frames = spaced.len();
        let nominal_hz = 440. * 2f64.powf((f64::from(note.pitch) - 69.) / 12.);
        let note_start = clock.seconds(note.start);
        let mut offsets = Vec::new();
        let mut voiced_range: Option<[f64; 2]> = None;
        let mut voiced_frames = 0;
        for (index, window) in spaced.enumerate() {
            if index % 16 == 0 {
                control.check()?
            }
            if let Some(hz) = estimate(window, 0) {
                voiced_frames += 1;
                let cents = 1200. * (hz / nominal_hz).log2();
                if cents.abs() <= 600. {
                    offsets.push(cents);
                    let midpoint = (begin + index * STEP) as f64
                        + 0.5 * (COMPARISON as f64 + ANALYSIS_RATE as f64 / hz);
                    let time = (first as f64 / f64::from(RATE) + midpoint / ANALYSIS_RATE as f64
                        - note_start)
                        .max(0.);
                    let ms = (time * 10000.).round() / 10.;
                    let range = voiced_range.get_or_insert([ms; 2]);
                    range[1] = ms;
                }
            }
        }
        let edge_frames = offsets.len().div_ceil(4).max(3).min(offsets.len());
        let percentiles = (offsets.len() >= 3).then(|| {
            let mut sorted = offsets.clone();
            sorted.sort_by(f64::total_cmp);
            [0.1, 0.9].map(|q| {
                let rank = (q * (sorted.len() - 1) as f64).round() as usize;
                (sorted[rank] * 10.).round() / 10.
            })
        });
        reports.push(VoicedNote {
            note_id: &note.id,
            beat_range,
            nominal_midi: note.pitch,
            analyzed_frames,
            voiced_frames,
            matched_pitch_frames: offsets.len(),
            first_offset_cents: median_offset(&offsets[..edge_frames]),
            median_offset_cents: median_offset(&offsets),
            tail_offset_cents: median_offset(&offsets[offsets.len() - edge_frames..]),
            offset_percentiles_cents: percentiles,
            voiced_within_note_ms: voiced_range,
        });
    }
    Ok((reports, intersecting.next().is_some()))
}

pub(crate) fn inspect_drafts(
    projects: [&Project; 2],
    track_id: &str,
    range: [f64; 2],
    first_frames: [usize; 2],
    samples: [&[[f32; 2]]; 2],
    control: &crate::jobs::Control,
) -> Result<Value> {
    let (a, truncated_a) = inspect_voice_notes(
        projects[0],
        track_id,
        range,
        first_frames[0],
        samples[0],
        control,
    )?;
    let (b, truncated_b) = inspect_voice_notes(
        projects[1],
        track_id,
        range,
        first_frames[1],
        samples[1],
        control,
    )?;
    let change = b
        .iter()
        .filter_map(|candidate| {
            let reference = a.iter().find(|note| note.note_id == candidate.note_id)?;
            let options = [
                (reference.first_offset_cents, candidate.first_offset_cents),
                (reference.median_offset_cents, candidate.median_offset_cents),
                (reference.tail_offset_cents, candidate.tail_offset_cents),
            ];
            let cents = options
                .into_iter()
                .filter_map(|(a, b)| a.zip(b).map(|(a, b)| b - a))
                .max_by(|x, y| x.abs().total_cmp(&y.abs()))?;
            (cents.abs() >= 20.).then_some((candidate, cents))
        })
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()));
    let suggested = change.map(|(candidate, cents)| json!({"note_id":candidate.note_id,
        "delta_cents":(cents*10.).round()/10.,
        "beat_range":[(candidate.beat_range[0]-0.125).max(range[0]),(candidate.beat_range[1]+0.125).min(range[1])]}));
    Ok(json!({"notes_ab":[a,b],"truncated_ab":[truncated_a,truncated_b],"suggested":suggested}))
}

fn splice(
    original: &[[f64; 2]],
    anchors: [f64; 2],
    points: impl Iterator<Item = [f64; 2]>,
) -> Vec<[f64; 2]> {
    let [start, end] = anchors;
    original
        .iter()
        .copied()
        .take_while(|point| point[0] < start)
        .chain([[start, curve_value(original, start)]])
        .chain(points)
        .chain([[end, curve_value(original, end)]])
        .chain(original.iter().copied().skip_while(|point| point[0] <= end))
        .collect()
}

pub(crate) fn preview(
    root: &Path,
    project: &Project,
    selection: &Selection,
    reference: &ClipRef,
) -> Result<Value> {
    ensure!(
        selection.project_id == project.id && selection.revision == project.revision,
        "REVISION_CONFLICT: selection expired; resolve again"
    );
    let notes = selected_vocal_notes(project, selection)?;
    ensure!(
        (1..=MAX_NOTES).contains(&notes.len()),
        "每次选择同轨的 1–32 个歌声音符"
    );
    let first = notes.first().unwrap().start;
    let last = notes
        .iter()
        .map(|n| n.start + n.duration)
        .fold(first, f64::max);
    let clock = project.timeline();
    ensure!(
        clock.seconds(last) - clock.seconds(first) <= MAX_SECONDS,
        "参考调声选区最多 15 秒"
    );
    let (_, clip) = reference.resolve(project)?;
    let [begin, end] = clip.recording_window(&clock, [first, last])?;
    let mut source = clip.clone();
    source.offset += begin;
    source.frames = end - begin;
    let origin = (clip.start_frame(&clock) + i64::from(begin)) as f64 / f64::from(RATE);
    let mut frames = Vec::with_capacity(source.frames as usize / DECIMATION);
    let mut group = [0.; 2];
    let mut energy = [0.; 2];
    crate::audio::visit_clip(root, &source, |index, frame| {
        for lane in 0..2 {
            group[lane] += frame[lane] / DECIMATION as f32;
        }
        if index as usize % DECIMATION == DECIMATION - 1 {
            frames.push(group);
            for lane in 0..2 {
                energy[lane] += f64::from(group[lane]).powi(2);
            }
            group = [0.; 2];
        }
        Ok(())
    })?;
    let channel = usize::from(energy[1] > energy[0]);
    let track = &selection.notes[0].0;
    let mut operations = vec![];
    let mut evidence = vec![];
    for note in notes {
        let start = clock.seconds(note.start);
        let end = clock.seconds(note.start + note.duration);
        let frame = |seconds: f64| {
            (((seconds - origin) * ANALYSIS_RATE as f64).round() as usize).min(frames.len())
        };
        let samples = &frames[frame(start)..frame(end)];
        let base_hz = 440. * 2f64.powf((f64::from(note.pitch) - 69.) / 12.);
        let mut observed = vec![];
        for (i, window) in samples.windows(WINDOW).step_by(STEP).enumerate() {
            let hz = estimate(window, channel);
            observed.push(hz.and_then(|hz| {
                let cents = 1200. * (hz / base_hz).log2();
                let midpoint = (frame(start) + i * STEP) as f64
                    + 0.5 * (COMPARISON as f64 + ANALYSIS_RATE as f64 / hz);
                (cents.abs() <= 300.).then_some([origin + midpoint / ANALYSIS_RATE as f64, cents])
            }));
        }
        // Use one contiguous voiced span. Never bridge a gap or a mismatching pitch.
        let run = observed
            .split(Option::is_none)
            .max_by_key(|run| run.len())
            .unwrap_or(&[]);
        if run.len() < 3 || run.len() * 3 < observed.len() {
            evidence.push(json!({"note_id":note.id,"status":"insufficient_voicing","accepted_frames":run.len()}));
            continue;
        }
        let span = [
            run.first().context("Missing pitch points")?.unwrap()[0],
            run.last().context("Missing pitch points")?.unwrap()[0],
        ];
        let relative = |time: f64| (clock.beat(time) - note.start).clamp(0., note.duration);
        let anchors = [
            relative((span[0] - 0.015).max(start)),
            relative((span[1] + 0.015).min(end)),
        ];
        let vibrato = crate::vibrato::Envelope::new(note, &clock);
        let measured = run.iter().flatten().map(|&[time, offset]| {
            let beat = relative(time);
            [beat, (offset - vibrato.cents(time)).clamp(-2400., 2400.)]
        });
        let points = splice(&note.pitch_points, anchors, measured);
        let blend = splice(
            &note.pitch_blend,
            anchors,
            [[relative(span[0]), 1.], [relative(span[1]), 1.]].into_iter(),
        );
        if points.len() > 2000 || blend.len() > 2000 {
            evidence.push(
                json!({"note_id":note.id,"status":"curve_capacity","accepted_frames":run.len()}),
            );
            continue;
        }
        operations.push(
            json!({"op":"update_note","track_id":track,"note_id":note.id,
            "values":{"pitch_points":points,"pitch_blend":blend}}),
        );
        evidence.push(json!({"note_id":note.id,"status":"ready","accepted_frames":run.len(),
            "voiced_seconds":span,"mean_offset_cents":run.iter().flatten().map(|p| p[1]).sum::<f64>() / run.len() as f64}));
    }
    let operations = json!(operations);
    let mut changes = vec![];
    if operations.as_array().is_some_and(|items| !items.is_empty()) {
        let (draft, _) = Store::preview_edit_with_refs(project, &operations)?;
        let old_track = project
            .tracks
            .iter()
            .find(|t| t.id == *track)
            .context("Missing vocal track")?;
        let new_track = draft
            .tracks
            .iter()
            .find(|t| t.id == *track)
            .context("Missing vocal track")?;
        for operation in operations.as_array().unwrap() {
            let id = &operation["note_id"];
            let before = old_track
                .notes
                .iter()
                .find(|n| *id == n.id)
                .context("Missing selected note")?;
            let after = new_track
                .notes
                .iter()
                .find(|n| *id == n.id)
                .context("Missing selected note draft")?;
            changes.push(json!({"track_id":track,"note_id":id,"before":before,"after":after}));
        }
    }
    Ok(
        json!({"project_id":project.id,"selection":selection.value(),"dry_run":true,"operations":operations,
        "changes":changes,"change_count":changes.len(),"pitch_reference":{
            "clip":reference,"range":[first,last],"notes":evidence}}),
    )
}

#[cfg(test)]
mod voice_inspection_tests {
    use super::*;
    use crate::{
        jobs::Control,
        model::{uid, Track},
    };

    #[test]
    fn reports_known_pitch_change_and_skips_unvoiced_interval() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_voice_inspect"));
        let store = Store::new(root.clone())?;
        let track = Track::new("Dry voice");
        let id = track.id.clone();
        let score = store.create("Dry pitch inspection", 120., [4, 4], vec![track])?;
        let score = Store::preview_edit(
            &score,
            &json!([{"op":"add_notes","track_id":id,"notes":[
                {"start":1,"duration":1,"pitch":69,"lyric":"あ"},
                {"start":2,"duration":1,"pitch":71,"lyric":"い"}
            ]}]),
        )?;
        let f = |cents: f64| {
            (0..RATE)
                .map(|i| {
                    if i >= RATE / 2 {
                        return [0.; 2];
                    }
                    let t = f64::from(i) / f64::from(RATE);
                    let phase = 2. * std::f64::consts::PI * 440. * 2f64.powf(cents / 1200.) * t;
                    [(0.3 * phase.sin()) as f32; 2]
                })
                .collect::<Vec<_>>()
        };
        let [a, b] = [-15., 44.].map(f);
        let control = Control::ephemeral("pitch_measurement", &score.id, score.revision, "guide");
        let report = inspect_drafts(
            [&score, &score],
            &id,
            [1., 3.],
            [RATE as usize / 2; 2],
            [&a, &b],
            &control,
        )?;
        for (side, expected) in [-15., 44.].into_iter().enumerate() {
            let notes = &report["notes_ab"][side];
            assert!((notes[0]["median_offset_cents"].as_f64().unwrap() - expected).abs() < 2.);
            assert_eq!(notes[1]["voiced_frames"], 0);
            assert_eq!(notes[1]["median_offset_cents"], Value::Null);
        }
        assert!((report["suggested"]["delta_cents"].as_f64().unwrap() - 59.).abs() < 3.);
        assert_eq!(report["suggested"]["beat_range"], json!([1., 2.125]));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
