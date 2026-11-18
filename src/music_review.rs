use crate::{
    lyrics,
    model::{Note, Project, Track},
    selection::Selection,
    timeline::Timeline,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

const PHRASE_GAP_SECONDS: f64 = 0.30;
const LYRIC_PREVIEW_LIMIT: usize = 40;
const NEARBY_ATTACK_SECONDS: f64 = 0.5;
const LISTEN_BEFORE_SECONDS: f64 = 0.55;
const LISTEN_AFTER_SECONDS: f64 = 0.75;

#[derive(Default)]
pub(crate) struct Window {
    pub bounds: Option<[f64; 2]>,
    pub track_ids: Option<HashSet<String>>,
}

impl Window {
    pub(crate) fn read(project: &Project, args: &Value) -> Result<Self> {
        let bounds = args
            .get("beat_range")
            .map(|range| -> Result<[f64; 2]> {
                let [start, end] =
                    serde_json::from_value(range.clone()).context("提供 [起拍, 止拍]")?;
                ensure!(
                    start >= 0. && end > start && end <= 50_000.,
                    "复核拍范围无效"
                );
                Ok([start, end])
            })
            .transpose()?;
        let track_ids = args
            .get("track_ids")
            .map(|value| -> Result<HashSet<String>> {
                let ids: Vec<String> = serde_json::from_value(value.clone())?;
                let unique: HashSet<_> = ids.iter().cloned().collect();
                ensure!(
                    !ids.is_empty()
                        && unique.len() == ids.len()
                        && ids
                            .iter()
                            .all(|id| project.tracks.iter().any(|track| &track.id == id)),
                    "复核需要有效且不重复的音轨 ID"
                );
                Ok(unique)
            })
            .transpose()?;
        Ok(Self { bounds, track_ids })
    }
}

struct Lane<'a> {
    track: &'a Track,
    notes: Vec<&'a Note>,
}

impl Lane<'_> {
    fn voice(&self) -> bool {
        self.track.instrument.is_none() && !self.track.is_audio()
    }
    fn adjacent(&self, before: &Note, after: &Note) -> bool {
        self.track
            .notes
            .get(
                self.track
                    .notes
                    .partition_point(|note| note.start <= before.start),
            )
            .is_some_and(|next| next.id == after.id)
    }
}

fn attacks(onsets: &[f64], [start, end]: [f64; 2]) -> usize {
    onsets.partition_point(|&beat| beat < end) - onsets.partition_point(|&beat| beat < start)
}

fn seconds(clock: &Timeline, [start, end]: [f64; 2]) -> f64 {
    clock.seconds(end) - clock.seconds(start)
}

fn rounded(value: f64) -> f64 {
    (value * 1000.).round() / 1000.
}

struct BackingAttack<'a> {
    track: &'a Track,
    beat: f64,
}

fn nearby_attacks(attacks: &[BackingAttack<'_>], clock: &Timeline, beat: f64) -> Value {
    let centre = attacks.partition_point(|attack| attack.beat < beat - 1e-9);
    let describe =
        |at: usize| -> Option<Value> {
            let at = attacks.get(at)?.beat;
            let offset = clock.seconds(at) - clock.seconds(beat);
            if offset.abs() > NEARBY_ATTACK_SECONDS + 1e-9 {
                return None;
            }
            let from = attacks.partition_point(|attack| attack.beat < at);
            let to = attacks.partition_point(|attack| attack.beat <= at);
            let mut tracks = BTreeMap::new();
            for attack in &attacks[from..to] {
                let (_, count) = tracks
                    .entry(&attack.track.id)
                    .or_insert((attack.track, 0usize));
                *count += 1;
            }
            let sources = tracks.into_values().map(|(track, count)| {
            let kind = if track.instrument.as_ref().is_some_and(|instrument| instrument.drums) {
                "drums"
            } else {
                "instrument"
            };
            json!({"track_id":track.id,"track_name":track.name,"kind":kind,"note_count":count})
        }).collect::<Vec<_>>();
            Some(json!({"beat":at,"offset_ms":(offset * 1000.).round() as i32,"tracks":sources}))
        };
    json!({
        "before":centre.checked_sub(1).and_then(describe),
        "on_or_after":describe(centre),
    })
}

fn listening_points<'a>(
    notes: &[&'a Note],
    syllables: &[lyrics::ScoreSyllable<'_>],
    bounds: [f64; 2],
    clock: &Timeline,
    backing: &[BackingAttack<'_>],
    rendered_end: f64,
) -> Vec<Value> {
    let first = notes[0];
    let last = notes.last().copied().unwrap();
    let mut entries: Vec<(&'a Note, f64, Vec<&str>)> = Vec::with_capacity(4);
    let mut include = |note: &'a Note, beat: f64, role: &'static str| {
        if let Some((_, _, roles)) = entries.iter_mut().find(|(_, at, _)| *at == beat) {
            roles.push(role);
        } else {
            entries.push((note, beat, vec![role]));
        }
    };
    if first.start >= bounds[0] && !first.vowel_continuation {
        include(first, first.start, "entry");
    }
    let end = last.start + last.duration;
    let full_heads = syllables.iter().filter(|syllable| {
        syllable.head_note_id.is_some()
            && (end <= bounds[1] || syllable.note_ids.last() != Some(&last.id.as_str()))
    });
    if let Some(held) =
        full_heads.max_by(|left, right| left.notated_seconds.total_cmp(&right.notated_seconds))
    {
        let index = notes.partition_point(|note| note.start < held.beat_range[0]);
        if let Some(note) = notes.get(index) {
            include(note, note.start, "longest_syllable");
        }
    }
    if let Some(peak) = notes
        .iter()
        .copied()
        .filter(|note| note.start >= bounds[0])
        .max_by(|left, right| {
            left.pitch
                .cmp(&right.pitch)
                .then_with(|| right.start.total_cmp(&left.start))
        })
    {
        include(peak, peak.start, "pitch_peak");
    }
    if end <= bounds[1] {
        include(last, end, "release");
    }
    entries.sort_by(|left, right| left.1.total_cmp(&right.1));
    entries
        .into_iter()
        .map(|(note, beat, roles)| {
            let sung = syllables
                .iter()
                .find(|syllable| syllable.note_ids.contains(&note.id.as_str()));
            let beat_range = [
                note.start.max(bounds[0]),
                (note.start + note.duration).min(bounds[1]),
            ];
            let centre = clock.seconds(beat);
            let audition_range = [
                clock
                    .beat((centre - LISTEN_BEFORE_SECONDS).max(0.))
                    .max(bounds[0]),
                clock
                    .beat(centre + LISTEN_AFTER_SECONDS)
                    .min(bounds[1])
                    .min(rendered_end),
            ];
            let [first, last] =
                audition_range.map(|at| (clock.seconds(at) * crate::audition::RATE as f64).round());
            json!({"roles":roles,"beat":beat,"position":clock.position(beat),"note_ids":[note.id],
            "beat_range":beat_range,"pitch":note.pitch,"velocity":note.velocity,
            "audition_range":(last>first).then_some(audition_range),
            "lyric":sung.and_then(|syllable|syllable.lyric),
            "syllable_ms":sung.map(|syllable|(syllable.notated_seconds*1000.).round() as u64),
            "backing":nearby_attacks(backing,clock,beat)})
        })
        .collect()
}

fn stats(
    notes: &[&Note],
    clock: &Timeline,
    bounds: [f64; 2],
    backing: &[BackingAttack<'_>],
    rendered_end: f64,
) -> Value {
    let duration = |note: &Note| {
        seconds(
            clock,
            [
                note.start.max(bounds[0]),
                (note.start + note.duration).min(bounds[1]),
            ],
        )
    };
    let held = notes
        .iter()
        .copied()
        .max_by(|a, b| duration(a).total_cmp(&duration(b)))
        .unwrap();
    let mut durations = BTreeMap::new();
    for note in notes {
        *durations
            .entry((note.duration * 480.).round() as u64)
            .or_insert(0usize) += 1;
    }
    let biggest = notes
        .windows(2)
        .max_by_key(|pair| pair[1].pitch.abs_diff(pair[0].pitch));
    let syllables = lyrics::score_syllables(notes, bounds, clock);
    let preview = &syllables[..syllables.len().min(LYRIC_PREVIEW_LIMIT)];
    let lyric = preview
        .iter()
        .map(lyrics::ScoreSyllable::label)
        .collect::<Vec<_>>()
        .join(" ");
    let longest = syllables
        .iter()
        .max_by(|a, b| a.notated_seconds.total_cmp(&b.notated_seconds));
    let lyric_onsets = syllables
        .iter()
        .filter(|event| event.head_note_id.is_some())
        .count();
    let dominant = durations.values().copied().max().unwrap() as f64 / notes.len() as f64;
    json!({
        "note_count":notes.len(),"lyric_samples":lyric,"lyric_samples_truncated":preview.len()<syllables.len(),
        "lyric_onsets":lyric_onsets,"syllables":preview,"longest_syllable":longest,
        "listening_points":listening_points(notes,&syllables,bounds,clock,backing,rendered_end),
        "vowel_continuation_notes":notes.iter().filter(|note|note.vowel_continuation).count(),
        "boundary_note_ids":[notes.first().unwrap().id,notes.last().unwrap().id],
        "distinct_notated_durations":durations.len(),"dominant_duration_fraction":rounded(dominant),
        "pitch_range":[notes.iter().map(|note|note.pitch).min(),notes.iter().map(|note|note.pitch).max()],
        "longest_note":{"note_id":held.id,
            "lyric":if held.start<bounds[0]{lyrics::CONTINUATION_MARK}else{lyrics::display(held)},"start":held.start,
            "seconds":rounded(duration(held))},
        "largest_leap":biggest.map(|pair|json!({"note_ids":[pair[0].id,pair[1].id],"at":pair[1].start,
            "semitones":i16::from(pair[1].pitch)-i16::from(pair[0].pitch)})),
    })
}

pub(crate) fn end_beat(project: &Project) -> f64 {
    let planned = crate::arrangement::section_markers(project)
        .iter()
        .filter_map(|section| section["end"].as_f64())
        .filter(|&beat| (0.0..=50_000.).contains(&beat))
        .fold(4., f64::max);
    project.timeline().beat(project.seconds()).max(planned)
}

pub(crate) fn report(
    project: &Project,
    window: &Window,
    selection: Option<&Selection>,
    limit: usize,
) -> Value {
    let clock = project.timeline();
    let bounds = window.bounds.unwrap_or_else(|| [0., end_beat(project)]);
    let rendered_end =
        clock.beat(crate::source_cache::frame_count(project) as f64 / crate::audition::RATE as f64);
    let [start, end] = bounds;
    let selected = selection.map(|s| {
        s.notes
            .iter()
            .map(|(_, id)| id.as_str())
            .collect::<HashSet<_>>()
    });
    let lanes: Vec<_> = project
        .tracks
        .iter()
        .filter(|track| {
            window
                .track_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&track.id))
        })
        .filter(|track| selection.is_none_or(|s| s.notes.iter().any(|(id, _)| id == &track.id)))
        .map(|track| Lane {
            track,
            notes: track
                .notes
                .iter()
                .filter(|note| {
                    note.start < end
                        && note.start + note.duration > start
                        && selected
                            .as_ref()
                            .is_none_or(|ids| ids.contains(note.id.as_str()))
                })
                .collect(),
        })
        .collect();
    let mut pitched = Vec::new();
    let mut drums = Vec::new();
    let mut vocal = Vec::new();
    let mut backing = Vec::new();
    let solo = project.tracks.iter().any(|track| track.solo && !track.mute);
    for lane in &lanes {
        if let Some(instrument) = &lane.track.instrument {
            let onsets = if instrument.drums {
                &mut drums
            } else {
                &mut pitched
            };
            if crate::source_cache::active(lane.track, solo) {
                onsets.extend(lane.notes.iter().map(|note| note.start));
                if crate::routing::group_open(
                    lane.track.output_group.as_deref(),
                    &project.mix_groups,
                ) {
                    backing.extend(lane.notes.iter().filter(|note| note.start >= start).map(
                        |note| BackingAttack {
                            track: lane.track,
                            beat: note.start,
                        },
                    ));
                }
            }
        } else if lane.voice() {
            vocal.extend(lane.notes.iter().copied());
        }
    }
    pitched.sort_unstable_by(f64::total_cmp);
    drums.sort_unstable_by(f64::total_cmp);
    backing.sort_unstable_by(|left, right| left.beat.total_cmp(&right.beat));
    let count_backing = |range| json!({"pitched_attacks":attacks(&pitched,range),"drum_hits":attacks(&drums,range)});
    let mut phrases = Vec::new();
    let mut phrase_count = 0usize;
    let mut observations = Vec::new();
    for lane in lanes.iter().filter(|lane| lane.voice()) {
        let mut remaining = lane.notes.as_slice();
        while !remaining.is_empty() {
            let len = remaining
                .windows(2)
                .position(|pair| {
                    !lane.adjacent(pair[0], pair[1])
                        || seconds(&clock, [pair[0].start + pair[0].duration, pair[1].start])
                            >= PHRASE_GAP_SECONDS
                })
                .map_or(remaining.len(), |index| index + 1);
            let (phrase, tail) = remaining.split_at(len);
            remaining = tail;
            let a = phrase[0].start.max(start);
            let last = phrase.last().unwrap();
            let b = (last.start + last.duration).min(end);
            let duration = seconds(&clock, [a, b]);
            phrase_count += 1;
            if phrases.len() >= limit {
                continue;
            }
            let mut row = stats(phrase, &clock, bounds, &backing, rendered_end);
            row["track_id"] = json!(lane.track.id);
            row["track_name"] = json!(lane.track.name);
            row["beat_range"] = json!([a, b]);
            row["bar"] = json!(clock.position(a));
            row["seconds"] = json!(rounded(duration));
            row["edge_clipped"] = json!(a > phrase[0].start || b < last.start + last.duration);
            row["audible_in_mix"] = json!(crate::source_cache::active(lane.track, solo));
            row["backing_during"] = count_backing([a, b]);
            let held = crate::harmony::held_voices(project, &clock, phrase, [a, b]);
            if !held.is_empty() {
                row["harmony_listen_points"] = json!(held);
            }
            if let Some(next) = remaining.first().filter(|next| lane.adjacent(last, next)) {
                let gap = [b, next.start];
                row["gap_after"] = json!({"beat_range":gap,"seconds":rounded(seconds(&clock,gap)),
                    "backing":count_backing(gap),"next_note_id":next.id});
            }
            let question = if duration >= 9. {
                Some((
                    "prosody",
                    "这一句连续时间较长。试听词义转折和实际换气位置。",
                ))
            } else if row["lyric_onsets"].as_u64().unwrap_or(0) >= 8
                && row["dominant_duration_fraction"].as_f64().unwrap_or(0.) >= 0.85
            {
                Some((
                    "rhythm",
                    "这一句大多使用相同时值。试听落字重音是否清楚，哪些字值得延长。",
                ))
            } else if row["largest_leap"]["semitones"]
                .as_i64()
                .is_some_and(|jump| jump.abs() >= 9)
            {
                Some((
                    "melody",
                    "这一句有较大跳进。试听落点是否符合词意或段落需要。",
                ))
            } else {
                None
            };
            if let Some((topic, text)) = question {
                observations.push(json!({"track_id":lane.track.id,"track_name":lane.track.name,
                    "beat_range":[a,b],"topic":topic,"question":text,"note_ids":row["boundary_note_ids"]}));
            }
            phrases.push(row);
        }
    }
    let mut sections = Vec::new();
    for section in crate::arrangement::section_markers(project) {
        let a = section["start"].as_f64().unwrap_or(0.).max(start);
        let b = section["end"].as_f64().unwrap_or(0.).min(end);
        if b <= a {
            continue;
        }
        let excerpt: Vec<_> = vocal
            .iter()
            .filter(|n| n.start >= a && n.start < b)
            .collect();
        let onsets = excerpt
            .iter()
            .filter(|note| !note.vowel_continuation)
            .count();
        let ps = attacks(&pitched, [a, b]);
        let ds = attacks(&drums, [a, b]);
        sections.push(json!({"name":section["name"],"role":section["role"],"beat_range":[a,b],
            "bar":clock.position(a),"vocal_onsets":onsets,"vocal_note_entries":excerpt.len(),
            "mean_vocal_pitch":(!excerpt.is_empty()).then(||rounded(excerpt.iter().map(|n|f64::from(n.pitch)).sum::<f64>()/excerpt.len() as f64)),
            "vocal_onsets_per_beat":rounded(onsets as f64/(b-a)),
            "pitched_attacks_per_beat":rounded(ps as f64/(b-a)),
            "drum_hits_per_beat":rounded(ds as f64/(b-a))}));
    }
    let mut lines = lanes
        .iter()
        .flat_map(|lane| crate::lyric_lines::report(lane.track, window.bounds, selected.as_ref()))
        .take(limit + 1)
        .collect::<Vec<_>>();
    let lines_truncated = lines.len() > limit;
    lines.truncate(limit);
    json!({"project_id":project.id,"revision":project.revision,"beat_range":bounds,
        "phrases":phrases,"total_phrases":phrase_count,"phrases_truncated":phrase_count>limit,
        "observations":observations,"sections":sections,"lyric_lines":lines,
        "lyric_lines_truncated":lines_truncated,
        "scope_matches_exact_notes":selection.is_some(),
        "audio_tracks_not_counted":lanes.iter().any(|lane|lane.track.is_audio()),
        "phrase_gap_seconds":PHRASE_GAP_SECONDS,"nearby_attack_window_ms":NEARBY_ATTACK_SECONDS*1000.,
        "requires_listening":true,"musical_quality_approved":false})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Track,
        timeline::{Curve, TempoPoint},
    };

    #[test]
    fn gaps_use_tempo_and_missing_selection_notes_are_not_a_silent_gap() -> Result<()> {
        let mut voice = Track::new("Vocal");
        let mut keys = Track::new("Keys");
        keys.instrument = Some(Default::default());
        let mk = |id: &str, start: f64, duration: f64| {
            Note::from_value(&json!({"id":id,"start":start,"duration":duration}))
        };
        voice.notes = vec![
            mk("note_a", 0., 0.5)?,
            mk("note_b", 0.5, 0.5)?,
            mk("note_c", 1., 1.)?,
            mk("note_d", 2.35, 0.5)?,
        ];
        voice.notes[2].pitch = 72;
        keys.notes = vec![mk("attack", 2.2, 0.2)?];
        let mut drums = Track::new("Drums");
        drums.instrument = Some(crate::model::Instrument {
            drums: true,
            ..Default::default()
        });
        drums.notes = vec![mk("drum_before", 1.8, 0.1)?, mk("drum_after", 2.2, 0.1)?];
        let mut project: Project = serde_json::from_value(json!({
            "id":"p_test","revision":1,"title":"technical fixture","format":"agent-vocal/2",
            "bpm":120,"meter":[6,8],"tracks":[voice,keys,drums]
        }))?;
        project.tempo_map = vec![TempoPoint {
            beat: 2.,
            bpm: 60.,
            curve: Curve::Step,
        }];
        let window = Window {
            bounds: Some([0., 3.]),
            track_ids: None,
        };
        let data = report(&project, &window, None, 20);
        assert_eq!(data["total_phrases"], 2);
        assert_eq!(data["phrases"][0]["gap_after"]["seconds"], 0.35);
        assert_eq!(
            data["phrases"][0]["gap_after"]["backing"]["pitched_attacks"],
            1
        );
        let points = data["phrases"][0]["listening_points"].as_array().unwrap();
        assert_eq!(
            points[1]["roles"],
            json!(["longest_syllable", "pitch_peak"])
        );
        assert_eq!(points[1]["note_ids"], json!(["note_c"]));
        assert_eq!(
            points[1]["position"],
            json!({"bar":1,"beat":3,"fraction":0.,"meter":[6,8]})
        );
        let release = &points[2];
        assert_eq!(release["roles"], json!(["release"]));
        assert_eq!(release["beat"], 2.);
        let listening_range: [f64; 2] = serde_json::from_value(release["audition_range"].clone())?;
        assert_eq!(
            crate::audition::frames(&project, listening_range)?,
            [21_600, 84_000]
        );
        assert_eq!(release["backing"]["before"]["offset_ms"], -100);
        assert_eq!(release["backing"]["on_or_after"]["offset_ms"], 200);
        assert_eq!(
            release["backing"]["on_or_after"]["tracks"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let bounded = report(
            &project,
            &Window {
                bounds: Some([1.8, 2.1]),
                track_ids: None,
            },
            None,
            20,
        );
        let release = &bounded["phrases"][0]["listening_points"][0];
        assert_eq!(release["roles"], json!(["release"]));
        assert_eq!(release["audition_range"], json!([1.8, 2.1]));
        assert!(release["backing"]["on_or_after"].is_null());
        let track = &project.tracks[0];
        let selection = Selection {
            id: "test".into(),
            project_id: project.id.clone(),
            revision: 1,
            notes: vec![
                (track.id.clone(), "note_a".into()),
                (track.id.clone(), "note_c".into()),
            ],
        };
        let selected = report(&project, &window, Some(&selection), 20);
        assert_eq!(selected["total_phrases"], 2);
        assert!(selected["phrases"][0].get("gap_after").is_none());
        assert!(selected["phrases"][0]["listening_points"][0]["backing"]["on_or_after"].is_null());
        assert!(
            !selected.to_string().contains("note_b") && !selected.to_string().contains("note_d")
        );
        Ok(())
    }
}
