//! Desktop-owned capture. The WASAPI callback writes only to a bounded ring.
use crate::{
    capture_clock::CaptureClock,
    model::{field, identifier, uid},
    transport_link::{CaptureCommand, Request as TransportRequest},
};
use anyhow::{bail, ensure, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, SampleFormat, SizedSample,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering::*},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
        Arc,
    },
    time::Duration,
};

const LIMIT_SECONDS: usize = 900;
const RING_FRAMES: usize = 131_072;
type Writer = hound::WavWriter<BufWriter<File>>;

pub(crate) enum Request {
    Call(u64, Value),
    Close(u64),
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    project_id: String,
    expected_revision: u64,
    start: f64,
    track_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Overdub {
    job_id: Option<String>,
    lead_beats: f64,
    correction_ms: f64,
    #[serde(default)]
    click: bool,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Take {
    id: String,
    binding: Binding,
    device: String,
    input_channels: usize,
    sample_rate: u32,
    frames: usize,
    peak: f32,
    warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset_frames: Option<i64>,
}

pub(crate) fn directory(root: &Path) -> PathBuf {
    root.join("runtime").join("recordings")
}

fn valid_id(id: &str) -> Result<()> {
    ensure!(
        id.strip_prefix("take_")
            .is_some_and(|part| part.len() == 32 && part.bytes().all(|b| b.is_ascii_hexdigit())),
        "无效的录音编号"
    );
    Ok(())
}

pub(crate) struct Prepared {
    pub raw: PathBuf,
    pub asset: String,
    pub track_id: Option<String>,
    pub start: f64,
    pub title: String,
    pub offset_frames: i64,
}

pub(crate) fn prepared(root: &Path, args: &Value) -> Result<Prepared> {
    let id = field(args, "take_id")?;
    valid_id(id)?;
    let take: Take = serde_json::from_reader(File::open(directory(root).join("pending.json"))?)?;
    ensure!(
        take.id == id && take.binding.project_id == field(args, "project_id")?,
        "录音与工程不匹配"
    );
    let track_id = match args.get("track_id") {
        Some(value) => serde_json::from_value(value.clone())?,
        None => take.binding.track_id,
    };
    let start = args
        .get("start")
        .map(|value| value.as_f64().context("无效录音拍位"))
        .transpose()?
        .unwrap_or(take.binding.start);
    ensure!((0.0..=50_000.).contains(&start), "录音拍位无效");
    Ok(Prepared {
        raw: directory(root).join(format!("{id}.wav")),
        asset: format!("audio_{id}.wav"),
        track_id,
        start,
        title: args["name"].as_str().unwrap_or("录音").to_owned(),
        offset_frames: take.offset_frames.unwrap_or(0),
    })
}

struct Ring {
    frames: Box<[AtomicU64]>,
    write: AtomicUsize,
    read: AtomicUsize,
    peak: AtomicU32,
    fault: AtomicU32,
    limit: usize,
    clock: Option<Arc<CaptureClock>>,
}

impl Ring {
    fn new(rate: u32, seconds: f64, clock: Option<Arc<CaptureClock>>) -> Self {
        Self {
            frames: (0..RING_FRAMES).map(|_| AtomicU64::new(0)).collect(),
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            peak: AtomicU32::new(0),
            fault: AtomicU32::new(0),
            limit: (seconds * f64::from(rate)).floor() as usize,
            clock,
        }
    }

    fn capture<T: Copy>(
        &self,
        input: &[T],
        stride: usize,
        first: usize,
        channels: usize,
        convert: impl Fn(T) -> f32,
    ) {
        if self.fault.load(Relaxed) != 0 {
            return;
        }
        let mut cursor = self.write.load(Relaxed);
        let room = (self.read.load(Acquire) + RING_FRAMES).min(self.limit);
        let mut peak = 0f32;
        for frame in input.chunks_exact(stride) {
            if cursor == room {
                let reason = if cursor == self.limit { 3 } else { 1 };
                let _ = self.fault.compare_exchange(0, reason, Relaxed, Relaxed);
                break;
            }
            let left = convert(frame[first]);
            let right = if channels == 2 {
                convert(frame[first + 1])
            } else {
                left
            };
            if !left.is_finite() || !right.is_finite() {
                let _ = self.fault.compare_exchange(0, 4, Relaxed, Relaxed);
                break;
            }
            peak = peak.max(left.abs()).max(right.abs());
            let bits = u64::from(left.to_bits()) | (u64::from(right.to_bits()) << 32);
            self.frames[cursor % RING_FRAMES].store(bits, Relaxed);
            cursor += 1;
        }
        self.write.store(cursor, Release);
        self.peak.fetch_max(peak.to_bits(), Relaxed);
    }

    fn drain(&self, writer: &mut Writer) -> Result<()> {
        let start = self.read.load(Relaxed);
        let end = self.write.load(Acquire);
        for i in start..end {
            let bits = self.frames[i % RING_FRAMES].load(Relaxed);
            writer.write_sample(f32::from_bits(bits as u32))?;
            writer.write_sample(f32::from_bits((bits >> 32) as u32))?;
        }
        self.read.store(end, Release);
        Ok(())
    }
}

fn input_stream<T: SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    capture: Arc<Ring>,
    first: usize,
    channels: usize,
) -> Result<cpal::Stream>
where
    f32: FromSample<T>,
{
    let stride = usize::from(config.channels);
    let fault = capture.clone();
    Ok(device.build_input_stream(
        config,
        move |samples: &[T], info| {
            if !samples.is_empty() {
                if let Some(clock) = &capture.clock {
                    clock.input(info.timestamp().capture);
                }
                capture.capture(samples, stride, first, channels, f32::from_sample_)
            }
        },
        move |_| {
            let _ = fault.fault.compare_exchange(0, 2, Relaxed, Relaxed);
        },
        None,
    )?)
}

struct Active {
    take: Take,
    path: PathBuf,
    writer: Writer,
    stream: Option<cpal::Stream>,
    ring: Arc<Ring>,
    overdub: Option<Overdub>,
    fixture_channels: Option<(usize, usize)>,
}

impl Active {
    fn status(&self) -> Value {
        let frames = self.ring.write.load(Acquire);
        let offset = self
            .ring
            .clock
            .as_ref()
            .zip(self.overdub.as_ref())
            .and_then(|(clock, mode)| clock.offset(self.take.sample_rate, mode.correction_ms));
        let preroll = self.overdub.is_some() && offset.is_none_or(|start| start > frames as i64);
        let usable = if preroll {
            0
        } else {
            (frames as i64 - offset.unwrap_or(0)).max(0)
        };
        let peak = f32::from_bits(self.ring.peak.load(Relaxed));
        json!({"phase":"recording","id":self.take.id,"device":self.take.device,"sample_rate":self.take.sample_rate,
            "frames":frames,"seconds":usable as f64/f64::from(self.take.sample_rate),"peak":peak,
            "overdub":self.overdub.is_some(),"preroll":preroll,
            "project_id":self.take.binding.project_id,"start":self.take.binding.start})
    }
}

struct Recorder {
    root: PathBuf,
    dir: PathBuf,
    devices: Vec<cpal::Device>,
    active: Option<Active>,
    pending: Option<Take>,
    error: Option<String>,
    transport: Option<SyncSender<TransportRequest>>,
}

impl Recorder {
    fn new(root: PathBuf) -> Self {
        let dir = directory(&root);
        let mut error = None;
        let pending = match File::open(dir.join("pending.json")) {
            Ok(file) => match serde_json::from_reader::<_, Take>(file) {
                Ok(take)
                    if valid_id(&take.id).is_ok()
                        && dir.join(format!("{}.wav", take.id)).is_file() =>
                {
                    Some(take)
                }
                _ => {
                    error = Some("无法恢复待导入录音，请检查工程的录音目录".into());
                    None
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                error = Some(e.to_string());
                None
            }
        };
        Self {
            root,
            dir,
            devices: vec![],
            active: None,
            pending,
            error,
            transport: None,
        }
    }

    fn send_transport(&self, command: CaptureCommand) -> Result<Value> {
        let (reply, receipt) = mpsc::sync_channel(1);
        self.transport
            .as_ref()
            .context("伴奏控制不可用")?
            .try_send(TransportRequest::Capture { command, reply })
            .map_err(|_| anyhow::anyhow!("伴奏控制暂时不可用"))?;
        receipt
            .recv_timeout(Duration::from_secs(45))
            .context("等待伴奏响应超时")?
    }

    fn devices(&mut self) -> Result<Value> {
        let host = cpal::default_host();
        self.devices = host.input_devices()?.collect();
        let default_id = host
            .default_input_device()
            .and_then(|device| device.id().ok());
        let devices = self
            .devices
            .iter()
            .enumerate()
            .filter_map(|(id, device)| {
                let config = device.default_input_config().ok()?;
                if !matches!(
                    config.sample_format(),
                    SampleFormat::F32 | SampleFormat::I16 | SampleFormat::I32 | SampleFormat::U16
                ) || !(8000..=192_000).contains(&config.sample_rate())
                    || config.channels() == 0
                {
                    return None;
                }
                let name = device.description().map(|description|description.name().to_owned())
                    .unwrap_or_else(|_| format!("输入 {}", id + 1));
                Some(
                    json!({"device_id":id,"name":name,"channels":config.channels(),
                "sample_rate":config.sample_rate(),"default":default_id.is_some()&&default_id==device.id().ok()}),
                )
            })
            .collect::<Vec<_>>();
        Ok(json!(devices))
    }

    fn status(&self) -> Value {
        if let Some(active) = &self.active {
            return active.status();
        }
        if let Some(take) = &self.pending {
            return json!({"phase":"ready","id":take.id,"device":take.device,"sample_rate":take.sample_rate,
                "frames":take.frames,"seconds":(take.frames as i64-take.offset_frames.unwrap_or(0)) as f64/f64::from(take.sample_rate),"peak":take.peak,
                "offset_frames":take.offset_frames,"aligned":take.offset_frames.is_some(),
                "project_id":take.binding.project_id,"expected_revision":take.binding.expected_revision,
                "track_id":take.binding.track_id,"start":take.binding.start,"warning":take.warning,"error":self.error});
        }
        json!({"phase":"idle","error":self.error})
    }

    fn start(&mut self, args: &Value, fixture: bool) -> Result<Value> {
        ensure!(
            self.active.is_none()
                && self.pending.is_none()
                && !self.dir.join("pending.json").exists(),
            "请先处理上一段录音"
        );
        let binding: Binding = serde_json::from_value(args["binding"].clone())?;
        let overdub: Option<Overdub> = args
            .get("overdub")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        if let Some(mode) = &overdub {
            ensure!(
                (0.0..=8.).contains(&mode.lead_beats)
                    && (-500.0..=500.).contains(&mode.correction_ms),
                "伴奏预听或校正值超出范围"
            );
        }
        let clock = overdub.as_ref().map(|_| Arc::new(CaptureClock::default()));
        identifier(&binding.project_id)?;
        ensure!((0.0..=50_000.).contains(&binding.start), "录音拍位无效");
        let project: crate::model::Project = serde_json::from_value(crate::service::proxy_source(
            &self.root,
            "project_read",
            &json!({"project_id":binding.project_id}),
            "user",
        )?)?;
        ensure!(
            project.revision == binding.expected_revision,
            "REVISION_CONFLICT: 请先刷新工程"
        );
        if let Some(track) = &binding.track_id {
            ensure!(
                project
                    .tracks
                    .iter()
                    .any(|t| &t.id == track && t.is_audio()),
                "请选择现有音频轨或新建轨"
            );
        }
        let first = args["first_channel"]
            .as_u64()
            .unwrap_or(1)
            .checked_sub(1)
            .context("输入通道从 1 开始")? as usize;
        let channels = args["channels"].as_u64().unwrap_or(1) as usize;
        ensure!([1, 2].contains(&channels), "请选择单声道或双声道录音");
        let seconds =
            LIMIT_SECONDS as f64 - project.timeline().seconds(binding.start) - 1. / 48_000.;
        ensure!(seconds >= 0.01, "拍位已接近工程时长上限");
        ensure!(!fixture || first <= 2 - channels, "无效的测试通道");
        let (name, rate, stream, ring) = if fixture {
            let rate = 44_100;
            (
                "测试信号".to_owned(),
                rate,
                None,
                Arc::new(Ring::new(rate, seconds, clock.clone())),
            )
        } else {
            let index = args["device_id"].as_u64().context("请选择输入设备")? as usize;
            let device = self.devices.get(index).context("输入设备列表已更新")?;
            let supported = device.default_input_config()?;
            let config = supported.config();
            let rate = config.sample_rate;
            ensure!(
                (8000..=192_000).contains(&rate)
                    && first <= usize::from(config.channels).saturating_sub(channels)
                    && usize::from(config.channels) >= channels,
                "所选设备或输入通道无效"
            );
            let ring = Arc::new(Ring::new(rate, seconds, clock.clone()));
            let stream = match supported.sample_format() {
                SampleFormat::F32 => {
                    input_stream::<f32>(device, &config, ring.clone(), first, channels)
                }
                SampleFormat::I16 => {
                    input_stream::<i16>(device, &config, ring.clone(), first, channels)
                }
                SampleFormat::I32 => {
                    input_stream::<i32>(device, &config, ring.clone(), first, channels)
                }
                SampleFormat::U16 => {
                    input_stream::<u16>(device, &config, ring.clone(), first, channels)
                }
                format => bail!("不支持输入格式 {format}"),
            }
            .context("无法打开 Windows WASAPI 输入")?;
            (
                device
                    .description()
                    .map(|description| description.name().to_owned())
                    .unwrap_or_else(|_| "录音输入".into()),
                rate,
                Some(stream),
                ring,
            )
        };
        fs::create_dir_all(&self.dir)?;
        let id = uid("take");
        let path = self.dir.join(format!("{id}.wav"));
        let writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        let start_device = (|| -> Result<()> {
            if let Some((mode, clock)) = overdub.as_ref().zip(clock) {
                self.send_transport(CaptureCommand::Prepare {
                    id: id.clone(),
                    project_id: binding.project_id.clone(),
                    revision: binding.expected_revision,
                    job_id: mode.job_id.clone(),
                    beat: binding.start,
                    lead_beats: mode.lead_beats,
                    click: mode.click,
                    clock,
                })?;
            }
            if let Some(device) = &stream {
                device.play().context("无法启动音频输入")?;
            }
            if overdub.is_some() {
                self.send_transport(CaptureCommand::Start(id.clone()))?;
            }
            Ok(())
        })();
        if let Err(error) = start_device {
            drop(stream);
            if overdub.is_some() {
                let _ = self.send_transport(CaptureCommand::End(id));
            }
            drop(writer);
            let _ = fs::remove_file(path);
            return Err(error);
        }
        if fixture && overdub.is_none() {
            fixture_signal(&ring, first, channels, 0);
        }
        self.error = None;
        self.active = Some(Active {
            take: Take {
                id,
                binding,
                device: name,
                input_channels: channels,
                sample_rate: rate,
                frames: 0,
                peak: 0.,
                warning: None,
                offset_frames: None,
            },
            path,
            writer,
            stream,
            ring,
            fixture_channels: (fixture && overdub.is_some()).then_some((first, channels)),
            overdub,
        });
        Ok(self.status())
    }

    fn stop(&mut self) -> Result<Value> {
        let Some(Active {
            mut take,
            path,
            mut writer,
            stream,
            ring,
            overdub,
            fixture_channels: _,
        }) = self.active.take()
        else {
            self.persist_pending()?;
            return Ok(self.status());
        };
        drop(stream);
        let audio_stop = overdub.as_ref().and_then(|_| {
            self.send_transport(CaptureCommand::End(take.id.clone()))
                .err()
        });
        ring.drain(&mut writer)?;
        writer.finalize()?;
        take.frames = ring.read.load(Acquire);
        if take.frames == 0 {
            fs::remove_file(path)?;
            self.error = Some("没有收到输入采样".into());
            return Ok(self.status());
        }
        take.peak = f32::from_bits(ring.peak.load(Relaxed));
        take.warning = match ring.fault.load(Relaxed) {
            1 => Some("录音缓冲满，已保存中断前的片段".into()),
            2 => Some("输入设备中断，已保存中断前的片段".into()),
            3 => Some("已达到该拍位可用的录音长度".into()),
            4 => Some("输入采样异常，已保存异常前的片段".into()),
            _ => None,
        };
        if let Some(mode) = overdub {
            take.offset_frames = ring
                .clock
                .as_ref()
                .and_then(|clock| clock.offset(take.sample_rate, mode.correction_ms))
                .filter(|&offset| {
                    offset >= -i64::from(take.sample_rate) * 2 && offset < take.frames as i64
                });
            if take.offset_frames.is_none() {
                take.warning = Some("录音未覆盖所选入点，原始素材可手动定位".into());
            }
            if let Some(error) = audio_stop {
                self.error = Some(error.to_string());
            }
        }
        self.pending = Some(take);
        self.persist_pending()?;
        Ok(self.status())
    }

    fn persist_pending(&self) -> Result<()> {
        let saved = self.dir.join("pending.json");
        if let Some(take) = self.pending.as_ref().filter(|_| !saved.exists()) {
            let temporary = self.dir.join(format!("{}.json.tmp", take.id));
            fs::write(&temporary, serde_json::to_vec(take)?)?;
            crate::model::move_file_windows(&temporary, &saved, false)?;
        }
        Ok(())
    }

    fn forget_pending(&mut self) -> Result<()> {
        if let Some(take) = &self.pending {
            match fs::remove_file(self.dir.join("pending.json")) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into())
                }
                _ => (),
            }
            let _ = fs::remove_file(self.dir.join(format!("{}.wav", take.id)));
            self.pending = None;
        }
        self.error = None;
        Ok(())
    }

    fn call(&mut self, args: &Value, verify: bool) -> Result<Value> {
        match field(args, "action")? {
            "devices" => self.devices(),
            "status" => Ok(self.status()),
            "start" => self.start(args, false),
            "fixture_start" if verify => self.start(args, true),
            "stop" => self.stop(),
            "discard" => {
                ensure!(self.active.is_none(), "请先停止录音");
                self.forget_pending()?;
                Ok(self.status())
            }
            "commit" => {
                let take = self.pending.as_ref().context("没有待导入录音")?;
                ensure!(
                    args["take_id"] == take.id && args["project_id"] == take.binding.project_id,
                    "录音或工程已切换"
                );
                self.persist_pending()?;
                let project =
                    crate::service::proxy_source(&self.root, "recording_attach", args, "user")?;
                if let Err(error) = self.forget_pending() {
                    self.error = Some(error.to_string());
                }
                Ok(json!({"project":project,"recording":self.status()}))
            }
            _ => bail!("未知录音操作"),
        }
    }

    fn tick(&mut self) -> Result<()> {
        if let Some(active) = &mut self.active {
            if let Some((first, channels)) = active.fixture_channels {
                if let Some((clock, punch)) = active
                    .ring
                    .clock
                    .as_ref()
                    .and_then(|clock| clock.punch_instant().map(|punch| (clock, punch)))
                {
                    let prefix = 4410;
                    clock.input(
                        punch
                            .sub(Duration::from_millis(100))
                            .context("测试录音时钟无效")?,
                    );
                    fixture_signal(&active.ring, first, channels, prefix);
                    active.fixture_channels = None;
                }
            }
            active.ring.drain(&mut active.writer)?;
            if active.ring.fault.load(Relaxed) != 0 {
                self.stop()?;
            }
        }
        Ok(())
    }
}

fn fixture_signal(ring: &Ring, first: usize, channels: usize, prefix: usize) {
    let mut frames = vec![0.; prefix * 2];
    for i in 0..8820 {
        let phase = std::f32::consts::TAU * i as f32 / 100.;
        frames.extend([0.32 * phase.sin(), 0.11 * (phase * 2.).cos()]);
    }
    ring.capture(&frames, 2, first, channels, |v| v);
}

pub(crate) fn run(
    root: PathBuf,
    requests: Receiver<Request>,
    verify: bool,
    transport: SyncSender<TransportRequest>,
    reply: impl Fn(u64, bool, Result<Value>) -> bool,
) {
    let mut recorder = Recorder::new(root);
    recorder.transport = Some(transport);
    loop {
        let wait = if recorder.active.is_some() {
            Duration::from_millis(12)
        } else {
            Duration::from_secs(2)
        };
        match requests.recv_timeout(wait) {
            Ok(Request::Call(id, args)) => {
                let response = recorder.call(&args, verify);
                if !reply(id, false, response) {
                    break;
                }
            }
            Ok(Request::Close(id)) => {
                let result = recorder.stop();
                let closed = result.is_ok();
                if !reply(id, closed, result) || closed {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => (),
        }
        if let Err(error) = recorder.tick() {
            recorder.error = Some(format!("{error:#}"));
            // A disk failure must close the input immediately.
            if let Some(active) = recorder.active.take() {
                if active.overdub.is_some() {
                    let _ = recorder.send_transport(CaptureCommand::End(active.take.id));
                }
            }
        }
    }
    let _ = recorder.stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_take_recovered_from_disk_with_original_samples() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_capture_test"));
        let mut receiver = Recorder::new(root.clone());
        fs::create_dir_all(&receiver.dir)?;
        let id = uid("take");
        let path = receiver.dir.join(format!("{id}.wav"));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let ring = Arc::new(Ring::new(spec.sample_rate, 1., None));
        ring.capture(&[0.1f32, -0.4, -0.2, 0.23], 2, 1, 1, |v| v);
        receiver.active = Some(Active {
            take: Take {
                id: id.clone(),
                binding: Binding {
                    project_id: "p_fixture".into(),
                    expected_revision: 5,
                    start: 1.5,
                    track_id: None,
                },
                device: "test".into(),
                input_channels: 1,
                sample_rate: spec.sample_rate,
                frames: 0,
                peak: 0.,
                warning: None,
                offset_frames: None,
            },
            path: path.clone(),
            writer: hound::WavWriter::create(&path, spec)?,
            stream: None,
            ring,
            overdub: None,
            fixture_channels: None,
        });
        receiver.stop()?;
        drop(receiver);
        let mut recovered = Recorder::new(root.clone());
        let state = recovered.status();
        assert_eq!(state["id"], id);
        assert_eq!(state["phase"], "ready");
        assert_eq!(state["frames"], 2);
        assert_eq!(state["start"], 1.5);
        let pcm = hound::WavReader::open(&path)?
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(pcm, vec![-0.4, -0.4, 0.23, 0.23]);
        let aligned = path.with_extension("aligned.wav");
        crate::audio::recording_asset(&path, &aligned, -1)?;
        let padded = hound::WavReader::open(&aligned)?
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(padded, [vec![0., 0.], pcm.clone()].concat());
        fs::remove_file(&aligned)?;
        crate::audio::recording_asset(&path, &aligned, 1)?;
        let trimmed = hound::WavReader::open(&aligned)?
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(trimmed, pcm[2..]);
        fs::remove_file(&aligned)?;
        recovered.forget_pending()?;
        assert!(!path.exists());
        fs::remove_dir(&recovered.dir)?;
        fs::remove_dir(root.join("runtime"))?;
        fs::remove_dir(root)?;
        Ok(())
    }
}
