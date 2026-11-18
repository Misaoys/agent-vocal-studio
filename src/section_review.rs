use crate::{
    model::{Note, Project, Track},
    music_review::Window,
    timeline::Timeline,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{collections::HashSet, hash::Hash};

pub(crate) struct Request {
    pub ranges: [[f64; 2]; 2],
    pub track_ids: Option<HashSet<String>>,
    lead: Option<String>,
}

impl Request {
    pub(crate) fn read(project: &Project, args: &Value) -> Result<Self> {
        let ranges: [[f64; 2]; 2] =
            serde_json::from_value(args["beat_ranges"].clone()).context("请选择两段拍范围")?;
        let content = crate::music_review::end_beat(project);
        let (_, bar, meter) = project.timeline().bar_start(content);
        let max = if (content - bar).abs() < 1e-7 {
            content
        } else {
            bar + crate::timeline::bar_length(meter)
        };
        ensure!(
            ranges[0] != ranges[1]
                && ranges
                    .iter()
                    .all(|&[a, b]| a >= 0. && b > a && b <= 50_000. && b <= max + 1e-7),
            "请选择两段不同的工程拍范围"
        );
        let track_ids = Window::read(project, args)?.track_ids;
        let lead = args
            .get("lead_track_id")
            .map(|value| {
                let id = value.as_str().context("主唱轨 ID 无效")?;
                ensure!(
                    project
                        .tracks
                        .iter()
                        .any(|track| track.id == id && voice(track))
                        && track_ids.as_ref().is_none_or(|ids| ids.contains(id)),
                    "主唱须是所选范围内的歌声轨"
                );
                Ok::<_, anyhow::Error>(id.to_owned())
            })
            .transpose()?;
        Ok(Self {
            ranges,
            track_ids,
            lead,
        })
    }
}

fn voice(track: &Track) -> bool {
    track.instrument.is_none() && !track.is_audio()
}

fn rounded(value: f64) -> f64 {
    (value * 1000.).round() / 1000.
}

fn covered_seconds(clock: &Timeline, spans: &mut [[f64; 2]]) -> f64 {
    spans.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
    let mut end = 0.;
    let mut total = 0.;
    for &[start, stop] in spans.iter() {
        if stop > end {
            total += clock.seconds(stop) - clock.seconds(start.max(end));
            end = stop;
        }
    }
    rounded(total)
}

fn match_ratio<T: Eq + Hash>(a: &HashSet<T>, b: &HashSet<T>) -> Option<f64> {
    (!a.is_empty() && !b.is_empty())
        .then(|| rounded(a.intersection(b).count() as f64 / a.union(b).count() as f64))
}

struct Lane {
    view: Value,
    onsets: HashSet<i64>,
    hit_pattern: HashSet<(i64, u8)>,
    entry: Option<f64>,
    reply: Option<f64>,
}

fn lane(
    track: &Track,
    clock: &Timeline,
    [start, end]: [f64; 2],
    lead: Option<&[&Note]>,
    drums: bool,
) -> Lane {
    let notes: Vec<_> = track
        .notes
        .iter()
        .filter(|note| note.start < end && note.start + note.duration > start)
        .collect();
    let clips = track.media_clips();
    let mut clip_spans = clips
        .iter()
        .map(|clip| clip.beat_span(clock))
        .filter(|[a, b]| *a < end && *b > start)
        .map(|[a, b]| [a.max(start), b.min(end)])
        .collect::<Vec<_>>();
    let mut score_spans = notes
        .iter()
        .map(|note| [note.start.max(start), (note.start + note.duration).min(end)])
        .collect::<Vec<_>>();
    let entry = score_spans
        .iter()
        .chain(&clip_spans)
        .map(|span| span[0])
        .min_by(f64::total_cmp);
    let attacks = notes
        .iter()
        .copied()
        .filter(|note| note.start >= start && !note.vowel_continuation);
    let mut onsets = HashSet::new();
    let mut hit_pattern = HashSet::new();
    let mut attack_count = 0;
    let mut under_lead = 0;
    let mut after_release = 0;
    let mut reply = None;
    for note in attacks {
        let tick = ((note.start - start) * 24.).round() as i64;
        onsets.insert(tick);
        if drums {
            hit_pattern.insert((tick, note.pitch));
        }
        attack_count += 1;
        if let Some(lead) = lead {
            let index = lead.partition_point(|s| s.start + s.duration <= note.start);
            if lead.get(index).is_some_and(|s| s.start <= note.start) {
                under_lead += 1;
            } else if index > 0 {
                let prev = lead[index - 1];
                let release = prev.start + prev.duration;
                let seconds = clock.seconds(note.start) - clock.seconds(release);
                if release >= start && (0. ..=0.30).contains(&seconds) {
                    reply =
                        Some(reply.map_or(note.start, |previous: f64| previous.min(note.start)));
                    after_release += 1;
                }
            }
        }
    }
    let register_overlap_seconds =
        lead.filter(|_| track.instrument.is_some() && !drums)
            .map(|lead| {
                let mut spans = vec![];
                for note in &notes {
                    let a = note.start.max(start);
                    let b = (note.start + note.duration).min(end);
                    let from = lead.partition_point(|s| s.start + s.duration <= a);
                    for singer in lead[from..].iter().take_while(|s| s.start < b) {
                        if singer.pitch.abs_diff(note.pitch) <= 5 {
                            spans
                                .push([a.max(singer.start), b.min(singer.start + singer.duration)]);
                        }
                    }
                }
                covered_seconds(clock, &mut spans)
            });
    let has_lead_counts = lead.is_some() && !track.is_audio();
    Lane {
        view: json!({"note_count":notes.len(),"attack_count":attack_count,"attack_positions":onsets.len(),
            "score_seconds":covered_seconds(clock,&mut score_spans),"clip_seconds":covered_seconds(clock,&mut clip_spans),
            "first_beat":entry,"clip_count":clip_spans.len(),
            "under_lead_attacks":has_lead_counts.then_some(under_lead),
            "in_lead_gaps_attacks":has_lead_counts.then_some(attack_count-under_lead),
            "near_release_attacks":has_lead_counts.then_some(after_release),
            "register_overlap_seconds":register_overlap_seconds}),
        onsets,
        hit_pattern,
        entry,
        reply,
    }
}

pub(crate) fn report(project: &Project, request: &Request) -> Value {
    let clock = project.timeline();
    let ranges = request.ranges;
    let tracks: Vec<_> = project
        .tracks
        .iter()
        .filter(|track| {
            request
                .track_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&track.id))
        })
        .collect();
    let voices: Vec<_> = tracks
        .iter()
        .copied()
        .filter(|track| voice(track) && !track.notes.is_empty())
        .collect();
    let lead = request
        .lead
        .as_ref()
        .and_then(|id| tracks.iter().copied().find(|track| track.id == *id))
        .or_else(|| {
            if voices.len() == 1 {
                Some(voices[0])
            } else {
                None
            }
        });
    let lead_notes = lead.map(|track| {
        let mut notes = track.notes.iter().collect::<Vec<_>>();
        notes.sort_unstable_by(|a, b| a.start.total_cmp(&b.start));
        notes
    });
    let meter = |[a, b]: [f64; 2]| {
        let (_, bar, meter) = clock.bar_start(a);
        (
            meter,
            rounded(a - bar),
            project
                .meter_map
                .iter()
                .filter(|point| point.beat > a && point.beat < b)
                .map(|point| (rounded(point.beat - a), point.meter))
                .collect::<Vec<_>>(),
        )
    };
    let comparable = ((ranges[0][1] - ranges[0][0]) - (ranges[1][1] - ranges[1][0])).abs() < 1e-6
        && meter(ranges[0]) == meter(ranges[1]);
    let solo = project.tracks.iter().any(|track| track.solo && !track.mute);
    let window = |i: usize, point: f64| {
        let [a, b] = ranges[i];
        let start = (point - 0.25).clamp(a, (b - 0.25).max(a));
        [start, b.min(clock.beat(clock.seconds(start) + 4.0))]
    };
    let cue_pair = |i: usize, beat: f64| {
        let offset = beat - ranges[i][0];
        std::array::from_fn::<_, 2, _>(|side| window(side, ranges[side][0] + offset))
    };
    let mut lanes = vec![];
    let mut cues = vec![json!({"kind":"section_opening","beat_ranges":cue_pair(0,ranges[0][0])})];
    let mut lead_cues = vec![];
    if let Some((track, notes)) = lead.zip(lead_notes.as_ref()).filter(|(track, _)| {
        crate::source_cache::active(track, solo)
            && crate::routing::group_open(track.output_group.as_deref(), &project.mix_groups)
    }) {
        let entries = ranges.map(|[a, b]| {
            notes.iter().copied().find(|note| {
                note.start >= a
                    && note.start < b
                    && !note.vowel_continuation
                    && !note.lyric.trim().is_empty()
            })
        });
        let releases = ranges.map(|[a, b]| {
            notes
                .iter()
                .rev()
                .copied()
                .find(|note| note.start < b && note.start + note.duration > a)
                .filter(|note| note.start >= a && note.start + note.duration <= b + 1e-8)
        });
        for (kind, notes, before, after) in [
            ("lead_entry", entries, 0.35_f64, 2.65_f64),
            ("lead_release", releases, 1.3, 1.2),
        ] {
            let [Some(a), Some(b)] = notes else { continue };
            let anchors = std::array::from_fn::<_, 2, _>(|side| {
                let note = [a, b][side];
                let beat = if kind == "lead_release" {
                    note.start + note.duration
                } else {
                    note.start
                };
                beat.clamp(ranges[side][0], ranges[side][1])
            });
            if anchors[0] == anchors[1] {
                continue;
            }
            let seconds = anchors.map(|beat| clock.seconds(beat));
            let sections = ranges.map(|range| range.map(|beat| clock.seconds(beat)));
            let before = before
                .min(seconds[0] - sections[0][0])
                .min(seconds[1] - sections[1][0]);
            let after = after
                .min(sections[0][1] - seconds[0])
                .min(sections[1][1] - seconds[1]);
            if before + after < 1. / crate::audition::RATE as f64 {
                continue;
            }
            let windows = std::array::from_fn::<_, 2, _>(|side| {
                let [low, high] = ranges[side];
                [
                    clock.beat(seconds[side] - before).clamp(low, anchors[side]),
                    clock.beat(seconds[side] + after).clamp(anchors[side], high),
                ]
            });
            lead_cues.push(
                json!({"kind":kind,"track_id":track.id,"track_name":track.name,
                "beat_ranges":windows,"anchor_beats":anchors,"anchor_note_ids":[a.id,b.id]}),
            );
        }
    }
    let mut replies = vec![];
    let mut repeated = vec![];
    let mut counts = [0, 0];
    for track in tracks {
        let drums = track
            .instrument
            .as_ref()
            .is_some_and(|instrument| instrument.drums);
        let role = if track.is_audio() {
            "audio"
        } else if voice(track) {
            "voice"
        } else if drums {
            "drums"
        } else {
            "instrument"
        };
        let around = (!voice(track) && !track.is_audio())
            .then_some(lead_notes.as_deref())
            .flatten();
        let [a, b] = ranges.map(|range| lane(track, &clock, range, around, drums));
        if a.entry.is_none() && b.entry.is_none() {
            continue;
        }
        let audible = crate::source_cache::active(track, solo)
            && crate::routing::group_open(track.output_group.as_deref(), &project.mix_groups);
        let timing = comparable
            .then(|| match_ratio(&a.onsets, &b.onsets))
            .flatten();
        let hits = (comparable && drums)
            .then(|| match_ratio(&a.hit_pattern, &b.hit_pattern))
            .flatten();
        if audible {
            counts[0] += usize::from(a.entry.is_some());
            counts[1] += usize::from(b.entry.is_some());
            if let Some((kind, side, beat)) = match (a.entry, b.entry) {
                (None, Some(beat)) => Some(("enters", 1, beat)),
                (Some(beat), None) => Some(("drops", 0, beat)),
                _ => None,
            } {
                if cues.len() < 5 {
                    cues.push(
                        json!({"kind":kind,"track_id":track.id,"track_name":track.name,
                    "beat_ranges":cue_pair(side,beat)}),
                    );
                }
            }
            if let Some(beat) = b.reply {
                replies.push((b.view["near_release_attacks"].as_u64().unwrap_or(0),
                    json!({"kind":"release_response","track_id":track.id,"track_name":track.name,"beat_ranges":cue_pair(1,beat)})));
            }
            if !voice(track)
                && a.onsets.len() >= 6
                && b.onsets.len() >= 6
                && hits.or(timing).is_some_and(|ratio| ratio >= 0.85)
            {
                repeated.push(
                    json!({"kind":"similar_attacks","track_id":track.id,"track_name":track.name,
                    "beat_ranges":cue_pair(0,a.entry.unwrap_or(ranges[0][0]))}),
                );
            }
        }
        lanes.push(
            json!({"track_id":track.id,"track_name":track.name,"kind":role,"enabled_in_mix":audible,
            "sides":[a.view,b.view],"onset_similarity":timing,"drum_hit_similarity":hits}),
        );
    }
    if let Some((_, cue)) = replies.into_iter().max_by_key(|(count, _)| *count) {
        cues.push(cue);
    }
    cues.extend(repeated.into_iter().take(2));
    cues.extend(lead_cues);
    let markers = crate::arrangement::section_markers(project);
    let sections = ranges.into_iter().enumerate().map(|(i,[a,b])| {
        let authored = markers.iter().find(|part| part["start"].as_f64()==Some(a)&&part["end"].as_f64()==Some(b));
        let length = rounded(clock.seconds(b)-clock.seconds(a));
        let lead_seconds = lead_notes.as_ref().map(|notes| {
            let mut clipped = notes.iter().filter(|n| n.start<b && n.start+n.duration>a)
                .map(|n|[n.start.max(a),(n.start+n.duration).min(b)]).collect::<Vec<_>>();
            covered_seconds(&clock,&mut clipped)
        });
        json!({"beat_range":[a,b],"seconds":length,"name":authored.and_then(|part|part["name"].as_str()),
            "role":authored.and_then(|part|part["role"].as_str()),
            "enabled_tracks_with_material":counts[i],"lead_score_seconds":lead_seconds,
            "lead_gap_seconds":lead_seconds.map(|value|rounded((length-value).max(0.)))})
    }).collect::<Vec<_>>();
    json!({"project_id":project.id,"revision":project.revision,"sections":sections,"tracks":lanes,
        "lead_track_id":lead.map(|track|&track.id),"rhythm_comparable":comparable,
        "onset_bins_per_beat":24,"listening_points":cues,"requires_listening":true})
}

pub(crate) fn attach_audio(
    root: &std::path::Path,
    project: &Project,
    render: &Value,
    cue_index: usize,
    report: &mut Value,
) -> Result<()> {
    let cue = report["listening_points"]
        .get(cue_index)
        .context("听点编号不存在")?;
    let ranges = serde_json::from_value(cue["beat_ranges"].clone())?;
    let lead = report["lead_track_id"].as_str();
    let mut audio = crate::balance::section_audio(root, project, render, ranges, lead)?;
    audio["cue_index"] = json!(cue_index);
    audio["cue_kind"] = cue["kind"].clone();
    report["audio"] = audio;
    Ok(())
}

pub(crate) fn audio_index(args: &Value) -> Result<Option<usize>> {
    args.get("audio_cue")
        .map(|field| {
            field
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .context("请输入有效的听点编号")
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{uid, Store};

    #[test]
    fn measure_tempo_aware_lead_space_and_distinguish_repeated_hits() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_section_contrast"));
        let store = Store::new(root.clone())?;
        let tracks = ["Lead", "Keys", "Bass", "Drums"].map(Track::new);
        let [lead, keys, bass, drum] = tracks.each_ref().map(|track| track.id.clone());
        let p = store.create("Section review", 120., [4, 4], tracks.into())?;
        let drum_notes = (0..16)
            .map(|i| {
                json!({"start":f64::from(i)*0.5,"duration":0.15,
            "pitch":if i<8 {36}else{42}})
            })
            .collect::<Vec<_>>();
        let p = Store::preview_edit(
            &p,
            &json!([
                {"op":"set_project","values":{"tempo_map":[{"beat":4,"bpm":90}]}},
                {"op":"update_track","track_id":keys,"values":{"instrument":{"program":4}}},
                {"op":"update_track","track_id":bass,"values":{"instrument":{"program":33}}},
                {"op":"update_track","track_id":drum,"values":{"instrument":{"drums":true}}},
                {"op":"add_notes","track_id":lead,"notes":[{"start":0.5,"duration":1,"pitch":60},{"start":4.5,"duration":1,"pitch":60}]},
                {"op":"add_notes","track_id":keys,"notes":[{"start":0.75,"duration":0.5,"pitch":61},
                    {"start":0.75,"duration":0.5,"pitch":63},{"start":5.6,"duration":0.25,"pitch":48}]},
                {"op":"add_notes","track_id":bass,"notes":[{"start":4.25,"duration":1,"pitch":43}]},
                {"op":"add_notes","track_id":drum,"notes":drum_notes}
            ]),
        )?;
        let request = Request::read(&p, &json!({"beat_ranges":[[0,4],[4,8]]}))?;
        let view = report(&p, &request);
        assert_eq!(view["sections"][0]["seconds"], 2.);
        assert_eq!(view["sections"][1]["seconds"], 2.667);
        assert_eq!(
            view["tracks"][1]["sides"][0]["register_overlap_seconds"],
            0.25
        );
        assert_eq!(view["tracks"][1]["sides"][1]["near_release_attacks"], 1);
        assert_eq!(view["tracks"][3]["onset_similarity"], 1.);
        assert_eq!(view["tracks"][3]["drum_hit_similarity"], 0.);
        assert!(view["listening_points"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cue| cue["kind"] == "enters" && cue["track_id"] == bass));
        assert!(view["listening_points"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cue| cue["kind"] == "release_response"));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
