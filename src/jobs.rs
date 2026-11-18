use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn terminal(status: &str) -> bool {
    matches!(status, "complete" | "failed" | "cancelled" | "interrupted")
}
struct Inner {
    value: Value,
    saved: Instant,
}
/// The job lock orders cancellation and the non-cancellable commit boundary.
/// It never takes the project-store lock, including while writing a receipt.
pub struct Control {
    entry: Entry,
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}
impl Control {
    pub fn new(
        root: &Path,
        id: &str,
        project_id: &str,
        revision: u64,
        engine: &str,
    ) -> Result<Self> {
        crate::model::identifier(id)?;
        let mut control = Self::ephemeral(id, project_id, revision, engine);
        control.path = Some(root.join("runtime/jobs").join(format!("{id}.json")));
        control.save(&control.inner.lock().unwrap().value)?;
        Ok(control)
    }
    pub(crate) fn ephemeral(id: &str, project_id: &str, revision: u64, engine: &str) -> Self {
        let started_at_ms = now();
        let value = json!({"job_id":id,"project_id":project_id,"revision":revision,"engine":engine,"status":"running","stage":"preparing","progress":0.,"track_id":null,"track_name":null,"cancel_requested":false,"can_cancel":true,"started_at_ms":started_at_ms,"updated_at_ms":started_at_ms});
        Self {
            entry: Entry {
                job_id: id.into(),
                project_id: project_id.into(),
                revision,
                engine: engine.into(),
                started_at_ms,
            },
            path: None,
            inner: Mutex::new(Inner {
                value,
                saved: Instant::now(),
            }),
        }
    }
    fn save(&self, value: &Value) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        fs::create_dir_all(path.parent().unwrap())?;
        let temp = path.with_extension("json.tmp");
        let mut f = fs::File::create(&temp)?;
        f.write_all(&serde_json::to_vec_pretty(value)?)?;
        f.sync_all()?;
        drop(f);
        #[cfg(windows)]
        crate::model::replace_windows(&temp, path)?;
        #[cfg(not(windows))]
        fs::rename(&temp, path)?;
        Ok(())
    }
    pub fn snapshot(&self) -> Value {
        self.inner.lock().unwrap().value.clone()
    }
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.inner.lock().unwrap().value["cancel_requested"]
                .as_bool()
                .unwrap_or(false),
            "RENDER_CANCELLED"
        );
        Ok(())
    }
    pub fn progress(&self, stage: &str, progress: f64, track: Option<(&str, &str)>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        ensure!(
            !inner.value["cancel_requested"].as_bool().unwrap_or(false),
            "RENDER_CANCELLED"
        );
        inner.value["stage"] = json!(stage);
        inner.value["progress"] =
            json!(progress.clamp(inner.value["progress"].as_f64().unwrap_or(0.), 0.99));
        inner.value["track_id"] = json!(track.map(|x| x.0).filter(|id| !id.is_empty()));
        inner.value["track_name"] = json!(track.map(|x| x.1));
        inner.value["updated_at_ms"] = json!(now());
        // The live job exposes every stage. Coalesce intermediate disk writes.
        if inner.saved.elapsed().as_secs() >= 1 {
            inner.saved = Instant::now();
            if let Err(error) = self.save(&inner.value) {
                eprintln!(
                    "Could not checkpoint render {}: {error:#}",
                    self.entry.job_id
                );
            }
        }
        Ok(())
    }
    pub fn cancel(&self) -> Result<Value> {
        let mut inner = self.inner.lock().unwrap();
        if terminal(inner.value["status"].as_str().unwrap_or(""))
            || !inner.value["can_cancel"].as_bool().unwrap_or(false)
        {
            return Ok(
                json!({"accepted":false,"job":inner.value,"reason":"Job has reached its terminal or output-commit boundary"}),
            );
        }
        let mut cancelling = inner.value.clone();
        cancelling["status"] = json!("cancelling");
        cancelling["cancel_requested"] = json!(true);
        cancelling["can_cancel"] = json!(false);
        cancelling["updated_at_ms"] = json!(now());
        self.save(&cancelling)?;
        inner.value = cancelling;
        Ok(
            json!({"accepted":true,"job":inner.value,"note":if self.entry.engine==crate::wave_delivery::ENGINE{
                "The delivery worker stops at the next block."
            }else{
                "Cooperative cancellation: an in-flight VOICEVOX request must return before the worker stops; busy remains true until then."
            }}),
        )
    }
    pub fn commit(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        ensure!(
            !inner.value["cancel_requested"].as_bool().unwrap_or(false),
            "RENDER_CANCELLED"
        );
        inner.value["status"] = json!("committing");
        inner.value["stage"] = json!("committing");
        inner.value["can_cancel"] = json!(false);
        inner.value["progress"] = json!(0.99);
        inner.value["updated_at_ms"] = json!(now());
        self.save(&inner.value)
    }
    pub fn finish(&self, result: Result<Value>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        match result {
            Ok(value) => {
                inner.value["status"] = json!("complete");
                inner.value["result"] = value;
                inner.value["progress"] = json!(1.);
            }
            Err(error) => {
                inner.value["status"] = json!(if inner.value["cancel_requested"] == true {
                    "cancelled"
                } else {
                    "failed"
                });
                if inner.value["status"] == "failed" {
                    inner.value["error"] = json!(format!("{error:#}"));
                }
            }
        }
        inner.value["stage"] = inner.value["status"].clone();
        inner.value["can_cancel"] = json!(false);
        inner.value["finished_at_ms"] = json!(now());
        inner.value["updated_at_ms"] = json!(now());
        self.save(&inner.value)
    }
}
pub fn read(root: &Path, id: &str) -> Result<Value> {
    crate::model::identifier(id)?;
    let report = root.join("exports").join(id).join("渲染报告.json");
    let path = root.join("runtime/jobs").join(format!("{id}.json"));
    let mut value = if path.exists() {
        serde_json::from_slice::<Value>(&fs::read(&path)?)?
    } else {
        ensure!(report.exists(), "Unknown job");
        json!({"job_id":id})
    };
    if !terminal(value["status"].as_str().unwrap_or("")) {
        value["can_cancel"] = json!(false);
        if report.exists() {
            let result: Value = serde_json::from_slice(&fs::read(report)?)?;
            value["project_id"] = result["project_id"].clone();
            value["revision"] = result["revision"].clone();
            value["engine"] = result["engine"].clone();
            value["status"] = json!("complete");
            value["result"] = result;
            value["progress"] = json!(1.);
        } else {
            value["status"] = json!("interrupted");
            value["error"] = json!("The owning worker stopped before publishing output. This job was not automatically retried.");
        }
        value["stage"] = value["status"].clone();
    }
    Ok(value)
}
#[derive(Clone, Deserialize)]
struct Entry {
    job_id: String,
    project_id: String,
    revision: u64,
    engine: String,
    #[serde(default)]
    started_at_ms: u64,
}

type ProjectEntries = HashMap<String, Entry>;

/// The service owns job creation. Index receipts once per service session and
/// follow jobs as they are registered. The receipts remain the source of truth.
pub(crate) struct History {
    root: PathBuf,
    entries: Mutex<Option<HashMap<String, ProjectEntries>>>,
}

impl History {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            entries: Mutex::new(None),
        }
    }

    fn scan(&self) -> Result<HashMap<String, ProjectEntries>> {
        let receipts = match fs::read_dir(self.root.join("runtime/jobs")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(error) => return Err(error.into()),
        };
        let mut index: HashMap<String, ProjectEntries> = HashMap::new();
        for receipt in receipts {
            let path = receipt?.path();
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            if path.extension().is_none_or(|e| e != "json") || crate::model::identifier(id).is_err()
            {
                continue;
            }
            let raw = match fs::read(&path) {
                Ok(raw) => raw,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    eprintln!("Could not index render receipt {id}: {error}");
                    continue;
                }
            };
            if let Ok(entry) = serde_json::from_slice::<Entry>(&raw) {
                if entry.job_id == id {
                    index
                        .entry(entry.project_id.clone())
                        .or_default()
                        .insert(id.to_owned(), entry);
                }
            }
        }
        Ok(index)
    }

    pub(crate) fn warm(&self) -> Result<()> {
        let mut entries = self.entries.lock().unwrap();
        if entries.is_none() {
            *entries = Some(self.scan()?);
        }
        Ok(())
    }

    pub(crate) fn record(&self, control: &Control) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entries) = entries.as_mut() {
            entries
                .entry(control.entry.project_id.clone())
                .or_default()
                .insert(control.entry.job_id.clone(), control.entry.clone());
        }
    }

    fn matching(
        &self,
        project: Option<&str>,
        includes: impl Fn(&Entry) -> bool,
    ) -> Result<Vec<String>> {
        self.warm()?;
        let entries = self.entries.lock().unwrap();
        let entries = entries.as_ref().unwrap();
        let mut matches: Vec<_> = if let Some(project) = project {
            entries
                .get(project)
                .into_iter()
                .flat_map(HashMap::values)
                .collect()
        } else {
            entries.values().flat_map(HashMap::values).collect()
        };
        matches.retain(|entry| includes(entry));
        matches.sort_unstable_by(|a, b| {
            b.started_at_ms
                .cmp(&a.started_at_ms)
                .then_with(|| b.job_id.cmp(&a.job_id))
        });
        Ok(matches
            .into_iter()
            .map(|entry| entry.job_id.clone())
            .collect())
    }

    pub(crate) fn recent(&self, project: Option<&str>, deliveries: bool) -> Result<Vec<Value>> {
        Ok(self
            .matching(project, |entry| {
                (entry.engine == crate::wave_delivery::ENGINE) == deliveries
            })?
            .into_iter()
            .filter_map(|id| read(&self.root, &id).ok())
            .filter(|value| project.is_none_or(|id| value["project_id"] == id))
            .take(20)
            .collect())
    }
}

fn cache_file(root: &Path, project: &str, revision: u64, engine: &str) -> Result<PathBuf> {
    crate::model::identifier(project)?;
    ensure!(
        crate::engine::RENDER_ENGINES.contains(&engine),
        "Unsupported engine"
    );
    Ok(root
        .join("runtime/preview_cache")
        .join(format!("{project}-{revision}-{engine}.json")))
}

pub(crate) fn remember_mix(root: &Path, result: &Value) -> Result<()> {
    let project = crate::model::field(result, "project_id")?;
    let job = crate::model::field(result, "job_id")?;
    let engine = crate::model::field(result, "engine")?;
    let revision = result["revision"]
        .as_u64()
        .context("Missing rendered revision")?;
    let path = cache_file(root, project, revision, engine)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension(format!("{}.tmp", crate::model::uid("cache")));
    fs::write(&temp, serde_json::to_vec(job)?)?;
    #[cfg(windows)]
    let written = crate::model::replace_windows(&temp, &path);
    #[cfg(not(windows))]
    let written = fs::rename(&temp, &path).map_err(Into::into);
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

impl History {
    pub(crate) fn existing_mix(
        &self,
        project: &str,
        revision: u64,
        engine: &str,
    ) -> Result<Option<Value>> {
        let root = &self.root;
        let path = cache_file(root, project, revision, engine)?;
        let usable = |job: &Value| {
            let result = &job["result"];
            job["status"] == "complete"
                && result["project_id"] == project
                && result["revision"] == revision
                && result["engine"] == engine
                && result["job_id"].as_str().is_some_and(|id| {
                    crate::model::identifier(id).is_ok()
                        && root.join("exports").join(id).join("试听混音.wav").is_file()
                })
        };
        if let Some(job) = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<String>(&bytes).ok())
            .and_then(|id| read(root, &id).ok())
            .filter(usable)
        {
            return Ok(Some(job));
        }
        let mix = self
            .matching(Some(project), |entry| {
                entry.revision == revision && entry.engine == engine
            })?
            .into_iter()
            .filter_map(|id| read(root, &id).ok())
            .find(usable);
        if let Some(job) = &mix {
            let _ = remember_mix(root, &job["result"]);
        }
        Ok(mix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_indexes_old_versions_and_registers_new_jobs() -> Result<()> {
        let root = std::env::temp_dir().join(crate::model::uid("avs_job_index"));
        let receipts = root.join("runtime/jobs");
        fs::create_dir_all(&receipts)?;
        let result = |id: &str, revision: u64, engine: &str| json!({"job_id":id,"revision":revision,"engine":engine,"project_id":"p_test"});
        for revision in 1..=25 {
            let id = format!("render_{revision}");
            let engine = if revision == 1 { "voicevox" } else { "guide" };
            let mut value = result(&id, revision, engine);
            value["started_at_ms"] = json!(revision);
            value["status"] = json!("complete");
            value["result"] = result(&id, revision, engine);
            fs::write(
                receipts.join(format!("{id}.json")),
                serde_json::to_vec(&value)?,
            )?;
        }
        fs::write(receipts.join("bad.json"), b"missing receipt metadata")?;
        let old = root.join("exports/render_1");
        fs::create_dir_all(&old)?;
        fs::write(old.join("试听混音.wav"), [])?;
        let history = History::new(&root);
        let recent = history.recent(Some("p_test"), false)?;
        assert_eq!(recent.len(), 20);
        assert_eq!(recent[0]["job_id"], "render_25");
        assert!(!recent.iter().any(|value| value["job_id"] == "render_1"));
        assert_eq!(
            history.existing_mix("p_test", 1, "voicevox")?.unwrap()["job_id"],
            "render_1"
        );
        assert!(history.existing_mix("p_test", 1, "guide")?.is_none());
        assert!(history.recent(Some("p_other"), false)?.is_empty());
        fs::remove_file(old.join("试听混音.wav"))?;
        assert!(history.existing_mix("p_test", 1, "voicevox")?.is_none());

        let new = Control::new(&root, "render_new", "p_test", 1, "voicevox")?;
        history.record(&new);
        let dir = root.join("exports/render_new");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("试听混音.wav"), [])?;
        new.finish(Ok(result("render_new", 1, "voicevox")))?;
        assert_eq!(
            history.recent(Some("p_test"), false)?[0]["job_id"],
            "render_new"
        );
        assert_eq!(
            history.existing_mix("p_test", 1, "voicevox")?.unwrap()["job_id"],
            "render_new"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn cancellation_and_output_commit_are_ordered_and_recoverable() -> Result<()> {
        let root = std::env::temp_dir().join(crate::model::uid("avs_jobs"));
        let cancelled = Control::new(&root, "render_cancel", "p_test", 2, "guide")?;
        cancelled.progress("instrument", 0.4, Some(("t_piano", "钢琴")))?;
        assert!(cancelled.cancel()?["accepted"] == true);
        assert!(cancelled.check().is_err() && cancelled.commit().is_err());
        cancelled.finish(Err(anyhow::anyhow!("RENDER_CANCELLED")))?;
        assert_eq!(read(&root, "render_cancel")?["status"], "cancelled");
        let completed = Control::new(&root, "render_done", "p_test", 3, "guide")?;
        completed.commit()?;
        assert!(completed.cancel()?["accepted"] == false);
        completed.finish(Ok(json!({"revision":3})))?;
        assert_eq!(read(&root, "render_done")?["status"], "complete");
        let _interrupted = Control::new(&root, "render_lost", "p_test", 4, "guide")?;
        assert_eq!(read(&root, "render_lost")?["status"], "interrupted");
        let _published = Control::new(&root, "render_published", "p_test", 5, "guide")?;
        let dir = root.join("exports/render_published");
        fs::create_dir_all(&dir)?;
        fs::write(
            dir.join("渲染报告.json"),
            serde_json::to_vec(&json!({"project_id":"p_test","revision":5,"engine":"guide"}))?,
        )?;
        assert_eq!(read(&root, "render_published")?["status"], "complete");
        assert_eq!(History::new(&root).recent(Some("p_test"), false)?.len(), 4);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn denied_receipt_does_not_claim_a_cancellation() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(crate::model::uid("avs_cancel_lock"));
        let control = Control::new(&root, "render_locked", "p_test", 2, "guide")?;
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(root.join("runtime/jobs/render_locked.json"))?;
        assert!(control.cancel().is_err());
        assert_eq!(control.snapshot()["status"], "running");
        control.check()?;
        drop(handle);
        assert_eq!(control.cancel()?["accepted"], true);
        control.finish(Err(anyhow::anyhow!("RENDER_CANCELLED")))?;
        assert_eq!(read(&root, "render_locked")?["status"], "cancelled");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
