//! Task-local comparison audio and explicit delivery of a prepared range pair.
use crate::{
    engine,
    jobs::Control,
    model::{identifier, Project},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) const RATE: usize = 48_000;
pub(crate) const MAX_FRAMES: usize = 45 * RATE;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MatchMethod {
    #[default]
    Rms,
    Lufs,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceInput {
    PreparedAudio,
    SavedMix,
    SavedStem,
    Rendered,
    LocalVoice,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub id: String,
    pub project_id: String,
    pub revision: u64,
    pub beat_range: [f64; 2],
    pub frame_ranges: [[usize; 2]; 2],
    pub timing: [Timing; 2],
    pub matched_rms: bool,
    pub level_match: bool,
    #[serde(default)]
    pub match_method: MatchMethod,
    #[serde(default)]
    pub matched_lufs: bool,
    #[serde(default)]
    pub lufs_before: [Option<f64>; 2],
    #[serde(default)]
    pub lufs_after: [Option<f64>; 2],
    #[serde(default)]
    pub short_rms_fallback: bool,
    pub gain_db: [f32; 2],
    pub silent: [bool; 2],
    #[serde(default)]
    pub source_trim_db: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<SavedSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recordings: Option<[crate::clips::ClipRef; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_inputs: Option<[SourceInput; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<SavedOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) local_voice_track_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SavedSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renders: Option<[SavedRender; 2]>,
    pub beat_ranges: [[f64; 2]; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<[Option<Vec<SavedOutput>>; 2]>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedRender {
    pub project_id: String,
    pub revision: u64,
    pub job_id: String,
}

impl SavedSource {
    fn jobs(&self) -> Result<[&str; 2]> {
        match (&self.job_id, &self.renders) {
            (Some(job), None) => Ok([job; 2]),
            (None, Some([a, b])) => Ok([&a.job_id, &b.job_id]),
            _ => anyhow::bail!("使用一个渲染 ID 或两份已保存的渲染"),
        }
    }
}

impl SavedRender {
    pub(crate) fn snapshot(&self, root: &Path) -> Result<(Project, Value)> {
        let result = completed_result_binding(root, &self.project_id, self.revision, &self.job_id)?;
        let directory = export_directory(root, &self.job_id)?;
        let score = fs::File::open(directory.join("工程快照.json"))?;
        let project: Project = serde_json::from_reader(score.take(32_000_000))?;
        ensure!(
            project.id == self.project_id && project.revision == self.revision,
            "RENDER_REVISION_MISMATCH: A/B 来源与渲染快照不符"
        );
        project.validate()?;
        Ok((project, result))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub(crate) enum SavedOutput {
    Track(String),
    Group(String),
    Return(String),
}

pub(crate) fn outputs_schema() -> Value {
    serde_json::json!({"type":"array","minItems":1,"maxItems":44,"uniqueItems":true,
        "items":{"type":"object","properties":{"kind":{"type":"string","enum":["track","group","return"]},"id":{"type":"string"}},
            "required":["kind","id"],"additionalProperties":false}})
}

pub(crate) fn output_name<'a>(project: &'a Project, output: &SavedOutput) -> Result<&'a str> {
    match output {
        SavedOutput::Track(id) => {
            let track = project
                .tracks
                .iter()
                .find(|track| &track.id == id)
                .context("找不到请求的直出轨")?;
            ensure!(
                track.output_group.is_none(),
                "该轨进入了编组，请选编组输出或全混音"
            );
            Ok(&track.name)
        }
        SavedOutput::Group(id) => project
            .mix_groups
            .iter()
            .find(|group| &group.id == id)
            .map(|group| group.name.as_str())
            .context("找不到请求的编组输出"),
        SavedOutput::Return(id) => project
            .aux_buses
            .iter()
            .find(|bus| &bus.id == id)
            .map(|bus| bus.name.as_str())
            .context("找不到请求的返回通道"),
    }
}

pub(crate) fn completed_result(root: &Path, project: &Project, job_id: &str) -> Result<Value> {
    completed_result_binding(root, &project.id, project.revision, job_id)
}

fn completed_result_binding(
    root: &Path,
    project_id: &str,
    revision: u64,
    job_id: &str,
) -> Result<Value> {
    let job = crate::jobs::read(root, job_id)?;
    ensure!(job["status"] == "complete", "RENDER_NOT_COMPLETE");
    let result = &job["result"];
    ensure!(
        result["project_id"] == project_id && result["revision"] == revision,
        "RENDER_REVISION_MISMATCH: 选择本版本的混音"
    );
    Ok(result.clone())
}

pub(crate) fn export_directory(root: &Path, job_id: &str) -> Result<PathBuf> {
    identifier(job_id)?;
    let exports = root.join("exports").canonicalize()?;
    let directory = exports.join(job_id).canonicalize()?;
    ensure!(
        directory.parent() == Some(exports.as_path()),
        "混音文件路径无效"
    );
    Ok(directory)
}

impl Manifest {
    pub(crate) fn ranges(&self) -> [[f64; 2]; 2] {
        self.saved
            .as_ref()
            .map_or([self.beat_range; 2], |source| source.beat_ranges)
    }

    pub(crate) fn view(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id,"project_id":self.project_id,"revision":self.revision,
            "beat_range":self.beat_range,"frame_ranges":self.frame_ranges,"matched_rms":self.matched_rms,
            "level_match":self.level_match,"match_method":self.match_method,"matched_lufs":self.matched_lufs,
            "lufs_before":self.lufs_before,"lufs_after":self.lufs_after,"short_rms_fallback":self.short_rms_fallback,
            "gain_db":self.gain_db,"silent":self.silent,"source_trim_db":self.source_trim_db,
            "beat_ranges":self.ranges(),"saved":self.saved,"recordings":self.recordings,
            "source_inputs":self.source_inputs,"output":self.output,
            "local_voice_track_id":self.local_voice_track_id})
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Timing {
    pub bpm: f64,
    pub tempos: Vec<crate::timeline::TempoPoint>,
}
impl Timing {
    fn from_project(project: &Project) -> Self {
        Self {
            bpm: project.bpm,
            tempos: project.tempo_map.clone(),
        }
    }
    #[cfg(windows)]
    pub(crate) fn clock(&self) -> crate::timeline::Timeline {
        crate::timeline::Timeline::new(self.bpm, [4, 4], &self.tempos, &[])
    }
}

fn folder(root: &Path, id: &str) -> PathBuf {
    root.join("runtime/agent_auditions").join(id)
}

pub(crate) struct Files {
    path: PathBuf,
}
impl Files {
    pub(crate) fn export_pair(&self, root: &Path, manifest: &Manifest) -> Result<Value> {
        let names = ["A.wav", "B.wav"];
        let directory = self.path.canonicalize()?;
        ensure!(
            directory.parent()
                == Some(
                    root.join("runtime/agent_auditions")
                        .canonicalize()?
                        .as_path()
                ),
            "试听路径无效"
        );
        let mut sources = Vec::new();
        for name in names {
            let path = directory.join(name).canonicalize()?;
            ensure!(
                path.parent() == Some(directory.as_path()),
                "试听文件路径无效"
            );
            sources.push(fs::File::open(path)?);
        }
        let mut credits = [Value::Null, Value::Null];
        if let Some(saved) = &manifest.saved {
            for (index, job) in saved.jobs()?.into_iter().enumerate() {
                let (project, revision) = saved.renders.as_ref().map_or(
                    (manifest.project_id.as_str(), manifest.revision),
                    |renders| (renders[index].project_id.as_str(), renders[index].revision),
                );
                credits[index] =
                    completed_result_binding(root, project, revision, job)?["credits"].clone();
            }
        }
        let contents = serde_json::json!({
            "format": "avs-range-audition", "schema_version": 1, "pair": manifest.view(),
            "timing": manifest.timing, "credits": credits, "sample_rate": RATE, "bit_depth": 24,
            "channels": 2, "files": names,
        });
        let id = crate::model::uid("audition_export");
        let name = format!("试听对照_r{}.zip", manifest.revision);
        let stage = root.join("runtime").join(&id);
        let pending = stage.join(&name);
        let exports = root.join("exports");
        fs::create_dir_all(&exports)?;
        let target = exports.join(&id);
        fs::create_dir(&stage)?;
        let result = (|| -> Result<Value> {
            let mut archive = zip::ZipWriter::new(BufWriter::new(fs::File::create(&pending)?));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, mut wav) in names.into_iter().zip(sources) {
                archive.start_file(name, options)?;
                io::copy(&mut wav, &mut archive)?;
            }
            archive.start_file("试听清单.json", options)?;
            archive.write_all(&serde_json::to_vec_pretty(&contents)?)?;
            archive.finish()?.into_inner()?.sync_all()?;
            fs::rename(&stage, &target)?;
            Ok(serde_json::json!({
                "audition_id": manifest.id, "path": target.join(&name),
                "url": format!("/exports/{id}/{name}"), "files": names,
            }))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&pending);
            let _ = fs::remove_dir(&stage);
        }
        result
    }

    pub(crate) fn measure(
        &self,
        manifest: &Manifest,
        projects: [&Project; 2],
        requested: Option<[f64; 2]>,
        edges: Option<[usize; 2]>,
    ) -> Result<Value> {
        let range = requested.unwrap_or(manifest.beat_range);
        ensure!(
            range[0] >= manifest.beat_range[0] && range[1] <= manifest.beat_range[1],
            "TUNING_RANGE_OUTSIDE_PAIR: 拍区间须在试听范围内"
        );
        let frame_ranges = [frames(projects[0], range)?, frames(projects[1], range)?];
        ensure!(
            frame_ranges
                .iter()
                .zip(manifest.frame_ranges)
                .all(|(&[first, end], [a, b])| first >= a
                    && end <= b
                    && (end - first) <= 12 * RATE),
            "实音测量每版最多 12 秒"
        );
        let folder = self.path.canonicalize()?;
        let read = |side: usize, file: &str| {
            let origin = manifest.frame_ranges[side][0];
            read_output(
                &folder,
                &[file.to_owned()],
                frame_ranges[side].map(|frame| frame - origin),
            )
        };
        let buffers = [read(0, "A.wav")?, read(1, "B.wav")?];
        let samples = buffers.each_ref().map(Vec::as_slice);
        let control = Control::ephemeral(
            &manifest.id,
            &manifest.project_id,
            manifest.revision,
            "prepared_pcm",
        );
        let mut report =
            crate::balance::compare_drafts(projects, range, samples, frame_ranges, &control)?;
        report["level_matched"] = serde_json::json!(manifest.matched_rms || manifest.matched_lufs);
        if let Some(edges) = edges {
            let jumps = std::array::from_fn::<_, 2, _>(|side| {
                let [first, end] = frame_ranges[side];
                edges.map(|edge| {
                    (first < edge && edge < end).then(|| {
                        let before = samples[side][edge - first - 1];
                        let after = samples[side][edge - first];
                        (after[0] - before[0])
                            .abs()
                            .max((after[1] - before[1]).abs())
                    })
                })
            });
            report["transition_edges"] = serde_json::json!({
                "frames":edges,"sample_step_ab":jumps
            });
        }
        if let Some(track) = &manifest.local_voice_track_id {
            report["vocal_pitch"] = crate::pitch_reference::inspect_drafts(
                projects,
                track,
                range,
                frame_ranges.map(|span| span[0]),
                samples,
                &control,
            )?;
        }
        report["audition_id"] = serde_json::json!(manifest.id);
        report["project_id"] = serde_json::json!(manifest.project_id);
        report["revision"] = serde_json::json!(manifest.revision);
        report["source"] = serde_json::json!("prepared_audition_pcm");
        Ok(report)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        for name in ["A.wav", "B.wav", "pair.json"] {
            let _ = fs::remove_file(self.path.join(name));
        }
        let _ = fs::remove_dir(&self.path);
    }
}

pub(crate) fn frames(project: &Project, range: [f64; 2]) -> Result<[usize; 2]> {
    let [start, end] = range;
    ensure!(
        start.is_finite() && end.is_finite() && start >= 0. && end > start && end <= 50_000.,
        "指定有效的试听拍区间"
    );
    let clock = project.timeline();
    let [first, end] = range.map(|beat| (clock.seconds(beat) * RATE as f64).round() as usize);
    ensure!(
        end > first && end - first <= MAX_FRAMES && end <= 960 * RATE,
        "A/B 试听区间须在工程时间内，且每版最长 45 秒"
    );
    Ok([first, end])
}

fn rms(frames: &[[f32; 2]]) -> f32 {
    let energy = frames
        .iter()
        .flatten()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>();
    (energy / (2 * frames.len()) as f64).sqrt() as f32
}

fn match_gated(meters: &[crate::loudness::Loudness; 2]) -> Option<[f32; 2]> {
    let mut gains = [1.; 2];
    let a = meters[0].integrated(1.)?;
    let b = meters[1].integrated(1.)?;
    let louder = usize::from(b > a);
    let target = a.min(b);
    for _ in 0..9 {
        let difference = meters[louder].integrated(gains[louder])? - target;
        if difference.abs() <= 0.02 {
            return Some(gains);
        }
        // Reapply the absolute gate at the new level before accepting a match.
        gains[louder] = (gains[louder] * crate::effects::db_gain(-difference) as f32).min(1.);
    }
    None
}

pub(crate) struct AudioRequest<'a> {
    pub root: &'a Path,
    pub base: &'a Project,
    pub candidate: &'a Project,
    pub engine: &'a str,
    pub range: [f64; 2],
    pub compare: bool,
    pub output: Option<&'a SavedOutput>,
    pub local_voice_track: Option<&'a str>,
    pub jobs: &'a crate::jobs::History,
}

pub(crate) struct Request<'a> {
    pub audio: AudioRequest<'a>,
    pub id: &'a str,
    pub level_match: bool,
    pub match_method: MatchMethod,
}

struct Segment {
    first: usize,
    samples: Vec<[f32; 2]>,
}

impl Segment {
    fn window(&self, [first, end]: [usize; 2]) -> Option<&[[f32; 2]]> {
        let first = first.checked_sub(self.first)?;
        let end = end.checked_sub(self.first)?;
        self.samples.get(first..end)
    }
}

#[derive(Clone)]
pub(crate) struct TaskAudio {
    engine: String,
    segments: [Arc<Segment>; 2],
    output: Option<SavedOutput>,
    local_voice_track: Option<String>,
}

fn prepare_segment(
    request: &AudioRequest<'_>,
    cached: Option<&TaskAudio>,
    side: usize,
    range: [usize; 2],
    control: &Control,
) -> Result<(Arc<Segment>, SourceInput)> {
    control.check()?;
    if let Some(audio) = cached.filter(|audio| {
        audio.engine == request.engine
            && audio.output.as_ref() == request.output
            && audio.local_voice_track.as_deref() == request.local_voice_track
    }) {
        let segment = &audio.segments[side];
        if segment.window(range).is_some() {
            return Ok((segment.clone(), SourceInput::PreparedAudio));
        }
    }
    let project = [request.base, request.candidate][side];
    if let Some(track) = request.local_voice_track {
        let samples = engine::audition_voice_excerpt(
            request.root,
            project,
            request.engine,
            track,
            range,
            control,
        )?;
        return Ok((
            Arc::new(Segment {
                first: range[0],
                samples,
            }),
            SourceInput::LocalVoice,
        ));
    }
    if side == 0 && !request.compare {
        let saved = (|| -> Result<_> {
            let mix = request
                .jobs
                .existing_mix(&project.id, project.revision, request.engine)?
                .context("No matching mix")?;
            let job = crate::model::field(&mix["result"], "job_id")?;
            let folder = export_directory(request.root, job)?;
            let names = output_files(
                project,
                &mix["result"],
                request.output.map(std::slice::from_ref),
            )?;
            read_output(&folder, &names, range)
        })();
        control.check()?;
        if let Ok(samples) = saved {
            return Ok((
                Arc::new(Segment {
                    first: range[0],
                    samples,
                }),
                if request.output.is_some() {
                    SourceInput::SavedStem
                } else {
                    SourceInput::SavedMix
                },
            ));
        }
    }
    let samples = engine::audition_mix(
        request.root,
        project,
        request.engine,
        range[0],
        range[1],
        request.output,
        control,
    )?;
    Ok((
        Arc::new(Segment {
            first: range[0],
            samples,
        }),
        SourceInput::Rendered,
    ))
}

fn prepare_audio(
    request: &AudioRequest<'_>,
    cached: Option<&TaskAudio>,
    ranges: [[usize; 2]; 2],
    control: &Control,
    side: impl Fn(&str) -> Result<()>,
) -> Result<(TaskAudio, [SourceInput; 2])> {
    if let Some(output) = request.output {
        output_name(request.base, output)?;
        output_name(request.candidate, output)?;
    }
    side(if request.local_voice_track.is_some() {
        "合成 A · 局部干声"
    } else if request.compare {
        "准备草稿 A"
    } else {
        "准备 A · 当前版本"
    })?;
    let (a, input_a) = prepare_segment(request, cached, 0, ranges[0], control)?;
    control.check()?;
    side(if request.local_voice_track.is_some() {
        "合成 B · 局部干声"
    } else if request.compare {
        "准备草稿 B"
    } else {
        "准备 B · Agent 草稿"
    })?;
    let (b, input_b) = prepare_segment(request, cached, 1, ranges[1], control)?;
    Ok((
        TaskAudio {
            engine: request.engine.into(),
            segments: [a, b],
            output: request.output.cloned(),
            local_voice_track: request.local_voice_track.map(str::to_owned),
        },
        [input_a, input_b],
    ))
}

pub(crate) fn prepare(
    request: Request<'_>,
    cached: Option<TaskAudio>,
    control: &Control,
    side: impl Fn(&str) -> Result<()>,
) -> Result<(Files, Manifest, TaskAudio)> {
    let Request {
        audio,
        id,
        level_match,
        match_method,
    } = request;
    identifier(id)?;
    let AudioRequest {
        root,
        base,
        candidate,
        range,
        ..
    } = audio;
    let fa = frames(base, range)?;
    let fb = frames(candidate, range)?;
    let (cache, inputs) = prepare_audio(&audio, cached.as_ref(), [fa, fb], control, &side)?;
    drop(cached);
    let manifest = Manifest {
        id: id.into(),
        project_id: base.id.clone(),
        revision: base.revision,
        beat_range: range,
        frame_ranges: [fa, fb],
        timing: [Timing::from_project(base), Timing::from_project(candidate)],
        matched_rms: false,
        level_match,
        match_method,
        matched_lufs: false,
        lufs_before: [None; 2],
        lufs_after: [None; 2],
        short_rms_fallback: false,
        gain_db: [0.; 2],
        silent: [false; 2],
        source_trim_db: 0.,
        saved: None,
        recordings: None,
        source_inputs: Some(inputs),
        output: audio.output.cloned(),
        local_voice_track_id: audio.local_voice_track.map(str::to_owned),
    };
    let (files, manifest) = publish(
        root,
        manifest,
        [
            cache.segments[0].window(fa).context("Missing A samples")?,
            cache.segments[1].window(fb).context("Missing B samples")?,
        ],
        control,
        side,
    )?;
    Ok((files, manifest, cache))
}

pub(crate) fn prepare_saved(
    root: &Path,
    project: &Project,
    source: SavedSource,
    inputs: [(&Project, &Value); 2],
    level_match: bool,
    match_method: MatchMethod,
) -> Result<(Files, Manifest)> {
    let [(a_project, a_render), (b_project, b_render)] = inputs;
    let frame_ranges = [
        frames(a_project, source.beat_ranges[0])?,
        frames(b_project, source.beat_ranges[1])?,
    ];
    let id = crate::model::uid("ranges");
    let control = Control::ephemeral(&id, &project.id, project.revision, "saved_mix");
    let jobs = source.jobs()?;
    let [directory_a, directory_b] = [
        export_directory(root, jobs[0])?,
        export_directory(root, jobs[1])?,
    ];
    let [files_a, files_b] = match &source.outputs {
        Some([a, b]) => [
            output_files(a_project, a_render, a.as_deref())?,
            output_files(b_project, b_render, b.as_deref())?,
        ],
        None => [vec!["试听混音.wav".into()], vec!["试听混音.wav".into()]],
    };
    ensure!(
        jobs[0] != jobs[1] || source.beat_ranges[0] != source.beat_ranges[1] || files_a != files_b,
        "选择不同的试听范围或输出"
    );
    let mut a = read_output(&directory_a, &files_a, frame_ranges[0])?;
    let mut b = read_output(&directory_b, &files_b, frame_ranges[1])?;
    // A subset can exceed full scale if omitted outputs had cancelled part of it.
    // Preserve the balance between both sides by applying the same headroom trim.
    let mut peak = 1_f32;
    if files_a.len() > 1 || files_b.len() > 1 {
        peak = a
            .iter()
            .chain(&b)
            .flatten()
            .fold(1., |peak, sample| peak.max(sample.abs()));
        if peak > 1. {
            a.iter_mut()
                .chain(&mut b)
                .flatten()
                .for_each(|sample| *sample /= peak);
        }
    }
    let manifest = Manifest {
        id,
        project_id: project.id.clone(),
        revision: project.revision,
        beat_range: source.beat_ranges[0],
        frame_ranges,
        timing: [
            Timing::from_project(a_project),
            Timing::from_project(b_project),
        ],
        matched_rms: false,
        level_match,
        match_method,
        matched_lufs: false,
        lufs_before: [None; 2],
        lufs_after: [None; 2],
        short_rms_fallback: false,
        gain_db: [0.; 2],
        silent: [false; 2],
        source_trim_db: -20. * peak.log10(),
        saved: Some(source),
        recordings: None,
        source_inputs: None,
        output: None,
        local_voice_track_id: None,
    };
    publish(root, manifest, [&a, &b], &control, |_| Ok(()))
}

pub(crate) fn prepare_recordings(
    root: &Path,
    project: &Project,
    recordings: [crate::clips::ClipRef; 2],
    range: [f64; 2],
    level_match: bool,
    match_method: MatchMethod,
) -> Result<(Files, Manifest)> {
    crate::clips::validate_refs(project, &recordings)?;
    let frame_range = frames(project, range)?;
    let clock = project.timeline();
    let id = crate::model::uid("ranges");
    let control = Control::ephemeral(&id, &project.id, project.revision, "recording_comparison");
    let read = |reference: &crate::clips::ClipRef| -> Result<Vec<[f32; 2]>> {
        let (_, mut clip) = reference.resolve(project)?;
        let [first, end] = clip.recording_window(&clock, range)?;
        let envelope = clip.rendered_envelope(&clock);
        let gain = crate::effects::db_gain(f64::from(clip.gain_db)) as f32;
        clip.offset += first;
        clip.frames = end - first;
        let mut samples = Vec::with_capacity(clip.frames as usize);
        crate::audio::visit_clip(root, &clip, |frame, sample| {
            samples.push(sample.map(|value| value * gain * envelope.gain(first + frame)));
            Ok(())
        })?;
        Ok(samples)
    };
    let [mut a, mut b] = [read(&recordings[0])?, read(&recordings[1])?];
    let peak = a
        .iter()
        .chain(&b)
        .flatten()
        .fold(1_f32, |peak, sample| peak.max(sample.abs()));
    if peak > 1. {
        a.iter_mut()
            .chain(&mut b)
            .flatten()
            .for_each(|sample| *sample /= peak);
    }
    let timing = Timing::from_project(project);
    let manifest = Manifest {
        id,
        project_id: project.id.clone(),
        revision: project.revision,
        beat_range: range,
        frame_ranges: [frame_range; 2],
        timing: [timing.clone(), timing],
        matched_rms: false,
        level_match,
        match_method,
        matched_lufs: false,
        lufs_before: [None; 2],
        lufs_after: [None; 2],
        short_rms_fallback: false,
        gain_db: [0.; 2],
        silent: [false; 2],
        source_trim_db: -20. * peak.log10(),
        saved: None,
        recordings: Some(recordings),
        source_inputs: None,
        output: None,
        local_voice_track_id: None,
    };
    publish(root, manifest, [&a, &b], &control, |_| Ok(()))
}

pub(crate) fn output_files(
    project: &Project,
    render: &Value,
    selected: Option<&[SavedOutput]>,
) -> Result<Vec<String>> {
    let Some(selected) = selected else {
        return Ok(vec!["试听混音.wav".into()]);
    };
    let output_limit = project.tracks.len() + project.mix_groups.len() + project.aux_buses.len();
    ensure!(
        !selected.is_empty() && selected.len() <= output_limit,
        "选择本工程已有的分轨输出"
    );
    let unique: HashSet<_> = selected.iter().collect();
    ensure!(unique.len() == selected.len(), "分轨输出不能重复");
    let names = render["stems"].as_array().context("本次渲染没有分轨清单")?;
    let bus_file = |key: &str, id: &str| -> Option<String> {
        let bus = render[key]
            .as_array()?
            .iter()
            .find(|bus| bus["bus_id"] == id)?;
        Some(bus["stem"].as_str()?.to_owned())
    };
    let mut files = Vec::with_capacity(selected.len());
    for output in selected {
        let file = match output {
            SavedOutput::Track(id) => {
                identifier(id)?;
                project
                    .tracks
                    .iter()
                    .enumerate()
                    .find(|(_, track)| &track.id == id && track.output_group.is_none())
                    .map(|(index, track)| engine::track_stem_filename(index, &track.name))
            }
            SavedOutput::Group(id) => {
                identifier(id)?;
                bus_file("group_stems", id)
            }
            SavedOutput::Return(id) => {
                identifier(id)?;
                bus_file("aux_returns", id)
            }
        };
        let file = file
            .filter(|file| names.iter().any(|value| value == file))
            .with_context(|| format!("STEM_OUTPUT_UNAVAILABLE: 本次渲染没有分轨 {output:?}"))?;
        files.push(file);
    }
    files.sort_unstable();
    let count = files.len();
    files.dedup();
    ensure!(count == files.len(), "分轨输出指向同一文件");
    Ok(files)
}

pub(crate) fn read_output(
    directory: &Path,
    names: &[String],
    [first, end]: [usize; 2],
) -> Result<Vec<[f32; 2]>> {
    let mut frames = vec![[0.; 2]; end - first];
    for name in names {
        let path = directory.join(name).canonicalize()?;
        ensure!(path.parent() == Some(directory), "试听文件路径无效");
        let mut wav = hound::WavReader::open(&path).context("无法读取已渲染的输出")?;
        let spec = wav.spec();
        ensure!(
            spec.sample_rate as usize == RATE
                && spec.channels == 2
                && spec.bits_per_sample == 24
                && spec.sample_format == hound::SampleFormat::Int,
            "试听需要 48 kHz 24 位立体声 WAV"
        );
        ensure!(end <= wav.duration() as usize, "试听范围超出已渲染输出");
        wav.seek(first as u32)?;
        let mut samples = wav.samples::<i32>();
        for channel in frames.iter_mut().flatten() {
            *channel += samples.next().context("输出文件提前结束")?? as f32 / 8_388_607.;
        }
    }
    Ok(frames)
}

fn publish(
    root: &Path,
    mut manifest: Manifest,
    [a, b]: [&[[f32; 2]]; 2],
    control: &Control,
    side: impl Fn(&str) -> Result<()>,
) -> Result<(Files, Manifest)> {
    control.check()?;
    let levels = [rms(a), rms(b)];
    let silent = levels.map(|n| n <= 1e-6);
    let short = a.len().min(b.len()) < 2 * RATE / 5;
    let can_match = manifest.level_match && !silent[0] && !silent[1];
    let match_method = manifest.match_method;
    let short_rms_fallback = can_match && short && match_method == MatchMethod::Lufs;
    let matched_rms = can_match && (match_method == MatchMethod::Rms || short_rms_fallback);
    let mut lufs_before = [None; 2];
    let mut lufs_after = [None; 2];
    let mut matched_lufs = false;
    let gains = if matched_rms {
        let lower = levels[0].min(levels[1]);
        levels.map(|level| lower / level)
    } else if can_match && match_method == MatchMethod::Lufs {
        side("匹配两版试听响度")?;
        let meters = [
            crate::loudness::Loudness::scan(a, || control.check())?,
            crate::loudness::Loudness::scan(b, || control.check())?,
        ];
        lufs_before = meters.each_ref().map(|meter| meter.integrated(1.));
        let adjustment = match_gated(&meters);
        matched_lufs = adjustment.is_some();
        let gains = adjustment.unwrap_or([1.; 2]);
        lufs_after = std::array::from_fn(|index| meters[index].integrated(gains[index]));
        gains
    } else {
        [1.; 2]
    };
    manifest.matched_rms = matched_rms;
    manifest.matched_lufs = matched_lufs;
    manifest.lufs_before = lufs_before;
    manifest.lufs_after = lufs_after;
    manifest.short_rms_fallback = short_rms_fallback;
    manifest.gain_db = gains.map(|gain| 20. * gain.log10());
    manifest.silent = silent;
    side("写入 A/B 试听片段")?;
    let path = folder(root, &manifest.id);
    fs::create_dir_all(path.parent().unwrap())?;
    fs::create_dir(&path)?;
    let files = Files { path };
    engine::write_wav(&files.path.join("A.wav"), a, gains[0], control)?;
    engine::write_wav(&files.path.join("B.wav"), b, gains[1], control)?;
    control.commit()?;
    fs::write(files.path.join("pair.json"), serde_json::to_vec(&manifest)?)?;
    Ok((files, manifest))
}

#[cfg(windows)]
pub(crate) fn read(root: &Path, id: &str) -> Result<(Manifest, [PathBuf; 2])> {
    identifier(id)?;
    let path = folder(root, id).canonicalize()?;
    let parent = root.join("runtime/agent_auditions").canonicalize()?;
    ensure!(path.parent() == Some(parent.as_path()), "试听路径无效");
    let manifest: Manifest = serde_json::from_slice(&fs::read(path.join("pair.json"))?)?;
    ensure!(
        manifest.id == id
            && manifest
                .ranges()
                .iter()
                .all(|[start, end]| *start >= 0. && end > start && *end <= 50_000.)
            && manifest
                .frame_ranges
                .iter()
                .all(|[first, end]| end > first && end - first <= MAX_FRAMES && *end <= 960 * RATE),
        "试听范围已失效"
    );
    if let Some(source) = &manifest.saved {
        for job in source.jobs()? {
            identifier(job)?;
        }
    }
    for timing in &manifest.timing {
        crate::timeline::validate(timing.bpm, [4, 4], &timing.tempos, &[])?;
    }
    let files = [
        path.join("A.wav").canonicalize()?,
        path.join("B.wav").canonicalize()?,
    ];
    ensure!(
        files
            .iter()
            .all(|file| file.parent() == Some(path.as_path())),
        "试听文件路径无效"
    );
    Ok((manifest, files))
}
