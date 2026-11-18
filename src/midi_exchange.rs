use crate::exchange::Parsed;
use crate::model::*;
use crate::timeline::{Curve, MeterPoint, TempoPoint};
use anyhow::{bail, ensure, Context, Result};
use midly::{
    num::{u15, u24, u28, u4, u7},
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
const PPQ: u32 = 9600;
const VOCAL: &[u8] = b"AgentVocalStudio:role=vocal";
const INSTRUMENT: &[u8] = b"AgentVocalStudio:role=instrument";
const SECTION_LIMIT: usize = 40;
#[derive(Default, Clone)]
struct Channel {
    bank: u16,
    program: u8,
    program_set: bool,
    notes: Vec<Note>,
    controllers: BTreeMap<(u64, u8), u8>,
}
pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let smf = Smf::parse(bytes)?;
    ensure!(
        smf.header.format != Format::Sequential,
        "Sequential (format 2) MIDI needs a separate-song import"
    );
    let ppq = match smf.header.timing {
        Timing::Metrical(v) => {
            ensure!(v.as_int() > 0, "Invalid MIDI PPQ");
            v.as_int() as f64
        }
        _ => bail!("SMPTE MIDI unsupported"),
    };
    let mut tempos = vec![];
    let mut meters = vec![];
    let mut tracks = vec![];
    let mut warnings = vec![];
    let mut unsupported = 0;
    let mut markers = BTreeMap::new();
    let mut skipped_markers = 0;
    let mut last_tick = 0;
    for source in &smf.tracks {
        let mut tick = 0u64;
        let mut name = "MIDI 音轨".to_string();
        let mut active: HashMap<(u8, u8), (u64, u8, String)> = HashMap::new();
        let mut channels: BTreeMap<u8, Channel> = BTreeMap::new();
        let mut lyric = String::new();
        let mut vocal_marker = false;
        let mut instrument_marker = false;
        for e in source {
            tick += e.delta.as_int() as u64;
            match e.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => tempos.push((tick, t.as_int())),
                TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, _, _)) => {
                    ensure!(d <= 5, "MIDI meter denominator exceeds 32");
                    meters.push((tick, [n as u32, 1u32 << d]));
                }
                TrackEventKind::Meta(MetaMessage::TrackName(s)) => {
                    name = String::from_utf8_lossy(s).into_owned()
                }
                TrackEventKind::Meta(MetaMessage::Text(s)) => {
                    vocal_marker |= s == VOCAL;
                    instrument_marker |= s == INSTRUMENT;
                }
                TrackEventKind::Meta(MetaMessage::Lyric(s)) => {
                    lyric = String::from_utf8_lossy(s).into_owned()
                }
                TrackEventKind::Meta(MetaMessage::Marker(text)) => {
                    let name = String::from_utf8_lossy(&text[..text.len().min(512)])
                        .trim()
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(128)
                        .collect::<String>();
                    if name.trim().is_empty() || markers.contains_key(&tick) {
                        skipped_markers += 1;
                    } else {
                        markers.insert(tick, name);
                        if markers.len() > SECTION_LIMIT {
                            markers.pop_last();
                            skipped_markers += 1;
                        }
                    }
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    let state = channels.entry(ch).or_default();
                    match message{
                    MidiMessage::ProgramChange{program}=>{ensure!((state.notes.is_empty()&&!active.keys().any(|k|k.0==ch))||state.program==program.as_int(),"Mid-track program changes need separate instrument tracks");state.program=program.as_int();state.program_set=true;},
                    MidiMessage::Controller{controller,value}=>match controller.as_int(){
                        0=>{ensure!((state.notes.is_empty()&&!active.keys().any(|k|k.0==ch))||state.bank==value.as_int() as u16,"Mid-track bank changes need separate tracks");state.bank=value.as_int() as u16;},
                        32=>ensure!(value.as_int()==0,"Non-zero MIDI bank LSB is not supported by this sample-bank adapter"),
                        cc @ (1|7|10|11|64|91|93)=>{state.controllers.insert((tick,cc),value.as_int());},
                        _=>unsupported+=1,
                    },
                    MidiMessage::NoteOn{key,vel} if vel.as_int()>0=>{let k=(ch,key.as_int());ensure!(!active.contains_key(&k),"Repeated active MIDI key");active.insert(k,(tick,vel.as_int(),std::mem::take(&mut lyric)));},
                    MidiMessage::NoteOn{key,..}|MidiMessage::NoteOff{key,..}=>{if let Some((start,velocity,l))=active.remove(&(ch,key.as_int())){if tick>start{state.notes.push(Note::from_value(&json!({"start":start as f64/ppq,"duration":(tick-start) as f64/ppq,"pitch":key.as_int(),"velocity":velocity,"lyric":l}))?)}}},
                    _=>unsupported+=1,
                }
                }
                _ => {}
            }
        }
        last_tick = last_tick.max(tick);
        ensure!(active.is_empty(), "MIDI has notes without note-off");
        let nonempty = channels.values().filter(|s| !s.notes.is_empty()).count();
        for (ch, mut state) in channels {
            if state.notes.is_empty() {
                continue;
            };
            state.notes.sort_by(|a, b| a.start.total_cmp(&b.start));
            let mut end = 0.;
            let mut polyphonic = false;
            for n in &state.notes {
                polyphonic |= n.start + 1e-7 < end;
                end = end.max(n.start + n.duration);
            }
            let has_lyrics = state.notes.iter().any(|n| !n.lyric.is_empty());
            let sampled = instrument_marker
                || ch == 9
                || (!vocal_marker && (state.program_set || polyphonic || !has_lyrics));
            let mut t = Track::new(&if nonempty > 1 {
                format!("{} · 通道 {}", name, ch + 1)
            } else {
                name.clone()
            });
            if sampled {
                t.instrument = Some(Instrument {
                    bank: if ch == 9 { 0 } else { state.bank },
                    program: state.program,
                    drums: ch == 9,
                });
                t.controllers = state
                    .controllers
                    .into_iter()
                    .map(|((tick, controller), value)| Controller {
                        beat: tick as f64 / ppq,
                        controller,
                        value,
                    })
                    .collect();
            } else {
                ensure!(
                    !polyphonic,
                    "Vocal MIDI contains polyphony; use separate vocal tracks"
                );
                for n in &mut state.notes {
                    if n.lyric.is_empty() {
                        n.lyric = "あ".into()
                    }
                }
                if !state.controllers.is_empty() {
                    warnings.push("歌声轨的 MIDI CC 未映射为歌声参数；音符与歌词已保留。".into());
                }
            }
            t.notes = state.notes;
            tracks.push(t);
        }
    }
    let mut tempo_values = BTreeMap::new();
    for (tick, value) in tempos {
        ensure!(value > 0, "Invalid MIDI tempo");
        if let Some(old) = tempo_values.insert(tick, value) {
            ensure!(
                old == value,
                "Conflicting MIDI tempo events at the same tick"
            );
        }
    }
    let mut meter_values = BTreeMap::new();
    for (tick, value) in meters {
        if let Some(old) = meter_values.insert(tick, value) {
            ensure!(
                old == value,
                "Conflicting MIDI meter events at the same tick"
            );
        }
    }
    let tempo = tempo_values.remove(&0).unwrap_or(500000);
    let bpm = 60_000_000. / tempo as f64;
    let meter = meter_values.remove(&0).unwrap_or([4, 4]);
    let mut last = tempo;
    let mut tempo_map = vec![];
    for (tick, value) in tempo_values {
        if value != last {
            tempo_map.push(TempoPoint {
                beat: tick as f64 / ppq,
                bpm: 60_000_000. / value as f64,
                curve: Curve::Step,
            });
            last = value;
        }
    }
    let mut last = meter;
    let mut meter_map = vec![];
    for (tick, value) in meter_values {
        if value != last {
            meter_map.push(MeterPoint {
                beat: tick as f64 / ppq,
                meter: value,
            });
            last = value;
        }
    }
    crate::timeline::validate(bpm, meter, &tempo_map, &meter_map)?;
    let clock = crate::timeline::Timeline::new(bpm, meter, &tempo_map, &meter_map);
    let limit = clock.beat(900.).min(50_000.);
    let mut points = markers.into_iter().peekable();
    let mut sections = vec![];
    while let Some((tick, name)) = points.next() {
        let start = tick as f64 / ppq;
        let end = if let Some((next, _)) = points.peek() {
            *next as f64 / ppq
        } else if last_tick > tick {
            last_tick as f64 / ppq
        } else {
            clock.bar_bounds(start)[1]
        }
        .min(limit);
        if end > start && clock.seconds(end) <= 900. {
            sections.push(SectionMarker {
                name,
                start,
                end,
                role: None,
            });
        } else {
            skipped_markers += 1;
        }
    }
    ensure!(
        !tracks.is_empty() || !sections.is_empty(),
        "MIDI contains no notes or usable markers"
    );
    if tracks.is_empty() {
        tracks.push(Track::new("主唱"));
    }
    if !sections.is_empty() {
        warnings.push("已导入 MIDI 标记的名称和起拍。终点采用下一标记或文件末尾，末尾无余量时取下一小节边界。".into());
    }
    if skipped_markers > 0 {
        warnings.push(format!(
            "有 {skipped_markers} 个空白、同拍或超限的 MIDI 标记未导入。"
        ));
    }
    warnings
        .push("已保留音符、复音、歌词、音色程序与已支持的 MIDI CC；原生调声需使用工程包。".into());
    if unsupported > 0 {
        warnings.push(format!(
            "有 {unsupported} 个暂不支持的 MIDI 控制/弯音事件未导入。"
        ));
    }
    warnings.sort();
    warnings.dedup();
    Ok((bpm, meter, tracks, warnings, tempo_map, meter_map, sections))
}
/// SMF has step tempos. Each ramp interval uses its exact integrated mean tempo.
/// A 1/9600-beat tick and adaptive subdivision bound midpoint timing error.
pub fn tempo_events(p: &Project) -> Result<Vec<(u32, u32)>> {
    let clock = p.timeline();
    let mut result = BTreeMap::new();
    result.insert(0, (60_000_000. / p.bpm).round() as u32);
    let mut previous = 0.;
    let mut previous_bpm = p.bpm;
    for point in &p.tempo_map {
        let start = (previous * PPQ as f64).round() as u32;
        let end = (point.beat * PPQ as f64).round() as u32;
        ensure!(
            end > start,
            "Tempo points are closer than one MIDI tick; use avpack for exact timing"
        );
        if point.curve == Curve::Linear {
            let mut pending = vec![(start, end)];
            while let Some((a, b)) = pending.pop() {
                let x = a as f64 / PPQ as f64;
                let y = b as f64 / PPQ as f64;
                let duration = clock.seconds(y) - clock.seconds(x);
                let slope = (point.bpm - previous_bpm) / (point.beat - previous);
                let minimum = clock.tempo(x).min(clock.tempo(y)).max(30.);
                let error_bound = 60. * slope.abs() / minimum.powi(2) * (y - x).powi(2) / 8.;
                if b - a > 1 && error_bound > 0.00004 {
                    let m = a + (b - a) / 2;
                    pending.push((m, b));
                    pending.push((a, m));
                } else {
                    result.insert(a, (duration / (y - x) * 1_000_000.).round() as u32);
                }
                ensure!(
                    result.len() + pending.len() < 20000,
                    "Tempo ramp exceeds MIDI event budget; use avpack"
                );
            }
        }
        result.insert(end, (60_000_000. / point.bpm).round() as u32);
        previous = point.beat;
        previous_bpm = point.bpm;
    }
    Ok(result.into_iter().collect())
}
pub fn export(p: &Project) -> Result<Vec<u8>> {
    let mut conductor: Vec<(u32, u8, TrackEventKind)> = vec![];
    let sections = crate::arrangement::section_markers(p);
    for (tick, micros) in tempo_events(p)? {
        conductor.push((
            tick,
            0,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(micros))),
        ));
    }
    for (beat, meter) in
        std::iter::once((0., p.meter)).chain(p.meter_map.iter().map(|m| (m.beat, m.meter)))
    {
        conductor.push((
            (beat * PPQ as f64).round() as u32,
            1,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                meter[0] as u8,
                meter[1].trailing_zeros() as u8,
                24,
                8,
            )),
        ));
    }
    let mut section_end = 0;
    for section in &sections {
        let (Some(name), Some(beat), Some(end)) = (
            section["name"].as_str(),
            section["start"].as_f64(),
            section["end"].as_f64(),
        ) else {
            continue;
        };
        conductor.push((
            (beat * PPQ as f64).round() as u32,
            2,
            TrackEventKind::Meta(MetaMessage::Marker(name.as_bytes())),
        ));
        section_end = section_end.max((end * PPQ as f64).round() as u32);
    }
    conductor.sort_by_key(|e| (e.0, e.1));
    let mut prev = 0;
    let mut events = vec![];
    for (tick, _, kind) in conductor {
        events.push(TrackEvent {
            delta: u28::new(tick - prev),
            kind,
        });
        prev = tick;
    }
    events.push(TrackEvent {
        delta: u28::new(section_end.saturating_sub(prev)),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    let mut tracks = vec![events];
    let mut melodic = (0u8..16).filter(|c| *c != 9);
    let mut percussion: Option<&Track> = None;
    for t in &p.tracks {
        if t.notes.is_empty() {
            continue;
        };
        let drums = t.instrument.as_ref().is_some_and(|i| i.drums);
        let ch = if drums {
            if let Some(old) = percussion {
                ensure!(serde_json::to_value(&old.instrument)?==serde_json::to_value(&t.instrument)?&&old.controllers.is_empty()&&t.controllers.is_empty(),"Multiple drum MIDI tracks must share a kit and have no conflicting controllers; use avpack for unrestricted track state");
            }
            percussion = Some(t);
            9
        } else {
            melodic.next().context(
                "MIDI 1.0 has only 15 independent melodic channels; export avpack for more tracks",
            )?
        };
        let channel = u4::new(ch);
        let mut events: Vec<(u32, u8, TrackEventKind)> = vec![
            (
                0,
                0,
                TrackEventKind::Meta(MetaMessage::TrackName(t.name.as_bytes())),
            ),
            (
                0,
                0,
                TrackEventKind::Meta(MetaMessage::Text(if t.instrument.is_some() {
                    INSTRUMENT
                } else {
                    VOCAL
                })),
            ),
        ];
        if let Some(i) = &t.instrument {
            ensure!(
                i.bank <= 127 || drums,
                "Melodic bank 128 cannot be represented by this GM MIDI adapter"
            );
            events.push((
                0,
                1,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::Controller {
                        controller: 0.into(),
                        value: u7::new(if drums { 0 } else { i.bank as u8 }),
                    },
                },
            ));
            events.push((
                0,
                2,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::ProgramChange {
                        program: u7::new(i.program),
                    },
                },
            ));
        }
        for c in &t.controllers {
            events.push((
                (c.beat * PPQ as f64).round() as u32,
                3,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::Controller {
                        controller: u7::new(c.controller),
                        value: u7::new(c.value),
                    },
                },
            ));
        }
        for n in &t.notes {
            let a = (n.start * PPQ as f64).round() as u32;
            let b = ((n.start + n.duration) * PPQ as f64).round() as u32;
            if !n.lyric.is_empty() && !n.vowel_continuation {
                events.push((
                    a,
                    4,
                    TrackEventKind::Meta(MetaMessage::Lyric(n.lyric.as_bytes())),
                ));
            }
            events.push((
                a,
                5,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn {
                        key: u7::new(n.pitch),
                        vel: u7::new(n.velocity.round() as u8),
                    },
                },
            ));
            events.push((
                b,
                0,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOff {
                        key: u7::new(n.pitch),
                        vel: 0.into(),
                    },
                },
            ));
        }
        events.sort_by_key(|e| (e.0, e.1));
        let mut last = 0;
        let mut out = vec![];
        for (tick, _, kind) in events {
            out.push(TrackEvent {
                delta: u28::new(tick - last),
                kind,
            });
            last = tick;
        }
        out.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        tracks.push(out);
    }
    let mut out = vec![];
    Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ as u16))),
        tracks,
    }
    .write_std(&mut out)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_section_markers_are_exchanged_on_the_conductor_track() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_midi_sections"));
        let store = Store::new(root.clone())?;
        let score = store.create("Test", 120., [4, 4], vec![Track::new("主唱")])?;
        let score = store.edit(
            &score.id,
            score.revision,
            &json!([
                {"op":"set_project","values":{"section_markers":[
                    {"name":"主歌","start":0.,"end":3.},
                    {"name":"副歌","start":4.5,"end":8.}
                ]}}
            ]),
        )?;
        let midi = export(&score)?;
        let smf = Smf::parse(&midi)?;
        let mut tick = 0;
        let points = smf.tracks[0]
            .iter()
            .filter_map(|event| {
                tick += event.delta.as_int();
                match event.kind {
                    TrackEventKind::Meta(MetaMessage::Marker(name)) => Some((tick, name)),
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            points,
            [
                (0, "主歌".as_bytes()),
                (4 * PPQ + PPQ / 2, "副歌".as_bytes())
            ]
        );
        let (_, _, tracks, _, _, _, sections) = parse(&midi)?;
        assert_eq!(sections[0].end, 4.5);
        assert_eq!(sections[1].end, 8.);
        assert_eq!(tracks.len(), 1);
        assert!(tracks[0].notes.is_empty());
        let plain = Smf {
            header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(480))),
            tracks: vec![vec![
                TrackEvent {
                    delta: 480.into(),
                    kind: TrackEventKind::Meta(MetaMessage::Marker(b"Verse")),
                },
                TrackEvent {
                    delta: 0.into(),
                    kind: TrackEventKind::Meta(MetaMessage::Marker(b"Duplicate")),
                },
                TrackEvent {
                    delta: 1440.into(),
                    kind: TrackEventKind::Meta(MetaMessage::Marker(b"Outro")),
                },
                TrackEvent {
                    delta: 0.into(),
                    kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                },
            ]],
        };
        let mut bytes = vec![];
        plain.write_std(&mut bytes)?;
        let (_, _, _, warnings, _, _, markers) = parse(&bytes)?;
        assert_eq!(markers.len(), 2);
        assert_eq!(
            (&*markers[0].name, markers[0].start, markers[0].end),
            ("Verse", 1., 4.)
        );
        assert_eq!(
            (&*markers[1].name, markers[1].start, markers[1].end),
            ("Outro", 4., 8.)
        );
        assert!(warnings.iter().any(|warning| warning.contains("同拍")));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn conductor_maps_preserve_steps_and_approximate_ramps_with_bounded_error() -> Result<()> {
        let root = std::env::temp_dir().join(uid("avs_conductor"));
        let store = Store::new(root.clone())?;
        let mut t = Track::new("钢琴");
        t.instrument = Some(Instrument::default());
        t.notes.push(Note::from_value(
            &json!({"start":1,"duration":12,"pitch":60,"lyric":""}),
        )?);
        let mut p = store.create("tempo", 120., [4, 4], vec![t])?;
        p.tempo_map = vec![
            TempoPoint {
                beat: 4.,
                bpm: 60.,
                curve: Curve::Step,
            },
            TempoPoint {
                beat: 8.,
                bpm: 180.,
                curve: Curve::Linear,
            },
            TempoPoint {
                beat: 12.,
                bpm: 90.,
                curve: Curve::Step,
            },
        ];
        p.meter_map = vec![
            MeterPoint {
                beat: 8.,
                meter: [6, 8],
            },
            MeterPoint {
                beat: 11.,
                meter: [3, 4],
            },
        ];
        p.validate()?;
        let (bpm, meter, tracks, _, tempos, meters, _) = parse(&export(&p)?)?;
        assert_eq!(tracks[0].notes[0].duration, 12.);
        assert_eq!(meters.len(), 2);
        assert!(tempos.iter().all(|p| p.curve == Curve::Step));
        let original = p.timeline();
        let restored = crate::timeline::Timeline::new(bpm, meter, &tempos, &meters);
        let mut max_error = 0f64;
        for i in 0..20001 {
            let beat = i as f64 * 14. / 20000.;
            max_error = max_error.max((restored.seconds(beat) - original.seconds(beat)).abs());
        }
        assert!(max_error < 0.00015, "MIDI ramp error {max_error}");
        assert_eq!(restored.position(11.)["bar"], 4);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn polyphony_program_drums_and_pedal_roundtrip() -> Result<()> {
        let mut piano = Track::new("钢琴");
        piano.instrument = Some(Instrument::default());
        for pitch in [60, 64, 67] {
            piano.notes.push(Note::from_value(
                &json!({"start":0,"duration":1,"pitch":pitch,"lyric":""}),
            )?);
        }
        piano.controllers = vec![
            Controller {
                beat: 0.,
                controller: 64,
                value: 127,
            },
            Controller {
                beat: 2.,
                controller: 64,
                value: 0,
            },
        ];
        let mut drums = Track::new("鼓组");
        drums.instrument = Some(Instrument {
            drums: true,
            ..Instrument::default()
        });
        drums.notes.push(Note::from_value(
            &json!({"start":0,"pitch":36,"duration":0.2,"lyric":""}),
        )?);
        let root = std::env::temp_dir().join(uid("midi_inst"));
        let store = Store::new(root.clone())?;
        let p = store.create("test", 120., [4, 4], vec![piano, drums])?;
        let (_, _, ts, _, _, _, _) = parse(&export(&p)?)?;
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].notes.len(), 3);
        assert_eq!(ts[0].controllers.len(), 2);
        assert_eq!(ts[0].instrument.as_ref().unwrap().program, 0);
        assert!(ts[1].instrument.as_ref().unwrap().drums);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
