use crate::{
    harmony::Chord,
    model::{Note, Project},
    selection::Selection,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Part {
    Upper,
    Bass,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    start: f64,
    duration: f64,
    velocity: f64,
    part: Part,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    events: Vec<Event>,
    upper_range: Option<[u8; 2]>,
    bass_range: Option<[u8; 2]>,
    voices: Option<usize>,
    melody_track_id: Option<String>,
    #[serde(default)]
    allow_carry: bool,
}

pub(crate) fn schema() -> Value {
    let register = json!({"type":"array","minItems":2,"maxItems":2,
        "items":{"type":"integer","minimum":0,"maximum":127}});
    json!({"type":"object","additionalProperties":false,"required":["events"],"properties":{
        "events":{"type":"array","minItems":1,"maxItems":96,"items":{
            "type":"object","additionalProperties":false,"required":["start","duration","velocity","part"],"properties":{
                "start":{"type":"number","minimum":0},"duration":{"type":"number","minimum":0.002084,"maximum":64},
                "velocity":{"type":"number","minimum":1,"maximum":127},"part":{"type":"string","enum":["upper","bass"]}}}},
        "voices":{"type":"integer","minimum":2,"maximum":4},"upper_range":register,"bass_range":register,
        "melody_track_id":{"type":"string"},"allow_carry":{"type":"boolean"}
    }})
}

impl Request {
    pub(crate) fn read(value: &Value) -> Result<Self> {
        let request: Self = serde_json::from_value(value.clone())?;
        ensure!(
            (1..=96).contains(&request.events.len()),
            "Provide 1–96 explicit accompaniment events"
        );
        let mut ends = [0f64; 2];
        for event in &request.events {
            let end = event.start + event.duration;
            ensure!(
                (0.0..50_000.).contains(&event.start)
                    && (1. / 480. ..=64.).contains(&event.duration)
                    && end <= 50_000.
                    && (1.0..=127.).contains(&event.velocity),
                "Invalid voicing event timing or velocity"
            );
            let lane = usize::from(event.part == Part::Bass);
            ensure!(
                event.start + 1e-7 >= ends[lane],
                "Sort events and avoid overlap within either part"
            );
            ends[lane] = end;
        }
        Ok(request)
    }
    pub(crate) fn range(&self) -> [f64; 2] {
        self.events.iter().fold([f64::MAX, 0.], |[a, b], event| {
            [a.min(event.start), b.max(event.start + event.duration)]
        })
    }
}

struct Attack<'a> {
    event: &'a Event,
    symbol: &'a str,
    chord: Chord,
}

#[derive(Clone)]
struct Candidate {
    notes: Vec<u8>,
    cost: f64,
}

#[derive(Clone, Serialize)]
struct PlayedNote {
    start: f64,
    duration: f64,
    velocity: f64,
    pitch: u8,
}

fn register(bounds: [u8; 2]) -> Result<[u8; 2]> {
    ensure!(
        bounds[0] <= bounds[1] && bounds[1] <= 127 && bounds[1] - bounds[0] <= 36,
        "MIDI registers must be increasing and no wider than 36 semitones"
    );
    Ok(bounds)
}

fn pitches_in(range: [u8; 2]) -> impl Iterator<Item = u8> {
    range[0]..=range[1]
}

fn required_tones(chord: &Chord, with_bass: Option<u8>) -> Vec<(u8, f64)> {
    chord
        .intervals
        .iter()
        .copied()
        .map(|interval| {
            let pitch = (chord.root + interval) % 12;
            let weight = if Some(pitch) == with_bass {
                0.
            } else if matches!(interval, 3 | 4 | 10 | 11)
                || interval == 5 && !chord.intervals.iter().any(|x| matches!(x, 3 | 4))
            {
                11.
            } else if interval == 0 {
                6.
            } else if interval == 7 {
                2.
            } else {
                8.
            };
            (pitch, weight)
        })
        .collect()
}

fn upper_candidates(
    attack: &Attack<'_>,
    range: [u8; 2],
    voices: usize,
    width: f64,
    bass: Option<u8>,
    melody: &[(u8, f64)],
) -> Vec<Candidate> {
    let required = required_tones(&attack.chord, bass.map(|bass| bass % 12));
    let values: Vec<_> = pitches_in(range)
        .filter(|p| required.iter().any(|(pc, _)| p % 12 == *pc))
        .collect();
    let center = (f64::from(range[0]) + f64::from(range[1])) / 2.;
    let mut out = Vec::new();
    combinations(
        &values,
        voices,
        &mut Vec::with_capacity(voices),
        &mut |notes| {
            let span = f64::from(notes[notes.len() - 1] - notes[0]);
            let missing = required
                .iter()
                .filter(|(pc, _)| !notes.iter().any(|note| note % 12 == *pc))
                .map(|(_, weight)| weight)
                .sum::<f64>();
            let center_penalty = notes
                .iter()
                .map(|&pitch| (f64::from(pitch) - center).abs())
                .sum::<f64>()
                * 0.11;
            let low_clusters = notes
                .windows(2)
                .filter(|pair| pair[0] < 53 && pair[1] - pair[0] < 3)
                .count() as f64
                * 7.;
            let collisions = notes
                .iter()
                .map(|note| {
                    melody
                        .iter()
                        .map(|(pitch, overlap)| {
                            overlap
                                * match pitch.abs_diff(*note) {
                                    0 => 3.5,
                                    1 => 4.5,
                                    2 => 1.5,
                                    _ => 0.,
                                }
                        })
                        .sum::<f64>()
                })
                .sum::<f64>();
            out.push(Candidate {
                notes: notes.to_vec(),
                cost: missing
                    + center_penalty
                    + low_clusters
                    + collisions
                    + (span - width).abs() * 0.55,
            });
        },
    );
    let order = |a: &Candidate, b: &Candidate| {
        a.cost
            .total_cmp(&b.cost)
            .then_with(|| a.notes.cmp(&b.notes))
    };
    if out.len() > 64 {
        out.select_nth_unstable_by(64, order);
        out.truncate(64);
    }
    out.sort_unstable_by(order);
    out
}

fn combinations(values: &[u8], count: usize, stack: &mut Vec<u8>, each: &mut impl FnMut(&[u8])) {
    if count == 0 {
        each(stack);
        return;
    }
    if values.len() < count {
        return;
    }
    for index in 0..=values.len() - count {
        let pitch = values[index];
        if stack.first().is_some_and(|first| pitch - *first > 28) {
            break;
        }
        stack.push(pitch);
        combinations(&values[index + 1..], count - 1, stack, each);
        stack.pop();
    }
}

fn movement(a: &[u8], b: &[u8]) -> u32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| u32::from(a.abs_diff(*b)))
        .sum()
}

fn sequence(choices: &[Vec<Candidate>]) -> Vec<Vec<u8>> {
    if choices.is_empty() {
        return vec![];
    }
    let mut totals: Vec<_> = choices[0].iter().map(|c| c.cost).collect();
    let mut previous = Vec::new();
    for pair in choices.windows(2) {
        let mut costs = Vec::with_capacity(pair[1].len());
        let mut indices = Vec::with_capacity(pair[1].len());
        for next in &pair[1] {
            let (index, cost) = pair[0]
                .iter()
                .enumerate()
                .map(|(i, old)| {
                    (
                        i,
                        totals[i] + f64::from(movement(&old.notes, &next.notes)) * 0.6,
                    )
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            costs.push(cost + next.cost);
            indices.push(index);
        }
        totals = costs;
        previous.push(indices);
    }
    let mut index = totals
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    let mut chosen = Vec::with_capacity(choices.len());
    for i in (0..choices.len()).rev() {
        chosen.push(choices[i][index].notes.clone());
        if i > 0 {
            index = previous[i - 1][index];
        }
    }
    chosen.reverse();
    chosen
}

fn notes(atoms: &[Attack<'_>], sequence: &[Vec<u8>]) -> Vec<PlayedNote> {
    atoms
        .iter()
        .zip(sequence)
        .flat_map(|(attack, voicing)| {
            voicing.iter().map(|&pitch| PlayedNote {
                start: attack.event.start,
                duration: attack.event.duration,
                velocity: attack.event.velocity,
                pitch,
            })
        })
        .collect()
}

pub(crate) fn preview(
    project: &Project,
    request: &Request,
    selection: Option<&Selection>,
) -> Result<Value> {
    ensure!(
        project.timeline().seconds(request.range()[1]) <= 900.,
        "Voicings must fit within the 15-minute project"
    );
    let upper_range = register(request.upper_range.unwrap_or([48, 76]))?;
    let bass_range = register(request.bass_range.unwrap_or([28, 47]))?;
    let voices = request.voices.unwrap_or(3);
    ensure!(
        (2..=4).contains(&voices),
        "Upper voicings support 2–4 voices"
    );
    let (mut upper, mut bass) = (Vec::new(), Vec::new());
    for event in &request.events {
        let index = project
            .harmony
            .partition_point(|point| point.beat <= event.start);
        let marker = index
            .checked_sub(1)
            .and_then(|i| project.harmony.get(i))
            .with_context(|| format!("No chord at beat {}", event.start))?;
        let chord = crate::harmony::parse(&marker.symbol)?
            .with_context(|| format!("No chord at beat {}", event.start))?;
        if !request.allow_carry {
            if let Some(next) = project.harmony[index..]
                .iter()
                .take_while(|next| next.beat + 1e-7 < event.start + event.duration)
                .find(|next| next.symbol != marker.symbol)
            {
                anyhow::bail!(
                    "Event at beat {} crosses a chord change at {}; split or allow_carry",
                    event.start,
                    next.beat
                );
            }
        }
        let item = Attack {
            event,
            symbol: &marker.symbol,
            chord,
        };
        if event.part == Part::Bass {
            bass.push(item)
        } else {
            upper.push(item)
        }
    }
    ensure!(
        upper.len() * voices + bass.len() <= 240,
        "Request at most 240 resulting accompaniment notes"
    );
    ensure!(
        upper.is_empty() || bass.is_empty() || bass_range[1] < upper_range[0],
        "Choose separate upper and bass registers"
    );
    let selected = selection.map(|s| {
        s.notes
            .iter()
            .map(|(_, id)| id.as_str())
            .collect::<HashSet<_>>()
    });
    let melody: Vec<&Note> = match request.melody_track_id.as_deref() {
        Some(id) => {
            let voice = project
                .tracks
                .iter()
                .find(|track| track.id == id && !track.is_audio() && track.instrument.is_none())
                .context("Choose a vocal melody track")?;
            let notes: Vec<_> = voice
                .notes
                .iter()
                .filter(|note| {
                    selected
                        .as_ref()
                        .is_none_or(|ids| ids.contains(note.id.as_str()))
                })
                .collect();
            ensure!(
                selection.is_none() || !notes.is_empty(),
                "The melody is outside the captured selection"
            );
            notes
        }
        None => vec![],
    };
    let melody_during = |event: &Event| {
        let end = event.start + event.duration;
        let first = melody.partition_point(|note| note.start + note.duration <= event.start);
        melody[first..]
            .iter()
            .take_while(|note| note.start < end)
            .map(|note| {
                (
                    note.pitch,
                    ((note.start + note.duration).min(end) - note.start.max(event.start))
                        / event.duration,
                )
            })
            .collect::<Vec<_>>()
    };
    let bass_options = bass
        .iter()
        .map(|attack| {
            let pc = attack.chord.bass.unwrap_or(attack.chord.root);
            let center = (f64::from(bass_range[0]) + f64::from(bass_range[1])) / 2.;
            let choices = pitches_in(bass_range)
                .filter(|pitch| pitch % 12 == pc)
                .map(|pitch| Candidate {
                    notes: vec![pitch],
                    cost: (f64::from(pitch) - center).abs() * 0.3,
                })
                .collect::<Vec<_>>();
            ensure!(
                !choices.is_empty(),
                "The specified bass for {} is outside the requested register",
                attack.symbol
            );
            Ok(choices)
        })
        .collect::<Result<Vec<_>>>()?;
    let bass_path = sequence(&bass_options);
    let bass_notes = notes(&bass, &bass_path);
    let width = |spread: bool| {
        if spread {
            (voices as f64 - 1.) * 5.5
        } else {
            (voices as f64 - 1.) * 3.4
        }
    };
    let build = |spread| {
        let options = upper
            .iter()
            .map(|attack| {
                let start = attack.event.start;
                let underlying = bass_notes
                    .iter()
                    .find(|note| note.start <= start && start < note.start + note.duration)
                    .map(|note| note.pitch);
                let choices = upper_candidates(
                    attack,
                    upper_range,
                    voices,
                    width(spread),
                    underlying,
                    &melody_during(attack.event),
                );
                ensure!(
                    !choices.is_empty(),
                    "Register does not fit {} at beat {}",
                    attack.symbol,
                    start
                );
                Ok(choices)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok::<_, anyhow::Error>(sequence(&options))
    };
    let compact = build(false)?;
    let spread = build(true)?;
    let stats = |path: &[Vec<u8>]| {
        json!({"total_voice_motion_semitones":path.windows(2).map(|p|movement(&p[0],&p[1])).sum::<u32>(),
        "held_common_pitches":path.windows(2).map(|p|p[0].iter().filter(|pitch|p[1].contains(pitch)).count()).sum::<usize>()})
    };
    Ok(
        json!({"project_id":project.id,"revision":project.revision,"beat_range":request.range(),
        "bass_notes":bass_notes,"distinct_upper_voicings":compact!=spread,
        "alternatives":[{"spacing":"compact","upper_notes":notes(&upper,&compact),"movement":stats(&compact)},
            {"spacing":"spread","upper_notes":notes(&upper,&spread),"movement":stats(&spread)}],
        "draft_only":true,"requires_listening":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_bass_and_pickups_survive_pitch_planning() -> Result<()> {
        let p: Project = serde_json::from_value(json!({
            "format":"agent-vocal/12","id":"p_test","revision":1,"title":"engineering fixture",
            "bpm":120,"meter":[4,4],"tracks":[],
            "harmony":[{"beat":0,"symbol":"Cmaj7/E"},{"beat":2,"symbol":"G13/D"},{"beat":4,"symbol":"N.C."}]
        }))?;
        let input = json!({"voices":3,"upper_range":[50,77],"bass_range":[29,48],"events":[
            {"part":"bass","start":0,"duration":1,"velocity":65},
            {"part":"upper","start":0.25,"duration":0.5,"velocity":71},
            {"part":"bass","start":2.25,"duration":1.5,"velocity":67},
            {"part":"upper","start":2.75,"duration":1,"velocity":58}
        ]});
        let value = preview(&p, &Request::read(&input)?, None)?;
        assert_eq!(value["bass_notes"][0]["pitch"].as_u64().unwrap() % 12, 4);
        assert_eq!(value["bass_notes"][1]["pitch"].as_u64().unwrap() % 12, 2);
        assert_eq!(value["distinct_upper_voicings"], true);
        for alternative in value["alternatives"].as_array().unwrap() {
            let notes = alternative["upper_notes"].as_array().unwrap();
            assert_eq!(notes.len(), 6);
            assert!(notes[..3].iter().all(|note| note["start"] == 0.25
                && note["duration"] == 0.5
                && note["velocity"] == 71.));
            assert!(notes[3..]
                .iter()
                .all(|note| note["start"] == 2.75 && note["velocity"] == 58.));
        }
        let mut broken = input;
        broken["events"][3]["duration"] = json!(2.0);
        assert!(preview(&p, &Request::read(&broken)?, None)
            .unwrap_err()
            .to_string()
            .contains("crosses a chord change"));
        broken["allow_carry"] = json!(true);
        assert_eq!(
            preview(&p, &Request::read(&broken)?, None)?["alternatives"][0]["upper_notes"][3]
                ["duration"],
            2.
        );
        Ok(())
    }
}
