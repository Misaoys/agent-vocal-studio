use crate::model::{identifier, uid, Project};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, Metadata},
    path::Path,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SCHEMA: u32 = 2;

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum Scope {
    #[default]
    All,
    Workspace,
    Technical,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Query {
    scope: Scope,
    query: String,
    offset: usize,
    limit: Option<usize>,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified_ns: u128,
    created_ns: Option<u128>,
}

fn timestamp(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

impl Stamp {
    fn from_metadata(metadata: Metadata) -> Option<Self> {
        if !metadata.is_file() {
            return None;
        }
        Some(Self {
            bytes: metadata.len(),
            modified_ns: timestamp(metadata.modified().ok()?),
            created_ns: metadata.created().ok().map(timestamp),
        })
    }

    fn at(path: &Path) -> Option<Self> {
        Self::from_metadata(fs::metadata(path).ok()?)
    }
}

#[derive(Deserialize, Serialize)]
struct Summary {
    id: String,
    title: String,
    revision: u64,
    bpm: f64,
    tracks: usize,
    notes: usize,
    modified_ms: u64,
    technical_test: bool,
}

#[derive(Deserialize, Serialize)]
struct Entry {
    stamp: Stamp,
    assets: Vec<(String, Stamp)>,
    summary: Summary,
}

type Entries = HashMap<String, Entry>;

#[derive(Deserialize, Serialize)]
struct Snapshot<T> {
    schema: u32,
    entries: T,
}

#[derive(Default)]
struct Catalog {
    entries: Option<Entries>,
    pending_write: bool,
    last_write: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct ProjectIndex {
    catalog: Mutex<Catalog>,
    changed: Mutex<HashSet<String>>,
}

impl ProjectIndex {
    pub(crate) fn invalidate(&self, id: &str) {
        self.changed.lock().unwrap().insert(id.to_owned());
    }

    pub(crate) fn list(
        &self,
        root: &Path,
        request: &Query,
        read: impl Fn(&str) -> Result<Project>,
    ) -> Result<Value> {
        ensure!(
            request.limit.is_none_or(|limit| (1..=200).contains(&limit)),
            "project_list limit must be 1–200"
        );
        ensure!(
            request.query.chars().count() <= 160,
            "project_list query must contain at most 160 characters"
        );
        let search = request.query.to_lowercase();
        let words = search.split_whitespace().collect::<Vec<_>>();
        let mut catalog = self.catalog.lock().unwrap();
        let files = fs::read_dir(root.join("projects"))?;
        let path = root.join("runtime/project-index.json");
        let mut previous = catalog.entries.take().unwrap_or_else(|| load(&path));
        let dirty = std::mem::take(&mut *self.changed.lock().unwrap());
        let mut next = HashMap::with_capacity(previous.len());
        let mut asset_stamps = HashMap::new();
        let asset_root = root.join("assets");
        let asset_stamp = |name: &str| -> Option<Stamp> {
            crate::clips::validate_asset_name(name).ok()?;
            Stamp::at(&asset_root.join(name))
        };
        let mut changed = false;
        for entry in files {
            let entry = entry?;
            let project_path = entry.path();
            if project_path.extension().and_then(|ext| ext.to_str()) != Some("avproj") {
                continue;
            }
            let Some(id) = project_path
                .file_stem()
                .and_then(|id| id.to_str())
                .filter(|id| identifier(id).is_ok())
            else {
                continue;
            };
            let Some(stamp) = entry.metadata().ok().and_then(Stamp::from_metadata) else {
                continue;
            };
            let old = previous.remove(id);
            if let Some(old) = old.filter(|old| {
                old.stamp == stamp
                    && !dirty.contains(id)
                    && old.assets.iter().all(|(name, expected)| {
                        *asset_stamps
                            .entry(name.clone())
                            .or_insert_with(|| asset_stamp(name))
                            == Some(*expected)
                    })
            }) {
                next.insert(id.to_owned(), old);
                continue;
            }
            changed = true;
            // An editor may atomically replace a project while the directory is being read.
            // Only retain a summary for the version whose document was actually validated.
            for _ in 0..2 {
                let Some(current) = Stamp::at(&project_path) else {
                    break;
                };
                let Ok(project) = read(id) else {
                    break;
                };
                if Stamp::at(&project_path) != Some(current) {
                    continue;
                }
                let names = project
                    .tracks
                    .iter()
                    .flat_map(|track| {
                        track
                            .audio
                            .iter()
                            .map(|audio| audio.asset.as_str())
                            .chain(track.clips.iter().flatten().map(|clip| clip.asset.as_str()))
                    })
                    .collect::<HashSet<_>>();
                let Some(assets) = names
                    .into_iter()
                    .map(|name| {
                        let stamp = asset_stamp(name);
                        asset_stamps.insert(name.to_owned(), stamp);
                        Some((name.to_owned(), stamp?))
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    break;
                };
                next.insert(
                    id.to_owned(),
                    Entry {
                        stamp: current,
                        assets,
                        summary: Summary {
                            technical_test: project.composition.as_ref().is_some_and(|design| {
                                design["purpose"] == "technical_test"
                                    || design["fixture_role"] == "technical_test"
                            }),
                            id: project.id,
                            title: project.title,
                            revision: project.revision,
                            bpm: project.bpm,
                            tracks: project.tracks.len(),
                            notes: project.tracks.iter().map(|track| track.notes.len()).sum(),
                            modified_ms: (current.modified_ns / 1_000_000) as u64,
                        },
                    },
                );
                break;
            }
        }
        changed |= !previous.is_empty();
        let mut rows = next
            .values()
            .map(|entry| &entry.summary)
            .filter(|row| match request.scope {
                Scope::All => true,
                Scope::Workspace => !row.technical_test,
                Scope::Technical => row.technical_test,
            })
            .filter(|row| {
                if words.is_empty() {
                    return true;
                }
                let title = row.title.to_lowercase();
                words
                    .iter()
                    .all(|word| title.contains(word) || row.id.contains(word))
            })
            .collect::<Vec<_>>();
        rows.sort_unstable_by(|left, right| {
            right
                .modified_ms
                .cmp(&left.modified_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        let result = serde_json::to_value(
            rows.into_iter()
                .skip(request.offset)
                .take(request.limit.unwrap_or(usize::MAX))
                .collect::<Vec<_>>(),
        )?;
        catalog.pending_write |= changed;
        if catalog.pending_write
            && catalog
                .last_write
                .is_none_or(|before| before.elapsed() >= Duration::from_secs(2))
        {
            catalog.pending_write = !persist(&path, &next);
            catalog.last_write = Some(Instant::now());
        }
        catalog.entries = Some(next);
        Ok(result)
    }
}

fn load(path: &Path) -> Entries {
    fs::read(path)
        .ok()
        .and_then(|contents| serde_json::from_slice::<Snapshot<Entries>>(&contents).ok())
        .filter(|snapshot| snapshot.schema == SCHEMA)
        .map_or_else(Entries::new, |snapshot| snapshot.entries)
}

fn persist(path: &Path, entries: &Entries) -> bool {
    let temporary = path.with_extension(format!("{}.tmp", uid("idx")));
    let result = (|| -> Result<()> {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(
            &temporary,
            serde_json::to_vec(&Snapshot {
                schema: SCHEMA,
                entries,
            })?,
        )?;
        #[cfg(windows)]
        crate::model::replace_windows(&temporary, path)?;
        #[cfg(not(windows))]
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AudioClip, Store, Track};
    use std::cell::Cell;

    #[test]
    fn catalog_reuses_verified_rows_and_rechecks_changed_projects_and_audio() -> Result<()> {
        let root = std::env::temp_dir().join(uid("studio_index_test"));
        let store = Store::new(root.clone())?;
        let assets = root.join("assets");
        fs::create_dir_all(&assets)?;
        let wav_path = assets.join("catalog.wav");
        let mut wav = hound::WavWriter::create(
            &wav_path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for _ in 0..96 {
            wav.write_sample(0f32)?;
        }
        wav.finalize()?;
        let mut track = Track::new("Audio");
        track.audio = Some(AudioClip {
            asset: "catalog.wav".into(),
            start: 0.,
            seconds: 0.001,
        });
        let project = store.create("Original", 120., [4, 4], vec![track])?;
        let project_path = root.join("projects").join(format!("{}.avproj", project.id));
        let index = ProjectIndex::default();
        let reads = Cell::new(0);
        let read = |id: &str| {
            reads.set(reads.get() + 1);
            store.read(id)
        };
        let request = Query::default();
        let initial = index.list(&root, &request, read)?;
        assert_eq!(initial[0]["title"], "Original");
        assert_eq!(reads.get(), 1);
        assert_eq!(index.list(&root, &request, read)?, initial);
        let reopened = ProjectIndex::default();
        assert_eq!(reopened.list(&root, &request, read)?, initial);
        assert_eq!(
            reads.get(),
            1,
            "warm and reopened catalogs reuse the validation"
        );

        let audio = fs::read(&wav_path)?;
        fs::write(&wav_path, b"invalid wav")?;
        assert_eq!(reopened.list(&root, &request, read)?, serde_json::json!([]));
        fs::write(&wav_path, audio)?;
        assert_eq!(reopened.list(&root, &request, read)?, initial);
        fs::remove_file(&wav_path)?;
        assert_eq!(reopened.list(&root, &request, read)?, serde_json::json!([]));
        fs::remove_file(&project_path)?;
        let changed = store.create("Replacement", 130., [3, 4], vec![Track::new("Vocal")])?;
        let rows = reopened.list(&root, &request, read)?;
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["id"], changed.id);
        assert_eq!(rows[0]["title"], "Replacement");
        let changed_path = root.join("projects").join(format!("{}.avproj", changed.id));
        fs::write(changed_path, b"invalid project")?;
        assert_eq!(reopened.list(&root, &request, read)?, serde_json::json!([]));
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
