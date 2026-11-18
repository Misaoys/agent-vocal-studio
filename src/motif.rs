//! Anchored, read-only evidence about returns of a notated vocal motive.
use crate::{
    lyrics,
    model::{Note, Project, Track},
    music_review::{end_beat, Window},
    selection::Selection,
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

const MAX_NOTES: usize = 16;
const MAX_RETURNS: usize = 12;
const EPS: f64 = 1e-8;
const TIMING_TOLERANCE: f64 = 1. / 32.;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    track_id: String,
    first_note_id: String,
    last_note_id: String,
}

pub(crate) fn anchor_schema() -> Value {
    json!({"type":"object","additionalProperties":false,
        "properties":{"track_id":{"type":"string"},"first_note_id":{"type":"string"},"last_note_id":{"type":"string"}},
        "required":["track_id","first_note_id","last_note_id"]})
}

fn span(notes: &[Note]) -> [f64; 2] {
    let tail = notes.last().unwrap();
    [notes[0].start, tail.start + tail.duration]
}

fn semitones(a: u8, b: u8) -> i16 {
    i16::from(b) - i16::from(a)
}

fn rounded(value: f64) -> f64 {
    (value * 1_000.).round() / 1_000.
}

struct Match {
    shift: i16,
    scale: f64,
    end_pitch_delta: i16,
    end_onset_delta: f64,
    end_duration_delta: f64,
    changed_lengths: Vec<usize>,
}

fn compare(anchor: &[Note], candidate: &[Note]) -> Option<Match> {
    let penultimate = anchor.len() - 2;
    let base = anchor[0].start;
    let start = candidate[0].start;
    let scale = (candidate[penultimate].start - start) / (anchor[penultimate].start - base);
    if !(0.5 - EPS..=2. + EPS).contains(&scale) {
        return None;
    }
    let shift = semitones(anchor[0].pitch, candidate[0].pitch);
    let onset_delta = |a: &Note, b: &Note| b.start - start - (a.start - base) * scale;
    let length_delta = |a: &Note, b: &Note| b.duration - a.duration * scale;
    let tolerance = TIMING_TOLERANCE * scale.max(1.);
    if anchor[..=penultimate]
        .iter()
        .zip(candidate)
        .any(|(a, b)| semitones(a.pitch, b.pitch) != shift || onset_delta(a, b).abs() > tolerance)
    {
        return None;
    }
    let last = anchor.last().unwrap();
    let return_last = candidate.last().unwrap();
    let raw_onset = onset_delta(last, return_last);
    let raw_duration = length_delta(last, return_last);
    // A distant closing note should not join an otherwise unrelated phrase.
    let expected_gap =
        (last.start - anchor[penultimate].start - anchor[penultimate].duration).max(0.) * scale;
    let gap = return_last.start - candidate[penultimate].start - candidate[penultimate].duration;
    if gap > expected_gap + 0.75 * scale.max(1.) {
        return None;
    }
    let significant = |delta: f64| {
        if delta.abs() > tolerance {
            rounded(delta)
        } else {
            0.
        }
    };
    Some(Match {
        shift,
        scale: rounded(scale),
        end_pitch_delta: semitones(last.pitch, return_last.pitch) - shift,
        end_onset_delta: significant(raw_onset),
        end_duration_delta: significant(raw_duration),
        changed_lengths: anchor
            .iter()
            .zip(candidate)
            .enumerate()
            .filter(|(_, (a, b))| length_delta(a, b).abs() > tolerance)
            .map(|(index, _)| index)
            .collect(),
    })
}

fn available(track: &Track, window: &Window) -> bool {
    window
        .track_ids
        .as_ref()
        .is_none_or(|ids| ids.contains(&track.id))
}

fn describe(
    project: &Project,
    notes: &[Note],
    track: &Track,
    window: &Window,
    selected: Option<&HashSet<&str>>,
) -> Value {
    let range = span(notes);
    let clock = project.timeline();
    let [start, end] = range;
    let section = crate::arrangement::section_markers(project)
        .into_iter()
        .find(|section| {
            section["start"].as_f64().is_some_and(|a| a <= start)
                && section["end"].as_f64().is_some_and(|b| start < b)
        })
        .map(|item| item["name"].clone());
    let solo = project.tracks.iter().any(|item| item.solo && !item.mute);
    let mut backing = Vec::new();
    for item in project.tracks.iter().filter(|item| available(item, window)) {
        let Some(instrument) = &item.instrument else {
            continue;
        };
        if !crate::source_cache::active(item, solo)
            || !crate::routing::group_open(item.output_group.as_deref(), &project.mix_groups)
        {
            continue;
        }
        let first = item.notes.partition_point(|note| note.start < start);
        let last = item.notes.partition_point(|note| note.start < end);
        let attacks = item.notes[first..last]
            .iter()
            .filter(|note| selected.is_none_or(|ids| ids.contains(note.id.as_str())))
            .count();
        if attacks > 0 {
            backing.push(json!({"track_id":item.id,"name":item.name,"drums":instrument.drums,"attacks":attacks}));
        }
    }
    let harmony = crate::harmony::query(project, Some(range));
    let spans: &[Value] = harmony["chords"].as_array().map_or(&[], Vec::as_slice);
    let chords = &spans[..spans.len().min(12)];
    let selected_notes: Vec<_> = notes.iter().collect();
    json!({"track_id":track.id,"beat_range":range,"bar":clock.position(start),
        "seconds":rounded(clock.seconds(end)-clock.seconds(start)),"section":section,
        "note_ids":notes.iter().map(|note|&note.id).collect::<Vec<_>>(),
        "lyrics":notes.iter().map(lyrics::display).collect::<Vec<_>>(),
        "syllables":lyrics::score_syllables(&selected_notes,range,&clock),
        "pitches":notes.iter().map(|note|note.pitch).collect::<Vec<_>>(),
        "onsets":notes.iter().map(|note|note.start).collect::<Vec<_>>(),
        "durations":notes.iter().map(|note|note.duration).collect::<Vec<_>>(),
        "backing_attacks":backing,"harmony_spans":chords,"harmony_truncated":spans.len()>chords.len()})
}

pub(crate) fn report(
    project: &Project,
    args: &Value,
    window: &Window,
    selection: Option<&Selection>,
) -> Result<Value> {
    let anchor: Anchor =
        serde_json::from_value(args["anchor"].clone()).context("请选择动机的首尾音符")?;
    let track = project
        .tracks
        .iter()
        .find(|item| item.id == anchor.track_id && available(item, window))
        .context("动机音轨不在复核范围")?;
    ensure!(
        track.instrument.is_none() && !track.is_audio(),
        "动机回归需要歌声音轨"
    );
    let index = |id: &str| {
        track
            .notes
            .iter()
            .position(|note| note.id == id)
            .context("动机音符已更新，请重新选择")
    };
    let a = index(&anchor.first_note_id)?;
    let b = index(&anchor.last_note_id)? + 1;
    let pattern = track
        .notes
        .get(a..b)
        .filter(|notes| (4..=MAX_NOTES).contains(&notes.len()))
        .context("选择连续 4–16 个歌声音符")?;
    let selected = selection.map(|scope| {
        scope
            .notes
            .iter()
            .filter(|(id, _)| id == &track.id)
            .map(|(_, note)| note.as_str())
            .collect::<HashSet<_>>()
    });
    let selected_all = selection.map(|scope| {
        scope
            .notes
            .iter()
            .map(|(_, note)| note.as_str())
            .collect::<HashSet<_>>()
    });
    let bounds = window.bounds.unwrap_or_else(|| [0., end_beat(project)]);
    let visible = |notes: &[Note]| {
        let [start, end] = span(notes);
        start >= bounds[0] - EPS
            && end <= bounds[1] + EPS
            && selected
                .as_ref()
                .is_none_or(|ids| notes.iter().all(|note| ids.contains(note.id.as_str())))
    };
    ensure!(visible(pattern), "动机超出当前音符选区或拍范围");
    ensure!(
        pattern[pattern.len() - 2].start > pattern[0].start,
        "动机需要不同的入拍"
    );
    let mut rows = Vec::new();
    let mut count = 0;
    let mut next_allowed = 0;
    for (first, candidate) in track.notes.windows(pattern.len()).enumerate() {
        let end = first + candidate.len();
        if first < next_allowed || first < b && end > a || !visible(candidate) {
            continue;
        }
        let Some(matched) = compare(pattern, candidate) else {
            continue;
        };
        count += 1;
        next_allowed = end;
        if rows.len() >= MAX_RETURNS {
            continue;
        }
        let mut row = describe(project, candidate, track, window, selected_all.as_ref());
        let ending = matched.end_pitch_delta != 0
            || matched.end_onset_delta != 0.
            || matched.end_duration_delta != 0.;
        row["relation"] = json!(if ending {
            "changed_ending"
        } else {
            "restatement"
        });
        row["semitone_shift"] = json!(matched.shift);
        row["beat_scale"] = json!(matched.scale);
        row["ending_delta"] = json!({"semitones":matched.end_pitch_delta,"onset_beats":matched.end_onset_delta,
            "duration_beats":matched.end_duration_delta});
        row["changed_length_indices"] = json!(matched.changed_lengths);
        let sung_tokens = |notes: &[Note]| {
            notes
                .iter()
                .filter(|note| !note.vowel_continuation)
                .map(|note| {
                    lyrics::kana(&note.lyric)
                        .map_or_else(|_| note.lyric.clone(), |tokens| tokens.join(" "))
                })
                .collect::<Vec<_>>()
        };
        row["same_lyric_tokens"] = json!((!pattern[0].vowel_continuation
            && !candidate[0].vowel_continuation)
            .then(|| sung_tokens(pattern) == sung_tokens(candidate)));
        row["same_lyric_placement"] = json!(pattern
            .iter()
            .zip(candidate)
            .all(|(a, b)| a.vowel_continuation == b.vowel_continuation));
        rows.push(row);
    }
    Ok(
        json!({"project_id":project.id,"revision":project.revision,"beat_range":bounds,
        "anchor":describe(project,pattern,track,window,selected_all.as_ref()),
        "returns":rows,"total_returns":count,"truncated":count>MAX_RETURNS,
        "head_notes":pattern.len()-1,"beat_tolerance":TIMING_TOLERANCE,
        "requires_listening":true}),
    )
}
