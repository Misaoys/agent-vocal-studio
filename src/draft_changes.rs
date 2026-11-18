use crate::{
    model::{Note, Project, Track},
    music_review::Window,
    selection::Selection,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const NOTE_DETAIL_LIMIT: usize = 32;
const WORD_DETAIL_LIMIT: usize = 12;

fn overlap(note: &Note, [start, end]: [f64; 2]) -> bool {
    note.start < end && note.start + note.duration > start
}

fn note_view(note: &Note, [start, end]: [f64; 2], voice: bool) -> Value {
    let note_end = note.start + note.duration;
    json!({
        "beat_range":[note.start.max(start),note_end.min(end)],
        "pitch":note.pitch,
        "reading":(voice && !note.vowel_continuation && note.start>=start).then_some(&note.lyric),
        "vowel_continuation":voice && note.vowel_continuation,
        "clipped":note.start<start || note_end>end,
    })
}

fn note_fields(a: &Note, b: &Note, voice: bool) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if a.pitch != b.pitch {
        fields.push("pitch");
    }
    if a.start != b.start {
        fields.push("onset");
    }
    if a.duration != b.duration {
        fields.push("duration");
    }
    if voice && (a.lyric != b.lyric || a.vowel_continuation != b.vowel_continuation) {
        fields.push("sung_reading");
    }
    if a.velocity != b.velocity
        || a.pitch_points != b.pitch_points
        || a.pitch_blend != b.pitch_blend
        || a.vibrato != b.vibrato
        || a.consonant_ms != b.consonant_ms
        || a.vowel_offset_ms != b.vowel_offset_ms
    {
        fields.push("expression");
    }
    fields
}

fn written(
    track: Option<&Track>,
    bounds: [f64; 2],
    selected: Option<&HashSet<&str>>,
) -> BTreeMap<String, Value> {
    track
        .into_iter()
        .flat_map(|track| crate::lyric_lines::report(track, Some(bounds), selected))
        .filter_map(|row| {
            Some((
                row["id"].as_str()?.to_owned(),
                json!({
                    "text":row["text"],"state":row["state"],"beat_range":row["beat_range"],
                }),
            ))
        })
        .collect()
}

struct Focus<'a> {
    priority: u8,
    beat: f64,
    track_id: &'a str,
    note_id: &'a str,
    reason: &'static str,
}

pub(crate) fn report(
    before: &Project,
    after: &Project,
    window: &Window,
    selection: Option<&Selection>,
) -> Value {
    let bounds = window.bounds.unwrap_or_else(|| {
        [
            0.,
            crate::music_review::end_beat(before).max(crate::music_review::end_beat(after)),
        ]
    });
    let mut tracks = Vec::new();
    let mut note_count = 0;
    let mut words_count = 0;
    let mut documented_notes: usize = 0;
    let mut documented_words: usize = 0;
    let mut focus: Option<Focus<'_>> = None;
    for track in before.tracks.iter().chain(
        after
            .tracks
            .iter()
            .filter(|track| !before.tracks.iter().any(|old| old.id == track.id)),
    ) {
        if window
            .track_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(&track.id))
        {
            continue;
        }
        let selected = selection.map(|scope| {
            scope
                .notes
                .iter()
                .filter(|(id, _)| id == &track.id)
                .map(|(_, note)| note.as_str())
                .collect::<HashSet<_>>()
        });
        if selected.as_ref().is_some_and(HashSet::is_empty) {
            continue;
        }
        let original = before.tracks.iter().find(|item| item.id == track.id);
        let revised = after.tracks.iter().find(|item| item.id == track.id);
        let display = revised.unwrap_or(track);
        let voice = display.instrument.is_none() && !display.is_audio();
        let kind = if voice {
            "voice"
        } else if display.is_audio() {
            "audio"
        } else if display.instrument.as_ref().is_some_and(|patch| patch.drums) {
            "drums"
        } else {
            "instrument"
        };
        let a = Notes::new(original, bounds, selected.as_ref());
        let b = Notes::new(revised, bounds, selected.as_ref());
        let prefix = a
            .visible
            .iter()
            .zip(&b.visible)
            .take_while(|(x, y)| x == y)
            .count();
        let mut detail = Vec::new();
        let mut counts = [0; 5];
        let mut retained = 0;
        for id in a.visible.iter().map(|note| note.id.as_str()).chain(
            b.visible
                .iter()
                .filter(|note| !a.shown.contains_key(note.id.as_str()))
                .map(|note| note.id.as_str()),
        ) {
            let first = a.shown.get(id).copied();
            let last = b.shown.get(id).copied();
            let (category, status, fields, beat, reason) = match (first, last) {
                (Some(old), Some(new)) => {
                    let fields = note_fields(old, new, voice);
                    if fields.is_empty() {
                        retained += 1;
                        continue;
                    }
                    let (beat, reason) = if fields.as_slice() == ["duration"] {
                        (
                            (old.start + old.duration).min(new.start + new.duration),
                            "release",
                        )
                    } else {
                        (old.start.min(new.start), "attack")
                    };
                    (0, "edited", fields, beat, reason)
                }
                (Some(old), None) if b.all.contains(id) => {
                    (1, "leaves_range", vec![], old.start, "removed_entry")
                }
                (None, Some(new)) if a.all.contains(id) => {
                    (2, "enters_range", vec![], new.start, "new_entry")
                }
                (Some(old), None) => (3, "removed", vec![], old.start, "removed_entry"),
                (None, Some(new)) => (4, "added", vec![], new.start, "new_entry"),
                (None, None) => unreachable!(),
            };
            counts[category] += 1;
            let priority = if voice { 0 } else { 1 };
            let beat = beat.clamp(bounds[0], bounds[1]);
            if focus
                .as_ref()
                .is_none_or(|point| (priority, beat) < (point.priority, point.beat))
            {
                focus = Some(Focus {
                    priority,
                    beat,
                    track_id: &track.id,
                    note_id: id,
                    reason,
                });
            }
            if detail.len() < 12 && documented_notes < NOTE_DETAIL_LIMIT {
                documented_notes += 1;
                detail.push(json!({"note_id":id,"kind":status,"fields":fields,
                    "before":first.map(|note|note_view(note,bounds,voice)),
                    "after":last.map(|note|note_view(note,bounds,voice))}));
            }
        }
        let a_words = written(original, bounds, selected.as_ref());
        let b_words = written(revised, bounds, selected.as_ref());
        let mut words = Vec::new();
        let mut word_total = 0;
        for id in a_words
            .keys()
            .chain(b_words.keys().filter(|id| !a_words.contains_key(*id)))
        {
            let first = a_words.get(id);
            let last = b_words.get(id);
            if first == last {
                continue;
            }
            word_total += 1;
            if documented_words < WORD_DETAIL_LIMIT {
                documented_words += 1;
                let kind = match (first, last) {
                    (Some(a), Some(b)) if a["text"] != b["text"] => "text",
                    (Some(a), Some(b)) if a["state"] != b["state"] => "alignment",
                    (Some(_), Some(_)) => "placement",
                    (Some(_), None)
                        if revised.is_some_and(|track| {
                            track.lyric_lines.iter().any(|line| &line.id == id)
                        }) =>
                    {
                        "leaves_range"
                    }
                    (None, Some(_))
                        if original.is_some_and(|track| {
                            track.lyric_lines.iter().any(|line| &line.id == id)
                        }) =>
                    {
                        "enters_range"
                    }
                    (Some(_), None) => "removed",
                    _ => "added",
                };
                words.push(json!({"line_id":id,"kind":kind,"before":first,"after":last}));
            }
        }
        let total: usize = counts.iter().sum();
        if total > 0 || word_total > 0 {
            note_count += total;
            words_count += word_total;
            tracks.push(json!({"track_id":track.id,"name":display.name,"kind":kind,
                "retained_prefix_count":prefix,"retained_note_count":retained,
                "prefix_note_ids":a.visible.iter().take(prefix.min(8)).map(|note|&note.id).collect::<Vec<_>>(),
                "note_count":total,"edited":counts[0],"left_range":counts[1],"entered_range":counts[2],
                "removed":counts[3],"added":counts[4],"notes":detail,"notes_truncated":total>detail.len(),
                "written_lyrics":words,"written_count":word_total,"written_truncated":word_total>words.len()}));
        }
    }
    let listen = focus.and_then(|point| {
        let clock = before.timeline();
        let time = clock.seconds(point.beat);
        // Use the same shared beat window for both drafts. This is a score cue;
        // measured audio and an explicit user range take priority.
        let end = bounds[1].min(clock.beat(
            before.seconds().max(after.seconds()) + before.tail_seconds.min(after.tail_seconds),
        ));
        let first = clock.beat((time - 0.45).max(0.)).max(bounds[0]);
        let last = clock.beat(time + 0.80).min(end);
        (last > first).then(|| {
            json!({"beat_range":[first,last],"focus_beat":point.beat,
            "track_id":point.track_id,"note_id":point.note_id,"reason":point.reason})
        })
    });
    json!({"beat_range":bounds,"tracks":tracks,"note_count":note_count,"written_count":words_count,
        "listening_point":listen})
}

struct Notes<'a> {
    visible: Vec<&'a Note>,
    shown: HashMap<&'a str, &'a Note>,
    all: HashSet<&'a str>,
}

impl<'a> Notes<'a> {
    fn new(track: Option<&'a Track>, bounds: [f64; 2], ids: Option<&HashSet<&str>>) -> Self {
        let notes = track.map_or(&[][..], |track| track.notes.as_slice());
        let visible = notes
            .iter()
            .filter(|note| {
                overlap(note, bounds) && ids.is_none_or(|ids| ids.contains(note.id.as_str()))
            })
            .collect::<Vec<_>>();
        let shown = visible
            .iter()
            .map(|note| (note.id.as_str(), *note))
            .collect();
        let all = notes.iter().map(|note| note.id.as_str()).collect();
        Self {
            visible,
            shown,
            all,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{uid, Store};

    #[test]
    fn reports_note_leaving_scope_without_revealing_its_new_lyrics() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(uid("avs_draft_changes"));
        let store = Store::new(root.clone())?;
        let mut track = Track::new("Voice");
        track.notes = [
            json!({"id":"lead","start":4.,"duration":0.75,"lyric":"し"}),
            json!({"id":"ending","start":5.,"duration":0.75,"lyric":"か"}),
            json!({"id":"secret","start":8.,"duration":0.75,"lyric":"け"}),
        ]
        .iter()
        .map(Note::from_value)
        .collect::<anyhow::Result<_>>()?;
        let tid = track.id.clone();
        let project = store.create("Draft scope", 120., [4, 4], vec![track])?;
        let mut after = project.clone();
        after.tracks[0].notes[1].start = 7.;
        after.tracks[0].notes[1].lyric = "は".into();
        after.tracks[0].notes[2].pitch += 6;
        let window = Window {
            bounds: Some([4., 6.]),
            track_ids: None,
        };
        let selection = Selection {
            id: "selection".into(),
            project_id: project.id.clone(),
            revision: project.revision,
            notes: vec![(tid.clone(), "lead".into()), (tid, "ending".into())],
        };
        let result = report(&project, &after, &window, Some(&selection));
        let delta = &result["tracks"][0];
        assert_eq!(delta["prefix_note_ids"], json!(["lead"]));
        assert_eq!(delta["notes"][0]["kind"], "leaves_range");
        assert_eq!(delta["notes"][0]["before"]["reading"], "か");
        assert!(delta["notes"][0]["after"].is_null());
        assert_eq!(delta["note_count"], 1);
        assert!(!result.to_string().contains("secret") && !result.to_string().contains('は'));
        assert_eq!(result["listening_point"]["note_id"], "ending");
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
