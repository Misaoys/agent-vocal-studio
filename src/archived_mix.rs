//! A revision-bound copy of the exported stereo master, kept outside the score tracks.
use crate::{
    loudness::{Meter, Report},
    model::{field, uid, Project},
    source_cache::frame_count,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::Path,
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

const MANIFEST: &str = "reference/mix.json";
const WAVE: &str = "reference/mix.wav";
const MAX_WAVE: u64 = 120_000_000;
const MAX_MANIFEST: u64 = 64_000;
const MAX_CREDITS: usize = 16_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    engine: String,
    score_sha256: String,
    wave_sha256: String,
    credits: String,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn limited(reader: impl Read, limit: u64, label: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "{label} exceeds size limit");
    Ok(bytes)
}

pub(crate) fn write_bundle<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    root: &Path,
    project: &Project,
    score: &[u8],
    job_id: &str,
) -> Result<()> {
    let job = crate::jobs::read(root, job_id)?;
    ensure!(job["status"] == "complete", "RENDER_NOT_COMPLETE");
    let result = &job["result"];
    ensure!(
        result["project_id"] == project.id && result["revision"] == project.revision,
        "RENDER_REVISION_MISMATCH: 选择本版本的已完成混音"
    );
    ensure!(
        result["audio_verified_non_silent"] == true,
        "The chosen mix must contain audio"
    );
    let engine = field(result, "engine")?;
    ensure!(
        crate::engine::RENDER_ENGINES.contains(&engine),
        "Unknown mix engine"
    );
    let dir = root.join("exports").join(job_id);
    let wave = limited(
        File::open(dir.join("试听混音.wav"))?,
        MAX_WAVE,
        "Reference WAV",
    )?;
    let credits = String::from_utf8(limited(
        File::open(dir.join("音源署名.txt"))?,
        MAX_CREDITS as u64,
        "Source credits",
    )?)?;
    let manifest = Manifest {
        version: 1,
        engine: engine.into(),
        score_sha256: sha256(score),
        wave_sha256: sha256(&wave),
        credits,
    };
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(MANIFEST, options)?;
    serde_json::to_writer(&mut *zip, &manifest)?;
    zip.start_file(WAVE, options)?;
    zip.write_all(&wave)?;
    Ok(())
}

pub(crate) struct ArchivedMix {
    manifest: Manifest,
    wave: Vec<u8>,
    frames: usize,
    peak: f64,
    level: Report,
}

impl ArchivedMix {
    pub(crate) fn from_bundle<R: Read + std::io::Seek>(
        archive: &mut ZipArchive<R>,
        project: &Project,
        score: &[u8],
    ) -> Result<Option<Self>> {
        let has_manifest = archive.index_for_name(MANIFEST).is_some();
        ensure!(
            has_manifest == archive.index_for_name(WAVE).is_some(),
            "Incomplete reference mix in the project package"
        );
        if !has_manifest {
            return Ok(None);
        }
        let data = limited(
            archive.by_name(MANIFEST)?,
            MAX_MANIFEST,
            "Reference metadata",
        )?;
        let manifest: Manifest = serde_json::from_slice(&data)?;
        ensure!(
            manifest.version == 1
                && crate::engine::RENDER_ENGINES.contains(&manifest.engine.as_str()),
            "Unsupported reference mix format"
        );
        ensure!(
            manifest.credits.len() <= MAX_CREDITS,
            "Source credits exceed size limit"
        );
        ensure!(
            manifest.score_sha256 == sha256(score),
            "Reference project checksum mismatch"
        );
        let wave = limited(archive.by_name(WAVE)?, MAX_WAVE, "Reference WAV")?;
        ensure!(
            manifest.wave_sha256 == sha256(&wave),
            "Reference WAV checksum mismatch"
        );

        let mut wav = hound::WavReader::new(Cursor::new(&wave))?;
        let spec = wav.spec();
        ensure!(
            spec.channels == 2
                && spec.sample_rate == 48000
                && spec.sample_format == hound::SampleFormat::Int
                && spec.bits_per_sample == 24,
            "Reference WAV must be stereo 24-bit PCM at 48 kHz"
        );
        let frames = wav.duration() as usize;
        ensure!(
            frames > 0 && frames == frame_count(project),
            "Reference WAV duration does not match the project"
        );
        ensure!(wave.len() >= frames * 6, "Reference WAV is truncated");
        let mut meter = Meter::default();
        let mut peak = 0i32;
        let mut samples = wav.samples::<i32>();
        for _ in 0..frames {
            let left = samples.next().context("Missing left reference sample")??;
            let right = samples.next().context("Missing right reference sample")??;
            peak = peak.max(left.abs()).max(right.abs());
            meter.push([left as f32 / 8_388_608., right as f32 / 8_388_608.]);
        }
        ensure!(
            samples.next().is_none() && peak > 0,
            "Reference WAV is invalid or silent"
        );
        let level = meter.finish();
        Ok(Some(Self {
            manifest,
            wave,
            frames,
            peak: f64::from(peak) / 8_388_608.,
            level,
        }))
    }

    pub(crate) fn publish(self, root: &Path, project: &Project) -> Result<Value> {
        ensure!(
            self.frames == frame_count(project),
            "Imported project timing changed"
        );
        let id = archive_job_id(&project.id);
        let directory = root.join("exports").join(&id);
        let staged = root.join("runtime").join(uid("archive_stage"));
        let result = json!({
            "job_id":id,"archived_mix":true,"engine":self.manifest.engine,"project_id":project.id,
            "revision":project.revision,"sample_rate":48000,"channels":2,"seconds":self.frames as f64/48000.,
            "loudness":self.level,"peak_dbfs":20.*self.peak.log10(),"audio_verified_non_silent":true,
            "mix_url":format!("/exports/{id}/试听混音.wav"),"directory":directory,"stems":[]
        });
        fs::create_dir_all(root.join("exports"))?;
        fs::create_dir(&staged)?;
        let committed = (|| -> Result<()> {
            fs::write(staged.join("试听混音.wav"), self.wave)?;
            fs::write(staged.join("音源署名.txt"), self.manifest.credits)?;
            fs::write(
                staged.join("渲染报告.json"),
                serde_json::to_vec_pretty(&result)?,
            )?;
            fs::rename(&staged, &directory)?;
            Ok(())
        })();
        if committed.is_err() {
            for name in ["试听混音.wav", "音源署名.txt", "渲染报告.json"] {
                let _ = fs::remove_file(staged.join(name));
            }
            let _ = fs::remove_dir(staged);
        }
        committed?;
        Ok(result)
    }
}

fn archive_job_id(project: &str) -> String {
    format!("archive_{project}")
}

pub(crate) fn existing(root: &Path, project: &Project) -> Result<Option<Value>> {
    let id = archive_job_id(&project.id);
    let dir = root.join("exports").join(&id);
    if !dir.join("渲染报告.json").is_file() || !dir.join("试听混音.wav").is_file() {
        return Ok(None);
    }
    let job = crate::jobs::read(root, &id)?;
    let result = &job["result"];
    ensure!(
        job["status"] == "complete"
            && result["job_id"] == id
            && result["project_id"] == project.id
            && result["archived_mix"] == true,
        "Invalid reference mix receipt"
    );
    Ok((result["revision"] == project.revision).then_some(job))
}
