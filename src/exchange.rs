use crate::model::*;
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read, Write},
    path::PathBuf,
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PpsfAudio {
    vocal_path: Option<PathBuf>,
    backing_path: Option<PathBuf>,
}

pub(crate) fn import(
    store: &Store,
    name: &str,
    bytes: &[u8],
    dry_run: bool,
    expected_source_sha256: Option<&str>,
    reference_audio: Option<PpsfAudio>,
) -> Result<Value> {
    ensure!(bytes.len() <= 128_000_000, "Import exceeds 128 MB");
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let title = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .rsplit_once('.')
        .map(|p| p.0)
        .unwrap_or(name);
    if ext == "ppsf" {
        return import_ppsf(
            store,
            title,
            bytes,
            dry_run,
            expected_source_sha256,
            reference_audio,
        );
    }
    ensure!(
        !dry_run && expected_source_sha256.is_none() && reference_audio.is_none(),
        "Import preview, source fingerprint and reference audio are available for PPSF files"
    );
    let mut archived_mix = None;
    let (bpm, meter, tracks, mut warnings, tempo_map, meter_map, sections) = match ext.as_str() {
        "avpack" => {
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
            let mut raw = vec![];
            archive
                .by_name("project.json")?
                .take(32_000_001)
                .read_to_end(&mut raw)?;
            ensure!(raw.len() <= 32_000_000, "Project JSON exceeds 32 MB");
            let mut p: Project = serde_json::from_slice(&raw)?;
            p.validate()?;
            archived_mix = crate::archived_mix::ArchivedMix::from_bundle(&mut archive, &p, &raw)?;
            let mut assets: HashMap<String, AudioClip> = HashMap::new();
            let mut unpacked_bytes = 0u64;
            let mut resolve = |key: &str| -> Result<AudioClip> {
                if let Some(asset) = assets.get(key) {
                    return Ok(asset.clone());
                }
                let mut entry = archive.by_name(&format!("assets/{key}"))?;
                ensure!(entry.size() <= 400_000_000, "Audio asset too large");
                unpacked_bytes += entry.size();
                ensure!(
                    unpacked_bytes <= 512_000_000,
                    "Expanded audio assets exceed 512 MB"
                );
                let audio = crate::audio::ingest_wav(&store.root, &mut entry, 0.)?;
                if let Err(error) = std::io::copy(&mut entry, &mut std::io::sink()) {
                    let _ = std::fs::remove_file(store.root.join("assets").join(&audio.asset));
                    return Err(error).context("Verify the complete bundled audio entry");
                }
                assets.insert(key.into(), audio.clone());
                Ok(audio)
            };
            for track in &mut p.tracks {
                if let Some(audio) = &mut track.audio {
                    let mut imported = resolve(&audio.asset)?;
                    imported.start = audio.start;
                    *audio = imported;
                }
                for clip in track.clips.iter_mut().flatten() {
                    let imported = resolve(&clip.asset)?;
                    clip.asset = imported.asset;
                }
            }
            (
                p.bpm,
                p.meter,
                p.tracks,
                vec![],
                p.tempo_map,
                p.meter_map,
                vec![],
            )
        }
        "mid" | "midi" => parse_midi(bytes)?,
        "avproj" | "json" => {
            let mut v: Value = serde_json::from_slice(bytes)?;
            if v.get("current").is_some() {
                v = v["current"].clone()
            };
            if PROJECT_FORMATS.contains(&v["format"].as_str().unwrap_or("")) {
                let p: Project = serde_json::from_value(v)?;
                p.validate()?;
                (
                    p.bpm,
                    p.meter,
                    p.tracks,
                    vec![],
                    p.tempo_map,
                    p.meter_map,
                    vec![],
                )
            } else {
                ensure!(v.get("format").is_none(), "Unsupported project format");
                let mut ts = vec![];
                for t in v["tracks"].as_array().context("Need tracks")? {
                    let mut tr = Track::new(t["name"].as_str().unwrap_or("主唱"));
                    tr.notes = t["notes"]
                        .as_array()
                        .context("Need notes")?
                        .iter()
                        .map(Note::from_value)
                        .collect::<Result<_>>()?;
                    ts.push(tr);
                }
                let meter = match v.get("meter") {
                    Some(Value::String(s)) => {
                        let parts = s
                            .split('/')
                            .map(str::parse)
                            .collect::<std::result::Result<Vec<u32>, _>>()?;
                        ensure!(parts.len() == 2, "Invalid meter");
                        [parts[0], parts[1]]
                    }
                    Some(Value::Array(_)) => serde_json::from_value(v["meter"].clone())?,
                    _ => [4, 4],
                };
                (
                    v["bpm"].as_f64().unwrap_or(120.),
                    meter,
                    ts,
                    vec![],
                    serde_json::from_value(v.get("tempo_map").cloned().unwrap_or(json!([])))?,
                    serde_json::from_value(v.get("meter_map").cloned().unwrap_or(json!([])))?,
                    vec![],
                )
            }
        }
        _ => bail!("Supported import: .avpack, .avproj, .json, .mid, .ppsf"),
    };
    let format = if tracks.iter().any(|t| t.clips.is_some()) {
        "agent-vocal/3"
    } else {
        "agent-vocal/2"
    };
    let mut p: Project = serde_json::from_value(
        json!({"format":format,"id":"import_copy","revision":1,"title":title,"bpm":bpm,"meter":meter,"tempo_map":tempo_map,"meter_map":meter_map,"tracks":tracks,"tail_seconds":2.}),
    )?;
    if !sections.is_empty() {
        p.section_markers = Some(sections);
        p.upgrade_feature_format();
    }
    let settings = if ext == "avpack" {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes))?;
        let v: Value = serde_json::from_reader(z.by_name("project.json")?)?;
        Some(v)
    } else if ["json", "avproj"].contains(&ext.as_str()) {
        let v: Value = serde_json::from_slice(bytes)?;
        Some(v.get("current").cloned().unwrap_or(v))
    } else {
        None
    };
    if let Some(settings) = settings {
        let mut values = json!({});
        for key in [
            "composition",
            "section_markers",
            "tail_seconds",
            "master_gain_db",
            "master_limiter",
            "harmony",
            "aux_buses",
            "mix_groups",
        ] {
            if let Some(value) = settings.get(key) {
                values[key] = value.clone()
            }
        }
        if values.as_object().is_some_and(|v| !v.is_empty()) {
            let mut score = serde_json::to_value(&p)?;
            for (key, value) in values.as_object().unwrap() {
                score[key] = value.clone()
            }
            p = serde_json::from_value(score)?;
        }
        if matches!(
            settings["format"].as_str(),
            Some(
                "agent-vocal/4"
                    | "agent-vocal/5"
                    | "agent-vocal/6"
                    | "agent-vocal/7"
                    | "agent-vocal/8"
                    | "agent-vocal/9"
                    | "agent-vocal/10"
                    | "agent-vocal/11"
                    | "agent-vocal/12"
                    | "agent-vocal/13"
                    | "agent-vocal/14"
                    | "agent-vocal/15"
                    | "agent-vocal/16"
                    | "agent-vocal/17"
                    | "agent-vocal/18"
                    | "agent-vocal/19"
                    | "agent-vocal/20"
                    | "agent-vocal/24"
            )
        ) {
            p.format = settings["format"].as_str().unwrap().into();
        }
        p.upgrade_feature_format();
    }
    p = store.create_copy(title, p)?;
    let mut result = json!({"project":&p});
    if let Some(archived_mix) = archived_mix {
        match archived_mix.publish(&store.root, &p) {
            Ok(mix) => result["archived_mix"] = mix,
            Err(error) => warnings.push(format!("工程已导入，归档混音保存失败：{error}")),
        }
    }
    result["warnings"] = json!(warnings);
    Ok(result)
}
pub fn bundle(store: &Store, p: &Project, mix_job_id: Option<&str>) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("project.json", options)?;
    let score = serde_json::to_vec_pretty(p)?;
    writer.write_all(&score)?;
    let mut added = std::collections::HashSet::new();
    for track in &p.tracks {
        for clip in track.media_clips().iter() {
            if added.insert(clip.asset.clone()) {
                writer.start_file(format!("assets/{}", clip.asset), options)?;
                std::io::copy(
                    &mut std::fs::File::open(store.root.join("assets").join(&clip.asset))?,
                    &mut writer,
                )?;
            }
        }
    }
    writer.start_file("README.txt", options)?;
    writer.write_all("Agent Vocal Studio 可移植工程包。包含曲谱、调声、歌词与伴奏音频。导入时新建副本。歌声音源由本机引擎提供，不包含在本包中。".as_bytes())?;
    if let Some(job_id) = mix_job_id {
        crate::archived_mix::write_bundle(&mut writer, &store.root, p, &score, job_id)?;
    }
    let bundle = writer.finish()?.into_inner();
    if mix_job_id.is_some() {
        ensure!(bundle.len() <= 128_000_000, "带归档混音的工程包超过 128 MB");
    }
    Ok(bundle)
}
pub(crate) type Parsed = (
    f64,
    [u32; 2],
    Vec<Track>,
    Vec<String>,
    Vec<crate::timeline::TempoPoint>,
    Vec<crate::timeline::MeterPoint>,
    Vec<SectionMarker>,
);
fn import_ppsf(
    store: &Store,
    title: &str,
    bytes: &[u8],
    dry_run: bool,
    expected_source_sha256: Option<&str>,
    reference_audio: Option<PpsfAudio>,
) -> Result<Value> {
    ensure!(
        !(dry_run && reference_audio.is_some()),
        "Select PPSF reference audio when confirming the import"
    );
    let source_sha256 = format!("{:x}", Sha256::digest(bytes));
    ensure!(
        expected_source_sha256.is_none_or(|expected| expected == source_sha256),
        "IMPORT_SOURCE_CHANGED: PPSF 文件已更新，请重新预览"
    );
    let mut z = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut raw = Vec::new();
    z.by_name("ppsf.json")?
        .take(32_000_001)
        .read_to_end(&mut raw)?;
    ensure!(raw.len() <= 32_000_000, "PPSF JSON exceeds 32 MB");
    let v: Value = serde_json::from_slice(&raw)?;
    let p = &v["ppsf"]["project"];
    for (key, message) in [
        ("tempo", "暂不支持导入含变速的 PPSF"),
        ("meter", "暂不支持导入含拍号变化的 PPSF"),
    ] {
        let timing = &p[key];
        let has_points = ["sequence", "events"].iter().any(|key| {
            timing[key]
                .as_array()
                .is_some_and(|points| !points.is_empty())
        });
        ensure!(
            timing["use_sequence"] != true && (timing["use_sequence"] == false || !has_points),
            "{message}"
        );
    }
    let bpm = p["tempo"]["const"].as_f64().unwrap_or(1200000.) / 10000.;
    let meter = [
        p["meter"]["const"]["nume"].as_u64().unwrap_or(4) as u32,
        p["meter"]["const"]["denomi"].as_u64().unwrap_or(4) as u32,
    ];
    let mut tracks = vec![];
    for t in p["dvl_track"]
        .as_array()
        .context("Missing Piapro singing tracks")?
    {
        let mut tr = Track::new(t["name"].as_str().unwrap_or("主唱"));
        for n in t["events"].as_array().context("Missing notes")? {
            tr.notes.push(Note::from_value(&json!({"start":n["pos"].as_f64().context("note pos")?/480.,"duration":n["length"].as_f64().context("note length")?/480.,"pitch":n["note_number"],"lyric":n["lyric"]}))?);
        }
        tracks.push(tr);
    }
    let mut project: Project = serde_json::from_value(json!({
        "format":"agent-vocal/2","id":"import_copy","revision":1,"title":title,
        "bpm":bpm,"meter":meter,"tracks":tracks,"tail_seconds":2.
    }))?;
    project.validate()?;
    let tracks: Vec<_> = project
        .tracks
        .iter()
        .map(|track| json!({"name":track.name,"note_count":track.notes.len()}))
        .collect();
    let migration = json!({
        "format":"ppsf","title":title,"source_sha256":source_sha256,
        "track_count":tracks.len(),"tracks":tracks,
        "note_count":project.tracks.iter().map(|track| track.notes.len()).sum::<usize>(),
        "bpm":bpm,"meter":meter,
        "preserved":["singing_tracks","note_pitch_and_timing","lyrics","constant_tempo_and_meter"],
        "unsupported":["voicebank","tuning","pronunciation_overrides","mixer","render_cache","backing_audio","legacy_vocaloid_tracks"],
        "omitted_audio_tracks":p["audio_track"].as_array().map_or(0, Vec::len),
        "omitted_vocaloid_tracks":p["vocaloid_track"].as_array().map_or(0, Vec::len),
    });
    let mut result = json!({"dry_run":dry_run,"migration":migration});
    if !dry_run {
        let audio = reference_audio.unwrap_or_default();
        let inputs = [
            ("vocal", "原版人声", audio.vocal_path),
            ("backing", "伴奏", audio.backing_path),
        ];
        let count = inputs.iter().filter(|(_, _, path)| path.is_some()).count();
        ensure!(
            project.tracks.len() + count <= TRACK_LIMIT,
            "工程最多可容纳 32 条轨道"
        );
        if inputs[0].2.is_some() {
            for track in &mut project.tracks {
                track.mute = true;
            }
        }
        let mut assets = Vec::with_capacity(count);
        let mut imported = json!({});
        let saved = (|| {
            for (role, name, path) in inputs {
                let Some(path) = path else { continue };
                ensure!(path.is_absolute(), "请选择{name}音频的绝对路径");
                let filename = path
                    .file_name()
                    .context("Missing audio filename")?
                    .to_string_lossy();
                let source =
                    fs::File::open(&path).with_context(|| format!("无法打开{name}音频"))?;
                ensure!(source.metadata()?.is_file(), "请选择{name}音频文件");
                let clip = crate::audio::ingest(&store.root, Box::new(source), &filename, 0.)
                    .with_context(|| format!("无法导入{name}音频"))?;
                assets.push(clip.asset.clone());
                let mut track = Track::new(name);
                imported[role] = json!({"file":filename,"track_id":track.id,"start":clip.start,"seconds":clip.seconds});
                track.audio = Some(clip);
                project.tracks.push(track);
            }
            store.create_copy(title, project)
        })();
        if saved.is_err() {
            for asset in assets {
                let _ = fs::remove_file(store.root.join("assets").join(asset));
            }
        }
        result["project"] = serde_json::to_value(saved?)?;
        result["warnings"] = json!([if imported.get("vocal").is_some() {
            "已导入原版人声和曲谱。曲谱轨已静音，可解除静音试听新合成。"
        } else {
            "曲谱已导入。Piapro 调声和音色需在本机重新配置。"
        }]);
        if count > 0 {
            result["reference_audio"] = imported;
        }
    }
    Ok(result)
}
fn parse_midi(bytes: &[u8]) -> Result<Parsed> {
    crate::midi_exchange::parse(bytes)
}
pub fn midi(p: &Project) -> Result<Vec<u8>> {
    crate::midi_exchange::export(p)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_formats_preserve_section_markers_and_composition() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_section_exchange"));
        let store = Store::new(root.clone())?;
        let base = store.create("Test", 120., [4, 4], vec![Track::new("主唱")])?;
        let markers = json!([{"name":"副歌","start":4.,"end":8.}]);
        let composition = json!({"purpose":"technical_test","brief":"Section exchange fixture",
            "sections":[{"name":"方案","start":0.,"end":4.}]});
        let project = store.edit(
            &base.id,
            base.revision,
            &json!([
                {"op":"set_project","values":{"section_markers":markers,"composition":composition}}
            ]),
        )?;
        for (name, bytes) in [
            ("section.avproj", serde_json::to_vec(&project)?),
            ("section.avpack", bundle(&store, &project, None)?),
        ] {
            let imported = import(&store, name, &bytes, false, None, None)?;
            assert_eq!(imported["project"]["format"], "agent-vocal/23");
            assert_eq!(imported["project"]["section_markers"], markers);
            assert_eq!(imported["project"]["composition"], composition);
            assert_ne!(imported["project"]["id"], project.id);
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn midi_roundtrip_retains_meter_lyrics_and_timing() -> Result<()> {
        let mut t = Track::new("主唱");
        t.notes.push(Note::from_value(
            &json!({"start":3,"duration":1.5,"pitch":69,"lyric":"そ"}),
        )?);
        let p = Project {
            format: "agent-vocal/1".into(),
            id: uid("p"),
            revision: 1,
            composition: None,
            section_markers: None,
            tail_seconds: 2.,
            master_gain_db: 0.,
            master_limiter: None,
            aux_buses: vec![],
            mix_groups: vec![],
            title: "Test".into(),
            bpm: 114.,
            meter: [6, 8],
            tempo_map: vec![],
            meter_map: vec![],
            harmony: vec![],
            tracks: vec![t],
        };
        let (b, m, ts, _, _, _, _) = parse_midi(&midi(&p)?)?;
        assert!((b - 114.).abs() < 0.001);
        assert_eq!(m, [6, 8]);
        let n = &ts[0].notes[0];
        assert_eq!(n.lyric, "そ");
        assert_eq!(n.start, 3.);
        assert_eq!(n.duration, 1.5);
        Ok(())
    }
}
