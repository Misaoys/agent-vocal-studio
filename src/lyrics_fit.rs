use crate::{
    lyrics::{kana, TOUCH_BEATS},
    model::{Note, Track},
    timeline::Timeline,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAX_NOTES: usize = 128;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    mora_index: usize,
    note_id: String,
}

#[derive(Serialize)]
pub(crate) struct Group<'a> {
    reading: String,
    note_ids: Vec<&'a str>,
    beat_range: [f64; 2],
    seconds: f64,
    anchored: bool,
    timing_reset: bool,
}

pub(crate) struct Fit<'a> {
    pub values: HashMap<&'a str, Value>,
    pub groups: Vec<Group<'a>>,
}

pub(crate) fn plan<'a>(
    track: &Track,
    notes: &[&'a Note],
    clock: &Timeline,
    reading: &str,
    anchors: Option<&Value>,
) -> Result<Fit<'a>> {
    let count = notes.len();
    ensure!(
        (1..=MAX_NOTES).contains(&count),
        "每次请选择 1–128 个乐句音符"
    );
    let moras = kana(reading)?;
    ensure!(
        moras.len() <= count,
        "{} 个读音 · {count} 个选中音符，请缩短读音或扩大选区",
        moras.len()
    );
    let selected: HashSet<_> = notes.iter().map(|note| note.id.as_str()).collect();
    let mut ordered = track.notes.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.start.total_cmp(&b.start));
    for pair in ordered.windows(2) {
        if pair[1].vowel_continuation {
            ensure!(
                selected.contains(pair[0].id.as_str()) == selected.contains(pair[1].id.as_str()),
                "选区切开了原有延音，请选中该读音的完整音符"
            );
        }
    }
    let positions = ordered
        .iter()
        .enumerate()
        .map(|(i, note)| (note.id.as_str(), i))
        .collect::<HashMap<_, _>>();
    let connected = notes
        .windows(2)
        .map(|pair| {
            positions[pair[0].id.as_str()] + 1 == positions[pair[1].id.as_str()]
                && (pair[1].start - pair[0].start - pair[0].duration).abs() <= TOUCH_BEATS
        })
        .collect::<Vec<_>>();
    let mut by_note = vec![None; count];
    let mut by_mora = vec![None; moras.len()];
    if let Some(anchors) = anchors {
        for anchor in serde_json::from_value::<Vec<Anchor>>(anchors.clone())? {
            let index = anchor
                .mora_index
                .checked_sub(1)
                .filter(|&index| index < moras.len())
                .context("锚定读音序号超出范围")?;
            let note = notes
                .iter()
                .position(|note| note.id == anchor.note_id)
                .context("锚定音符在选区之外")?;
            ensure!(
                by_mora[index].is_none() && by_note[note].is_none(),
                "同一读音或音符只能锚定一次"
            );
            by_mora[index] = Some(note);
            by_note[note] = Some(index);
        }
    }
    let times = notes
        .iter()
        .map(|note| {
            [
                clock.seconds(note.start),
                clock.seconds(note.start + note.duration),
            ]
        })
        .collect::<Vec<_>>();
    let old_readings = notes
        .iter()
        .map(|note| {
            kana(&note.lyric)
                .ok()
                .and_then(|reading| (reading.len() == 1).then(|| reading[0].clone()))
        })
        .collect::<Vec<_>>();
    let mut costs = vec![vec![f64::INFINITY; count + 1]; moras.len() + 1];
    let mut previous = vec![vec![0; count + 1]; moras.len() + 1];
    costs[0][0] = 0.;
    for (index, mora) in moras.iter().enumerate() {
        let can_hold = !matches!(mora.as_str(), "ン" | "ッ");
        let limit = count - (moras.len() - index - 1);
        for start in index..limit {
            let cost = costs[index][start];
            if !cost.is_finite()
                || by_note[start].is_some_and(|value| value != index)
                || by_mora[index].is_some_and(|value| value != start)
            {
                continue;
            }
            // Preserve intentional syllable starts and favor sustaining short connecting notes.
            let mut score =
                cost + if notes[start].vowel_continuation {
                    1.35
                } else {
                    0.
                } + if old_readings[start].as_ref() == Some(mora) {
                    0.
                } else {
                    0.28
                };
            for end in start + 1..=limit {
                if end > start + 1 {
                    let held = end - 1;
                    if !can_hold || !connected[held - 1] || by_note[held].is_some() {
                        break;
                    }
                    if !notes[held].vowel_continuation {
                        let [onset, offset] = times[held];
                        score += 0.7 + (offset - onset).min(0.7) * 2.;
                    }
                }
                let duration = times[end - 1][1] - times[start][0];
                let candidate = score
                    + if can_hold {
                        (0.10 - duration).max(0.) * 3.
                    } else {
                        0.
                    };
                if candidate < costs[index + 1][end] {
                    costs[index + 1][end] = candidate;
                    previous[index + 1][end] = start;
                }
            }
        }
    }
    ensure!(
        costs[moras.len()][count].is_finite(),
        "这些读音无法在选区内延音，请检查休止、锚点和「ン」「ッ」落点"
    );
    let mut groups = Vec::with_capacity(moras.len());
    let mut values = HashMap::new();
    let mut end = count;
    for (index, reading) in moras.into_iter().enumerate().rev() {
        let start = previous[index + 1][end];
        let segment = &notes[start..end];
        let head = segment[0];
        let same_reading = old_readings[start].as_ref() == Some(&reading);
        let timing_reset =
            !same_reading && (head.consonant_ms.is_some() || head.vowel_offset_ms.is_some());
        if head.lyric != reading || head.vowel_continuation {
            let mut change = json!({"lyric":reading,"vowel_continuation":false});
            // Onset parameters can be incompatible with a different phoneme. Show the
            // same reset as the commit, while retaining adjustments for an unchanged reading.
            if head.consonant_ms.is_some() {
                change["consonant_ms"] = json!(head.consonant_ms.filter(|_| same_reading));
            }
            if head.vowel_offset_ms.is_some() {
                change["vowel_offset_ms"] = json!(head.vowel_offset_ms.filter(|_| same_reading));
            }
            values.insert(head.id.as_str(), change);
        }
        for note in &segment[1..] {
            if !note.vowel_continuation {
                values.insert(note.id.as_str(), json!({"vowel_continuation":true}));
            }
        }
        let last = segment[segment.len() - 1];
        let beat_range = [head.start, last.start + last.duration];
        groups.push(Group {
            reading,
            note_ids: segment.iter().map(|note| note.id.as_str()).collect(),
            beat_range,
            seconds: times[end - 1][1] - times[start][0],
            anchored: by_mora[index].is_some(),
            timing_reset,
        });
        end = start;
    }
    groups.reverse();
    Ok(Fit { values, groups })
}
