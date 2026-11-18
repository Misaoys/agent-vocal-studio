use crate::model::*;
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
#[derive(Clone)]
pub struct Selection {
    pub id: String,
    pub project_id: String,
    pub revision: u64,
    pub notes: Vec<(String, String)>,
}
#[derive(Default)]
pub struct Workspace {
    pub saved: HashMap<String, Selection>,
    pub ui: Option<Selection>,
    pub(crate) ui_clips: Vec<crate::clips::ClipRef>,
    pub(crate) ui_range: Option<crate::arrangement::BoxRange>,
    boxes: VecDeque<Arc<crate::arrangement::Capture>>,
}
impl Selection {
    pub fn value(&self) -> Value {
        json!({"selection_id":self.id,"project_id":self.project_id,"revision":self.revision,"count":self.notes.len(),"notes":self.notes.iter().map(|(t,n)|json!({"track_id":t,"note_id":n})).collect::<Vec<_>>(),"note_ids":self.notes.iter().map(|(_,n)|n).collect::<Vec<_>>()})
    }
}
impl Workspace {
    pub(crate) fn current_ui(&self, project: &Project) -> Result<&Selection> {
        let selected = self
            .ui
            .as_ref()
            .context("No UI selection has been published")?;
        ensure!(
            selected.project_id == project.id && selected.revision == project.revision,
            "UI_SELECTION_STALE: select in the current revision"
        );
        Ok(selected)
    }

    pub(crate) fn capture_box(
        &mut self,
        project: &Project,
    ) -> Result<Arc<crate::arrangement::Capture>> {
        let selection = self.current_ui(project)?;
        let range = self
            .ui_range
            .as_ref()
            .context("BOX_REQUIRED: 请在编排区框选拍范围和音轨")?;
        if let Some(found) = self
            .boxes
            .iter()
            .find(|entry| entry.selection.id == selection.id)
        {
            return Ok(found.clone());
        }
        let capture = Arc::new(crate::arrangement::Capture {
            selection: selection.clone(),
            range: range.clone(),
            clips: self.ui_clips.clone(),
        });
        self.boxes.push_back(capture.clone());
        if self.boxes.len() > 12 {
            self.boxes.pop_front();
        }
        Ok(capture)
    }

    pub(crate) fn frozen_box(&self, id: &str) -> Result<Arc<crate::arrangement::Capture>> {
        self.boxes
            .iter()
            .find(|capture| capture.selection.id == id)
            .cloned()
            .context("BOX_NOT_FOUND: 请重新读取编排框选")
    }

    pub fn resolve(&mut self, p: &Project, a: &Value) -> Result<Selection> {
        let ui = if a["from_ui"] == true {
            Some(self.current_ui(p)?)
        } else {
            None
        };
        let tracks = a["track_ids"].as_array();
        let ids = a["note_ids"].as_array();
        if let Some(requested) = tracks {
            for id in requested {
                ensure!(
                    p.tracks.iter().any(|t| id.as_str() == Some(&t.id)),
                    "Unknown track in selection"
                );
            }
        }
        if let Some(requested) = ids {
            for id in requested {
                ensure!(
                    p.tracks
                        .iter()
                        .flat_map(|t| &t.notes)
                        .any(|n| id.as_str() == Some(&n.id)),
                    "Unknown note in selection"
                );
            }
        }
        let time = range(a.get("time_range"), 0., 50000.)?;
        let pitch = range(a.get("pitch_range"), 0., 127.)?;
        let intersects = a["time_match"] == "intersects";
        let mut notes = vec![];
        for t in &p.tracks {
            if tracks.is_some_and(|ids| !ids.iter().any(|id| id.as_str() == Some(&t.id))) {
                continue;
            }
            for n in &t.notes {
                if ui.is_some_and(|s| !s.notes.iter().any(|(_, id)| id == &n.id)) {
                    continue;
                }
                if ids.is_some_and(|ids| !ids.iter().any(|id| id.as_str() == Some(&n.id))) {
                    continue;
                }
                if time.is_some_and(|[a, b]| {
                    if intersects {
                        n.start + n.duration <= a || n.start >= b
                    } else {
                        n.start < a || n.start >= b
                    }
                }) {
                    continue;
                }
                if pitch.is_some_and(|[a, b]| (n.pitch as f64) < a || (n.pitch as f64) > b) {
                    continue;
                }
                notes.push((t.id.clone(), n.id.clone()));
            }
        }
        ensure!(!notes.is_empty(), "Selection contains no notes");
        let selection = Selection {
            id: uid("sel"),
            project_id: p.id.clone(),
            revision: p.revision,
            notes,
        };
        if self.saved.len() >= 100 {
            self.saved.clear()
        };
        self.saved.insert(selection.id.clone(), selection.clone());
        Ok(selection)
    }
    pub fn set_ui(&mut self, p: &Project, ids: &Value) -> Result<Value> {
        let ids = ids.as_array().context("note_ids must be an array")?;
        let mut notes = Vec::with_capacity(ids.len());
        let owners: HashMap<_, _> = if ids.is_empty() {
            HashMap::new()
        } else {
            p.tracks
                .iter()
                .flat_map(|track| {
                    track
                        .notes
                        .iter()
                        .map(|note| (note.id.as_str(), track.id.as_str()))
                })
                .collect()
        };
        for id in ids {
            let id = id.as_str().context("Invalid note id")?;
            let track = owners
                .get(id)
                .context("UI selection references an unknown note")?;
            notes.push(((*track).into(), id.into()));
        }
        let s = Selection {
            id: uid("ui"),
            project_id: p.id.clone(),
            revision: p.revision,
            notes,
        };
        let v = s.value();
        self.ui = Some(s);
        Ok(v)
    }
}
fn range(v: Option<&Value>, lo: f64, hi: f64) -> Result<Option<[f64; 2]>> {
    let Some(v) = v else { return Ok(None) };
    let r: [f64; 2] = serde_json::from_value(v.clone())?;
    ensure!(
        r.iter().all(|n| n.is_finite() && *n >= lo && *n <= hi) && r[0] <= r[1],
        "Invalid selection range"
    );
    Ok(Some(r))
}
pub(crate) fn selected_vocal_notes<'a>(p: &'a Project, s: &Selection) -> Result<Vec<&'a Note>> {
    let (track_id, _) = s.notes.first().context("请选中歌声音符")?;
    ensure!(
        s.notes.iter().all(|(id, _)| id == track_id),
        "请从同一歌声轨选择一个乐句"
    );
    let track = p
        .tracks
        .iter()
        .find(|t| t.id == *track_id)
        .context("选中的音轨已不存在")?;
    ensure!(
        track.instrument.is_none() && !track.is_audio(),
        "请选择歌声轨上的音符"
    );
    let selected = s
        .notes
        .iter()
        .map(|(_, id)| id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut notes = track
        .notes
        .iter()
        .filter(|n| selected.contains(n.id.as_str()))
        .collect::<Vec<_>>();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    ensure!(notes.len() == s.notes.len(), "选中的音符已变更");
    Ok(notes)
}

fn align_lyrics<'a>(p: &'a Project, s: &Selection, text: &str) -> Result<HashMap<&'a str, String>> {
    let notes = selected_vocal_notes(p, s)?
        .into_iter()
        .filter(|note| !note.vowel_continuation)
        .collect::<Vec<_>>();
    ensure!(!notes.is_empty(), "请选择需要落字的歌声音符");
    let moras = crate::lyrics::kana(text)?;
    ensure!(
        moras.len() == notes.len(),
        "{} 个读音 · {} 个选中音符，请调整读音或选区",
        moras.len(),
        notes.len()
    );
    Ok(notes
        .into_iter()
        .zip(moras)
        .map(|(note, part)| (note.id.as_str(), part))
        .collect())
}

pub fn preview(p: &Project, s: &Selection, transform: &Value) -> Result<Value> {
    ensure!(
        s.project_id == p.id && s.revision == p.revision,
        "REVISION_CONFLICT: selection expired; resolve again"
    );
    let kind = field(transform, "kind")?;
    let written = transform
        .get("written_text")
        .map(|value| {
            ensure!(
                matches!(kind, "fit_lyrics" | "align_lyrics" | "set_lyrics"),
                "原词只能与本句读音一起确认"
            );
            if value.is_null() {
                return Ok(None);
            }
            Ok(Some(value.as_str().context("原词需要文本")?))
        })
        .transpose()?;
    let mut next = p.clone();
    let mut operations = vec![];
    let mut changes = vec![];
    let aligned = (kind == "align_lyrics")
        .then(|| align_lyrics(p, s, field(transform, "reading")?))
        .transpose()?;
    let fitted = if kind == "fit_lyrics" {
        let notes = selected_vocal_notes(p, s)?;
        let track = p
            .tracks
            .iter()
            .find(|track| track.id == s.notes[0].0)
            .context("歌声轨已更新")?;
        Some(crate::lyrics_fit::plan(
            track,
            &notes,
            &p.timeline(),
            field(transform, "reading")?,
            transform.get("anchors"),
        )?)
    } else {
        None
    };
    let tuning = if matches!(kind, "pitch_join" | "pitch_edge") {
        let notes = selected_vocal_notes(p, s)?;
        let curves = match kind {
            "pitch_join" => crate::pitch_join::plan(p, &notes, transform)?.into(),
            _ => vec![crate::pitch_join::edge(p, &notes, transform)?],
        };
        notes
            .into_iter()
            .zip(curves)
            .map(|(note, curve)| (note.id.as_str(), curve))
            .collect()
    } else {
        HashMap::new()
    };
    let vibrato = if kind == "vibrato_envelope" {
        let notes = selected_vocal_notes(p, s)?;
        ensure!(notes.len() == 1, "请选择一个歌声音符来调整颤音");
        Some(crate::vibrato::settings(&transform["settings"])?)
    } else {
        None
    };
    let lyrics = if kind == "set_lyrics" {
        let ls = transform["lyrics"]
            .as_array()
            .context("Need lyrics array")?;
        ensure!(
            ls.len() == s.notes.len(),
            "Lyric count must match the selected notes"
        );
        Some(ls)
    } else {
        None
    };
    for (i, (tid, nid)) in s.notes.iter().enumerate() {
        let t = next.track_mut(tid)?;
        let n = t
            .notes
            .iter_mut()
            .find(|n| n.id == *nid)
            .context("Selected note missing")?;
        let before = serde_json::to_value(&*n)?;
        let values = match kind {
            "transpose" => {
                let delta = transform["semitones"]
                    .as_i64()
                    .context("semitones must be an integer")?;
                json!({"pitch":n.pitch as i64+delta})
            }
            "move" => {
                json!({"start":n.start+transform["beats"].as_f64().context("Need beat offset")?})
            }
            "quantize" => {
                let step = transform["step"]
                    .as_f64()
                    .context("Need step in quarter-note beats")?;
                ensure!(
                    step.is_finite() && (1. / 480. ..=16.).contains(&step),
                    "Invalid quantize step"
                );
                json!({"start":(n.start/step).round()*step})
            }
            "velocity_scale" => {
                let f = transform["factor"].as_f64().context("Need factor")?;
                ensure!(
                    f.is_finite() && f > 0. && f <= 4.,
                    "Velocity scale must be 0..4"
                );
                json!({"velocity":(n.velocity*f).clamp(1.,127.)})
            }
            "set_lyrics" => json!({"lyric":lyrics.unwrap()[i]}),
            "align_lyrics" => {
                let Some(lyric) = aligned.as_ref().unwrap().get(nid.as_str()) else { continue };
                json!({"lyric":lyric})
            }
            "fit_lyrics" => {
                let Some(values)=fitted.as_ref().unwrap().values.get(nid.as_str()) else {continue;};
                values.clone()
            }
            "pitch_join" | "pitch_edge" => serde_json::to_value(&tuning[nid.as_str()])?,
            "vibrato_envelope" => json!({"vibrato":vibrato.as_ref()}),
            "set" => {
                let values = transform["values"].as_object().context("Need values")?;
                for k in values.keys() {
                    ensure!(
                        [
                            "duration",
                            "pitch",
                            "velocity",
                            "lyric",
                            "vowel_continuation",
                            "vibrato",
                            "pitch_points",
                            "consonant_ms",
                            "vowel_offset_ms",
                            "pitch_blend"
                        ]
                        .contains(&k.as_str()),
                        "Unsupported selection field {k}"
                    )
                }
                Value::Object(values.clone())
            }
            _ => bail!(
                "Supported transforms: transpose, move, quantize, velocity_scale, align_lyrics, fit_lyrics, pitch_join, pitch_edge, vibrato_envelope, set_lyrics, set"
            ),
        };
        let mut value = before.clone();
        for (k, v) in values.as_object().unwrap() {
            value[k] = v.clone()
        }
        *n = serde_json::from_value(value)?;
        changes.push(json!({"track_id":tid,"note_id":nid,"before":before,"after":n}));
        operations.push(json!({"op":"update_note","track_id":tid,"note_id":nid,"values":values}));
    }
    let line_change = if let Some(written) = written {
        let note_ids = selected_vocal_notes(&next, s)?
            .into_iter()
            .map(|note| note.id.clone())
            .collect::<Vec<_>>();
        let track_id = &s.notes[0].0;
        let track = next
            .tracks
            .iter_mut()
            .find(|track| &track.id == track_id)
            .context("歌声轨已更新")?;
        if let Some(text) = written {
            let line = crate::lyric_lines::set(track, &note_ids, text)?;
            operations.push(json!({"op":"set_lyric_line","track_id":track_id,"note_ids":note_ids,"text":line.text}));
            Some(json!({"action":"save","line":line}))
        } else if let Some(line) = track
            .lyric_lines
            .iter()
            .find(|line| line.note_ids == note_ids)
        {
            let id = line.id.clone();
            crate::lyric_lines::remove(track, &id)?;
            operations.push(json!({"op":"remove_lyric_line","track_id":track_id,"line_id":id}));
            Some(json!({"action":"remove","line_id":id}))
        } else {
            None
        }
    } else {
        None
    };
    next.upgrade_feature_format();
    next.validate()?;
    let mut response = json!({"selection":s.value(),"change_count":changes.len(),"changes":changes,"operations":operations,"dry_run":true});
    if let Some(fitted) = fitted {
        response["lyric_fit"] = json!({"groups":fitted.groups});
    }
    if let Some(line_change) = line_change {
        response["lyric_line"] = line_change;
    }
    if vibrato.is_some() {
        let note = selected_vocal_notes(&next, s)?[0];
        response["vibrato_envelope"] = crate::vibrato::describe(note, &p.timeline());
    }
    Ok(response)
}

pub(crate) fn preview_with_audio(
    root: &std::path::Path,
    project: &Project,
    selection: &Selection,
    transform: &Value,
) -> Result<Value> {
    if transform["kind"] == "pitch_reference" {
        let reference = serde_json::from_value(transform["reference"].clone())?;
        return crate::pitch_reference::preview(root, project, selection, &reference);
    }
    preview(project, selection, transform)
}

pub(crate) struct TuningPlan {
    pub candidate: Project,
    pub range: [f64; 2],
    pub preview: Value,
}

pub(crate) fn tuning_kind(transform: &Value) -> Result<&str> {
    let kind = field(transform, "kind")?;
    ensure!(
        matches!(
            kind,
            "pitch_join"
                | "pitch_edge"
                | "pitch_reference"
                | "vibrato_envelope"
                | "align_lyrics"
                | "fit_lyrics"
                | "set"
        ),
        "TUNING_TRANSFORM_UNSUPPORTED: 请选择歌声调声或歌词编辑"
    );
    if kind == "set" {
        let values = transform["values"].as_object().context("Need values")?;
        ensure!(
            !values.is_empty() && !values.contains_key("duration"),
            "调声试听的 set 用于音高、颤音、咬字、歌词或力度"
        );
    }
    Ok(kind)
}

pub(crate) fn pitch_audition(
    root: &std::path::Path,
    project: &Project,
    selection: &Selection,
    transform: &Value,
) -> Result<TuningPlan> {
    let kind = tuning_kind(transform)?;
    let notes = selected_vocal_notes(project, selection)?;
    let edit = preview_with_audio(root, project, selection, transform)?;
    ensure!(
        edit["change_count"].as_u64().unwrap_or_default() > 0,
        "没有可试听的调声改动"
    );
    let (candidate, _) = crate::model::Store::preview_edit_with_refs(project, &edit["operations"])?;
    let clock = project.timeline();
    let millis = |key: &str| transform[key].as_f64().unwrap_or_default() / 1000.;
    let [first, last] = match (kind, notes.as_slice()) {
        ("vibrato_envelope", [note]) => {
            let draft = selected_vocal_notes(&candidate, selection)?[0];
            [*note, draft]
                .into_iter()
                .filter(|note| note.vibrato.depth > 0. && note.vibrato.start < note.vibrato.end)
                .map(|note| crate::vibrato::region(note, &clock))
                .reduce(|a, b| [a[0].min(b[0]), a[1].max(b[1])])
                .unwrap_or_else(|| {
                    [
                        clock.seconds(note.start),
                        clock.seconds(note.start + note.duration),
                    ]
                })
        }
        ("pitch_reference" | "set" | "align_lyrics" | "fit_lyrics", [first, ..]) => [
            clock.seconds(first.start),
            notes
                .iter()
                .map(|note| clock.seconds(note.start + note.duration))
                .fold(0., f64::max),
        ],
        (_, [note]) if transform["side"] == "start" => {
            let at = clock.seconds(note.start);
            [at, at + millis("length_ms")]
        }
        (_, [note]) => {
            let at = clock.seconds(note.start + note.duration);
            [at - millis("length_ms"), at]
        }
        (_, [left, _]) => {
            let at = clock.seconds(left.start + left.duration);
            [at - millis("lead_ms"), at + millis("trail_ms")]
        }
        _ => bail!("调声试听范围无效"),
    };
    let available = crate::source_cache::frame_count(project) as f64 / crate::audition::RATE as f64;
    let range = [
        clock.beat((first - 0.25).max(0.)),
        clock.beat((last + 0.30).min(available)),
    ];
    crate::audition::frames(project, range)?;
    Ok(TuningPlan {
        candidate,
        range,
        preview: edit,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_scopes_do_not_touch_other_notes_and_stale_selections_fail() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_scope"));
        let store = Store::new(root.clone())?;
        let mut t = Track::new("主唱");
        for i in 0..4 {
            t.notes.push(Note::from_value(
                &json!({"start":i,"duration":0.5,"pitch":60+i,"lyric":"あ"}),
            )?)
        }
        let p = store.create("scope", 120., [4, 4], vec![t])?;
        let mut ws = Workspace::default();
        let s = ws.resolve(&p, &json!({"time_range":[1,3],"pitch_range":[61,62]}))?;
        assert_eq!(s.notes.len(), 2);
        let pre = preview(&p, &s, &json!({"kind":"transpose","semitones":12}))?;
        assert_eq!(store.read(&p.id)?.revision, 1);
        let after = store.edit(&p.id, p.revision, &pre["operations"])?;
        let pitches = after.tracks[0]
            .notes
            .iter()
            .map(|n| n.pitch)
            .collect::<Vec<_>>();
        assert_eq!(pitches, vec![60, 73, 74, 63]);
        assert!(preview(&after, &s, &json!({"kind":"transpose","semitones":1})).is_err());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
