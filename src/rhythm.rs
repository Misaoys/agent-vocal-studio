use crate::{
    clips::RATE,
    model::{field, Note, Project, Store, Track},
    timeline::Timeline,
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

const MAX_ROWS: usize = 96;

pub(crate) fn fields() -> Value {
    json!({"drum_track_id":{"type":"string"},"bass_track_id":{"type":"string"},
        "beat_range":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2},
        "kick_pitches":{"type":"array","items":{"type":"integer","minimum":0,"maximum":127},"minItems":1,"maxItems":16,"uniqueItems":true},
        "tolerance_ms":{"type":"number","minimum":1,"maximum":300},
        "placement":{"type":"object","additionalProperties":false,"required":["kick_note_id","bass_note_id","offset_ms"],
            "properties":{"kick_note_id":{"type":"string"},"bass_note_id":{"type":"string"},
                "offset_ms":{"type":"number"},"length_ms":{"type":"number","exclusiveMinimum":0}}}})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Placement {
    pub kick_note_id: String,
    pub bass_note_id: String,
    offset_ms: f64,
    length_ms: Option<f64>,
}

pub(crate) struct Request<'a> {
    pub range: [f64; 2],
    pub drums: &'a Track,
    pub bass: &'a Track,
    pub placement: Option<Placement>,
    kick_pitches: Vec<u8>,
    tolerance_ms: f64,
}

impl<'a> Request<'a> {
    pub(crate) fn read(project: &'a Project, args: &Value) -> Result<Self> {
        let [start, end]: [f64; 2] =
            serde_json::from_value(args["beat_range"].clone()).context("提供起拍与止拍")?;
        ensure!(
            start >= 0. && end > start && end <= 50_000.,
            "鼓贝斯拍范围无效"
        );
        let track = |key| -> Result<&Track> {
            let id = field(args, key)?;
            project
                .tracks
                .iter()
                .find(|track| track.id == id)
                .context("找不到指定的节奏组音轨")
        };
        let drums = track("drum_track_id")?;
        let bass = track("bass_track_id")?;
        ensure!(
            drums.instrument.as_ref().is_some_and(|i| i.drums)
                && bass.instrument.as_ref().is_some_and(|i| !i.drums),
            "需要一条鼓组轨和一条有音乐器轨"
        );
        let kick_pitches: Vec<u8> = args
            .get("kick_pitches")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_else(|| vec![35, 36]);
        let unique = kick_pitches.iter().collect::<HashSet<_>>();
        ensure!(
            !kick_pitches.is_empty()
                && kick_pitches.len() <= 16
                && unique.len() == kick_pitches.len()
                && kick_pitches.iter().all(|&pitch| pitch <= 127),
            "底鼓音高需使用 1–16 个不重复的 MIDI 音高"
        );
        let tolerance_ms = args
            .get("tolerance_ms")
            .map(|v| v.as_f64().context("配对距离需为数字"))
            .transpose()?
            .unwrap_or(75.);
        ensure!(
            (1. ..=300.).contains(&tolerance_ms),
            "配对距离须为 1–300 毫秒"
        );
        let placement = args
            .get("placement")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        Ok(Self {
            range: [start, end],
            drums,
            bass,
            placement,
            kick_pitches,
            tolerance_ms,
        })
    }
}

struct Attack<'a> {
    frame: i64,
    notes: Vec<&'a Note>,
}

fn frame(clock: &Timeline, beat: f64) -> i64 {
    (clock.seconds(beat) * f64::from(RATE)).round() as i64
}

fn attacks<'a>(
    notes: impl Iterator<Item = &'a Note>,
    clock: &Timeline,
    [start, end]: [f64; 2],
) -> Vec<Attack<'a>> {
    let mut picked = notes
        .filter(|note| note.start >= start && note.start < end)
        .collect::<Vec<_>>();
    picked.sort_unstable_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    let mut attacks: Vec<Attack<'_>> = vec![];
    for note in picked {
        let frame = frame(clock, note.start);
        if let Some(last) = attacks.last_mut().filter(|item| item.frame == frame) {
            last.notes.push(note);
        } else {
            attacks.push(Attack {
                frame,
                notes: vec![note],
            });
        }
    }
    attacks
}

fn nearest(source: &[Attack<'_>], frame: i64, tolerance: i64) -> Option<usize> {
    let right = source.partition_point(|hit| hit.frame < frame);
    [right.checked_sub(1), Some(right)]
        .into_iter()
        .flatten()
        .filter_map(|i| source.get(i).map(|hit| (i, (hit.frame - frame).abs())))
        .filter(|&(_, distance)| distance <= tolerance)
        .min_by_key(|&(i, distance)| (distance, i))
        .map(|(i, _)| i)
}

fn ms(frames: i64) -> f64 {
    (frames as f64 * 10000. / f64::from(RATE)).round() / 10.
}

fn attack_view(attack: &Attack<'_>, clock: &Timeline, limit: i64) -> Value {
    let notes = attack
        .notes
        .iter()
        .map(|note| {
            json!({"note_id":note.id,"pitch":note.pitch,"velocity":note.velocity,
            "length_in_scope_ms":ms(frame(clock,note.start+note.duration).min(limit)-attack.frame)})
        })
        .collect::<Vec<_>>();
    json!({"beat":attack.notes[0].start,"notes":notes})
}

fn overlaps(a: [f64; 2], b: [f64; 2]) -> bool {
    a[0] < b[1] - 1e-8 && b[0] < a[1] - 1e-8
}

fn placement_preview(
    project: &Project,
    request: &Request<'_>,
    placement: &Placement,
    clock: &Timeline,
) -> Result<Value> {
    let [from, to] = request.range;
    let kick = request
        .drums
        .notes
        .iter()
        .find(|n| n.id == placement.kick_note_id && request.kick_pitches.contains(&n.pitch))
        .context("指定音符不是当前选择的底鼓")?;
    let bass = request
        .bass
        .notes
        .iter()
        .find(|n| n.id == placement.bass_note_id)
        .context("找不到贝斯音符")?;
    let onsets = |n: &Note| n.start >= from && n.start < to;
    let old_end = bass.start + bass.duration;
    ensure!(
        onsets(kick) && onsets(bass) && old_end <= to + 1e-8,
        "底鼓和完整贝斯音符须位于当前拍段"
    );
    let old_seconds = clock.seconds(old_end) - clock.seconds(bass.start);
    let start_seconds = clock.seconds(kick.start) + placement.offset_ms / 1000.;
    let end_seconds = start_seconds
        + placement
            .length_ms
            .map_or(old_seconds, |value| value / 1000.);
    ensure!(
        start_seconds.is_finite()
            && end_seconds.is_finite()
            && end_seconds > start_seconds
            && start_seconds >= clock.seconds(from)
            && end_seconds <= clock.seconds(to) + 1e-9,
        "移动后贝斯音符须完整留在当前拍段"
    );
    let next = [clock.beat(start_seconds), clock.beat(end_seconds)];
    ensure!(
        request
            .bass
            .notes
            .iter()
            .filter(|note| note.id != bass.id)
            .all(|note| {
                let other = [note.start, note.start + note.duration];
                !overlaps(next, other) || overlaps([bass.start, old_end], other)
            }),
        "移动会使贝斯与另一个音符发生新的重叠"
    );
    let operations = json!([{"op":"update_note","track_id":request.bass.id,"note_id":bass.id,
        "values":{"start":next[0],"duration":next[1]-next[0]}}]);
    Store::preview_edit_with_refs(project, &operations)?;
    Ok(
        json!({"operations":operations,"before_beat_range":[bass.start,old_end],"after_beat_range":next,
        "offset_ms":ms(frame(clock,next[0])-frame(clock,kick.start)),
        "length_ms":ms(frame(clock,next[1])-frame(clock,next[0]))}),
    )
}

pub(crate) fn report(project: &Project, request: &Request<'_>) -> Result<Value> {
    let clock = project.timeline();
    let [from, to] = request.range;
    if let Some(placement) = &request.placement {
        return Ok(
            json!({"project_id":project.id,"revision":project.revision,"beat_range":request.range,
            "placement":placement_preview(project,request,placement,&clock)?}),
        );
    }
    let kicks = attacks(
        request
            .drums
            .notes
            .iter()
            .filter(|note| request.kick_pitches.contains(&note.pitch)),
        &clock,
        request.range,
    );
    let bass = attacks(request.bass.notes.iter(), &clock, request.range);
    let tolerance = (request.tolerance_ms * f64::from(RATE) / 1000.).round() as i64;
    let mut events = vec![];
    let mut paired_bass = HashSet::new();
    let mut signed = vec![];
    let limit = frame(&clock, to);
    let listen = |at: f64| {
        let start = (clock.seconds(at) - 0.4).max(clock.seconds(from));
        [
            clock.beat(start).max(from),
            clock.beat((start + 2.8).min(clock.seconds(to))).min(to),
        ]
    };
    let mut releases: Vec<_> = request
        .bass
        .notes
        .iter()
        .filter(|note| note.start < to && note.start + note.duration > from)
        .map(|n| (frame(&clock, n.start), frame(&clock, n.start + n.duration)))
        .collect();
    releases.sort_unstable_by_key(|&(start, _)| start);
    let mut release = 0_i64;
    let mut release_index = 0;
    let mut held_without_attack = 0;
    for (i, kick) in kicks.iter().enumerate() {
        while releases
            .get(release_index)
            .is_some_and(|&(start, _)| start < kick.frame)
        {
            release = release.max(releases[release_index].1);
            release_index += 1;
        }
        let other = nearest(&bass, kick.frame, tolerance)
            .filter(|&j| nearest(&kicks, bass[j].frame, tolerance) == Some(i));
        if let Some(j) = other {
            paired_bass.insert(j);
            signed.push(bass[j].frame - kick.frame);
        } else if release > kick.frame {
            held_without_attack += 1;
        }
        if i < MAX_ROWS {
            let at = kick.notes[0].start;
            events.push((kick.frame,json!({"kind":if other.is_some(){"paired"}else{"kick_only"},
                "kick":attack_view(kick,&clock,limit),"bass":other.map(|j|attack_view(&bass[j],&clock,limit)),
                "offset_ms":other.map(|j| ms(bass[j].frame-kick.frame)),
                "held_bass_at_kick":release>kick.frame,"listen_range":listen(at)})));
        }
    }
    for (_, hit) in bass
        .iter()
        .enumerate()
        .filter(|(i, _)| !paired_bass.contains(i))
        .take(MAX_ROWS)
    {
        events.push((
            hit.frame,
            json!({"kind":"bass_only","kick":null,"bass":attack_view(hit,&clock,limit),
            "listen_range":listen(hit.notes[0].start)}),
        ));
    }
    let count = kicks.len() + bass.len() - paired_bass.len();
    events.sort_by_key(|(frame, _)| *frame);
    signed.sort_unstable();
    let mid = signed.len() / 2;
    let median =
        (!signed.is_empty()).then(|| (ms(signed[mid]) + ms(signed[(signed.len() - 1) / 2])) * 0.5);
    Ok(
        json!({"project_id":project.id,"revision":project.revision,"beat_range":request.range,
        "drum_track_id":request.drums.id,"bass_track_id":request.bass.id,"tolerance_ms":request.tolerance_ms,
        "kick_pitches":request.kick_pitches,"summary":{"kick_attacks":kicks.len(),"bass_attacks":bass.len(),
            "paired":paired_bass.len(),"kick_only":kicks.len()-paired_bass.len(),"bass_only":bass.len()-paired_bass.len(),
            "held_at_unmatched_kicks":held_without_attack,"median_offset_ms":median},
        "events":events.into_iter().take(MAX_ROWS).map(|(_,view)|view).collect::<Vec<_>>(),
        "events_truncated":count>MAX_ROWS,"timing_source":"score_at_48khz","requires_listening":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::uid;

    #[test]
    fn distinguishes_early_late_and_held_bass_and_previews_across_tempo() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_rhythm"));
        let store = Store::new(root.clone())?;
        let [drum, bass] = ["Kick", "Bass"].map(Track::new);
        let [d, b] = [drum.id.clone(), bass.id.clone()];
        let p = store.create("Rhythm timing", 120., [4, 4], vec![drum, bass])?;
        let p = Store::preview_edit(
            &p,
            &json!([
                {"op":"set_project","values":{"tempo_map":[{"beat":4,"bpm":90}]}},
                {"op":"update_track","track_id":d,"values":{"instrument":{"drums":true}}},
                {"op":"update_track","track_id":b,"values":{"instrument":{"program":34}}},
                {"op":"add_notes","track_id":d,"notes":[
                    {"start":2,"duration":0.1,"pitch":36},{"start":3,"duration":0.1,"pitch":36},
                    {"start":4,"duration":0.1,"pitch":36},{"start":4,"duration":0.1,"pitch":35}]},
                {"op":"add_notes","track_id":b,"notes":[{"start":1.964,"duration":1.2,"pitch":43},
                    {"start":4.03,"duration":0.5,"pitch":45},{"start":5,"duration":0.25,"pitch":48}]}
            ]),
        )?;
        let args = json!({"beat_range":[1,6],"drum_track_id":d,"bass_track_id":b});
        let reading = report(&p, &Request::read(&p, &args)?)?;
        assert_eq!(reading["summary"]["paired"], 2);
        assert_eq!(reading["summary"]["kick_attacks"], 3);
        assert_eq!(reading["summary"]["held_at_unmatched_kicks"], 1);
        assert_eq!(reading["events"][0]["offset_ms"], -18.);
        assert_eq!(reading["events"][2]["offset_ms"], 20.);
        assert_eq!(
            reading["events"][2]["kick"]["notes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let mut target = args;
        target["placement"] = json!({"kick_note_id":p.tracks[0].notes[2].id,
            "bass_note_id":p.tracks[1].notes[1].id,"offset_ms":-24.});
        let draft = report(&p, &Request::read(&p, &target)?)?;
        assert_eq!(draft["placement"]["offset_ms"], -24.);
        assert_eq!(draft["placement"]["length_ms"], 333.3);
        let pair = draft["placement"]["after_beat_range"].as_array().unwrap();
        assert!(pair[0].as_f64().unwrap() < 4. && pair[1].as_f64().unwrap() > 4.);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
