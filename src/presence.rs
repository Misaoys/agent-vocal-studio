use crate::model::Project;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Default)]
pub struct Journal {
    sequence: u64,
    events: VecDeque<Value>,
}

#[derive(Clone)]
pub(crate) struct Intent {
    id: String,
    reason: String,
    sequence: u64,
}

impl Journal {
    pub fn push(&mut self, kind: &str, mut data: Value) -> Value {
        self.sequence += 1;
        data["sequence"] = json!(self.sequence);
        data["kind"] = json!(kind);
        data["timestamp_ms"] = json!(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64);
        self.events.push_back(data.clone());
        if self.events.len() > 200 {
            self.events.pop_front();
        }
        data
    }
    pub fn since(&self, after: u64) -> Value {
        json!({"sequence":self.sequence,"events":self.events.iter().filter(|e|e["sequence"].as_u64().unwrap_or(0)>after).collect::<Vec<_>>()})
    }

    pub(crate) fn intent(&self, id: &str, project: &str, revision: u64) -> Option<Intent> {
        let event = self.events.iter().rev().find(|event| {
            event["kind"] == "focus"
                && event["intent_id"] == id
                && event["project_id"] == project
                && event["revision"] == revision
        })?;
        Some(Intent {
            id: id.into(),
            reason: event["reason"].as_str()?.into(),
            sequence: event["sequence"].as_u64()?,
        })
    }

    pub(crate) fn push_with_intent(
        &mut self,
        kind: &str,
        mut data: Value,
        intent: Option<&Intent>,
    ) -> Value {
        if let Some(intent) = intent {
            data["intent_id"] = json!(intent.id);
            data["intent_reason"] = json!(intent.reason);
            data["intent_sequence"] = json!(intent.sequence);
        }
        self.push(kind, data)
    }
}

pub fn working_label(name: &str, args: &Value) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "project_read" => ("reading", "读取曲谱与混音参数"),
        "selection_resolve" => ("reading", "核对选区里的音符"),
        "composition_review" => ("reading", "检查乐句、呼吸与段落变化"),
        "section_contrast" => ("reading", "对照两段的入场、律动和主唱留白"),
        "rhythm_interplay" => ("reading", "核对底鼓与贝斯的入音和延音"),
        "arrangement_query" => ("reading", "检查各音轨的入场与留白"),
        "timeline_query" => ("reading", "核对速度与拍号"),
        "audio_clips" => ("reading", "读取音频片段与淡化"),
        "lyrics_reading" => ("reading", "核对日语歌词读音"),
        "selection_edit" if args.get("audition_id").is_some() => {
            if args["dry_run"] == false {
                ("editing", "应用已确认的调声试听草稿")
            } else {
                ("reading", "核对已试听的调声草稿与目标音符")
            }
        }
        "selection_edit" if args["transform"]["kind"] == "pitch_join" => {
            if args["dry_run"] == false {
                ("editing", "应用两音符的滑音曲线")
            } else {
                ("reading", "核对两音符的滑音衔接")
            }
        }
        "selection_edit" if args["transform"]["kind"] == "pitch_edge" => {
            if args["dry_run"] == false {
                ("editing", "应用单音起尾滑音")
            } else {
                ("reading", "检查单音起尾滑音曲线")
            }
        }
        "selection_edit" if args["transform"]["kind"] == "vibrato_envelope" => {
            if args["dry_run"] == false {
                ("editing", "应用所选音符的颤音起止和淡化")
            } else {
                ("reading", "预览所选音符的颤音起止和淡化")
            }
        }
        "selection_edit" if args["transform"]["kind"] == "pitch_reference" => {
            if args["dry_run"] == false {
                ("editing", "应用已选乐句的参考音高曲线")
            } else {
                ("reading", "测量参考人声与选区音符的音高")
            }
        }
        "selection_edit" if args["transform"]["kind"] == "align_lyrics" => {
            if args["dry_run"] == false {
                ("editing", "应用选区歌词落字")
            } else {
                ("reading", "预览选区歌词落字")
            }
        }
        "selection_edit" => {
            if args["dry_run"] == false {
                ("editing", "应用选区修改")
            } else {
                ("reading", "计算选区修改预览")
            }
        }
        "audio_clip_group" if args["action"]["kind"] == "warp" => {
            if args.get("preview_id").is_some() {
                ("editing", "保存片段对拍点")
            } else {
                ("reading", "核对源采样和目标拍位")
            }
        }
        "audio_clip_group" if args["action"]["kind"] == "tempo" => {
            if args.get("preview_id").is_some() {
                ("editing", "保存片段跟速设置")
            } else {
                ("reading", "计算跟速后的拍数和时长")
            }
        }
        "audio_clip_group" if args["action"]["kind"] == "crossfade" => {
            if args.get("preview_id").is_some() {
                ("editing", "应用片段交叉淡化")
            } else {
                ("reading", "核对接缝和源素材余量")
            }
        }
        "arrangement_edit" | "audio_clip_edit" | "audio_clip_group" => {
            if args.get("preview_id").is_some() {
                ("editing", "应用片段修改")
            } else {
                ("reading", "计算片段修改预览")
            }
        }
        "project_edit" => ("editing", "校验并保存工程修改"),
        "composition_design" => ("editing", "保存动机、段落与配器设计"),
        "render_start" => ("rendering", "核对音源与工程版本"),
        "project_import" if args["dry_run"] == true => ("working", "检查 PPSF 曲谱迁移内容"),
        "project_export" => ("working", "打包工程和素材"),
        _ => return None,
    })
}
pub fn focus(name: &str, args: &Value, result: &Value, before: Option<&Project>) -> Value {
    let reason = match name {
        "project_create" => "创建新的可编辑工程",
        "project_read" => "已读取曲谱、歌词与调声参数",
        "project_edit" => "已保存工程修改",
        "project_undo" => "撤销上一处编辑",
        "project_redo" => "恢复被撤销的编辑",
        "project_import" if result["dry_run"] == true => "已检查 PPSF 可迁移内容",
        "project_import" => "已导入曲谱副本",
        "audio_import" => "导入伴奏并对齐时间",
        "composition_design" => "已保存动机、段落与配器设计",
        "composition_review" => "已检查乐句、呼吸与段落变化",
        "section_contrast" => "已对照两段的配器与入音",
        "rhythm_interplay" => "已核对底鼓与贝斯的落点",
        "arrangement_query" => "已读取各轨的入场与留白",
        "timeline_query" => "已读取速度与拍号",
        "audio_clips" => "已读取片段位置、增益与淡化",
        "project_export" => "导出可编辑工程",
        "render_start" => "开始本地歌声渲染",
        _ => "检查工程",
    };
    let p = if result.get("project").is_some() {
        &result["project"]
    } else {
        result
    };
    let mut e = json!({"project_id":args.get("project_id").or_else(||p.get("id")),"revision":p.get("revision"),"reason":reason,"region":"piano_roll","note_ids":[]});
    if e["project_id"].is_null() {
        e["project_id"] = result["project_id"].clone();
    }
    if name.starts_with("selection_") {
        let selection = result.get("selection").unwrap_or(result);
        let count = result["change_count"]
            .as_u64()
            .unwrap_or_else(|| selection["count"].as_u64().unwrap_or(0));
        e["reason"] = json!(if name == "selection_resolve" {
            format!("已核对选区 · {count} 个音符")
        } else if result["dry_run"] == true {
            format!("选区预览已就绪 · {count} 个音符")
        } else {
            format!("已修改选区 · {count} 个音符")
        });
        e["note_ids"] = selection["note_ids"].clone();
        if args.get("audition_id").is_some() {
            e["region"] = json!("pitch");
            e["reason"] = json!(if result["dry_run"] == true {
                "已读取调声试听采用的曲谱改动"
            } else {
                "已保存确认的调声草稿"
            });
        }
        if args["transform"]["kind"] == "pitch_join" {
            e["region"] = json!("pitch");
            e["reason"] = json!(if result["dry_run"] == true {
                "两音符的滑音曲线已预览"
            } else {
                "已连接两个歌声音符的音高曲线"
            });
        }
        if args["transform"]["kind"] == "pitch_edge" {
            e["region"] = json!("pitch");
            e["reason"] = json!(if result["dry_run"] == true {
                "单音起尾滑音已预览"
            } else {
                "已调整单音起尾滑音"
            });
        }
        if args["transform"]["kind"] == "vibrato_envelope" {
            e["region"] = json!("pitch");
            e["reason"] = json!(if result["dry_run"] == true {
                "单音颤音曲线已预览"
            } else {
                "已调整所选音符的颤音"
            });
        }
        if args["transform"]["kind"] == "align_lyrics" {
            e["region"] = json!("lyrics");
            e["reason"] = json!(if result["dry_run"] == true {
                format!("已预览 {count} 个音符的落字")
            } else {
                format!("已应用 {count} 个音符的落字")
            });
        }
    }
    if let Some(t) = args.get("track_id") {
        e["track_id"] = t.clone()
    };
    if let Some(ops) = args["operations"].as_array() {
        let mut gain_ranges = Vec::new();
        for op in ops {
            if let Some(t) = op.get("track_id") {
                e["track_id"] = t.clone()
            };
            if let Some(id) = op.get("note_id") {
                e["note_ids"].as_array_mut().unwrap().push(id.clone())
            };
            let values = &op["values"];
            if op["op"] == "move_track" {
                e["region"] = json!("arrangement");
                e["reason"] = json!("已重排音轨");
            }
            if op["op"] == "duplicate_track" {
                e["region"] = json!("arrangement");
                e["reason"] = json!("已复制音轨");
            }
            if op["op"] == "set_gain_range" {
                gain_ranges.push(json!({"track_id":op["track_id"],"beat_range":op["beat_range"]}));
            } else if values.get("gain_automation").is_some() {
                e["region"] = json!("mixer");
                e["reason"] = json!("调整轨道音量曲线");
            }
            if op["op"] == "set_harmony_range" || values.get("harmony").is_some() {
                e["region"] = json!("timeline");
                e["reason"] = json!("已更新工程和弦");
                if let Some(bounds) = op.get("beat_range") {
                    e["beat_range"] = bounds.clone();
                }
            }
            if values.get("section_markers").is_some() {
                e["region"] = json!("timeline");
                e["reason"] = json!("已更新段落标记");
            }
            if values.get("master_gain_db").is_some() {
                e["region"] = json!("mixer");
                e["reason"] = json!("已调整总输出电平");
            }
            if values.get("effects").is_some() || values.get("master_limiter").is_some() {
                e["region"] = json!("effects");
                e["reason"] = json!("应用混音效果器参数");
            }
            if values.get("sends").is_some() || values.get("aux_buses").is_some() {
                e["region"] = json!("mixer");
                e["reason"] = json!("已更新发送与返回");
            }
            if values.get("output_group").is_some() || values.get("mix_groups").is_some() {
                e["region"] = json!("mixer");
                e["reason"] = json!("已更新混音编组");
            }
            if let Some(cs) = values["controllers"].as_array() {
                let old = before.and_then(|p| {
                    p.tracks
                        .iter()
                        .find(|t| Some(t.id.as_str()) == op["track_id"].as_str())
                });
                let changed = cs
                    .iter()
                    .find(|c| {
                        !old.is_some_and(|t| {
                            t.controllers.iter().any(|o| {
                                Some(o.beat) == c["beat"].as_f64()
                                    && Some(o.controller as u64) == c["controller"].as_u64()
                                    && Some(o.value as u64) == c["value"].as_u64()
                            })
                        })
                    })
                    .or_else(|| cs.first());
                if let Some(c) = changed {
                    e["controller"] = c["controller"].clone();
                }
                e["region"] = json!("velocity");
                e["reason"] = json!("调整乐器踏板与表情控制");
            }
            if ["tempo_map", "meter_map", "bpm", "meter"]
                .iter()
                .any(|k| values.get(k).is_some())
            {
                e["region"] = json!("timeline");
                e["reason"] = json!("调整速度、渐变与拍号");
            }
            if values.get("lyric").is_some() {
                e["region"] = json!("lyrics");
                e["reason"] = json!("调整歌词落字")
            } else if values.get("pitch_points").is_some() || values.get("vibrato").is_some() {
                e["region"] = json!("pitch");
                e["reason"] = json!("调整滑音与颤音")
            } else if values.get("velocity").is_some() {
                e["region"] = json!("velocity");
                e["reason"] = json!("调整音符演唱力度")
            } else if ["gain_db", "pan", "mute", "solo"]
                .iter()
                .any(|key| values.get(key).is_some())
            {
                e["region"] = json!("mixer");
                e["reason"] = json!("调整轨道混音")
            };
            if op["op"] == "add_notes" || op["op"] == "replace_notes" {
                if let Some(t) = p["tracks"]
                    .as_array()
                    .and_then(|ts| ts.iter().find(|t| t["id"] == op["track_id"]))
                {
                    if let Some(ns) = t["notes"].as_array() {
                        let ids = op["notes"]
                            .as_array()
                            .map(|new| {
                                ns.iter()
                                    .filter(|n| {
                                        new.iter().any(|x| {
                                            x["start"] == n["start"] && x["pitch"] == n["pitch"]
                                        })
                                    })
                                    .map(|n| n["id"].clone())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        e["note_ids"] = json!(ids);
                    }
                }
            }
        }
        if !gain_ranges.is_empty() {
            e["region"] = json!("arrangement");
            e["regions"] = json!(gain_ranges);
            e["reason"] = json!("已调整选定范围的音量曲线");
        }
    }
    if let Some(old) = before {
        if let Some(n) = old.tracks.iter().flat_map(|t| &t.notes).find(|n| {
            e["note_ids"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(&n.id)))
        }) {
            e["note_position"] = json!({"start":n.start,"pitch":n.pitch});
        }
    }
    if ["render_start", "project_export"].contains(&name) {
        e["region"] = json!("transport")
    };
    if name.starts_with("composition_") {
        e["region"] = json!("composition")
    };
    if name == "audio_import" {
        if args["path"].as_str().is_some_and(|path| {
            std::path::Path::new(path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        }) {
            e["region"] = json!("arrangement");
            e["reason"] = json!(if result["dry_run"] == true {
                "已核对分轨 ZIP 的音轨与起拍"
            } else {
                "已导入并对齐整套分轨"
            });
        }
        e["track_id"] = p["tracks"]
            .as_array()
            .and_then(|ts| ts.last())
            .map(|t| t["id"].clone())
            .unwrap_or(Value::Null);
        e["note_ids"] = json!([]);
    }
    e
}
