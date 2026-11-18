use crate::{
    engine::VoiceEngine,
    jobs::Control,
    model::{Note, Project, Track},
    source_cache::{DryReader, Source},
};
use anyhow::Result;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

const RATE: usize = 48_000;
const REST_MARGIN: usize = RATE * 2 / 5;
const PATCH_FRAMES: usize = 8 * RATE;
const FADE_FRAMES: usize = RATE / 100;
const QUIET_PEAK: f32 = 0.004;
pub(crate) const TOTAL_PATCH_FRAMES: usize = 2_097_152;

#[derive(Clone)]
pub(crate) struct Input {
    pub(crate) source: Source,
    patches: Vec<Arc<Patch>>,
    voice_backend: Option<String>,
    anchor: Option<Arc<Mutex<DryReader>>>,
}

struct Patch {
    range: [usize; 2],
    samples: Vec<[f32; 2]>,
}

pub(crate) struct Plan {
    previous: Track,
    pub(crate) range: [usize; 2],
}

impl Plan {
    pub(crate) fn new(
        project: &Project,
        previous: &Track,
        next: &Track,
        len: usize,
    ) -> Option<Self> {
        if previous.instrument.is_some()
            || next.instrument.is_some()
            || previous.is_audio()
            || next.is_audio()
            || previous.singer_id != next.singer_id
            || previous.openutau_voice != next.openutau_voice
        {
            return None;
        }
        let clock = project.timeline();
        let span = |note: &Note| {
            [
                (clock.seconds(note.start) * RATE as f64).floor() as usize,
                (clock.seconds(note.start + note.duration) * RATE as f64).ceil() as usize,
            ]
        };
        fn notes_by_id(notes: &[Note]) -> HashMap<&str, &Note> {
            notes
                .iter()
                .map(|note| (note.id.as_str(), note))
                .collect::<HashMap<_, _>>()
        }
        let old = notes_by_id(&previous.notes);
        let new = notes_by_id(&next.notes);
        let all = previous.notes.iter().chain(&next.notes);
        let mut affected = [usize::MAX, 0];
        for (notes, counterpart) in [(&previous.notes, &new), (&next.notes, &old)] {
            for note in notes {
                if counterpart.get(note.id.as_str()).copied() != Some(note) {
                    let [first, end] = span(note);
                    affected = [affected[0].min(first), affected[1].max(end)];
                }
            }
        }
        if affected[0] >= affected[1] {
            return None;
        }
        let mut spans = all.map(span).collect::<Vec<_>>();
        spans.sort_unstable();
        let mut phrases = Vec::<[usize; 2]>::new();
        for [first, end] in spans {
            if let Some(last) = phrases.last_mut() {
                if first <= last[1] + 2 * REST_MARGIN {
                    last[1] = last[1].max(end);
                    continue;
                }
            }
            phrases.push([first, end]);
        }
        // Use the union of both scores so an old vowel or a shifted onset cannot
        // cross a seam that appears safe in only one of the two revisions.
        let first = phrases.iter().find(|range| range[1] > affected[0])?[0];
        let end = phrases.iter().rev().find(|range| range[0] < affected[1])?[1];
        let range = [
            first.saturating_sub(REST_MARGIN),
            (end + REST_MARGIN).min(len),
        ];
        let frames = range[1].checked_sub(range[0])?;
        ((2 * FADE_FRAMES..=PATCH_FRAMES).contains(&frames) && frames <= len / 2).then(|| Self {
            previous: previous.clone(),
            range,
        })
    }
}

impl From<Source> for Input {
    fn from(source: Source) -> Self {
        Self {
            source,
            patches: vec![],
            voice_backend: None,
            anchor: None,
        }
    }
}

impl Input {
    pub(crate) fn patch_frames(&self) -> usize {
        self.patches.iter().map(|patch| patch.samples.len()).sum()
    }

    pub(crate) fn open(&self, root: &Path, project_id: &str, len: usize) -> Result<Reader> {
        let source = match &self.anchor {
            Some(anchor) => anchor.clone(),
            None => Arc::new(Mutex::new(self.source.open(root, project_id, len)?)),
        };
        Ok(Reader {
            source,
            patches: self.patches.clone(),
        })
    }

    pub(crate) fn update_voice(
        &self,
        root: &Path,
        project: &Project,
        track: &Track,
        plan: &Plan,
        allowance: usize,
        control: &Control,
    ) -> Result<Option<Self>> {
        let [start, end] = plan.range;
        let len = crate::source_cache::frame_count(project);
        let mut revised = self.clone();
        revised
            .patches
            .retain(|patch| patch.range[0] < start || patch.range[1] > end);
        if revised.patches.len() >= 4 || revised.patch_frames() + end - start > allowance {
            return Ok(None);
        }
        let voice = VoiceEngine::load(control)?;
        let backend = voice.fingerprint()?;
        let next_complete = Source::new(project, track, "voicevox", &backend)?;
        if next_complete.open(root, &project.id, len).is_ok() {
            return Ok(None);
        }
        let compatible = if let Some(active) = &self.voice_backend {
            active == &backend
        } else {
            Source::new(project, &plan.previous, "voicevox", &backend)?.key == self.source.key
        };
        if !compatible {
            return Ok(None);
        }
        let mut original = self.open(root, &project.id, len)?;
        let Some(mut candidate) = voice.monitor_excerpt(project, track, plan.range, control)?
        else {
            return Ok(None);
        };
        if !candidate.iter().flatten().all(|sample| sample.is_finite()) {
            return Ok(None);
        }
        // Retain a quiet overlap at both ends. The original samples are read
        // through earlier patches so even successive edits use what is playing.
        for (boundary, begin) in [(start, true), (end, false)] {
            if boundary == 0 || boundary == len {
                continue;
            }
            let seam = if begin { start } else { end - FADE_FRAMES };
            let mut tail = Vec::new();
            original.read_into(seam, FADE_FRAMES, &mut tail, || control.check())?;
            let inside = seam - start;
            let window = &mut candidate[inside..inside + FADE_FRAMES];
            if tail
                .iter()
                .chain(window.iter())
                .flatten()
                .any(|sample| sample.abs() > QUIET_PEAK)
            {
                return Ok(None);
            }
            for (index, (sample, previous)) in window.iter_mut().zip(tail).enumerate() {
                let weight = index as f32 / (FADE_FRAMES - 1) as f32;
                let weight = if begin { weight } else { 1. - weight };
                for (side, old) in sample.iter_mut().zip(previous) {
                    *side = old + (*side - old) * weight;
                }
            }
        }
        revised.voice_backend = Some(backend);
        revised.anchor = Some(original.source.clone());
        revised.patches.push(Arc::new(Patch {
            range: plan.range,
            samples: candidate,
        }));
        Ok(Some(revised))
    }
}

pub(crate) struct Reader {
    source: Arc<Mutex<DryReader>>,
    patches: Vec<Arc<Patch>>,
}

impl Reader {
    pub(crate) fn read_into(
        &mut self,
        start: usize,
        count: usize,
        out: &mut Vec<[f32; 2]>,
        poll: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let offset = out.len();
        self.source
            .lock()
            .unwrap()
            .read_into(start, count, out, poll)?;
        for patch in &self.patches {
            let first = start.max(patch.range[0]);
            let end = (start + count).min(patch.range[1]);
            if first < end {
                out[offset + first - start..offset + end - start]
                    .copy_from_slice(&patch.samples[first - patch.range[0]..end - patch.range[0]]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{uid, Store};
    use serde_json::json;

    #[test]
    fn plan_keeps_the_whole_vowel_and_bounds_edits_to_nearby_rests() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_voice_patch_plan"));
        let store = Store::new(root.clone())?;
        let mut track = Track::new("Lead");
        let project = store.create("Monitor patch", 120., [4, 4], vec![track.clone()])?;
        track.notes = [
            json!({"id":"before","start":0.5,"duration":1.0}),
            json!({"id":"head","start":6.0,"duration":1.0,"lyric":"あ"}),
            json!({"id":"tail","start":7.0,"duration":1.0,"vowel_continuation":true}),
            json!({"id":"after","start":26.0,"duration":1.0}),
        ]
        .iter()
        .map(Note::from_value)
        .collect::<Result<_>>()?;
        let project = store.edit(
            &project.id,
            project.revision,
            &json!([{"op":"add_notes","track_id":track.id,"notes":track.notes}]),
        )?;
        let len = crate::source_cache::frame_count(&project);
        let mut updated = track.clone();
        updated.notes[2].pitch += 4;
        let plan = Plan::new(&project, &track, &updated, len).unwrap();
        assert_eq!(plan.range, [3 * RATE - REST_MARGIN, 4 * RATE + REST_MARGIN]);
        updated.notes[3].pitch += 2;
        assert!(Plan::new(&project, &track, &updated, len).is_none());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn patched_reads_keep_every_outside_sample_and_apply_latest_overlap() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_voice_patch_read"));
        let project = uid("p");
        let source = Source {
            kind: "voicevox".into(),
            track_id: uid("t"),
            key: "a".repeat(64),
        };
        let raw = (0..32).map(|i| [i as f32 * 0.01; 2]).collect::<Vec<_>>();
        source.write(&root, &project, &raw, || Ok(()))?;
        let mut input = Input::from(source.clone());
        input.anchor = Some(input.open(&root, &project, 32)?.source);
        input.patches = vec![
            Arc::new(Patch {
                range: [8, 15],
                samples: vec![[0.5; 2]; 7],
            }),
            Arc::new(Patch {
                range: [13, 19],
                samples: vec![[0.9; 2]; 6],
            }),
        ];
        let mut reader = input.open(&root, &project, 32)?;
        let mut chunks = Vec::new();
        reader.read_into(0, 12, &mut chunks, || Ok(()))?;
        reader.read_into(12, 20, &mut chunks, || Ok(()))?;
        assert_eq!(chunks[..8], raw[..8]);
        assert_eq!(chunks[8..13], [[0.5; 2]; 5]);
        assert_eq!(chunks[13..19], [[0.9; 2]; 6]);
        assert_eq!(chunks[19..], raw[19..]);
        assert_eq!(source.read(&root, &project, 32, || Ok(()))?, raw);
        drop((reader, input));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
