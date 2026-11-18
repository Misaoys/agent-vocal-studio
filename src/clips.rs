use crate::model::{identifier, AudioClip, Project, Track};
use crate::timeline::Timeline;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::HashSet, path::Path};

pub(crate) const RATE: u32 = 48000;
const MAX_FRAMES: u32 = RATE * 900;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Clip {
    pub id: String,
    pub name: String,
    pub asset: String,
    pub start: f64,
    pub offset: u32,
    pub frames: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat_length: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warp_points: Vec<WarpPoint>,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub fade_in: u32,
    #[serde(default)]
    pub fade_out: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_beats: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_beats: Option<f64>,
    #[serde(default, skip_serializing_if = "FadeCurve::is_linear")]
    pub fade_in_curve: FadeCurve,
    #[serde(default, skip_serializing_if = "FadeCurve::is_linear")]
    pub fade_out_curve: FadeCurve,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WarpPoint {
    pub frame: u32,
    pub beat: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FadeCurve {
    #[default]
    Linear,
    EqualPower,
}
impl FadeCurve {
    fn is_linear(&self) -> bool {
        matches!(self, Self::Linear)
    }
    fn gain(self, progress: f32) -> f32 {
        match self {
            Self::Linear => progress,
            Self::EqualPower => (std::f32::consts::FRAC_PI_2 * progress).sin(),
        }
    }
}

pub(crate) fn validate_asset_name(asset: &str) -> Result<()> {
    identifier(
        asset
            .strip_suffix(".wav")
            .context("Audio asset must be WAV")?,
    )
}

impl Clip {
    pub fn new(id: String, audio: &AudioClip, name: &str) -> Self {
        Self {
            id,
            name: if name.trim().is_empty() {
                "音频".into()
            } else {
                name.into()
            },
            asset: audio.asset.clone(),
            start: audio.start,
            offset: 0,
            frames: (audio.seconds * f64::from(RATE)).round() as u32,
            beat_length: None,
            warp_points: Vec::new(),
            gain_db: 0.,
            fade_in: 0,
            fade_out: 0,
            fade_in_beats: None,
            fade_out_beats: None,
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
        }
    }

    pub fn validate(&self, clock: &Timeline) -> Result<()> {
        identifier(&self.id)?;
        validate_asset_name(&self.asset)?;
        ensure!(
            !self.name.trim().is_empty() && self.name.chars().count() <= 100,
            "Invalid clip name"
        );
        ensure!(
            self.start.is_finite() && (0. ..=50000.).contains(&self.start),
            "Invalid clip start"
        );
        ensure!(
            (1..=MAX_FRAMES).contains(&self.frames) && self.offset <= MAX_FRAMES - self.frames,
            "Invalid source frame range"
        );
        ensure!(
            self.fade_in <= self.frames && self.fade_out <= self.frames - self.fade_in,
            "Fades cannot exceed clip length"
        );
        ensure!(
            self.gain_db.is_finite() && (-60. ..=12.).contains(&self.gain_db),
            "Clip gain must be -60..12 dB"
        );
        if let Some(beats) = self.beat_length {
            ensure!(
                beats.is_finite() && beats > 0. && self.start + beats <= 50_000.,
                "跟速片段需要有效的正拍数"
            );
        }
        if !self.warp_points.is_empty() {
            let beats = self.beat_length.context("对拍点需要片段处于跟速模式")?;
            ensure!(self.warp_points.len() <= 1024, "单个片段最多 1024 个对拍点");
            let mut previous = WarpPoint { frame: 0, beat: 0. };
            let mut previous_output = self.start_frame(clock);
            let end = self.end_frame(clock);
            for &point in &self.warp_points {
                ensure!(
                    point.frame > previous.frame
                        && point.frame < self.frames
                        && point.beat.is_finite()
                        && point.beat > previous.beat
                        && point.beat < beats,
                    "对拍点的源帧和相对拍位都须递增，并处于片段内部"
                );
                let output =
                    (clock.seconds(self.start + point.beat) * f64::from(RATE)).round() as i64;
                ensure!(
                    output > previous_output && output < end,
                    "对拍点在渲染时须落在不同采样帧"
                );
                previous = point;
                previous_output = output;
            }
        }
        for (frames, fade) in [
            (self.fade_in, self.fade_in_beats),
            (self.fade_out, self.fade_out_beats),
        ] {
            if let Some(fade) = fade {
                ensure!(
                    frames == 0
                        && self
                            .beat_length
                            .is_some_and(|length| fade.is_finite() && fade > 0. && fade <= length),
                    "拍淡化需要跟速片段，且拍数须在片段长度内"
                );
            }
        }
        ensure!(
            self.end_frame(clock) > self.start_frame(clock)
                && self.end_frame(clock) <= i64::from(MAX_FRAMES),
            "Audio clip exceeds 15 minutes"
        );
        if self.has_beat_fade() {
            let [fade_in, fade_out] = self.fade_edges(clock);
            ensure!(fade_in <= fade_out, "Fades cannot exceed clip length");
        }
        Ok(())
    }

    pub fn start_frame(&self, clock: &Timeline) -> i64 {
        (clock.seconds(self.start) * f64::from(RATE)).round() as i64
    }

    pub(crate) fn recording_window(
        &self,
        clock: &Timeline,
        [start, end]: [f64; 2],
    ) -> Result<[u32; 2]> {
        ensure!(self.beat_length.is_none(), "请选择固定秒数的录音片段");
        ensure!(
            0. <= start && start < end && end <= 50_000.,
            "请选择有效的录音拍区间"
        );
        let origin = self.start_frame(clock);
        let [first, last] = [start, end]
            .map(|beat| (clock.seconds(beat) * f64::from(RATE)).round() as i64 - origin);
        ensure!(
            0 <= first && first < last && last <= i64::from(self.frames),
            "录音片段需要覆盖整个拍区间"
        );
        Ok([first as u32, last as u32])
    }

    fn fixed_slice(&self, [first, end]: [u32; 2], start: f64) -> Result<Self> {
        ensure!(self.beat_length.is_none(), "请选择固定秒数的片段");
        ensure!(
            first < end && end <= self.frames,
            "Invalid source frame range"
        );
        for edge in [first, end] {
            ensure!(
                edge == 0
                    || edge == self.frames
                    || (edge >= self.fade_in && edge <= self.frames - self.fade_out),
                "SPLIT_IN_FADE: {}，请把范围边界移出淡化",
                self.name
            );
        }
        let mut piece = self.clone();
        piece.start = start;
        piece.offset += first;
        piece.frames = end - first;
        if first != 0 {
            piece.fade_in = 0;
        }
        if end != self.frames {
            piece.fade_out = 0;
        }
        piece.reset_empty_fades();
        Ok(piece)
    }
    pub fn end_frame(&self, clock: &Timeline) -> i64 {
        self.beat_length.map_or_else(
            || self.start_frame(clock) + i64::from(self.frames),
            |beats| (clock.seconds(self.start + beats) * f64::from(RATE)).round() as i64,
        )
    }
    pub fn frame_at_source(&self, clock: &Timeline, frame: u32) -> i64 {
        if let Some(beats) = self.beat_length {
            let beat = self.start + self.relative_beat_at_source(frame, beats);
            (clock.seconds(beat) * f64::from(RATE)).round() as i64
        } else {
            self.start_frame(clock) + i64::from(frame)
        }
    }
    fn source_at_beat(&self, clock: &Timeline, beat: f64) -> f64 {
        match self.beat_length {
            Some(beats) => {
                let relative = (beat - self.start).clamp(0., beats);
                let index = self
                    .warp_points
                    .partition_point(|point| point.beat <= relative);
                let [a, b] = self.warp_segment(index, beats);
                f64::from(a.frame)
                    + f64::from(b.frame - a.frame) * (relative - a.beat) / (b.beat - a.beat)
            }
            None => {
                (clock.seconds(beat) * f64::from(RATE)).round() - self.start_frame(clock) as f64
            }
        }
    }
    pub fn source_at_render_frame(&self, clock: &Timeline, frame: i64) -> f64 {
        self.source_at_beat(clock, clock.beat(frame as f64 / f64::from(RATE)))
    }
    pub(crate) fn beat_at_source(&self, clock: &Timeline, frame: u32) -> f64 {
        match self.beat_length {
            Some(beats) => self.start + self.relative_beat_at_source(frame, beats),
            None => clock.beat(self.frame_at_source(clock, frame) as f64 / f64::from(RATE)),
        }
    }
    fn relative_beat_at_source(&self, frame: u32, beats: f64) -> f64 {
        if frame == self.frames {
            return beats;
        }
        let index = self
            .warp_points
            .partition_point(|point| point.frame <= frame);
        let [a, b] = self.warp_segment(index, beats);
        a.beat + (b.beat - a.beat) * f64::from(frame - a.frame) / f64::from(b.frame - a.frame)
    }
    fn warp_segment(&self, index: usize, beats: f64) -> [WarpPoint; 2] {
        [
            index
                .checked_sub(1)
                .map_or(WarpPoint { frame: 0, beat: 0. }, |i| self.warp_points[i]),
            self.warp_points.get(index).copied().unwrap_or(WarpPoint {
                frame: self.frames,
                beat: beats,
            }),
        ]
    }
    fn relative_beat_at_edge(&self, frame: i64, beats: f64) -> f64 {
        if (0..=i64::from(self.frames)).contains(&frame) {
            return self.relative_beat_at_source(frame as u32, beats);
        }
        let index = if frame < 0 { 0 } else { self.warp_points.len() };
        let [a, b] = self.warp_segment(index, beats);
        a.beat
            + (b.beat - a.beat) * (frame - i64::from(a.frame)) as f64 / f64::from(b.frame - a.frame)
    }

    fn slice_warp(&mut self, first: i64, end: i64) {
        if let Some(beats) = self.beat_length {
            let start = self.relative_beat_at_edge(first, beats).max(-self.start);
            let end_beat = self.relative_beat_at_edge(end, beats);
            self.start += start;
            self.beat_length = Some(end_beat - start);
            self.warp_points.retain(|point| {
                let frame = i64::from(point.frame);
                first < frame && frame < end
            });
            for point in &mut self.warp_points {
                point.frame = (i64::from(point.frame) - first) as u32;
                point.beat -= start;
            }
        }
    }
    pub fn end_seconds(&self, clock: &Timeline) -> f64 {
        self.end_frame(clock) as f64 / f64::from(RATE)
    }
    pub fn beat_span(&self, clock: &Timeline) -> [f64; 2] {
        [self.start, self.beat_at_source(clock, self.frames)]
    }
    pub fn overlaps(&self, other: &Self, clock: &Timeline) -> bool {
        self.start_frame(clock) < other.end_frame(clock)
            && other.start_frame(clock) < self.end_frame(clock)
    }
    pub(crate) fn has_shaped_fade(&self) -> bool {
        !self.fade_in_curve.is_linear() || !self.fade_out_curve.is_linear()
    }
    pub(crate) fn has_beat_fade(&self) -> bool {
        self.fade_in_beats.is_some() || self.fade_out_beats.is_some()
    }
    fn clear_fade_in(&mut self) {
        self.fade_in = 0;
        self.fade_in_beats = None;
    }
    fn clear_fade_out(&mut self) {
        self.fade_out = 0;
        self.fade_out_beats = None;
    }
    fn reset_empty_fades(&mut self) {
        if self.fade_in == 0 && self.fade_in_beats.is_none() {
            self.fade_in_curve = FadeCurve::Linear;
        }
        if self.fade_out == 0 && self.fade_out_beats.is_none() {
            self.fade_out_curve = FadeCurve::Linear;
        }
    }

    fn fade_edges(&self, clock: &Timeline) -> [i64; 2] {
        let start = self.start_frame(clock);
        let end = self.end_frame(clock);
        let at = |beat| (clock.seconds(beat) * f64::from(RATE)).round() as i64;
        let fade_in = self.fade_in_beats.map_or_else(
            || self.frame_at_source(clock, self.fade_in),
            |length| at(self.start + length),
        );
        let fade_out = self.fade_out_beats.map_or_else(
            || self.frame_at_source(clock, self.frames - self.fade_out),
            |length| at(self.start + self.beat_length.unwrap_or_default() - length),
        );
        [fade_in.clamp(start, end), fade_out.clamp(start, end)]
    }

    fn can_split_at(&self, clock: &Timeline, frame: u32) -> bool {
        let [incoming_end, outgoing_start] = self.fade_edges(clock);
        let position = self.frame_at_source(clock, frame);
        let beat_in_body = self.beat_length.is_none_or(|length| {
            let beat = self.relative_beat_at_source(frame, length);
            beat >= self.fade_in_beats.unwrap_or(0.)
                && beat <= length - self.fade_out_beats.unwrap_or(0.)
        });
        frame >= self.fade_in
            && frame <= self.frames - self.fade_out
            && incoming_end <= position
            && position <= outgoing_start
            && beat_in_body
    }

    fn split_at(mut self, clock: &Timeline, at: f64, right_id: String) -> Result<[Self; 2]> {
        ensure!(at.is_finite() && at >= 0., "Invalid split beat");
        ensure!(
            (self.start..=self.beat_span(clock)[1]).contains(&at),
            "Split must land inside the clip"
        );
        let frame = self.source_at_beat(clock, at).round() as i64;
        ensure!(
            frame > 0 && frame < i64::from(self.frames),
            "Split must land inside the clip"
        );
        let frame = frame as u32;
        ensure!(
            self.can_split_at(clock, frame),
            "SPLIT_IN_FADE: move the split outside the fade"
        );
        let mut right = self.clone();
        right.id = right_id;
        if self.beat_length.is_some() {
            right.slice_warp(i64::from(frame), i64::from(self.frames));
            self.slice_warp(0, i64::from(frame));
        } else {
            right.start = at;
        }
        right.offset += frame;
        right.frames -= frame;
        right.clear_fade_in();
        self.frames = frame;
        self.clear_fade_out();
        Ok([self, right])
    }

    fn update_beat_fades(&mut self, clock: &Timeline, new_length: Option<f64>) {
        let Some(original_length) = self.beat_length else {
            return;
        };
        if let Some(new_length) = new_length {
            let factor = new_length / original_length;
            for duration in [&mut self.fade_in_beats, &mut self.fade_out_beats]
                .into_iter()
                .flatten()
            {
                *duration = (*duration * factor).min(new_length);
            }
        } else {
            if let Some(beats) = self.fade_in_beats.take() {
                self.fade_in = self.source_at_beat(clock, self.start + beats).round() as u32;
            }
            if let Some(beats) = self.fade_out_beats.take() {
                let frame = self
                    .source_at_beat(clock, self.start + original_length - beats)
                    .round() as u32;
                self.fade_out = self.frames - frame;
            }
            self.fade_out = self.fade_out.min(self.frames - self.fade_in);
        }
    }

    pub(crate) fn rendered_envelope(&self, clock: &Timeline) -> ClipEnvelope {
        let start = self.start_frame(clock);
        let end = self.end_frame(clock);
        let [fade_in, fade_out] = self.fade_edges(clock);
        ClipEnvelope {
            frames: (end - start) as u32,
            fade_in: (fade_in - start) as u32,
            fade_out: (end - fade_out) as u32,
            fade_in_curve: self.fade_in_curve,
            fade_out_curve: self.fade_out_curve,
        }
    }
}

pub(crate) struct ClipEnvelope {
    frames: u32,
    fade_in: u32,
    fade_out: u32,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
}
impl ClipEnvelope {
    pub(crate) fn gain(&self, frame: u32) -> f32 {
        if frame < self.fade_in {
            return self
                .fade_in_curve
                .gain(frame as f32 / self.fade_in.saturating_sub(1).max(1) as f32);
        }
        if frame >= self.frames - self.fade_out {
            return self.fade_out_curve.gain(
                (self.frames - 1 - frame) as f32 / self.fade_out.saturating_sub(1).max(1) as f32,
            );
        }
        1.
    }
}

impl Track {
    pub(crate) fn is_audio(&self) -> bool {
        self.audio.is_some() || self.clips.is_some()
    }
    pub(crate) fn has_audio(&self) -> bool {
        self.audio.is_some() || self.clips.as_ref().is_some_and(|clips| !clips.is_empty())
    }
    pub(crate) fn media_clips(&self) -> Cow<'_, [Clip]> {
        if let Some(clips) = &self.clips {
            return Cow::Borrowed(clips);
        }
        if let Some(audio) = &self.audio {
            return Cow::Owned(vec![Clip::new("legacy".into(), audio, &self.name)]);
        }
        Cow::Borrowed(&[])
    }
    fn legacy_clip_id(&self) -> String {
        format!("clip_{:x}", Sha256::digest(&self.id))
    }
    pub(crate) fn promote_clips(&mut self) -> Result<&mut Vec<Clip>> {
        if let Some(legacy) = self.audio.take() {
            self.clips = Some(vec![Clip::new(self.legacy_clip_id(), &legacy, &self.name)]);
        }
        self.clips
            .as_mut()
            .context("Destination must be an audio track")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Edit {
    pub track_id: String,
    pub clip_id: String,
    pub action: Action,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Split {
        at: f64,
    },
    Move {
        start: f64,
        to_track_id: Option<String>,
        #[serde(default)]
        allow_overlap: bool,
    },
    Copy {
        start: f64,
        to_track_id: Option<String>,
        #[serde(default)]
        allow_overlap: bool,
    },
    Trim {
        first: i32,
        end: i32,
        #[serde(default)]
        allow_overlap: bool,
    },
    Set {
        name: Option<String>,
        gain_db: Option<f32>,
        fade_in: Option<u32>,
        fade_out: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fade_in_beats: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fade_out_beats: Option<f64>,
    },
    Remove,
}

fn track_clip(track: &Track, clip: &Clip, clock: &Timeline) -> Value {
    json!({"track_id":track.id,"clip":clip,"beat_range":clip.beat_span(clock),"start_frame":clip.start_frame(clock),
        "end_frame":clip.end_frame(clock),"fade_edges":clip.fade_edges(clock)})
}

pub(crate) fn focus_regions(summary: &Value, committed: bool) -> impl Iterator<Item = Value> + '_ {
    let transition = summary["transition"]["beat_range"]
        .as_array()
        .or_else(|| summary["warp"]["beat_range"].as_array())
        .map(|range| json!({"track_id":summary["before"][0]["track_id"],"beat_range":range}));
    let has_transition = transition.is_some();
    transition.into_iter().chain(
        summary["after"]
            .as_array()
            .filter(|items| committed && !items.is_empty())
            .or_else(|| summary["before"].as_array())
            .into_iter()
            .flatten()
            .filter(move |_| !has_transition)
            .map(|item| json!({"track_id":item["track_id"],"beat_range":item["beat_range"]})),
    )
}

#[derive(Clone, Debug, Deserialize, Serialize, Hash, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClipRef {
    pub track_id: String,
    pub clip_id: String,
}

impl ClipRef {
    pub(crate) fn resolve<'a>(&self, p: &'a Project) -> Result<(&'a Track, Clip)> {
        let track = p
            .tracks
            .iter()
            .find(|track| track.id == self.track_id)
            .context("Unknown audio track")?;
        let clip = track
            .media_clips()
            .iter()
            .find(|clip| clip.id == self.clip_id)
            .cloned()
            .context("Unknown audio clip")?;
        Ok((track, clip))
    }
}

pub(crate) fn validate_refs(p: &Project, refs: &[ClipRef]) -> Result<()> {
    ensure!(refs.len() <= 1024, "Too many selected clips");
    let mut seen = HashSet::with_capacity(refs.len());
    for reference in refs {
        ensure!(seen.insert(reference), "A selected clip was repeated");
        reference.resolve(p)?;
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum GroupAction {
    Split {
        at: f64,
    },
    Move {
        delta: f64,
        #[serde(default)]
        lane_offset: i32,
    },
    Copy {
        delta: f64,
        #[serde(default)]
        lane_offset: i32,
    },
    Gain {
        delta_db: f32,
    },
    Slip {
        source_delta_frames: i32,
    },
    Crossfade {
        length_frames: Option<u32>,
        curve: Option<FadeCurve>,
    },
    Tempo {
        #[serde(default)]
        fixed: bool,
        beats: Option<f64>,
        source_bpm: Option<f64>,
        #[serde(default)]
        allow_overlap: bool,
    },
    Warp {
        beats: Option<f64>,
        points: Vec<WarpPoint>,
        #[serde(default)]
        allow_overlap: bool,
    },
    Extract {
        beat_range: [f64; 2],
        to_track_id: Option<String>,
    },
    Remove,
}

struct Crossfade {
    clips: [Clip; 2],
    frames: [i64; 2],
    curve: FadeCurve,
    time_basis: &'static str,
}

fn plan_crossfade(
    root: &Path,
    project: &Project,
    refs: &[ClipRef],
    clock: &Timeline,
    length: Option<u32>,
    curve: Option<FadeCurve>,
) -> Result<Crossfade> {
    let [first, second] = refs else {
        anyhow::bail!("请选择同一音轨上的两个音频片段");
    };
    ensure!(first.track_id == second.track_id, "交叉淡化需要同一音轨");
    let (track, one) = first.resolve(project)?;
    let (_, two) = second.resolve(project)?;
    let mut ordered = [one, two];
    ordered.sort_by_key(|clip| clip.start_frame(clock));
    let [mut left, mut right] = ordered;
    let follows_tempo = left.beat_length.is_some() || right.beat_length.is_some();
    let [left_start, left_end] = [left.start_frame(clock), left.end_frame(clock)];
    let [right_start, right_end] = [right.start_frame(clock), right.end_frame(clock)];
    ensure!(
        left_start < right_start && left_end < right_end,
        "片段需要前后排列，不能完全包含"
    );
    ensure!(left_end >= right_start, "片段之间有空隙，请先对齐或重叠");
    let mut frames = [right_start, left_end];
    if left_end == right_start {
        ensure!(
            !follows_tempo,
            "跟速交叉淡化需要已有重叠，请先把两个片段叠放"
        );
        let asset_end = crate::audio::asset_reader(root, &left.asset)?.duration();
        let tail = asset_end
            .saturating_sub(left.offset + left.frames)
            .min(right.frames - right.fade_out)
            .min(right.frames.saturating_sub(1));
        let head = right
            .offset
            .min(left.frames - left.fade_in)
            .min(left.frames.saturating_sub(1));
        let capacity = head + tail;
        let fade = length.unwrap_or(capacity.min(RATE / 50));
        ensure!(
            fade <= capacity,
            "源素材余量最多 {:.1} ms，缩短淡化或先重叠片段",
            f64::from(capacity) / 48.
        );
        ensure!(fade >= 2, "源素材没有足够余量，请先重叠片段");
        let before = (fade / 2).clamp(fade.saturating_sub(tail), head.min(fade));
        let after = fade - before;
        frames = [right_start - i64::from(before), left_end + i64::from(after)];
        left.frames += after;
        right.offset -= before;
        right.frames += before;
        if before > 0 {
            right.start = clock.beat(frames[0] as f64 / f64::from(RATE));
        }
    } else {
        let overlap = (left_end - right_start) as u32;
        ensure!(
            length.is_none_or(|length| length == overlap),
            "已有重叠时按完整重叠区淡化"
        );
    }
    let overlap = (frames[1] - frames[0]) as u32;
    ensure!(overlap >= 2, "交叉淡化至少需要两个采样帧");
    ensure!(
        !track.media_clips().iter().any(|other| other.id != left.id
            && other.id != right.id
            && other.start_frame(clock) < frames[1]
            && frames[0] < other.end_frame(clock)),
        "交叉淡化区有第三个片段，请先调整选区"
    );
    let curve = curve.unwrap_or_else(|| {
        let aligned = !follows_tempo
            && left.asset == right.asset
            && i64::from(left.offset) - left.start_frame(clock)
                == i64::from(right.offset) - right.start_frame(clock);
        if aligned {
            FadeCurve::Linear
        } else {
            FadeCurve::EqualPower
        }
    });
    if follows_tempo {
        let overlap_start = right.start;
        let overlap_end = left.beat_span(clock)[1];
        if left.beat_length.is_some() {
            left.fade_out = 0;
            left.fade_out_beats = Some(overlap_end - overlap_start);
        } else {
            left.fade_out = overlap;
        }
        if right.beat_length.is_some() {
            right.fade_in = 0;
            right.fade_in_beats = Some(overlap_end - overlap_start);
        } else {
            right.fade_in = overlap;
        }
    } else {
        left.fade_out = overlap;
        right.fade_in = overlap;
    }
    left.fade_out_curve = curve;
    right.fade_in_curve = curve;
    Ok(Crossfade {
        time_basis: match (left.beat_length.is_some(), right.beat_length.is_some()) {
            (true, true) => "beats",
            (false, false) => "source_frames",
            _ => "mixed",
        },
        clips: [left, right],
        frames,
        curve,
    })
}

pub(crate) fn plan_group(
    root: &Path,
    p: &Project,
    refs: &[ClipRef],
    action: &GroupAction,
) -> Result<(Project, Value)> {
    plan_group_window(root, p, refs, action, None)
}

pub(crate) fn plan_group_window(
    root: &Path,
    p: &Project,
    refs: &[ClipRef],
    action: &GroupAction,
    window: Option<[f64; 2]>,
) -> Result<(Project, Value)> {
    ensure!(!refs.is_empty(), "Select audio clips first");
    ensure!(
        window.is_none()
            || matches!(
                action,
                GroupAction::Move { .. } | GroupAction::Copy { .. } | GroupAction::Remove
            ),
        "Clip windows are supported for copy, move and remove"
    );
    validate_refs(p, refs)?;
    let mut identity = json!(["clip_group", p.id, p.revision, refs, action]);
    if let Some(window) = window {
        identity.as_array_mut().unwrap().push(json!(window));
    }
    let preview_id = format!("{:x}", Sha256::digest(serde_json::to_vec(&identity)?));
    let clock = p.timeline();
    if let GroupAction::Extract {
        beat_range,
        to_track_id,
    } = action
    {
        let [reference] = refs else {
            anyhow::bail!("请选择一遍录音的片段")
        };
        let (track, mut clip) = reference.resolve(p)?;
        let before = track_clip(track, &clip, &clock);
        let [first, end] = clip.recording_window(&clock, *beat_range)?;
        ensure!(
            first >= clip.fade_in || first == 0 && end >= clip.fade_in,
            "选段边界在原录音淡入内"
        );
        let fade_out_start = clip.frames - clip.fade_out;
        ensure!(
            end <= fade_out_start || end == clip.frames && first <= fade_out_start,
            "选段边界在原录音淡出内"
        );
        if first > 0 {
            clip.fade_in = 0;
        }
        if end < clip.frames {
            clip.fade_out = 0;
        }
        clip.id = format!("clip_{preview_id}");
        clip.start = beat_range[0];
        clip.offset += first;
        clip.frames = end - first;
        clip.reset_empty_fades();
        let mut candidate = p.clone();
        candidate.enable_clip_format();
        let target = if let Some(id) = to_track_id {
            ensure!(id != &track.id, "请选择独立的拼接轨");
            candidate.track_mut(id)?
        } else {
            let mut target = Track::new("录音拼接");
            target.id = format!("t_{}", &preview_id[..32]);
            target.clips = Some(Vec::new());
            candidate.tracks.push(target);
            candidate.tracks.last_mut().unwrap()
        };
        let clips = target.promote_clips()?;
        ensure!(
            !clips.iter().any(|item| item.overlaps(&clip, &clock)),
            "CLIP_OVERLAP: 目标录音拼接轨在这段已有片段"
        );
        clips.push(clip.clone());
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        let after = track_clip(target, &clip, &clock);
        candidate.upgrade_feature_format();
        candidate.validate()?;
        return Ok((
            candidate,
            json!({"project_id":p.id,"revision":p.revision,"clips":refs,
            "preview_id":preview_id,"before":[before],"after":[after],"count":1,"sample_rate":RATE,
            "extracted":true,"transition":null,"warp":null}),
        ));
    }
    if matches!(action, GroupAction::Warp { .. }) {
        ensure!(refs.len() == 1, "一次为一个片段编辑对拍点");
    }
    let crossfade = match *action {
        GroupAction::Crossfade {
            length_frames,
            curve,
        } => Some(plan_crossfade(root, p, refs, &clock, length_frames, curve)?),
        _ => None,
    };
    let lane_ids = p
        .tracks
        .iter()
        .filter(|track| track.is_audio())
        .map(|track| track.id.as_str())
        .collect::<Vec<_>>();
    let mut candidate = p.clone();
    candidate.enable_clip_format();
    let mut before = Vec::with_capacity(refs.len());
    let mut placements = Vec::with_capacity(refs.len());
    let mut cuts = Vec::new();
    let copy = matches!(action, GroupAction::Copy { .. });
    for (index, reference) in refs.iter().enumerate() {
        let (track, mut clip) = reference.resolve(p)?;
        let canonical = if track.audio.is_some() {
            track.legacy_clip_id()
        } else {
            clip.id.clone()
        };
        let mut remainders = Vec::new();
        let mut partial = false;
        if let Some([left, right]) = window {
            let origin = clip.start_frame(&clock);
            let end = clip.end_frame(&clock);
            let cut_frame = |beat| (clock.seconds(beat) * f64::from(RATE)).round() as i64;
            let edges = [cut_frame(left).max(origin), cut_frame(right).min(end)];
            ensure!(
                edges[0] < edges[1],
                "Selected clip does not intersect the audio window"
            );
            if edges != [origin, end] {
                ensure!(
                    clip.beat_length.is_none(),
                    "RANGE_CUTS_STRETCHED_CLIP: {}，跟速与对拍片段需要完整框入",
                    clip.name
                );
                let [first, last] = edges.map(|edge| (edge - origin) as u32);
                let start = if first == 0 { clip.start } else { left };
                let slice = clip.fixed_slice([first, last], start)?;
                if !copy {
                    if first > 0 {
                        remainders.push(clip.fixed_slice([0, first], clip.start)?);
                    }
                    if last < clip.frames {
                        let mut tail = clip.fixed_slice([last, clip.frames], right)?;
                        if first > 0 {
                            tail.id = format!(
                                "clip_{:x}",
                                Sha256::digest(format!("{preview_id}/{index}/tail"))
                            );
                        }
                        remainders.push(tail);
                    }
                }
                cuts.push(json!({"track_id":track.id,"clip_id":reference.clip_id,
                    "project_frames":edges,"source_frames":[slice.offset,slice.offset+slice.frames],
                    "remainder_count":remainders.len()}));
                clip = slice;
                partial = true;
            }
        }
        let original = clip.clone();
        before.push(track_clip(track, &clip, &clock));
        let source = candidate.track_mut(&track.id)?.promote_clips()?;
        if !copy {
            source.retain(|other| other.id != canonical);
            for mut remainder in remainders {
                if remainder.id == reference.clip_id {
                    remainder.id.clone_from(&canonical);
                }
                source.push(remainder);
            }
        }
        clip.id = if copy || partial {
            format!("clip_{:x}", Sha256::digest(format!("{preview_id}/{index}")))
        } else {
            canonical
        };
        let mut target = track.id.as_str();
        match action {
            GroupAction::Extract { .. } => unreachable!(),
            GroupAction::Split { at } => {
                let right_id = format!(
                    "clip_{:x}",
                    Sha256::digest(format!("{preview_id}/{index}/right"))
                );
                for half in clip.split_at(&clock, *at, right_id)? {
                    half.validate(&clock)?;
                    placements.push((target, half, original.clone()));
                }
                continue;
            }
            GroupAction::Remove => continue,
            GroupAction::Move { delta, lane_offset } | GroupAction::Copy { delta, lane_offset } => {
                let lane = lane_ids
                    .iter()
                    .position(|id| *id == target)
                    .context("Unknown source audio lane")?;
                target = lane_ids
                    .get(
                        lane.checked_add_signed(*lane_offset as isize)
                            .context("CLIP_LANE_OUT_OF_RANGE")?,
                    )
                    .context("CLIP_LANE_OUT_OF_RANGE")?;
                clip.start += delta;
            }
            GroupAction::Gain { delta_db } => clip.gain_db += delta_db,
            GroupAction::Slip {
                source_delta_frames,
            } => {
                ensure!(*source_delta_frames != 0, "滑移量不能为零");
                let source_frames = crate::audio::asset_reader(root, &clip.asset)?.duration();
                let new_offset = i64::from(clip.offset) + i64::from(*source_delta_frames);
                ensure!(
                    new_offset >= 0
                        && new_offset + i64::from(clip.frames) <= i64::from(source_frames),
                    "SLIP_OUT_OF_SOURCE: {}，可滑移 {:.3} 至 {:.3} ms",
                    clip.name,
                    -f64::from(clip.offset) / 48.,
                    (f64::from(source_frames) - f64::from(clip.offset + clip.frames)) / 48.
                );
                clip.offset = new_offset as u32;
            }
            GroupAction::Tempo {
                fixed,
                beats,
                source_bpm,
                ..
            } => {
                ensure!(
                    u8::from(*fixed) + u8::from(beats.is_some()) + u8::from(source_bpm.is_some())
                        <= 1,
                    "请选择固定秒数、目标拍数或源 BPM 之一"
                );
                if let Some(bpm) = source_bpm {
                    ensure!(bpm.is_finite() && *bpm > 0., "源 BPM 需要为正数");
                }
                let new_length = if *fixed {
                    None
                } else {
                    Some(
                        beats
                            .or_else(|| {
                                source_bpm
                                    .map(|bpm| f64::from(clip.frames) / f64::from(RATE) * bpm / 60.)
                            })
                            .unwrap_or_else(|| {
                                clip.beat_length
                                    .unwrap_or_else(|| clip.beat_span(&clock)[1] - clip.start)
                            }),
                    )
                };
                if let (Some(old), Some(new)) = (clip.beat_length, new_length) {
                    for point in &mut clip.warp_points {
                        point.beat *= new / old;
                    }
                } else if new_length.is_none() {
                    clip.update_beat_fades(&clock, None);
                    clip.warp_points.clear();
                }
                if new_length.is_some() {
                    clip.update_beat_fades(&clock, new_length);
                }
                clip.beat_length = new_length;
                clip.reset_empty_fades();
            }
            GroupAction::Warp { beats, points, .. } => {
                let new_length = Some(
                    beats
                        .or(clip.beat_length)
                        .unwrap_or_else(|| clip.beat_span(&clock)[1] - clip.start),
                );
                clip.update_beat_fades(&clock, new_length);
                clip.beat_length = new_length;
                clip.warp_points.clone_from(points);
            }
            GroupAction::Crossfade { .. } => {
                clip = crossfade
                    .as_ref()
                    .unwrap()
                    .clips
                    .iter()
                    .find(|edited| edited.id == clip.id)
                    .unwrap()
                    .clone();
            }
        }
        clip.validate(&clock)?;
        placements.push((target, clip, original));
    }
    let tempo_edit = matches!(
        action,
        GroupAction::Tempo {
            allow_overlap: false,
            ..
        } | GroupAction::Warp {
            allow_overlap: false,
            ..
        }
    );
    if tempo_edit || matches!(action, GroupAction::Move { .. } | GroupAction::Copy { .. }) {
        for (index, (target, clip, old)) in placements.iter().enumerate() {
            let track = candidate.track_mut(target)?;
            ensure!(
                !track
                    .promote_clips()?
                    .iter()
                    .any(|other| clip.overlaps(other, &clock)
                        && (!tempo_edit || !old.overlaps(other, &clock))),
                "CLIP_OVERLAP: a destination overlaps an existing clip"
            );
            ensure!(
                !placements[..index]
                    .iter()
                    .any(|(other_track, other, other_old)| *other_track == *target
                        && clip.overlaps(other, &clock)
                        && !old.overlaps(other_old, &clock)),
                "CLIP_OVERLAP: a tempo change would create overlap within the moved group"
            );
        }
    }
    let mut after = Vec::with_capacity(placements.len());
    let mut modified = HashSet::new();
    if window.is_some() {
        modified.extend(refs.iter().map(|item| item.track_id.as_str()));
    }
    for (target, clip, _) in placements {
        let track = candidate.track_mut(target)?;
        after.push(track_clip(track, &clip, &clock));
        track.promote_clips()?.push(clip);
        modified.insert(target);
    }
    for track in modified {
        candidate
            .track_mut(track)?
            .promote_clips()?
            .sort_by(|left, right| left.start.total_cmp(&right.start));
    }
    candidate.upgrade_feature_format();
    candidate.validate()?;
    let transition = crossfade.map(|fade| {
        json!({"frame_range":fade.frames,
        "beat_range":fade.frames.map(|frame|clock.beat(frame as f64/f64::from(RATE))),
        "curve":fade.curve,"time_basis":fade.time_basis})
    });
    let warp = if matches!(action, GroupAction::Warp { .. }) {
        let reference = ClipRef {
            track_id: refs[0].track_id.clone(),
            clip_id: after[0]["clip"]["id"].as_str().unwrap().into(),
        };
        let (_, clip) = reference.resolve(&candidate)?;
        let marks = clip
            .warp_points
            .iter()
            .map(|point| {
                json!({"frame":point.frame,
            "beat":clip.start+point.beat,"render_frame":clip.frame_at_source(&clock,point.frame)})
            })
            .collect::<Vec<_>>();
        let (_, old) = refs[0].resolve(p)?;
        let focus = clip
            .warp_points
            .iter()
            .find(|point| !old.warp_points.contains(point))
            .or_else(|| {
                old.warp_points
                    .iter()
                    .find(|point| !clip.warp_points.contains(point))
            })
            .map(|point| point.beat);
        let span = clip.beat_span(&clock);
        let range = focus.map_or(span, |beat| {
            [
                (clip.start + beat - 0.125).max(span[0]),
                (clip.start + beat + 0.125).min(span[1]),
            ]
        });
        Some(json!({"anchors":marks,"beat_range":range}))
    } else {
        None
    };
    Ok((
        candidate,
        json!({"project_id":p.id,"revision":p.revision,"clips":refs,"preview_id":preview_id,
        "before":before,"after":after,"count":refs.len(),"sample_rate":RATE,"transition":transition,"warp":warp,
        "cuts":cuts}),
    ))
}

pub(crate) fn query(p: &Project) -> Value {
    let clock = p.timeline();
    let tracks: Vec<_> = p
        .tracks
        .iter()
        .filter(|track| track.is_audio())
        .map(|track| {
            let clips: Vec<_> = track
                .media_clips()
                .iter()
                .map(|clip| track_clip(track, clip, &clock))
                .collect();
            json!({"track_id":track.id,"name":track.name,"clips":clips})
        })
        .collect();
    json!({"project_id":p.id,"revision":p.revision,"sample_rate":RATE,"tracks":tracks})
}

pub(crate) fn plan(root: &Path, p: &Project, edit: &Edit) -> Result<(Project, Value)> {
    let track = p
        .tracks
        .iter()
        .find(|track| track.id == edit.track_id)
        .context("Unknown source track")?;
    let originals = track.media_clips();
    let original = originals
        .iter()
        .find(|clip| clip.id == edit.clip_id)
        .context("Unknown clip")?;
    let clock = p.timeline();
    let preview_id = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!([p.id, p.revision, edit]))?)
    );
    let new_id = format!("clip_{preview_id}");
    let before = track_clip(track, original, &clock);
    let mut candidate = p.clone();
    candidate.enable_clip_format();
    let source = candidate.track_mut(&edit.track_id)?;
    let canonical_id = if source.audio.is_some() {
        source.legacy_clip_id()
    } else {
        edit.clip_id.clone()
    };
    let clips = source.promote_clips()?;
    let index = clips
        .iter()
        .position(|clip| clip.id == canonical_id)
        .context("Unknown clip after migration")?;
    let mut clip = clips.remove(index);
    let mut placement = vec![];
    let mut fades_reset = false;
    let action = match &edit.action {
        Action::Split { at } => {
            placement.extend(
                clip.split_at(&clock, *at, new_id)?
                    .into_iter()
                    .map(|half| (edit.track_id.clone(), half, false)),
            );
            "split"
        }
        Action::Move {
            start,
            to_track_id,
            allow_overlap,
        }
        | Action::Copy {
            start,
            to_track_id,
            allow_overlap,
        } => {
            let copy = matches!(edit.action, Action::Copy { .. });
            if copy {
                clips.push(clip.clone());
                clip.id = new_id;
            }
            clip.start = *start;
            placement.push((
                to_track_id.clone().unwrap_or_else(|| edit.track_id.clone()),
                clip,
                !allow_overlap,
            ));
            if copy {
                "copy"
            } else {
                "move"
            }
        }
        Action::Trim {
            first,
            end,
            allow_overlap,
        } => {
            let [first, end] = [*first, *end].map(i64::from);
            let old_frames = i64::from(clip.frames);
            ensure!(
                first < end && first < old_frames && end > 0,
                "Trim must retain part of the current clip"
            );
            let extends = first < 0 || end > old_frames;
            if extends {
                let duration = crate::audio::asset_reader(root, &clip.asset)?.duration();
                ensure!(
                    i64::from(clip.offset) + first >= 0
                        && i64::from(clip.offset) + end <= i64::from(duration),
                    "TRIM_OUT_OF_SOURCE: {}，源音频余量不足",
                    clip.name
                );
            }
            let [fade_in_end, fade_out_start] = clip.fade_edges(&clock);
            let old_start = clip.start_frame(&clock);
            let old_end = clip.end_frame(&clock);
            let [new_start, new_end] = if let Some(beats) = clip.beat_length {
                let boundaries =
                    [first, end].map(|frame| clip.start + clip.relative_beat_at_edge(frame, beats));
                ensure!(
                    boundaries[0] >= -1e-10,
                    "TRIM_BEFORE_START: 片段不能早于工程起点"
                );
                boundaries.map(|beat| (clock.seconds(beat) * f64::from(RATE)).round() as i64)
            } else {
                ensure!(
                    old_start + first >= 0,
                    "TRIM_BEFORE_START: 片段不能早于工程起点"
                );
                if first != 0 {
                    clip.start = clock.beat((old_start + first) as f64 / f64::from(RATE));
                }
                [old_start + first, old_start + end]
            };
            if extends && !allow_overlap {
                ensure!(
                    !originals
                        .iter()
                        .filter(|other| other.id != original.id)
                        .any(|other| {
                            let [a, b] = [other.start_frame(&clock), other.end_frame(&clock)];
                            (new_start < old_start && new_start < b && a < old_start)
                                || (new_end > old_end && old_end < b && a < new_end)
                        }),
                    "CLIP_OVERLAP: 延长处已有片段"
                );
            }
            if (clip.fade_in > 0 || clip.fade_in_beats.is_some())
                && (first > 0 || new_end < fade_in_end)
            {
                clip.clear_fade_in();
                fades_reset = true;
            }
            if (clip.fade_out > 0 || clip.fade_out_beats.is_some())
                && (end < old_frames || new_start > fade_out_start)
            {
                clip.clear_fade_out();
                fades_reset = true;
            }
            clip.offset = (i64::from(clip.offset) + first) as u32;
            clip.slice_warp(first, end);
            clip.frames = (end - first) as u32;
            if clip.fade_in > clip.frames
                || clip
                    .fade_in_beats
                    .is_some_and(|fade| fade > clip.beat_length.unwrap_or_default())
            {
                clip.clear_fade_in();
                fades_reset = true;
            }
            if clip.fade_out > clip.frames - clip.fade_in
                || clip
                    .fade_out_beats
                    .is_some_and(|fade| fade > clip.beat_length.unwrap_or_default())
            {
                clip.clear_fade_out();
                fades_reset = true;
            }
            placement.push((edit.track_id.clone(), clip, false));
            "trim"
        }
        Action::Set {
            name,
            gain_db,
            fade_in,
            fade_out,
            fade_in_beats,
            fade_out_beats,
        } => {
            ensure!(
                name.is_some()
                    || gain_db.is_some()
                    || fade_in.is_some()
                    || fade_out.is_some()
                    || fade_in_beats.is_some()
                    || fade_out_beats.is_some(),
                "Provide at least one clip setting"
            );
            ensure!(
                !(fade_in.is_some() && fade_in_beats.is_some())
                    && !(fade_out.is_some() && fade_out_beats.is_some()),
                "每个淡化只需提供帧数或拍数"
            );
            if let Some(name) = name {
                clip.name = name.clone();
            }
            if let Some(gain) = gain_db {
                clip.gain_db = *gain;
            }
            if let Some(fade) = fade_in {
                clip.fade_in = *fade;
                clip.fade_in_beats = None;
            }
            if let Some(fade) = fade_out {
                clip.fade_out = *fade;
                clip.fade_out_beats = None;
            }
            if let Some(beats) = fade_in_beats {
                clip.fade_in = 0;
                clip.fade_in_beats = (*beats != 0.).then_some(*beats);
            }
            if let Some(beats) = fade_out_beats {
                clip.fade_out = 0;
                clip.fade_out_beats = (*beats != 0.).then_some(*beats);
            }
            placement.push((edit.track_id.clone(), clip, false));
            "set"
        }
        Action::Remove => "remove",
    };
    let mut after = vec![];
    for (id, mut clip, check_overlap) in placement {
        clip.reset_empty_fades();
        clip.validate(&clock)?;
        let track = candidate.track_mut(&id)?;
        let clips = track.promote_clips()?;
        ensure!(!check_overlap||!clips.iter().any(|other|clip.overlaps(other,&clock)), "CLIP_OVERLAP: destination overlaps a clip. Select an empty place or explicitly allow overlap");
        clips.push(clip.clone());
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        after.push(track_clip(track, &clip, &clock));
    }
    candidate.upgrade_feature_format();
    candidate.validate()?;
    Ok((
        candidate,
        json!({"project_id":p.id,"revision":p.revision,"preview_id":preview_id,"action":action,"before":before,"after":after,"fades_reset":fades_reset,"sample_rate":RATE}),
    ))
}
