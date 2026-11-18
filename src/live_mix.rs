//! The output callback processes prepared chains. File I/O, allocation and
//! destruction of retired chains belong to the transport worker.
use crate::{
    automation::Envelope,
    effects::{db_gain, peak, smooth_ms, Chain, Effect, Limiter, CHAIN_LIMIT},
    model::{Project, Track, TRACK_LIMIT},
    monitor_stream::{Pages, Stream, STREAM_AFTER},
    monitor_voice::{Input, Plan, TOTAL_PATCH_FRAMES},
    resample::playback::Window,
    routing::{self, Bus, Levels, Send},
    source_cache::{
        active, fader, fader_values, frame_count, has_material, source_fingerprint, Source,
    },
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashSet, path::Path, sync::Arc};

const MAX_FX_BYTES: usize = 96 * 1024 * 1024;
const RESIDENT_TRACK_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct Session {
    pub(crate) engine: String,
    project: Project,
    material: Vec<Material>,
    local_frames: usize,
    current: Settings,
    rate: u64,
}

struct Material {
    input: Input,
    fingerprint: String,
}

pub(crate) enum Advance {
    Incompatible,
    Settings(Settings),
    Preparing(Box<Preparation>),
}

pub(crate) struct Preparation {
    pub(crate) project: Project,
    pub(crate) control: Arc<crate::jobs::Control>,
    engine: String,
    sources: Vec<Input>,
    fingerprints: Vec<String>,
    changed: Vec<usize>,
    vocal_plans: Vec<(usize, Plan)>,
    settings: Settings,
    origin: (usize, Option<(usize, usize)>),
}

pub(crate) struct Prepared {
    project: Project,
    material: Vec<Material>,
    settings: Settings,
    local_frames: usize,
    pub(crate) replacement: Replacement,
}

pub(crate) struct Replacement {
    inputs: DryInputs,
    changes: Vec<(usize, Option<[usize; 2]>)>,
}

impl Preparation {
    pub(crate) fn run(self, root: &Path) -> Result<Prepared> {
        let mut sources = self.sources;
        let mut changes = Vec::new();
        let mut used: usize = sources.iter().map(Input::patch_frames).sum();
        let mut local_frames = 0;
        for (index, plan) in &self.vocal_plans {
            let input = &sources[*index];
            let track = self
                .project
                .tracks
                .iter()
                .find(|track| track.id == input.source.track_id)
                .context("监听音轨不存在")?;
            let others = used - input.patch_frames();
            let attempt = input.update_voice(
                root,
                &self.project,
                track,
                plan,
                TOTAL_PATCH_FRAMES.saturating_sub(others),
                &self.control,
            );
            self.control.check()?;
            if let Ok(Some(updated)) = attempt {
                used = others + updated.patch_frames();
                sources[*index] = updated;
                changes.push((*index, Some(plan.range)));
                local_frames += plan.range[1] - plan.range[0];
            }
        }
        let remaining = self
            .changed
            .iter()
            .copied()
            .filter(|index| !changes.iter().any(|(done, _)| index == done))
            .collect::<Vec<_>>();
        let ids = remaining
            .iter()
            .map(|i| sources[*i].source.track_id.as_str())
            .collect::<HashSet<_>>();
        let replacements = if ids.is_empty() {
            vec![]
        } else {
            crate::engine::prepare_monitor_sources(
                root,
                &self.project,
                &self.engine,
                &ids,
                &self.control,
            )?
        };
        let mut replacements = replacements
            .into_iter()
            .map(|source| (source.track_id.clone(), source))
            .collect::<std::collections::HashMap<_, _>>();
        for index in remaining {
            sources[index] = replacements
                .remove(&sources[index].source.track_id)
                .context("监听音轨缺失")?
                .into();
            changes.push((index, None));
        }
        self.control.check()?;
        let inputs = DryInputs::load(root, &self.project, &sources, self.origin)?;
        self.control.check()?;
        Ok(Prepared {
            project: self.project,
            material: sources
                .into_iter()
                .zip(self.fingerprints)
                .map(|(input, fingerprint)| Material { input, fingerprint })
                .collect(),
            settings: self.settings,
            local_frames,
            replacement: Replacement { inputs, changes },
        })
    }
}

#[derive(Clone, PartialEq)]
struct TrackSettings {
    fader: [f64; 2],
    effects: Vec<Effect>,
    automation: Envelope,
    routes: Levels,
}

#[derive(Clone, PartialEq)]
pub(crate) struct Settings {
    tracks: Vec<TrackSettings>,
    limiter: Option<Limiter>,
    buses: Vec<Bus>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackDraft {
    track_id: String,
    effects: Option<Vec<Effect>>,
    gain_db: Option<f64>,
    pan: Option<f64>,
    sends: Option<Vec<Send>>,
    #[serde(default, deserialize_with = "draft_output")]
    output_group: Option<Option<String>>,
}

fn draft_output<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(value).map(Some)
}
impl Settings {
    fn saved(project: &Project, tracks: &[&Track]) -> Self {
        let solo = project.tracks.iter().any(|t| t.solo && !t.mute);
        let clock = project.timeline();
        Self {
            tracks: tracks
                .iter()
                .map(|t| TrackSettings {
                    fader: if active(t, solo) { fader(t) } else { [0.; 2] },
                    effects: t.effects.clone(),
                    automation: Envelope::compile(&t.gain_automation, &clock),
                    routes: routing::levels(
                        t.output_group.as_deref(),
                        &t.sends,
                        &project.mix_groups,
                        &project.aux_buses,
                    ),
                })
                .collect(),
            limiter: project.master_limiter.clone(),
            buses: routing::buses(project).cloned().collect(),
        }
    }

    fn validate_budget(&self, rate: u64) -> Result<()> {
        ensure!(
            !self.limiter.as_ref().is_some_and(Limiter::is_true_peak),
            "真峰值模式使用渲染后的 WAV 试听"
        );
        ensure!(
            (8_000..=96_000).contains(&rate),
            "实时监听支持 8–96 kHz 的输出设备"
        );
        let bytes: usize = self
            .tracks
            .iter()
            .flat_map(|t| &t.effects)
            .chain(self.buses.iter().flat_map(|bus| &bus.effects))
            .filter(|effect| !effect.bypassed())
            .map(|e| match e {
                Effect::Delay { time_ms, .. } => {
                    (time_ms * rate as f64 / 1000.).ceil() as usize * 16
                }
                Effect::Reverb { .. } => rate as usize * 3,
                _ => 0,
            })
            .sum();
        ensure!(bytes <= MAX_FX_BYTES, "当前效果链超过实时监听缓冲容量");
        Ok(())
    }

    fn missing_sidechain(&self, project: &Project, loaded: &HashSet<&str>) -> bool {
        self.tracks
            .iter()
            .flat_map(|track| &track.effects)
            .chain(self.buses.iter().flat_map(|bus| &bus.effects))
            .filter_map(Effect::active_sidechain)
            .any(|id| {
                !loaded.contains(id)
                    && project
                        .tracks
                        .iter()
                        .any(|track| track.id == id && has_material(track))
            })
    }
}

impl Session {
    pub(crate) fn revision(&self) -> u64 {
        self.project.revision
    }

    pub(crate) fn local_frames(&self) -> usize {
        self.local_frames
    }

    pub(crate) fn track_ids(&self) -> Vec<&str> {
        self.material
            .iter()
            .map(|item| item.input.source.track_id.as_str())
            .chain(routing::buses(&self.project).map(|bus| bus.id.as_str()))
            .collect()
    }

    pub(crate) fn load(
        root: &Path,
        project: Project,
        render: &Value,
        rate: u64,
        origin: (usize, Option<(usize, usize)>),
    ) -> Result<(Self, Mixer)> {
        ensure!(
            render["project_id"] == project.id && render["revision"] == project.revision,
            "试听需要使用当前工程版本的混音"
        );
        let engine = crate::model::field(render, "engine")?.to_owned();
        let len = frame_count(&project);
        let sources: Vec<Source> = serde_json::from_value(render["preview_sources"].clone())
            .context("此混音没有实时监听音轨，请重新渲染")?;
        let ids: HashSet<_> = sources.iter().map(|s| s.track_id.as_str()).collect();
        let solo = project.tracks.iter().any(|t| t.solo && !t.mute);
        ensure!(
            !sources.is_empty()
                && ids.len() == sources.len()
                && project.tracks.iter().all(|t| !active(t, solo)
                    || !routing::group_open(t.output_group.as_deref(), &project.mix_groups)
                    || ids.contains(t.id.as_str())),
            "此混音没有完整的实时监听音轨"
        );
        let ordered = sources
            .iter()
            .map(|s| {
                project
                    .tracks
                    .iter()
                    .find(|t| t.id == s.track_id)
                    .context("实时监听音轨不存在")
            })
            .collect::<Result<Vec<_>>>()?;
        let material: Vec<_> = ordered
            .iter()
            .zip(&sources)
            .map(|(t, source)| {
                Ok(Material {
                    input: source.clone().into(),
                    fingerprint: source_fingerprint(&project, t, len)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let settings = Settings::saved(&project, &ordered);
        settings.validate_budget(rate)?;
        ensure!(
            !settings.missing_sidechain(&project, &ids),
            "缺少旁链触发源，请重新渲染试听"
        );
        let key = |id: &str| sources.iter().position(|source| source.track_id == id);
        let inputs = DryInputs::load(
            root,
            &project,
            &material
                .iter()
                .map(|item| item.input.clone())
                .collect::<Vec<_>>(),
            origin,
        )?;
        let tracks = settings
            .tracks
            .iter()
            .map(|setting| MixTrack {
                output: Insert::new(Chain::new(&setting.effects, rate as f64, key)),
                fader: setting.fader,
                target_fader: setting.fader,
                slope: [0.; 2],
                ramp: 0,
                meter: Meter::default(),
                routes: Fade::new(setting.routes),
                automation: TrackCurve::new(setting.automation.clone()),
                previous_automation: None,
                automation_fade: 0,
                previous_input: [0.; 2],
                input_fade: 0,
                last_input: [0.; 2],
            })
            .collect();
        let mixer = Mixer {
            tracks,
            inputs,
            len,
            buses: settings
                .buses
                .iter()
                .map(|bus| MixBus {
                    output: Insert::new(Chain::new(&bus.effects, rate as f64, key)),
                    gain: Fade::new([bus.gain()]),
                    meter: Meter::default(),
                })
                .collect(),
            limiter: PeakControl::new(settings.limiter.as_ref(), rate),
            fade_frames: (rate / 125) as u32,
            rate,
        };
        Ok((
            Self {
                engine,
                project,
                material,
                local_frames: 0,
                current: settings,
                rate,
            },
            mixer,
        ))
    }

    pub(crate) fn draft(&self, value: &Value) -> Result<Settings> {
        let tracks: Vec<TrackDraft> = serde_json::from_value(value["tracks"].clone())?;
        ensure!(
            tracks.len() <= self.project.tracks.len(),
            "监听音轨数量不合法"
        );
        let ordered = self
            .material
            .iter()
            .map(|item| {
                self.project
                    .tracks
                    .iter()
                    .find(|t| t.id == item.input.source.track_id)
                    .context("实时监听音轨不存在")
            })
            .collect::<Result<Vec<_>>>()?;
        let solo = self.project.tracks.iter().any(|t| t.solo && !t.mute);
        let mut settings = Settings::saved(&self.project, &ordered);
        let group_draft: Option<Vec<Bus>> = value
            .get("mix_groups")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        let aux_draft: Option<Vec<Bus>> = value
            .get("aux_buses")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        let groups = group_draft.as_deref().unwrap_or(&self.project.mix_groups);
        let returns = aux_draft.as_deref().unwrap_or(&self.project.aux_buses);
        if group_draft.is_some() || aux_draft.is_some() {
            routing::validate_buses(groups, returns)?;
            for bus in groups.iter().chain(returns) {
                crate::effects::validate_sidechains(&bus.effects, &self.project.tracks, None)?;
            }
            ensure!(
                routing::same_buses(&self.project.mix_groups, groups)
                    && routing::same_buses(&self.project.aux_buses, returns),
                "试听草稿须保留当前编组与返回通道"
            );
            settings.buses = groups.iter().chain(returns).cloned().collect();
        }
        for (track, setting) in ordered.iter().zip(&mut settings.tracks) {
            setting.routes =
                routing::levels(track.output_group.as_deref(), &track.sends, groups, returns);
        }
        for track in &self.project.tracks {
            let output = tracks
                .iter()
                .find(|d| d.track_id == track.id)
                .and_then(|d| d.output_group.as_ref())
                .map_or(track.output_group.as_deref(), |group| group.as_deref());
            ensure!(
                !active(track, solo)
                    || !routing::group_open(output, groups)
                    || self
                        .material
                        .iter()
                        .any(|item| item.input.source.track_id == track.id),
                "当前试听缺少编组成员的监听源"
            );
        }
        for draft in tracks {
            if let Some(effects) = &draft.effects {
                ensure!(effects.len() <= CHAIN_LIMIT, "音轨插入数量超出限制");
                for effect in effects {
                    effect.validate()?;
                }
                crate::effects::validate_sidechains(
                    effects,
                    &self.project.tracks,
                    Some(&draft.track_id),
                )?;
            }
            if let Some(sends) = &draft.sends {
                routing::validate_sends(sends, returns)?;
            }
            if let Some(Some(id)) = &draft.output_group {
                ensure!(
                    groups.iter().any(|group| &group.id == id),
                    "监听输出编组不存在"
                );
            }
            ensure!(
                self.project.tracks.iter().any(|t| t.id == draft.track_id),
                "监听音轨不存在"
            );
            if let Some(index) = self
                .material
                .iter()
                .position(|item| item.input.source.track_id == draft.track_id)
            {
                let track = ordered[index];
                let gain = draft.gain_db.unwrap_or(track.gain_db);
                let pan = draft.pan.unwrap_or(track.pan);
                ensure!(
                    (-60.0..=12.).contains(&gain) && (-1.0..=1.).contains(&pan),
                    "电平或声像不在可用范围"
                );
                if active(track, solo) {
                    settings.tracks[index].fader = fader_values(track, gain, pan);
                }
                if let Some(effects) = draft.effects {
                    settings.tracks[index].effects = effects;
                }
                let output = draft
                    .output_group
                    .as_ref()
                    .map_or(track.output_group.as_deref(), |group| group.as_deref());
                let sends = draft.sends.as_deref().unwrap_or(&track.sends);
                settings.tracks[index].routes = routing::levels(output, sends, groups, returns);
            }
        }
        if let Some(value) = value.get("master_limiter") {
            settings.limiter = serde_json::from_value(value.clone())?;
            if let Some(limiter) = &settings.limiter {
                limiter.validate()?;
            }
        }
        let loaded = self
            .material
            .iter()
            .map(|item| item.input.source.track_id.as_str())
            .collect();
        ensure!(
            !settings.missing_sidechain(&self.project, &loaded),
            "缺少旁链触发源，请应用并重新试听"
        );
        Ok(settings)
    }

    pub(crate) fn advance(
        &mut self,
        project: Project,
        origin: (usize, Option<(usize, usize)>),
    ) -> Result<Advance> {
        let len = frame_count(&project);
        if project.id != self.project.id
            || project.bpm != self.project.bpm
            || project.tempo_map != self.project.tempo_map
            || len != frame_count(&self.project)
            || project
                .master_limiter
                .as_ref()
                .is_some_and(Limiter::is_true_peak)
            || !routing::same_buses(&self.project.aux_buses, &project.aux_buses)
            || !routing::same_buses(&self.project.mix_groups, &project.mix_groups)
        {
            return Ok(Advance::Incompatible);
        }
        let ids: HashSet<_> = self
            .material
            .iter()
            .map(|item| item.input.source.track_id.as_str())
            .collect();
        let solo = project.tracks.iter().any(|t| t.solo && !t.mute);
        if project.tracks.iter().any(|t| {
            active(t, solo)
                && routing::group_open(t.output_group.as_deref(), &project.mix_groups)
                && !ids.contains(t.id.as_str())
        }) {
            return Ok(Advance::Incompatible);
        }
        let mut tracks = Vec::with_capacity(ids.len());
        let mut changed = Vec::new();
        let mut vocal_plans = Vec::new();
        let mut fingerprints = Vec::with_capacity(ids.len());
        for (index, item) in self.material.iter().enumerate() {
            let Some(track) = project
                .tracks
                .iter()
                .find(|t| t.id == item.input.source.track_id)
            else {
                return Ok(Advance::Incompatible);
            };
            let fingerprint = source_fingerprint(&project, track, len)?;
            if fingerprint != item.fingerprint {
                if !has_material(track) {
                    return Ok(Advance::Incompatible);
                }
                changed.push(index);
                if let Some(plan) = (item.input.source.kind == "voicevox")
                    .then(|| {
                        self.project
                            .tracks
                            .iter()
                            .find(|previous| previous.id == track.id)
                    })
                    .flatten()
                    .and_then(|previous| Plan::new(&project, previous, track, len))
                {
                    vocal_plans.push((index, plan));
                }
            }
            fingerprints.push(fingerprint);
            tracks.push(track);
        }
        let settings = Settings::saved(&project, &tracks);
        if settings.missing_sidechain(&project, &ids) {
            return Ok(Advance::Incompatible);
        }
        settings.validate_budget(self.rate)?;
        if !changed.is_empty() {
            let control = Arc::new(crate::jobs::Control::ephemeral(
                &crate::model::uid("monitor"),
                &project.id,
                project.revision,
                &self.engine,
            ));
            return Ok(Advance::Preparing(Box::new(Preparation {
                project,
                control,
                engine: self.engine.clone(),
                settings,
                origin,
                changed,
                vocal_plans,
                fingerprints,
                sources: self
                    .material
                    .iter()
                    .map(|item| item.input.clone())
                    .collect(),
            })));
        }
        self.project = project;
        self.local_frames = 0;
        Ok(Advance::Settings(settings))
    }

    pub(crate) fn install(
        &mut self,
        prepared: Prepared,
        rate: u64,
    ) -> Result<(Replacement, Option<Update>)> {
        let update = self.update(prepared.settings, rate)?;
        self.project = prepared.project;
        self.material = prepared.material;
        self.local_frames = prepared.local_frames;
        Ok((prepared.replacement, update))
    }

    pub(crate) fn update(&mut self, settings: Settings, rate: u64) -> Result<Option<Update>> {
        settings.validate_budget(rate)?;
        if self.current == settings && self.rate == rate {
            return Ok(None);
        }
        let key = |id: &str| {
            self.material
                .iter()
                .position(|item| item.input.source.track_id == id)
        };
        let tracks = self
            .current
            .tracks
            .iter()
            .zip(&settings.tracks)
            .map(|(a, b)| TrackUpdate {
                chain: (a.effects != b.effects || self.rate != rate)
                    .then(|| Chain::new(&b.effects, rate as f64, key)),
                fader: b.fader,
                automation: (a.automation != b.automation).then(|| b.automation.clone()),
                routes: b.routes,
            })
            .collect();
        let buses = self
            .current
            .buses
            .iter()
            .zip(&settings.buses)
            .map(|(a, b)| BusUpdate {
                chain: (a.effects != b.effects || self.rate != rate)
                    .then(|| Chain::new(&b.effects, rate as f64, key)),
                gain: b.gain(),
            })
            .collect();
        let limiter = (self.current.limiter != settings.limiter || self.rate != rate)
            .then(|| PeakControl::new(settings.limiter.as_ref(), rate));
        self.current = settings;
        self.rate = rate;
        Ok(Some(Update {
            tracks,
            buses,
            limiter,
            rate,
        }))
    }
}

struct PeakControl {
    input: f32,
    ceiling: f32,
    release: f32,
    gain: f32,
}
impl PeakControl {
    fn new(limiter: Option<&Limiter>, rate: u64) -> Self {
        let (input, ceiling, release) = limiter.map_or((1., 0.95, 80.), |l| {
            (db_gain(l.input_db), db_gain(l.ceiling_db), l.release_ms)
        });
        Self {
            input: input as f32,
            ceiling: ceiling as f32,
            release: smooth_ms(release, rate as f64) as f32,
            gain: 1.,
        }
    }
    fn apply(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let limit = self.ceiling / (peak(frame) * self.input).max(self.ceiling);
        self.gain = limit.min(1. - (1. - self.gain) * self.release);
        frame.map(|sample| sample * self.gain * self.input)
    }
}

struct MixTrack {
    output: Insert,
    fader: [f64; 2],
    target_fader: [f64; 2],
    slope: [f64; 2],
    ramp: u32,
    meter: Meter,
    routes: Fade<{ routing::BUS_LIMIT + 1 }>,
    automation: TrackCurve,
    previous_automation: Option<TrackCurve>,
    automation_fade: u32,
    previous_input: [f32; 2],
    input_fade: u32,
    last_input: [f32; 2],
}
struct TrackCurve {
    envelope: Envelope,
    upper: usize,
}
impl TrackCurve {
    fn new(envelope: Envelope) -> Self {
        Self { envelope, upper: 0 }
    }
    fn at(&mut self, frame: f64) -> f64 {
        self.envelope.at(frame, &mut self.upper)
    }
}
struct TrackUpdate {
    chain: Option<Chain>,
    fader: [f64; 2],
    automation: Option<Envelope>,
    routes: Levels,
}
struct BusUpdate {
    chain: Option<Chain>,
    gain: f64,
}
struct MixBus {
    output: Insert,
    gain: Fade<1>,
    meter: Meter,
}
pub(crate) struct Update {
    tracks: Vec<TrackUpdate>,
    buses: Vec<BusUpdate>,
    limiter: Option<PeakControl>,
    rate: u64,
}
pub(crate) struct Mixer {
    tracks: Vec<MixTrack>,
    inputs: DryInputs,
    len: usize,
    buses: Vec<MixBus>,
    limiter: PeakControl,
    fade_frames: u32,
    rate: u64,
}

pub(crate) enum DryInputs {
    Memory(Vec<Vec<[f32; 2]>>),
    Paged(Stream),
}
impl DryInputs {
    fn load(
        root: &Path,
        project: &Project,
        sources: &[Input],
        origin: (usize, Option<(usize, usize)>),
    ) -> Result<Self> {
        let len = frame_count(project);
        let count = sources.len();
        if len <= STREAM_AFTER
            && len.saturating_mul(count).saturating_mul(8) <= RESIDENT_TRACK_BYTES
        {
            return Ok(Self::Memory(
                sources
                    .iter()
                    .map(|source| {
                        let mut samples = Vec::new();
                        source.open(root, &project.id, len)?.read_into(
                            0,
                            len,
                            &mut samples,
                            || Ok(()),
                        )?;
                        Ok(samples)
                    })
                    .collect::<Result<Vec<_>>>()
                    .context("实时监听音轨缓存已更新，请重新渲染")?,
            ));
        }
        let mut readers = sources
            .iter()
            .map(|source| source.open(root, &project.id, len))
            .collect::<Result<Vec<_>>>()
            .context("实时监听音轨缓存已更新，请重新渲染")?;
        let stream = Stream::from_reader(len, count, origin.0, origin.1, move |start, frames| {
            let mut samples = Vec::new();
            samples.try_reserve_exact(frames * count)?;
            for reader in &mut readers {
                reader.read_into(start, frames, &mut samples, || Ok(()))?;
            }
            Ok(samples)
        })
        .context("无法准备实时监听缓存，请重新渲染")?;
        Ok(Self::Paged(stream))
    }
}
pub(crate) struct Retired {
    chains: Vec<Chain>,
    curves: Vec<TrackCurve>,
}
impl Retired {
    pub(crate) fn new() -> Self {
        Self {
            chains: Vec::with_capacity(2 * (32 + routing::BUS_LIMIT)),
            curves: Vec::with_capacity(32),
        }
    }
}

fn blend(old: [f32; 2], sample: [f32; 2], weight: f32) -> [f32; 2] {
    [
        old[0] * weight + sample[0] * (1. - weight),
        old[1] * weight + sample[1] * (1. - weight),
    ]
}

struct Fade<const N: usize> {
    current: [f64; N],
    target: [f64; N],
    slope: [f64; N],
    remaining: u32,
}

impl<const N: usize> Fade<N> {
    fn new(value: [f64; N]) -> Self {
        Self {
            current: value,
            target: value,
            slope: [0.; N],
            remaining: 0,
        }
    }

    fn update(&mut self, target: [f64; N], frames: u32) {
        if self.target != target {
            self.target = target;
            self.remaining = frames;
            self.slope = std::array::from_fn(|i| (target[i] - self.current[i]) / f64::from(frames));
        }
    }

    fn reset(&mut self) {
        self.current = self.target;
        self.remaining = 0;
    }

    fn step(&mut self) -> [f64; N] {
        if self.remaining > 1 {
            for (gain, slope) in self.current.iter_mut().zip(self.slope) {
                *gain += slope;
            }
            self.remaining -= 1;
        } else if self.remaining == 1 {
            self.reset();
        }
        self.current
    }
}

struct Insert {
    chain: Chain,
    retiring: Option<Chain>,
    transition: u32,
}

impl Insert {
    fn new(chain: Chain) -> Self {
        Self {
            chain,
            retiring: None,
            transition: 0,
        }
    }

    fn reset(&mut self) {
        self.chain.reset();
        self.transition = 0;
    }

    fn update(&mut self, processor: Option<Chain>, fade: u32, retired: &mut Retired) {
        if let Some(processor) = processor {
            if let Some(old) = self.retiring.take() {
                retired.chains.push(old);
            }
            let old = std::mem::replace(&mut self.chain, processor);
            self.transition = fade;
            if fade > 0 {
                self.retiring = Some(old);
            } else {
                retired.chains.push(old);
            }
        }
    }

    fn process(&mut self, input: [f32; 2], fade_frames: u32, peaks: &[f32]) -> [f32; 2] {
        let mut output = input;
        self.chain.process(&mut output, peaks);
        if self.transition > 0 {
            if let Some(old) = &mut self.retiring {
                let mut prior = input;
                old.process(&mut prior, peaks);
                output = blend(prior, output, self.transition as f32 / fade_frames as f32);
            }
            self.transition -= 1;
        }
        output
    }
}

#[derive(Default)]
struct Meter {
    peak: [f32; 2],
    block_peak: [f32; 2],
}

impl Meter {
    fn observe(&mut self, frame: [f32; 2]) {
        for (held, sample) in self.block_peak.iter_mut().zip(frame) {
            *held = held.max(sample.abs());
        }
    }

    fn finish_block(&mut self, release: f32) {
        for (held, sample) in self.peak.iter_mut().zip(self.block_peak) {
            *held = (*held * release).max(sample);
        }
        self.block_peak = [0.; 2];
    }
}

impl Mixer {
    pub(crate) fn replace_sources(
        &mut self,
        replacement: Replacement,
        position: usize,
    ) -> DryInputs {
        for (index, bounds) in replacement.changes {
            if bounds.is_some_and(|[first, end]| !(first..end).contains(&position)) {
                continue;
            }
            let track = &mut self.tracks[index];
            track.previous_input = track.last_input;
            track.input_fade = self.fade_frames;
        }
        std::mem::replace(&mut self.inputs, replacement.inputs)
    }

    pub(crate) fn rate(&self) -> u64 {
        self.rate
    }

    pub(crate) fn frame_count(&self) -> usize {
        self.len
    }

    pub(crate) fn stream(&self) -> Option<&Stream> {
        match &self.inputs {
            DryInputs::Memory(_) => None,
            DryInputs::Paged(stream) => Some(stream),
        }
    }

    pub(crate) fn buffered_frames(&self) -> usize {
        self.stream()
            .map_or(self.len * self.tracks.len(), Stream::buffered_frames)
    }

    pub(crate) fn update(&mut self, update: Update, retired: &mut Retired) {
        let same_rate = self.rate == update.rate;
        self.rate = update.rate;
        self.fade_frames = (self.rate / 125) as u32;
        let insert_fade = if same_rate { self.fade_frames } else { 0 };
        for (track, parameters) in self.tracks.iter_mut().zip(update.tracks) {
            if track.target_fader != parameters.fader {
                track.target_fader = parameters.fader;
                track.ramp = self.fade_frames;
                track.slope = std::array::from_fn(|side| {
                    (track.target_fader[side] - track.fader[side]) / f64::from(track.ramp)
                });
            }
            track.output.update(parameters.chain, insert_fade, retired);
            track.routes.update(parameters.routes, self.fade_frames);
            if let Some(curve) = parameters.automation {
                if let Some(older) = track.previous_automation.take() {
                    retired.curves.push(older);
                }
                let previous = std::mem::replace(&mut track.automation, TrackCurve::new(curve));
                track.previous_automation = Some(previous);
                track.automation_fade = self.fade_frames;
            }
        }
        for (bus, parameters) in self.buses.iter_mut().zip(update.buses) {
            bus.gain.update([parameters.gain], self.fade_frames);
            bus.output.update(parameters.chain, insert_fade, retired);
        }
        if let Some(limiter) = update.limiter {
            self.limiter = limiter;
        }
    }

    pub(crate) fn reset(&mut self) {
        self.limiter.gain = 1.;
        for track in &mut self.tracks {
            track.output.reset();
            track.automation_fade = 0;
            track.input_fade = 0;
            track.last_input = [0.; 2];
            track.fader = track.target_fader;
            track.ramp = 0;
            track.meter = Meter::default();
            track.routes.reset();
        }
        for bus in &mut self.buses {
            bus.output.reset();
            bus.gain.reset();
            bus.meter = Meter::default();
        }
    }

    pub(crate) fn meters(&self) -> Vec<[f32; 2]> {
        self.tracks
            .iter()
            .map(|track| track.meter.peak)
            .chain(self.buses.iter().map(|bus| bus.meter.peak))
            .collect()
    }

    pub(crate) fn finish_block(&mut self, release: f32) {
        for meter in self
            .tracks
            .iter_mut()
            .map(|track| &mut track.meter)
            .chain(self.buses.iter_mut().map(|bus| &mut bus.meter))
        {
            meter.finish_block(release);
        }
    }

    pub(crate) fn sample(
        &mut self,
        window: &Window<'_>,
        pages: Option<&Pages>,
    ) -> Option<[f32; 2]> {
        let mut sum = [0.; 2];
        let mut bus_inputs = [[0.; 2]; routing::BUS_LIMIT];
        let position = window.position;
        let mut inputs = [[0.; 2]; TRACK_LIMIT];
        let mut peaks = [0.; TRACK_LIMIT];
        match &self.inputs {
            DryInputs::Memory(tracks) => {
                for (input, track) in inputs.iter_mut().zip(tracks) {
                    *input = window.read(track);
                }
            }
            DryInputs::Paged(_) => {
                pages?.read_inputs(window, &mut inputs[..self.tracks.len()])?;
            }
        }
        for ((track, peak_value), input) in self.tracks.iter_mut().zip(&mut peaks).zip(&mut inputs)
        {
            if track.input_fade > 0 {
                let weight = track.input_fade as f32 / self.fade_frames as f32;
                *input = blend(track.previous_input, *input, weight);
                track.input_fade -= 1;
            }
            track.last_input = *input;
            *peak_value = peak(*input);
        }
        for (track, input) in self.tracks.iter_mut().zip(inputs) {
            let output = track.output.process(input, self.fade_frames, &peaks);
            if track.ramp > 0 {
                for (gain, slope) in track.fader.iter_mut().zip(track.slope) {
                    *gain += slope;
                }
                track.ramp -= 1;
                if track.ramp == 0 {
                    track.fader = track.target_fader;
                }
            }
            let mut automated_gain = track.automation.at(position);
            if track.automation_fade > 0 {
                if let Some(previous) = &mut track.previous_automation {
                    let weight = f64::from(track.automation_fade) / f64::from(self.fade_frames);
                    automated_gain += (previous.at(position) - automated_gain) * weight;
                }
                track.automation_fade -= 1;
            }
            let frame = std::array::from_fn(|side| {
                (f64::from(output[side]) * track.fader[side] * automated_gain) as f32
            });
            track.meter.observe(frame);
            let routes = track.routes.step();
            if routes[0] != 0. {
                routing::add(frame, routes[0], &mut sum);
            }
            for (gain, total) in routes[1..]
                .iter()
                .copied()
                .zip(&mut bus_inputs[..self.buses.len()])
            {
                if gain != 0. {
                    routing::add(frame, gain, total);
                }
            }
        }
        for (bus, input) in self.buses.iter_mut().zip(bus_inputs) {
            let frame = bus.output.process(input, self.fade_frames, &peaks);
            let [gain] = bus.gain.step();
            let frame = frame.map(|sample| (f64::from(sample) * gain) as f32);
            bus.meter.observe(frame);
            for (sample, total) in frame.into_iter().zip(&mut sum) {
                *total += sample;
            }
        }
        Some(self.limiter.apply(sum))
    }
}
