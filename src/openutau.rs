use crate::{
    jobs::Control,
    model::{identifier, uid, Note, Project, Track},
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const TICKS_PER_SECOND: f64 = 2400.; // OpenUtau: 480 ticks per quarter at a fixed 300 BPM.
pub(crate) const DEFAULT_PROFILE: &str = "default";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    singer: String,
    name: String,
    terms_url: String,
    phonemizer: Phonemizer,
    #[serde(default)]
    lyric_suffix: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phonemizer {
    Direct,
    JapanesePresamp,
}

impl Phonemizer {
    fn class(&self) -> Option<&'static str> {
        match self {
            Self::Direct => None,
            Self::JapanesePresamp => Some("OpenUtau.Plugin.Builtin.JapanesePresampPhonemizer"),
        }
    }
}

pub(crate) struct Engine {
    executable: PathBuf,
    bank: PathBuf,
    settings: Settings,
    configuration: Vec<u8>,
}

impl Engine {
    pub(crate) fn load(root: &Path, profile: &str) -> Result<Self> {
        identifier(profile)?;
        let base = root.join("runtime/openutau");
        let executable = base.join("Studio.OpenUtau.exe");
        ensure!(executable.is_file(), "找不到 Studio 的 OpenUtau 歌声适配器");
        let path = if profile == DEFAULT_PROFILE {
            base.join("voice.json")
        } else {
            base.join("voices").join(format!("{profile}.json"))
        };
        let metadata = path
            .metadata()
            .with_context(|| format!("本机未配置 OpenUtau 声库 {profile}"))?;
        ensure!(
            metadata.is_file() && metadata.len() <= 16_384,
            "OpenUtau 声库配置必须是普通 JSON 文件"
        );
        let configuration = fs::read(path)?;
        let settings: Settings =
            serde_json::from_slice(&configuration).context("OpenUtau 声库配置无效")?;
        ensure!(
            settings.lyric_suffix.chars().count() <= 16
                && !settings
                    .lyric_suffix
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '[' | ']'))
                && (settings.lyric_suffix.is_empty()
                    || matches!(settings.phonemizer, Phonemizer::Direct)),
            "假名后缀只可用于直接匹配声库标签"
        );
        let id = Path::new(&settings.singer);
        ensure!(
            id.components().count() == 1
                && matches!(id.components().next(), Some(Component::Normal(_)))
                && !settings.name.trim().is_empty()
                && settings.terms_url.starts_with("https://"),
            "请配置 OpenUtau 声库文件夹、名称和许可网址"
        );
        let bank = base.join("Singers").join(id);
        ensure!(
            bank.join("character.txt").is_file(),
            "找不到配置的 OpenUtau 声库"
        );
        Ok(Self {
            executable,
            bank,
            settings,
            configuration,
        })
    }

    pub(crate) fn credit(&self) -> String {
        format!(
            "OpenUtau / {} · {}",
            self.settings.name, self.settings.terms_url
        )
    }

    pub(crate) fn describe(&self, profile: &str) -> Value {
        json!({"engine":"openutau","id":profile,"name":self.settings.name,"singer":self.settings.singer,"terms_url":self.settings.terms_url})
    }

    pub(crate) fn fingerprint(&self, control: &Control) -> Result<String> {
        let mut hash = Sha256::new();
        hash.update(b"studio-openutau-score-v1");
        hash.update(&self.configuration);
        for file in [
            "Studio.OpenUtau.dll",
            "OpenUtau.Core.dll",
            "OpenUtau.Plugin.Builtin.dll",
            "worldline.dll",
        ] {
            hash.update(fs::read(self.executable.with_file_name(file))?);
        }
        let mut dirs = vec![self.bank.clone()];
        while let Some(directory) = dirs.pop() {
            control.check()?;
            let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    dirs.push(entry.path());
                } else if kind.is_file() {
                    control.check()?;
                    let path = entry.path();
                    let label = path.strip_prefix(&self.bank)?.to_string_lossy();
                    hash.update((label.len() as u64).to_le_bytes());
                    hash.update(label.as_bytes());
                    let mut file = fs::File::open(path)?;
                    hash.update(file.metadata()?.len().to_le_bytes());
                    let mut block = [0; 65536];
                    loop {
                        let bytes = file.read(&mut block)?;
                        if bytes == 0 {
                            break;
                        }
                        hash.update(&block[..bytes]);
                        control.check()?;
                    }
                }
            }
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub(crate) fn render(
        &self,
        root: &Path,
        project: &Project,
        track: &Track,
        notes: &[Note],
        origin: f64,
        control: &Control,
    ) -> Result<Vec<f32>> {
        let score = self.score(project, track, notes, origin)?;
        let directory = root.join("runtime/render_work").join(uid("openutau"));
        fs::create_dir_all(directory.parent().unwrap())?;
        fs::create_dir(&directory)?;
        let _cleanup = Work(directory.clone());
        let input = directory.join("input.ustx");
        let output = directory.join("voice.wav");
        let errors = directory.join("stderr.txt");
        fs::write(&input, serde_json::to_vec(&score)?)?;
        let mut command = Command::new(&self.executable);
        command
            .arg("render")
            .arg(&input)
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(&errors)?);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        control.check()?;
        let mut child = command.spawn().context("无法启动 OpenUtau 歌声适配器")?;
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let state = (|| -> Result<_> {
                control.check()?;
                ensure!(Instant::now() < deadline, "OpenUtau 歌声合成超时");
                Ok(child.try_wait()?)
            })();
            match state {
                Ok(Some(status)) => {
                    ensure!(
                        status.success(),
                        "OpenUtau：{}",
                        fs::read_to_string(&errors).unwrap_or_default().trim()
                    );
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            }
        }
        let mut wave = hound::WavReader::open(&output)?;
        let spec = wave.spec();
        ensure!(
            spec.channels == 1
                && spec.sample_rate == 44100
                && spec.bits_per_sample == 16
                && spec.sample_format == hound::SampleFormat::Int
                && wave.duration() <= 44100 * 920,
            "OpenUtau 返回了非预期的单声道 PCM"
        );
        let mut input = Vec::with_capacity(wave.duration() as usize);
        for (index, sample) in wave.samples::<i16>().enumerate() {
            if index % 8192 == 0 {
                control.check()?;
            }
            input.push(f32::from(sample?) / 32768.);
        }
        let resampler = crate::audio::Sinc::new(spec.sample_rate);
        (0..resampler.output_frames(input.len()))
            .map(|frame| {
                if frame % 8192 == 0 {
                    control.check()?;
                }
                Ok(resampler.frame(frame, |index| input.get(index).map(|sample| [*sample, 0.]))[0])
            })
            .collect()
    }

    fn score(
        &self,
        project: &Project,
        track: &Track,
        notes: &[Note],
        origin: f64,
    ) -> Result<Value> {
        let clock = project.timeline();
        let mut position = 0;
        let mut dynamics = Vec::new();
        let mut pitches = Vec::new();
        let mut encoded = Vec::with_capacity(notes.len());
        for note in notes {
            ensure!(
                note.consonant_ms.is_none()
                    && note.vowel_offset_ms.is_none()
                    && note.pitch_blend.is_empty(),
                "{} / {} 使用 VOICEVOX 专用的咬字或音高修正",
                track.name,
                note.id
            );
            let start = ((clock.seconds(note.start) - origin) * TICKS_PER_SECOND).round() as i32;
            let end = ((clock.seconds(note.start + note.duration) - origin) * TICKS_PER_SECOND)
                .round() as i32;
            ensure!(
                end - start >= 10 && start >= position,
                "{} / {} 的时值超出 OpenUtau 可用精度",
                track.name,
                note.id
            );
            let lyric = if note.vowel_continuation {
                ensure!(
                    start == position && !encoded.is_empty(),
                    "OpenUtau 延音必须与上一音符相连"
                );
                "+".to_owned()
            } else {
                ensure!(
                    crate::lyrics::kana(&note.lyric).is_ok_and(|parts| parts.len() == 1),
                    "{} / {} 需要逐音假名读音",
                    track.name,
                    note.id
                );
                format!("{}{}", note.lyric.trim(), self.settings.lyric_suffix)
            };
            let vibrato = crate::vibrato::Envelope::new(note, &clock);
            if start > position {
                if position > 0 {
                    pitches.push((position, 0));
                    dynamics.push((position, 0));
                }
                if start - position > 1 {
                    pitches.push((start - 1, 0));
                    dynamics.push((start - 1, 0));
                }
            }
            let gain = ((note.velocity - 90.) * 2.5).round().clamp(-240., 120.) as i32;
            dynamics.extend([(start, gain), (end - 1, gain)]);
            for tick in (start..end).step_by(5).chain(std::iter::once(end - 1)) {
                let time = origin + tick as f64 / TICKS_PER_SECOND;
                let beat = (clock.beat(time) - note.start).clamp(0., note.duration);
                let cents =
                    (crate::engine::curve_cents(note, beat) + vibrato.cents(time)).round() as i32;
                ensure!(
                    (-1200..=1200).contains(&cents),
                    "OpenUtau 音高曲线超出 ±1200 音分"
                );
                if pitches.last().is_none_or(|(previous, _)| *previous != tick) {
                    pitches.push((tick, cents));
                }
            }
            encoded.push(json!({"position":start,"duration":end-start,"tone":note.pitch,"lyric":lyric,
                "pitch":{"data":[{"x":-40,"y":0,"shape":"l"},{"x":40,"y":0,"shape":"l"}],"snap_first":false},
                "vibrato":{"length":0}}));
            position = end;
        }
        let curve = |abbr, values: Vec<(i32, i32)>| {
            let (xs, ys): (Vec<_>, Vec<_>) = values.into_iter().unzip();
            json!({"abbr":abbr,"xs":xs,"ys":ys})
        };
        let mut singer = json!({"singer":self.settings.singer,"track_name":track.name,
            "renderer_settings":{"renderer":"WORLDLINE-R"}});
        if let Some(class) = self.settings.phonemizer.class() {
            singer["phonemizer"] = json!(class);
        }
        Ok(
            json!({"name":project.title,"ustx_version":"0.10","resolution":480,
            "time_signatures":[{"bar_position":0,"beat_per_bar":4,"beat_unit":4}],
            "tempos":[{"position":0,"bpm":300}],"tracks":[singer],"wave_parts":[],
            "voice_parts":[{"track_no":0,"position":0,"duration":position+480,"notes":encoded,
                "curves":[curve("pitd",pitches),curve("dyn",dynamics)]}]}),
        )
    }
}

struct Work(PathBuf);
impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn status(root: &Path) -> Value {
    let profiles = catalog(root);
    if let Some(default) = profiles.iter().find(|entry| entry["id"] == DEFAULT_PROFILE) {
        json!({"configured":true,"name":default["name"],"terms_url":default["terms_url"],"profiles":profiles})
    } else {
        json!({"configured":false,"profiles":profiles})
    }
}

pub(crate) fn catalog(root: &Path) -> Vec<Value> {
    let mut ids = vec![DEFAULT_PROFILE.to_owned()];
    if let Ok(entries) = fs::read_dir(root.join("runtime/openutau/voices")) {
        ids.extend(entries.flatten().filter_map(|entry| {
            let path = entry.path();
            if !entry.file_type().ok()?.is_file() || path.extension()? != "json" {
                return None;
            }
            Some(path.file_stem()?.to_str()?.to_owned())
        }));
    }
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .filter_map(|profile| {
            Engine::load(root, &profile)
                .ok()
                .map(|engine| engine.describe(&profile))
        })
        .collect()
}
