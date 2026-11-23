use crate::{drum_kit, model::*};
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
    let font = load(root);
    let mut presets=font.as_ref().ok().into_iter().flat_map(|font| font.get_presets().iter()).map(|p|{let bank=p.get_bank_number();let program=p.get_patch_number();let drums=bank==128;json!({"name":p.get_name(),"bank":bank,"program":program,"drums":drums,"family":family(program,drums)})}).collect::<Vec<_>>();
    presets.extend(drum_kit::KITS.iter().map(|kit| json!({"name":kit.name,"bank":drum_kit::BANK,"program":kit.program,"drums":true,"family":"鼓组","built_in":true,"supported_controllers":[7,10,11]})));
    presets.sort_by_key(|p| (p["bank"].as_i64(), p["program"].as_i64()));
    Ok(
        json!({"engine":"rustysynth","native_language":"Rust","sample_bank":FONT_NAME,"sha256":FONT_SHA,"installed":font.is_ok(),"soundfont_error":font.err().map(|e| format!("{e:#}")),"presets":presets,"supported_controllers":[1,7,10,11,64,91,93],"controller_defaults":{"1":0,"7":100,"10":64,"11":127,"64":0,"91":40,"93":0},"controller_names":{"1":"颤音/调制","7":"MIDI 音量","10":"MIDI 声像","11":"表情","64":"延音踏板","91":"混响发送","93":"合唱发送"},"percussion_notes":drums(),"limits":["每音音高曲线暂属歌声；乐器使用已支持的轨道 MIDI 控制器","同音高重叠重击使用另一音轨","MIDI 渐变速度使用离散事件；原生工程保留精确曲线"],"source":"https://github.com/mrbumpy409/GeneralUser-GS"}),
    )
}
pub fn status(root: &Path) -> Value {
    let built_in = drum_kit::KITS.map(|kit| kit.name);
    match load(root) {
        Ok(font) => {
            json!({"installed":true,"built_in_drum_kit":drum_kit::NAME,"built_in_drum_kits":built_in,"sample_bank":FONT_NAME,"preset_count":font.get_presets().len(),"sha256":FONT_SHA})
        }
        Err(e) => {
            json!({"installed":false,"built_in_drum_kit":drum_kit::NAME,"built_in_drum_kits":built_in,"error":format!("{e:#}")})
        }
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
fn render_builtin(
    kit: drum_kit::Kit,
    p: &Project,
    t: &Track,
    len: usize,
    control: &crate::jobs::Control,
    base: f64,
    span: f64,
) -> Result<Vec<[f32; 2]>> {
    let events = events(p, t);
    // Precompute the next pedal / closed hat position to choke open hats.
    let mut chokes = vec![len; events.len()];
    let mut next = len;
    for (i, event) in events.iter().enumerate().rev() {
        chokes[i] = next;
        if event.message == 0x90 && event.data2 != 0 && matches!(event.data1, 42 | 44) {
            next = event.sample;
        }
    }
    let mut out = vec![[0.; 2]; len];
    for (i, event) in events.iter().enumerate() {
        if i % 64 == 0 {
            control.progress(
                "instrument",
                base + span * event.sample.min(len) as f64 / len.max(1) as f64,
                Some((&t.id, &t.name)),
            )?;
        }
        if event.message != 0x90 || event.data2 == 0 || event.sample >= len {
            continue;
        }
        let pcm = drum_kit::kit_sample(kit, event.data1);
        let hit_gain = 0.78 * (f32::from(event.data2) / 127.).powf(1.45);
        let end = (event.sample + pcm.len()).min(len);
        let choke = chokes[i].max(event.sample);
        let fade = drum_kit::RATE * 6 / 1000;
        let stop = if event.data1 == 46 {
            end.min(choke + fade)
        } else {
            end
        };
        for (frame, (out, sample)) in out[event.sample..stop].iter_mut().zip(pcm).enumerate() {
            let fade_gain = if event.data1 == 46 && event.sample + frame >= choke {
                1. - (event.sample + frame - choke) as f32 / fade as f32
            } else {
                1.
            };
            out[0] += hit_gain * fade_gain * sample;
        }
    }
    let (mut volume, mut pan, mut expression) = (100, 64, 127);
    let mut events = events.iter().filter(|e| e.message == 0xb0).peekable();
    let mut gains = [0.70, 0.70];
    let mut target = gains;
    for (frame, channels) in out.iter_mut().enumerate() {
        if frame % 8192 == 0 {
            control.check()?;
        }
        while events.peek().is_some_and(|e| e.sample <= frame) {
            let e = events.next().unwrap();
            match e.data1 {
                7 => volume = e.data2,
                10 => pan = e.data2,
                11 => expression = e.data2,
                _ => continue,
            }
            let angle = (f32::from(pan) / 127.) * std::f32::consts::FRAC_PI_2;
            let level = f32::from(volume) / 100. * f32::from(expression) / 127.;
            target = [level * angle.cos(), level * angle.sin()];
        }
        for channel in 0..2 {
            gains[channel] += (target[channel] - gains[channel]) * 0.018;
        }
        let mono = channels[0];
        *channels = [gains[0] * mono, gains[1] * mono];
    }
    Ok(out)
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
        .context("Track is not an instrument")?;
    if let Some(kit) = drum_kit::for_instrument(instrument) {
        return render_builtin(kit, p, t, len, control, base, span);
    }
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
    fn built_in_drums_render_at_tempo_without_soundfont_and_respect_velocity() -> Result<()> {
        let mut track = Track::new("Drum test");
        track.instrument = Some(Instrument {
            bank: drum_kit::BANK,
            program: drum_kit::PROGRAM,
            drums: true,
        });
        track.notes = vec![
            Note::from_value(&json!({"start":1,"duration":0.25,"pitch":36,"velocity":45}))?,
            Note::from_value(&json!({"start":2,"duration":0.25,"pitch":36,"velocity":110}))?,
        ];
        let root = std::env::temp_dir().join(uid("inst_built_in"));
        let store = Store::new(root.clone())?;
        let project = store.create("Original warm drum kit", 120., [4, 4], vec![track])?;
        let ctl = crate::jobs::Control::ephemeral("drums", &project.id, project.revision, "guide");
        let frames = render(&root, &project, &project.tracks[0], 72_000, &ctl, 0., 1.)?;
        let peak = |first, end| {
            frames[first..end]
                .iter()
                .map(|s: &[f32; 2]| s[0].abs())
                .fold(0.0_f32, f32::max)
        };
        assert_eq!(peak(0, 24_000), 0.);
        assert!(peak(24_000, 45_000) > 0.01);
        assert!(peak(48_000, 69_000) > 2. * peak(24_000, 45_000));
        let options = catalog(&root)?;
        assert_eq!(options["installed"], false);
        assert_eq!(
            options["presets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["program"].as_u64().unwrap() as u8)
                .collect::<Vec<_>>(),
            drum_kit::KITS.map(|k| k.program)
        );
        assert_eq!(options["presets"][0]["bank"], drum_kit::BANK);
        fs::remove_dir_all(root)?;
        Ok(())
    }

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
