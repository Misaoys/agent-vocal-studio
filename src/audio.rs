use crate::model::*;
use anyhow::{ensure, Context, Result};
use std::{
    collections::VecDeque,
    fs,
    io::{BufReader, BufWriter, Read, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
use symphonia::core::{
    codecs::audio::{AudioDecoder, AudioDecoderOptions},
    formats::{probe::Hint, FormatOptions, FormatReader, TrackType},
    io::{MediaSource, MediaSourceStream},
};

pub(crate) const IMPORT_EXTENSIONS: [&str; 3] = ["wav", "mp3", "flac"];
const RATE: u32 = crate::clips::RATE;
const MAX_AUDIO_SECONDS: usize = 900;
const PEAK_BLOCK: u32 = 256;
const PEAK_ASSETS: usize = 8;

type Peak = [f32; 2];
type PeakBlocks = Arc<Mutex<Vec<Option<Peak>>>>;
struct WaveformAsset {
    path: PathBuf,
    modified: Option<SystemTime>,
    bytes: u64,
    blocks: PeakBlocks,
}
static WAVEFORM_ASSETS: Mutex<VecDeque<WaveformAsset>> = Mutex::new(VecDeque::new());

fn peak_blocks(root: &Path, asset: &str, duration: u32) -> Result<PeakBlocks> {
    let path = root.join("assets").join(asset);
    let metadata = fs::metadata(&path)?;
    let (modified, bytes) = (metadata.modified().ok(), metadata.len());
    let mut cache = WAVEFORM_ASSETS.lock().unwrap();
    let found = cache
        .iter()
        .position(|entry| entry.path == path)
        .and_then(|index| cache.remove(index));
    let entry = found
        .filter(|entry| entry.modified == modified && entry.bytes == bytes)
        .unwrap_or_else(|| WaveformAsset {
            path,
            modified,
            bytes,
            blocks: Arc::new(Mutex::new(vec![
                None;
                duration.div_ceil(PEAK_BLOCK) as usize
            ])),
        });
    let blocks = Arc::clone(&entry.blocks);
    cache.push_back(entry);
    if cache.len() > PEAK_ASSETS {
        cache.pop_front();
    }
    Ok(blocks)
}

fn merge_peak(peak: &mut Peak, [a, b]: Peak) {
    peak[0] = peak[0].min(a).min(b);
    peak[1] = peak[1].max(a).max(b);
}

fn window_peaks(
    root: &Path,
    asset: &str,
    start: u32,
    end: u32,
    peaks: &mut [Peak],
) -> Result<(u32, u32)> {
    let mut reader = asset_reader(root, asset)?;
    let duration = reader.duration();
    ensure!(end <= duration, "Clip source frames exceed its WAV asset");
    let cache = peak_blocks(root, asset, duration)?;
    let mut blocks = cache.lock().unwrap();
    let bins = peaks.len() as u64;
    let frames = u64::from(end - start);
    let bin = |frame| (u64::from(frame - start) * bins / frames) as usize;
    let mut frame = start;
    let mut reader_position = 0;
    let mut decoded = 0;
    while frame < end {
        let block = frame / PEAK_BLOCK;
        let limit = ((block + 1) * PEAK_BLOCK).min(end);
        let whole = limit - frame == PEAK_BLOCK;
        let first_bin = bin(frame);
        let last_bin = bin(limit - 1);
        if whole && first_bin == last_bin {
            if let Some(peak) = blocks[block as usize] {
                merge_peak(&mut peaks[first_bin], peak);
                frame = limit;
                continue;
            }
        }
        if frame != reader_position {
            reader.seek(frame)?;
        }
        let mut block_peak = [0.; 2];
        let mut samples = reader.samples::<f32>();
        for position in frame..limit {
            let left = samples.next().context("Truncated audio asset")??;
            let right = samples.next().context("Truncated audio asset")??;
            ensure!(
                left.is_finite() && right.is_finite(),
                "Non-finite audio asset sample"
            );
            if first_bin != last_bin {
                merge_peak(&mut peaks[bin(position)], [left, right]);
            }
            merge_peak(&mut block_peak, [left, right]);
        }
        if first_bin == last_bin {
            merge_peak(&mut peaks[first_bin], block_peak);
        }
        if whole {
            blocks[block as usize] = Some(block_peak);
        }
        decoded += limit - frame;
        reader_position = limit;
        frame = limit;
    }
    Ok((duration, decoded))
}

#[cfg(test)]
fn resample(data: &[[f32; 2]], rate: u32) -> Vec<[f32; 2]> {
    if rate == 48000 {
        return data.to_vec();
    }
    let kernel = Sinc::new(rate);
    (0..kernel.output_frames(data.len()))
        .map(|frame| kernel.frame(frame, |index| data.get(index).copied()))
        .collect()
}

fn checked_rate(rate: u32) -> Result<()> {
    ensure!(
        (8000..=192000).contains(&rate),
        "Audio sample rate must be 8–192 kHz"
    );
    Ok(())
}

fn write_normalized(
    destination: &Path,
    rate: u32,
    mut next: impl FnMut() -> Result<Option<[f32; 2]>>,
) -> Result<usize> {
    checked_rate(rate)?;
    let mut read = || -> Result<Option<[f32; 2]>> {
        let Some(frame) = next()? else {
            return Ok(None);
        };
        ensure!(
            frame
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() < 100.),
            "Invalid audio sample"
        );
        Ok(Some(frame))
    };
    let mut writer = hound::WavWriter::new(
        BufWriter::new(fs::File::create_new(destination)?),
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let mut write = |frame: [f32; 2]| -> Result<()> {
        for sample in frame {
            writer.write_sample(sample)?;
        }
        Ok(())
    };
    let mut frames = 0;
    if rate == RATE {
        while let Some(frame) = read()? {
            write(frame)?;
            frames += 1;
        }
    } else {
        let kernel = Sinc::new(rate);
        let mut window = VecDeque::with_capacity(64);
        let (mut first, mut consumed, mut ended) = (0, 0, false);
        loop {
            let base = (frames as f64 * kernel.ratio).floor() as usize;
            let low = base.saturating_sub(31);
            while first < low && first < consumed {
                window.pop_front();
                first += 1;
            }
            while consumed <= base + 32 && !ended {
                if let Some(input) = read()? {
                    window.push_back(input);
                    consumed += 1;
                } else {
                    ended = true;
                }
            }
            if ended && frames >= kernel.output_frames(consumed) {
                break;
            }
            write(kernel.frame(frames, |index| {
                index
                    .checked_sub(first)
                    .and_then(|slot| window.get(slot).copied())
            }))?;
            frames += 1;
        }
    }
    ensure!(
        frames > 0,
        "Audio is empty or shorter than one normalized frame"
    );
    writer.finalize()?;
    Ok(frames)
}

fn wav_frames(mut wav: hound::WavReader<impl Read>, destination: &Path) -> Result<usize> {
    let spec = wav.spec();
    ensure!(
        [1, 2].contains(&spec.channels) && [16, 24, 32].contains(&spec.bits_per_sample),
        "Use mono/stereo WAV at 16, 24 or 32 bits"
    );
    ensure!(
        wav.duration() as u64 <= u64::from(spec.sample_rate) * MAX_AUDIO_SECONDS as u64,
        "Audio exceeds 15 minutes"
    );
    let scale = 2f32.powi(i32::from(spec.bits_per_sample) - 1);
    let mut samples: Box<dyn Iterator<Item = std::result::Result<f32, hound::Error>> + '_> =
        if spec.sample_format == hound::SampleFormat::Float {
            Box::new(wav.samples::<f32>())
        } else {
            Box::new(
                wav.samples::<i32>()
                    .map(|sample| sample.map(|value| value as f32 / scale)),
            )
        };
    write_normalized(destination, spec.sample_rate, || {
        let Some(left) = samples.next().transpose()? else {
            return Ok(None);
        };
        let right = if spec.channels == 1 {
            left
        } else {
            samples
                .next()
                .context("WAV contains a partial stereo frame")??
        };
        Ok(Some([left, right]))
    })
}

struct CompressedAudio {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    rate: u32,
    channels: usize,
    samples: Vec<f32>,
    offset: usize,
    frames: usize,
}

impl CompressedAudio {
    fn open(source: Box<dyn MediaSource>, filename: &str) -> Result<Self> {
        let stream = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = Path::new(filename)
            .extension()
            .and_then(|part| part.to_str())
        {
            hint.with_extension(&ext.to_ascii_lowercase());
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, stream, FormatOptions::default(), Default::default())
            .context("Unable to open MP3 or FLAC audio")?;
        let track = format
            .default_track(TrackType::Audio)
            .context("No audio stream found")?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .context("Audio codec metadata missing")?;
        let rate = params.sample_rate.context("Audio sample rate missing")?;
        checked_rate(rate)?;
        let channels = params
            .channels
            .as_ref()
            .context("Audio channel layout missing")?
            .count();
        ensure!([1, 2].contains(&channels), "Import mono or stereo audio");
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(
                params,
                &AudioDecoderOptions::default().gapless(true).verify(true),
            )
            .context("Unable to initialize the audio decoder")?;
        Ok(Self {
            format,
            decoder,
            track_id,
            rate,
            channels,
            samples: Vec::new(),
            offset: 0,
            frames: 0,
        })
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>> {
        while self.offset == self.samples.len() {
            let Some(packet) = self.format.next_packet().context("Damaged audio stream")? else {
                ensure!(
                    self.decoder.finalize().verify_ok != Some(false),
                    "Audio stream checksum mismatch"
                );
                return Ok(None);
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let audio = self
                .decoder
                .decode(&packet)
                .context("Damaged audio packet")?;
            ensure!(
                audio.spec().rate() == self.rate
                    && audio.spec().channels().count() == self.channels,
                "Audio changes sample rate or channel count"
            );
            self.frames += audio.samples_interleaved() / self.channels;
            ensure!(
                self.frames <= self.rate as usize * MAX_AUDIO_SECONDS,
                "Audio exceeds 15 minutes"
            );
            self.samples.resize(audio.samples_interleaved(), 0.);
            audio.copy_to_slice_interleaved(&mut self.samples);
            self.offset = 0;
        }
        let left = self.samples[self.offset];
        let right = if self.channels == 1 {
            left
        } else {
            self.samples[self.offset + 1]
        };
        self.offset += self.channels;
        Ok(Some([left, right]))
    }
}

pub(crate) struct Sinc {
    ratio: f64,
    table: Vec<[f64; 64]>,
}

impl Sinc {
    pub(crate) fn new(rate: u32) -> Self {
        let ratio = rate as f64 / 48000.;
        let cutoff = (1. / ratio).min(1.) * 0.94;
        let table = (0..1024)
            .map(|phase| crate::resample::blackman_sinc(cutoff, phase as f64 / 1024.))
            .collect();
        Self { ratio, table }
    }

    pub(crate) fn output_frames(&self, input: usize) -> usize {
        (input as f64 / self.ratio).round() as usize
    }

    pub(crate) fn frame(
        &self,
        frame: usize,
        sample_at: impl Fn(usize) -> Option<[f32; 2]>,
    ) -> [f32; 2] {
        let x = frame as f64 * self.ratio;
        let base = x.floor() as isize;
        let phase = ((x - x.floor()) * 1024.).floor() as usize;
        let mut sample = [0f64; 2];
        for (k, weight) in self.table[phase.min(1023)].iter().enumerate() {
            let index = base + k as isize - 31;
            if let Some([left, right]) = usize::try_from(index).ok().and_then(&sample_at) {
                sample[0] += f64::from(left) * weight;
                sample[1] += f64::from(right) * weight;
            }
        }
        [sample[0] as f32, sample[1] as f32]
    }
}

#[cfg(windows)]
pub(crate) fn recording_asset(raw: &Path, destination: &Path, offset: i64) -> Result<f64> {
    let mut wav = hound::WavReader::open(raw)?;
    let spec = wav.spec();
    let frames = wav.duration() as usize;
    ensure!(
        spec.channels == 2
            && spec.bits_per_sample == 32
            && spec.sample_format == hound::SampleFormat::Float
            && (8000..=192_000).contains(&spec.sample_rate),
        "无效的录音格式"
    );
    ensure!(
        frames > 0 && frames <= spec.sample_rate as usize * 900,
        "录音为空或超过 15 分钟"
    );
    if spec.sample_rate == 48_000 && offset == 0 {
        fs::copy(raw, destination)?;
        return Ok(frames as f64 / 48_000.);
    }
    ensure!(
        (-i64::from(spec.sample_rate) * 2..frames as i64).contains(&offset),
        "录音对齐位置无效"
    );
    let skip = offset.max(0) as usize;
    let pad = offset.saturating_neg().max(0) as usize;
    let frames = frames - skip + pad;
    wav.seek(skip as u32)?;
    let mut samples = wav.samples::<f32>();
    let mut index = 0;
    let output = write_normalized(destination, spec.sample_rate, || {
        if index == frames {
            return Ok(None);
        }
        index += 1;
        if index <= pad {
            return Ok(Some([0.; 2]));
        }
        let left = samples.next().context("录音文件提前结束")??;
        let right = samples.next().context("录音文件提前结束")??;
        Ok(Some([left, right]))
    })?;
    Ok(output as f64 / f64::from(RATE))
}

pub(crate) fn ingest(
    root: &Path,
    mut source: Box<dyn MediaSource>,
    filename: &str,
    start: f64,
) -> Result<AudioClip> {
    let mut header = [0; 12];
    source
        .read_exact(&mut header)
        .context("Incomplete audio file")?;
    source.seek(SeekFrom::Start(0))?;
    let is_wav = &header[..4] == b"RIFF" && &header[8..] == b"WAVE";
    if is_wav {
        return ingest_wav(root, BufReader::new(source), start);
    }
    let mut compressed = CompressedAudio::open(source, filename)?;
    publish_asset(root, start, |temporary| {
        write_normalized(temporary, compressed.rate, || compressed.next_frame())
    })
}

pub(crate) fn ingest_wav(root: &Path, source: impl Read, start: f64) -> Result<AudioClip> {
    ingest_wave_reader(root, hound::WavReader::new(source)?, start)
}

pub(crate) fn ingest_wave_reader(
    root: &Path,
    wav: hound::WavReader<impl Read>,
    start: f64,
) -> Result<AudioClip> {
    publish_asset(root, start, |temporary| wav_frames(wav, temporary))
}

fn publish_asset(
    root: &Path,
    start: f64,
    write: impl FnOnce(&Path) -> Result<usize>,
) -> Result<AudioClip> {
    let asset = format!("{}.wav", uid("audio"));
    let directory = root.join("assets");
    fs::create_dir_all(&directory)?;
    let destination = directory.join(&asset);
    let temporary = destination.with_extension("import.tmp");
    let result = (|| -> Result<AudioClip> {
        let frames = write(&temporary)?;
        #[cfg(windows)]
        crate::model::move_file_windows(&temporary, &destination, false)?;
        #[cfg(not(windows))]
        fs::rename(&temporary, &destination)?;
        Ok(AudioClip {
            asset,
            start,
            seconds: frames as f64 / f64::from(RATE),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn asset_reader(
    root: &Path,
    asset: &str,
) -> Result<hound::WavReader<BufReader<fs::File>>> {
    crate::clips::validate_asset_name(asset)?;
    let reader = hound::WavReader::open(root.join("assets").join(asset))
        .context("Audio asset missing; use the portable avpack to restore it")?;
    let format = reader.spec();
    ensure!(
        format.sample_rate == crate::clips::RATE
            && format.channels == 2
            && format.bits_per_sample == 32
            && format.sample_format == hound::SampleFormat::Float,
        "Invalid normalized audio asset format"
    );
    Ok(reader)
}
pub(crate) fn visit_clip(
    root: &Path,
    clip: &crate::clips::Clip,
    mut visit: impl FnMut(u32, [f32; 2]) -> Result<()>,
) -> Result<()> {
    let mut reader = asset_reader(root, &clip.asset)?;
    let duration = reader.duration();
    ensure!(
        clip.offset <= duration && clip.frames <= duration - clip.offset,
        "Clip source frames exceed its WAV asset"
    );
    reader.seek(clip.offset)?;
    let mut samples = reader.samples::<f32>();
    for frame in 0..clip.frames {
        let left = samples.next().context("Truncated audio asset")??;
        let right = samples.next().context("Truncated audio asset")??;
        ensure!(
            left.is_finite() && right.is_finite(),
            "Non-finite audio asset sample"
        );
        visit(frame, [left, right])?;
    }
    Ok(())
}
pub(crate) fn clip_waveform(
    root: &Path,
    clip: &crate::clips::Clip,
    clock: &crate::timeline::Timeline,
    frame_range: Option<[u32; 2]>,
    include_transients: bool,
) -> Result<serde_json::Value> {
    let [first, end] = frame_range.unwrap_or([0, clip.frames]);
    ensure!(first < end && end <= clip.frames, "源采样范围超出片段");
    let frames = end - first;
    let bins = 1200.min(frames) as usize;
    let mut peaks = vec![[0f32; 2]; bins];
    let read_start = if include_transients {
        crate::transient::analysis_start(first)
    } else {
        first
    };
    let mut source = clip.clone();
    source.offset += read_start;
    source.frames = end - read_start;
    let mut detect = include_transients.then(crate::transient::Detector::default);
    let asset_frames = if let Some(detector) = &mut detect {
        visit_clip(root, &source, |frame, samples| {
            detector.push(samples);
            if let Some(relative) = (read_start + frame).checked_sub(first) {
                let index = (u64::from(relative) * bins as u64 / u64::from(frames)) as usize;
                merge_peak(&mut peaks[index], samples);
            }
            Ok(())
        })?;
        asset_reader(root, &clip.asset)?.duration()
    } else {
        window_peaks(
            root,
            &clip.asset,
            clip.offset + first,
            clip.offset + end,
            &mut peaks,
        )?
        .0
    };
    let mut result = serde_json::json!({
        "clip_id": clip.id, "seconds": f64::from(frames)/f64::from(RATE),
        "source_offset": clip.offset+first, "frames": frames, "asset_frames": asset_frames,
        "sample_rate": RATE, "frame_range": [first,end], "peaks": peaks
    });
    if let Some(detector) = detect {
        let (attacks, limited) = detector.finish(read_start, [first, end]);
        result["transients"] = serde_json::Value::Array(attacks.into_iter().map(|attack| {
            serde_json::json!({"frame":attack.frame,"beat":clip.beat_at_source(clock,attack.frame),
                "contrast_db":(attack.contrast * std::f32::consts::LOG10_E * 2000.).round()/100.})
        }).collect());
        result["transients_limited"] = limited.into();
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;
    #[test]
    fn waveform_block_reuse_preserves_exact_unaligned_trim_bins() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_waveform"));
        let folder = root.join("assets");
        fs::create_dir_all(&folder)?;
        let asset = format!("{}.wav", uid("audio"));
        let seconds = std::env::var("AVS_WAVE_BENCH_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(12)
            .clamp(12, 120);
        let duration = RATE * seconds;
        let result = (|| -> Result<()> {
            let mut wav = hound::WavWriter::create(
                folder.join(&asset),
                hound::WavSpec {
                    channels: 2,
                    sample_rate: RATE,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )?;
            for i in 0..duration {
                wav.write_sample((i % 997) as f32 / 997. - 0.5)?;
                wav.write_sample(0.3 - (i % 733) as f32 / 733.)?;
            }
            wav.finalize()?;
            let start = 173;
            let end = duration - 923;
            let frames = end - start;
            let reference_started = std::time::Instant::now();
            let mut reader = asset_reader(&root, &asset)?;
            reader.seek(start)?;
            let mut reference = [[0.; 2]; 1200];
            let mut samples = reader.samples::<f32>();
            for frame in 0..frames {
                let index = (u64::from(frame) * 1200 / u64::from(frames)) as usize;
                let left = samples.next().context("fixture left")??;
                let right = samples.next().context("fixture right")??;
                merge_peak(&mut reference[index], [left, right]);
            }
            let reference_ms = reference_started.elapsed().as_secs_f64() * 1000.;
            drop(reader);
            let cold_started = std::time::Instant::now();
            let mut whole = [[0.; 2]; 1200];
            assert_eq!(
                window_peaks(&root, &asset, 0, duration, &mut whole)?,
                (duration, duration)
            );
            let cold_ms = cold_started.elapsed().as_secs_f64() * 1000.;
            let cached_started = std::time::Instant::now();
            let mut trimmed = [[0.; 2]; 1200];
            let (_, decoded) = window_peaks(&root, &asset, start, end, &mut trimmed)?;
            let cached_ms = cached_started.elapsed().as_secs_f64() * 1000.;
            assert_eq!(trimmed, reference);
            assert!(
                decoded < frames * 2 / 3,
                "decoded {decoded} of {frames} frames"
            );
            println!(
                "WAVEFORM_REPORT={}",
                serde_json::json!({
                    "fixture_seconds":seconds,"full_scan_frames":frames,"trim_decoded_frames":decoded,
                    "reference_ms":reference_ms,"cold_full_asset_ms":cold_ms,"warm_trim_ms":cached_ms,
                    "exact_peaks":true
                })
            );
            Ok(())
        })();
        fs::remove_file(folder.join(asset))?;
        fs::remove_dir(folder)?;
        fs::remove_dir(root)?;
        result
    }
    #[test]
    fn sinc_preserves_frequency_and_duration() {
        let sr = 44100;
        let data = (0..sr)
            .map(|i| {
                let v = (2. * PI * 440. * i as f64 / sr as f64).sin() as f32;
                [v, v]
            })
            .collect::<Vec<_>>();
        let out = resample(&data, sr);
        assert_eq!(out.len(), 48000);
        let error = out
            .iter()
            .enumerate()
            .skip(100)
            .take(47000)
            .map(|(i, x)| (x[0] as f64 - (2. * PI * 440. * i as f64 / 48000.).sin()).powi(2))
            .sum::<f64>()
            / 47000.;
        assert!(error < 1e-5, "Resampling error {error}");
    }
}
