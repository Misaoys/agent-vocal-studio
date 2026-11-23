use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

pub fn uid(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4().simple())
}
const VIBRATO_END: f64 = 1.;
const VIBRATO_FADE_IN_MS: f64 = 80.;
const VIBRATO_FADE_OUT_MS: f64 = 0.;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Vibrato {
    pub depth: f64,
    pub rate: f64,
    pub start: f64,
    #[serde(default = "vibrato_end", skip_serializing_if = "is_vibrato_end")]
    pub end: f64,
    #[serde(
        default = "vibrato_fade_in",
        skip_serializing_if = "is_vibrato_fade_in"
    )]
    pub fade_in_ms: f64,
    #[serde(
        default = "vibrato_fade_out",
        skip_serializing_if = "is_vibrato_fade_out"
    )]
    pub fade_out_ms: f64,
}
fn vibrato_end() -> f64 {
    VIBRATO_END
}
fn is_vibrato_end(value: &f64) -> bool {
    *value == VIBRATO_END
}
fn vibrato_fade_in() -> f64 {
    VIBRATO_FADE_IN_MS
}
fn is_vibrato_fade_in(value: &f64) -> bool {
    *value == VIBRATO_FADE_IN_MS
}
fn vibrato_fade_out() -> f64 {
    VIBRATO_FADE_OUT_MS
}
fn is_vibrato_fade_out(value: &f64) -> bool {
    *value == VIBRATO_FADE_OUT_MS
}
impl Default for Vibrato {
    fn default() -> Self {
        Self {
            depth: 0.,
            rate: 5.5,
            start: 0.6,
            end: VIBRATO_END,
            fade_in_ms: VIBRATO_FADE_IN_MS,
            fade_out_ms: VIBRATO_FADE_OUT_MS,
        }
    }
}
impl Vibrato {
    pub(crate) fn has_envelope(&self) -> bool {
        !is_vibrato_end(&self.end)
            || !is_vibrato_fade_in(&self.fade_in_ms)
            || !is_vibrato_fade_out(&self.fade_out_ms)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            finite(self.depth, 0., 200.)
                && finite(self.rate, 1., 12.)
                && finite(self.start, 0., 1.)
                && finite(self.end, self.start, 1.)
                && finite(self.fade_in_ms, 0., 2000.)
                && finite(self.fade_out_ms, 0., 2000.),
            "颤音参数无效。起点须早于或等于终点，淡入淡出须为 0–2000 毫秒。"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    pub start: f64,
    pub duration: f64,
    pub pitch: u8,
    pub lyric: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vowel_continuation: bool,
    pub velocity: f64,
    pub pitch_points: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pitch_blend: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consonant_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vowel_offset_ms: Option<f64>,
    pub vibrato: Vibrato,
}
impl Note {
    pub fn from_value(v: &Value) -> Result<Self> {
        let mut v = v.clone();
        let o = v.as_object_mut().context("Note must be an object")?;
        o.entry("id").or_insert(uid("n").into());
        o.entry("start").or_insert(0.into());
        o.entry("duration").or_insert(1.into());
        o.entry("pitch").or_insert(60.into());
        o.entry("lyric").or_insert("あ".into());
        o.entry("velocity").or_insert(90.into());
        o.entry("pitch_points").or_insert(serde_json::json!([]));
        o.entry("vibrato")
            .or_insert(serde_json::to_value(Vibrato::default())?);
        Ok(serde_json::from_value(v)?)
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Track {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) color_slot: Option<u8>,
    pub gain_db: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) gain_automation: Vec<crate::automation::Point>,
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
    pub singer_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) openutau_voice: Option<String>,
    pub notes: Vec<Note>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) lyric_lines: Vec<crate::lyric_lines::Line>,
    #[serde(default)]
    pub audio: Option<AudioClip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clips: Option<Vec<crate::clips::Clip>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<Instrument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controllers: Vec<Controller>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<crate::effects::Effect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sends: Vec<crate::routing::Send>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) output_group: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Instrument {
    #[serde(default)]
    pub bank: u16,
    #[serde(default)]
    pub program: u8,
    #[serde(default)]
    pub drums: bool,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Controller {
    pub beat: f64,
    pub controller: u8,
    pub value: u8,
}
fn default_tail() -> f64 {
    0.35
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AudioClip {
    pub asset: String,
    pub start: f64,
    pub seconds: f64,
}
impl Track {
    pub fn new(name: &str) -> Self {
        Self {
            id: uid("t"),
            name: name.into(),
            color_slot: None,
            gain_db: 0.,
            gain_automation: vec![],
            pan: 0.,
            mute: false,
            solo: false,
            singer_id: None,
            openutau_voice: None,
            notes: vec![],
            lyric_lines: vec![],
            audio: None,
            clips: None,
            instrument: None,
            controllers: vec![],
            effects: vec![],
            sends: vec![],
            output_group: None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Project {
    pub format: String,
    pub id: String,
    pub revision: u64,
    pub title: String,
    pub bpm: f64,
    pub meter: [u32; 2],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo_map: Vec<crate::timeline::TempoPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meter_map: Vec<crate::timeline::MeterPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) harmony: Vec<crate::harmony::Marker>,
    pub tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) section_markers: Option<Vec<SectionMarker>>,
    #[serde(default = "default_tail")]
    pub tail_seconds: f64,
    #[serde(default, skip_serializing_if = "zero_gain")]
    pub master_gain_db: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_limiter: Option<crate::effects::Limiter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) aux_buses: Vec<crate::routing::Bus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) mix_groups: Vec<crate::routing::Bus>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SectionMarker {
    pub name: String,
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

// Ordered by feature version for minimum-version validation.
pub(crate) const TRACK_LIMIT: usize = 32;
const TRACK_COLOR_COUNT: usize = 8;
pub const PROJECT_FORMATS: [&str; 24] = [
    "agent-vocal/1",
    "agent-vocal/2",
    "agent-vocal/3",
    "agent-vocal/4",
    "agent-vocal/5",
    "agent-vocal/6",
    "agent-vocal/7",
    "agent-vocal/8",
    "agent-vocal/9",
    "agent-vocal/10",
    "agent-vocal/11",
    "agent-vocal/12",
    "agent-vocal/13",
    "agent-vocal/14",
    "agent-vocal/15",
    "agent-vocal/16",
    "agent-vocal/17",
    "agent-vocal/18",
    "agent-vocal/19",
    "agent-vocal/20",
    "agent-vocal/21",
    "agent-vocal/22",
    "agent-vocal/23",
    "agent-vocal/24",
];
pub(crate) const MAX_VOWEL_OFFSET_MS: f64 = 120.;
#[derive(Default, Serialize, Deserialize)]
struct Document {
    current: Option<Project>,
    undo: Vec<Project>,
    redo: Vec<Project>,
}
fn zero_gain(value: &f64) -> bool {
    *value == 0.
}
pub(crate) fn validate_master_gain(db: f64) -> Result<()> {
    ensure!(finite(db, -60., 12.), "总输出电平须在 -60 到 +12 dB 之间");
    Ok(())
}
fn finite(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && v >= lo && v <= hi
}
pub fn identifier(v: &str) -> Result<()> {
    ensure!(
        !v.is_empty()
            && v.len() <= 80
            && v.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
        "Invalid identifier"
    );
    Ok(())
}
impl Project {
    pub fn has_effect_settings(&self) -> bool {
        self.master_limiter.is_some() || self.tracks.iter().any(|track| !track.effects.is_empty())
    }

    pub fn enable_clip_format(&mut self) {
        if matches!(self.format.as_str(), "agent-vocal/1" | "agent-vocal/2") {
            self.format = "agent-vocal/3".into();
        }
    }

    fn inserts(&self) -> impl Iterator<Item = &crate::effects::Effect> {
        self.tracks
            .iter()
            .flat_map(|track| &track.effects)
            .chain(crate::routing::buses(self).flat_map(|bus| &bus.effects))
    }

    fn has_sidechains(&self) -> bool {
        self.inserts()
            .any(|effect| effect.sidechain_source().is_some())
    }

    fn has_expander(&self) -> bool {
        self.inserts()
            .any(|effect| matches!(effect, crate::effects::Effect::Expander { .. }))
    }

    pub(crate) fn upgrade_feature_format(&mut self) {
        if self.format == "agent-vocal/24" || self.master_gain_db != 0. {
            self.format = "agent-vocal/24".into();
            return;
        }
        if self.format == "agent-vocal/23" || self.section_markers.is_some() {
            self.format = "agent-vocal/23".into();
            return;
        }
        if self.format == "agent-vocal/22"
            || self
                .tracks
                .iter()
                .any(|track| !track.lyric_lines.is_empty())
        {
            self.format = "agent-vocal/22".into();
            return;
        }
        if self.format == "agent-vocal/21" || self.tracks.iter().any(|t| t.openutau_voice.is_some())
        {
            self.format = "agent-vocal/21".into();
            return;
        }
        if self.format == "agent-vocal/20" || self.has_expander() {
            self.format = "agent-vocal/20".into();
            return;
        }
        if self.format == "agent-vocal/19"
            || self
                .tracks
                .iter()
                .filter_map(|track| track.clips.as_ref())
                .flatten()
                .any(crate::clips::Clip::has_beat_fade)
        {
            self.format = "agent-vocal/19".into();
            return;
        }
        if self.format == "agent-vocal/18"
            || self
                .tracks
                .iter()
                .flat_map(|track| &track.notes)
                .any(|note| note.vibrato.has_envelope())
        {
            self.format = "agent-vocal/18".into();
            return;
        }
        if self.format == "agent-vocal/17" || self.has_sidechains() {
            self.format = "agent-vocal/17".into();
            return;
        }
        if self.format == "agent-vocal/16"
            || self
                .tracks
                .iter()
                .flat_map(|track| &track.notes)
                .any(|note| note.vowel_continuation)
        {
            self.format = "agent-vocal/16".into();
            return;
        }
        if self.format == "agent-vocal/15"
            || self
                .master_limiter
                .as_ref()
                .is_some_and(crate::effects::Limiter::is_true_peak)
        {
            self.format = "agent-vocal/15".into();
            return;
        }
        if self.format == "agent-vocal/14" || !self.mix_groups.is_empty() {
            self.format = "agent-vocal/14".into();
            return;
        }
        if self.format == "agent-vocal/13" || !self.aux_buses.is_empty() {
            self.format = "agent-vocal/13".into();
            return;
        }
        if self.format == "agent-vocal/12" || !self.harmony.is_empty() {
            self.format = "agent-vocal/12".into();
            return;
        }
        if self.format == "agent-vocal/11"
            || self.tracks.iter().any(|t| !t.gain_automation.is_empty())
        {
            self.format = "agent-vocal/11".into();
            return;
        }
        if self.format == "agent-vocal/10"
            || self
                .tracks
                .iter()
                .flat_map(|track| &track.notes)
                .any(|note| note.vowel_offset_ms.is_some())
        {
            self.format = "agent-vocal/10".into();
            return;
        }
        if self.format == "agent-vocal/9"
            || self
                .tracks
                .iter()
                .flat_map(|track| &track.notes)
                .any(|note| note.consonant_ms.is_some())
        {
            self.format = "agent-vocal/9".into();
            return;
        }
        if self.format == "agent-vocal/8" {
            return;
        }
        let mut clips = self
            .tracks
            .iter()
            .filter_map(|track| track.clips.as_ref())
            .flatten();
        if clips.clone().any(|clip| !clip.warp_points.is_empty()) {
            self.format = "agent-vocal/8".into();
            return;
        }
        if self.format == "agent-vocal/7" {
            return;
        }
        if clips.clone().any(|clip| clip.beat_length.is_some()) {
            self.format = "agent-vocal/7".into();
            return;
        }
        if self.format == "agent-vocal/6" {
            return;
        }
        if clips.any(crate::clips::Clip::has_shaped_fade) {
            self.format = "agent-vocal/6".into();
        } else if self
            .tracks
            .iter()
            .flat_map(|track| &track.notes)
            .any(|note| !note.pitch_blend.is_empty())
        {
            self.format = "agent-vocal/5".into();
        } else if self.format != "agent-vocal/5" && self.has_effect_settings() {
            self.format = "agent-vocal/4".into();
        }
    }

    pub fn validate(&self) -> Result<()> {
        crate::timeline::validate(self.bpm, self.meter, &self.tempo_map, &self.meter_map)?;
        let clock = self.timeline();
        ensure!(
            self.tempo_map.iter().all(|p| clock.seconds(p.beat) <= 900.)
                && self.meter_map.iter().all(|p| clock.seconds(p.beat) <= 900.),
            "Timeline exceeds 15 minutes"
        );
        if let Some(plan) = &self.composition {
            crate::music::validate_plan(plan)?;
        }
        let format_version = PROJECT_FORMATS
            .iter()
            .position(|&format| format == self.format)
            .context("Unsupported project format")?
            + 1;
        validate_master_gain(self.master_gain_db)?;
        ensure!(
            format_version >= 24 || self.master_gain_db == 0.,
            "Master gain requires agent-vocal/24"
        );
        if let Some(markers) = &self.section_markers {
            ensure!(format_version >= 23, "段落标记需要 agent-vocal/23");
            ensure!(markers.len() <= 40, "最多 40 个段落标记");
            let mut previous_end = 0.;
            for marker in markers {
                ensure!(
                    !marker.name.trim().is_empty()
                        && marker.name.chars().count() <= 128
                        && !marker.name.chars().any(char::is_control)
                        && marker
                            .role
                            .as_ref()
                            .is_none_or(|role| role.chars().count() <= 4000),
                    "段落需要有效名称，名称最多 128 个字符"
                );
                ensure!(
                    marker.start >= previous_end
                        && marker.end > marker.start
                        && marker.end <= 50_000.
                        && clock.seconds(marker.end) <= 900.,
                    "段落须按起点排序，范围不得重叠或超过 15 分钟"
                );
                previous_end = marker.end;
            }
        }
        ensure!(
            format_version >= 12 || self.harmony.is_empty(),
            "Harmony markers require agent-vocal/12"
        );
        crate::harmony::validate(&self.harmony)?;
        ensure!(
            format_version >= 13 || self.aux_buses.is_empty(),
            "Auxiliary returns require agent-vocal/13"
        );
        ensure!(
            format_version >= 14 || self.mix_groups.is_empty(),
            "Mix groups require agent-vocal/14"
        );
        crate::routing::validate_buses(&self.mix_groups, &self.aux_buses)?;
        ensure!(
            format_version >= 17 || !self.has_sidechains(),
            "Sidechain requires agent-vocal/17"
        );
        ensure!(
            format_version >= 20 || !self.has_expander(),
            "Downward expansion requires agent-vocal/20"
        );
        for bus in crate::routing::buses(self) {
            crate::effects::validate_sidechains(&bus.effects, &self.tracks, None)?;
        }
        ensure!(
            format_version >= 2 || (self.tempo_map.is_empty() && self.meter_map.is_empty()),
            "Tempo/meter maps require agent-vocal/2; preserve the original and upgrade the project"
        );
        ensure!(
            format_version >= 4 || !self.has_effect_settings(),
            "Effects require agent-vocal/4 or newer"
        );
        if let Some(limiter) = &self.master_limiter {
            ensure!(
                format_version >= 15 || !limiter.is_true_peak(),
                "True peak limiting requires agent-vocal/15"
            );
            limiter.validate()?;
        }
        identifier(&self.id)?;
        ensure!(self.title.chars().count() <= 200, "Title too long");
        ensure!(finite(self.bpm, 30., 300.), "BPM must be 30..300");
        ensure!(
            finite(self.tail_seconds, 0.1, 12.),
            "Render tail must be 0.1..12 seconds"
        );
        ensure!(
            (1..=32).contains(&self.meter[0]) && [1, 2, 4, 8, 16, 32].contains(&self.meter[1]),
            "Invalid time signature"
        );
        ensure!(
            !self.tracks.is_empty() && self.tracks.len() <= TRACK_LIMIT,
            "Need 1..32 tracks"
        );
        let mut ids = std::collections::HashSet::new();
        let mut count = 0;
        for t in &self.tracks {
            identifier(&t.id)?;
            ensure!(ids.insert(t.id.clone()), "Duplicate id");
            ensure!(
                t.color_slot
                    .is_none_or(|slot| usize::from(slot) < TRACK_COLOR_COUNT),
                "Invalid track color"
            );
            crate::routing::validate_track(t, self)?;
            ensure!(
                t.effects.len() <= crate::effects::CHAIN_LIMIT,
                "Track effect limit is 8"
            );
            crate::effects::validate_sidechains(&t.effects, &self.tracks, Some(&t.id))?;
            for effect in &t.effects {
                effect.validate()?;
            }
            ensure!(
                t.name.chars().count() <= 100
                    && finite(t.gain_db, -60., 12.)
                    && finite(t.pan, -1., 1.),
                "Invalid track settings"
            );
            ensure!(
                format_version >= 11 || t.gain_automation.is_empty(),
                "Gain automation requires agent-vocal/11"
            );
            if let Some(profile) = &t.openutau_voice {
                identifier(profile).context("OpenUtau 声库 ID 无效")?;
                ensure!(
                    format_version >= 21 && t.instrument.is_none() && !t.is_audio(),
                    "OpenUtau 声库只可分配给歌声轨"
                );
            }
            crate::automation::validate(&t.gain_automation)?;
            ensure!(
                format_version >= 22 || t.lyric_lines.is_empty(),
                "原词绑定需要 agent-vocal/22"
            );
            crate::lyric_lines::validate(t)?;
            ensure!(
                t.audio.is_none() || t.clips.is_none(),
                "Track has both legacy audio and clips"
            );
            if t.is_audio() {
                ensure!(
                    t.instrument.is_none(),
                    "A track cannot contain both audio and a sampled instrument"
                );
                ensure!(
                    t.notes.is_empty(),
                    "Audio tracks cannot contain vocal notes"
                );
            }
            if let Some(a) = &t.audio {
                crate::clips::validate_asset_name(&a.asset)?;
                ensure!(
                    finite(a.start, 0., 50000.)
                        && finite(a.seconds, 0.001, 900.)
                        && clock.seconds(a.start) + a.seconds <= 900.,
                    "Invalid audio clip timing"
                );
            }
            if let Some(clips) = &t.clips {
                ensure!(
                    format_version >= 3,
                    "Audio clips require agent-vocal/3 or newer"
                );
                ensure!(
                    clips.len() <= 256,
                    "Too many clips on the audio track (maximum 256)"
                );
                for clip in clips {
                    clip.validate(&clock)?;
                    ensure!(
                        !clip.has_beat_fade() || format_version >= 19,
                        "Beat-anchored fades require agent-vocal/19"
                    );
                    ensure!(
                        !clip.has_shaped_fade() || format_version >= 6,
                        "Fade curves require agent-vocal/6"
                    );
                    ensure!(
                        clip.beat_length.is_none() || format_version >= 7,
                        "Tempo clips require agent-vocal/7"
                    );
                    ensure!(
                        clip.warp_points.is_empty() || format_version >= 8,
                        "Warp points require agent-vocal/8"
                    );
                    ensure!(ids.insert(clip.id.clone()), "Duplicate clip id");
                }
            }
            if let Some(i) = &t.instrument {
                ensure!(
                    i.program <= 127
                        && if i.drums {
                            [0, 128].contains(&i.bank) || crate::drum_kit::is_kit(i)
                        } else {
                            i.bank <= 127
                        },
                    "Invalid instrument bank/program"
                );
                if crate::drum_kit::is_kit(i) {
                    ensure!(
                        t.notes.iter().all(|n| (35..=81).contains(&n.pitch)),
                        "Built-in drum kits use GM notes 35–81"
                    );
                    ensure!(
                        t.controllers
                            .iter()
                            .all(|c| matches!(c.controller, 7 | 10 | 11)),
                        "Built-in drum kits support MIDI volume, pan and expression only"
                    );
                }
            }
            ensure!(
                t.controllers.is_empty() || t.instrument.is_some(),
                "MIDI controllers require an instrument track"
            );
            ensure!(
                t.controllers.len() <= 20000,
                "Too many MIDI controller events"
            );
            let mut control_keys = std::collections::HashSet::new();
            let mut previous = -1.;
            for c in &t.controllers {
                ensure!(
                    finite(c.beat, 0., 50000.)
                        && c.beat >= previous
                        && clock.seconds(c.beat) <= 900.,
                    "Controller times must increase and stay within the project limit"
                );
                previous = c.beat;
                ensure!(
                    [1, 7, 10, 11, 64, 91, 93].contains(&c.controller) && c.value <= 127,
                    "Supported MIDI CC: 1,7,10,11,64,91,93"
                );
                ensure!(
                    control_keys.insert((c.beat.to_bits(), c.controller)),
                    "Duplicate MIDI CC at the same beat"
                );
            }
            let mut ns = t.notes.iter().collect::<Vec<_>>();
            ns.sort_by(|a, b| a.start.total_cmp(&b.start));
            let mut end = 0.;
            let mut key_ends = [0f64; 128];
            for (index, n) in ns.iter().enumerate() {
                count += 1;
                identifier(&n.id)?;
                ensure!(ids.insert(n.id.clone()), "Duplicate note id");
                ensure!(
                    finite(n.start, 0., 50000.) && finite(n.duration, 1. / 480., 256.),
                    "Invalid note timing"
                );
                if t.instrument.is_some() {
                    ensure!(n.start+1e-7>=key_ends[n.pitch.min(127) as usize],"Overlapping repetitions of the same MIDI key need a separate instrument track");
                    key_ends[n.pitch.min(127) as usize] = n.start + n.duration;
                } else {
                    ensure!(
                        n.start + 1e-7 >= end,
                        "Vocal notes overlap; place harmony on a separate track"
                    );
                }
                if n.vowel_continuation {
                    ensure!(
                        format_version >= 16,
                        "Vowel continuation requires agent-vocal/16"
                    );
                    ensure!(
                        t.instrument.is_none() && !t.is_audio(),
                        "Vowel continuation requires a vocal track"
                    );
                    let previous = index
                        .checked_sub(1)
                        .map(|i| ns[i])
                        .context("A vowel continuation needs a preceding note")?;
                    ensure!(
                        (n.start - previous.start - previous.duration).abs()
                            <= crate::lyrics::TOUCH_BEATS,
                        "Vowel continuation must touch the preceding note"
                    );
                    if !previous.vowel_continuation {
                        crate::lyrics::sustained_kana(&previous.lyric)?;
                    }
                }
                if let Some(ms) = n.consonant_ms {
                    ensure!(
                        t.instrument.is_none(),
                        "Consonant timing requires a vocal note"
                    );
                    ensure!(
                        format_version >= 9,
                        "Consonant timing requires agent-vocal/9"
                    );
                    ensure!(
                        finite(ms, 1., 300.),
                        "Consonant duration must be from 1 to 300 ms"
                    );
                }
                if let Some(ms) = n.vowel_offset_ms {
                    ensure!(t.instrument.is_none(), "Vowel timing requires a vocal note");
                    ensure!(format_version >= 10, "Vowel timing requires agent-vocal/10");
                    ensure!(
                        finite(ms, -MAX_VOWEL_OFFSET_MS, MAX_VOWEL_OFFSET_MS),
                        "Vowel offset must be from -120 to 120 ms"
                    );
                }
                end = n.start + n.duration;
                ensure!(clock.seconds(end) <= 900., "Project exceeds 15 minutes");
                ensure!(
                    n.pitch <= 127 && finite(n.velocity, 1., 127.),
                    "Invalid pitch/velocity"
                );
                ensure!(
                    (t.instrument.is_some() || !n.lyric.is_empty())
                        && n.lyric.chars().count() <= 40,
                    "Each note needs a lyric (one kana/mora for synthesis)"
                );
                ensure!(n.pitch_points.len() <= 2000, "Pitch curve too long");
                if t.instrument.is_some() {
                    ensure!(n.pitch_points.is_empty()&&n.pitch_blend.is_empty()&&n.vibrato.depth==0.&&!n.vibrato.has_envelope(),"Per-note pitch curves and vibrato currently belong to vocal tracks; use instrument MIDI CC for supported expression");
                }
                if !n.pitch_blend.is_empty() {
                    ensure!(
                        format_version >= 5,
                        "Pitch blend requires agent-vocal/5 or newer"
                    );
                    ensure!(n.pitch_blend.len() <= 2000, "Pitch blend too long");
                    ensure!(n.pitch_blend.iter().all(|point| finite(point[0],0.,n.duration)&&finite(point[1],0.,1.))
                        &&n.pitch_blend.windows(2).all(|pair| pair[0][0]<pair[1][0]),"Invalid pitch blend: [relative beat, weight from 0 to 1], increasing times");
                }
                let mut prev = -1.;
                for pt in &n.pitch_points {
                    ensure!(
                        finite(pt[0], 0., n.duration)
                            && pt[0] > prev
                            && finite(pt[1], -2400., 2400.),
                        "Invalid pitch curve: [relative beat,cents], increasing times"
                    );
                    prev = pt[0];
                }
                n.vibrato.validate()?;
                ensure!(
                    !n.vibrato.has_envelope() || format_version >= 18,
                    "Vibrato envelopes require agent-vocal/18"
                );
            }
        }
        ensure!(count <= 20000, "Project exceeds 20000 notes");
        if self
            .composition
            .as_ref()
            .is_some_and(|p| p["purpose"] == "technical_test")
        {
            ensure!(count<=64&&self.seconds()<=30.,"Technical probes are limited to 64 notes and 30 seconds; design a song for longer material");
        }
        Ok(())
    }
    pub fn track_mut(&mut self, id: &str) -> Result<&mut Track> {
        self.tracks
            .iter_mut()
            .find(|t| t.id == id)
            .context("Unknown track id")
    }
    fn stabilize_track_colors(&mut self) {
        let mut usage = [0; TRACK_COLOR_COUNT];
        for slot in self.tracks.iter().filter_map(|track| track.color_slot) {
            if let Some(count) = usage.get_mut(usize::from(slot)) {
                *count += 1;
            }
        }
        for track in self
            .tracks
            .iter_mut()
            .filter(|track| track.color_slot.is_none())
        {
            let slot = (0..usage.len()).min_by_key(|&slot| usage[slot]).unwrap();
            usage[slot] += 1;
            track.color_slot = Some(slot as u8);
        }
    }

    fn move_track(&mut self, id: &str, before: Option<&str>) -> Result<()> {
        let position = |id: &str| self.tracks.iter().position(|track| track.id == id);
        let from = position(id).context("Unknown track to move")?;
        ensure!(before != Some(id), "Cannot move a track before itself");
        let boundary = match before {
            Some(anchor) => position(anchor).context("Unknown destination track")?,
            None => self.tracks.len(),
        };
        let to = boundary - usize::from(boundary > from);
        if from != to {
            self.stabilize_track_colors();
            let track = self.tracks.remove(from);
            self.tracks.insert(to, track);
        }
        Ok(())
    }

    fn duplicate_track(&mut self, id: &str, name: Option<&str>) -> Result<String> {
        ensure!(self.tracks.len() < TRACK_LIMIT, "Need 1..32 tracks");
        let index = self
            .tracks
            .iter()
            .position(|track| track.id == id)
            .context("Unknown track to duplicate")?;
        let mut copy = self.tracks[index].clone();
        copy.id = uid("t");
        if let Some(audio) = copy.audio.take() {
            copy.clips = Some(vec![crate::clips::Clip::new(
                uid("clip"),
                &audio,
                &copy.name,
            )]);
        } else {
            for clip in copy.clips.iter_mut().flatten() {
                clip.id = uid("clip");
            }
        }
        copy.name = name.map(str::to_owned).unwrap_or_else(|| {
            let base: String = copy.name.chars().take(97).collect();
            format!("{base} 副本")
        });
        copy.mute = true;
        copy.solo = false;
        copy.color_slot = None;
        let mut note_ids = HashMap::new();
        for note in &mut copy.notes {
            let new_id = uid("n");
            note_ids.insert(std::mem::replace(&mut note.id, new_id.clone()), new_id);
        }
        crate::lyric_lines::remap(&mut copy, &note_ids);
        self.stabilize_track_colors();
        let id = copy.id.clone();
        self.tracks.insert(index + 1, copy);
        self.stabilize_track_colors();
        Ok(id)
    }
    pub fn seconds(&self) -> f64 {
        let clock = self.timeline();
        self.tracks
            .iter()
            .map(|t| {
                let n = t
                    .notes
                    .iter()
                    .map(|n| clock.seconds(n.start + n.duration))
                    .fold(0., f64::max);
                let a = t
                    .media_clips()
                    .iter()
                    .map(|clip| clip.end_seconds(&clock))
                    .fold(0., f64::max);
                let c = t
                    .controllers
                    .last()
                    .map(|c| clock.seconds(c.beat))
                    .unwrap_or(0.);
                n.max(a).max(c)
            })
            .fold(0., f64::max)
    }
    pub fn timeline(&self) -> crate::timeline::Timeline {
        crate::timeline::Timeline::new(self.bpm, self.meter, &self.tempo_map, &self.meter_map)
    }
}
#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
    index: Arc<crate::project_index::ProjectIndex>,
}
impl Store {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("projects"))?;
        Ok(Self {
            root,
            index: Arc::default(),
        })
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        identifier(id)?;
        Ok(self.root.join("projects").join(format!("{id}.avproj")))
    }
    fn document(&self, id: &str) -> Result<Document> {
        Ok(serde_json::from_slice(&fs::read(self.path(id)?)?)?)
    }
    fn write(&self, id: &str, d: &Document) -> Result<()> {
        let path = self.path(id)?;
        let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(d)?;
        use std::io::Write;
        let mut f = fs::File::create(&temp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        if path.exists() {
            fs::copy(&path, path.with_extension("avproj.bak"))?;
        }
        // The old file remains intact until the atomic replacement succeeds.
        #[cfg(windows)]
        {
            replace_windows(&temp, &path)?;
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temp, &path)?;
        }
        self.index.invalidate(id);
        Ok(())
    }
    pub fn read(&self, id: &str) -> Result<Project> {
        let p = self.document(id)?.current.context("Missing project")?;
        p.validate()?;
        self.validate_assets(&p)?;
        Ok(p)
    }
    fn validate_assets(&self, p: &Project) -> Result<()> {
        let mut durations = std::collections::HashMap::new();
        for t in &p.tracks {
            if let Some(a) = &t.audio {
                let wav = hound::WavReader::open(self.root.join("assets").join(&a.asset)).context(
                    "Backing asset missing; import the portable avpack to restore its audio",
                )?;
                let spec = wav.spec();
                ensure!(
                    spec.sample_rate == 48000
                        && spec.channels == 2
                        && spec.sample_format == hound::SampleFormat::Float,
                    "Invalid backing audio format"
                );
                ensure!(
                    (wav.duration() as f64 / 48000. - a.seconds).abs() <= 1. / 48000.,
                    "Backing clip duration does not match its audio asset"
                );
            }
            for clip in t.clips.iter().flatten() {
                let duration = match durations.get(&clip.asset) {
                    Some(&frames) => frames,
                    None => {
                        let frames =
                            crate::audio::asset_reader(&self.root, &clip.asset)?.duration();
                        durations.insert(clip.asset.clone(), frames);
                        frames
                    }
                };
                ensure!(
                    clip.offset <= duration && clip.frames <= duration - clip.offset,
                    "Clip source frames exceed its WAV asset"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn list(&self, query: &crate::project_index::Query) -> Result<Value> {
        self.index.list(&self.root, query, |id| self.read(id))
    }
    pub fn create(
        &self,
        title: &str,
        bpm: f64,
        meter: [u32; 2],
        tracks: Vec<Track>,
    ) -> Result<Project> {
        let p = Project {
            format: "agent-vocal/2".into(),
            id: uid("p"),
            revision: 1,
            composition: None,
            section_markers: None,
            tail_seconds: 2.,
            master_gain_db: 0.,
            master_limiter: None,
            aux_buses: vec![],
            mix_groups: vec![],
            title: title.into(),
            bpm,
            meter,
            tempo_map: vec![],
            meter_map: vec![],
            harmony: vec![],
            tracks,
        };
        self.create_copy(title, p)
    }
    pub fn create_copy(&self, title: &str, mut p: Project) -> Result<Project> {
        ensure!(
            PROJECT_FORMATS.contains(&p.format.as_str()),
            "Unsupported project format"
        );
        if p.format == "agent-vocal/1" {
            p.format = "agent-vocal/2".into();
        }
        p.id = uid("p");
        p.revision = 1;
        p.title = title.into();
        p.validate()?;
        if p.tracks.iter().any(|track| track.color_slot.is_some()) {
            p.stabilize_track_colors();
        }
        self.validate_assets(&p)?;
        self.write(
            &p.id,
            &Document {
                current: Some(p.clone()),
                ..Document::default()
            },
        )?;
        Ok(p)
    }
    pub fn edit(&self, id: &str, revision: u64, ops: &Value) -> Result<Project> {
        let d = self.document(id)?;
        let old = d.current.as_ref().context("Missing project")?;
        ensure!(
            old.revision == revision,
            "REVISION_CONFLICT: current={}, expected={}; read project again",
            old.revision,
            revision
        );
        let p = Self::preview_edit(old, ops)?;
        self.save_edit(id, d, p)
    }
    pub(crate) fn preview_edit(old: &Project, ops: &Value) -> Result<Project> {
        Self::preview_edit_with_refs(old, ops).map(|(project, _)| project)
    }
    pub(crate) fn preview_edit_with_refs(
        old: &Project,
        ops: &Value,
    ) -> Result<(Project, HashMap<String, String>)> {
        let mut p = old.clone();
        if p.format == "agent-vocal/1" {
            p.format = "agent-vocal/2".into();
        }
        let ops = ops.as_array().context("operations must be an array")?;
        ensure!(
            !ops.is_empty() && ops.len() <= 10000,
            "Need 1..10000 operations"
        );
        let mut track_refs = HashMap::<String, String>::new();
        for op in ops {
            let kind = op["op"].as_str().context("Missing operation")?;
            let track_id = || {
                if let Some(reference) = op.get("track_ref") {
                    ensure!(
                        op.get("track_id").is_none(),
                        "Use either track_ref or track_id"
                    );
                    return track_refs
                        .get(reference.as_str().context("track_ref must be a string")?)
                        .map(String::as_str)
                        .context("track_ref must name a track created earlier in this edit");
                }
                field(op, "track_id")
            };
            match kind {
                "set_project" => {
                    ensure!(
                        op.get("track_ref").is_none(),
                        "set_project does not target a track"
                    );
                    let v = &op["values"];
                    allowed(
                        v,
                        &[
                            "title",
                            "bpm",
                            "meter",
                            "tempo_map",
                            "meter_map",
                            "harmony",
                            "composition",
                            "section_markers",
                            "tail_seconds",
                            "master_gain_db",
                            "master_limiter",
                            "aux_buses",
                            "mix_groups",
                        ],
                    )?;
                    let mut pv = serde_json::to_value(&p)?;
                    merge(&mut pv, v)?;
                    p = serde_json::from_value(pv)?;
                }
                "set_harmony_range" => {
                    allowed(op, &["op", "beat_range", "markers"])?;
                    p.harmony = crate::harmony::replace_range(
                        &p.harmony,
                        serde_json::from_value(op["markers"].clone())?,
                        crate::harmony::range(&op["beat_range"])?,
                    )?;
                }
                "add_track" => {
                    let track = Track::new("和声");
                    if op.get("track_ref").is_some() {
                        ensure!(
                            op.get("track_id").is_none(),
                            "Use track_ref for a new track"
                        );
                        let reference = field(op, "track_ref")?;
                        identifier(reference)?;
                        ensure!(!track_refs.contains_key(reference), "Duplicate track_ref");
                        track_refs.insert(reference.to_owned(), track.id.clone());
                    }
                    let mut v = serde_json::to_value(track)?;
                    let values = op.get("values").cloned().unwrap_or(serde_json::json!({}));
                    allowed(
                        &values,
                        &[
                            "name",
                            "gain_db",
                            "gain_automation",
                            "pan",
                            "mute",
                            "solo",
                            "singer_id",
                            "openutau_voice",
                            "instrument",
                            "controllers",
                            "effects",
                            "sends",
                            "output_group",
                        ],
                    )?;
                    merge(&mut v, &values)?;
                    p.tracks.push(serde_json::from_value(v)?);
                }
                "remove_track" => {
                    let tid = track_id()?;
                    ensure!(p.tracks.iter().any(|t| t.id == tid), "Unknown track");
                    p.tracks.retain(|t| t.id != tid);
                }
                "move_track" => {
                    allowed(op, &["op", "track_id", "track_ref", "before_track_id"])?;
                    let before = op
                        .get("before_track_id")
                        .context("Missing before_track_id")?;
                    let anchor = if before.is_null() {
                        None
                    } else {
                        Some(
                            before
                                .as_str()
                                .context("before_track_id must be a track ID or null")?,
                        )
                    };
                    p.move_track(track_id()?, anchor)?;
                }
                "duplicate_track" => {
                    allowed(
                        op,
                        &["op", "track_id", "track_ref", "name", "new_track_ref"],
                    )?;
                    let name = op.get("name").map(|_| field(op, "name")).transpose()?;
                    let reference = op
                        .get("new_track_ref")
                        .map(|_| field(op, "new_track_ref"))
                        .transpose()?;
                    if let Some(reference) = reference {
                        identifier(reference)?;
                        ensure!(!track_refs.contains_key(reference), "Duplicate track_ref");
                    }
                    let copy = p.duplicate_track(track_id()?, name)?;
                    if let Some(reference) = reference {
                        track_refs.insert(reference.to_owned(), copy);
                    }
                }
                "update_track" => {
                    let t = p.track_mut(track_id()?)?;
                    allowed(
                        &op["values"],
                        &[
                            "name",
                            "gain_db",
                            "gain_automation",
                            "pan",
                            "mute",
                            "solo",
                            "singer_id",
                            "openutau_voice",
                            "audio",
                            "instrument",
                            "controllers",
                            "effects",
                            "sends",
                            "output_group",
                        ],
                    )?;
                    let mut v = serde_json::to_value(&*t)?;
                    merge(&mut v, &op["values"])?;
                    *t = serde_json::from_value(v)?;
                }
                "set_gain_range" => {
                    allowed(op, &["op", "track_id", "track_ref", "beat_range", "points"])?;
                    let range = crate::automation::beat_range(&op["beat_range"])?;
                    let points: Vec<crate::automation::Point> =
                        serde_json::from_value(op["points"].clone())?;
                    let clock = p.timeline();
                    let track = p.track_mut(track_id()?)?;
                    track.gain_automation = crate::automation::replace_range(
                        &track.gain_automation,
                        &points,
                        range,
                        &clock,
                    )?;
                }
                "add_notes" | "replace_notes" => {
                    let t = p.track_mut(track_id()?)?;
                    let ns = op["notes"]
                        .as_array()
                        .context("notes must be an array")?
                        .iter()
                        .map(|value| {
                            let mut n = Note::from_value(value)?;
                            if t.instrument.is_some() && value.get("lyric").is_none() {
                                n.lyric.clear();
                            }
                            Ok(n)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if kind == "replace_notes" {
                        t.notes = ns
                    } else {
                        t.notes.extend(ns)
                    };
                }
                "update_note" => {
                    let t = p.track_mut(track_id()?)?;
                    let nid = field(op, "note_id")?;
                    let n = t
                        .notes
                        .iter_mut()
                        .find(|n| n.id == nid)
                        .context("Unknown note")?;
                    allowed(
                        &op["values"],
                        &[
                            "start",
                            "duration",
                            "pitch",
                            "lyric",
                            "vowel_continuation",
                            "velocity",
                            "pitch_points",
                            "pitch_blend",
                            "consonant_ms",
                            "vowel_offset_ms",
                            "vibrato",
                        ],
                    )?;
                    let mut v = serde_json::to_value(&*n)?;
                    merge(&mut v, &op["values"])?;
                    if v["lyric"] != n.lyric {
                        for timing in ["consonant_ms", "vowel_offset_ms"] {
                            if op["values"].get(timing).is_none() {
                                v[timing] = Value::Null;
                            }
                        }
                    }
                    *n = serde_json::from_value(v)?;
                }
                "delete_notes" => {
                    let t = p.track_mut(track_id()?)?;
                    let ids = op["note_ids"]
                        .as_array()
                        .context("note_ids must be an array")?;
                    for id in ids {
                        ensure!(
                            t.notes.iter().any(|n| Some(n.id.as_str()) == id.as_str()),
                            "Unknown note"
                        )
                    }
                    t.notes
                        .retain(|n| !ids.iter().any(|id| id.as_str() == Some(&n.id)));
                }
                "set_lyric_line" => {
                    allowed(op, &["op", "track_id", "track_ref", "note_ids", "text"])?;
                    let ids: Vec<String> = serde_json::from_value(op["note_ids"].clone())?;
                    crate::lyric_lines::set(p.track_mut(track_id()?)?, &ids, field(op, "text")?)?;
                }
                "remove_lyric_line" => {
                    allowed(op, &["op", "track_id", "track_ref", "line_id"])?;
                    crate::lyric_lines::remove(p.track_mut(track_id()?)?, field(op, "line_id")?)?;
                }
                _ => bail!("Unknown operation {kind}"),
            }
        }
        for t in &mut p.tracks {
            t.notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        }
        p.upgrade_feature_format();
        Ok((p, track_refs))
    }
    pub(crate) fn arrangement_edit(
        &self,
        id: &str,
        revision: u64,
        edit: &crate::arrangement::RangeEdit,
        approved_preview: Option<&str>,
        capture: Option<&crate::arrangement::Capture>,
    ) -> Result<Value> {
        let document = self.document(id)?;
        let current = document.current.as_ref().context("Missing project")?;
        ensure!(
            current.revision == revision,
            "REVISION_CONFLICT: current={}, expected={}; preview again",
            current.revision,
            revision
        );
        current.validate()?;
        let preview = if let Some(capture) = capture {
            capture.preview(&self.root, current, edit)?
        } else {
            crate::arrangement::preview(&self.root, current, edit)?
        };
        let mut summary = preview.summary;
        summary["dry_run"] = Value::Bool(approved_preview.is_none());
        if let Some(approved_preview) = approved_preview {
            ensure!(
                approved_preview == preview.id,
                "PREVIEW_MISMATCH: preview this exact edit before committing"
            );
            summary["project"] =
                serde_json::to_value(self.save_edit(id, document, preview.next)?)?;
        }
        Ok(summary)
    }
    fn save_edit(&self, id: &str, mut d: Document, mut p: Project) -> Result<Project> {
        let old = d.current.take().context("Missing project")?;
        p.revision = old.revision + 1;
        p.validate()?;
        if p.tracks.iter().any(|track| track.color_slot.is_some()) {
            p.stabilize_track_colors();
        }
        self.validate_assets(&p)?;
        d.undo.push(old);
        if d.undo.len() > 50 {
            d.undo.remove(0);
        };
        d.redo.clear();
        d.current = Some(p.clone());
        self.write(id, &d)?;
        Ok(p)
    }
    pub(crate) fn commit_preview(&self, preview: Project) -> Result<Project> {
        let id = preview.id.clone();
        let document = self.document(&id)?;
        let current = document.current.as_ref().context("Missing project")?;
        ensure!(
            current.revision == preview.revision,
            "REVISION_CONFLICT: 工程已更新，请生成新预览"
        );
        self.save_edit(&id, document, preview)
    }
    pub fn undo(&self, id: &str, revision: u64, redo: bool) -> Result<Project> {
        let mut d = self.document(id)?;
        let old = d.current.as_ref().context("Missing project")?.clone();
        ensure!(old.revision == revision, "REVISION_CONFLICT");
        let mut p =
            if redo { d.redo.pop() } else { d.undo.pop() }.context("No history available")?;
        if redo {
            d.undo.push(old)
        } else {
            d.redo.push(old)
        };
        p.revision = revision + 1;
        p.validate()?;
        d.current = Some(p.clone());
        self.write(id, &d)?;
        Ok(p)
    }
    pub fn add_audio(
        &self,
        id: &str,
        revision: u64,
        audio: AudioClip,
        name: &str,
    ) -> Result<Project> {
        let mut d = self.document(id)?;
        let old = d.current.as_ref().context("Missing project")?.clone();
        ensure!(old.revision == revision, "REVISION_CONFLICT");
        let mut p = old.clone();
        let mut t = Track::new(name);
        t.audio = Some(audio);
        p.tracks.push(t);
        if p.tracks.iter().any(|track| track.color_slot.is_some()) {
            p.stabilize_track_colors();
        }
        p.revision += 1;
        p.validate()?;
        d.undo.push(old);
        if d.undo.len() > 50 {
            d.undo.remove(0);
        };
        d.redo.clear();
        d.current = Some(p.clone());
        self.write(id, &d)?;
        Ok(p)
    }
    pub(crate) fn append_clip(
        &self,
        id: &str,
        revision: u64,
        track_id: &str,
        audio: AudioClip,
        name: &str,
    ) -> Result<Project> {
        let document = self.document(id)?;
        let old = document.current.as_ref().context("Missing project")?;
        ensure!(old.revision == revision, "REVISION_CONFLICT");
        let mut next = old.clone();
        next.enable_clip_format();
        let clock = next.timeline();
        let track = next.track_mut(track_id)?;
        let clip = crate::clips::Clip::new(uid("clip"), &audio, name);
        let clips = track.promote_clips()?;
        ensure!(
            !clips.iter().any(|other| clip.overlaps(other, &clock)),
            "CLIP_OVERLAP: importing here would overlap another clip"
        );
        clips.push(clip);
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.save_edit(id, document, next)
    }
    pub(crate) fn add_stem_tracks(
        &self,
        id: &str,
        revision: u64,
        tracks: &[(String, AudioClip)],
    ) -> Result<Project> {
        let document = self.document(id)?;
        let old = document.current.as_ref().context("Missing project")?;
        ensure!(old.revision == revision, "REVISION_CONFLICT");
        let mut next = old.clone();
        next.enable_clip_format();
        next.tracks.extend(tracks.iter().map(|(name, audio)| {
            let mut track = Track::new(name);
            track.clips = Some(vec![crate::clips::Clip::new(uid("clip"), audio, name)]);
            track
        }));
        self.save_edit(id, document, next)
    }
    pub(crate) fn clip_edit(
        &self,
        id: &str,
        revision: u64,
        request: &crate::clips::Edit,
        approved: Option<&str>,
    ) -> Result<Value> {
        self.clip_transaction(id, revision, approved, |old| {
            crate::clips::plan(&self.root, old, request)
        })
    }

    pub(crate) fn clip_transaction(
        &self,
        id: &str,
        revision: u64,
        approved: Option<&str>,
        plan: impl FnOnce(&Project) -> Result<(Project, Value)>,
    ) -> Result<Value> {
        let document = self.document(id)?;
        let old = document.current.as_ref().context("Missing project")?;
        ensure!(
            old.revision == revision,
            "REVISION_CONFLICT: preview the current revision"
        );
        old.validate()?;
        self.validate_assets(old)?;
        let (next, mut result) = plan(old)?;
        result["dry_run"] = Value::Bool(approved.is_none());
        if let Some(approved) = approved {
            ensure!(
                result["preview_id"].as_str() == Some(approved),
                "PREVIEW_MISMATCH: preview this exact clip edit"
            );
            result["project"] = serde_json::to_value(self.save_edit(id, document, next)?)?;
        }
        Ok(result)
    }
    pub fn recover(&self, id: &str, expected_revision: Option<u64>) -> Result<Project> {
        if let Ok(p) = self.read(id) {
            ensure!(
                expected_revision == Some(p.revision),
                "Read the current revision before recovering a valid project"
            );
        }
        let path = self.path(id)?;
        let backup = path.with_extension("avproj.bak");
        let backup_bytes = fs::read(&backup).context("No recovery backup available")?;
        let mut d: Document = serde_json::from_slice(&backup_bytes)?;
        let mut p = d.current.clone().context("Backup has no project")?;
        p.revision += 2;
        p.validate()?;
        self.validate_assets(&p)?;
        let preserved = path.with_extension(format!("preserved_{}.json", Uuid::new_v4().simple()));
        if path.exists() {
            fs::copy(&path, preserved)?;
        };
        d.current = Some(p.clone());
        d.redo.clear();
        self.write(id, &d)?;
        fs::write(backup, backup_bytes)?;
        Ok(p)
    }
}
pub fn field<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().with_context(|| format!("Missing string {k}"))
}
fn allowed(v: &Value, fields: &[&str]) -> Result<()> {
    for k in v.as_object().context("values must be an object")?.keys() {
        ensure!(fields.contains(&k.as_str()), "Unsupported field: {k}")
    }
    Ok(())
}
fn merge(dst: &mut Value, src: &Value) -> Result<()> {
    for (k, v) in src.as_object().context("values must be an object")? {
        dst[k] = v.clone()
    }
    Ok(())
}
#[cfg(windows)]
pub(crate) fn replace_windows(src: &Path, dst: &Path) -> Result<()> {
    move_file_windows(src, dst, true)
}
#[cfg(windows)]
pub(crate) fn move_file_windows(src: &Path, dst: &Path, replace: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
    }
    let a: Vec<u16> = src.as_os_str().encode_wide().chain(Some(0)).collect();
    let b: Vec<u16> = dst.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut backoff_ms = [8, 16, 32, 64].into_iter();
    loop {
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), u32::from(replace) | 8) } != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        match (error.raw_os_error(), backoff_ms.next()) {
            // Win32 access denied, sharing violation, or a locked file region.
            (Some(5 | 32 | 33), Some(delay)) => {
                std::thread::sleep(std::time::Duration::from_millis(delay))
            }
            _ => return Err(error).context("Atomic save failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn atomic_replace_allows_a_short_file_handle_conflict() -> Result<()> {
        use std::{
            os::windows::fs::OpenOptionsExt,
            time::{Duration, Instant},
        };
        let dir = std::env::temp_dir().join(uid("avs_atomic"));
        fs::create_dir(&dir)?;
        let temporary = dir.join("next");
        let saved = dir.join("current");
        fs::write(&temporary, b"new")?;
        fs::write(&saved, b"old")?;
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&saved)?;
        let start = Instant::now();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            drop(handle);
        });
        let replaced = replace_windows(&temporary, &saved);
        let elapsed = start.elapsed();
        release.join().unwrap();
        replaced?;
        assert!(elapsed >= Duration::from_millis(40));
        assert_eq!(fs::read(&saved)?, b"new");
        assert!(!temporary.exists());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn timeline_edits_migrate_legacy_atomically_with_history() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_time_migration"));
        let store = Store::new(root.clone())?;
        let mut track = Track::new("主唱");
        track.notes.push(Note::from_value(
            &serde_json::json!({"start":3,"duration":4,"pitch":60,"lyric":"あ"}),
        )?);
        let mut p = store.create("legacy", 120., [4, 4], vec![track])?;
        p.format = "agent-vocal/1".into();
        store.write(
            &p.id,
            &Document {
                current: Some(p.clone()),
                ..Document::default()
            },
        )?;
        let original = fs::read(store.path(&p.id)?)?;
        let bad = serde_json::json!([{"op":"set_project","values":{"tempo_map":[{"beat":4,"bpm":60},{"beat":3,"bpm":90}]}}]);
        assert!(store.edit(&p.id, p.revision, &bad).is_err());
        assert_eq!(fs::read(store.path(&p.id)?)?, original);
        let good = serde_json::json!([{"op":"set_project","values":{"tempo_map":[{"beat":4,"bpm":60},{"beat":8,"bpm":120,"curve":"linear"}],"meter_map":[{"beat":8,"meter":[6,8]}]}}]);
        let new = store.edit(&p.id, p.revision, &good)?;
        assert_eq!(new.format, "agent-vocal/2");
        assert!((new.seconds() - (2. + 4. * 1.75f64.ln())).abs() < 1e-10);
        let backup: Document =
            serde_json::from_slice(&fs::read(store.path(&p.id)?.with_extension("avproj.bak"))?)?;
        assert_eq!(backup.current.unwrap().format, "agent-vocal/1");
        let undone = store.undo(&p.id, new.revision, false)?;
        assert!(undone.tempo_map.is_empty());
        let redone = store.undo(&p.id, undone.revision, true)?;
        assert_eq!(redone.tempo_map.len(), 2);
        assert_eq!(store.read(&p.id)?.meter_map.len(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn sampled_chords_are_allowed_but_vocal_conversion_is_atomic() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_chords"));
        let store = Store::new(root.clone())?;
        let mut track = Track::new("钢琴");
        track.instrument = Some(Instrument::default());
        for pitch in [60, 64, 67] {
            track.notes.push(Note::from_value(
                &serde_json::json!({"start":0,"duration":1,"pitch":pitch,"lyric":""}),
            )?)
        }
        let p = store.create("test", 120., [4, 4], vec![track])?;
        let tid = &p.tracks[0].id;
        assert!(store.edit(&p.id,p.revision,&serde_json::json!([{"op":"update_track","track_id":tid,"values":{"instrument":null}}])).is_err());
        assert_eq!(store.read(&p.id)?.revision, p.revision);
        let broken = serde_json::json!([{"op":"update_track","track_id":tid,"values":{"controllers":[{"beat":2,"controller":64,"value":0},{"beat":0,"controller":64,"value":127}]}}]);
        assert!(store.edit(&p.id, p.revision, &broken).is_err());
        let mut old = serde_json::to_value(Track::new("旧歌声"))?;
        old.as_object_mut().unwrap().remove("instrument");
        old.as_object_mut().unwrap().remove("controllers");
        let old: Track = serde_json::from_value(old)?;
        assert!(old.instrument.is_none() && old.controllers.is_empty());
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn corrupt_project_recovery_preserves_both_files() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_recovery"));
        let s = Store::new(root.clone())?;
        let p = s.create("Before", 120., [4, 4], vec![Track::new("主唱")])?;
        let p = s.edit(
            &p.id,
            1,
            &serde_json::json!([{"op":"set_project","values":{"title":"After"}}]),
        )?;
        assert!(s.recover(&p.id, None).is_err());
        fs::write(s.path(&p.id)?, b"interrupted/corrupt bytes")?;
        let recovered = s.recover(&p.id, None)?;
        assert_eq!(recovered.title, "Before");
        assert!(recovered.revision > p.revision);
        assert_eq!(s.read(&p.id)?.title, "Before");
        assert!(fs::read_dir(root.join("projects"))?
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("preserved_")));
        assert!(serde_json::from_slice::<Document>(&fs::read(
            s.path(&p.id)?.with_extension("avproj.bak")
        )?)
        .is_ok());
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn transactions_conflicts_undo_and_reopen() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_test"));
        let s = Store::new(root.clone())?;
        let p = s.create("测试", 120., [6, 8], vec![Track::new("主唱")])?;
        let tid = &p.tracks[0].id;
        let p2=s.edit(&p.id,1,&serde_json::json!([{"op":"add_notes","track_id":tid,"notes":[{"start":0,"pitch":64,"lyric":"あ"}]}]))?;
        assert!(s
            .edit(
                &p.id,
                1,
                &serde_json::json!([{"op":"set_project","values":{"title":"lost"}}])
            )
            .is_err());
        let bad=s.edit(&p.id,2,&serde_json::json!([{"op":"set_project","values":{"title":"must rollback"}},{"op":"add_notes","track_id":tid,"notes":[{"start":0.5}]}]));
        assert!(bad.is_err());
        assert_eq!(s.read(&p.id)?.title, "测试");
        assert_eq!(s.undo(&p.id, 2, false)?.tracks[0].notes.len(), 0);
        assert_eq!(s.undo(&p.id, 3, true)?.tracks[0].notes.len(), 1);
        let reopened = Store::new(root.clone())?.read(&p.id)?;
        assert_eq!(reopened.revision, 4);
        assert_eq!(reopened.tracks[0].notes[0].id, p2.tracks[0].notes[0].id);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
