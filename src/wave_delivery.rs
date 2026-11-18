use crate::model::uid;
use anyhow::{ensure, Context, Result};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};
use zip::write::SimpleFileOptions;

const SOURCE_RATE: u32 = 48_000;
const TAPS: usize = 64;
const LEFT: usize = TAPS / 2 - 1;
pub(crate) const ENGINE: &str = "wav_delivery";

#[derive(Clone, Copy)]
pub(crate) struct BeatSpan {
    beats: [f64; 2],
    frames: [u32; 2],
}
impl BeatSpan {
    pub(crate) fn new(project: &crate::model::Project, beats: [f64; 2]) -> Result<Self> {
        let [start, end] = beats;
        ensure!(
            start.is_finite() && end.is_finite() && start >= 0. && end > start && end <= 50_000.,
            "指定有效的导出拍区间"
        );
        let clock = project.timeline();
        let frames =
            beats.map(|beat| (clock.seconds(beat) * f64::from(SOURCE_RATE)).round() as u32);
        ensure!(frames[1] > frames[0], "导出区间不足一个采样帧");
        Ok(Self { beats, frames })
    }
}

fn source_bounds(span: Option<BeatSpan>, source_frames: u32) -> Result<[u32; 2]> {
    let bounds = span.map_or([0, source_frames], |span| span.frames);
    ensure!(bounds[1] <= source_frames, "导出区间超出已完成渲染");
    Ok(bounds)
}

fn span_suffix(span: Option<BeatSpan>) -> &'static str {
    if span.is_some() {
        "_选区"
    } else {
        ""
    }
}

#[derive(Clone, Copy)]
struct Format {
    rate: u32,
    bits: u16,
}
impl Format {
    fn read(args: &Value) -> Result<Self> {
        let setting = |key: &str, default: u64| {
            args.get(key).map_or(Ok(default), |value| {
                value
                    .as_u64()
                    .with_context(|| format!("{key} must be an integer"))
            })
        };
        let rate = setting("sample_rate", 48000)?;
        let bits = setting("bit_depth", 24)?;
        ensure!(
            [44100, 48000].contains(&rate) && [16, 24].contains(&bits),
            "WAV 仅支持 44100 / 48000 Hz 和 16 / 24 bit"
        );
        Ok(Self {
            rate: rate as u32,
            bits: bits as u16,
        })
    }
    fn frames(self, source: u32) -> u64 {
        (u64::from(source) * u64::from(self.rate) + u64::from(SOURCE_RATE / 2))
            / u64::from(SOURCE_RATE)
    }
    fn exact(self) -> bool {
        self.rate == SOURCE_RATE && self.bits == 24
    }
    fn dither(self) -> Option<&'static str> {
        (self.bits == 16).then_some("tpdf")
    }
}

fn open_source(directory: &Path, name: &str) -> Result<(PathBuf, WavReader<BufReader<fs::File>>)> {
    ensure!(
        Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name)
            && name.ends_with(".wav")
            && !name.contains(['/', '\\', ':']),
        "Invalid rendered wave name"
    );
    let source = directory.join(name).canonicalize()?;
    ensure!(
        source.parent() == Some(directory),
        "Invalid rendered wave path"
    );
    let reader = WavReader::open(&source)?;
    ensure!(
        reader.spec()
            == WavSpec {
                channels: 2,
                sample_rate: SOURCE_RATE,
                bits_per_sample: 24,
                sample_format: SampleFormat::Int
            }
            && reader.duration() > 0,
        "Source render must be 48 kHz 24-bit stereo PCM"
    );
    Ok((source, reader))
}

pub(crate) struct StemDelivery {
    format: Format,
    render: Value,
    stems: Vec<String>,
    span: Option<BeatSpan>,
}

impl StemDelivery {
    pub(crate) fn new(
        root: &Path,
        render: Value,
        args: &Value,
        span: Option<BeatSpan>,
    ) -> Result<Self> {
        ensure!(
            args.get("stem").is_none(),
            "分轨 ZIP 会使用这次渲染的全部分轨"
        );
        let format = Format::read(args)?;
        let stems: Vec<String> = serde_json::from_value(render["stems"].clone())?;
        ensure!(!stems.is_empty(), "该渲染没有可交付的分轨");
        let unique: HashSet<_> = stems.iter().map(|name| name.to_lowercase()).collect();
        ensure!(unique.len() == stems.len(), "渲染内有同名分轨");
        if span.is_some() {
            let job = render["job_id"].as_str().context("Missing source render")?;
            let directory = crate::audition::export_directory(root, job)?;
            let (_, mix) = open_source(&directory, "试听混音.wav")?;
            source_bounds(span, mix.duration())?;
        }
        Ok(Self {
            format,
            render,
            stems,
            span,
        })
    }

    pub(crate) fn run(
        &self,
        root: &Path,
        id: &str,
        control: &crate::jobs::Control,
    ) -> Result<Value> {
        let render = &self.render;
        let job = render["job_id"].as_str().context("Missing source render")?;
        let directory = crate::audition::export_directory(root, job)?;
        let (_, mix) = open_source(&directory, "试听混音.wav")?;
        let source_frames = mix.duration();
        drop(mix);
        let Format { rate, bits } = self.format;
        let bounds = source_bounds(self.span, source_frames)?;
        let [first, end] = bounds;
        let frames = self.format.frames(end - first);
        let direct = self.format.exact() && bounds == [0, source_frames];
        let revision = render["revision"]
            .as_u64()
            .context("Missing render revision")?;
        let filename = format!(
            "分轨_r{revision}{}_{rate}Hz_{bits}bit.zip",
            span_suffix(self.span)
        );
        let stage = root.join("runtime").join(id);
        let target = root.join("exports").join(id);
        let pending = stage.join(&filename);
        let scratch = stage.join("converted.wav.tmp");
        let receipt = stage.join("渲染报告.json");
        fs::create_dir(&stage)?;
        let result = (|| -> Result<Value> {
            let mut archive = zip::ZipWriter::new(BufWriter::new(fs::File::create(&pending)?));
            let stored =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            let mut files = Vec::new();
            let mut total_clipped = 0;
            let count = self.stems.len() as f64;
            for (index, name) in self.stems.iter().enumerate() {
                let progress = |fraction| {
                    control.progress(
                        "packing_stems",
                        (index as f64 + fraction) / count * 0.98,
                        Some(("", name)),
                    )
                };
                progress(0.)?;
                let (source, mut reader) = open_source(&directory, name)?;
                ensure!(
                    reader.duration() == source_frames,
                    "分轨和混音的采样帧数不一致"
                );
                let (input, clipped) = if direct {
                    (source, 0)
                } else {
                    let clipped = convert(
                        &mut reader,
                        &scratch,
                        self.format,
                        bounds,
                        seed(job, name),
                        |part| progress(part * 0.85),
                    )?;
                    (scratch.clone(), clipped)
                };
                drop(reader);
                let mut file = fs::File::open(&input)?;
                archive.start_file(
                    name,
                    stored.large_file(file.metadata()?.len() >= u64::from(u32::MAX)),
                )?;
                let mut bytes = [0; 64 * 1024];
                loop {
                    control.check()?;
                    let read = file.read(&mut bytes)?;
                    if read == 0 {
                        break;
                    }
                    archive.write_all(&bytes[..read])?;
                }
                drop(file);
                if !direct {
                    fs::remove_file(&scratch)?;
                }
                total_clipped += clipped;
                files.push(json!({"name":name,"clipped_samples":clipped}));
            }
            control.check()?;
            let manifest = json!({"source_render_job_id":job,"project_id":render["project_id"],"revision":revision,
                "sample_rate":rate,"bit_depth":bits,"channels":2,"start_frame":0,"frames":frames,
                "beat_range":self.span.map(|span| span.beats),"source_sample_rate":SOURCE_RATE,"source_frame_range":bounds,
                "bit_exact":self.format.exact(),"dither":self.format.dither(),"files":files,"credits":render["credits"]});
            archive.start_file("分轨清单.json", stored)?;
            archive.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
            archive.finish()?.into_inner()?.sync_all()?;
            let mut result = manifest;
            result["job_id"] = json!(id);
            result["engine"] = json!(ENGINE);
            result["path"] = json!(target.join(&filename));
            result["url"] = json!(format!("/exports/{id}/{filename}"));
            result["clipped_samples"] = json!(total_clipped);
            result["warning"] = json!(if total_clipped > 0 {
                format!("分轨转换共有 {total_clipped} 个采样达到 PCM 幅度上限")
            } else {
                String::new()
            });
            fs::write(&receipt, serde_json::to_vec_pretty(&result)?)?;
            control.commit()?;
            fs::rename(&stage, &target)?;
            Ok(result)
        })();
        if result.is_err() {
            for file in [pending, scratch, receipt] {
                let _ = fs::remove_file(file);
            }
            let _ = fs::remove_dir(&stage);
        }
        result
    }
}

pub(crate) fn export(
    root: &Path,
    render: &Value,
    args: &Value,
    span: Option<BeatSpan>,
) -> Result<Value> {
    let format = Format::read(args)?;
    let Format { rate, bits } = format;
    let stem = args
        .get("stem")
        .map(|value| value.as_str().context("stem must be a string"))
        .transpose()?;
    if let Some(stem) = stem {
        ensure!(
            render["stems"]
                .as_array()
                .is_some_and(|files| files.iter().any(|item| item == stem)),
            "找不到该渲染的分轨"
        );
    }
    let source_name = stem.unwrap_or("试听混音.wav");
    let job = render["job_id"].as_str().context("Missing render job")?;
    let directory = crate::audition::export_directory(root, job)?;
    let (source, mut reader) = open_source(&directory, source_name)?;
    let bounds = source_bounds(span, reader.duration())?;
    let [first, end] = bounds;
    let frames = format.frames(end - first);
    let label: String = source_name
        .trim_end_matches(".wav")
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, ' ' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .take(72)
        .collect();
    let filename = format!("{label}{}_{rate}Hz_{bits}bit.wav", span_suffix(span));
    let id = uid("export");
    let pending = root.join("runtime").join(format!("{id}.wav.tmp"));
    let target_dir = root.join("exports").join(&id);
    let target = target_dir.join(&filename);
    let same_pcm = format.exact();
    let outcome = (|| -> Result<u64> {
        let clipped = if same_pcm && bounds == [0, reader.duration()] {
            drop(reader);
            fs::copy(&source, &pending)?;
            0
        } else {
            convert(
                &mut reader,
                &pending,
                format,
                bounds,
                seed(job, source_name),
                |_| Ok(()),
            )?
        };
        fs::create_dir(&target_dir)?;
        fs::rename(&pending, &target)?;
        Ok(clipped)
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&pending);
        let _ = fs::remove_dir(&target_dir);
    }
    let clipped = outcome?;
    Ok(
        json!({"path":target,"url":format!("/exports/{id}/{filename}"),"job_id":job,
            "project_id":render["project_id"],"revision":render["revision"],"sample_rate":rate,"bit_depth":bits,
            "channels":2,"frames":frames,"stem":stem,"bit_exact":same_pcm,
            "beat_range":span.map(|span| span.beats),"source_sample_rate":SOURCE_RATE,"source_frame_range":bounds,
            "dither":format.dither(),"clipped_samples":clipped,
            "warning":if clipped>0 { format!("转换时有 {clipped} 个采样达到 PCM 幅度上限") } else { String::new() }
        }),
    )
}

fn convert(
    reader: &mut WavReader<impl std::io::Read + std::io::Seek>,
    target: &Path,
    format: Format,
    [first, end]: [u32; 2],
    mut seed: u64,
    mut progress: impl FnMut(f64) -> Result<()>,
) -> Result<u64> {
    let Format { rate, bits } = format;
    let input_frames = u64::from(end - first);
    let frames = format.frames(end - first);
    reader.seek(first)?;
    let mut output = WavWriter::create(
        target,
        WavSpec {
            sample_rate: rate,
            bits_per_sample: bits,
            ..reader.spec()
        },
    )?;
    let mut clipped = 0;
    let half = (1_i32 << (bits - 1)) as f64;
    let mut samples = reader.samples::<i32>();
    let mut read = || -> Result<[f64; 2]> {
        let mut frame = [0.; 2];
        for channel in &mut frame {
            *channel = f64::from(samples.next().context("Rendered WAV ended early")??) / 8_388_608.;
        }
        Ok(frame)
    };
    let phases = (rate != SOURCE_RATE).then(|| {
        // 44.1/48 reduces to 147/160, so every fractional input position is exact.
        (0..147)
            .map(|index| {
                crate::resample::blackman_sinc::<TAPS>(
                    0.95 * f64::from(rate) / f64::from(SOURCE_RATE),
                    index as f64 / 147.,
                )
            })
            .collect::<Vec<_>>()
    });
    let mut input = VecDeque::with_capacity(TAPS);
    let mut head = 0;
    for point in 0..frames {
        if point % 8192 == 0 {
            progress(point as f64 / frames as f64)?;
        }
        let frame = if let Some(phases) = &phases {
            let position = point * 160;
            let middle = position / 147;
            let first = middle.saturating_sub(LEFT as u64);
            let last = (middle + (TAPS - 1 - LEFT) as u64).min(input_frames - 1);
            while head < first {
                input.pop_front();
                head += 1;
            }
            while head + (input.len() as u64) <= last {
                input.push_back(read()?);
            }
            let offset = (first + LEFT as u64 - middle) as usize;
            let weights = &phases[(position % 147) as usize][offset..];
            let mut stereo = [0.; 2];
            for (source, weight) in input.iter().zip(weights) {
                stereo[0] += source[0] * weight;
                stereo[1] += source[1] * weight;
            }
            stereo
        } else {
            read()?
        };
        for sample in frame {
            let noise = if bits == 16 {
                uniform(&mut seed) - uniform(&mut seed)
            } else {
                0.
            };
            let quantized = (sample * half + noise).round();
            let bounded = quantized.clamp(-half, half - 1.);
            clipped += u64::from(quantized != bounded);
            output.write_sample(bounded as i32)?;
        }
    }
    output.finalize()?;
    Ok(clipped)
}

fn seed(job: &str, name: &str) -> u64 {
    let hash = Sha256::digest(format!("{job}/{name}").as_bytes());
    u64::from_le_bytes(*hash.first_chunk::<8>().expect("SHA-256 is 32 bytes"))
}

fn uniform(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (*seed >> 32) as f64 / 4_294_967_296.
}
