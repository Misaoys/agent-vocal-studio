use crate::{model::Note, timeline::Timeline};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;

const MAX_TEXT: usize = 1_200;
const MAX_READING: usize = 4_800;
pub(crate) const TOUCH_BEATS: f64 = 1e-7;
pub(crate) const CONTINUATION_MARK: &str = "〜";
const SMALL: &str = "ァィゥェォャュョヮ";
const COMBINATIONS: &str = "
    イェ ウィ ウェ ウォ
    キェ キャ キュ キョ ギェ ギャ ギュ ギョ クァ クィ クェ クォ クヮ グァ グィ グェ グォ グヮ
    シェ シャ シュ ショ ジェ ジャ ジュ ジョ スィ ズィ
    チェ チャ チュ チョ ツァ ツィ ツェ ツォ ティ テャ テュ テョ ディ デャ デュ デョ トゥ ドゥ
    ニェ ニャ ニュ ニョ ヒェ ヒャ ヒュ ヒョ ビェ ビャ ビュ ビョ ピェ ピャ ピュ ピョ
    ファ フィ フェ フォ ヴァ ヴィ ヴェ ヴォ ヴャ ヴュ ヴョ
    ミェ ミャ ミュ ミョ リェ リャ リュ リョ";

fn normalized(text: &str, limit: usize) -> Result<String> {
    ensure!(text.chars().count() <= limit, "每次处理不超过 {limit} 个字");
    let text: String = text.trim().nfkc().collect();
    ensure!(!text.is_empty(), "请输入一句歌词或假名读音");
    Ok(text)
}

fn vowel(kana: char) -> Option<char> {
    [
        ("アァカガサザタダナハバパマャヤラワヮ", 'ア'),
        ("イィキギシジチヂニヒビピミリヰ", 'イ'),
        ("ウゥクグスズツヅヌフブプムュユルヴ", 'ウ'),
        ("エェケゲセゼテデネヘベペメレヱヶ", 'エ'),
        ("オォコゴソゾトドノホボポモョヨロヲ", 'オ'),
        ("ン", 'ン'),
    ]
    .into_iter()
    .find_map(|(row, sound)| row.contains(kana).then_some(sound))
}

pub(crate) fn kana(text: &str) -> Result<Vec<String>> {
    let text = normalized(text, MAX_READING)?;
    let mut chars = text
        .chars()
        .map(|c| match c {
            'ぁ'..='ゖ' => char::from_u32(c as u32 + 0x60).unwrap(),
            _ => c,
        })
        .peekable();
    let mut result = Vec::new();
    let mut previous = None;
    while let Some(c) = chars.next() {
        if c.is_whitespace() && !matches!(c, '\r' | '\n') {
            continue;
        }
        if matches!(
            c,
            '\r' | '\n' | '、' | '。' | '・' | '…' | '「' | '」' | '『' | '』'
        ) || ",.?!;:/()[]\"'".contains(c)
        {
            previous = None;
            continue;
        }
        if c == 'ー' {
            let hold = previous.context("延音符 ー 前需要一个可延长的假名")?;
            result.push(String::from(hold));
            continue;
        }
        ensure!(
            vowel(c).is_some() || c == 'ッ',
            "「{c}」需要写成假名读音，或先提取读音"
        );
        let mut mora = String::from(c);
        let mut last = c;
        if let Some(&small) = chars.peek().filter(|c| SMALL.contains(**c)) {
            mora.push(small);
            ensure!(
                COMBINATIONS.split_whitespace().any(|pair| pair == mora),
                "无法识别「{mora}」，如果是两个读音请用空格分开"
            );
            last = chars.next().unwrap();
        }
        result.push(mora);
        previous = vowel(last);
    }
    ensure!(!result.is_empty(), "没有可分配的假名读音");
    Ok(result)
}

pub(crate) fn reading(args: &Value) -> Result<Value> {
    let text = crate::model::field(args, "text")?;
    let mode = args
        .get("mode")
        .map(|mode| mode.as_str().context("mode 需要字符串"))
        .transpose()?;
    let (moras, source) = match mode {
        None | Some("kana") => (kana(text)?, "kana"),
        Some("japanese") => (
            crate::engine::japanese_reading(&normalized(text, MAX_TEXT)?)?,
            "voicevox",
        ),
        _ => bail!("mode 使用 kana 或 japanese"),
    };
    Ok(json!({"reading":moras.join(" "),"moras":moras,"source":source}))
}

pub(crate) fn sustained_kana(text: &str) -> Result<String> {
    let mut moras = kana(text)?;
    ensure!(
        moras.len() == 1 && !matches!(moras[0].as_str(), "ン" | "ッ"),
        "A vowel continuation needs one kana with a vowel on its first note"
    );
    Ok(moras.pop().unwrap())
}

pub(crate) fn display(note: &Note) -> &str {
    if note.vowel_continuation {
        CONTINUATION_MARK
    } else {
        &note.lyric
    }
}

#[derive(Serialize)]
pub(crate) struct ScoreSyllable<'a> {
    pub lyric: Option<&'a str>,
    pub head_note_id: Option<&'a str>,
    pub note_ids: Vec<&'a str>,
    pub beat_range: [f64; 2],
    pub notated_seconds: f64,
}

impl ScoreSyllable<'_> {
    pub(crate) fn label(&self) -> String {
        let Some(lyric) = self.lyric else {
            return CONTINUATION_MARK.to_owned();
        };
        let mut label: String = lyric.chars().take(8).collect();
        if self.note_ids.len() > 1 {
            label.push_str(CONTINUATION_MARK);
        }
        label
    }
}

/// Present only the syllable material and stable note IDs within the visible score.
pub(crate) fn score_syllables<'a>(
    notes: &[&'a Note],
    bounds: [f64; 2],
    clock: &Timeline,
) -> Vec<ScoreSyllable<'a>> {
    let mut syllables: Vec<ScoreSyllable<'_>> = Vec::new();
    let [low, high] = bounds;
    let mut previous_end: Option<f64> = None;
    for note in notes {
        let last = note.start + note.duration;
        let (start, end) = (note.start.max(low), last.min(high));
        if end <= start {
            continue;
        }
        let continues_in_view = note.vowel_continuation
            && previous_end.is_some_and(|previous| (note.start - previous).abs() <= TOUCH_BEATS);
        if let Some(active) = syllables.last_mut().filter(|_| continues_in_view) {
            active.note_ids.push(note.id.as_str());
            active.beat_range[1] = end;
        } else {
            let head = !note.vowel_continuation && note.start >= low;
            syllables.push(ScoreSyllable {
                lyric: head.then_some(note.lyric.as_str()),
                head_note_id: head.then_some(note.id.as_str()),
                note_ids: vec![note.id.as_str()],
                beat_range: [start, end],
                notated_seconds: 0.,
            });
        }
        previous_end = Some(last);
    }
    for syllable in &mut syllables {
        let [start, end] = syllable.beat_range;
        syllable.notated_seconds = clock.seconds(end) - clock.seconds(start);
    }
    syllables
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_small_kana_and_sustains_the_previous_vowel() -> Result<()> {
        assert_eq!(
            kana("  きゃっ、ｼｭｰ。 コーヒー\nぎょ！ ")?,
            ["キャ", "ッ", "シュ", "ウ", "コ", "オ", "ヒ", "イ", "ギョ"]
        );
        assert_eq!(kana("は を へ　きょ ー")?, ["ハ", "ヲ", "ヘ", "キョ", "オ"]);
        for invalid in ["東京", "ーア", "ッー", "あ、ー", "アャ", "。"] {
            assert!(kana(invalid).is_err(), "{invalid}");
        }
        Ok(())
    }
}
