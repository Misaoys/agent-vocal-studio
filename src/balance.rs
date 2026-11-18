//! Beat-aligned measurements from published 48 kHz stems at their rendered levels.
use crate::{
    audition::{self, SavedOutput, RATE},
    effects::{Biquad, Shape},
    model::Project,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{f64::consts::FRAC_1_SQRT_2, path::Path};

const BAND_HZ: [f64; 2] = [1000., 4000.];
const MAX_WINDOWS: usize = 64;
const NOISE_FLOOR: f64 = 1e-6;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub beat_range: [f64; 2],
    pub focus: Vec<SavedOutput>,
    pub reference_outputs: Option<Vec<SavedOutput>>,
    #[serde(default)]
    pub include_remainder: bool,
    #[serde(default = "default_window")]
    window_beats: f64,
}

fn default_window() -> f64 {
    0.5
}

pub(crate) fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["beat_range","focus"],
    "properties":{
        "beat_range":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"number","minimum":0}},
        "focus":audition::outputs_schema(),"reference_outputs":audition::outputs_schema(),
        "include_remainder":{"type":"boolean","default":false},
        "window_beats":{"type":"number","minimum":0.125,"maximum":16,"default":0.5}
    }})
}

impl Request {
    pub(crate) fn read(args: &Value) -> Result<Self> {
        let request: Self =
            serde_json::from_value(args.clone()).context("Specify the balance request")?;
        ensure!(
            !request.include_remainder || request.reference_outputs.is_none(),
            "Choose either explicit reference outputs or the mix remainder"
        );
        ensure!(
            (0.125..=16.).contains(&request.window_beats),
            "window_beats must be 0.125..16"
        );
        Ok(request)
    }
}

#[derive(Default)]
struct Energy {
    peak: f32,
    peak_offset: usize,
    broadband: f64,
    band: f64,
    frames: usize,
}

#[derive(Serialize)]
struct Levels {
    peak_dbfs: Option<f64>,
    rms_dbfs: Option<f64>,
    band_rms_dbfs: Option<f64>,
}

fn dbfs(linear: f64) -> Option<f64> {
    (linear >= NOISE_FLOOR).then(|| (20_000. * linear.log10()).round() / 1000.)
}

impl Energy {
    fn levels(&self) -> Levels {
        let samples = (2 * self.frames) as f64;
        Levels {
            peak_dbfs: dbfs(f64::from(self.peak)),
            rms_dbfs: dbfs((self.broadband / samples).sqrt()),
            band_rms_dbfs: dbfs((self.band / samples).sqrt()),
        }
    }

    fn add(&mut self, window: &Self) {
        self.peak = self.peak.max(window.peak);
        self.broadband += window.broadband;
        self.band += window.band;
        self.frames += window.frames;
    }
}

struct Band([Biquad; 2]);

impl Band {
    fn new() -> Self {
        Self([
            Biquad::new(Shape::HighPass, BAND_HZ[0], FRAC_1_SQRT_2, 0., RATE as f64),
            Biquad::new(Shape::LowPass, BAND_HZ[1], FRAC_1_SQRT_2, 0., RATE as f64),
        ])
    }

    fn measure(&mut self, frames: &[[f32; 2]]) -> Energy {
        let mut energy = Energy {
            frames: frames.len(),
            ..Default::default()
        };
        let square = |[a, b]: [f32; 2]| f64::from(a).powi(2) + f64::from(b).powi(2);
        for (index, &input) in frames.iter().enumerate() {
            let peak = crate::effects::peak(input);
            if peak > energy.peak {
                energy.peak = peak;
                energy.peak_offset = index;
            }
            energy.broadband += square(input);
            let mut band = input;
            for filter in &mut self.0 {
                filter.process(&mut band);
            }
            energy.band += square(band);
        }
        energy
    }
}

pub(crate) fn compare_drafts(
    projects: [&Project; 2],
    bounds: [f64; 2],
    samples: [&[[f32; 2]]; 2],
    ranges: [[usize; 2]; 2],
    control: &crate::jobs::Control,
) -> Result<Value> {
    const MAX_REVIEW_WINDOWS: usize = 24;
    let clocks = projects.map(Project::timeline);
    let mut bands = std::array::from_fn::<_, 2, _>(|_| Band::new());
    let mut totals = std::array::from_fn::<_, 2, _>(|_| Energy::default());
    let mut first = ranges.map(|range| range[0]);
    let [start, end] = bounds;
    let mut beat = start;
    let step = 0.5_f64.max((end - start) / MAX_REVIEW_WINDOWS as f64);
    let mut windows = Vec::new();
    let mut strongest: Option<([f64; 2], f64)> = None;
    for index in 1..=MAX_REVIEW_WINDOWS {
        control.check()?;
        let next = if index == MAX_REVIEW_WINDOWS {
            end
        } else {
            (start + index as f64 * step).min(end)
        };
        let last = std::array::from_fn::<_, 2, _>(|side| {
            let frame = (clocks[side].seconds(next) * RATE as f64).round() as usize;
            frame.clamp(first[side], ranges[side][1])
        });
        if first.iter().zip(last).all(|(&a, b)| a < b) {
            let readings = std::array::from_fn::<_, 2, _>(|side| {
                let origin = ranges[side][0];
                let energy =
                    bands[side].measure(&samples[side][first[side] - origin..last[side] - origin]);
                totals[side].add(&energy);
                energy
            });
            let mut values = levels(&readings[1], Some(&readings[0]));
            values["beat_range"] = json!([beat, next]);
            values["peak_beats_ab"] = json!(std::array::from_fn::<_, 2, _>(|side| (f64::from(
                readings[side].peak
            )
                >= NOISE_FLOOR)
                .then(|| clocks[side]
                    .beat((first[side] + readings[side].peak_offset) as f64 / RATE as f64))));
            let rms = readings.map(|item| (item.broadband / (2 * item.frames) as f64).sqrt());
            let difference = (rms[1] - rms[0]).abs();
            if difference >= 0.003 && strongest.is_none_or(|(_, current)| difference > current) {
                strongest = Some(([beat, next], difference));
            }
            windows.push(values);
            (first, beat) = (last, next);
        }
        if next == end {
            break;
        }
    }
    ensure!(!windows.is_empty(), "试听分析需要有效的音频采样窗口");
    Ok(json!({
        "beat_range":bounds,"frame_ranges":ranges,"sample_rate":RATE,"band_hz":BAND_HZ,
        "overall":levels(&totals[1],Some(&totals[0])), "windows":windows,
        "suggested_listen_range":strongest.map(|(range,_)|range),
        "focus_is_b":true,"level_matched":false,"requires_listening":true,
    }))
}

fn levels(focus: &Energy, reference: Option<&Energy>) -> Value {
    let a = focus.levels();
    let b = reference.map(Energy::levels);
    let relative = |focus: Option<f64>, reference: Option<f64>| {
        focus
            .zip(reference)
            .map(|(a, b)| ((a - b) * 1000.).round() / 1000.)
    };
    json!({"focus":a,"reference":b,
        "focus_minus_reference_db":relative(a.rms_dbfs,b.as_ref().and_then(|levels|levels.rms_dbfs)),
        "focus_minus_reference_band_db":relative(a.band_rms_dbfs,b.as_ref().and_then(|levels|levels.band_rms_dbfs))})
}

pub(crate) fn section_audio(
    root: &Path,
    project: &Project,
    render: &Value,
    ranges: [[f64; 2]; 2],
    lead: Option<&str>,
) -> Result<Value> {
    let job_id = crate::model::field(render, "job_id")?;
    let directory = audition::export_directory(root, job_id)?;
    let frame_ranges = [
        audition::frames(project, ranges[0])?,
        audition::frames(project, ranges[1])?,
    ];
    ensure!(
        frame_ranges.iter().all(|[a, b]| b - a <= 4 * RATE + 1),
        "段落听点每侧最多 4 秒"
    );
    let mix_files = audition::output_files(project, render, None)?;
    let lead_files = lead.and_then(|id| {
        audition::output_files(project, render, Some(&[SavedOutput::Track(id.into())])).ok()
    });
    let measure = |range| -> Result<Value> {
        let mut samples = audition::read_output(&directory, &mix_files, range)?;
        let mix = Band::new().measure(&samples).levels();
        let loudness = crate::loudness::Loudness::scan(&samples, || Ok(()))?.integrated(1.);
        let lead_balance = if let Some(files) = &lead_files {
            let voice = audition::read_output(&directory, files, range)?;
            for (frame, vocal) in samples.iter_mut().zip(&voice) {
                frame[0] -= vocal[0];
                frame[1] -= vocal[1];
            }
            Some(levels(
                &Band::new().measure(&voice),
                Some(&Band::new().measure(&samples)),
            ))
        } else {
            None
        };
        Ok(
            json!({"mix":mix,"integrated_lufs":loudness.map(|value|(value*1000.).round()/1000.),
                  "lead_balance":lead_balance}),
        )
    };
    let sides = [measure(frame_ranges[0])?, measure(frame_ranges[1])?];
    let difference = |key: &str| {
        sides[1]["mix"][key]
            .as_f64()
            .zip(sides[0]["mix"][key].as_f64())
            .map(|(b, a)| ((b - a) * 1000.).round() / 1000.)
    };
    Ok(
        json!({"project_id":project.id,"revision":project.revision,"job_id":job_id,"engine":render["engine"],
        "source":"saved_original_mix","beat_ranges":ranges,"frame_ranges":frame_ranges,"sample_rate":RATE,
        "level_matched":false,"lead_track_id":lead.filter(|_| lead_files.is_some()),"band_hz":BAND_HZ,
        "sides":sides,"b_minus_a":{"rms_db":difference("rms_dbfs"),"band_rms_db":difference("band_rms_dbfs")},
        "requires_listening":true}),
    )
}

pub(crate) fn report(
    root: &Path,
    project: &Project,
    render: &Value,
    request: &Request,
) -> Result<Value> {
    let job_id = crate::model::field(render, "job_id")?;
    let directory = audition::export_directory(root, job_id)?;
    let frame_range = audition::frames(project, request.beat_range)?;
    let focus_files = audition::output_files(project, render, Some(&request.focus))?;
    let focus = audition::read_output(&directory, &focus_files, frame_range)?;
    let reference = if request.include_remainder {
        let mix = audition::output_files(project, render, None)?;
        let mut remainder = audition::read_output(&directory, &mix, frame_range)?;
        for (output, input) in remainder.iter_mut().zip(&focus) {
            output[0] -= input[0];
            output[1] -= input[1];
        }
        Some(remainder)
    } else {
        request
            .reference_outputs
            .as_deref()
            .map(|outputs| {
                let files = audition::output_files(project, render, Some(outputs))?;
                audition::read_output(&directory, &files, frame_range)
            })
            .transpose()?
    };
    let clock = project.timeline();
    let [start, end] = request.beat_range;
    let [first, limit] = frame_range;
    let mut step = request.window_beats;
    while ((end - start) / step).ceil() > MAX_WINDOWS as f64 {
        step *= 2.;
    }
    let [mut focus_band, mut reference_band] = [Band::new(), Band::new()];
    let [mut total_focus, mut total_reference] = [Energy::default(), Energy::default()];
    let mut windows = Vec::new();
    let (mut previous_beat, mut previous_frame) = (start, first);
    for segment in 1..=MAX_WINDOWS {
        let beat_end = (start + step * segment as f64).min(end);
        let end_frame = ((clock.seconds(beat_end) * RATE as f64).round() as usize).min(limit);
        if end_frame > previous_frame {
            let slice = previous_frame - first..end_frame - first;
            let a = focus_band.measure(&focus[slice.clone()]);
            let b = reference
                .as_ref()
                .map(|frames| reference_band.measure(&frames[slice]));
            total_focus.add(&a);
            if let Some(b) = &b {
                total_reference.add(b);
            }
            let mut window = levels(&a, b.as_ref());
            let beat_end = if end_frame == limit { end } else { beat_end };
            window["beat_range"] = json!([previous_beat, beat_end]);
            window["frame_range"] = json!([previous_frame, end_frame]);
            windows.push(window);
            (previous_beat, previous_frame) = (beat_end, end_frame);
        }
        if end_frame == limit {
            break;
        }
    }
    Ok(
        json!({"project_id":project.id,"revision":project.revision,"job_id":job_id,"engine":render["engine"],
        "request":request,"frame_range":frame_range,"sample_rate":RATE,"band_hz":BAND_HZ,
        "window_beats":step,"reference_kind":if request.include_remainder{"mix_minus_focus"}
            else if reference.is_some(){"selected_outputs"}else{"none"},
        "overall":levels(&total_focus,reference.as_ref().map(|_| &total_reference)),
        "windows":windows}),
    )
}
