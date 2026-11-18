use crate::model::{identifier, uid, Project, Track};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const BLOCK: usize = 8192;
const HEADER: usize = 64;
// Current content per track and renderer. Open readers may outlive its directory entry.
const MAX_TRACK_BYTES: usize = (15 * 60 + 12) * 48_000 * 8;
const PREVIEW_VOICE_BYTES: usize = 96 * 1024 * 1024;
const PREVIEW_VOICE_LIMIT: usize = 12;

// Keep dry mono voice from recent auditions in memory. Saved export sources retain
// their separate disk cache and are never replaced by a draft.
type PreviewVoice = (String, Arc<Vec<f32>>);
static PREVIEW_VOICES: Mutex<VecDeque<PreviewVoice>> = Mutex::new(VecDeque::new());

pub(crate) struct DryReader {
    reader: BufReader<File>,
    len: usize,
}

impl DryReader {
    pub(crate) fn read_into(
        &mut self,
        start: usize,
        count: usize,
        out: &mut Vec<[f32; 2]>,
        poll: impl Fn() -> Result<()>,
    ) -> Result<()> {
        ensure!(
            start <= self.len && count <= self.len - start,
            "Dry track range outside source"
        );
        self.reader
            .seek(SeekFrom::Start((HEADER + start * 8) as u64))?;
        out.try_reserve_exact(count)?;
        let mut remain = count;
        let mut buf = [0; BLOCK * 8];
        while remain > 0 {
            poll()?;
            let frames = remain.min(BLOCK);
            self.reader.read_exact(&mut buf[..frames * 8])?;
            for bytes in buf[..frames * 8].as_chunks::<8>().0 {
                let frame = [
                    f32::from_le_bytes(bytes[..4].try_into().unwrap()),
                    f32::from_le_bytes(bytes[4..].try_into().unwrap()),
                ];
                ensure!(
                    frame[0].is_finite() && frame[1].is_finite(),
                    "Dry track contains invalid samples"
                );
                out.push(frame);
            }
            remain -= frames;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Source {
    pub track_id: String,
    pub kind: String,
    pub key: String,
}

pub(crate) fn active(track: &Track, solo: bool) -> bool {
    !track.mute && (!solo || track.solo) && has_material(track)
}

pub(crate) fn has_material(track: &Track) -> bool {
    !track.notes.is_empty() || track.has_audio()
}

pub(crate) fn frame_count(project: &Project) -> usize {
    ((project.seconds() + project.tail_seconds) * 48_000.).ceil() as usize
}

pub(crate) fn cache_frame_count(frames: usize) -> usize {
    (frames + (frames / 8).min(48_000)).min(MAX_TRACK_BYTES / 8)
}

fn hash(value: serde_json::Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?)))
}

fn material(project: &Project, track: &Track) -> serde_json::Value {
    let mut value = json!({"bpm":project.bpm,"tempo":project.tempo_map,"notes":track.notes,
        "patch":track.instrument,"controls":track.controllers,"singer":track.singer_id,
        "clips":track.media_clips()});
    if let Some(profile) = &track.openutau_voice {
        value["openutau_voice"] = json!(profile);
    }
    value
}

pub(crate) fn source_fingerprint(project: &Project, track: &Track, len: usize) -> Result<String> {
    hash(json!([len, material(project, track)]))
}

impl Source {
    pub(crate) fn new(project: &Project, track: &Track, kind: &str, backend: &str) -> Result<Self> {
        Ok(Self {
            track_id: track.id.clone(),
            kind: kind.into(),
            key: hash(json!([
                "avs-dry-v2",
                env!("CARGO_PKG_VERSION"),
                material(project, track),
                kind,
                backend
            ]))?,
        })
    }

    pub(crate) fn preview_voice(&self) -> Option<Arc<Vec<f32>>> {
        let mut voices = PREVIEW_VOICES.lock().unwrap();
        let index = voices.iter().position(|(key, _)| key == &self.key)?;
        let hit = voices.remove(index).unwrap();
        let samples = hit.1.clone();
        voices.push_front(hit);
        Some(samples)
    }

    pub(crate) fn keep_preview_voice(&self, mono: Vec<f32>) -> Arc<Vec<f32>> {
        let mono = Arc::new(mono);
        let bytes = mono.capacity() * size_of::<f32>();
        if bytes > PREVIEW_VOICE_BYTES {
            return mono;
        }
        let mut voices = PREVIEW_VOICES.lock().unwrap();
        voices.retain(|(key, _)| key != &self.key);
        let mut used = voices
            .iter()
            .map(|(_, samples)| samples.capacity() * size_of::<f32>())
            .sum::<usize>();
        while used + bytes > PREVIEW_VOICE_BYTES || voices.len() >= PREVIEW_VOICE_LIMIT {
            let (_, removed) = voices.pop_back().unwrap();
            used -= removed.capacity() * size_of::<f32>();
        }
        voices.push_front((self.key.clone(), mono.clone()));
        mono
    }

    pub(crate) fn discard_preview_voice(&self) {
        PREVIEW_VOICES
            .lock()
            .unwrap()
            .retain(|(key, _)| key != &self.key);
    }

    fn path(&self, root: &Path, project_id: &str) -> Result<PathBuf> {
        identifier(project_id)?;
        identifier(&self.track_id)?;
        ensure!(
            ["audio", "instrument", "guide", "voicevox", "openutau"].contains(&self.kind.as_str()),
            "Unknown dry track type"
        );
        ensure!(
            self.key.len() == HEADER && self.key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid dry track fingerprint"
        );
        Ok(root
            .join("runtime/source_cache")
            .join(project_id)
            .join(&self.track_id)
            .join(format!("{}-{}.pcm", self.kind, self.key)))
    }

    pub(crate) fn read(
        &self,
        root: &Path,
        project_id: &str,
        len: usize,
        poll: impl Fn() -> Result<()>,
    ) -> Result<Vec<[f32; 2]>> {
        let mut reader = self.open(root, project_id, len)?;
        let mut out = Vec::new();
        reader.read_into(0, len, &mut out, poll)?;
        Ok(out)
    }

    pub(crate) fn open(&self, root: &Path, project_id: &str, len: usize) -> Result<DryReader> {
        let len_bytes = len
            .checked_mul(8)
            .filter(|n| *n <= MAX_TRACK_BYTES)
            .ok_or_else(|| anyhow::anyhow!("Dry track exceeds cache size"))?;
        let path = self.path(root, project_id)?;
        let file = match File::open(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                File::open(path.with_file_name(format!("{}.pcm", self.kind)))?
            }
            result => result?,
        };
        let bytes = file
            .metadata()?
            .len()
            .checked_sub(HEADER as u64)
            .context("Dry track header is incomplete")?;
        ensure!(
            bytes <= MAX_TRACK_BYTES as u64 && bytes % 8 == 0,
            "Dry track length is invalid"
        );
        ensure!(
            bytes >= len_bytes as u64,
            "Dry track does not cover the requested duration"
        );
        let mut reader = BufReader::new(file);
        let mut header = [0; HEADER];
        reader.read_exact(&mut header)?;
        ensure!(header == self.key.as_bytes(), "Dry track source changed");
        Ok(DryReader { reader, len })
    }

    pub(crate) fn write(
        &self,
        root: &Path,
        project_id: &str,
        frames: &[[f32; 2]],
        poll: impl Fn() -> Result<()>,
    ) -> Result<()> {
        ensure!(
            frames.len() <= MAX_TRACK_BYTES / 8,
            "Dry track exceeds cache size"
        );
        let path = self.path(root, project_id)?;
        fs::create_dir_all(path.parent().unwrap())?;
        #[cfg(windows)]
        let path = path
            .parent()
            .unwrap()
            .canonicalize()?
            .join(path.file_name().unwrap());
        let temp = path.with_file_name(uid("dry") + ".tmp");
        let result = (|| {
            let mut writer = BufWriter::new(File::create_new(&temp)?);
            writer.write_all(self.key.as_bytes())?;
            let mut buffer = [0; BLOCK * 8];
            for chunk in frames.chunks(BLOCK) {
                poll()?;
                for (frame, bytes) in chunk.iter().zip(buffer.as_chunks_mut::<8>().0) {
                    ensure!(
                        frame[0].is_finite() && frame[1].is_finite(),
                        "Invalid source sample"
                    );
                    bytes[..4].copy_from_slice(&frame[0].to_le_bytes());
                    bytes[4..].copy_from_slice(&frame[1].to_le_bytes());
                }
                writer.write_all(&buffer[..chunk.len() * 8])?;
            }
            writer.flush()?;
            drop(writer);
            #[cfg(windows)]
            crate::model::replace_windows(&temp, &path)?;
            #[cfg(not(windows))]
            fs::rename(&temp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        } else {
            let prefix = format!("{}-", self.kind);
            let legacy = format!("{}.pcm", self.kind);
            if let Ok(entries) = fs::read_dir(path.parent().unwrap()) {
                let mut neighbouring_keys = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else { continue };
                    let older = name
                        .strip_prefix(&prefix)
                        .and_then(|tail| tail.strip_suffix(".pcm"))
                        .is_some_and(|key| {
                            key != self.key
                                && key.len() == HEADER
                                && key.bytes().all(|b| b.is_ascii_hexdigit())
                        });
                    if name == legacy {
                        let _ = fs::remove_file(entry.path());
                    } else if let Some(modified) = older
                        .then(|| {
                            entry
                                .metadata()
                                .and_then(|metadata| metadata.modified())
                                .ok()
                        })
                        .flatten()
                    {
                        neighbouring_keys.push((modified, entry.path()));
                    }
                }
                // A concurrent monitor or export may still be opening an adjacent revision.
                neighbouring_keys
                    .sort_unstable_by_key(|(modified, _)| std::cmp::Reverse(*modified));
                for (modified, candidate) in neighbouring_keys.into_iter().skip(2) {
                    if fs::metadata(&candidate)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        == Some(modified)
                    {
                        let _ = fs::remove_file(candidate);
                    }
                }
            }
        }
        result
    }
}

pub(crate) fn fader(track: &Track) -> [f64; 2] {
    fader_values(track, track.gain_db, track.pan)
}

pub(crate) fn fader_values(track: &Track, gain_db: f64, pan: f64) -> [f64; 2] {
    let gain = 10f64.powf(gain_db / 20.);
    if track.is_audio() || track.instrument.is_some() {
        [1. - pan.max(0.), 1. + pan.min(0.)].map(|balance| gain * balance)
    } else {
        [
            ((1. - pan) / 2.).sqrt() * gain,
            ((1. + pan) / 2.).sqrt() * gain,
        ]
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn saved_pitch_draft_has_the_same_source_key() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_pitch_cache"));
        let store = crate::model::Store::new(root.clone())?;
        let mut track = Track::new("Vocal");
        let note = crate::model::Note::from_value(
            &json!({"start":5.,"duration":1.,"pitch":62,"lyric":"イ"}),
        )?;
        let notes = vec![(track.id.clone(), note.id.clone())];
        track.notes.push(note);
        let base = store.create("Cache fixture", 120., [4, 4], vec![track])?;
        let base = store.edit(
            &base.id,
            base.revision,
            &json!([{"op":"set_project","values":{
            "tempo_map":[{"beat":4,"bpm":100,"curve":"linear"}]}}]),
        )?;
        let selection = crate::selection::Selection {
            id: uid("selection"),
            project_id: base.id.clone(),
            revision: base.revision,
            notes,
        };
        let request = json!({"kind":"pitch_edge","side":"end","length_ms":85,"offset_cents":-150});
        let draft = crate::selection::pitch_audition(&root, &base, &selection, &request)?.candidate;
        let preview = crate::selection::preview(&base, &selection, &request)?;
        store.edit(&base.id, base.revision, &preview["operations"])?;
        let saved = store.read(&base.id)?;
        let key = |project: &Project| {
            source_fingerprint(project, &project.tracks[0], frame_count(project))
        };
        assert_eq!(key(&draft)?, key(&saved)?);
        let changed = json!({"kind":"pitch_edge","side":"end","length_ms":85,"offset_cents":-151});
        let changed =
            crate::selection::pitch_audition(&root, &base, &selection, &changed)?.candidate;
        assert_ne!(key(&changed)?, key(&saved)?);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn open_dry_reader_keeps_the_original_snapshot_after_cache_replacement() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_dry_reader"));
        let p = uid("p");
        let track = uid("t");
        let source = Source {
            track_id: track,
            kind: "audio".into(),
            key: "a".repeat(64),
        };
        let old = [[0.25, -0.25], [0.1, 0.2], [0.15, -0.6], [0.5, 0.75]];
        source.write(&root, &p, &old, || Ok(()))?;
        assert_eq!(source.read(&root, &p, 2, || Ok(()))?, old[..2]);
        assert!(source.open(&root, &p, old.len() + 1).is_err());
        let mut reader = source.open(&root, &p, old.len())?;
        let replacement = Source {
            key: "b".repeat(64),
            ..source.clone()
        };
        let new = [[0.8, 0.9]; 4];
        replacement.write(&root, &p, &new, || Ok(()))?;
        let mut samples = Vec::new();
        reader.read_into(2, 2, &mut samples, || Ok(()))?;
        reader.read_into(0, 2, &mut samples, || Ok(()))?;
        assert_eq!(samples, [old[2], old[3], old[0], old[1]]);
        assert_eq!(replacement.read(&root, &p, new.len(), || Ok(()))?, new);
        drop(reader);
        let _ = fs::remove_file(source.path(&root, &p)?);
        let path = replacement.path(&root, &p)?;
        fs::remove_file(&path)?;
        for parent in path.ancestors().skip(1).take(5) {
            fs::remove_dir(parent)?;
        }
        Ok(())
    }
}
