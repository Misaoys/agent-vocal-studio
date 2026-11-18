use crate::model::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, f64::consts::PI, fs, io::Cursor, path::Path};
pub const ENGINE: &str = "http://127.0.0.1:50021";
pub(crate) const RENDER_ENGINES: [&str; 4] = ["guide", "voicevox", "openutau", "project"];

fn voice_route<'a>(mode: &'a str, track: &'a Track) -> (&'a str, &'a str) {
    if mode == "project" {
        if let Some(profile) = track.openutau_voice.as_deref() {
            return ("openutau", profile);
        }
        return ("voicevox", crate::openutau::DEFAULT_PROFILE);
    }
    (mode, crate::openutau::DEFAULT_PROFILE)
}
pub fn get(route: &str) -> Result<Value> {
    Ok(ureq::get(&format!("{ENGINE}{route}"))
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .build()
        .call()?
        .body_mut()
        .read_json()?)
}
fn post(route: &str, v: &Value) -> Result<Value> {
    Ok(ureq::post(&format!("{ENGINE}{route}"))
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(300)))
        .build()
        .send_json(v)?
        .body_mut()
        .with_config()
        .limit(100_000_000)
        .read_json()?)
}
pub fn singers() -> Result<Value> {
    get("/singers")
}

pub(crate) fn japanese_reading(text: &str) -> Result<Vec<String>> {
    let speakers = get("/speakers").context("无法连接本地 VOICEVOX，可直接填写假名")?;
    let speaker = speakers
        .as_array()
        .context("无法读取 VOICEVOX 语音列表")?
        .iter()
        .filter_map(|speaker| speaker["styles"].as_array())
        .flatten()
        .filter(|style| {
            matches!(
                style["type"].as_str(),
                Some("talk" | "streaming_talk") | None
            )
        })
        .find_map(|style| style["id"].as_u64())
        .context("本地 VOICEVOX 没有可用的读音分析音源")?;
    let phrases: Vec<Value> = ureq::post(&format!("{ENGINE}/accent_phrases"))
        .query("speaker", speaker.to_string())
        .query("text", text)
        .query("enable_katakana_english", "false")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(8)))
        .build()
        .send_empty()
        .context("VOICEVOX 未能提取日语读音，可直接填写假名")?
        .body_mut()
        .with_config()
        .limit(512_000)
        .read_json()?;
    let mut words = Vec::new();
    for phrase in &phrases {
        let moras = phrase["moras"]
            .as_array()
            .context("VOICEVOX 读音返回格式有误")?;
        for mora in moras {
            words.push(field(mora, "text")?);
        }
    }
    crate::lyrics::kana(&words.join(" "))
}

pub fn status() -> Value {
    match get("/version") {
        Ok(v) => json!({"connected":true,"version":v,"url":ENGINE}),
        Err(e) => json!({"connected":false,"url":ENGINE,"error":e.to_string()}),
    }
}
pub(crate) fn curve_cents(n: &Note, beat: f64) -> f64 {
    curve_value(&n.pitch_points, beat)
}
pub(crate) fn curve_value(pts: &[[f64; 2]], beat: f64) -> f64 {
    let i = pts.partition_point(|p| p[0] <= beat).saturating_sub(1);
    let Some(point) = pts.get(i) else { return 0. };
    if let Some(next) = pts.get(i + 1) {
        let f = ((beat - point[0]) / (next[0] - point[0])).clamp(0., 1.);
        point[1] + f * (next[1] - point[1])
    } else {
        point[1]
    }
}
struct VocalScore<'a> {
    body: Value,
    slots: Vec<VocalSlot<'a>>,
    pad: usize,
    frames: usize,
    origin: usize,
}

struct VocalSlot<'a> {
    note: &'a Note,
    head: &'a Note,
    start: usize,
    end: usize,
    latest_vowel: Option<usize>,
}

fn vocal_score<'a>(
    p: &Project,
    notes: &'a [Note],
    rate: f64,
    origin: usize,
) -> Result<VocalScore<'a>> {
    let clock = p.timeline();
    let pad = (0.2 * rate).round() as usize;
    let mut score = vec![json!({"key":null,"frame_length":pad,"lyric":""})];
    let mut cursor = pad;
    let mut slots: Vec<VocalSlot<'_>> = vec![];
    let mut group = None;
    for n in notes {
        let rounded_start = pad
            + ((clock.seconds(n.start) * rate).round() as usize)
                .checked_sub(origin)
                .context("Vocal note lies before synthesis context")?;
        let a = if n.vowel_continuation && rounded_start.abs_diff(cursor) <= 1 {
            cursor
        } else {
            rounded_start
        };
        let b = pad
            + ((clock.seconds(n.start + n.duration) * rate).round() as usize)
                .checked_sub(origin)
                .context("Vocal note lies before synthesis context")?;
        ensure!(b > a, "Note {} is shorter than one engine frame", n.id);
        let head = if n.vowel_continuation {
            let (head, group_start): (&Note, usize) =
                group.context("A vowel continuation needs a syllable head")?;
            ensure!(
                a == cursor,
                "Vowel continuation does not touch the previous vocal frame"
            );
            let syllable = score.last_mut().unwrap();
            if let Some(last) = slots
                .last_mut()
                .filter(|slot| !slot.note.vowel_continuation)
            {
                last.latest_vowel = Some(last.end - 1);
                syllable["lyric"] = json!(crate::lyrics::sustained_kana(&head.lyric)?);
            }
            syllable["frame_length"] = json!(b - group_start);
            head
        } else {
            if !slots.is_empty() && a == cursor + 1 {
                // VOICEVOX 0.26.2 may drop a consonant after a one-frame pause.
                // Extend only the synthesized syllable; nominal onsets stay fixed.
                let syllable = score.last_mut().unwrap();
                let frames = syllable["frame_length"].as_u64().unwrap();
                syllable["frame_length"] = json!(frames + 1);
            } else if a > cursor {
                score.push(json!({"key":null,"frame_length":a-cursor,"lyric":""}));
            }
            score.push(json!({"id":n.id,"key":n.pitch,"frame_length":b-a,"lyric":n.lyric}));
            group = Some((n, a));
            n
        };
        cursor = b;
        slots.push(VocalSlot {
            note: n,
            head,
            start: a,
            end: b,
            latest_vowel: None,
        });
    }
    let tail = (rate * 0.35).ceil() as usize;
    score.push(json!({"key":null,"frame_length":tail,"lyric":""}));
    Ok(VocalScore {
        body: json!({"notes":score}),
        slots,
        pad,
        frames: cursor + tail,
        origin,
    })
}

fn singing_query(teacher: u32, score: &VocalScore<'_>) -> Result<Value> {
    let q = post(
        &format!("/sing_frame_audio_query?speaker={teacher}"),
        &score.body,
    )?;
    ensure!(
        ["f0", "volume"].iter().all(|key| q[key]
            .as_array()
            .is_some_and(|frames| frames.len() == score.frames)),
        "VOICEVOX query length does not match the score"
    );
    Ok(q)
}

#[derive(Deserialize, Serialize)]
struct FramePhoneme {
    phoneme: String,
    frame_length: usize,
    note_id: Option<String>,
}

#[derive(Serialize)]
struct PronunciationTiming {
    note_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    vowel_continuation_of: Option<String>,
    lyric: String,
    requested_ms: Option<f64>,
    consonant: Option<String>,
    next_phoneme: Option<String>,
    default_lead_ms: Option<f64>,
    effective_lead_ms: Option<f64>,
    audible_lead_ms: Option<f64>,
    max_lead_ms: Option<f64>,
    vowel_onset_seconds: Option<f64>,
    limited_by_previous_phoneme: bool,
    requested_vowel_offset_ms: Option<f64>,
    effective_vowel_offset_ms: Option<f64>,
    vowel_offset_range_ms: Option<[f64; 2]>,
    score_onset_seconds: f64,
    vowel_offset_limited: bool,
}

struct NotePhonemes {
    first: usize,
    tail: usize,
    default_lead: usize,
    has_vowel: bool,
}

impl NotePhonemes {
    /// Offsets are relative to the scored vowel frame. Every adjacent phoneme retains a frame.
    fn offset_range(
        &self,
        edges: &[usize],
        slot: &VocalSlot,
        pad: usize,
        limit: isize,
    ) -> [isize; 2] {
        let lead = edges[self.tail] - edges[self.first];
        let earliest = (edges[self.first - 1] + 1 + lead).max(pad);
        let latest = (edges[self.tail + 1] - 1).min(slot.latest_vowel.unwrap_or(usize::MAX));
        [
            (-limit).max(earliest as isize - slot.start as isize),
            limit.min(latest as isize - slot.start as isize),
        ]
    }
}

struct Pronunciation {
    phonemes: Vec<FramePhoneme>,
    timings: Vec<PronunciationTiming>,
    expression_windows: Vec<(usize, usize)>,
    changed: bool,
}

fn pronunciation_frames(q: &Value, score: &VocalScore<'_>, rate: f64) -> Result<Pronunciation> {
    let mut phonemes: Vec<FramePhoneme> = serde_json::from_value(q["phonemes"].clone())?;
    let mut edges = Vec::with_capacity(phonemes.len() + 1);
    edges.push(0);
    let mut total = 0usize;
    let mut first_by_id = HashMap::new();
    for (i, segment) in phonemes.iter().enumerate() {
        ensure!(segment.frame_length > 0, "Empty VOICEVOX phoneme segment");
        total = total
            .checked_add(segment.frame_length)
            .context("VOICEVOX phoneme length overflow")?;
        edges.push(total);
        if let Some(id) = segment.note_id.as_deref() {
            first_by_id.entry(id).or_insert(i);
        }
    }
    ensure!(
        total == score.frames,
        "VOICEVOX phoneme lengths do not match the score"
    );
    let to_frames = |value: f64| (value * rate / 1000.).round() as isize;
    let to_ms = |frames: isize| frames as f64 * 1000. / rate;
    let offset_limit = to_frames(MAX_VOWEL_OFFSET_MS);
    let mut syllables = Vec::with_capacity(score.slots.len());
    for slot in &score.slots {
        let note = slot.note;
        let syllable = first_by_id.get(note.id.as_str()).and_then(|&first| {
            let tail = if phonemes
                .get(first + 1)
                .is_some_and(|next| next.note_id.as_deref() == Some(&note.id))
            {
                first + 1
            } else {
                first
            };
            (first > 0 && edges[tail] == slot.start).then(|| NotePhonemes {
                first,
                tail,
                default_lead: edges[tail] - edges[first],
                has_vowel: matches!(
                    phonemes[tail].phoneme.as_str(),
                    "a" | "e" | "i" | "o" | "u" | "A" | "E" | "I" | "O" | "U"
                ),
            })
        });
        if let Some(syllable) = &syllable {
            if syllable.first != syllable.tail {
                let requested = note
                    .consonant_ms
                    .map_or(syllable.default_lead, |ms| to_frames(ms).max(1) as usize);
                let available = edges[syllable.tail] - edges[syllable.first - 1] - 1;
                edges[syllable.first] = edges[syllable.tail] - requested.min(available);
            }
        }
        syllables.push(syllable);
    }
    let mut expression_windows: Vec<_> = score
        .slots
        .iter()
        .map(|slot| (slot.start, slot.end))
        .collect();
    // Move both edges of the syllable lead. The existing consonant duration and phoneme sequence stay intact.
    for (index, (slot, syllable)) in score.slots.iter().zip(&syllables).enumerate() {
        if let Some(syllable) = syllable.as_ref().filter(|syllable| syllable.has_vowel) {
            let [earliest, latest] = syllable.offset_range(&edges, slot, score.pad, offset_limit);
            let shift = to_frames(slot.note.vowel_offset_ms.unwrap_or(0.)).clamp(earliest, latest);
            let performed = slot.start.saturating_add_signed(shift);
            edges[syllable.first] = edges[syllable.first].saturating_add_signed(shift);
            edges[syllable.tail] = performed;
            expression_windows[index].0 = performed;
            if index > 0 && score.slots[index - 1].end == slot.start {
                expression_windows[index - 1].1 = performed;
            }
        }
    }
    let mut timings = Vec::with_capacity(score.slots.len());
    for (slot, syllable) in score.slots.iter().zip(syllables) {
        let note = slot.note;
        let continues = note.vowel_continuation;
        let mut timing = PronunciationTiming {
            note_id: note.id.clone(),
            vowel_continuation_of: continues.then(|| slot.head.id.clone()),
            lyric: slot.head.lyric.clone(),
            requested_ms: (!continues).then_some(note.consonant_ms).flatten(),
            consonant: None,
            next_phoneme: None,
            default_lead_ms: None,
            effective_lead_ms: None,
            audible_lead_ms: None,
            max_lead_ms: None,
            vowel_onset_seconds: None,
            limited_by_previous_phoneme: false,
            requested_vowel_offset_ms: (!continues).then_some(note.vowel_offset_ms).flatten(),
            effective_vowel_offset_ms: None,
            vowel_offset_range_ms: None,
            score_onset_seconds: (slot.start - score.pad + score.origin) as f64 / rate,
            vowel_offset_limited: false,
        };
        if let Some(syllable) = syllable {
            let performed = edges[syllable.tail];
            let lead = performed - edges[syllable.first];
            if syllable.first != syllable.tail {
                let available = performed - edges[syllable.first - 1] - 1;
                timing.consonant = Some(phonemes[syllable.first].phoneme.clone());
                timing.next_phoneme = Some(phonemes[syllable.tail].phoneme.clone());
                timing.default_lead_ms = Some(to_ms(syllable.default_lead as isize));
                timing.effective_lead_ms = Some(to_ms(lead as isize));
                timing.audible_lead_ms = Some(to_ms(lead.min(performed - score.pad) as isize));
                timing.max_lead_ms = Some(to_ms((available as isize).min(to_frames(300.))));
                timing.limited_by_previous_phoneme = note
                    .consonant_ms
                    .is_some_and(|ms| to_frames(ms).max(1) as usize > lead);
            }
            if syllable.has_vowel {
                let delta = performed as isize - slot.start as isize;
                timing.vowel_onset_seconds =
                    Some((performed - score.pad + score.origin) as f64 / rate);
                timing.effective_vowel_offset_ms = Some(to_ms(delta));
                timing.vowel_offset_range_ms = Some(
                    syllable
                        .offset_range(&edges, slot, score.pad, offset_limit)
                        .map(to_ms),
                );
                timing.vowel_offset_limited =
                    delta != to_frames(note.vowel_offset_ms.unwrap_or(0.));
            }
        }
        timings.push(timing);
    }
    let mut changed = false;
    for (segment, boundaries) in phonemes.iter_mut().zip(edges.windows(2)) {
        let length = boundaries[1] - boundaries[0];
        changed |= segment.frame_length != length;
        segment.frame_length = length;
    }
    Ok(Pronunciation {
        phonemes,
        timings,
        expression_windows,
        changed,
    })
}

pub(crate) fn pronunciation_note_ids(args: &Value) -> Result<Vec<&str>> {
    args["note_ids"]
        .as_array()
        .context("note_ids must be an array")?
        .iter()
        .map(|id| id.as_str().context("Invalid note ID"))
        .collect()
}

pub(crate) fn pronunciation(p: &Project, track_id: &str, ids: &[&str]) -> Result<Value> {
    ensure!(
        (1..=240).contains(&ids.len()),
        "Select from 1 to 240 vocal notes"
    );
    let t = p
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .context("Unknown track")?;
    ensure!(
        t.instrument.is_none() && !t.is_audio(),
        "Select a vocal track"
    );
    let picked = ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    ensure!(
        picked.len() == ids.len()
            && t.notes
                .iter()
                .filter(|note| picked.contains(note.id.as_str()))
                .count()
                == ids.len(),
        "Unknown or repeated note ID"
    );
    let banks = singers().context("本地 VOICEVOX 未连接，无法分析子音")?;
    let teacher = singing_styles(&banks)?
        .find(|style| matches!(style["type"].as_str(), Some("singing_teacher" | "sing")))
        .and_then(|style| style["id"].as_u64())
        .context("No singing teacher voice")? as u32;
    let rate = frame_rate()?;
    let score = vocal_score(p, &t.notes, rate, 0)?;
    let q = singing_query(teacher, &score)?;
    let mut timings = pronunciation_frames(&q, &score, rate)?.timings;
    timings.retain(|timing| picked.contains(timing.note_id.as_str()));
    Ok(json!({
        "project_id":p.id,"revision":p.revision,"track_id":t.id,
        "frame_ms":1000./rate,"notes":timings
    }))
}

fn frame_rate() -> Result<f64> {
    get("/engine_manifest")?["frame_rate"]
        .as_f64()
        .filter(|rate| rate.is_finite() && *rate >= 10. && *rate <= 1000.)
        .context("Engine has no usable singing frame_rate")
}

fn singing_styles(banks: &Value) -> Result<impl Iterator<Item = &Value>> {
    Ok(banks
        .as_array()
        .context("Invalid singers")?
        .iter()
        .flat_map(|bank| bank["styles"].as_array().into_iter().flatten()))
}

pub(crate) struct VoiceEngine {
    banks: Value,
    rate: f64,
    teacher: u32,
    default_singer: u32,
}

impl VoiceEngine {
    pub(crate) fn load(control: &crate::jobs::Control) -> Result<Self> {
        control.progress("preparing_voice", 0.01, None)?;
        let banks = singers()?;
        control.check()?;
        let rate = frame_rate()?;
        let mut query = None;
        let mut decode = None;
        for style in singing_styles(&banks)? {
            match style["type"].as_str() {
                Some("singing_teacher" | "sing") if query.is_none() => query = style["id"].as_u64(),
                Some("frame_decode") if decode.is_none() => decode = style["id"].as_u64(),
                _ => (),
            }
        }
        Ok(Self {
            banks,
            rate,
            teacher: query.context("No singing teacher voice")? as u32,
            default_singer: decode.context("No frame_decode singer")? as u32,
        })
    }

    fn singer_bank(&self, singer: u32) -> Result<&Value> {
        self.banks
            .as_array()
            .into_iter()
            .flatten()
            .find(|bank| {
                bank["styles"].as_array().is_some_and(|styles| {
                    styles.iter().any(|style| {
                        style["id"].as_u64() == Some(u64::from(singer))
                            && style["type"] == "frame_decode"
                    })
                })
            })
            .context("Selected singer is unavailable or cannot decode singing")
    }

    pub(crate) fn fingerprint(&self) -> Result<String> {
        let Self {
            rate,
            teacher,
            default_singer,
            ..
        } = self;
        Ok(format!(
            "score2:{}:{rate}:{teacher}:{default_singer}",
            get("/version")?
        ))
    }

    pub(crate) fn monitor_excerpt(
        &self,
        p: &Project,
        track: &Track,
        range: [usize; 2],
        control: &crate::jobs::Control,
    ) -> Result<Option<Vec<[f32; 2]>>> {
        let notes = vocal_context(p, track, range);
        if let (Some(first), Some(last)) = (notes.first(), notes.last()) {
            let clock = p.timeline();
            let origin = (range[0] as f64 / 48000. - 0.85)
                .min(clock.seconds(first.start) - 0.5)
                .max(0.);
            let ends = clock.seconds(last.start + last.duration) + 0.4;
            if ends - origin > 12. {
                return Ok(None);
            }
        }
        self.render_excerpt(p, track, notes, range, control)
            .map(Some)
    }

    fn render_excerpt(
        &self,
        p: &Project,
        track: &Track,
        notes: &[Note],
        [first, end]: [usize; 2],
        control: &crate::jobs::Control,
    ) -> Result<Vec<[f32; 2]>> {
        if notes.is_empty() {
            return Ok(vec![[0.; 2]; end - first]);
        }
        self.singer_bank(track.singer_id.unwrap_or(self.default_singer))?;
        let context_start = (first as f64 / 48000. - 0.85).max(0.);
        let start = context_start.min((p.timeline().seconds(notes[0].start) - 0.5).max(0.));
        let origin = (start * self.rate).floor() as usize;
        let origin_sample = (origin as f64 / self.rate * 48000.).round() as usize;
        ensure!(origin_sample <= first, "Vocal synthesis context shifted");
        let mono = voice_track(p, track, notes, origin, self, control, (0.05, 0.65))?;
        Ok((first - origin_sample..end - origin_sample)
            .map(|sample| [mono.get(sample).copied().unwrap_or_default(); 2])
            .collect())
    }
}

fn voice_track(
    p: &Project,
    t: &Track,
    notes: &[Note],
    origin: usize,
    engine: &VoiceEngine,
    control: &crate::jobs::Control,
    progress: (f64, f64),
) -> Result<Vec<f32>> {
    let (base, span) = progress;
    let clock = p.timeline();
    let VoiceEngine { rate, teacher, .. } = *engine;
    let singer = t.singer_id.unwrap_or(engine.default_singer);
    let score = vocal_score(p, notes, rate, origin)?;
    let pad = score.pad;
    control.progress("voice_query", base, Some((&t.id, &t.name)))?;
    let mut q = singing_query(teacher, &score)?;
    control.check()?;
    let mut expression_windows = score
        .slots
        .iter()
        .map(|slot| (slot.start, slot.end))
        .collect::<Vec<_>>();
    if notes.iter().any(|note| {
        !note.vowel_continuation && (note.consonant_ms.is_some() || note.vowel_offset_ms.is_some())
    }) {
        let timing = pronunciation_frames(&q, &score, rate)?;
        if let Some(unsupported) = timing
            .timings
            .iter()
            .find(|timing| timing.requested_ms.is_some() && timing.consonant.is_none())
        {
            anyhow::bail!(
                "音符 {}（{}）没有可编辑的前置子音，请恢复自动子音",
                unsupported.note_id,
                unsupported.lyric
            );
        }
        if let Some(unsupported) = timing.timings.iter().find(|entry| {
            entry.requested_vowel_offset_ms.is_some() && entry.vowel_onset_seconds.is_none()
        }) {
            anyhow::bail!(
                "音符 {}（{}）没有可移动的元音，请清除落拍偏移",
                unsupported.note_id,
                unsupported.lyric
            );
        }
        expression_windows = timing.expression_windows;
        if timing.changed {
            q["phonemes"] = serde_json::to_value(timing.phonemes)?;
            for key in ["f0", "volume"] {
                let frames = post(
                    &format!("/sing_frame_{key}?speaker={teacher}"),
                    &json!({"score":score.body,"frame_audio_query":q}),
                )?;
                control.check()?;
                ensure!(
                    frames
                        .as_array()
                        .is_some_and(|frames| frames.len() == score.frames),
                    "Retimed VOICEVOX {key} length does not match the score"
                );
                q[key] = frames;
            }
        }
    }
    for (slot, (a, b)) in score.slots.into_iter().zip(expression_windows) {
        let n = slot.note;
        let vibrato = crate::vibrato::Envelope::new(n, &clock);
        let midi_f0 = 440. * 2f64.powf((f64::from(n.pitch) - 69.) / 12.);
        let pitch_ratio = 2f64.powf((f64::from(n.pitch) - f64::from(slot.head.pitch)) / 12.);
        for i in a..b {
            let time = (i - pad + origin) as f64 / rate;
            let beat = (clock.beat(time) - n.start).clamp(0., n.duration);
            let f = q["f0"][i].as_f64().context("Invalid f0")? * pitch_ratio;
            // Apply score expression after predicting the retimed singing frames.
            if f > 0. {
                let weight = curve_value(&n.pitch_blend, beat);
                let base = if weight == 0. {
                    f
                } else {
                    f * (midi_f0 / f).powf(weight)
                };
                let offset = curve_cents(n, beat) + vibrato.cents(time);
                q["f0"][i] = json!(base * 2f64.powf(offset / 1200.));
            }
            let v = q["volume"][i].as_f64().context("Invalid volume")?;
            q["volume"][i] = json!(v * 10f64.powf((n.velocity - 90.) / 80.));
        }
    }
    q["outputSamplingRate"] = json!(48000);
    q["outputStereo"] = json!(false);
    control.progress("voice_synthesis", base + span * 0.2, Some((&t.id, &t.name)))?;
    let bytes = ureq::post(&format!("{ENGINE}/frame_synthesis?speaker={singer}"))
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(300)))
        .build()
        .send_json(&q)?
        .body_mut()
        .with_config()
        .limit(200_000_000)
        .read_to_vec()?;
    control.check()?;
    let mut wav = hound::WavReader::new(Cursor::new(bytes))?;
    let spec = wav.spec();
    ensure!(
        spec.sample_rate == 48000 && spec.channels == 1,
        "Unexpected synthesis WAV format"
    );
    let samples = if spec.sample_format == hound::SampleFormat::Float {
        wav.samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
        wav.samples::<i32>()
            .map(|s| s.map(|x| x as f32 / scale))
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let trim = (pad as f64 / rate * 48000.).round() as usize;
    ensure!(samples.len() > trim, "Engine produced no audio");
    Ok(samples[trim..].to_vec())
}
fn guide_range(
    p: &Project,
    notes: &[Note],
    [first, end]: [usize; 2],
    control: &crate::jobs::Control,
) -> Result<Vec<f32>> {
    let clock = p.timeline();
    let mut out = vec![0.; end - first];
    let mut phase = 0.;
    for (index, n) in notes.iter().enumerate() {
        let start = clock.seconds(n.start);
        let vibrato = crate::vibrato::Envelope::new(n, &clock);
        let a = (start * 48000.).round() as usize;
        let len = (clock.seconds(n.start + n.duration) * 48000.).round() as usize - a;
        if !n.vowel_continuation {
            phase = 0.;
        }
        let carries_on = notes
            .get(index + 1)
            .is_some_and(|next| next.vowel_continuation);
        for j in 0..len {
            if j % 8192 == 0 {
                control.check()?;
            }
            if a + j >= end {
                break;
            };
            let time = j as f64 / 48000.;
            let attack = if n.vowel_continuation {
                1.
            } else {
                (time / 0.012).min(1.)
            };
            let release = if carries_on {
                1.
            } else {
                ((len - j) as f64 / 48000. / 0.025).min(1.)
            };
            let env = attack * release;
            let beat = (clock.beat(start + time) - n.start).clamp(0., n.duration);
            let offset = curve_cents(n, beat) + vibrato.cents(start + time);
            let freq = 440. * 2f64.powf((n.pitch as f64 - 69. + offset / 100.) / 12.);
            phase += 2. * PI * freq / 48000.;
            if a + j >= first {
                out[a + j - first] +=
                    (env * (phase.sin() + 0.18 * (phase * 2.).sin()) * 0.2 * n.velocity / 127.)
                        as f32;
            }
        }
    }
    Ok(out)
}

fn guide_track(p: &Project, t: &Track, control: &crate::jobs::Control) -> Result<Vec<f32>> {
    guide_range(
        p,
        &t.notes,
        [0, ((p.seconds() + 0.35) * 48000.).ceil() as usize],
        control,
    )
}

fn vocal_context<'a>(p: &Project, track: &'a Track, [first, end]: [usize; 2]) -> &'a [Note] {
    let clock = p.timeline();
    let context_start = (first as f64 / 48000. - 0.85).max(0.);
    let context_end = end as f64 / 48000. + 0.85;
    let mut left = track
        .notes
        .partition_point(|note| clock.seconds(note.start + note.duration) <= context_start);
    let mut right = track
        .notes
        .partition_point(|note| clock.seconds(note.start) < context_end);
    if left >= right {
        return &[];
    }
    // Retain the complete sustained syllable at either edge, including its head.
    while left > 0 && track.notes[left].vowel_continuation {
        left -= 1;
    }
    while track
        .notes
        .get(right)
        .is_some_and(|note| note.vowel_continuation)
    {
        right += 1;
    }
    &track.notes[left..right]
}

pub(crate) fn audition_voice_excerpt(
    root: &Path,
    p: &Project,
    mode: &str,
    track_id: &str,
    [first, end]: [usize; 2],
    control: &crate::jobs::Control,
) -> Result<Vec<[f32; 2]>> {
    let track = p
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .context("请选择歌声轨")?;
    ensure!(
        track.instrument.is_none() && !track.is_audio(),
        "请选择歌声轨"
    );
    let (mode, profile) = voice_route(mode, track);
    let notes = vocal_context(p, track, [first, end]);
    if notes.is_empty() {
        return Ok(vec![[0.; 2]; end - first]);
    }
    if mode == "voicevox" {
        return VoiceEngine::load(control)?.render_excerpt(p, track, notes, [first, end], control);
    }
    if mode == "guide" {
        return Ok(guide_range(p, notes, [first, end], control)?
            .into_iter()
            .map(|x| [x; 2])
            .collect());
    }
    if mode == "openutau" {
        let engine = crate::openutau::Engine::load(root, profile)?;
        let origin = ((p.timeline().seconds(notes[0].start) - 0.85).max(0.) * 48000.) as usize;
        let origin = origin.min(first);
        let mono = engine.render(root, p, track, notes, origin as f64 / 48000., control)?;
        return Ok((first - origin..end - origin)
            .map(|sample| [mono.get(sample).copied().unwrap_or_default(); 2])
            .collect());
    }
    anyhow::bail!("Unsupported engine")
}
/// A unique, private staging directory is never exposed as a successful export.
/// Dropping it cleans only the directory this worker created exclusively.
struct RenderWork(std::path::PathBuf);
impl Drop for RenderWork {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn render(
    root: &Path,
    p: &Project,
    job: &str,
    mode: &str,
    monitor: bool,
    control: &crate::jobs::Control,
) -> Result<Value> {
    match render_inner(
        root,
        p,
        mode,
        RenderTarget::Export { job, monitor },
        control,
    )? {
        Rendered::Export(report) => Ok(report),
        _ => unreachable!(),
    }
}

pub(crate) fn prepare_monitor_sources(
    root: &Path,
    project: &Project,
    engine: &str,
    tracks: &std::collections::HashSet<&str>,
    control: &crate::jobs::Control,
) -> Result<Vec<crate::source_cache::Source>> {
    match render_inner(
        root,
        project,
        engine,
        RenderTarget::Monitor(tracks),
        control,
    )? {
        Rendered::Monitor(sources) => Ok(sources),
        _ => unreachable!(),
    }
}

enum RenderTarget<'a> {
    Export {
        job: &'a str,
        monitor: bool,
    },
    Audition {
        first: usize,
        end: usize,
        output: Option<&'a crate::audition::SavedOutput>,
    },
    Monitor(&'a std::collections::HashSet<&'a str>),
}
enum Rendered {
    Export(Value),
    Audition(Vec<[f32; 2]>),
    Monitor(Vec<crate::source_cache::Source>),
}

struct OutputExcerpt<'a> {
    output: &'a crate::audition::SavedOutput,
    first: usize,
    frames: Vec<[f32; 2]>,
}

impl OutputExcerpt<'_> {
    fn write(&mut self, index: usize, frame: [f32; 2]) {
        if let Some(index) = index.checked_sub(self.first) {
            if let Some(output) = self.frames.get_mut(index) {
                *output = frame;
            }
        }
    }
}

enum BusOutput<'a, 'b> {
    Export,
    Audition(Option<&'a mut OutputExcerpt<'b>>),
}

fn float_stem(path: &Path) -> Result<hound::WavWriter<std::io::BufWriter<std::fs::File>>> {
    Ok(hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?)
}

type Stem = (String, std::path::PathBuf);

struct BusStem<'a> {
    index: usize,
    bus_id: &'a str,
    gain: f64,
    chain: crate::effects::Chain,
    input: Vec<[f32; 2]>,
    file: Stem,
    writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
    peak: f32,
}

#[derive(Default)]
struct BusStems {
    files: Vec<Stem>,
    groups: Vec<Value>,
    returns: Vec<Value>,
    peak: f32,
}

type KeySamples = hound::WavIntoSamples<std::io::BufReader<std::fs::File>, f32>;

struct SidechainFiles<'a> {
    dir: &'a Path,
    files: std::collections::HashMap<&'a str, (usize, std::path::PathBuf)>,
}

impl<'a> SidechainFiles<'a> {
    fn write(
        &mut self,
        id: &'a str,
        index: usize,
        data: &[[f32; 2]],
        control: &crate::jobs::Control,
    ) -> Result<()> {
        let path = self.dir.join(format!(".key_{index}.float.wav"));
        let mut writer = float_stem(&path)?;
        for chunk in data.chunks(8192) {
            control.check()?;
            for &sample in chunk.iter().flatten() {
                ensure!(
                    sample.is_finite(),
                    "Sidechain source produced non-finite audio"
                );
                writer.write_sample(sample)?;
            }
        }
        writer.finalize()?;
        self.files.insert(id, (index, path));
        Ok(())
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.files.get(id).map(|(index, _)| *index)
    }

    fn read(
        &self,
        id: &str,
        len: usize,
        control: &crate::jobs::Control,
    ) -> Result<Option<Vec<[f32; 2]>>> {
        let Some((_, path)) = self.files.get(id) else {
            return Ok(None);
        };
        let mut samples = hound::WavReader::open(path)?.into_samples::<f32>();
        let mut data = Vec::with_capacity(len);
        for index in 0..len {
            if index % 8192 == 0 {
                control.check()?;
            }
            data.push(read_sidechain_frame(&mut samples)?);
        }
        Ok(Some(data))
    }

    fn inputs<'e>(
        &self,
        effects: impl Iterator<Item = &'e crate::effects::Effect>,
    ) -> Result<SidechainInputs> {
        let ids: std::collections::HashSet<_> = effects
            .filter_map(crate::effects::Effect::active_sidechain)
            .collect();
        let readers = ids
            .into_iter()
            .filter_map(|id| self.files.get(id))
            .map(|(index, file)| Ok((*index, hound::WavReader::open(file)?.into_samples::<f32>())))
            .collect::<Result<_>>()?;
        Ok(SidechainInputs {
            readers,
            peaks: [0.; crate::model::TRACK_LIMIT],
        })
    }

    fn remove(self) -> Result<()> {
        for (_, file) in self.files.into_values() {
            fs::remove_file(file)?;
        }
        Ok(())
    }
}

fn read_sidechain_frame(samples: &mut KeySamples) -> Result<[f32; 2]> {
    let mut sample =
        || -> Result<f32> { Ok(samples.next().context("Sidechain source ended early")??) };
    Ok([sample()?, sample()?])
}

struct SidechainInputs {
    readers: Vec<(usize, KeySamples)>,
    peaks: [f32; crate::model::TRACK_LIMIT],
}

impl SidechainInputs {
    fn next(&mut self) -> Result<&[f32]> {
        for (index, samples) in &mut self.readers {
            self.peaks[*index] = crate::effects::peak(read_sidechain_frame(samples)?);
        }
        Ok(&self.peaks)
    }
}

fn bus_stems(
    project: &Project,
    sidechains: &SidechainFiles<'_>,
    mix: &mut [[f32; 2]],
    mut envelope: Option<&mut [f32]>,
    feeds: Vec<(std::path::PathBuf, crate::routing::Levels)>,
    output: BusOutput<'_, '_>,
    control: &crate::jobs::Control,
) -> Result<BusStems> {
    use crate::routing::add;
    const BLOCK: usize = 4096;
    let mut stems = BusStems::default();
    if feeds.is_empty() {
        return Ok(stems);
    }
    let (exporting, mut excerpt) = match output {
        BusOutput::Export => (true, None),
        BusOutput::Audition(excerpt) => (false, excerpt),
    };
    control.progress("mix_buses", 0.78, None)?;
    // Only routed inputs are read. The scratch and effect state are independent of song length.
    let mut readers = feeds
        .into_iter()
        .map(|(path, levels)| Ok((hound::WavReader::open(path)?.into_samples::<f32>(), levels)))
        .collect::<Result<Vec<_>>>()?;
    let group_count = project.mix_groups.len();
    let capture_bus = excerpt.as_ref().and_then(|excerpt| {
        use crate::audition::SavedOutput::{Group, Return, Track};
        match excerpt.output {
            Group(id) => project.mix_groups.iter().position(|bus| &bus.id == id),
            Return(id) => project
                .aux_buses
                .iter()
                .position(|bus| &bus.id == id)
                .map(|index| index + group_count),
            Track(_) => None,
        }
    });
    let mut buses = crate::routing::buses(project)
        .enumerate()
        .filter(|(index, bus)| {
            !bus.mute && readers.iter().any(|(_, levels)| levels[1 + *index] != 0.)
        })
        .map(|(index, bus)| {
            let temp = sidechains.dir.join(format!(".bus_{index}.float.wav"));
            let writer = exporting.then(|| float_stem(&temp)).transpose()?;
            let name = file_component(&bus.name);
            let (prefix, number) = if index < group_count {
                ("GR", index + 1)
            } else {
                ("FX", index - group_count + 1)
            };
            let file = format!(
                "{prefix}{number:02}_{}.wav",
                name.trim_end_matches(['.', ' '])
            );
            Ok(BusStem {
                index,
                bus_id: &bus.id,
                gain: bus.gain(),
                chain: crate::effects::Chain::new(&bus.effects, 48_000., |id| sidechains.index(id)),
                input: vec![[0.; 2]; BLOCK],
                file: (file, temp),
                writer,
                peak: 0.,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut keys = sidechains.inputs(
        crate::routing::buses(project)
            .filter(|bus| buses.iter().any(|active| active.bus_id == bus.id))
            .flat_map(|bus| &bus.effects),
    )?;
    let mut dry = vec![[0.; 2]; BLOCK];
    for (block, mixed) in mix.chunks_mut(BLOCK).enumerate() {
        control.check()?;
        let size = mixed.len();
        for bus in &mut buses {
            bus.input[..size].fill([0.; 2]);
        }
        for (samples, levels) in &mut readers {
            for frame in &mut dry[..size] {
                for channel in frame {
                    *channel = samples.next().context("Mix bus source ended early")??;
                }
            }
            for bus in &mut buses {
                let gain = levels[1 + bus.index];
                if gain != 0. {
                    for (source, total) in dry.iter().copied().zip(&mut bus.input[..size]) {
                        add(source, gain, total);
                    }
                }
            }
        }
        for (j, total) in mixed.iter_mut().enumerate() {
            let peaks = keys.next()?;
            for bus in &mut buses {
                let mut frame = bus.input[j];
                bus.chain.process(&mut frame, peaks);
                let frame = frame.map(|sample| (f64::from(sample) * bus.gain) as f32);
                ensure!(
                    frame.iter().all(|sample| sample.is_finite()),
                    "A mix bus produced non-finite audio"
                );
                let peak = crate::effects::peak(frame);
                bus.peak = bus.peak.max(peak);
                if capture_bus == Some(bus.index) {
                    excerpt.as_mut().unwrap().write(block * BLOCK + j, frame);
                }
                if let Some(values) = &mut envelope {
                    let stem = &mut values[block * BLOCK + j];
                    *stem = stem.max(peak);
                }
                for (sample, mixed) in frame.into_iter().zip(total.iter_mut()) {
                    *mixed += sample;
                    if let Some(writer) = &mut bus.writer {
                        writer.write_sample(sample)?;
                    }
                }
            }
        }
    }
    for bus in buses {
        stems.peak = stems.peak.max(bus.peak);
        if let Some(writer) = bus.writer {
            writer.finalize()?;
            let report = if bus.index < group_count {
                &mut stems.groups
            } else {
                &mut stems.returns
            };
            let mut line =
                json!({"bus_id":bus.bus_id,"stem":bus.file.0,"peak_linear_before_master":bus.peak});
            bus.chain.reduction_report(&mut line);
            report.push(line);
            stems.files.push(bus.file);
        }
    }
    Ok(stems)
}

fn file_component(name: &str) -> String {
    name.chars()
        .filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control())
        .take(60)
        .collect()
}

pub(crate) fn track_stem_filename(index: usize, name: &str) -> String {
    let component = file_component(name);
    format!(
        "{:02}_{}.wav",
        index + 1,
        component.trim_end_matches(['.', ' '])
    )
}

pub(crate) fn audition_mix(
    root: &Path,
    p: &Project,
    mode: &str,
    first: usize,
    end: usize,
    output: Option<&crate::audition::SavedOutput>,
    control: &crate::jobs::Control,
) -> Result<Vec<[f32; 2]>> {
    if p.seconds() == 0. {
        return Ok(vec![[0.; 2]; end - first]);
    }
    match render_inner(
        root,
        p,
        mode,
        RenderTarget::Audition { first, end, output },
        control,
    )? {
        Rendered::Audition(frames) => Ok(frames),
        _ => unreachable!(),
    }
}

fn render_inner(
    root: &Path,
    p: &Project,
    mode: &str,
    target: RenderTarget<'_>,
    control: &crate::jobs::Control,
) -> Result<Rendered> {
    ensure!(RENDER_ENGINES.contains(&mode), "Unsupported engine");
    let scratch;
    let (job, monitor, audition) = match target {
        RenderTarget::Export { job, monitor } => (job, monitor, false),
        RenderTarget::Audition { .. } | RenderTarget::Monitor(_) => {
            scratch = uid("compare_work");
            (
                scratch.as_str(),
                false,
                matches!(target, RenderTarget::Audition { .. }),
            )
        }
    };
    let monitor_tracks = match target {
        RenderTarget::Monitor(ids) => Some(ids),
        _ => None,
    };
    let mut excerpt = match target {
        RenderTarget::Audition {
            first,
            end,
            output: Some(output),
        } => Some(OutputExcerpt {
            output,
            first,
            frames: vec![[0.; 2]; end - first],
        }),
        _ => None,
    };
    identifier(job)?;
    control.check()?;
    let final_dir = root.join("exports").join(job);
    ensure!(!final_dir.exists(), "Render output already exists");
    let dir = root.join("runtime/render_work").join(job);
    fs::create_dir_all(dir.parent().unwrap())?;
    fs::create_dir(&dir)?;
    let _work = RenderWork(dir.clone());
    let solo = p.tracks.iter().any(|t| t.solo && !t.mute);
    use crate::source_cache::{active, fader, frame_count, has_material, Source};
    let audible_track = |track: &Track| {
        active(track, solo)
            && crate::routing::group_open(track.output_group.as_deref(), &p.mix_groups)
    };
    use crate::effects::Effect;
    let mut requested_keys = std::collections::HashSet::new();
    let mut bus_inputs = [false; crate::routing::BUS_LIMIT];
    for track in p
        .tracks
        .iter()
        .filter(|track| monitor_tracks.is_none() && audible_track(track))
    {
        requested_keys.extend(track.effects.iter().filter_map(Effect::active_sidechain));
        let levels = crate::routing::levels(
            track.output_group.as_deref(),
            &track.sends,
            &p.mix_groups,
            &p.aux_buses,
        );
        for (active, gain) in bus_inputs.iter_mut().zip(&levels[1..]) {
            *active |= *gain != 0.;
        }
    }
    for (bus, active) in crate::routing::buses(p).zip(bus_inputs) {
        if active && !bus.mute {
            requested_keys.extend(bus.effects.iter().filter_map(Effect::active_sidechain));
        }
    }
    let len = frame_count(p);
    let prepare_all = monitor;
    let uses_voicevox = |t: &Track| {
        !t.notes.is_empty()
            && t.instrument.is_none()
            && !t.is_audio()
            && voice_route(mode, t).0 == "voicevox"
    };
    let needs_voicevox = p.tracks.iter().filter(|t| uses_voicevox(t)).any(|t| {
        monitor_tracks.map_or_else(
            || audible_track(t) || requested_keys.contains(t.id.as_str()),
            |ids| ids.contains(t.id.as_str()),
        )
    });
    let mut voice_backend = String::new();
    let voice_engine = if needs_voicevox || prepare_all && p.tracks.iter().any(uses_voicevox) {
        let setup = (|| -> Result<VoiceEngine> {
            let engine = VoiceEngine::load(control)?;
            voice_backend = engine.fingerprint()?;
            Ok(engine)
        })();
        control.check()?;
        if needs_voicevox {
            Some(setup?)
        } else {
            setup.ok()
        }
    } else {
        None
    };
    ensure!(
        len > if audition { 0 } else { 16800 },
        "Project has no notes"
    );
    let mut mix = vec![[0f32; 2]; if monitor_tracks.is_some() { 0 } else { len }];
    let mut envelope = p
        .master_limiter
        .as_ref()
        .filter(|_| monitor_tracks.is_none())
        .map(|_| vec![0_f32; len]);
    let mut stem_peak = 0_f32;
    let mut processed_tracks = Vec::new();
    let mut stems = vec![];
    let mut feeds = vec![];
    let mut input_only = vec![];
    let mut preview_sources = Vec::new();
    let mut reused = Vec::new();
    let mut reused_preview_voice = Vec::new();
    let mut monitor_unavailable = Vec::new();
    let mut credits = vec![];
    let mut voice_sources = serde_json::Map::new();
    let mut openutau_profiles = HashMap::<String, Result<(_, String), String>>::new();
    let mut track_progress = 0;
    let active_count = p
        .tracks
        .iter()
        .filter(|t| audible_track(t) || prepare_all && has_material(t))
        .count()
        + p.tracks
            .iter()
            .filter(|t| requested_keys.contains(t.id.as_str()) && has_material(t))
            .count();
    let span = 0.74 / active_count.max(1) as f64;
    let mut render_source = |t: &Track, base: f64, credited: bool| -> Result<Vec<[f32; 2]>> {
        let (voice_kind, profile) = voice_route(mode, t);
        let openutau = if t.instrument.is_none() && !t.is_audio() && voice_kind == "openutau" {
            let provider = openutau_profiles
                .entry(profile.to_owned())
                .or_insert_with(|| {
                    (|| -> Result<_> {
                        let engine = crate::openutau::Engine::load(root, profile)?;
                        let fingerprint = engine.fingerprint(control)?;
                        Ok((engine, fingerprint))
                    })()
                    .map_err(|error| error.to_string())
                });
            control.check()?;
            Some(
                provider
                    .as_ref()
                    .map_err(|error| anyhow::anyhow!("{} · {}", t.name, error))?,
            )
        } else {
            None
        };
        let (kind, backend) = if t.instrument.is_some() {
            ("instrument", crate::instruments::FONT_SHA)
        } else if t.is_audio() {
            ("audio", "")
        } else {
            (
                voice_kind,
                openutau.map_or(voice_backend.as_str(), |(_, fingerprint)| fingerprint),
            )
        };
        let voice = (kind == "voicevox")
            .then(|| -> Result<_> {
                let engine = voice_engine
                    .as_ref()
                    .context("Voice source is unavailable")?;
                let singer = t.singer_id.unwrap_or(engine.default_singer);
                Ok((engine, singer, engine.singer_bank(singer)?))
            })
            .transpose()?;
        if !audition && credited && kind == "voicevox" {
            let (_, singer, bank) = voice.context("Voice source is unavailable")?;
            credits.push(format!("VOICEVOX:{}", bank["name"].as_str().unwrap_or("")));
            if let Some(uuid) = bank["speaker_uuid"].as_str() {
                if let Ok(info) = get(&format!("/singer_info?speaker_uuid={uuid}")) {
                    fs::write(
                        dir.join(format!("音源说明_{}.json", singer)),
                        serde_json::to_vec_pretty(
                            &json!({"name":bank["name"],"policy":info["policy"],"source":"https://voicevox.hiroshiba.jp/term/"}),
                        )?,
                    )?;
                }
            }
        }
        if !audition && credited && kind == "openutau" {
            let (engine, _) = openutau.context("OpenUtau 声库不可用")?;
            credits.push(engine.credit());
        }
        if let Some((engine, _)) = openutau {
            voice_sources.insert(t.id.clone(), engine.describe(profile));
        } else if let Some((_, singer, bank)) = voice {
            voice_sources.insert(
                t.id.clone(),
                json!({"engine":"voicevox","singer_id":singer,"name":bank["name"]}),
            );
        } else if kind == "guide" {
            voice_sources.insert(t.id.clone(), json!({"engine":"guide"}));
        }
        if !audition && credited && t.instrument.is_some() {
            credits.push(crate::instruments::FONT_NAME.to_string());
            fs::copy(
                root.join("runtime/soundfonts/LICENSE.txt"),
                dir.join("乐器音源许可.txt"),
            )?;
        }
        let source = Source::new(p, t, kind, backend)?;
        if monitor_tracks.is_some() && source.open(root, &p.id, len).is_ok() {
            control.check()?;
            preview_sources.push(source);
            return Ok(vec![]);
        }
        Ok(match source.read(root, &p.id, len, || control.check()) {
            Ok(samples) => {
                control.progress("cached_source", base, Some((&t.id, &t.name)))?;
                reused.push(t.id.clone());
                preview_sources.push(source);
                samples
            }
            Err(_) => {
                control.check()?;
                let preview = matches!(kind, "voicevox" | "openutau")
                    .then(|| source.preview_voice())
                    .flatten();
                control.progress(
                    if preview.is_some() {
                        "cached_voice_preview"
                    } else if kind == "voicevox" {
                        "voice_query"
                    } else if kind == "openutau" {
                        "voice_synthesis"
                    } else {
                        kind
                    },
                    base,
                    Some((&t.id, &t.name)),
                )?;
                let coverage = if audition {
                    len
                } else {
                    crate::source_cache::cache_frame_count(len).max(len)
                };
                let mut samples = if t.instrument.is_some() {
                    crate::instruments::render(root, p, t, coverage, control, base, span * 0.9)?
                } else if t.is_audio() {
                    let clock = p.timeline();
                    let mut stereo = vec![[0.; 2]; coverage];
                    for clip in t.media_clips().iter() {
                        let start = clip.start_frame(&clock) as usize;
                        let gain = 10f64.powf(f64::from(clip.gain_db) / 20.);
                        let envelope = clip.rendered_envelope(&clock);
                        crate::stretch::visit(root, clip, &clock, |frame, sample| {
                            if frame % 8192 == 0 {
                                control.check()?;
                            }
                            if let Some(target) = stereo.get_mut(start + frame as usize) {
                                let scale = gain * f64::from(envelope.gain(frame));
                                for (output, input) in target.iter_mut().zip(sample) {
                                    *output += (f64::from(input) * scale) as f32;
                                }
                            }
                            Ok(())
                        })?;
                    }
                    stereo
                } else {
                    let voice_samples;
                    let guide_samples;
                    let mono = if matches!(kind, "voicevox" | "openutau") {
                        voice_samples = if let Some(prepared) = preview {
                            reused_preview_voice.push(t.id.clone());
                            prepared
                        } else {
                            let rendered = if let Some((engine, _)) = openutau {
                                engine.render(root, p, t, &t.notes, 0., control)?
                            } else {
                                let (engine, _, _) =
                                    voice.context("Voice source is unavailable")?;
                                voice_track(p, t, &t.notes, 0, engine, control, (base, span))?
                            };
                            if audition {
                                source.keep_preview_voice(rendered)
                            } else {
                                std::sync::Arc::new(rendered)
                            }
                        };
                        voice_samples.as_slice()
                    } else {
                        guide_samples = guide_track(p, t, control)?;
                        guide_samples.as_slice()
                    };
                    let mut stereo = vec![[0.; 2]; coverage];
                    for (j, (target, input)) in stereo.iter_mut().zip(mono).enumerate() {
                        if j % 8192 == 0 {
                            control.check()?;
                        }
                        *target = [*input; 2];
                    }
                    stereo
                };
                if !audition {
                    let written = source.write(root, &p.id, &samples, || control.check());
                    if monitor_tracks.is_some() {
                        written
                            .as_ref()
                            .map_err(|error| anyhow::anyhow!("无法保存监听音轨: {error:#}"))?;
                    }
                    if written.is_ok() {
                        if matches!(kind, "voicevox" | "openutau") {
                            source.discard_preview_voice();
                        }
                        preview_sources.push(source);
                    }
                }
                control.check()?;
                samples.truncate(len);
                samples
            }
        })
    };
    if let Some(ids) = monitor_tracks {
        let span = 0.9 / ids.len().max(1) as f64;
        for (index, track) in p
            .tracks
            .iter()
            .filter(|track| ids.contains(track.id.as_str()))
            .enumerate()
        {
            render_source(track, 0.04 + index as f64 * span, false)?;
        }
        ensure!(preview_sources.len() == ids.len(), "监听音轨缺失");
        return Ok(Rendered::Monitor(preview_sources));
    }
    let mut sidechains = SidechainFiles {
        dir: &dir,
        files: Default::default(),
    };
    for (index, track) in p.tracks.iter().enumerate() {
        if requested_keys.contains(track.id.as_str()) && has_material(track) {
            let base = 0.04 + track_progress as f64 * span;
            track_progress += 1;
            control.progress("sidechain_sources", base, Some((&track.id, &track.name)))?;
            let samples = render_source(track, base, true)?;
            sidechains.write(&track.id, index, &samples, control)?;
        }
    }
    for (i, t) in p.tracks.iter().enumerate() {
        let audible = audible_track(t);
        if !(audible || prepare_all && has_material(t)) {
            continue;
        }
        let base = 0.04 + track_progress as f64 * span;
        track_progress += 1;
        if !audible && sidechains.index(&t.id).is_some() {
            continue;
        }
        let dry = match sidechains.read(&t.id, len, control)? {
            Some(samples) => Ok(samples),
            None => render_source(t, base, audible),
        };
        let mut stereo = match dry {
            Ok(samples) => samples,
            Err(error) if !audible => {
                control.check()?;
                monitor_unavailable.push(json!({"track_id":t.id,"reason":error.to_string()}));
                continue;
            }
            Err(error) => return Err(error),
        };
        if !audible {
            continue;
        }
        if t.effects.iter().any(|effect| !effect.bypassed()) {
            control.progress("track_effects", base + span * 0.91, Some((&t.id, &t.name)))?;
            let mut chain =
                crate::effects::Chain::new(&t.effects, 48_000., |id| sidechains.index(id));
            let mut keys = sidechains.inputs(t.effects.iter())?;
            for block in stereo.chunks_mut(8192) {
                control.check()?;
                for frame in block {
                    chain.process(frame, keys.next()?);
                }
            }
            let mut report = json!({"track_id":t.id,"effects":t.effects});
            chain.reduction_report(&mut report);
            processed_tracks.push(report);
        }
        let fader = fader(t);
        let gain_envelope = crate::automation::Envelope::compile(&t.gain_automation, &p.timeline());
        let mut automation = gain_envelope.cursor();
        let file = track_stem_filename(i, &t.name);
        let temp = dir.join(format!(".stem_{}.float.wav", i));
        let levels = crate::routing::levels(
            t.output_group.as_deref(),
            &t.sends,
            &p.mix_groups,
            &p.aux_buses,
        );
        let direct = levels[0] != 0.;
        let capture_track = direct && excerpt.as_ref().is_some_and(|excerpt|
            matches!(excerpt.output, crate::audition::SavedOutput::Track(id) if id == &t.id));
        let feeds_bus = crate::routing::buses(p)
            .zip(levels[1..].iter().copied())
            .any(|(bus, gain)| !bus.mute && gain != 0.);
        if !direct && !feeds_bus {
            continue;
        }
        let mut writer = (!audition && direct || feeds_bus)
            .then(|| float_stem(&temp))
            .transpose()?;
        control.progress("mixing", base + span * 0.95, Some((&t.id, &t.name)))?;
        for (j, (frame, total)) in stereo.into_iter().zip(&mut mix).enumerate() {
            if j % 8192 == 0 {
                control.check()?;
            }
            let trim = automation.at(j as f64);
            let frame = [
                (f64::from(frame[0]) * fader[0] * trim) as f32,
                (f64::from(frame[1]) * fader[1] * trim) as f32,
            ];
            ensure!(
                frame.iter().all(|sample| sample.is_finite()),
                "A track produced non-finite audio"
            );
            if direct {
                let peak = crate::effects::peak(frame);
                stem_peak = stem_peak.max(peak);
                if let Some(values) = &mut envelope {
                    values[j] = values[j].max(peak);
                }
                for (sample, mixed) in frame.into_iter().zip(total) {
                    *mixed += sample;
                }
            }
            if capture_track {
                excerpt.as_mut().unwrap().write(j, frame);
            }
            if let Some(writer) = &mut writer {
                for sample in frame {
                    writer.write_sample(sample)?;
                }
            }
        }
        if let Some(writer) = writer {
            writer.finalize()?;
        }
        if feeds_bus {
            feeds.push((temp.clone(), levels));
        }
        if !audition && direct {
            stems.push((file, temp));
        } else if feeds_bus {
            input_only.push(temp);
        }
    }
    let routed = bus_stems(
        p,
        &sidechains,
        &mut mix,
        envelope.as_deref_mut(),
        feeds,
        if audition {
            BusOutput::Audition(excerpt.as_mut())
        } else {
            BusOutput::Export
        },
        control,
    )?;
    sidechains.remove()?;
    if !requested_keys.is_empty() {
        let order = |id: &str| p.tracks.iter().position(|track| track.id == id);
        preview_sources.sort_by_key(|source| order(&source.track_id));
        reused.sort_by_key(|id| order(id));
        reused_preview_voice.sort_by_key(|id| order(id));
    }
    for path in input_only {
        fs::remove_file(path)?;
    }
    stems.extend(routed.files);
    stem_peak = stem_peak.max(routed.peak);
    ensure!(
        audition || !stems.is_empty() || monitor && !preview_sources.is_empty(),
        "All tracks are empty or muted"
    );
    let mut master_report = Value::Null;
    let mut final_trim = 1_f32;
    if let (Some(limiter), Some(envelope)) = (&p.master_limiter, &mut envelope) {
        control.progress("master_effects", 0.785, None)?;
        let reduction = limiter.gain_envelope(&mix, envelope, || control.check())?;
        let mut detector = limiter
            .is_true_peak()
            .then(crate::loudness::TruePeak::default);
        for (index, (frame, gain)) in mix.iter_mut().zip(envelope.iter()).enumerate() {
            if index % 8192 == 0 {
                control.check()?;
            }
            *frame = frame.map(|sample| sample * gain);
            if let Some(detector) = &mut detector {
                detector.push(*frame);
            }
        }
        master_report = json!({"settings":limiter,"max_gain_reduction_db":reduction,"measurement":"sample_peak"});
        if let Some(mut detector) = detector {
            let limit = limiter.detection_ceiling();
            let post_peak = detector.flush();
            final_trim = limit / post_peak.max(limit);
            let trim_db = 20. * final_trim.log10();
            master_report["measurement"] = json!("true_peak");
            master_report["final_trim_db"] = json!(trim_db);
            master_report["margin_db"] = json!(crate::effects::Limiter::TRUE_PEAK_MARGIN_DB);
            master_report["max_gain_reduction_db"] = json!(reduction - trim_db);
        }
    }
    // Share master gain with the aligned stems, including any per-frame limiter gain.
    let peak = mix
        .iter()
        .copied()
        .map(crate::effects::peak)
        .fold(0f32, f32::max);
    let non_silent = peak > 1e-6;
    ensure!(
        audition || non_silent || monitor && !preview_sources.is_empty(),
        "Rendered audio is silent"
    );
    let mut scale = if envelope.is_none() && peak > 0.95 {
        0.95 / peak
    } else {
        final_trim
    };
    if envelope.is_none() && stem_peak * scale > 1. {
        scale = 0.999 / stem_peak;
    }
    if let Some(mut excerpt) = excerpt {
        for (index, frame) in excerpt.frames.iter_mut().enumerate() {
            if index % 8192 == 0 {
                control.check()?;
            }
            let master = envelope
                .as_ref()
                .and_then(|gains| gains.get(excerpt.first + index))
                .copied()
                .unwrap_or(1.);
            *frame = frame.map(|sample| sample * master * scale);
        }
        return Ok(Rendered::Audition(excerpt.frames));
    }
    if let RenderTarget::Audition { first, end, .. } = target {
        let mut segment = vec![[0.; 2]; end - first];
        if first < mix.len() {
            for (index, (output, input)) in segment.iter_mut().zip(&mix[first..]).enumerate() {
                if index % 8192 == 0 {
                    control.check()?;
                }
                *output = input.map(|sample| sample * scale);
            }
        }
        return Ok(Rendered::Audition(segment));
    }
    let sumsq: f64 = mix
        .iter()
        .flatten()
        .map(|x| (*x as f64 * scale as f64).powi(2))
        .sum();
    for (index, (file, temp)) in stems.iter().enumerate() {
        control.progress(
            "writing_stems",
            0.79 + 0.13 * index as f64 / stems.len() as f64,
            None,
        )?;
        let mut reader = hound::WavReader::open(temp)?;
        let mut writer = hound::WavWriter::create(
            dir.join(file),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for (j, sample) in reader.samples::<f32>().enumerate() {
            if j % 16384 == 0 {
                control.check()?;
            }
            let master_gain = envelope.as_ref().map_or(1., |gains| gains[j / 2]);
            writer.write_sample((sample? * master_gain * scale * 8_388_607.).round() as i32)?
        }
        writer.finalize()?;
        drop(reader);
        fs::remove_file(temp)?
    }
    control.progress("writing_mix", 0.93, None)?;
    let mut meter = crate::loudness::Meter::default();
    write_wave(
        &dir.join("试听混音.wav"),
        &mix,
        scale,
        control,
        Some(&mut meter),
    )?;
    let loudness = meter.finish();
    if let Some(limiter) = p
        .master_limiter
        .as_ref()
        .filter(|limiter| limiter.is_true_peak())
    {
        ensure!(
            loudness
                .true_peak_dbtp
                .is_none_or(|peak| peak <= limiter.ceiling_db),
            "Exported PCM exceeded the requested true-peak ceiling"
        );
    }
    fs::write(dir.join("工程快照.json"), serde_json::to_vec_pretty(p)?)?;
    credits.sort();
    credits.dedup();
    fs::write(
        dir.join("音源署名.txt"),
        format!("{}\n{}\n{}\n乐器音源：https://github.com/mrbumpy409/GeneralUser-GS\n合成与采样乐器不标为真人录制。",credits.join("\n"),if voice_sources.values().any(|source|source["engine"]=="guide"){"歌声导唱使用内置振荡器，不合成歌词。"}else{""},
            if voice_sources.values().any(|source|source["engine"]=="voicevox") {"歌声音源：https://voicevox.hiroshiba.jp/term/"} else {""}),
    )?;
    let result = json!({"job_id":job,"engine":mode,"project_id":p.id,"revision":p.revision,"sample_rate":48000,"channels":2,"seconds":len as f64/48000.,"tail_seconds":p.tail_seconds,"timing_schema":p.format,"tempo_points":p.tempo_map.len(),"meter_points":p.meter_map.len(),"instrument_bank_sha256":if p.tracks.iter().any(|t|t.instrument.is_some()){Some(crate::instruments::FONT_SHA)}else{None},"peak_dbfs":20.*(peak*scale).log10(),"rms_dbfs":20.*(sumsq/(len*2) as f64).sqrt().log10(),"attenuation":scale,"loudness":loudness,"track_effects":processed_tracks,"aux_returns":routed.returns,"group_stems":routed.groups,"master_limiter":master_report,"mix_url":format!("/exports/{job}/试听混音.wav"),"directory":final_dir,"stems":stems.iter().map(|x|x.0.clone()).collect::<Vec<_>>(),"credits":credits,"voice_sources":voice_sources,"audio_verified_non_silent":non_silent,"musical_quality_approved":false,"preview_sources":preview_sources,"reused_source_track_ids":reused,"reused_preview_voice_track_ids":reused_preview_voice,"prepared_for_monitor":monitor,"monitor_unavailable":monitor_unavailable});
    control.commit()?;
    fs::write(
        dir.join("渲染报告.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    fs::create_dir_all(root.join("exports"))?;
    fs::rename(&dir, &final_dir)?;
    Ok(Rendered::Export(result))
}
pub(crate) fn write_wav(
    path: &Path,
    data: &[[f32; 2]],
    scale: f32,
    control: &crate::jobs::Control,
) -> Result<()> {
    write_wave(path, data, scale, control, None)
}

fn write_wave(
    path: &Path,
    data: &[[f32; 2]],
    scale: f32,
    control: &crate::jobs::Control,
    mut meter: Option<&mut crate::loudness::Meter>,
) -> Result<()> {
    const PCM_FULL_SCALE: f32 = 8_388_607.;
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for (j, frame) in data.iter().enumerate() {
        if j % 8192 == 0 {
            control.check()?;
        }
        let samples = frame.map(|sample| (sample * scale * PCM_FULL_SCALE).round() as i32);
        for sample in samples {
            w.write_sample(sample)?;
        }
        if let Some(meter) = &mut meter {
            meter.push(samples.map(|sample| sample as f32 / 8_388_608.));
        }
    }
    w.finalize()?;
    Ok(())
}

#[cfg(test)]
mod output_tests {
    use super::*;
    use crate::{audition::SavedOutput, jobs::Control, model::Store};

    #[test]
    fn local_voice_keeps_global_timing_and_sustained_vowel_phase() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_local_voice"));
        let store = Store::new(root.clone())?;
        let track = Track::new("Sustained vowel");
        let id = track.id.clone();
        let project = store.create("Local voice fixture", 120., [4, 4], vec![track])?;
        let project = Store::preview_edit(
            &project,
            &json!([
                {"op":"set_project","values":{"tempo_map":[{"beat":4,"bpm":92}]}},
                {"op":"update_track","track_id":id,"values":{"mute":true}},
                {"op":"add_notes","track_id":id,"notes":[
                    {"start":0,"duration":1,"pitch":60,"lyric":"あ"},
                    {"start":2,"duration":2,"pitch":64,"lyric":"か"},
                    {"start":4,"duration":4,"pitch":65,"lyric":"あ","vowel_continuation":true},
                    {"start":8,"duration":1,"pitch":67,"lyric":"あ","vowel_continuation":true},
                    {"start":12,"duration":1,"pitch":69,"lyric":"あ"}
                ]}
            ]),
        )?;
        let clock = project.timeline();
        let [first, end] = [7.8, 8.6].map(|beat| (clock.seconds(beat) * 48000.).round() as usize);
        let control =
            Control::ephemeral("local_voice_test", &project.id, project.revision, "guide");
        let full = guide_track(&project, &project.tracks[0], &control)?;
        let local = audition_voice_excerpt(&root, &project, "guide", &id, [first, end], &control)?;
        assert!(local.iter().any(|sample| sample[0] != 0.));
        assert!(local
            .iter()
            .zip(&full[first..end])
            .all(|(local, &full)| *local == [full; 2]));
        let settings = vocal_score(&project, &project.tracks[0].notes[1..4], 93.75, 40)?;
        assert_eq!(settings.slots.len(), 3);
        assert_eq!(settings.slots[2].head.id, project.tracks[0].notes[1].id);
        assert_eq!(
            settings.slots[2].start - settings.pad + settings.origin,
            (clock.seconds(8.) * 93.75).round() as usize
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn audition_outputs_match_exported_stems_under_master_limit() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_output_excerpt"));
        let store = Store::new(root.clone())?;
        let direct = Track::new("Guide direct");
        let routed = Track::new("Guide routed");
        let ids = [direct.id.clone(), routed.id.clone()];
        let project = store.create(
            "Focused audio excerpt fixture",
            120.,
            [4, 4],
            vec![direct, routed],
        )?;
        let project = Store::preview_edit(
            &project,
            &json!([
                {"op":"set_project","values":{
                    "mix_groups":[{"id":"g_tap","name":"Group","gain_db":2,"effects":[]}],
                    "aux_buses":[{"id":"r_tap","name":"Return","gain_db":-5,"effects":[
                        {"kind":"delay","time_ms":63,"mix":1,"feedback":0.16,"damping_hz":5000,"ping_pong":false}
                    ]}],
                    "master_limiter":{"ceiling_db":-20,"input_db":8,"lookahead_ms":5,"release_ms":80,"peak_mode":"true_peak"}
                }},
                {"op":"update_track","track_id":ids[1],"values":{"output_group":"g_tap","sends":[{"bus_id":"r_tap","gain_db":-3}]}},
                {"op":"update_track","track_id":ids[0],"values":{"effects":[{
                    "kind":"ducker","source_track_id":ids[1],"threshold_db":-34,
                    "range_db":6,"ratio":4,"attack_ms":8,"release_ms":150
                }]}},
                {"op":"add_notes","track_id":ids[0],"notes":[{"start":0,"duration":2,"pitch":60}]},
                {"op":"add_notes","track_id":ids[1],"notes":[{"start":0.1,"duration":1.5,"pitch":67}]}
            ]),
        )?;
        let control =
            || Control::ephemeral("measure_excerpt", &project.id, project.revision, "guide");
        let exported = render(
            &root,
            &project,
            "render_excerpt",
            "guide",
            false,
            &control(),
        )?;
        assert!(
            exported["master_limiter"]["max_gain_reduction_db"]
                .as_f64()
                .unwrap()
                > 0.5
        );
        let dir = crate::audition::export_directory(&root, "render_excerpt")?;
        let range = [9_800, 21_420];
        for output in [
            SavedOutput::Track(ids[0].clone()),
            SavedOutput::Group("g_tap".into()),
            SavedOutput::Return("r_tap".into()),
        ] {
            let names = crate::audition::output_files(
                &project,
                &exported,
                Some(std::slice::from_ref(&output)),
            )?;
            let normal = crate::audition::read_output(&dir, &names, range)?;
            let tapped = audition_mix(
                &root,
                &project,
                "guide",
                range[0],
                range[1],
                Some(&output),
                &control(),
            )?;
            assert!(normal.iter().flatten().any(|sample| sample.abs() > 0.002));
            assert_eq!(normal.len(), tapped.len());
            assert!(
                normal
                    .iter()
                    .flatten()
                    .zip(tapped.iter().flatten())
                    .all(|(a, b)| (a * 8_388_607.).round() == (b * 8_388_607.).round()),
                "{output:?} diverged from export"
            );
        }
        let muted = Store::preview_edit(
            &project,
            &json!([
                {"op":"update_track","track_id":ids[0],"values":{"mute":true}}
            ]),
        )?;
        let quiet = audition_mix(
            &root,
            &muted,
            "guide",
            range[0],
            range[1],
            Some(&SavedOutput::Track(ids[0].clone())),
            &control(),
        )?;
        assert!(quiet.iter().flatten().all(|sample| *sample == 0.));
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
