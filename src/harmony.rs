use crate::model::{Note, Project};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_MARKERS: usize = 1500;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Marker {
    pub beat: f64,
    pub symbol: String,
}

pub(crate) struct Chord {
    pub(crate) root: u8,
    pub(crate) intervals: &'static [u8],
    pub(crate) bass: Option<u8>,
}

fn note_prefix(text: &str) -> Option<(u8, &str)> {
    let mut characters = text.char_indices();
    let (_, note) = characters.next()?;
    let natural: u8 = match note {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = &text[1..];
    if let Some(rest) = rest.strip_prefix('#') {
        Some(((natural + 1) % 12, rest))
    } else if let Some(rest) = rest.strip_prefix('b') {
        Some(((natural + 11) % 12, rest))
    } else {
        Some((natural, rest))
    }
}

pub(crate) fn parse(symbol: &str) -> Result<Option<Chord>> {
    ensure!(
        !symbol.trim().is_empty() && symbol.chars().count() <= 38,
        "和弦标记需为 1–38 字"
    );
    let clean = symbol
        .trim()
        .replace('♯', "#")
        .replace('♭', "b")
        .replace(['Δ', '△'], "maj")
        .replace('ø', "m7b5")
        .replace(['°', 'º'], "dim");
    if matches!(clean.as_str(), "N.C." | "NC" | "N/C") {
        return Ok(None);
    }
    let (head, bass) = match clean.rsplit_once('/') {
        Some((head, part)) if note_prefix(part).is_some_and(|(_, rest)| rest.is_empty()) => {
            (head, note_prefix(part).map(|(note, _)| note))
        }
        _ => (clean.as_str(), None),
    };
    let (root, tokens) = note_prefix(head).context("和弦根音用 A–G，可带 # 或 b")?;
    let quality = if let Some((before, after)) = tokens.split_once('(') {
        let after = after.strip_suffix(')').context("和弦括号没有闭合")?;
        format!("{before}{}", after.replace(',', ""))
    } else {
        tokens.to_owned()
    };
    let intervals: &[u8] = match quality.as_str() {
        "" | "maj" | "M" => &[0, 4, 7],
        "m" | "min" | "-" => &[0, 3, 7],
        "5" => &[0, 7],
        "6" => &[0, 4, 7, 9],
        "m6" => &[0, 3, 7, 9],
        "6/9" => &[0, 4, 7, 9, 14],
        "m6/9" => &[0, 3, 7, 9, 14],
        "sus" | "sus4" => &[0, 5, 7],
        "sus2" => &[0, 2, 7],
        "7sus" | "7sus4" => &[0, 5, 7, 10],
        "9sus" | "9sus4" => &[0, 5, 7, 10, 14],
        "add9" | "add2" => &[0, 4, 7, 14],
        "madd9" => &[0, 3, 7, 14],
        "add11" => &[0, 4, 7, 17],
        "dim" => &[0, 3, 6],
        "dim7" => &[0, 3, 6, 9],
        "aug" | "+" => &[0, 4, 8],
        "7" => &[0, 4, 7, 10],
        "maj7" | "M7" => &[0, 4, 7, 11],
        "m7" | "min7" | "-7" => &[0, 3, 7, 10],
        "mMaj7" | "mmaj7" => &[0, 3, 7, 11],
        "m7b5" | "halfdim" => &[0, 3, 6, 10],
        "7b5" => &[0, 4, 6, 10],
        "7#5" | "aug7" => &[0, 4, 8, 10],
        "7b9" => &[0, 4, 7, 10, 13],
        "7#9" => &[0, 4, 7, 10, 15],
        "7#11" => &[0, 4, 7, 10, 18],
        "7b13" => &[0, 4, 7, 10, 20],
        "7b9#11" => &[0, 4, 7, 10, 13, 18],
        "9" => &[0, 4, 7, 10, 14],
        "9#11" => &[0, 4, 7, 10, 14, 18],
        "m9" => &[0, 3, 7, 10, 14],
        "maj9" | "M9" => &[0, 4, 7, 11, 14],
        "11" => &[0, 4, 7, 10, 14, 17],
        "m11" => &[0, 3, 7, 10, 14, 17],
        "13" => &[0, 4, 7, 10, 14, 17, 21],
        "maj13" => &[0, 4, 7, 11, 14, 17, 21],
        "m13" => &[0, 3, 7, 10, 14, 17, 21],
        _ => anyhow::bail!("暂不支持和弦符号 {symbol}"),
    };
    Ok(Some(Chord {
        root,
        intervals,
        bass,
    }))
}

pub(crate) fn validate(markers: &[Marker]) -> Result<()> {
    ensure!(markers.len() <= MAX_MARKERS, "每个工程最多 1500 个和弦标记");
    ensure!(
        markers
            .iter()
            .all(|marker| (0.0..=50_000.).contains(&marker.beat))
            && markers.windows(2).all(|pair| pair[0].beat < pair[1].beat),
        "和弦标记的拍位必须有效且递增"
    );
    for marker in markers {
        parse(&marker.symbol).with_context(|| format!("第 {} 拍的和弦无效", marker.beat))?;
    }
    Ok(())
}

pub(crate) fn range(value: &Value) -> Result<[f64; 2]> {
    let [start, end] = serde_json::from_value(value.clone()).context("提供 [起拍, 止拍]")?;
    ensure!(
        start >= 0. && end > start && end <= 50_000.,
        "无效的和弦拍范围"
    );
    Ok([start, end])
}

pub(crate) fn replace_range(
    existing: &[Marker],
    replacement: Vec<Marker>,
    [start, end]: [f64; 2],
) -> Result<Vec<Marker>> {
    validate(existing)?;
    validate(&replacement)?;
    ensure!(
        replacement.first().is_some_and(|point| point.beat == start)
            && replacement.last().is_some_and(|point| point.beat < end),
        "选区和弦从起拍开始，所有标记须位于选区内；留空段用 N.C."
    );
    let first = existing.partition_point(|point| point.beat < start);
    let after = existing.partition_point(|point| point.beat <= end);
    let current = after.checked_sub(1).and_then(|index| existing.get(index));
    let carries = current.map_or("N.C.", |marker| marker.symbol.as_str());
    let mut points = existing[..first].to_vec();
    points.extend(replacement);
    if let Some(current) = current.filter(|point| point.beat == end) {
        points.push(current.clone());
    } else if points.last().is_some_and(|point| point.symbol != carries) {
        points.push(Marker {
            beat: end,
            symbol: carries.to_owned(),
        });
    }
    points.extend_from_slice(&existing[after..]);
    validate(&points)?;
    Ok(points)
}

fn segments(
    markers: &[Marker],
    [start, end]: [f64; 2],
) -> impl Iterator<Item = (&Marker, [f64; 2])> {
    let first = markers
        .partition_point(|point| point.beat <= start)
        .saturating_sub(1);
    markers
        .iter()
        .enumerate()
        .skip(first)
        .take_while(move |(_, point)| point.beat < end)
        .filter_map(move |(index, point)| {
            let a = start.max(point.beat);
            let b = markers
                .get(index + 1)
                .map_or(end, |next| next.beat.min(end));
            (a < b).then_some((point, [a, b]))
        })
}

pub(crate) fn query(p: &Project, window: Option<[f64; 2]>) -> Value {
    let tempo = p.timeline();
    let last = p.harmony.last().map_or(0., |point| point.beat);
    let bounds = window.unwrap_or_else(|| {
        [
            0.,
            crate::music_review::end_beat(p).max(last + 1.).min(50_000.),
        ]
    });
    let entries: Vec<_> = segments(&p.harmony, bounds)
        .filter_map(|(point, region)| {
            let chord = parse(&point.symbol).ok()?;
            let pitch_classes = chord.as_ref().map(|chord| {
                chord
                    .intervals
                    .iter()
                    .map(|interval| (chord.root + interval) % 12)
                    .collect::<Vec<_>>()
            });
            Some(
                json!({"symbol":point.symbol,"marker_beat":point.beat,"beat_range":region,
            "start_position":tempo.position(region[0]),"root_pc":chord.as_ref().map(|c|c.root),
            "specified_bass_pc":chord.as_ref().and_then(|c|c.bass),"pitch_classes":pitch_classes}),
            )
        })
        .collect();
    let markers = p
        .harmony
        .iter()
        .filter(|point| window.is_none_or(|[a, b]| point.beat >= a && point.beat < b))
        .collect::<Vec<_>>();
    json!({"project_id":p.id,"revision":p.revision,"beat_range":bounds,"markers":markers,
        "chords":entries,"pitch_class_reference":"C=0; annotations reflect intended harmony",
        "audio_is_unchanged":true})
}

pub(crate) fn held_voices(
    p: &Project,
    clock: &crate::timeline::Timeline,
    notes: &[&Note],
    bounds: [f64; 2],
) -> Vec<Value> {
    let mut examples = Vec::new();
    if p.harmony.is_empty() {
        return examples;
    }
    for note in notes {
        let window = [
            note.start.max(bounds[0]),
            (note.start + note.duration).min(bounds[1]),
        ];
        for (point, range) in segments(&p.harmony, window) {
            if examples.len() >= 6 {
                return examples;
            }
            if clock.seconds(range[1]) - clock.seconds(range[0]) < 0.23 {
                continue;
            }
            if let Ok(Some(chord)) = parse(&point.symbol) {
                let pc = note.pitch % 12;
                if chord.bass != Some(pc)
                    && !chord
                        .intervals
                        .iter()
                        .any(|delta| (chord.root + delta) % 12 == pc)
                {
                    examples.push(
                        json!({"note_id":note.id,"pitch":note.pitch,"beat_range":range,
                        "symbol":point.symbol,"relation":"unlisted_tone"}),
                    );
                }
            }
        }
    }
    examples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_bass_and_extensions_parse_and_range_restores_carried_chord() -> Result<()> {
        let chord = parse("G7(♭9,♯11)/D")?.unwrap();
        assert_eq!(chord.root, 7);
        assert_eq!(chord.bass, Some(2));
        assert!(chord.intervals.contains(&13) && chord.intervals.contains(&18));
        assert_eq!(parse("C6/9/E")?.unwrap().bass, Some(4));
        assert_eq!(parse("B♭ø/Db")?.unwrap().intervals, &[0, 3, 6, 10]);
        assert!(parse("Hmaj7").is_err());
        let marker = |beat, symbol: &str| Marker {
            beat,
            symbol: symbol.to_owned(),
        };
        let original = vec![marker(0., "C"), marker(2., "Dm7"), marker(7., "G7")];
        let replacement = replace_range(&original, vec![marker(1., "Am7")], [1., 4.])?;
        assert_eq!(
            replacement,
            vec![
                marker(0., "C"),
                marker(1., "Am7"),
                marker(4., "Dm7"),
                marker(7., "G7")
            ]
        );
        let cleared = replace_range(&original, vec![marker(0., "N.C.")], [0., 2.])?;
        assert_eq!(cleared[1], original[1]);
        Ok(())
    }
}
