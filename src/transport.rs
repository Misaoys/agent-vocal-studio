use crate::monitor_stream::{self, Pages, Stream, MAX_FRAMES, RATE as SOURCE_RATE};
use crate::resample::playback::{phase_step, Kernel, Window};
use anyhow::{bail, ensure, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, SampleFormat, SizedSample,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        Arc, Mutex,
    },
};

pub(crate) fn read_mix(path: &Path) -> Result<Vec<[f32; 2]>> {
    let mut wav = monitor_stream::open_mix(path)?;
    let count = wav.duration() as usize;
    monitor_stream::read_frames(&mut wav, count)
}

enum RenderSource {
    Memory(Vec<[f32; 2]>),
    Paged(Stream),
}

impl RenderSource {
    fn open(path: &Path, position: f64, bounds: Option<(f64, f64)>) -> Result<Self> {
        let mut wav = monitor_stream::open_mix(path)?;
        let len = wav.duration() as usize;
        if len <= monitor_stream::STREAM_AFTER {
            return Ok(Self::Memory(monitor_stream::read_frames(&mut wav, len)?));
        }
        let mut first = source_frame(position) as usize;
        let head = bounds.and_then(|(a, b)| {
            let [a, b] = [
                source_frame(a) as usize,
                (source_frame(b) as usize).min(len),
            ];
            if a >= b {
                return None;
            }
            if !(a..b).contains(&first) {
                first = a;
            }
            Some((a, b))
        });
        if first >= len {
            first = 0;
        }
        Ok(Self::Paged(Stream::open(wav, first, head)?))
    }

    fn len(&self) -> u64 {
        match self {
            Self::Memory(frames) => frames.len() as u64,
            Self::Paged(stream) => stream.len() as u64,
        }
    }
}

fn source_frame(seconds: f64) -> u64 {
    (seconds * SOURCE_RATE as f64).round() as u64
}

struct Playback {
    frames: Vec<[f32; 2]>,
    streamed: Option<Stream>,
    buffer_wait_frames: u64,
    buffering: bool,
    // Position expressed as source frames multiplied by the output sample rate.
    position: u64,
    output_rate: u64,
    resample: Option<Kernel>,
    loop_frames: Option<(u64, u64)>,
    stop_range: Option<(u64, u64)>,
    playing: bool,
    ended: bool,
    audio_frames: u64,
    wraps: u64,
    peak: [f32; 2],
    mixer: Option<crate::live_mix::Mixer>,
    mix_fade: u32,
    live_frames: u64,
    mix_changes: u64,
    audition: Option<Comparison>,
    capture: Option<(Arc<crate::capture_clock::CaptureClock>, u64)>,
    largest_buffer: usize,
    click: Option<crate::metronome::Click>,
    silent_frames: Option<u64>,
    prelude_position: i64,
}

struct Comparison {
    manifest: crate::audition::Manifest,
    clocks: [crate::timeline::Timeline; 2],
    variant: usize,
    alternate: Vec<[f32; 2]>,
    old_position: u64,
    fade: u32,
}

impl Comparison {
    fn beat(&self, position: u64, rate: u64) -> f64 {
        let start = self.manifest.frame_ranges[self.variant][0];
        let seconds = (start as f64 + position as f64 / rate as f64) / SOURCE_RATE as f64;
        self.clocks[self.variant].beat(seconds).clamp(
            self.manifest.ranges()[self.variant][0],
            self.manifest.ranges()[self.variant][1],
        )
    }
    fn position(&self, variant: usize, beat: f64, rate: u64) -> u64 {
        let [start, end] = self.manifest.frame_ranges[variant];
        let relative = self.clocks[variant].seconds(beat) * SOURCE_RATE as f64 - start as f64;
        let step = phase_step(rate);
        let position = (relative * rate as f64 / step as f64).round() as u64 * step;
        position.min((end - start) as u64 * rate - step)
    }

    fn switch_position(&self, variant: usize, position: u64, rate: u64) -> u64 {
        let mut beat = self.beat(position, rate);
        if self.manifest.saved.is_some() {
            let ranges = self.manifest.ranges();
            let [from, until] = ranges[self.variant];
            let [start, end] = ranges[variant];
            beat = start + (beat - from) / (until - from) * (end - start);
        }
        self.position(variant, beat, rate)
    }
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            frames: Vec::new(),
            streamed: None,
            buffer_wait_frames: 0,
            buffering: false,
            position: 0,
            output_rate: SOURCE_RATE,
            resample: None,
            loop_frames: None,
            stop_range: None,
            playing: false,
            ended: false,
            audio_frames: 0,
            wraps: 0,
            peak: [0.; 2],
            mixer: None,
            mix_fade: 0,
            live_frames: 0,
            mix_changes: 0,
            audition: None,
            capture: None,
            largest_buffer: 0,
            click: None,
            silent_frames: None,
            prelude_position: 0,
        }
    }
}

impl Playback {
    fn length(&self) -> u64 {
        if let Some(mixer) = &self.mixer {
            return mixer.frame_count() as u64;
        }
        self.silent_frames.unwrap_or_else(|| {
            self.streamed
                .as_ref()
                .map_or(self.frames.len() as u64, |source| source.len() as u64)
        })
    }

    fn replace_render(&mut self, source: RenderSource) -> (Vec<[f32; 2]>, Option<Stream>) {
        self.buffer_wait_frames = 0;
        self.buffering = false;
        let (frames, streamed) = match source {
            RenderSource::Memory(frames) => (frames, None),
            RenderSource::Paged(stream) => (Vec::new(), Some(stream)),
        };
        (
            std::mem::replace(&mut self.frames, frames),
            std::mem::replace(&mut self.streamed, streamed),
        )
    }

    fn window(&self) -> (usize, Option<(usize, usize)>) {
        let position = self.position / self.output_rate;
        let first = self
            .loop_frames
            .filter(|&(a, b)| !(a..b).contains(&position))
            .map_or_else(
                || self.stop_range.map_or(position, |(a, _)| position.max(a)),
                |(a, _)| a,
            );
        (
            first as usize,
            self.loop_frames.map(|(a, b)| (a as usize, b as usize)),
        )
    }

    fn request_window(&self, render: bool) {
        let (first, repeat) = self.window();
        for source in [
            self.streamed.as_ref().filter(|_| render),
            self.monitor_stream(),
        ]
        .into_iter()
        .flatten()
        {
            source.request(first, repeat);
        }
    }

    fn monitor_stream(&self) -> Option<&Stream> {
        self.mixer.as_ref().and_then(crate::live_mix::Mixer::stream)
    }

    fn stream_error(&self) -> bool {
        ((self.mixer.is_none() || self.mix_fade > 0)
            && self.streamed.as_ref().is_some_and(Stream::failed))
            || self.monitor_stream().is_some_and(Stream::failed)
    }

    fn seek(&mut self, seconds: f64) {
        self.position = source_frame(seconds).min(self.length()) * self.output_rate;
        self.buffering = false;
        self.prelude_position = 0;
        if let Some(click) = &mut self.click {
            click.reset();
        }
        self.ended = false;
        self.peak = [0.; 2];
        if let Some(mixer) = &mut self.mixer {
            mixer.reset();
        }
        if let Some(audition) = &mut self.audition {
            audition.fade = 0;
        }
        self.request_window(true);
    }

    fn set_loop(&mut self, bounds: Option<(f64, f64)>) {
        self.stop_range = None;
        self.loop_frames = bounds.and_then(|(start, end)| {
            let start = source_frame(start);
            let end = source_frame(end).min(self.length());
            (start < end).then_some((start, end))
        });
        self.request_window(true);
    }

    fn next(&mut self, pages: Option<&Pages>, tracks: Option<&Pages>) -> Option<[f32; 2]> {
        if !self.playing {
            return None;
        }
        let rate = self.output_rate;
        if self.prelude_position < 0 {
            let frame = self.prelude_position.div_euclid(rate as i64);
            let tick = self.click.as_mut().map_or(0., |click| click.sample(frame));
            self.prelude_position = (self.prelude_position + SOURCE_RATE as i64).min(0);
            self.audio_frames += 1;
            return Some([tick; 2]);
        }
        let repeating = self.loop_frames.is_some();
        let (start, end) = self
            .loop_frames
            .or(self.stop_range)
            .unwrap_or_else(|| (0, self.length()));
        if self.position < start * rate || (repeating && self.position >= end * rate) {
            self.position = start * rate;
            if let Some(click) = &mut self.click {
                click.seek(start as i64);
            }
        }
        let index = self.position / rate;
        if index >= end {
            self.playing = false;
            self.ended = true;
            return None;
        }
        let window = Window::new(
            self.position,
            rate,
            start as usize..end as usize,
            repeating,
            self.resample.as_ref(),
        );
        let mut sample = [0.; 2];
        if self.mixer.is_none() || self.mix_fade > 0 {
            if self.streamed.is_some() {
                if pages
                    .and_then(|pages| pages.read_inputs(&window, std::slice::from_mut(&mut sample)))
                    .is_none()
                {
                    self.buffering = true;
                    self.buffer_wait_frames += 1;
                    return None;
                }
            } else if self.silent_frames.is_none() {
                sample = window.read(&self.frames);
            }
        }
        self.buffering = false;
        if let Some(mixer) = &mut self.mixer {
            let processed = mixer.sample(&window, tracks);
            if let Some(processed) = processed {
                self.live_frames += 1;
                if self.mix_fade > 0 {
                    let weight = self.mix_fade as f32 / (rate / 125) as f32;
                    self.mix_fade -= 1;
                    sample = [
                        sample[0] * weight + processed[0] * (1. - weight),
                        sample[1] * weight + processed[1] * (1. - weight),
                    ];
                } else {
                    sample = processed;
                }
            } else if self.mix_fade == 0 {
                self.buffering = true;
                self.buffer_wait_frames += 1;
                return None;
            }
        }
        if let Some(audition) = &mut self.audition {
            if audition.fade > 0 {
                let old = &audition.alternate;
                let prior = Window::new(
                    audition.old_position,
                    rate,
                    0..old.len(),
                    true,
                    self.resample.as_ref(),
                )
                .read(old);
                let weight = audition.fade as f32 / (rate / 125).max(1) as f32;
                for (value, prior) in sample.iter_mut().zip(prior) {
                    *value = prior * weight + *value * (1. - weight);
                }
                audition.old_position =
                    (audition.old_position + SOURCE_RATE) % (old.len() as u64 * rate);
                audition.fade -= 1;
            }
        } else if let Some(click) = &mut self.click {
            let tick = click.sample(index as i64);
            // Keep a full-scale monitor sample within range during the short click.
            sample = sample.map(|value| value * (1. - tick.abs()) + tick);
        }
        self.audio_frames += 1;
        self.position += SOURCE_RATE;
        if self.position >= end * rate {
            if repeating {
                let span = (end - start) * rate;
                let offset = self.position - start * rate;
                self.wraps += offset / span;
                self.position = start * rate + offset % span;
                if let Some(click) = &mut self.click {
                    click.seek(start as i64);
                }
            } else {
                self.position = end * rate;
                self.ended = true;
                self.playing = false;
            }
        }
        Some(sample)
    }

    fn fill<T: SizedSample + FromSample<f32>>(
        &mut self,
        output: &mut [T],
        channels: usize,
        callback: cpal::StreamInstant,
    ) {
        let needs_render = self.mixer.is_none() || self.mix_fade > 0;
        if self.stream_error() {
            self.playing = false;
            self.buffering = false;
        }
        // One nonblocking cache acquisition per callback. The reader decodes outside this lock.
        if self.playing {
            self.request_window(needs_render);
        }
        let shared = self
            .streamed
            .as_ref()
            .filter(|_| self.playing && needs_render)
            .map(Stream::pages);
        let pages = shared.as_ref().and_then(|cache| cache.try_lock().ok());
        let dry_shared = self
            .monitor_stream()
            .filter(|_| self.playing)
            .map(Stream::pages);
        let dry = dry_shared.as_ref().and_then(|cache| cache.try_lock().ok());
        let available = output.len() / channels;
        self.largest_buffer = self.largest_buffer.max(available);
        if self.playing {
            if let Some((clock, punch)) = &self.capture {
                // In WASAPI shared mode CPAL reports the writable frames to this callback.
                // The rest of the observed endpoint buffer is the estimated queued audio.
                let padding = (self.largest_buffer - available) as f64 / self.output_rate as f64;
                let delta = (punch.saturating_sub(self.position)
                    + self.prelude_position.unsigned_abs()) as f64
                    / (SOURCE_RATE * self.output_rate) as f64;
                clock.punch(callback, padding + delta);
            }
        }
        let mut peaks = [0_f32; 2];
        for frame in output.chunks_exact_mut(channels) {
            let [left, right] = self
                .next(pages.as_deref(), dry.as_deref())
                .unwrap_or_default();
            if channels == 1 {
                frame[0] = T::from_sample(((left + right) * 0.5).clamp(-1., 1.));
            } else {
                frame[0] = T::from_sample(left.clamp(-1., 1.));
                frame[1] = T::from_sample(right.clamp(-1., 1.));
                frame[2..].fill(T::EQUILIBRIUM);
            }
            peaks[0] = peaks[0].max(left.abs());
            peaks[1] = peaks[1].max(right.abs());
        }
        // Smooth only the meter's release, with approximately 300 ms to reach -20 dB.
        let seconds = output.len() as f32 / channels as f32 / self.output_rate as f32;
        let release = 10_f32.powf(-seconds / 0.3);
        for (previous, peak) in self.peak.iter_mut().zip(peaks) {
            *previous = (*previous * release).max(peak);
        }
        if let Some(mixer) = &mut self.mixer {
            mixer.finish_block(release);
        }
    }
}

#[derive(Serialize)]
struct Status<'a> {
    backend: &'static str,
    device: Option<&'a str>,
    output_device_id: Option<&'a str>,
    output_preference: Option<&'a str>,
    playing: bool,
    ended: bool,
    seconds: f64,
    duration: f64,
    output_rate: Option<u64>,
    sample_rate_conversion: Option<&'static str>,
    output_channels: Option<u16>,
    callbacks: u64,
    contended_callbacks: u64,
    audio_frames: u64,
    loop_frames: Option<(u64, u64)>,
    stop_frame: Option<u64>,
    url: Option<&'a str>,
    loop_wraps: u64,
    peak: [f32; 2],
    live_mix: bool,
    live_frames: u64,
    mix_changes: u64,
    track_peaks: Vec<[f32; 2]>,
    audition_id: Option<String>,
    audition_variant: Option<usize>,
    audition_timeline_seconds: Option<f64>,
    click_pulses: u64,
    last_click_frame: Option<i64>,
    streaming: bool,
    buffering: bool,
    buffered_frames: usize,
    buffer_wait_frames: u64,
    stream_error: bool,
    live_streaming: bool,
    live_buffered_frames: usize,
}

#[derive(Default)]
pub(crate) struct Transport {
    playback: Arc<Mutex<Playback>>,
    fault: Arc<AtomicBool>,
    callbacks: Arc<AtomicU64>,
    contended_callbacks: Arc<AtomicU64>,
    stream: Option<cpal::Stream>,
    device: Option<String>,
    device_id: Option<String>,
    output_preference: Option<String>,
    channels: Option<u16>,
    source: Option<String>,
}

pub(crate) fn seconds(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
        .context("无效的播放时间")
}

pub(crate) fn loop_bounds(value: &Value) -> Result<Option<(f64, f64)>> {
    if value.is_null() {
        return Ok(None);
    }
    let start = seconds(&value["start"])?;
    let end = seconds(&value["end"])?;
    ensure!(end > start, "循环结束必须晚于开始");
    Ok(Some((start, end)))
}

fn selected_output(id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let Some(id) = id else {
        return host
            .default_output_device()
            .context("Windows 没有默认音频输出设备");
    };
    let id = id.parse::<cpal::DeviceId>().context("无效的输出设备标识")?;
    host.output_devices()?
        .find(|device| device.id().is_ok_and(|candidate| candidate == id))
        .context("所选输出设备不可用。请打开音频输出设置。")
}

impl Transport {
    pub(crate) fn with_output_settings(root: &Path) -> Self {
        let parsed = fs::read(root.join("runtime/audio-output.json"))
            .map_err(anyhow::Error::from)
            .and_then(|bytes| serde_json::from_slice::<Option<String>>(&bytes).map_err(Into::into));
        Self {
            output_preference: parsed.ok().flatten(),
            ..Self::default()
        }
    }

    pub(crate) fn output_devices(&self) -> Result<Value> {
        let host = cpal::default_host();
        let default = host.default_output_device();
        let default_name = default
            .as_ref()
            .and_then(|device| device.description().ok())
            .map(|value| value.name().to_owned());
        let default_id = default
            .and_then(|device| device.id().ok())
            .map(|value| value.to_string());
        let mut choices = Vec::new();
        for device in host.output_devices()? {
            let (Ok(description), Ok(id)) = (device.description(), device.id()) else {
                continue;
            };
            choices.push((
                description.name().to_owned(),
                id.to_string(),
                device.default_output_config().ok(),
            ));
        }
        choices.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        let choices = choices
            .into_iter()
            .map(|(name, id, config)| {
                json!({"name":name,"id":id,"selectable":config.is_some(),
                "channels":config.as_ref().map(|config| config.channels()),
                "sample_rate":config.map(|config|config.sample_rate())})
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"attached":true,"backend":"WASAPI","output_preference":self.output_preference,
            "active_output":self.device,"active_output_id":self.device_id,
            "default_output":default_name,"default_output_id":default_id,"outputs":choices}),
        )
    }

    pub(crate) fn set_output_preference(
        &mut self,
        root: &Path,
        name: Option<String>,
    ) -> Result<()> {
        if let Some(name) = name.as_deref() {
            selected_output(Some(name))?
                .default_output_config()
                .context("无法读取所选输出设备的格式")?;
        }
        let target = root.join("runtime/audio-output.json");
        let temporary = root
            .join("runtime")
            .join(format!("{}.tmp", crate::model::uid("audio_output")));
        let saved = fs::write(&temporary, serde_json::to_vec(&name)?)
            .map_err(anyhow::Error::from)
            .and_then(|()| crate::model::replace_windows(&temporary, &target));
        if saved.is_err() {
            let _ = fs::remove_file(temporary);
        }
        saved?;
        self.output_preference = name;
        Ok(())
    }

    pub(crate) fn set_click(&mut self, click: Option<crate::metronome::Click>) {
        let mut state = self.playback.lock().unwrap();
        let previous = std::mem::replace(&mut state.click, click);
        drop(state);
        drop(previous);
    }

    pub(crate) fn is_playing(&self) -> bool {
        self.playback.lock().unwrap().playing && !self.fault.load(Relaxed)
    }

    pub(crate) fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub(crate) fn has_audition(&self, id: Option<&str>) -> bool {
        self.playback
            .lock()
            .unwrap()
            .audition
            .as_ref()
            .is_some_and(|a| id.is_none_or(|id| a.manifest.id == id))
    }

    pub(crate) fn play_audition(
        &mut self,
        manifest: crate::audition::Manifest,
        paths: [std::path::PathBuf; 2],
        variant: usize,
        commit: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        ensure!(variant < 2, "选择 A 或 B 试听");
        let new_pair = if self.has_audition(Some(&manifest.id)) {
            None
        } else {
            let a = read_mix(&paths[0])?;
            let b = read_mix(&paths[1])?;
            let [fa, fb] = manifest.frame_ranges;
            ensure!(
                a.len() == fa[1] - fa[0] && b.len() == fb[1] - fb[0],
                "试听文件长度不匹配"
            );
            let (frames, alternate) = if variant == 0 { (a, b) } else { (b, a) };
            Some((
                frames,
                alternate,
                [manifest.timing[0].clock(), manifest.timing[1].clock()],
            ))
        };
        self.ensure_stream()?;
        commit()?;
        let mut state = self.playback.lock().unwrap();
        let retired_mix = state.mixer.take();
        let retired_stream = state.streamed.take();
        state.silent_frames = None;
        state.buffering = false;
        state.buffer_wait_frames = 0;
        let mut retired_pair = None;
        let mut retired_frames = None;
        state.mix_fade = 0;
        if let Some((frames, alternate, clocks)) = new_pair {
            retired_frames = Some(std::mem::replace(&mut state.frames, frames));
            retired_pair = state.audition.replace(Comparison {
                clocks,
                manifest,
                variant,
                alternate,
                old_position: 0,
                fade: 0,
            });
            state.position = 0;
            state.audio_frames = 0;
            state.wraps = 0;
            state.peak = [0.; 2];
        } else {
            let rate = state.output_rate;
            let Playback {
                frames,
                audition,
                position,
                playing,
                ..
            } = &mut *state;
            let comparison = audition.as_mut().context("A/B 试听已失效")?;
            if comparison.variant != variant {
                let before = frames.len() as u64;
                comparison.old_position = *position % (before * rate);
                comparison.fade = if *playing {
                    (rate / 125).max(1) as u32
                } else {
                    0
                };
                *position = comparison.switch_position(variant, comparison.old_position, rate);
                std::mem::swap(frames, &mut comparison.alternate);
                comparison.variant = variant;
            }
        }
        state.loop_frames = Some((0, state.frames.len() as u64));
        state.stop_range = None;
        state.ended = false;
        state.playing = true;
        self.source = None;
        drop(state);
        drop((retired_mix, retired_pair, retired_frames, retired_stream));
        Ok(())
    }

    pub(crate) fn output_rate(&self) -> u64 {
        self.playback.lock().unwrap().output_rate
    }

    pub(crate) fn monitor_origin(&self) -> (usize, Option<(usize, usize)>) {
        self.playback.lock().unwrap().window()
    }

    pub(crate) fn prepare_live_output(&mut self) -> Result<u64> {
        self.ensure_stream()?;
        Ok(self.output_rate())
    }

    pub(crate) fn play_live(
        &mut self,
        mixer: crate::live_mix::Mixer,
        position: f64,
        bounds: Option<(f64, f64)>,
    ) -> Result<()> {
        self.ensure_stream()?;
        let mut state = self.playback.lock().unwrap();
        ensure!(mixer.rate() == state.output_rate, "试听期间音频设备已变化");
        let retired_render = state.replace_render(RenderSource::Memory(Vec::new()));
        let retired_mixer = state.mixer.replace(mixer);
        let retired_audition = state.audition.take();
        state.silent_frames = None;
        state.mix_fade = 0;
        state.live_frames = 0;
        state.mix_changes = 0;
        state.audio_frames = 0;
        state.wraps = 0;
        state.set_loop(bounds);
        let frame = source_frame(position);
        let position = if let Some((start, end)) = state.loop_frames {
            if !(start..end).contains(&frame) {
                start as f64 / SOURCE_RATE as f64
            } else {
                position
            }
        } else if frame >= state.length() {
            0.
        } else {
            position
        };
        state.seek(position);
        state.playing = true;
        self.source = None;
        drop(state);
        drop((retired_render, retired_mixer, retired_audition));
        Ok(())
    }

    pub(crate) fn begin_live(&mut self, mixer: crate::live_mix::Mixer) -> Result<()> {
        let mut state = self.playback.lock().unwrap();
        ensure!(
            mixer.rate() == state.output_rate && mixer.frame_count() as u64 == state.length(),
            "实时监听与已载入混音的长度或采样率不一致"
        );
        state.mix_fade = (state.output_rate / 125) as u32;
        state.live_frames = 0;
        state.mix_changes = 0;
        let previous = state.mixer.replace(mixer);
        state.request_window(true);
        drop(state);
        drop(previous);
        Ok(())
    }

    pub(crate) fn update_live(&mut self, update: crate::live_mix::Update) -> Result<()> {
        let mut retired = crate::live_mix::Retired::new();
        let mut state = self.playback.lock().unwrap();
        state
            .mixer
            .as_mut()
            .context("实时监听已结束")?
            .update(update, &mut retired);
        state.mix_changes += 1;
        drop(state);
        drop(retired);
        Ok(())
    }

    pub(crate) fn replace_live_sources(
        &mut self,
        replacement: crate::live_mix::Replacement,
        update: Option<crate::live_mix::Update>,
    ) -> Result<()> {
        let mut retired = crate::live_mix::Retired::new();
        let mut state = self.playback.lock().unwrap();
        let position = state.window().0;
        let mixer = state.mixer.as_mut().context("实时监听已结束")?;
        if let Some(update) = update {
            mixer.update(update, &mut retired);
        }
        let previous = mixer.replace_sources(replacement, position);
        state.mix_changes += 1;
        state.request_window(false);
        drop(state);
        drop((previous, retired));
        Ok(())
    }

    pub(crate) fn play(
        &mut self,
        url: &str,
        position: f64,
        bounds: Option<(f64, f64)>,
        repeat: bool,
        resolve_mix: impl FnOnce(&str) -> Result<std::path::PathBuf>,
        commit: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let reuse = self.source.as_deref() == Some(url) && {
            let state = self.playback.lock().unwrap();
            state.streamed.as_ref().map_or(
                state.frames.len() <= monitor_stream::STREAM_AFTER,
                |source| !source.failed(),
            )
        };
        let frames = if reuse {
            None
        } else {
            Some(RenderSource::open(
                &resolve_mix(url)?,
                position,
                if repeat { bounds } else { None },
            )?)
        };
        self.ensure_stream()?;
        let mut state = self.playback.lock().unwrap();
        let length = frames
            .as_ref()
            .map_or_else(|| state.length(), RenderSource::len);
        let range = bounds.map(|(start, end)| (source_frame(start), source_frame(end)));
        if !repeat {
            ensure!(
                range.is_some_and(|(start, end)| start < end && end <= length),
                "PREVIEW_RANGE_OUTSIDE_MIX"
            );
        }
        commit()?;
        let previous_mix = state.mixer.take();
        state.silent_frames = None;
        let previous_audition = state.audition.take();
        state.mix_fade = 0;
        let old = frames.map(|frames| {
            state.audio_frames = 0;
            state.wraps = 0;
            state.replace_render(frames)
        });
        state.set_loop(if repeat { bounds } else { None });
        state.stop_range = range.filter(|_| !repeat);
        state.seek(position);
        if let Some((start, end)) = state.loop_frames {
            let rate = state.output_rate;
            if !(start * rate..end * rate).contains(&state.position) {
                state.position = start * rate;
            }
        } else if state.position == length * state.output_rate {
            state.seek(0.);
        }
        state.playing = true;
        state.request_window(true);
        drop(state);
        drop(previous_mix);
        drop(previous_audition);
        drop(old);
        if self.source.as_deref() != Some(url) {
            self.source = Some(url.to_owned());
        }
        Ok(())
    }

    pub(crate) fn prepare_capture(
        &mut self,
        backing: Option<(&str, &Path)>,
        punch_seconds: f64,
    ) -> Result<()> {
        let frames = match backing {
            Some((url, _))
                if self.source.as_deref() == Some(url)
                    && self.playback.lock().unwrap().streamed.is_none() =>
            {
                None
            }
            Some((_, path)) => Some(read_mix(path)?),
            None => Some(Vec::new()),
        };
        self.ensure_stream()?;
        let mut state = self.playback.lock().unwrap();
        ensure!(!state.playing, "请先停止当前试听");
        let len = if backing.is_some() {
            frames.as_ref().unwrap_or(&state.frames).len() as u64
        } else {
            MAX_FRAMES
        };
        ensure!(source_frame(punch_seconds) < len, "录音入点超出伴奏长度");
        let old_frames = frames.map(|frames| state.replace_render(RenderSource::Memory(frames)));
        state.silent_frames = backing.is_none().then_some(len);
        let retired = (state.mixer.take(), state.audition.take(), old_frames);
        state.mix_fade = 0;
        state.set_loop(None);
        state.seek(0.);
        self.source = backing.map(|(url, _)| url.to_owned());
        drop(state);
        drop(retired);
        Ok(())
    }

    pub(crate) fn start_capture(
        &mut self,
        clock: Arc<crate::capture_clock::CaptureClock>,
        lead_seconds: f64,
        punch_seconds: f64,
    ) {
        let mut state = self.playback.lock().unwrap();
        let punch = source_frame(punch_seconds) * state.output_rate;
        state.seek(lead_seconds.max(0.));
        if lead_seconds < 0. {
            state.prelude_position =
                (lead_seconds * SOURCE_RATE as f64).round() as i64 * state.output_rate as i64;
        }
        state.capture = Some((clock, punch));
        state.playing = true;
    }

    pub(crate) fn end_capture(&mut self) {
        let mut state = self.playback.lock().unwrap();
        state.playing = false;
        state.capture = None;
        state.prelude_position = 0;
        if state.silent_frames.take().is_some() {
            state.seek(0.);
        }
    }

    fn install<T>(&self, device: &cpal::Device, config: &cpal::StreamConfig) -> Result<cpal::Stream>
    where
        T: SizedSample + FromSample<f32>,
    {
        let shared = Arc::clone(&self.playback);
        let fault = Arc::clone(&self.fault);
        let callbacks = Arc::clone(&self.callbacks);
        let contended = Arc::clone(&self.contended_callbacks);
        let channels = config.channels as usize;
        Ok(device.build_output_stream(
            config,
            move |output: &mut [T], info| {
                callbacks.fetch_add(1, Relaxed);
                if let Ok(mut transport) = shared.try_lock() {
                    transport.fill(output, channels, info.timestamp().callback);
                } else {
                    contended.fetch_add(1, Relaxed);
                    output.fill(T::EQUILIBRIUM);
                }
            },
            move |_| fault.store(true, Relaxed),
            None,
        )?)
    }

    fn ensure_stream(&mut self) -> Result<()> {
        if self.stream.is_some() && !self.fault.load(Relaxed) {
            return Ok(());
        }
        self.stream = None;
        self.device = None;
        self.device_id = None;
        self.channels = None;
        self.fault.store(false, Relaxed);
        let device = selected_output(self.output_preference.as_deref())?;
        let supported = device
            .default_output_config()
            .context("无法读取 Windows 输出格式")?;
        let config = supported.config();
        ensure!(
            config.channels > 0 && config.sample_rate > 0,
            "无效的 Windows 输出格式"
        );
        let resample = Kernel::new(config.sample_rate.into());
        let stream = match supported.sample_format() {
            SampleFormat::F32 => self.install::<f32>(&device, &config),
            SampleFormat::I16 => self.install::<i16>(&device, &config),
            SampleFormat::I32 => self.install::<i32>(&device, &config),
            SampleFormat::U16 => self.install::<u16>(&device, &config),
            format => bail!("不支持 Windows 输出采样格式 {format}"),
        }
        .context("无法打开 Windows WASAPI 输出")?;
        {
            let mut state = self.playback.lock().unwrap();
            let old_rate = state.output_rate;
            state.output_rate = u64::from(config.sample_rate);
            state.resample = resample;
            state.largest_buffer = 0;
            let position =
                u128::from(state.position) * u128::from(state.output_rate) / u128::from(old_rate);
            let step = phase_step(state.output_rate);
            state.position = (position as u64 / step) * step;
            if let Some(audition) = &mut state.audition {
                audition.fade = 0;
            }
        }
        stream.play().context("无法启动 Windows WASAPI 输出")?;
        self.device = device
            .description()
            .ok()
            .map(|description| description.name().to_owned());
        self.device_id = device.id().ok().map(|id| id.to_string());
        self.channels = Some(config.channels);
        self.stream = Some(stream);
        Ok(())
    }

    pub(crate) fn execute(
        &mut self,
        args: &Value,
        resolve_mix: impl FnOnce(&str) -> Result<std::path::PathBuf>,
    ) -> Result<Value> {
        let command = crate::model::field(args, "command")?;
        if self.fault.load(Relaxed) {
            self.playback.lock().unwrap().playing = false;
            if matches!(command, "status" | "loop" | "seek") {
                bail!("Windows 音频输出已中断。检查音频设备后重新播放。")
            }
        }
        match command {
            "play" => {
                let url = crate::model::field(args, "url")?;
                let position = seconds(&args["seconds"])?;
                let bounds = loop_bounds(&args["loop"])?;
                self.play(url, position, bounds, true, resolve_mix, || Ok(()))?;
            }
            "pause" => self.playback.lock().unwrap().playing = false,
            "resume_live" => {
                self.ensure_stream()?;
                let bounds = loop_bounds(&args["loop"])?;
                let mut state = self.playback.lock().unwrap();
                ensure!(
                    state
                        .mixer
                        .as_ref()
                        .is_some_and(|m| m.rate() == state.output_rate),
                    "监听设备已变更，请从混音重新播放"
                );
                let was_ended = state.ended;
                state.set_loop(bounds);
                if let Some((start, end)) = state.loop_frames {
                    let rate = state.output_rate;
                    if !(start * rate..end * rate).contains(&state.position) {
                        state.seek(start as f64 / SOURCE_RATE as f64);
                    }
                } else if was_ended {
                    state.seek(0.);
                }
                state.playing = true;
            }
            "stop" => {
                let mut state = self.playback.lock().unwrap();
                state.playing = false;
                state.seek(0.);
            }
            "seek" => self
                .playback
                .lock()
                .unwrap()
                .seek(seconds(&args["seconds"])?),
            "loop" => self
                .playback
                .lock()
                .unwrap()
                .set_loop(loop_bounds(&args["loop"])?),
            "clear" => {
                // Drop the stream and buffers outside the callback and release the endpoint.
                self.stream = None;
                *self = Self {
                    output_preference: self.output_preference.take(),
                    ..Self::default()
                };
            }
            "status" => (),
            _ => bail!("Unknown audio transport command"),
        }
        let state = self.playback.lock().unwrap();
        let status = Status {
            backend: "WASAPI",
            device: self.device.as_deref(),
            output_device_id: self.device_id.as_deref(),
            output_preference: self.output_preference.as_deref(),
            playing: state.playing,
            ended: state.ended,
            seconds: state.position as f64 / (state.output_rate * SOURCE_RATE) as f64,
            duration: state.length() as f64 / SOURCE_RATE as f64,
            output_rate: self.stream.as_ref().map(|_| state.output_rate),
            sample_rate_conversion: self.stream.as_ref().map(|_| {
                if state.resample.is_some() {
                    "band_limited_sinc"
                } else {
                    "native_48khz"
                }
            }),
            output_channels: self.channels,
            callbacks: self.callbacks.load(Relaxed),
            contended_callbacks: self.contended_callbacks.load(Relaxed),
            audio_frames: state.audio_frames,
            loop_frames: state.loop_frames,
            stop_frame: state.stop_range.map(|(_, end)| end),
            url: self.source.as_deref(),
            loop_wraps: state.wraps,
            peak: state.peak,
            live_mix: state.mixer.is_some(),
            live_frames: state.live_frames,
            mix_changes: state.mix_changes,
            track_peaks: state
                .mixer
                .as_ref()
                .map_or_else(Vec::new, crate::live_mix::Mixer::meters),
            audition_id: state.audition.as_ref().map(|a| a.manifest.id.clone()),
            audition_variant: state.audition.as_ref().map(|a| a.variant),
            audition_timeline_seconds: state
                .audition
                .as_ref()
                .map(|a| a.clocks[0].seconds(a.beat(state.position, state.output_rate))),
            click_pulses: state.click.as_ref().map_or(0, |click| click.played),
            last_click_frame: state.click.as_ref().and_then(|click| click.last_frame),
            streaming: state.streamed.is_some(),
            buffering: state.playing && state.buffering,
            buffered_frames: state
                .streamed
                .as_ref()
                .map_or(state.frames.len(), Stream::buffered_frames),
            buffer_wait_frames: state.buffer_wait_frames,
            stream_error: state.stream_error(),
            live_streaming: state.monitor_stream().is_some(),
            live_buffered_frames: state
                .mixer
                .as_ref()
                .map_or(0, crate::live_mix::Mixer::buffered_frames),
        };
        drop(state);
        Ok(serde_json::to_value(status)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paged_mix_matches_fractional_memory_playback_and_waits_without_skipping() -> Result<()> {
        let path = std::env::temp_dir().join(format!("{}.wav", crate::model::uid("paged_output")));
        monitor_stream::fixture_wav(&path, 90_000)?;
        let source = Stream::open(
            monitor_stream::open_mix(&path)?,
            24_574,
            Some((24_574, 73_731)),
        )?;
        let pages = source.pages();
        let mut disk = Playback {
            streamed: Some(source),
            output_rate: 44_100,
            resample: Kernel::new(44_100),
            playing: true,
            loop_frames: Some((24_574, 73_731)),
            position: 24_574 * 44_100 + 19_200,
            ..Playback::default()
        };
        let mut memory = Playback {
            frames: read_mix(&path)?,
            output_rate: disk.output_rate,
            resample: Kernel::new(disk.output_rate),
            playing: true,
            loop_frames: disk.loop_frames,
            position: disk.position,
            ..Playback::default()
        };
        assert_eq!(disk.next(None, None), None);
        assert!(disk.buffering);
        assert_eq!(disk.position, memory.position);
        assert_eq!(disk.buffer_wait_frames, 1);
        let guard = pages.lock().unwrap();
        for start in [disk.position, 73_727 * 44_100 + 19_200] {
            disk.position = start;
            memory.position = start;
            for _ in 0..96 {
                let frame = disk
                    .next(Some(&guard), None)
                    .expect("needed taps are resident");
                assert_eq!(Some(frame), memory.next(None, None));
                assert_eq!(disk.position, memory.position);
            }
        }
        assert!(disk.wraps > 0 && !disk.buffering);
        drop(guard);
        drop(disk);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn click_follows_fractional_device_clock_and_repeats_at_loop_boundary() {
        let timeline = crate::timeline::Timeline::new(120., [4, 4], &[], &[]);
        let mut state = Playback {
            frames: vec![[0.91; 2]; 480],
            output_rate: 44_100,
            resample: Kernel::new(44_100),
            loop_frames: Some((0, 480)),
            click: Some(crate::metronome::Click::new(timeline.pulses([0., 1.]))),
            playing: true,
            ..Playback::default()
        };
        let a: Vec<_> = (0..441).map(|_| state.next(None, None).unwrap()).collect();
        let b: Vec<_> = (0..441).map(|_| state.next(None, None).unwrap()).collect();
        assert_eq!(a, b);
        assert!(a.iter().any(|sample| sample[0] < 0.88));
        assert!(a.iter().flatten().all(|&sample| sample <= 1.));
        assert!(state.frames.iter().all(|sample| *sample == [0.91; 2]));
        assert_eq!(state.click.as_ref().unwrap().played, 2);
        assert_eq!(state.wraps, 2);
    }

    #[test]
    fn negative_preroll_has_compound_clicks_and_keeps_capture_punch_at_zero() {
        let timeline = crate::timeline::Timeline::new(120., [6, 8], &[], &[]);
        let pulses = timeline.pulses([-3., 1.]);
        assert_eq!(
            pulses
                .iter()
                .map(|p| (p.frame, p.accent))
                .collect::<Vec<_>>(),
            [(-72000, true), (-36000, false), (0, true)]
        );
        let timing = Arc::new(crate::capture_clock::CaptureClock::default());
        let instant = cpal::StreamInstant::new(20, 0);
        timing.input(instant);
        let mut state = Playback {
            silent_frames: Some(96_000),
            output_rate: 44_100,
            resample: Kernel::new(44_100),
            prelude_position: -72_000 * 44_100,
            capture: Some((timing.clone(), 0)),
            click: Some(crate::metronome::Click::new(pulses)),
            playing: true,
            ..Playback::default()
        };
        let mut monitor = vec![0_f32; 2 * (66_150 + 700)];
        state.fill(&mut monitor, 2, instant);
        for onset in [0, 33_075, 66_150] {
            assert!(monitor[onset * 2..(onset + 130) * 2]
                .iter()
                .any(|sample| sample.abs() > 0.05));
        }
        assert_eq!(state.click.unwrap().played, 3);
        assert_eq!(state.position, 700 * SOURCE_RATE);
        assert_eq!(timing.offset(48_000, 0.), Some(72_000));
    }

    #[test]
    fn comparison_switch_uses_musical_beats_across_different_tempo_maps() {
        use crate::audition::{Manifest, Timing};
        use crate::timeline::{Curve, TempoPoint};
        let timing = [
            Timing {
                bpm: 120.,
                tempos: vec![],
            },
            Timing {
                bpm: 60.,
                tempos: vec![TempoPoint {
                    beat: 4.,
                    bpm: 120.,
                    curve: Curve::Step,
                }],
            },
        ];
        let mut comparison = Comparison {
            clocks: [timing[0].clock(), timing[1].clock()],
            manifest: Manifest {
                id: "ab_test".into(),
                project_id: "p_test".into(),
                revision: 3,
                beat_range: [0., 8.],
                frame_ranges: [[0, 192_000], [0, 288_000]],
                timing,
                matched_rms: true,
                level_match: true,
                match_method: Default::default(),
                matched_lufs: false,
                lufs_before: [None; 2],
                lufs_after: [None; 2],
                short_rms_fallback: false,
                gain_db: [0.; 2],
                silent: [false; 2],
                source_trim_db: 0.,
                saved: None,
                recordings: None,
                source_inputs: None,
                output: None,
                local_voice_track_id: None,
            },
            variant: 0,
            alternate: vec![],
            old_position: 0,
            fade: 0,
        };
        let rate = 44_100;
        let beat = comparison.beat(SOURCE_RATE * rate, rate);
        assert_eq!(beat, 2.);
        assert_eq!(comparison.position(1, beat, rate), 2 * SOURCE_RATE * rate);
        comparison.manifest.frame_ranges[1] = [384_000, 480_000];
        comparison.manifest.saved = Some(crate::audition::SavedSource {
            job_id: Some("render_test".into()),
            renders: None,
            beat_ranges: [[0., 8.], [12., 16.]],
            outputs: None,
        });
        assert_eq!(
            comparison.switch_position(1, SOURCE_RATE * rate, rate),
            24_000 * rate
        );
    }

    #[test]
    fn one_shot_cannot_read_past_its_endpoint() {
        let mut state = Playback {
            frames: (0..8).map(|n| [n as f32 / 8., -(n as f32) / 8.]).collect(),
            position: 2 * SOURCE_RATE,
            stop_range: Some((2, 5)),
            playing: true,
            ..Playback::default()
        };
        let mut output = [0_f32; 10];
        state.fill(&mut output, 2, cpal::StreamInstant::new(1, 0));
        assert_eq!(
            output,
            [0.25, -0.25, 0.375, -0.375, 0.5, -0.5, 0., 0., 0., 0.]
        );
        assert_eq!(state.position, 5 * SOURCE_RATE);
        assert_eq!(state.audio_frames, 3);
        assert!(state.ended && !state.playing);
    }

    #[test]
    fn end_excludes_last_loop_boundary_and_pause_preserves_position() {
        let mut state = Playback {
            frames: vec![[0., 0.], [0.1, -0.1], [0.2, -0.2], [0.3, -0.3]],
            playing: true,
            loop_frames: Some((1, 3)),
            ..Playback::default()
        };
        let mut output = [0_f32; 10];
        state.fill(&mut output, 2, cpal::StreamInstant::new(1, 0));
        assert_eq!(
            output,
            [0.1, -0.1, 0.2, -0.2, 0.1, -0.1, 0.2, -0.2, 0.1, -0.1]
        );
        assert_eq!(state.wraps, 2);
        let position = state.position;
        state.playing = false;
        state.fill(&mut output, 2, cpal::StreamInstant::new(1, 0));
        assert_eq!(output, [0.; 10]);
        assert_eq!(state.position, position);
        assert_eq!(state.audio_frames, 5);
    }

    #[test]
    fn different_device_rate_keeps_fractional_position_across_loop() {
        let mut state = Playback {
            frames: vec![[0., 0.], [0.5, -0.5], [1., -1.]],
            output_rate: 96_000,
            resample: Kernel::new(96_000),
            playing: true,
            loop_frames: Some((1, 3)),
            ..Playback::default()
        };
        let mut output = [0_f32; 12];
        state.fill(&mut output, 2, cpal::StreamInstant::new(1, 0));
        for frame in output.as_chunks::<2>().0 {
            assert!((frame[0] - 0.75).abs() < 1e-4);
            assert_eq!(frame[0], -frame[1]);
        }
        assert_eq!(state.wraps, 1);
        assert_eq!(state.position, 2 * 96_000);
        state.loop_frames = None;
        assert!(state.next(None, None).is_some());
        assert!(state.next(None, None).is_some());
        assert_eq!(state.next(None, None), None);
        assert!(state.ended);
    }

    #[test]
    fn resampled_one_shot_never_samples_outside_the_selected_range() {
        let mut frames = vec![[1., 1.]; 420];
        frames[200..301].fill([0.25, -0.5]);
        let mut state = Playback {
            frames,
            output_rate: 44_100,
            resample: Kernel::new(44_100),
            position: 200 * 44_100,
            stop_range: Some((200, 301)),
            playing: true,
            ..Playback::default()
        };
        for _ in 0..93 {
            let [left, right] = state.next(None, None).unwrap();
            assert!((left - 0.25).abs() < 1e-6 && (right + 0.5).abs() < 1e-6);
        }
        assert_eq!(state.next(None, None), None);
        assert_eq!(state.position, 301 * 44_100);
    }

    #[test]
    fn device_resampling_preserves_channels_and_rejects_images() {
        use std::f64::consts::TAU;
        fn signal(frequency: u64, frame: usize, rate: u64) -> f64 {
            TAU * frequency as f64 * frame as f64 / rate as f64
        }
        fn amplitude(data: &[f32], rate: u64, channel: usize, frequency: u64) -> f64 {
            let count = rate as usize / 10;
            let mut correlation = [0., 0.];
            for (index, frame) in data
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .skip(count)
                .take(count)
            {
                let (sin, cos) = signal(frequency, index, rate).sin_cos();
                correlation[0] += f64::from(frame[channel]) * sin;
                correlation[1] += f64::from(frame[channel]) * cos;
            }
            2. * correlation[0].hypot(correlation[1]) / count as f64
        }
        let mut measurements = Vec::new();
        for (rate, pass_hz, input_hz, test_channel, rejected_hz) in [
            (44_100, 20_000, 23_000, 1, 21_100),
            (8_000, 1_000, 7_000, 1, 1_000),
            (96_000, 19_000, 2_000, 0, 29_000),
        ] {
            let frames = (0..SOURCE_RATE as usize / 4)
                .map(|index| {
                    [pass_hz, input_hz]
                        .map(|hz| (0.5 * signal(hz, index, SOURCE_RATE).sin()) as f32)
                })
                .collect();
            let mut state = Playback {
                frames,
                output_rate: rate,
                resample: Kernel::new(rate),
                playing: true,
                ..Playback::default()
            };
            let mut data = vec![0_f32; rate as usize / 2];
            state.fill(&mut data, 2, cpal::StreamInstant::new(1, 0));
            assert!(state.ended);
            assert_eq!(state.audio_frames, rate / 4);
            let pass_db = 20. * (2. * amplitude(&data, rate, 0, pass_hz)).log10();
            let rejected_db =
                20. * (2. * amplitude(&data, rate, test_channel, rejected_hz)).log10();
            assert!(
                pass_db.abs() < 0.04 && rejected_db < -70.,
                "{rate}: {pass_db}, {rejected_db}"
            );
            measurements.push(serde_json::json!({
                "output_rate": rate, "passband_hz": pass_hz, "passband_gain_db": pass_db,
                "rejected_hz": rejected_hz, "rejected_signal_db": rejected_db,
            }));
        }
        println!(
            "SINC_PLAYBACK_MEASUREMENTS={}",
            serde_json::to_string(&measurements).unwrap()
        );
        assert!(Kernel::new(SOURCE_RATE).is_none());
    }
}
