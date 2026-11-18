use crate::{
    clips::{ClipRef, GroupAction, RATE},
    model::Project,
    selection::Selection,
    timeline::Timeline,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Copy,
    Move,
    Clear,
    Transpose,
    Quantize,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuantizeSettings {
    step: f64,
    strength: f64,
    swing: f64,
}

pub(crate) fn quantize_schema() -> Value {
    json!({
        "type":"object","required":["step","strength","swing"],"additionalProperties":false,
        "properties":{
            "step":{"type":"number","minimum":0.0625,"maximum":4.0,"description":"Grid spacing in quarter-note beats"},
            "strength":{"type":"number","exclusiveMinimum":0.0,"maximum":1.0,"description":"Fraction of the distance to the target grid"},
            "swing":{"type":"number","minimum":0.5,"maximum":0.75,"description":"Position of the second subdivision in each pair. 0.5 gives a straight grid"}
        }
    })
}

impl QuantizeSettings {
    fn shift(self, clock: &Timeline, beat: f64) -> f64 {
        let [start, end] = clock.bar_bounds(beat);
        let pair = 2. * self.step;
        let origin = start + ((beat - start).max(0.) / pair).floor() * pair;
        let target = [
            origin,
            (origin + self.swing * pair).min(end),
            (origin + pair).min(end),
        ]
        .into_iter()
        .min_by(|a, b| (a - beat).abs().total_cmp(&(b - beat).abs()))
        .expect("three grid points");
        (target - beat) * self.strength
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RangeEdit {
    pub action: Action,
    pub track_ids: Vec<String>,
    pub source: [f64; 2],
    pub target_start: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub slice_audio: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semitones: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantize: Option<QuantizeSettings>,
}

pub(crate) struct Preview {
    pub id: String,
    pub summary: Value,
    pub(crate) next: Project,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoxRange {
    pub(crate) track_ids: Vec<String>,
    pub(crate) beat_range: [f64; 2],
}

impl BoxRange {
    pub(crate) fn read(project: &Project, args: &Value, default: Option<&Self>) -> Result<Self> {
        let track_ids = args
            .get("track_ids")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?
            .unwrap_or_else(|| {
                default.map_or_else(
                    || {
                        project
                            .tracks
                            .iter()
                            .map(|track| track.id.clone())
                            .collect()
                    },
                    |range| range.track_ids.clone(),
                )
            });
        let beat_range = args
            .get("beat_range")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?
            .or_else(|| default.map(|range| range.beat_range))
            .context("请提供起止拍或读取已框选的范围")?;
        let scope = Self {
            track_ids,
            beat_range,
        };
        scope.validate(project)?;
        Ok(scope)
    }

    pub(crate) fn validate(&self, project: &Project) -> Result<()> {
        checked_range(self.beat_range, project)?;
        let distinct = self.track_ids.iter().collect::<HashSet<_>>();
        ensure!(
            !distinct.is_empty()
                && distinct.len() == self.track_ids.len()
                && distinct
                    .iter()
                    .all(|id| project.tracks.iter().any(|track| &track.id == *id)),
            "请选择有效且不重复的音轨"
        );
        Ok(())
    }

    pub(crate) fn covers(&self, track_id: &str, [start, end]: [f64; 2]) -> bool {
        self.beat_range[0] <= start
            && end <= self.beat_range[1]
            && self.track_ids.iter().any(|id| id == track_id)
    }
}

pub(crate) struct Capture {
    pub(crate) range: BoxRange,
    pub(crate) selection: Selection,
    pub(crate) clips: Vec<ClipRef>,
}

impl Capture {
    pub(crate) fn bound(&self, project: &Project) -> Result<()> {
        ensure!(
            self.selection.project_id == project.id && self.selection.revision == project.revision,
            "BOX_STALE: 工程已更新，请重新读取当前框选"
        );
        Ok(())
    }

    pub(crate) fn view(&self, project: &Project, range: &BoxRange) -> Result<Value> {
        self.bound(project)?;
        ensure!(
            range
                .track_ids
                .iter()
                .all(|id| self.range.covers(id, range.beat_range)),
            "编排范围在框选之外"
        );
        let mut result = inventory(project, range, Some(self));
        result["box_id"] = json!(self.selection.id);
        Ok(result)
    }

    pub(crate) fn preview(
        &self,
        root: &Path,
        project: &Project,
        edit: &RangeEdit,
    ) -> Result<Preview> {
        self.bound(project)?;
        ensure!(
            edit.track_ids
                .iter()
                .all(|id| self.range.covers(id, edit.source)),
            "编排范围在框选之外"
        );
        let mut preview = preview(root, project, edit)?;
        let notes = self
            .selection
            .notes
            .iter()
            .map(|(track, note)| (track.as_str(), note.as_str()))
            .collect::<HashSet<_>>();
        for track in preview.summary["tracks"].as_array().into_iter().flatten() {
            ensure!(
                track["notes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|note| track["track_id"]
                        .as_str()
                        .zip(note["note_id"].as_str())
                        .is_some_and(|key| notes.contains(&key))),
                "音符在选区之外"
            );
        }
        let clips = self
            .clips
            .iter()
            .map(|clip| (clip.track_id.as_str(), clip.clip_id.as_str()))
            .collect::<HashSet<_>>();
        ensure!(
            preview.summary["clip_change"]["clips"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|clip| clip["track_id"]
                    .as_str()
                    .zip(clip["clip_id"].as_str())
                    .is_some_and(|key| clips.contains(&key))),
            "片段在选区之外"
        );
        preview.id = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&json!([preview.id, self.selection.id]))?)
        );
        preview.summary["preview_id"] = json!(preview.id);
        preview.summary["box_id"] = json!(self.selection.id);
        Ok(preview)
    }
}

fn overlaps(start: f64, end: f64, range: [f64; 2]) -> bool {
    start < range[1] - 1e-8 && end > range[0] + 1e-8
}

fn checked_range([start, end]: [f64; 2], project: &Project) -> Result<()> {
    ensure!(
        start.is_finite() && end.is_finite() && start >= 0. && end <= 50000. && end > start + 1e-8,
        "Range must contain increasing quarter-note beats"
    );
    ensure!(
        project.timeline().seconds(end) <= 900. + 1e-8,
        "Range exceeds 15 minutes"
    );
    Ok(())
}

fn frame_range(clock: &Timeline, beats: [f64; 2]) -> [i64; 2] {
    beats.map(|beat| (clock.seconds(beat) * f64::from(RATE)).round() as i64)
}

pub(crate) fn preview(root: &Path, project: &Project, edit: &RangeEdit) -> Result<Preview> {
    checked_range(edit.source, project)?;
    let [start, end] = edit.source;
    let semitones = if edit.action == Action::Transpose {
        ensure!(
            !edit.slice_audio && edit.target_start.is_none(),
            "移调需要原地操作，且不会裁剪音频"
        );
        let shift = edit.semitones.context("请输入移调半音数")?;
        ensure!(
            (-127..=127).contains(&shift) && shift != 0,
            "移调须为 ±1 至 ±127 个半音"
        );
        Some(shift)
    } else {
        ensure!(
            edit.semitones.is_none(),
            "semitones is only valid for transpose"
        );
        None
    };
    let quantize = if edit.action == Action::Quantize {
        ensure!(
            !edit.slice_audio && edit.target_start.is_none(),
            "量化需要在所选范围内调整音符起点"
        );
        let settings = edit.quantize.context("请选择量化网格、强度与摇摆")?;
        ensure!(
            (0.0625..=4.).contains(&settings.step)
                && settings.strength > 0.
                && settings.strength <= 1.
                && (0.5..=0.75).contains(&settings.swing),
            "量化参数无效：网格 1/16 至 4 拍，强度大于 0% 至 100%，摇摆 50% 至 75%"
        );
        Some(settings)
    } else {
        ensure!(
            edit.quantize.is_none(),
            "quantize settings require action quantize"
        );
        None
    };
    let target = match edit.action {
        Action::Transpose | Action::Quantize => None,
        Action::Clear => {
            ensure!(edit.target_start.is_none(), "Clear has no destination");
            None
        }
        _ => {
            let anchor = edit.target_start.context("Destination start is required")?;
            let range = [anchor, anchor + end - start];
            checked_range(range, project)?;
            ensure!(
                (anchor - start).abs() > 1e-8,
                "Source and destination start at the same beat"
            );
            Some(range)
        }
    };
    ensure!(
        !edit.track_ids.is_empty() && edit.track_ids.len() <= project.tracks.len(),
        "Select one or more existing tracks"
    );
    let by_id: HashMap<_, _> = project
        .tracks
        .iter()
        .map(|track| (track.id.as_str(), track))
        .collect();
    let mut unique = HashSet::new();
    for id in &edit.track_ids {
        by_id
            .get(id.as_str())
            .context("Unknown track in arrangement range")?;
        ensure!(
            unique.insert(id.as_str()),
            "Duplicate track in arrangement range"
        );
    }

    // Scope, action, project and revision identify the reviewed preview and its copies.
    let id = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!([
            project.id,
            project.revision,
            edit
        ]))?)
    );
    let mut next = project.clone();
    if next.format == "agent-vocal/1" {
        next.format = "agent-vocal/2".into();
    }
    let mut tracks = vec![];
    let mut total = 0;
    let mut max_shift_ms: f64 = 0.;
    let mut warnings = vec![];
    let clock = project.timeline();
    let [source_first, source_end] = frame_range(&clock, edit.source);
    let mut refs = Vec::new();
    for (original, track) in project.tracks.iter().zip(&mut next.tracks) {
        if !unique.contains(track.id.as_str()) {
            continue;
        }
        if semitones.is_some() {
            let skipped = if track.is_audio() {
                Some((
                    "audio",
                    track
                        .media_clips()
                        .iter()
                        .filter(|clip| {
                            clip.start_frame(&clock) < source_end
                                && clip.end_frame(&clock) > source_first
                        })
                        .count(),
                ))
            } else if track
                .instrument
                .as_ref()
                .is_some_and(|instrument| instrument.drums)
            {
                Some((
                    "drums",
                    track
                        .notes
                        .iter()
                        .filter(|note| {
                            overlaps(note.start, note.start + note.duration, edit.source)
                        })
                        .count(),
                ))
            } else {
                None
            };
            if let Some((track_type, count)) = skipped {
                if count > 0 {
                    warnings.push(json!({"kind":"transpose_skipped","track_type":track_type,
                        "track_id":track.id,"name":track.name,"count":count}));
                }
                continue;
            }
        }
        if track.is_audio() {
            let clips = track.media_clips();
            if quantize.is_some() {
                let count = clips
                    .iter()
                    .filter(|clip| {
                        clip.start_frame(&clock) < source_end
                            && clip.end_frame(&clock) > source_first
                    })
                    .count();
                if count > 0 {
                    warnings.push(json!({"kind":"quantize_audio_stays","track_id":track.id,"name":track.name,"count":count}));
                }
                continue;
            }
            ensure!(
                !edit.slice_audio || source_first < source_end,
                "音频范围需要覆盖至少一个采样帧"
            );
            let first = refs.len();
            for clip in clips.iter().filter(|clip| {
                clip.start_frame(&clock) < source_end && clip.end_frame(&clock) > source_first
            }) {
                ensure!(
                    edit.slice_audio
                        || (clip.start_frame(&clock) >= source_first
                            && clip.end_frame(&clock) <= source_end),
                    "RANGE_CUTS_CLIP: {} / {}，请框选完整片段或先切分",
                    track.name,
                    clip.name
                );
                refs.push(ClipRef {
                    track_id: track.id.clone(),
                    clip_id: clip.id.clone(),
                });
            }
            if refs.len() > first {
                if let Some([left, right]) = target.map(|range| frame_range(&clock, range)) {
                    let selected: HashSet<_> =
                        refs[first..].iter().map(|r| r.clip_id.as_str()).collect();
                    if let Some(clip) = clips.iter().find(|clip| {
                        let begin = clip.start_frame(&clock);
                        let end = clip.end_frame(&clock);
                        let intersects = |a: i64, b: i64| a < b && a < right && b > left;
                        if edit.action == Action::Move && selected.contains(clip.id.as_str()) {
                            intersects(begin, end.min(source_first))
                                || intersects(begin.max(source_end), end)
                        } else {
                            intersects(begin, end)
                        }
                    }) {
                        anyhow::bail!("DESTINATION_OCCUPIED: {} / {}", track.name, clip.name);
                    }
                }
                tracks.push(json!({"track_id":track.id,"name":track.name,"count":0,
                    "notes":[],"clip_count":refs.len()-first}));
            }
            continue;
        }
        let mut selected = HashSet::new();
        let mut note_changes = vec![];
        for note in &track.notes {
            let note_end = note.start + note.duration;
            if !overlaps(note.start, note_end, edit.source) {
                continue;
            }
            ensure!(note.start >= start - 1e-8 && note_end <= end + 1e-8,
                "RANGE_CUTS_NOTE: {} / {} at {:.4}–{:.4}; expand the source to include the whole note", track.name, note.id, note.start, note_end);
            selected.insert(note.id.clone());
        }
        if selected.is_empty() {
            continue;
        }
        let mut ordered = track.notes.iter().collect::<Vec<_>>();
        ordered.sort_by(|a, b| a.start.total_cmp(&b.start));
        for pair in ordered.windows(2).filter(|pair| pair[1].vowel_continuation) {
            ensure!(
                selected.contains(&pair[0].id) == selected.contains(&pair[1].id),
                "RANGE_CUTS_SYLLABLE: {}，请框选该读音的起音和所有延音",
                track.name
            );
        }
        if let Some(dest) = target {
            // Region paste requires an empty destination. A move may overlap its own source.
            if let Some(collision) = track.notes.iter().find(|n| {
                !(edit.action == Action::Move && selected.contains(&n.id))
                    && overlaps(n.start, n.start + n.duration, dest)
            }) {
                anyhow::bail!(
                    "DESTINATION_OCCUPIED: {} / {} at {:.4}–{:.4}",
                    track.name,
                    collision.id,
                    collision.start,
                    collision.start + collision.duration
                );
            }
        }
        let mut additions = vec![];
        let mut copied_ids = HashMap::new();
        let mut pitches = [127, 0];
        let mut shifts = HashMap::new();
        let mut track_shift_ms: f64 = 0.;
        if let Some(settings) = quantize {
            let mut delta = 0.;
            for note in ordered
                .into_iter()
                .filter(|note| selected.contains(&note.id))
            {
                if !note.vowel_continuation {
                    delta = settings.shift(&clock, note.start);
                }
                if delta.abs() > 1e-8 {
                    shifts.insert(note.id.clone(), delta);
                }
            }
            if shifts.is_empty() {
                continue;
            }
        }
        for note in &mut track.notes {
            if !selected.contains(&note.id) {
                continue;
            }
            let before = note.start;
            if quantize.is_some() {
                let Some(delta) = shifts.get(&note.id) else {
                    continue;
                };
                let after = before + delta;
                ensure!(
                    after >= start - 1e-8 && after + note.duration <= end + 1e-8,
                    "QUANTIZE_OUTSIDE_RANGE: {} / {}，请扩大框选范围或降低量化强度",
                    track.name,
                    note.id
                );
                let shift_ms = 1000. * (clock.seconds(after) - clock.seconds(before));
                track_shift_ms = track_shift_ms.max(shift_ms.abs());
                note.start = after;
                note_changes.push(
                    json!({"note_id":note.id,"result_note_id":note.id,"pitch":note.pitch,
                    "duration":note.duration,"before":before,"after":after,"shift_ms":shift_ms}),
                );
                continue;
            }
            if let Some(shift) = semitones {
                let pitch = note.pitch;
                let after = i16::from(pitch) + shift;
                ensure!(
                    (0..=127).contains(&after),
                    "PITCH_OUT_OF_RANGE: {} / {}（MIDI {} → {}）",
                    track.name,
                    note.id,
                    pitch,
                    after
                );
                note.pitch = after as u8;
                pitches = [pitches[0].min(pitch), pitches[1].max(pitch)];
                note_changes.push(json!({"note_id":note.id,"result_note_id":note.id,"pitch":pitch,
                    "pitch_after":note.pitch,"duration":note.duration,"before":before,"after":before}));
                continue;
            }
            let after = target.map(|dest| dest[0] + before - start);
            let mut result_id = None;
            if let Some(after) = after {
                match edit.action {
                    Action::Copy => {
                        let mut cloned = note.clone();
                        cloned.id = format!("n_{:x}", Sha256::digest(format!("{id}:{}", note.id)));
                        result_id = Some(cloned.id.clone());
                        if !track.lyric_lines.is_empty() {
                            copied_ids.insert(note.id.clone(), cloned.id.clone());
                        }
                        cloned.start = after;
                        additions.push(cloned);
                    }
                    Action::Move => {
                        note.start = after;
                        result_id = Some(note.id.clone());
                    }
                    Action::Clear | Action::Transpose | Action::Quantize => unreachable!(),
                }
            }
            note_changes.push(json!({"note_id":note.id,"result_note_id":result_id,"pitch":note.pitch,"duration":note.duration,"before":before,"after":after}));
        }
        if edit.action == Action::Clear {
            track.notes.retain(|note| !selected.contains(&note.id));
        }
        track.notes.extend(additions);
        let copied_lines = if copied_ids.is_empty() {
            0
        } else {
            crate::lyric_lines::copy_complete(track, &copied_ids)?
        };
        track.notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        let controller_count = track
            .controllers
            .iter()
            .filter(|cc| {
                (cc.beat >= start && cc.beat < end)
                    || target.is_some_and(|[a, b]| cc.beat >= a && cc.beat < b)
            })
            .count();
        if controller_count > 0 && semitones.is_none() {
            warnings.push(
                json!({"track_id":track.id,"kind":"controllers_stay","count":controller_count}),
            );
        }
        total += note_changes.len();
        let mut row = json!({"track_id":track.id,"name":track.name,"count":note_changes.len(),"notes":note_changes,"clip_count":0});
        if copied_lines > 0 {
            row["copied_lyric_lines"] = json!(copied_lines);
        }
        let lyric_changes = crate::lyric_lines::arrangement_impact(
            original,
            track,
            edit.action,
            &selected,
            &copied_ids,
        );
        if !lyric_changes.is_empty() {
            row["lyric_changes"] = json!(lyric_changes);
        }
        if let Some(shift) = semitones {
            row["pitch_range"] =
                json!({"before":pitches,"after":pitches.map(|pitch| i16::from(pitch) + shift)});
        }
        if quantize.is_some() {
            row["max_shift_ms"] = json!(track_shift_ms);
            max_shift_ms = max_shift_ms.max(track_shift_ms);
        }
        tracks.push(row);
    }
    if semitones.is_some() {
        ensure!(total > 0, "所选范围内没有可移调的完整歌声或乐器音符");
    }
    if quantize.is_some() {
        ensure!(total > 0, "所选范围内没有需要量化的完整音符");
    }
    ensure!(
        total > 0 || !refs.is_empty(),
        "所选范围内没有完整音符或片段"
    );
    let mut clip_change = None;
    if !refs.is_empty() {
        let delta = target.map_or(0., |range| range[0] - start);
        let action = match edit.action {
            Action::Move => GroupAction::Move {
                delta,
                lane_offset: 0,
            },
            Action::Copy => GroupAction::Copy {
                delta,
                lane_offset: 0,
            },
            Action::Clear => GroupAction::Remove,
            Action::Transpose | Action::Quantize => unreachable!(),
        };
        let (candidate, mut summary) = crate::clips::plan_group_window(
            root,
            &next,
            &refs,
            &action,
            edit.slice_audio.then_some(edit.source),
        )?;
        if let Some(range) = target {
            let [first, end] = frame_range(&clock, range);
            for row in summary["after"]
                .as_array()
                .context("Missing clip placements")?
            {
                ensure!(row["start_frame"].as_i64().is_some_and(|at| at >= first)
                    && row["end_frame"].as_i64().is_some_and(|at| at <= end),
                    "DESTINATION_CLIP_SPILL: {}，片段按固定秒数延伸到了目标范围外，请扩大范围或使用跟速",
                    row["clip"]["name"].as_str().unwrap_or("音频片段"));
            }
        }
        summary["action"] = serde_json::to_value(action)?;
        clip_change = Some(summary);
        next = candidate;
    }
    if semitones.is_none()
        && next
            .tracks
            .iter()
            .filter(|track| unique.contains(track.id.as_str()))
            .any(|track| !track.gain_automation.is_empty())
    {
        warnings.push(json!({"kind":"gain_automation_stays"}));
    }
    if quantize.is_some() {
        next.validate()
            .context("量化后的音符无法保存，请检查重叠与延音衔接")?;
    } else {
        next.validate()?;
    }
    let cut_count = clip_change
        .as_ref()
        .and_then(|change| change["cuts"].as_array())
        .map_or(0, Vec::len);
    let mut layers = vec![
        "midi_controllers",
        "gain_automation",
        "tempo_map",
        "meter_map",
        "composition_sections",
    ];
    if semitones.is_some() {
        layers.extend(["harmony_markers", "drums", "audio_clips"]);
    }
    if quantize.is_some() {
        layers.extend(["harmony_markers", "audio_clips"]);
    }
    let mut summary = json!({"preview_id":id,"project_id":project.id,"revision":project.revision,"source":edit.source,"destination":target,"tracks":tracks,"note_count":total,"clip_count":refs.len(),"cut_count":cut_count,"clip_change":clip_change,"action":edit.action,"warnings":warnings,"unchanged_layers":layers});
    if let Some(shift) = semitones {
        summary["semitones"] = json!(shift);
    }
    if let Some(settings) = quantize {
        summary["quantize"] = json!(settings);
        summary["max_shift_ms"] = json!(max_shift_ms);
    }
    Ok(Preview { summary, id, next })
}

pub(crate) fn section_markers(project: &Project) -> Vec<Value> {
    let clock = project.timeline();
    let explicit = project
        .section_markers
        .as_ref()
        .map(|markers| json!(markers));
    explicit
        .as_ref()
        .or_else(|| {
            project
                .composition
                .as_ref()
                .and_then(|plan| plan.get("sections"))
        })
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|section| {
            let Some(start) = section["start"].as_f64() else {
                return false;
            };
            let Some(end) = section["end"].as_f64() else {
                return false;
            };
            start >= 0. && end > start && clock.seconds(end) <= 900.
        })
        .cloned()
        .collect()
}

pub(crate) fn query(project: &Project) -> Value {
    let clock = project.timeline();
    let sections: Vec<_> = section_markers(project).into_iter()
        .map(|section| {
            let start=section["start"].as_f64().expect("validated section start");
            let end=section["end"].as_f64().expect("validated section end");
            let tracks: Vec<_>=project.tracks.iter().filter_map(|track| {
                let count=track.notes.iter().filter(|note|overlaps(note.start,note.start+note.duration,[start,end])).count();
                let audio=track.media_clips().iter().any(|clip| overlaps(clip.start,clip.beat_span(&clock)[1],[start,end]));
                (count>0||audio).then(||json!({"track_id":track.id,"note_count":count,"audio":audio}))
            }).collect();
            json!({"name":section["name"],"start":start,"end":end,"start_seconds":clock.seconds(start),"end_seconds":clock.seconds(end),"role":section["role"],"tracks":tracks})
        }).collect();
    let tracks: Vec<_> = project.tracks.iter().map(|track| {
        let clip_spans:Vec<_>=track.media_clips().iter().map(|clip|clip.beat_span(&clock)).collect();
        let audio_span=clip_spans.iter().copied().reduce(|a,b|[a[0].min(b[0]),a[1].max(b[1])]);
        json!({"track_id":track.id,"name":track.name,"kind":if track.is_audio(){"audio"}else if track.instrument.is_some(){"instrument"}else{"voice"},
            "note_count":track.notes.len(),"controller_count":track.controllers.len(),"audio_span":audio_span,
            "audio_clips":clip_spans,
            "start":track.notes.first().map(|note|note.start),"end":track.notes.iter().map(|note|note.start+note.duration).reduce(f64::max)})
    }).collect();
    json!({"project_id":project.id,"revision":project.revision,"tracks":tracks,"sections":sections,"sections_from_design":project.section_markers.is_none(),"duration_seconds":project.seconds(),"time_unit":"quarter_note_beats"})
}

pub(crate) fn inventory(project: &Project, range: &BoxRange, capture: Option<&Capture>) -> Value {
    let clock = project.timeline();
    let bounds = range.beat_range;
    let [first_frame, end_frame] = frame_range(&clock, bounds);
    let note_ids = capture.map(|item| {
        item.selection
            .notes
            .iter()
            .map(|(track, id)| (track.as_str(), id.as_str()))
            .collect::<HashSet<_>>()
    });
    let clip_ids = capture.map(|item| {
        item.clips
            .iter()
            .map(|clip| (clip.track_id.as_str(), clip.clip_id.as_str()))
            .collect::<HashSet<_>>()
    });
    let mut rows = Vec::new();
    let mut details_remaining = 120_usize;
    for track in project
        .tracks
        .iter()
        .filter(|track| range.track_ids.contains(&track.id))
    {
        let track_id = track.id.as_str();
        let whole = |note: &crate::model::Note| {
            note.start >= bounds[0] - 1e-8 && note.start + note.duration <= bounds[1] + 1e-8
        };
        let selected = |note: &crate::model::Note| {
            whole(note)
                && note_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&(track_id, note.id.as_str())))
        };
        let mut notes = Vec::new();
        let mut note_count = 0;
        let mut boundary_count = 0;
        let mut uncaptured_notes = 0;
        let mut pitch_range = None::<[u8; 2]>;
        for note in track
            .notes
            .iter()
            .filter(|note| overlaps(note.start, note.start + note.duration, bounds))
        {
            if !whole(note) {
                boundary_count += 1;
            } else if !selected(note) {
                uncaptured_notes += 1;
            } else {
                note_count += 1;
                let [low, high] = pitch_range.unwrap_or([note.pitch; 2]);
                pitch_range = Some([low.min(note.pitch), high.max(note.pitch)]);
                if details_remaining > 0 && notes.len() < 24 {
                    notes.push(
                        json!({"note_id":note.id,"beat_range":[note.start,note.start+note.duration],
                        "pitch":note.pitch,"vowel_continuation":note.vowel_continuation}),
                    );
                    details_remaining -= 1;
                }
            }
        }
        let cut_syllables = track
            .notes
            .windows(2)
            .filter(|pair| pair[1].vowel_continuation && selected(&pair[0]) != selected(&pair[1]))
            .count();
        let mut clip_count = 0;
        let mut uncaptured_clips = 0;
        let mut partial_clips = 0;
        let mut clips = Vec::new();
        for clip in track.media_clips().iter() {
            let span = [clip.start_frame(&clock), clip.end_frame(&clock)];
            if span[0] >= end_frame || span[1] <= first_frame || first_frame >= end_frame {
                continue;
            }
            if clip_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(&(track_id, clip.id.as_str())))
            {
                uncaptured_clips += 1;
                continue;
            }
            clip_count += 1;
            let partial = span[0] < first_frame || span[1] > end_frame;
            partial_clips += usize::from(partial);
            if details_remaining > 0 && clips.len() < 24 {
                let [start, end] = clip.beat_span(&clock);
                clips.push(json!({"clip_id":clip.id,"name":clip.name,
                    "beat_range":[start.max(bounds[0]),end.min(bounds[1])],"partial":partial,
                    "timing":if !clip.warp_points.is_empty(){"warped"}else if clip.beat_length.is_some(){"follow"}else{"fixed"}}));
                details_remaining -= 1;
            }
        }
        let kind = match &track.instrument {
            _ if track.is_audio() => "audio",
            Some(patch) if patch.drums => "drums",
            Some(_) => "instrument",
            None => "voice",
        };
        rows.push(json!({"track_id":track.id,"name":track.name,"kind":kind,"muted":track.mute,
            "note_count":note_count,"notes":notes,"boundary_note_count":boundary_count,"uncaptured_note_count":uncaptured_notes,
            "syllable_boundary_count":cut_syllables,"pitch_range":pitch_range,"clip_count":clip_count,"clips":clips,
            "partial_clip_count":partial_clips,"uncaptured_clip_count":uncaptured_clips,
            "details_truncated":note_count>notes.len() || clip_count>clips.len()}));
    }
    json!({"project_id":project.id,"revision":project.revision,"beat_range":bounds,"track_ids":range.track_ids,
        "seconds_range":bounds.map(|beat|clock.seconds(beat)),"time_unit":"quarter_note_beats","tracks":rows})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{uid, Store, Track};

    #[test]
    fn reports_a_cut_syllable_without_exposing_the_unselected_head() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_arrange_box"));
        let store = Store::new(root.clone())?;
        let track = Track::new("Voice");
        let id = track.id.clone();
        let base = store.create("Box boundaries", 120., [4, 4], vec![track])?;
        let project = store.edit(
            &base.id,
            base.revision,
            &json!([{"op":"add_notes","track_id":id,"notes":[
                {"id":"voice_head","start":0.75,"duration":0.5,"pitch":60,"lyric":"か"},
                {"id":"voice_tail","start":1.25,"duration":0.5,"pitch":65,"vowel_continuation":true}
            ]}]),
        )?;
        let capture = Capture {
            range: BoxRange {
                track_ids: vec![id.clone()],
                beat_range: [1., 1.75],
            },
            selection: Selection {
                id: "ui_fixture".into(),
                project_id: project.id.clone(),
                revision: project.revision,
                notes: vec![(id, "voice_tail".into())],
            },
            clips: vec![],
        };
        let view = capture.view(&project, &capture.range)?;
        let lane = &view["tracks"][0];
        assert_eq!(lane["note_count"], 1);
        assert_eq!(lane["boundary_note_count"], 1);
        assert_eq!(lane["syllable_boundary_count"], 1);
        assert_eq!(lane["notes"][0]["note_id"], "voice_tail");
        assert!(!view.to_string().contains("voice_head"));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
