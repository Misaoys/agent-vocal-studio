use crate::model::*;
use anyhow::{ensure, Context, Result};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
pub const FONT_SHA: &str = "9575028c7a1f589f5770fccc8cff2734566af40cd26ed836944e9a5152688cfe";
pub const FONT_NAME: &str = "GeneralUser GS 2.0.3";
type FontCache = Mutex<HashMap<PathBuf, Arc<SoundFont>>>;
static FONTS: OnceLock<FontCache> = OnceLock::new();
fn load(root: &Path) -> Result<Arc<SoundFont>> {
    let path = root.join("runtime/soundfonts/GeneralUser-GS.sf2");
    let cache = FONTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(font) = cache.lock().unwrap().get(&path) {
        return Ok(font.clone());
    }
    let bytes = fs::read(&path).context("Instrument samples unavailable: run 安装乐器音源.ps1")?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == FONT_SHA,
        "Instrument sample bank checksum mismatch; reinstall the pinned bank"
    );
    let font = Arc::new(SoundFont::new(&mut Cursor::new(bytes))?);
    cache.lock().unwrap().insert(path, font.clone());
    Ok(font)
}
fn family(program: i32, drums: bool) -> &'static str {
    if drums {
        return "鼓组";
    }
    let families = [
        "钢琴",
        "半音阶打击",
        "风琴",
        "吉他",
        "贝斯",
        "弦乐",
        "合奏",
        "铜管",
        "簧管",
        "笛管",
        "合成主音",
        "合成铺底",
        "合成音效",
        "民族乐器",
        "打击乐器",
        "音效",
    ];
    families[(program as usize / 8).min(15)]
}
pub fn catalog(root: &Path) -> Result<Value> {
    let font = load(root)?;
    let mut presets=font.get_presets().iter().map(|p|{let bank=p.get_bank_number();let program=p.get_patch_number();let drums=bank==128;json!({"name":p.get_name(),"bank":bank,"program":program,"drums":drums,"family":family(program,drums)})}).collect::<Vec<_>>();
    presets.sort_by_key(|p| (p["bank"].as_i64(), p["program"].as_i64()));
    Ok(
        json!({"engine":"rustysynth","native_language":"Rust","sample_bank":FONT_NAME,"sha256":FONT_SHA,"installed":true,"presets":presets,"supported_controllers":[1,7,10,11,64,91,93],"controller_defaults":{"1":0,"7":100,"10":64,"11":127,"64":0,"91":40,"93":0},"controller_names":{"1":"颤音/调制","7":"MIDI 音量","10":"MIDI 声像","11":"表情","64":"延音踏板","91":"混响发送","93":"合唱发送"},"percussion_notes":drums(),"limits":["每音音高曲线暂属歌声；乐器使用已支持的轨道 MIDI 控制器","同音高重叠重击使用另一音轨","MIDI 渐变速度使用离散事件；原生工程保留精确曲线"],"source":"https://github.com/mrbumpy409/GeneralUser-GS"}),
    )
}
pub fn status(root: &Path) -> Value {
    match load(root) {
        Ok(font) => {
            json!({"installed":true,"sample_bank":FONT_NAME,"preset_count":font.get_presets().len(),"sha256":FONT_SHA})
        }
        Err(e) => json!({"installed":false,"error":format!("{e:#}")}),
    }
}
pub fn drums() -> Value {
    let names = [
        "原声底鼓",
        "底鼓",
        "边击",
        "原声军鼓",
        "拍手",
        "电军鼓",
        "低地嗵",
        "闭镲",
        "高地嗵",
        "踏镲",
        "低嗵",
        "开镲",
        "低中嗵",
        "高嗵",
        "碎音镲 1",
        "高中嗵",
        "叮叮镲 1",
        "中国镲",
        "叮叮镲帽",
        "铃鼓",
        "水镲",
        "牛铃",
        "碎音镲 2",
        "振音叉",
        "叮叮镲 2",
        "高邦戈",
        "低邦戈",
        "弱音高康加",
        "开放高康加",
        "低康加",
        "高天巴鼓",
        "低天巴鼓",
        "高阿果果",
        "低阿果果",
        "沙锤",
        "响葫芦",
        "短哨",
        "长哨",
        "短刮瓜",
        "长刮瓜",
        "击木",
        "高木鱼",
        "低木鱼",
        "弱音库卡",
        "开放库卡",
        "弱音三角铁",
        "开放三角铁",
    ];
    json!(names
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"pitch":i+35,"name":n}))
        .collect::<Vec<_>>())
}
#[derive(Clone, Debug)]
struct Event {
    sample: usize,
    order: u8,
    message: u8,
    data1: u8,
    data2: u8,
}
fn events(p: &Project, t: &Track) -> Vec<Event> {
    let clock = p.timeline();
    let to_sample = |beat: f64| (clock.seconds(beat) * 48000.).round() as usize;
    let mut events = vec![];
    for n in &t.notes {
        events.push(Event {
            sample: to_sample(n.start),
            order: 2,
            message: 0x90,
            data1: n.pitch,
            data2: n.velocity.round() as u8,
        });
        events.push(Event {
            sample: to_sample(n.start + n.duration),
            order: 0,
            message: 0x80,
            data1: n.pitch,
            data2: 0,
        });
    }
    for c in &t.controllers {
        events.push(Event {
            sample: to_sample(c.beat),
            order: 1,
            message: 0xb0,
            data1: c.controller,
            data2: c.value,
        });
    }
    let end = events.iter().map(|e| e.sample).max().unwrap_or(0);
    events.push(Event {
        sample: end,
        order: 3,
        message: 0xb0,
        data1: 64,
        data2: 0,
    });
    events.sort_by_key(|e| (e.sample, e.order));
    events
}
pub fn render(
    root: &Path,
    p: &Project,
    t: &Track,
    len: usize,
    control: &crate::jobs::Control,
    base: f64,
    span: f64,
) -> Result<Vec<[f32; 2]>> {
    let instrument = t
        .instrument
        .as_ref()
        .context("Track is not a sampled instrument")?;
    let font = load(root)?;
    let bank = if instrument.drums {
        128
    } else {
        instrument.bank as i32
    };
    ensure!(
        font.get_presets()
            .iter()
            .any(|preset| preset.get_bank_number() == bank
                && preset.get_patch_number() == instrument.program as i32),
        "The selected bank/program does not exist in the installed soundfont"
    );
    let mut settings = SynthesizerSettings::new(48000);
    settings.maximum_polyphony = 256;
    let mut synth = Synthesizer::new(&font, &settings)?;
    let channel = if instrument.drums { 9 } else { 0 };
    synth.process_midi_message(channel, 0xb0, 0, if instrument.drums { 0 } else { bank });
    synth.process_midi_message(channel, 0xc0, instrument.program as i32, 0);
    let mut out = vec![[0f32; 2]; len];
    let mut left = vec![0f32; 4096];
    let mut right = vec![0f32; 4096];
    let mut position = 0;
    let render_until = |end: usize,
                        position: &mut usize,
                        synth: &mut Synthesizer,
                        out: &mut Vec<[f32; 2]>,
                        left: &mut Vec<f32>,
                        right: &mut Vec<f32>|
     -> Result<()> {
        while *position < end {
            control.progress(
                "instrument",
                base + span * *position as f64 / len.max(1) as f64,
                Some((&t.id, &t.name)),
            )?;
            let count = (end - *position).min(left.len());
            synth.render(&mut left[..count], &mut right[..count]);
            for i in 0..count {
                out[*position + i] = [left[i], right[i]]
            }
            *position += count
        }
        Ok(())
    };
    for event in events(p, t) {
        render_until(
            event.sample.min(len),
            &mut position,
            &mut synth,
            &mut out,
            &mut left,
            &mut right,
        )?;
        synth.process_midi_message(
            channel,
            event.message as i32,
            event.data1 as i32,
            event.data2 as i32,
        );
    }
    render_until(
        len,
        &mut position,
        &mut synth,
        &mut out,
        &mut left,
        &mut right,
    )?;
    ensure!(
        out.iter().flatten().all(|s| s.is_finite()),
        "Non-finite instrument output"
    );
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instrument_events_are_timed_and_release_before_retrigger() -> Result<()> {
        let mut t = Track::new("钢琴");
        t.instrument = Some(Instrument::default());
        t.notes = vec![
            Note::from_value(&json!({"start":1,"duration":1,"pitch":60}))?,
            Note::from_value(&json!({"start":2,"duration":1,"pitch":60}))?,
        ];
        let root = std::env::temp_dir().join(uid("inst_events"));
        let store = Store::new(root.clone())?;
        let p = store.create("test", 120., [4, 4], vec![t])?;
        let ev = events(&p, &p.tracks[0]);
        assert_eq!(ev[0].sample, 24000);
        let at = ev
            .iter()
            .filter(|e| e.sample == 48000)
            .map(|e| e.message)
            .collect::<Vec<_>>();
        assert_eq!(at, vec![0x80, 0x90]);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
