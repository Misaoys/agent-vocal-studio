use crate::{
    arrangement::{self, RangeEdit},
    clips::{self, ClipRef, GroupAction, RATE},
    model::Project,
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Deserialize)]
#[serde(tag = "kind", content = "edit", rename_all = "snake_case")]
pub(crate) enum Edit {
    Range(RangeEdit),
    ClipGroup {
        clips: Vec<ClipRef>,
        action: GroupAction,
    },
}

pub(crate) struct Plan {
    pub candidate: Project,
    pub summary: Value,
    pub range: [f64; 2],
    pub regions: Vec<Value>,
}

impl Edit {
    pub(crate) fn plan(
        &self,
        root: &Path,
        base: &Project,
        preview_id: &str,
        requested: Option<[f64; 2]>,
        capture: Option<&arrangement::Capture>,
    ) -> Result<Plan> {
        let (candidate, summary, position, regions) = match self {
            Self::Range(edit) => {
                let preview = if let Some(capture) = capture {
                    capture.preview(root, base, edit)?
                } else {
                    arrangement::preview(root, base, edit)?
                };
                let position = span(&preview.summary["destination"]).unwrap_or(edit.source);
                let regions = preview.summary["tracks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|track| {
                        json!({
                            "track_id":track["track_id"],"beat_range":position
                        })
                    })
                    .collect();
                (preview.next, preview.summary, position, regions)
            }
            Self::ClipGroup { clips, action } => {
                ensure!(capture.is_none(), "box_id applies to range edits");
                let (candidate, summary) = clips::plan_group(root, base, clips, action)?;
                let positions = |key: &str| {
                    summary[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|item| span(&item["beat_range"]))
                        .reduce(|[a, b], [c, d]| [a.min(c), b.max(d)])
                };
                let split = match action {
                    clips::GroupAction::Split { at } => Some([*at, *at]),
                    _ => None,
                };
                let position = split
                    .or_else(|| span(&summary["transition"]["beat_range"]))
                    .or_else(|| span(&summary["warp"]["beat_range"]))
                    .or_else(|| positions("after"))
                    .or_else(|| positions("before"))
                    .context("没有可试听的编排变更")?;
                let regions = clips::focus_regions(&summary, true).collect();
                (candidate, summary, position, regions)
            }
        };
        ensure!(
            summary["preview_id"] == preview_id,
            "PREVIEW_MISMATCH: 请试听本版本的准确预览"
        );
        let clock = base.timeline();
        let range = requested.unwrap_or_else(|| {
            let [a, b] = position.map(|beat| clock.seconds(beat));
            let available = crate::source_cache::frame_count(base)
                .max(crate::source_cache::frame_count(&candidate))
                as f64
                / f64::from(RATE);
            let start = (a - 0.3).max(0.);
            [start, (b + 0.5).min(start + 45.).min(available)].map(|seconds| clock.beat(seconds))
        });
        crate::audition::frames(base, range)?;
        Ok(Plan {
            candidate,
            summary,
            range,
            regions,
        })
    }
}

fn span(value: &Value) -> Option<[f64; 2]> {
    Some([value[0].as_f64()?, value[1].as_f64()?])
}
