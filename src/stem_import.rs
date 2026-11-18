use anyhow::{ensure, Context, Result};
use hound::{SampleFormat, WavReader, WavSpec};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

const MANIFEST: &str = "分轨清单.json";
const MAX_MANIFEST: u64 = 1_000_000;
const MAX_WAV_HEADER: u64 = 1_048_576;

#[derive(Deserialize)]
struct Manifest {
    sample_rate: u32,
    bit_depth: u16,
    channels: u16,
    frames: u32,
    start_frame: u32,
    beat_range: Option<[f64; 2]>,
    files: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
}

pub(crate) struct StemArchive {
    zip: ZipArchive<fs::File>,
    manifest: Manifest,
    sha256: Option<String>,
}

impl StemArchive {
    pub(crate) fn open(path: &Path, hash: bool) -> Result<Self> {
        ensure!(path.is_absolute(), "请提供分轨 ZIP 的绝对路径");
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // Allow readers while keeping the submitted file stable.
        }
        let mut file = options.open(path)?;
        ensure!(file.metadata()?.is_file(), "请选择分轨 ZIP");
        let sha256 = if hash {
            let mut hash = Sha256::new();
            std::io::copy(&mut file, &mut hash)?;
            file.rewind()?;
            Some(format!("{:x}", hash.finalize()))
        } else {
            None
        };
        let mut zip = ZipArchive::new(file)?;
        let manifest: Manifest = {
            let member = zip.by_name(MANIFEST).context("ZIP 缺少 Studio 分轨清单")?;
            ensure!(member.size() <= MAX_MANIFEST, "分轨清单过大");
            let mut raw = Vec::new();
            member.take(MAX_MANIFEST + 1).read_to_end(&mut raw)?;
            ensure!(raw.len() as u64 <= MAX_MANIFEST, "分轨清单过大");
            serde_json::from_slice(&raw).context("分轨清单无效")?
        };
        ensure!(
            manifest.start_frame == 0
                && manifest.channels == 2
                && [44100, 48000].contains(&manifest.sample_rate)
                && [16, 24].contains(&manifest.bit_depth)
                && (1..=manifest.sample_rate * 900).contains(&manifest.frames),
            "分轨清单的采样格式或时长无效"
        );
        if let Some([first, end]) = manifest.beat_range {
            ensure!(
                first.is_finite()
                    && end.is_finite()
                    && first >= 0.
                    && end > first
                    && end <= 50_000.,
                "分轨清单的拍范围无效"
            );
        }
        ensure!(
            (1..=crate::model::TRACK_LIMIT).contains(&manifest.files.len()),
            "分轨 ZIP 必须包含 1–32 条分轨"
        );
        let mut names = HashSet::new();
        for entry in &manifest.files {
            let name = entry.name.as_str();
            ensure!(
                name.len() > 4
                    && name.ends_with(".wav")
                    && !name.contains(['/', '\\', ':'])
                    && !name.chars().any(char::is_control)
                    && name.chars().count() <= 104
                    && names.insert(name.to_lowercase()),
                "分轨清单包含无效或重复的文件名"
            );
        }
        ensure!(
            zip.len() == manifest.files.len() + 1
                && zip.file_names().all(|name| name == MANIFEST
                    || manifest.files.iter().any(|entry| entry.name == name)),
            "ZIP 内文件与分轨清单不一致"
        );
        Ok(Self {
            zip,
            manifest,
            sha256,
        })
    }

    pub(crate) fn start(&self, explicit: Option<f64>) -> f64 {
        explicit.unwrap_or_else(|| self.manifest.beat_range.map_or(0., |range| range[0]))
    }

    pub(crate) fn frames(&self) -> u64 {
        (u64::from(self.manifest.frames) * 48000 + u64::from(self.manifest.sample_rate / 2))
            / u64::from(self.manifest.sample_rate)
    }

    pub(crate) fn verify_sha256(&self, expected: &str) -> Result<()> {
        ensure!(
            self.sha256
                .as_deref()
                .is_some_and(|hash| hash.eq_ignore_ascii_case(expected)),
            "SOURCE_CHANGED: 分轨 ZIP 已更新，请重新预览"
        );
        Ok(())
    }

    pub(crate) fn len(&self) -> usize {
        self.manifest.files.len()
    }

    fn spec(&self) -> WavSpec {
        WavSpec {
            channels: 2,
            sample_rate: self.manifest.sample_rate,
            bits_per_sample: self.manifest.bit_depth,
            sample_format: SampleFormat::Int,
        }
    }

    fn wave_limit(&self) -> u64 {
        u64::from(self.manifest.frames) * 2 * u64::from(self.manifest.bit_depth / 8)
            + MAX_WAV_HEADER
    }

    pub(crate) fn preview(&mut self, start: f64) -> Result<Value> {
        let (spec, frames, limit) = (self.spec(), self.manifest.frames, self.wave_limit());
        for entry in &self.manifest.files {
            let member = self.zip.by_name(&entry.name)?;
            ensure!(member.size() <= limit, "分轨文件过大");
            let wav = WavReader::new(member.take(limit))?;
            ensure!(
                wav.spec() == spec && wav.duration() == frames,
                "分轨的采样格式或长度与清单不一致"
            );
        }
        Ok(
            json!({"dry_run":true,"source_sha256":self.sha256,"start":start,"frames":self.frames(),
            "sample_rate":48000,"tracks":self.manifest.files.iter().map(|entry|track_name(&entry.name)).collect::<Vec<_>>() }),
        )
    }

    pub(crate) fn decode(&mut self, root: &Path, start: f64) -> Result<StagedStems> {
        let (spec, frames, normalized, limit) = (
            self.spec(),
            self.manifest.frames,
            self.frames(),
            self.wave_limit(),
        );
        let mut staged = StagedStems {
            root: root.into(),
            tracks: Vec::new(),
        };
        for entry in &self.manifest.files {
            let member = self.zip.by_name(&entry.name)?;
            ensure!(member.size() <= limit, "分轨文件过大");
            let mut limited = member.take(limit + 1);
            let wav = WavReader::new(&mut limited)?;
            ensure!(
                wav.spec() == spec && wav.duration() == frames,
                "分轨的采样格式或长度与清单不一致"
            );
            let audio = crate::audio::ingest_wave_reader(root, wav, start)?;
            let actual_frames = (audio.seconds * 48000.).round() as u64;
            staged.tracks.push((track_name(&entry.name), audio));
            std::io::copy(&mut limited, &mut std::io::sink())?;
            ensure!(limited.limit() > 0, "分轨文件过大");
            ensure!(actual_frames == normalized, "分轨标准化后的帧数不一致");
        }
        Ok(staged)
    }
}

fn track_name(name: &str) -> String {
    name.trim_end_matches(".wav").to_owned()
}

pub(crate) struct StagedStems {
    root: PathBuf,
    pub(crate) tracks: Vec<(String, crate::model::AudioClip)>,
}
impl StagedStems {
    pub(crate) fn committed(mut self) {
        self.tracks.clear();
    }
}
impl Drop for StagedStems {
    fn drop(&mut self) {
        for (_, clip) in &self.tracks {
            let _ = fs::remove_file(self.root.join("assets").join(&clip.asset));
        }
    }
}
