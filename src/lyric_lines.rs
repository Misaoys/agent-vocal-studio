use crate::{
    arrangement::Action,
    model::{identifier, Note, Track},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const LINE_LIMIT: usize = 512;
const NOTE_LIMIT: usize = 240;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Line {
    pub id: String,
    pub text: String,
    pub note_ids: Vec<String>,
    pub reading: Vec<String>,
}

fn line_id(track_id: &str, note_ids: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(track_id.as_bytes());
    for id in note_ids {
        hasher.update(b":");
        hasher.update(id.as_bytes());
    }
    format!("line_{:x}", hasher.finalize())
}

fn check_text(text: &str) -> Result<()> {
    ensure!(
        !text.trim().is_empty()
            && text.chars().count() <= 1200
            && text
                .chars()
                .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')),
        "请填写 1–1200 字的原词"
    );
    Ok(())
}

fn check_ids(ids: &[String]) -> Result<()> {
    ensure!(
        !ids.is_empty()
            && ids.len() <= NOTE_LIMIT
            && ids.iter().collect::<HashSet<_>>().len() == ids.len(),
        "一条原词需要 1–240 个不重复的音符"
    );
    ids.iter().try_for_each(|id| identifier(id))
}

fn reading(note: &Note) -> &str {
    if note.vowel_continuation {
        "+"
    } else {
        &note.lyric
    }
}

fn covers_syllables(notes: &[&Note], start: usize, end: usize) -> bool {
    !notes[start].vowel_continuation && notes.get(end).is_none_or(|note| !note.vowel_continuation)
}

pub(crate) fn set(track: &mut Track, note_ids: &[String], text: &str) -> Result<Line> {
    ensure!(
        track.instrument.is_none() && !track.is_audio(),
        "原词需要歌声轨"
    );
    check_ids(note_ids)?;
    let text = text.trim();
    check_text(text)?;
    let selected: HashSet<_> = note_ids.iter().map(String::as_str).collect();
    let mut ordered = track.notes.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.start.total_cmp(&b.start));
    let start = ordered
        .iter()
        .position(|note| selected.contains(note.id.as_str()))
        .context("原词需要有效的音符")?;
    let end = start + selected.len();
    let notes = ordered.get(start..end).context("请选择一条连续的乐句")?;
    ensure!(
        notes.iter().all(|note| selected.contains(note.id.as_str())),
        "请选择一条连续的乐句"
    );
    ensure!(
        covers_syllables(&ordered, start, end),
        "请包含起音和这一个字的全部延音"
    );
    let note_ids: Vec<_> = notes.iter().map(|note| note.id.clone()).collect();
    let line = Line {
        id: line_id(&track.id, &note_ids),
        text: text.to_owned(),
        note_ids,
        reading: notes.iter().map(|note| reading(note).to_owned()).collect(),
    };
    if let Some(previous) = track
        .lyric_lines
        .iter_mut()
        .find(|previous| previous.id == line.id)
    {
        *previous = line.clone();
    } else {
        ensure!(
            track.lyric_lines.len() < LINE_LIMIT,
            "此轨最多保留 {LINE_LIMIT} 条原词"
        );
        track.lyric_lines.push(line.clone());
    }
    Ok(line)
}

pub(crate) fn remove(track: &mut Track, id: &str) -> Result<()> {
    let index = track
        .lyric_lines
        .iter()
        .position(|line| line.id == id)
        .context("原词已更新")?;
    track.lyric_lines.remove(index);
    Ok(())
}

pub(crate) fn validate(track: &Track) -> Result<()> {
    ensure!(track.lyric_lines.len() <= LINE_LIMIT, "原词数量超出限制");
    let mut existing = HashSet::new();
    for line in &track.lyric_lines {
        check_text(&line.text)?;
        check_ids(&line.note_ids)?;
        ensure!(
            line.id == line_id(&track.id, &line.note_ids)
                && existing.insert(&line.id)
                && line.reading.len() == line.note_ids.len()
                && line.reading.iter().all(|text| text.chars().count() <= 40),
            "原词绑定无效"
        );
    }
    Ok(())
}

pub(crate) fn remap(track: &mut Track, map: &HashMap<String, String>) {
    for line in &mut track.lyric_lines {
        for id in &mut line.note_ids {
            if let Some(new_id) = map.get(id) {
                id.clone_from(new_id);
            }
        }
        line.id = line_id(&track.id, &line.note_ids);
    }
}

pub(crate) fn copy_complete(track: &mut Track, map: &HashMap<String, String>) -> Result<usize> {
    let copies = track
        .lyric_lines
        .iter()
        .filter_map(|line| {
            let ids = line
                .note_ids
                .iter()
                .map(|id| map.get(id).cloned())
                .collect::<Option<Vec<_>>>()?;
            let mut copy = line.clone();
            copy.id = line_id(&track.id, &ids);
            copy.note_ids = ids;
            Some(copy)
        })
        .collect::<Vec<_>>();
    let count = copies.len();
    ensure!(
        track.lyric_lines.len() + count <= LINE_LIMIT,
        "此轨最多保留 {LINE_LIMIT} 条原词"
    );
    track.lyric_lines.extend(copies);
    Ok(count)
}

struct Score<'a> {
    notes: Vec<&'a Note>,
    positions: HashMap<&'a str, usize>,
}

impl<'a> Score<'a> {
    fn new(track: &'a Track) -> Self {
        let mut notes = track.notes.iter().collect::<Vec<_>>();
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        let positions = notes
            .iter()
            .enumerate()
            .map(|(i, note)| (note.id.as_str(), i))
            .collect();
        Self { notes, positions }
    }

    fn anchors(&self, ids: &[String]) -> Vec<usize> {
        ids.iter()
            .filter_map(|id| self.positions.get(id.as_str()).copied())
            .collect()
    }

    fn layout(&self, anchors: &[usize], stored: &[String]) -> Layout<'a> {
        let current = anchors
            .iter()
            .map(|index| reading(self.notes[*index]))
            .collect::<Vec<_>>();
        if anchors.len() != stored.len() || anchors.is_empty() {
            return Layout {
                state: "missing_notes",
                beat_range: None,
                current,
            };
        }
        let [first, last] = [anchors[0], anchors[anchors.len() - 1]];
        let state = if anchors.windows(2).any(|pair| pair[1] != pair[0] + 1)
            || !covers_syllables(&self.notes, first, last + 1)
            || current != stored
        {
            "alignment_changed"
        } else {
            "aligned"
        };
        let beat_range = anchors.iter().fold([f64::INFINITY, 0.0], |[a, b], index| {
            let note = self.notes[*index];
            [a.min(note.start), b.max(note.start + note.duration)]
        });
        Layout {
            state,
            beat_range: Some(beat_range),
            current,
        }
    }
}

struct Layout<'a> {
    state: &'static str,
    beat_range: Option<[f64; 2]>,
    current: Vec<&'a str>,
}

pub(crate) fn arrangement_impact(
    original: &Track,
    candidate: &Track,
    action: Action,
    selected: &HashSet<String>,
    copied: &HashMap<String, String>,
) -> Vec<Value> {
    if original.lyric_lines.is_empty() || matches!(action, Action::Transpose | Action::Quantize) {
        return vec![];
    }
    let before = Score::new(original);
    let after = Score::new(candidate);
    let mut changes = Vec::new();
    for line in &original.lyric_lines {
        let source = before.layout(&before.anchors(&line.note_ids), &line.reading);
        let retained = after.layout(&after.anchors(&line.note_ids), &line.reading);
        let affected = line
            .note_ids
            .iter()
            .filter(|id| selected.contains(*id))
            .count();
        let complete = affected == line.note_ids.len();
        // Show collateral alignment changes when newly placed notes interrupt another line.
        if (affected == 0 || action == Action::Copy) && source.state != retained.state {
            changes.push(json!({"line_id":line.id,"kind":"alignment_changed",
                "affected_note_count":affected,"line_note_count":line.note_ids.len(),
                "state_before":source.state,"state_after":retained.state}));
        }
        if affected == 0 {
            continue;
        }
        let mut change = json!({"line_id":line.id,"affected_note_count":affected,
            "line_note_count":line.note_ids.len(),"state_before":source.state});
        let (kind, result) = match action {
            Action::Copy if complete => {
                let ids = line
                    .note_ids
                    .iter()
                    .map(|id| copied[id].clone())
                    .collect::<Vec<_>>();
                change["result_line_id"] = json!(line_id(&candidate.id, &ids));
                ("copied", after.layout(&after.anchors(&ids), &line.reading))
            }
            Action::Copy => ("not_copied", retained),
            Action::Move => ("moved", retained),
            Action::Clear => ("notes_removed", retained),
            Action::Transpose | Action::Quantize => unreachable!(),
        };
        change["kind"] = json!(kind);
        change["state_after"] = json!(result.state);
        if complete {
            change["text"] = json!(line.text);
            change["before"] = json!(source.beat_range);
            change["after"] = json!(result.beat_range);
        }
        changes.push(change);
    }
    changes
}

pub(crate) fn report(
    track: &Track,
    bounds: Option<[f64; 2]>,
    selected: Option<&HashSet<&str>>,
) -> Vec<Value> {
    if track.lyric_lines.is_empty() {
        return vec![];
    }
    let score = Score::new(track);
    let mut rows = track
        .lyric_lines
        .iter()
        .filter_map(|line| {
            if selected.is_some_and(|ids| !line.note_ids.iter().all(|id| ids.contains(id.as_str())))
            {
                return None;
            }
            let layout = score.layout(&score.anchors(&line.note_ids), &line.reading);
            if (selected.is_some() && layout.beat_range.is_none())
                || bounds.is_some_and(|[start, end]| {
                    layout.beat_range.is_none_or(|[a, b]| a < start - 1e-8 || b > end + 1e-8)
                })
            {
                return None;
            }
            Some(
                json!({"id":line.id,"track_id":track.id,"track_name":track.name,"text":line.text,
            "note_ids":line.note_ids,"reading_at_save":line.reading,"current_reading":layout.current,
            "beat_range":layout.beat_range,"state":layout.state}),
            )
        })
        .collect::<Vec<_>>();
    let start = |row: &Value| row["beat_range"][0].as_f64().unwrap_or(f64::INFINITY);
    rows.sort_by(|a, b| start(a).total_cmp(&start(b)));
    rows
}
