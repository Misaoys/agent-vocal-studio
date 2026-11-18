use crate::model::*;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
pub fn library() -> Value {
    serde_json::from_str(include_str!("../knowledge/music.json"))
        .expect("Bundled knowledge JSON must be valid")
}
pub fn knowledge(topic: Option<&str>, style: Option<&str>) -> Value {
    let lib = library();
    let topics = lib["topics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| {
            topic.is_none_or(|q| {
                t["id"].as_str() == Some(q) || t["name"].as_str().unwrap_or("").contains(q)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let styles = lib["styles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| {
            style.is_none_or(|q| {
                t["id"].as_str() == Some(q)
                    || t["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q.to_lowercase())
            })
        })
        .map(|s| {
            if style.is_some() {
                s.clone()
            } else {
                json!({"id":s["id"],"name":s["name"]})
            }
        })
        .collect::<Vec<_>>();
    json!({"scope":lib["scope"],"workflow":lib["workflow"],"topics":topics,"styles":styles,"sources":lib["sources"],"listening_required":lib["listening_required"]})
}
pub fn validate_plan(plan: &Value) -> Result<()> {
    if plan.is_null() {
        return Ok(());
    };
    ensure!(plan.is_object(), "Composition plan must be an object");
    let purpose = plan["purpose"].as_str().unwrap_or("song");
    ensure!(
        ["song", "technical_test", "existing_material"].contains(&purpose),
        "Invalid composition purpose"
    );
    ensure!(
        plan["brief"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 4000),
        "A concrete brief is required"
    );
    if purpose == "song" {
        for k in ["style", "key", "motif", "emotional_arc", "lyric_language"] {
            ensure!(
                plan[k]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 4000),
                "Composition plan needs {k}"
            );
        }
        let sections = plan["sections"]
            .as_array()
            .context("Composition plan needs sections")?;
        ensure!(
            !sections.is_empty() && sections.len() <= 40,
            "Need 1..40 sections"
        );
        let mut end = 0.;
        for s in sections {
            ensure!(
                s["name"].as_str().is_some_and(|s| !s.is_empty()),
                "Section needs a name"
            );
            let a = s["start"]
                .as_f64()
                .context("Section start is in quarter-note beats")?;
            let b = s["end"]
                .as_f64()
                .context("Section end is in quarter-note beats")?;
            ensure!(
                a.is_finite() && b.is_finite() && a >= end && b > a && b <= 50000.,
                "Section ranges must be increasing and non-overlapping"
            );
            ensure!(
                s["role"].as_str().is_some_and(|s| !s.is_empty()),
                "Each section needs a musical/expressive role"
            );
            end = b;
        }
        ensure!(
            plan["arrangement"]
                .as_array()
                .is_some_and(|a| !a.is_empty() && a.len() <= 40),
            "Assign arrangement roles"
        );
    }
    ensure!(
        serde_json::to_vec(plan)?.len() <= 64000,
        "Composition plan too large"
    );
    Ok(())
}
pub fn review(p: &Project) -> Value {
    let mut tracks = vec![];
    let mut findings = vec![];
    let clock = p.timeline();
    for t in &p.tracks {
        if t.notes.is_empty() {
            continue;
        };
        if t.instrument.is_some() {
            tracks.push(json!({"track_id":t.id,"name":t.name,"notes":t.notes.len(),"kind":"instrument","pitch_range":[t.notes.iter().map(|n|n.pitch).min(),t.notes.iter().map(|n|n.pitch).max()],"controllers":t.controllers.len(),"note":"乐器声部不使用人声换气与歌词时值指标"}));
            continue;
        }
        let ns = &t.notes;
        let n = ns.len();
        let low = ns.iter().map(|n| n.pitch).min().unwrap();
        let high = ns.iter().map(|n| n.pitch).max().unwrap();
        let mut duration_counts = HashMap::new();
        let mut bars: HashMap<u64, Vec<&Note>> = HashMap::new();
        let mut intervals = HashMap::new();
        let mut rests = vec![];
        let mut run_start = ns[0].start;
        let mut longest = 0f64;
        for (i, note) in ns.iter().enumerate() {
            *duration_counts
                .entry((note.duration * 480.).round() as u64)
                .or_insert(0usize) += 1;
            bars.entry(clock.bar_start(note.start).0)
                .or_default()
                .push(note);
            if i > 0 {
                let prev = &ns[i - 1];
                let gap = clock.seconds(note.start) - clock.seconds(prev.start + prev.duration);
                rests.push(gap);
                *intervals
                    .entry(note.pitch as i16 - prev.pitch as i16)
                    .or_insert(0usize) += 1;
                if gap >= 0.3 {
                    longest = longest
                        .max(clock.seconds(prev.start + prev.duration) - clock.seconds(run_start));
                    run_start = note.start;
                }
            }
        }
        longest = longest.max(
            clock.seconds(ns.last().unwrap().start + ns.last().unwrap().duration)
                - clock.seconds(run_start),
        );
        let same_duration = *duration_counts.values().max().unwrap() as f64 / n as f64;
        let vel_mean = ns.iter().map(|n| n.velocity).sum::<f64>() / n as f64;
        let vel_std = (ns
            .iter()
            .map(|n| (n.velocity - vel_mean).powi(2))
            .sum::<f64>()
            / n as f64)
            .sqrt();
        let mut patterns = HashMap::new();
        for notes in bars.values() {
            let first = notes[0];
            let signature = notes
                .iter()
                .map(|n| {
                    format!(
                        "{:.3}:{:.3}:{}",
                        (n.start - clock.bar_start(n.start).1),
                        n.duration,
                        n.pitch as i16 - first.pitch as i16
                    )
                })
                .collect::<Vec<_>>()
                .join("|");
            *patterns.entry(signature).or_insert(0usize) += 1;
        }
        let repeated = patterns.values().filter(|c| **c > 1).sum::<usize>();
        let mut sections = vec![];
        if let Some(ss) = p
            .composition
            .as_ref()
            .and_then(|c| c["sections"].as_array())
        {
            for s in ss {
                let a = s["start"].as_f64().unwrap_or(0.);
                let b = s["end"].as_f64().unwrap_or(a);
                let slice = ns
                    .iter()
                    .filter(|n| n.start >= a && n.start < b)
                    .collect::<Vec<_>>();
                let count = slice.len();
                sections.push(json!({"name":s["name"],"notes":count,"notes_per_beat":count as f64/(b-a).max(0.001),"mean_pitch":if count>0{Some(slice.iter().map(|n|n.pitch as f64).sum::<f64>()/count as f64)}else{None},"written_role":s["role"]}));
            }
        }
        if n >= 16 && same_duration > 0.85 {
            findings.push(json!({"track_id":t.id,"topic":"prosody","observation":"超过 85% 音符使用同一时值。","review":"复核是否为有意的节奏语法；若像念词，先改落字、长短音与乐句，不靠随机偏移。"}));
        }
        if longest > 12. {
            findings.push(json!({"track_id":t.id,"topic":"prosody","observation":format!("最长约 {longest:.1} 秒没有达到 300 ms 的谱面间隙。"),"review":"这只是谱面提示；长音、音源尾音与实际换气须试听，不能机械按阈值切碎乐句。"}));
        }
        if bars.len() >= 6 && repeated as f64 / bars.len() as f64 > 0.75 {
            findings.push(json!({"track_id":t.id,"topic":"melody","observation":"多数非空小节重复相同的节奏与相对音程轮廓。","review":"循环风格可能有意如此；检查配器、和声、音色与叙事是否提供发展，不直接判为单调。"}));
        }
        tracks.push(json!({"track_id":t.id,"name":t.name,"notes":n,"pitch_range":[low,high],"most_common_duration_fraction":same_duration,"distinct_durations":duration_counts.len(),"distinct_intervals":intervals.len(),"velocity_stddev":vel_std,"longest_span_without_300ms_gap_seconds":longest,"nonempty_bars":bars.len(),"repeated_bar_fraction":repeated as f64/bars.len().max(1) as f64,"breath_gaps_at_least_300ms":rests.iter().filter(|r|**r>=0.3).count(),"sections":sections}));
    }
    json!({"project_id":p.id,"revision":p.revision,"has_design":p.composition.is_some(),"tracks":tracks,"findings":findings,"interpretation_not_verdict":true,"musical_quality_approved":false,"requires_listening":["旋律记忆点","歌词自然度与叙事成熟度","语气与辨词","主唱和伴奏配合","风格语法与段落推进"],"next_step":"按具体发现选择知识主题；逐段试听并记录问题，不以统计合格宣布好听。"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn style_library_and_design_are_structured() {
        let k = knowledge(Some("prosody"), Some("jpop"));
        assert_eq!(k["topics"].as_array().unwrap().len(), 1);
        assert_eq!(k["styles"][0]["id"], "jpop");
        assert!(validate_plan(&json!({"purpose":"song","brief":"generic"})).is_err());
        assert!(validate_plan(
            &json!({"purpose":"technical_test","brief":"8-note synthesis and editor verification"})
        )
        .is_ok());
    }
}
