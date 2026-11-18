use crate::{engine, exchange, model::*};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead, Read, Write},
    net::SocketAddrV4,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tiny_http::{Header, Response, Server, StatusCode};
const DEVELOPMENT_ADDRESS: &str = "127.0.0.1:49732";
const SESSION_FILE: &str = "runtime/session.json";

#[derive(Deserialize, Serialize)]
struct ServiceSession {
    address: SocketAddrV4,
    token: String,
    pid: u32,
    instance_id: Option<String>,
}
struct State {
    store: Store,
    job_history: Arc<crate::jobs::History>,
    jobs: HashMap<String, Arc<crate::jobs::Control>>,
    busy: bool,
    journal: crate::presence::Journal,
    selections: crate::selection::Workspace,
    tuning_auditions: [Option<TuningAudition>; 2],
    arrangement_auditions: [Option<ArrangementAudition>; 2],
    // Preparing an Agent pair never replaces a pair owned by the editor.
    saved_auditions: [Option<(crate::audition::Manifest, Arc<crate::audition::Files>)>; 2],
}
type Shared = Arc<Mutex<State>>;

struct EditAudition<Request, Prepared> {
    request: Request,
    id: String,
    control: Arc<crate::jobs::Control>,
    ready: Option<Arc<Prepared>>,
}
type TuningAudition = EditAudition<TuningRequest, PreparedTuning>;
type ArrangementAudition = EditAudition<ArrangementRequest, PreparedArrangement>;

#[derive(Clone, PartialEq)]
struct ArrangementRequest {
    project_id: String,
    revision: u64,
    engine: String,
    preview_id: String,
    box_id: Option<String>,
    edit: Value,
    beat_range: Option<[f64; 2]>,
    level_match: bool,
}

struct PreparedArrangement {
    files: crate::audition::Files,
    manifest: crate::audition::Manifest,
    candidate: Project,
    summary: Value,
}

impl ArrangementAudition {
    fn bound(&self, id: &str, revision: u64) -> bool {
        self.request.project_id == id && self.request.revision == revision
    }

    fn snapshot(&self) -> Value {
        let mut status = self.control.snapshot();
        status["preview_id"] = json!(self.request.preview_id);
        status
    }
}

#[derive(Clone)]
struct TuningRequest {
    selection: crate::selection::Selection,
    transform: Value,
    engine: String,
    local_voice: bool,
}

struct PreparedTuning {
    files: crate::audition::Files,
    manifest: crate::audition::Manifest,
    preview: Value,
}

impl TuningRequest {
    fn same_as(&self, other: &Self) -> bool {
        self.selection.id == other.selection.id
            && self.transform == other.transform
            && self.engine == other.engine
            && self.local_voice == other.local_voice
    }
}

impl TuningAudition {
    fn bound(&self, id: &str, revision: u64) -> bool {
        self.request.selection.project_id == id && self.request.selection.revision == revision
    }

    fn snapshot(&self) -> Value {
        let mut status = self.control.snapshot();
        status["selection_id"] = json!(self.request.selection.id);
        status
    }
}

impl<Request, Prepared> Drop for EditAudition<Request, Prepared> {
    fn drop(&mut self) {
        let _ = self.control.cancel();
    }
}

fn properties(fields: Value, required: Vec<&str>) -> Value {
    json!({"type":"object","properties":fields,"required":required,"additionalProperties":false})
}
pub fn tools() -> Value {
    let string = json!({"type":"string"});
    let rev = json!({"type":"integer","minimum":1});
    let common = json!({"project_id":string,"expected_revision":rev});
    let mut out = vec![];
    let mut add = |name: &str, desc: &str, schema: Value| {
        out.push(json!({"name":name,"description":desc,"inputSchema":schema}))
    };
    add(
        "studio_status",
        "Report the Rust editor, local singing engine and file root.",
        properties(json!({}), vec![]),
    );
    add(
        "singer_list",
        "List installed VOICEVOX singing styles by default. Pass engine:openutau to list available local profiles. Use a frame_decode style as singer_id or an OpenUtau profile id as openutau_voice on a singing track.",
        properties(json!({"engine":{"type":"string","enum":["voicevox","openutau"]}}), vec![]),
    );
    add(
        "project_list",
        "Find saved editable projects, newest first. Use scope:workspace for works in progress and songs, scope:technical for explicitly marked test projects, or omit scope to list all projects. query matches title words and project IDs, case-insensitively for titles. Use limit (1–200) and zero-based offset to page the list. Each row has technical_test. Open a chosen ID with project_read.",
        properties(json!({"scope":{"type":"string","enum":["workspace","technical","all"]},
            "query":{"type":"string","maxLength":160},"limit":{"type":"integer","minimum":1,"maximum":200},
            "offset":{"type":"integer","minimum":0}}), vec![]),
    );
    add("project_create","Create a new vocal project, never replace another. Time is quarter-note beats; meter [6,8] has 3 beats/bar.",properties(json!({"title":string,"bpm":{"type":"number","minimum":30,"maximum":300},"meter":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2}}),vec!["title"]));
    add("project_read","Read stable track/note ids, current revision, lyrics, note-relative pitch_points [beat,cents], and vibrato.",properties(json!({"project_id":string}),vec!["project_id"]));
    let mut edit = common.clone();
    edit["intent_id"] = string.clone();
    edit["operations"] = json!({"type":"array","items":{"type":"object"},"description":"Atomic ops: set_project{values:{title,composition,bpm,meter,tail_seconds,tempo_map:[{beat,bpm,curve:step|linear}],meter_map:[{beat,meter:[numerator,denominator]}]}}; add_track{track_ref?,values:{name,gain_db,pan,mute,solo,singer_id,instrument:{bank,program,drums},controllers:[{beat,controller,value}]}}; update_track{track_id,values}; move_track{track_id,before_track_id}; set before_track_id to an existing track ID or null for the end; duplicate_track{track_id,name?,new_track_ref?} inserts a muted copy after the source with independent note and clip IDs and shared audio assets. Use new_track_ref as track_ref in subsequent operations in that batch; remove_track{track_id}; add_notes or replace_notes{track_id,notes:[{start,duration,pitch,lyric,velocity,vowel_continuation?:boolean,pitch_points:[[relative_beat,cents]],vibrato:{depth:0..200,rate:1..12,start:0..1}}]}; update_note{track_id,note_id,values}; delete_notes{track_id,note_ids}. Set vowel_continuation:true on a touching vocal note to sing the preceding vowel through its pitch. Use align_lyrics for syllable heads. For a new track use track_ref in place of track_id later in the same batch. Vocal harmony needs separate monophonic tracks; sampled instrument tracks allow chords. Instrument notes may omit lyrics; use supported MIDI CC instead of per-note vocal pitch/vibrato. Rejected batches leave the score unchanged."});
    let operations = edit["operations"]["description"]
        .as_str()
        .unwrap_or_default();
    edit["operations"]["description"] = json!(format!("{operations} Mixing: update_track.values.effects replaces the track insert chain; set_project.values.master_limiter sets or clears the master limiter (null clears it). Use peak_mode:true_peak for an offline dBTP ceiling, or omit peak_mode for the existing dBFS mode. set_project.values.aux_buses replaces the return list with [{{id,name,gain_db,mute,effects}}]. update_track.values.sends replaces that track's sends with [{{bus_id,gain_db}}]. set_project.values.mix_groups replaces the group bus list with the same bus fields. update_track.values.output_group is a mix group ID or null for direct master. Clear references in the same batch when deleting a bus. Read effect_catalog for ranges. For a singing track set openutau_voice to a profile id from singer_list(engine:openutau), or null for its VOICEVOX singer_id. Use engine:project when rendering these assignments. Use set_lyric_line with track_id, note_ids for the full phrase (including vowel continuations), and text for the original written line. Use remove_lyric_line with track_id and line_id."));
    let mut preview = edit.clone();
    preview.as_object_mut().unwrap().remove("intent_id");
    preview["beat_range"] =
        json!({"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2});
    preview["track_ids"] = json!({"type":"array","items":string});
    add("project_preview","Rehearse project_edit operations on the current revision without saving. Returns the score difference and the revision that was reviewed. Setting section_markers also returns section_changes. Optional beat_range and track_ids filter the score report only. For edits that must stay inside a user selection, use selection_edit or arrangement_edit with its bound preview instead.",properties(preview,vec!["project_id","expected_revision","operations"]));
    add("project_edit","Apply an all-or-nothing edit with optimistic revision checking. To create and fill a track in one edit, give add_track a unique track_ref and use it instead of track_id in later operations of that batch. Returns the full saved project with stable track IDs. Pass the optional intent_id from agent_focus to show the declared task with the applied result. For timeline section markers use set_project with values.section_markers as sorted non-overlapping {name,start,end,role?} entries (quarter-note beats), [] to clear, or null to follow the composition design. A conflict requires a fresh read; never blindly replay.",properties(edit,vec!["project_id","expected_revision","operations"]));
    add("effect_catalog","Read available audio inserts and parameter ranges, group routing, auxiliary returns, post-fader sends and master limiter settings. The expander attenuates low levels with stereo linking, a soft knee and capped range. Use its defaults as a starting point and compare quiet breaths and voiced phrases. Use project_edit to save the target's complete insert chain. Render results report expander_max_reduction_db on processed tracks or buses.",properties(json!({}),vec![]));
    add("gain_automation_query","Read post-effects gain trim automation on a track for this project revision. Optional beat_range returns the pinned endpoint levels and points in the range. In project_edit, set_gain_range with track_id, beat_range, points [{beat,db}] replaces only strictly interior points and keeps the curve outside intact. update_track.values.gain_automation replaces the whole curve. The track fader remains an independent trim.",properties(json!({"project_id":string,"expected_revision":rev,"track_id":string,"beat_range":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}}),vec!["project_id","expected_revision","track_id"]));
    add("harmony_query","Read chord symbols and active spans for this project revision. Root and slash bass are pitch classes (C=0). An optional beat_range clips spans while retaining the carried chord at the start. Modify globally with set_project.values.harmony or make a range edit via project_edit op set_harmony_range {beat_range, markers:[{beat,symbol}]}. The first marker must match the range start. Use N.C. for no chord. Markers do not change audio.",properties(json!({"project_id":string,"expected_revision":rev,"beat_range":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}}),vec!["project_id","expected_revision"]));
    add("voicing_preview","Preview compact and spread upper voicings and specified slash bass over authored chord markers. Supply independent, explicitly timed upper or bass events, registers and optional vocal melody. Returns editable note values without writing. Listen to the alternatives, choose one, then add notes to the intended instrument tracks with project_edit.",properties(json!({"project_id":string,"expected_revision":rev,"request":crate::voicing::schema()}),vec!["project_id","expected_revision","request"]));
    add(
        "project_undo",
        "Undo one saved edit; revision always increases.",
        properties(common.clone(), vec!["project_id", "expected_revision"]),
    );
    add(
        "project_redo",
        "Redo one saved edit; revision always increases.",
        properties(common, vec!["project_id", "expected_revision"]),
    );
    add("project_recover","Recover the previous atomic-save backup, preserving the current/damaged file separately. If the current project is readable, expected_revision is required. This restores the previous saved revision, not unsaved edits.",properties(json!({"project_id":string,"expected_revision":rev}),vec!["project_id"]));
    add("project_import","Import a copy from an absolute path or base64 data + filename. Supported: AVPACK, AVPROJ/JSON, MIDI, NT/NT2 PPSF. For PPSF, dry_run=true previews constant-tempo score migration. Confirm with expected_source_sha256 to require the same score bytes. At confirmation, reference_audio can supply absolute vocal_path and/or backing_path (rendered WAV/FLAC/MP3 from song start). A vocal reference mutes the editable score tracks so the original audio plays without doubling. MIDI imports tempo, meter and standard markers. Section ends are inferred and import warnings describe the conversion.",properties(json!({"path":string,"filename":string,"data_base64":string,"dry_run":{"type":"boolean"},"expected_source_sha256":string,"reference_audio":properties(json!({"vocal_path":string,"backing_path":string}),vec![])}),vec![]));
    add("project_export","Export avpack (editable score + tuning + backing audio), avproj (score JSON; audio stays in this home), MIDI, WAV or a ZIP of all WAV stems (wav_stems). For WAV, pass mix_job_id and expected_revision, optional stem (an exact filename from render_status.result.stems), sample_rate (44100 or 48000), and bit_depth (16 or 24). Default is the original 48000 Hz 24-bit PCM. For WAV or wav_stems, optionally use beat_range [start,end) or box_id from arrangement_query. Box exports use its time range. The selected output or entire rendered stem set is retained. Each file begins at that range start. The receipt gives source_frame_range at 48000 Hz. For wav_stems, omit stem. Returns a delivery job_id immediately. Read render_status and cancel with render_cancel. For avpack, pass mix_job_id and expected_revision to preserve the exact completed stereo WAV of this project revision. A new import returns archived_mix when the reference was restored.",properties(json!({"project_id":string,"format":{"type":"string","enum":["avpack","avproj","mid","wav","wav_stems"]},"expected_revision":rev,"mix_job_id":string,"stem":string,"sample_rate":{"type":"integer","enum":[44100,48000]},"bit_depth":{"type":"integer","enum":[16,24]},"box_id":string,"beat_range":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2}}),vec!["project_id","format"]));
    add("audio_import","Import mono/stereo WAV, MP3 or FLAC, or append a clip when track_id identifies an audio track. Placement is in quarter-note beats. Absolute paths are streamed; filename plus data_base64 is also supported. Up to 15 minutes and 8–192 kHz. Decoded audio is normalized to 48 kHz float. The expected revision is checked again when saving. Import a Studio stem ZIP from an absolute path to add all stems in one edit. Default start is the manifest start beat, or zero for full exports. For ZIP, dry_run previews tracks and returns source_sha256. Use expected_source_sha256 to confirm the same ZIP. A supplied start overrides the manifest.",properties(json!({"project_id":string,"expected_revision":rev,"path":string,"filename":string,"data_base64":string,"start":{"type":"number","minimum":0},"name":string,"track_id":string,"dry_run":{"type":"boolean"},"expected_source_sha256":string}),vec!["project_id","expected_revision"]));
    add(
        "audio_waveform",
        "Read peak bins and optional attack candidates for one clip source window. Include transients to find pronounced energy attacks. frame_range [first,end) and each candidate's frame are relative to the full clip source window at 48 kHz. Candidate beat is its current project beat, including tempo and warp. Returns up to 96 candidates. Specify clip_id on multi-clip tracks; legacy identifies old single backing clips.",
        properties(json!({"project_id":string,"track_id":string,"clip_id":string,"expected_revision":rev,
            "frame_range":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"integer","minimum":0}},
            "include_transients":{"type":"boolean"}}),vec!["project_id","track_id"]),
    );
    add("audio_clips","List audio tracks and their clips with stable clip IDs, source offset and length in 48 kHz frames, placement in quarter-note beats, and resolved waveform timing. Pass from_ui=true to also read the user's exact clip selection for this revision. Legacy single clips are identified as legacy within their track until their first clip edit.",properties(json!({"project_id":string,"from_ui":{"type":"boolean"}}),vec!["project_id"]));
    add("audio_clip_edit","Preview or atomically split, move, copy, trim, set gain/fades, or remove a WAV clip. Send action {kind:split,at}, {kind:move|copy,start,to_track_id?,allow_overlap?}, {kind:trim,first,end,allow_overlap?}, {kind:set,name?,gain_db?,fade_in?,fade_out?,fade_in_beats?,fade_out_beats?}, or {kind:remove}. at/start are quarter-note beats; trim/fade fields are 48 kHz frames. Trim uses a half-open range relative to the current source start. Negative first or end above the current length reveals more source audio. The range must retain part of the clip. Extension must fit the asset and arrangement bounds. It rejects newly occupied overlap unless requested. For tempo-follow clips each fade may instead use a quarter-note duration; 0 clears that side. Move/copy reject overlap unless explicitly allowed. Commit requires the same request plus preview_id.",properties(json!({"project_id":string,"expected_revision":rev,"track_id":string,"clip_id":string,"action":{"type":"object","required":["kind"],"properties":{"kind":{"type":"string","enum":["split","move","copy","trim","set","remove"]},"start":{"type":"number","minimum":0},"at":{"type":"number","minimum":0},"to_track_id":string,"allow_overlap":{"type":"boolean"},"first":{"type":"integer","minimum":-43200000},"end":{"type":"integer","minimum":1},"name":string,"gain_db":{"type":"number","minimum":-60,"maximum":12},"fade_in":{"type":"integer","minimum":0},"fade_out":{"type":"integer","minimum":0},"fade_in_beats":{"type":"number","minimum":0},"fade_out_beats":{"type":"number","minimum":0}},"additionalProperties":false},"preview_id":string}),vec!["project_id","expected_revision","track_id","clip_id","action"]));
    let clip_refs = json!({"type":"array","maxItems":1024,"items":properties(json!({"track_id":string,"clip_id":string}),vec!["track_id","clip_id"])});
    add("audio_clip_group","Preview or atomically edit explicit clips. Supply clips or from_ui=true for preview; commit with the same action, returned clips and preview_id. Move/copy take delta in quarter-note beats and optional lane_offset. Gain takes delta_db. Crossfade uses the full existing overlap between two same-track clips, including tempo-follow and warped clips. Two touching fixed-time clips may extend into source handles. Both tempo-follow sides share the same beat-anchored overlap. Extract {kind:extract,beat_range:[start,end],to_track_id?} copies a range of one fixed-time clip to the same beats on a different audio track; omit the target to create a comp track. Tempo {kind:tempo,beats?,source_bpm?,fixed?,allow_overlap?} follows the tempo map. fixed:true restores source seconds and clears internal warp points. Warp {kind:warp,beats?,points:[{frame,beat}],allow_overlap?} replaces the full internal point list of one clip. frame is a 48 kHz frame relative to its source window; beat is relative to clip start. Both coordinates increase strictly. Endpoints are implicit. One commit is one undo.",
        properties(json!({"project_id":string,"expected_revision":rev,"clips":clip_refs,"from_ui":{"type":"boolean"},
            "action":{"type":"object","required":["kind"],"properties":{
                "kind":{"type":"string","enum":["move","copy","gain","slip","crossfade","tempo","warp","extract","remove","split"]},
                "at":{"type":"number","minimum":0,"description":"Split all supplied clips at this quarter-note beat. Each supplied clip must cross the cut outside its fade."},
                "delta":{"type":"number"},"lane_offset":{"type":"integer"},"delta_db":{"type":"number"},
                "source_delta_frames":{"type":"integer","minimum":-43200000,"maximum":43200000,
                    "description":"Slip all selected source windows by this signed 48 kHz frame count. Positive reads later in each source. Clip positions, lengths, fades, and warp coordinates stay fixed. If any asset lacks enough material, the whole edit fails."},
                "length_frames":{"type":"integer","minimum":2},"curve":{"type":"string","enum":["linear","equal_power"]},
                "beats":{"type":"number","exclusiveMinimum":0},"source_bpm":{"type":"number","exclusiveMinimum":0},
                "fixed":{"type":"boolean"},"allow_overlap":{"type":"boolean"},
                "beat_range":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"number","minimum":0}},
                "to_track_id":string,
                "points":{"type":"array","maxItems":1024,"items":{"type":"object","required":["frame","beat"],
                    "properties":{"frame":{"type":"integer","minimum":1},"beat":{"type":"number","exclusiveMinimum":0}},
                    "additionalProperties":false}}
            },"additionalProperties":false},"preview_id":string}),vec!["project_id","expected_revision","action"]));
    add("render_start","Start an asynchronous local render of a specific revision. project uses each track's assigned voice. Tracks without openutau_voice use their VOICEVOX singer_id. voicevox forces VOICEVOX; openutau uses a configured local Japanese Classic voicebank; guide uses an oscillator for vocals; sampled instruments remain their real local patches. prepare_monitor also prepares muted/solo-excluded track sources for desktop monitoring without including them in the rendered stems. Read render_status.result.loudness for stereo programme LUFS, maximum momentary and short-term LUFS, and measured dBTP of the exported WAV. Returns job_id. Optional intent_id correlates the render and its outcome with a declared focus.",properties(json!({"project_id":string,"expected_revision":rev,"engine":{"type":"string","enum":engine::RENDER_ENGINES},"prepare_monitor":{"type":"boolean"},"intent_id":string}),vec!["project_id","expected_revision","engine"]));
    add("render_status","Read render progress or result with measured peak/RMS, aligned Chinese-named stems and mix path, or the separate WAV stems delivery report. Technical success does not certify musical quality.",properties(json!({"job_id":string}),vec!["job_id"]));
    add("render_cancel","Request cooperative cancellation of a specific job. Rendering stays busy until the worker acknowledges; an in-flight VOICEVOX request must return first. Once the atomic output commit begins cancellation is too late. Never terminate the shared voice engine.",properties(json!({"job_id":string}),vec!["job_id"]));
    add("render_jobs","List active and recent render jobs. Pass deliveries_only:true to list stem ZIP delivery jobs instead. With project_id, expected_revision and engine, return a matching render for reuse. For the exact reference in an imported AVPACK, pass project_id, expected_revision and archived_only:true. The reference is outside the live render cache.",properties(json!({"project_id":string,"active_only":{"type":"boolean"},"expected_revision":rev,"engine":{"type":"string","enum":engine::RENDER_ENGINES},"archived_only":{"type":"boolean"},"deliveries_only":{"type":"boolean"}}),vec![]));
    add("render_balance","Measure original-level aligned outputs of a completed current revision. Focus is one or more direct tracks, mix groups, or returns. Optionally compare explicit reference_outputs or the full rendered mix minus focus (include_remainder). Reports beat windows with sample peak, stereo RMS, and 1–4 kHz RMS. Read-only, max 45 seconds, numeric results only. Use for choosing what to audition.",properties(json!({"project_id":string,"expected_revision":rev,"job_id":string,"request":crate::balance::schema()}),vec!["project_id","expected_revision","job_id","request"]));
    #[cfg(windows)]
    add("desktop_transport","Read or control the open Windows editor's WASAPI preview. Read status for the visible revision, ownership, position and fresh ticket. Devices lists Windows output endpoints and preference read-only. For play, send the current project/revision, completed render job_id, beat_range and reason. For audition_side, send the current project/revision, audition_id from range_audition or a completed tuning_audition or arrangement_audition, variant A or B and reason. Pause/stop use fresh tickets and Agent ownership. User actions supersede Agent control. A timeout requires status before retrying.",properties(json!({"action":{"type":"string","enum":["status","devices","play","audition_side","pause","stop"]},"ticket":string,"project_id":string,"expected_revision":rev,"job_id":string,"beat_range":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2},"repeat":{"type":"boolean","default":false},"audition_id":string,"variant":{"type":"string","enum":["A","B"]},"reason":string}),vec!["action"]));
    add("range_audition","Compare two ranges or output selections from a completed render, or compare two fixed-time recordings in the same beat range. Prepare with beat_ranges [[a_start,a_end],[b_start,b_end]], each at most 45 seconds. Use job_id, recordings, or renders. For renders supply [{project_id,revision,job_id},{project_id,revision,job_id}] with completed jobs. At least one side must match the project_id and expected_revision that own playback; each side uses its own render snapshot timing. Recordings use clip gain and fades, independent of track effects, faders, mute and solo. Optionally pass outputs [side_A,side_B]; null uses the full mix and each list selects rendered track, group or return IDs. For grouped tracks use their group ID. Matching loudness is optional. Status reads the current pair. Export saves the prepared WAVs and source manifest to a local ZIP. Export and release require the exact audition_id from the same caller slot. Play with desktop_transport audition_side.",properties(json!({"action":{"type":"string","enum":["prepare","status","export","release"]},"project_id":string,"expected_revision":rev,"job_id":string,"renders":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"object","properties":{"project_id":string,"revision":rev,"job_id":string},"required":["project_id","revision","job_id"],"additionalProperties":false}},"audition_id":string,"recordings":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"object","properties":{"track_id":string,"clip_id":string},"required":["track_id","clip_id"],"additionalProperties":false}},"beat_ranges":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"number","minimum":0}}},"outputs":{"type":"array","minItems":2,"maxItems":2,"items":{"type":["array","null"],"minItems":1,"maxItems":44,"uniqueItems":true,"items":{"type":"object","properties":{"kind":{"type":"string","enum":["track","group","return"]},"id":string},"required":["kind","id"],"additionalProperties":false}}},"level_match":{"type":"boolean","default":true},"match_method":{"type":"string","enum":["lufs","rms"]}}),vec!["action","project_id","expected_revision"]));
    add("arrangement_audition",
        "Prepare a full-mix A/B of an exact arrangement_edit or audio_clip_group preview without writing the score. Pass the matching preview_id and edit: {kind:range,edit:{action,track_ids,source,target_start?,slice_audio?,semitones?,quantize?}} or {kind:clip_group,edit:{clips,action}}. For a preview bound to the editor box, pass the same box_id on prepare. Use an explicit beat_range or let Studio choose around the destination, affected section or seam. Up to 45 seconds per side. Original level by default, optional LUFS match. Poll status (audition_id is job_id, or omit it to recover the active binding). Measure reads the same pair for up to 12 seconds and reports sample steps at a crossfade's exact edges. Play via desktop_transport audition_side. To commit the frozen candidate, call apply with matching audition_id and preview_id after confirmation. Cancel or release only the exact pair.",
        properties(json!({"action":{"type":"string","enum":["prepare","status","measure","apply","cancel","release"]},
            "project_id":string,"expected_revision":rev,"preview_id":string,"audition_id":string,"box_id":string,
            "engine":{"type":"string","enum":engine::RENDER_ENGINES},"edit":{"type":"object"},
            "level_match":{"type":"boolean","default":false},"intent_id":string,
            "beat_range":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2}}),
            vec!["action","project_id","expected_revision"]));
    add("tuning_audition",
        "Prepare A current and B selected vocal draft without changing the score. Pass project_id, expected_revision, selection_id, engine and the same transform as selection_edit. Supports pitch_join, pitch_edge, pitch_reference, vibrato_envelope, align_lyrics, fit_lyrics and set of vocal pitch, expression, pronunciation, lyrics or velocity. The default local_voice:true synthesizes just one vocal track. Set false for full mix, or an isolated rendered track for pitch_reference. Poll status with the returned job_id as audition_id, or omit the ID to recover the sole active tuning job for this binding. A duplicate in-flight or completed request returns the same job. Measure uses the exact published A/B PCM with an optional in-pair beat_range up to 12 seconds. Play with desktop_transport audition_side. Apply the frozen operations with selection_edit and audition_id or release the pair. Cancel requests wait for the engine to become idle; release and cancel bind to the exact audition_id.",
        properties(json!({"action":{"type":"string","enum":["prepare","status","cancel","measure","release"]},
            "project_id":string,"expected_revision":rev,"selection_id":string,"audition_id":string,
            "engine":{"type":"string","enum":engine::RENDER_ENGINES},"transform":{"type":"object"},
            "local_voice":{"type":"boolean","default":true},"intent_id":string,
            "beat_range":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2}}),
            vec!["action","project_id","expected_revision"]));
    add("agent_focus","Announce the next action in the editor. The editor draws an Agent cursor and highlights the target. Returns intent_id bound to the visible project revision. Pass it to project_edit, selection_edit, or render_start to retain the declared task in the activity label.",properties(json!({"project_id":string,"expected_revision":rev,"track_id":string,"note_ids":{"type":"array","items":{"type":"string"}},"region":{"type":"string","enum":["piano_roll","lyrics","pitch","velocity","mixer","effects","transport","composition","timeline","arrangement"]},"reason":string,"beat_range":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2},"controller":{"type":"integer","enum":[1,7,10,11,64,91,93]}}),vec!["project_id","region","reason"]));
    add("agent_events","Read the bounded Agent activity journal after a sequence number. Used by the editor overlay.",properties(json!({"after":{"type":"integer","minimum":0}}),vec![]));
    add("music_knowledge","Read built-in composition workflow, music theory, production topics and style cards, with sources and listening limits. topic: melody/harmony/rhythm/prosody/form/arrangement/mix/style_blending. style: jpop/vocaloid/pop/citypop/rock/metal/jazz/rnb/hiphop/house/dnb/orchestral/folk/ambient.",properties(json!({"topic":string,"style":string}),vec![]));
    add("composition_design","Save a concrete song design before writing a new song. plan: purpose song|technical_test|existing_material, brief, style, key, motif, emotional_arc, lyric_language, sections:[{name,start,end,role}], arrangement:[role descriptions], optional reference_traits and exclusions. Technical tests need a brief and purpose; they are not songs.",properties(json!({"project_id":string,"expected_revision":rev,"plan":{"type":"object"}}),vec!["project_id","expected_revision","plan"]));
    add("composition_review","Read scoped score phrases with written lyric lines and their saved and current readings, timed syllables and their stable note IDs, gaps, backing attacks and section density. listening_points gives exact note IDs and meter positions at phrase entry, longest syllable, top pitch and release. Nearby scored instrument attacks include signed offsets in milliseconds. audition_range is a short, tempo-aware excerpt limited by the review and render bounds; use it as both A/B beat_ranges to compare the full mix and a vocal output. Held vowels have one lyric onset. Null syllable lyric means its onset lies outside the range. Optional track_ids and beat_range restrict the review. Check expected_revision and choose where to listen.",properties(json!({"project_id":string,"expected_revision":rev,"track_ids":{"type":"array","items":string},"beat_range":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}}),vec!["project_id"]));
    add("section_contrast","Compare arrangement evidence between two explicit beat ranges. Reads track entries, meter-comparable attack and drum-hit patterns, attacks during the lead or its gaps, notated register overlap, and audio clip placement. Listening points include matching the lead entries and last releases when available. Pass job_id and the zero-based audio_cue index for measured original mix and available lead balance for that cue, then reuse audio.beat_ranges for A/B. Omit lead_track_id to infer it only for a single vocal track. Counts describe the score; audio clip coverage describes placement. Check the sound before editing.",properties(json!({"project_id":string,"expected_revision":rev,"beat_ranges":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"number","minimum":0}}},"track_ids":{"type":"array","items":string},"lead_track_id":string,"job_id":string,"audio_cue":{"type":"integer","minimum":0}}),vec!["project_id","expected_revision","beat_ranges"]));
    let mut rhythm_fields = crate::rhythm::fields();
    rhythm_fields["project_id"] = string.clone();
    rhythm_fields["expected_revision"] = rev.clone();
    add("rhythm_interplay","Inspect notated kick and bass timing at the project sample clock. Kick pitches default to GM 35 and 36. Pairs are mutual nearest attacks within tolerance, and signed offset_ms is bass minus kick. Use placement for one explicitly named bass note and kick to preview exact update_note operations, preserving the bass length in seconds unless length_ms is supplied. No write. Audition the intended timing before commit.",properties(rhythm_fields,vec!["project_id","expected_revision","drum_track_id","bass_track_id","beat_range"]));
    add("motif_returns","Find scored returns of a reference of 4–16 consecutive vocal notes on the same track. Provide first and last stable note IDs. Results include transposition, rhythmic scale, changed endings, lyric tokens, chord spans, and backing attacks. beat_range must also contain the reference. track_ids restrict visible score context, including accompaniment. These are candidates for playback and revision, not a quality score.",properties(json!({"project_id":string,"expected_revision":rev,"anchor":crate::motif::anchor_schema(),"track_ids":{"type":"array","items":string},"beat_range":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}}),vec!["project_id","expected_revision","anchor"]));
    add("selection_resolve","Resolve a fixed selection by track_ids, note_ids, time_range [inclusive start, exclusive end], pitch_range [inclusive low,high], or from_ui true. Returns stable note ids and a revision-bound selection_id. time_match is onset (default) or intersects. Default scope is all notes; use filters for targeted edits.",properties(json!({"project_id":string,"track_ids":{"type":"array","items":{"type":"string"}},"note_ids":{"type":"array","items":{"type":"string"}},"time_range":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"pitch_range":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"time_match":{"type":"string","enum":["onset","intersects"]},"from_ui":{"type":"boolean"}}),vec!["project_id"]));
    add("lyrics_reading","Read only. Split continuous Japanese kana into singable morae with mode kana (offline, default). Use mode japanese to extract a suggested reading for lyrics containing kanji using local VOICEVOX. Confirm the reading and note count before committing. Returns a space-delimited reading and the mora array.",properties(json!({"text":string,"mode":{"type":"string","enum":["kana","japanese"]}}),vec!["text"]));
    add("pronunciation_query","Read VOICEVOX consonant lead and vowel placement for selected vocal notes, including actual engine frame resolution and limits. Set consonant_ms (1–300 ms, null restores auto) or vowel_offset_ms (-120 to 120 ms; negative sings early, positive sings late, null resets) through update_note or selection_edit. A null vowel_onset_seconds means the syllable has no movable vowel. MIDI timing is preserved. Render using voicevox.",properties(json!({"project_id":string,"expected_revision":rev,"track_id":string,"note_ids":{"type":"array","items":string,"minItems":1,"maxItems":240,"uniqueItems":true}}),vec!["project_id","expected_revision","track_id","note_ids"]));
    let mut selection_edit_schema = properties(
        json!({"selection_id":string,"expected_revision":rev,
        "intent_id":string,"dry_run":{"type":"boolean"},"transform":{"type":"object"},"audition_id":string}),
        vec!["selection_id", "expected_revision"],
    );
    selection_edit_schema["anyOf"] =
        json!([{"required":["transform"]},{"required":["audition_id"]}]);
    add("selection_edit","Preview or atomically edit exactly the resolved notes. dry_run defaults true. Transform kinds: transpose{semitones}, move{beats}, quantize{step}, velocity_scale{factor}, align_lyrics{reading}, fit_lyrics{reading,anchors?}, pitch_join{lead_ms,trail_ms}, pitch_edge{side:start|end,length_ms,offset_cents}, set_lyrics{lyrics:[...]}, set{values:{duration,pitch,velocity,lyric,vibrato,pitch_points,consonant_ms,vowel_offset_ms,vowel_continuation}}. align_lyrics assigns readings only to syllable heads. align_lyrics, fit_lyrics and set_lyrics accept written_text to save the original alongside the reading, or null to remove the original on the exact same note IDs. Include any complete vowel continuations for this metadata. fit_lyrics distributes kana across 1–128 selected vocal notes, preserving rests and sustaining vowels across touching notes. Optional anchors are [{mora_index:1,note_id}]; the preview returns the note groups and exact operations. pitch_join draws a glide before and after the shared boundary of exactly two touching vocal notes. pitch_edge offsets the beginning or end of exactly one vocal note relative to its existing curve, over 20–2000 ms with up to ±600 cents. Timing, lyrics and pitch curve outside that window are preserved. pitch_reference{reference:{track_id,clip_id}} estimates editable pitch from one aligned isolated vocal clip in fixed-time mode. It analyzes up to 32 notes across 15 seconds and skips unreliable notes. vibrato_envelope{settings:{depth,rate,start,end,fade_in_ms,fade_out_ms}} changes only the vibrato of one selected vocal note. Start and end are note fractions. Fades are milliseconds under the actual tempo map. The preview gives the beat span, nominal cycles and maximum envelope depth in cents. Optional intent_id correlates the preview and commit with a declared focus. A changed project invalidates the selection. To re-read or apply the exact frozen operations of a completed tuning_audition, pass audition_id instead of transform and reuse its selection_id and expected_revision. Set dry_run:false only to apply. Application releases that pair and is one undo.",selection_edit_schema);
    add("instrument_catalog","Read the actually installed local sample-bank presets, bank/program numbers (zero-based), drum note map and supported MIDI CC. Create a sampled track with add_track values:{name,instrument:{bank,program,drums}}. Different pitches may overlap for chords; repeated overlapping instances of the same key need separate tracks. Use controllers:[{beat,controller,value}] sorted by beat for pedal/expression. Instrument notes do not need lyrics; vocal-only per-note pitch/vibrato must not be put on sampled tracks.",properties(json!({}),vec![]));
    add("timeline_query","Convert quarter-note beats and seconds using this project's stepped/linear tempo map, and read bar/beat positions in its meter map. tempo_map points have positive increasing beat,bpm,curve step|linear; curve is the transition from the preceding point (implicit beat 0 uses project.bpm). meter_map points beat,meter start a new bar, preserving an incomplete preceding bar. Edit maps atomically with set_project; query actual time rather than using beat*60/base_bpm. Optional pulse_range [start,end) returns metronome pulses, with 48 kHz frame positions and downbeat accents, at most 45 seconds. Use a start as low as -8 quarter-note beats to inspect the pre-zero count-in.",properties(json!({"project_id":string,"pulse_range":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"number","minimum":-8}},"beats":{"type":"array","items":{"type":"number","minimum":0},"maxItems":256},"seconds":{"type":"array","items":{"type":"number","minimum":0},"maxItems":256}}),vec!["project_id"]));
    add("arrangement_query","Read whole-project track spans and timeline section coverage (from explicit markers or the composition design), or provide beat_range and optional track_ids for a per-track inventory. from_ui:true freezes the published arrangement box and returns a box_id for preview and commit. box_id reads that same snapshot. Optional beats and tracks must stay inside the box. Inventory reports notes, clips, empty lanes and boundary cuts. Positions are quarter-note beats.",properties(json!({"project_id":string,"expected_revision":rev,"from_ui":{"type":"boolean"},"box_id":string,"track_ids":{"type":"array","items":string,"minItems":1,"uniqueItems":true},"beat_range":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2}}),vec!["project_id"]));
    add("arrangement_edit","Preview or atomically copy, move or clear whole notes and audio clips, transpose pitched notes, or quantize note onsets within the half-open source [start,end) on explicit track_ids. Provide target_start for copy/move. Use a nonzero integer semitones for transpose; drum pads, audio clips and harmony markers keep their original settings. Review pitch_range per affected track and skipped material in warnings. For quantize pass quantize:{step,strength,swing}; step is in quarter-note beats, strength is a fraction up to 1, and swing is 0.5 (straight) to 0.75. It retimes singing, pitched instruments and drums by the local bar grid, moves sustained syllables together, and reports shift_ms. Copy generates new IDs. Per-track lyric_changes lists copied or moved original lyric lines, removed anchors and alignment effects. Inspect state_after before applying. Original text is limited to lines entirely inside the source. Moving a whole clip retains its ID except on legacy promotion. A moved excerpt gets a new ID. Set slice_audio:true to crop fixed-time clip windows at the bounds without overwriting the source file. Cuts through notes or sustained syllables, cuts in fades or stretched clips, occupied destinations and spill are rejected. MIDI CC, gain automation, tempo/meter, and composition sections stay at their own positions. Pass box_id from arrangement_query to bind the source beats, tracks and item IDs. An explicit copy or move destination can be elsewhere and must pass collision checks. Commit with identical arguments plus the returned preview_id at the same revision.",properties(json!({"project_id":string,"expected_revision":rev,"box_id":string,"action":{"type":"string","enum":["copy","move","clear","transpose","quantize"]},"track_ids":{"type":"array","items":{"type":"string"},"minItems":1,"uniqueItems":true},"source":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2},"target_start":{"type":"number","minimum":0},"slice_audio":{"type":"boolean"},"semitones":{"type":"integer","minimum":-127,"maximum":127},"quantize":crate::arrangement::quantize_schema(),"preview_id":string}),vec!["project_id","expected_revision","action","track_ids","source"]));
    Value::Array(out)
}
fn call(state: &Shared, name: &str, a: &Value, source: &str) -> Result<Value> {
    if name == "render_balance" {
        let (root, project) = {
            let s = state.lock().unwrap();
            let project = s.store.read(field(a, "project_id")?)?;
            ensure!(
                project.revision == revision(a)?,
                "REVISION_CONFLICT: 请选择当前工程版本"
            );
            (s.store.root.clone(), project)
        };
        let request = crate::balance::Request::read(&a["request"])?;
        let result = crate::audition::completed_result(&root, &project, field(a, "job_id")?)?;
        return crate::balance::report(&root, &project, &result, &request);
    }
    if name == "range_audition" {
        return range_audition(state, a, source);
    }
    if name == "tuning_audition" {
        return tuning_audition(state, field(a, "action")?, a, source, true);
    }
    if name == "arrangement_audition" {
        return arrangement_audition(state, a, source);
    }
    if matches!(
        name,
        "pitch_audition_start" | "pitch_audition_status" | "pitch_audition_release"
    ) {
        ensure!(source == "user", "此操作由桌面滑音面板发起");
        let action = match name {
            "pitch_audition_start" => "prepare",
            "pitch_audition_status" => "status",
            _ => "release",
        };
        return tuning_audition(state, action, a, source, false);
    }
    #[cfg(windows)]
    if name == "desktop_transport" {
        return desktop_transport(state, a);
    }
    if name == "agent_events" {
        return Ok(state
            .lock()
            .unwrap()
            .journal
            .since(a["after"].as_u64().unwrap_or(0)));
    }
    if name == "music_knowledge" {
        return Ok(crate::music::knowledge(
            a["topic"].as_str(),
            a["style"].as_str(),
        ));
    }
    if name == "effect_catalog" {
        return Ok(crate::effects::catalog());
    }
    if name == "agent_focus" {
        let mut s = state.lock().unwrap();
        let project = s.store.read(field(a, "project_id")?)?;
        if a.get("expected_revision").is_some() {
            ensure!(project.revision == revision(a)?, "REVISION_CONFLICT");
        }
        ensure!(
            a["reason"]
                .as_str()
                .is_some_and(|r| !r.trim().is_empty() && r.chars().count() <= 300),
            "Focus reason must be 1..300 characters"
        );
        ensure!(
            [
                "piano_roll",
                "lyrics",
                "pitch",
                "velocity",
                "mixer",
                "effects",
                "transport",
                "composition",
                "timeline",
                "arrangement"
            ]
            .contains(&field(a, "region")?),
            "Invalid focus region"
        );
        if let Some(c) = a.get("controller") {
            ensure!(
                c.as_u64()
                    .is_some_and(|v| [1, 7, 10, 11, 64, 91, 93].contains(&v)),
                "Unsupported focus controller"
            );
        }
        if let Some(range) = a.get("beat_range") {
            let [start, end]: [f64; 2] = serde_json::from_value(range.clone())?;
            ensure!(
                start >= 0. && end > start && end <= 50000.,
                "Invalid focus beat range"
            );
        }
        let mut event = json!({
            "project_id":project.id,"revision":project.revision,"intent_id":uid("intent"),
            "region":a["region"],"reason":a["reason"],"phase":"intent"
        });
        for key in ["track_id", "note_ids", "beat_range", "controller"] {
            if let Some(value) = a.get(key) {
                event[key] = value.clone();
            }
        }
        return Ok(s.journal.push("focus", event));
    }
    let before = if name == "project_edit" && source == "agent" {
        a["project_id"]
            .as_str()
            .and_then(|id| state.lock().unwrap().store.read(id).ok())
    } else {
        None
    };
    let activity = if source == "agent" {
        crate::presence::working_label(name, a)
    } else {
        None
    };
    let activity_project = if name == "selection_edit" && activity.is_some() {
        a["selection_id"].as_str().and_then(|id| {
            let s = state.lock().unwrap();
            s.selections
                .saved
                .get(id)
                .map(|selection| selection.project_id.clone())
                .or_else(|| {
                    s.tuning_auditions[1]
                        .as_ref()
                        .filter(|job| a["audition_id"] == job.id && job.request.selection.id == id)
                        .map(|job| job.request.selection.project_id.clone())
                })
        })
    } else {
        a["project_id"].as_str().map(str::to_owned)
    };
    let intent = if a.get("intent_id").is_some() {
        ensure!(
            source == "agent" && matches!(name, "project_edit" | "selection_edit" | "render_start"),
            "intent_id is supported for Agent edits and renders"
        );
        let id = field(a, "intent_id")?;
        let project = activity_project
            .as_deref()
            .context("Missing project for intent")?;
        let expected = revision(a)?;
        Some(
            state
                .lock()
                .unwrap()
                .journal
                .intent(id, project, expected)
                .context("INTENT_MISMATCH: announce a focus for this project revision")?,
        )
    } else {
        None
    };
    if let Some((phase, label)) = activity {
        state.lock().unwrap().journal.push_with_intent(
            "agent_activity",
            json!({"project_id":activity_project,"phase":phase,"reason":label,"operation":name}),
            intent.as_ref(),
        );
    }
    let result = match (|| {
        if let (Some(p), Some(ops)) = (&before, a["operations"].as_array()) {
            let plan = ops
                .iter()
                .rev()
                .filter(|op| op["op"] == "set_project")
                .find_map(|op| op["values"].get("composition"))
                .or(p.composition.as_ref());
            ensure!(
            plan.is_some_and(Value::is_object)
                || p.tracks.iter().any(|t| !t.notes.is_empty())
                || !ops.iter().any(|op| op["op"] == "add_notes" || op["op"] == "replace_notes"),
            "COMPOSITION_DESIGN_REQUIRED: read music_knowledge and include the plan in this edit or save it via composition_design before writing notes"
        );
        }
        execute(state, name, a, source, intent.clone())
    })() {
        Ok(result) => result,
        Err(error) => {
            if let Some((_, label)) = activity {
                let reason = if is_revision_conflict(&error) {
                    "工程已更新，操作未执行".to_owned()
                } else {
                    format!("{label}失败")
                };
                state.lock().unwrap().journal.push_with_intent(
                    "agent_activity",
                    json!({"project_id":activity_project,"phase":"failed","reason":reason,"operation":name}),
                    intent.as_ref(),
                );
            }
            return Err(error);
        }
    };
    if source == "agent"
        && [
            "project_create",
            "project_read",
            "project_edit",
            "project_undo",
            "project_redo",
            "project_import",
            "audio_import",
            "composition_design",
            "composition_review",
            "section_contrast",
            "rhythm_interplay",
            "project_export",
            "selection_resolve",
            "selection_edit",
            "arrangement_query",
            "timeline_query",
            "audio_clips",
        ]
        .contains(&name)
    {
        let mut e = crate::presence::focus(name, a, &result, before.as_ref());
        e["phase"] = json!("done");
        if name == "selection_edit" && intent.is_some() && result["dry_run"] == true {
            e["phase"] = json!("review");
        }
        if name == "project_edit" && intent.is_some() {
            let count = a["operations"].as_array().map_or(0, Vec::len);
            e["outcome"] = json!(format!("已保存 {count} 项修改"));
        }
        state
            .lock()
            .unwrap()
            .journal
            .push_with_intent(name, e, intent.as_ref());
    }
    Ok(result)
}

fn forget_range(root: &std::path::Path, id: String) {
    #[cfg(windows)]
    let _ = crate::transport_link::send(
        root,
        &crate::transport_link::AgentCommand::ForgetRange { id },
    );
    #[cfg(not(windows))]
    let _ = (root, id);
}

fn range_audition(state: &Shared, args: &Value, source: &str) -> Result<Value> {
    static PREPARATION: Mutex<()> = Mutex::new(());
    let slot = usize::from(source != "user");
    let project_id = field(args, "project_id")?;
    identifier(project_id)?;
    let expected = revision(args)?;
    let action = field(args, "action")?;
    let is_bound = |manifest: &crate::audition::Manifest| {
        manifest.project_id == project_id && manifest.revision == expected
    };
    if action == "export" {
        let id = field(args, "audition_id")?;
        let (root, manifest, files) = {
            let locked = state.lock().unwrap();
            ensure!(
                locked.store.read(project_id)?.revision == expected,
                "REVISION_CONFLICT: 请重新选择试听范围"
            );
            let (manifest, files) = locked.saved_auditions[slot]
                .as_ref()
                .filter(|(manifest, _)| is_bound(manifest) && manifest.id == id)
                .context("A/B 试听已更换")?;
            (
                locked.store.root.clone(),
                manifest.clone(),
                Arc::clone(files),
            )
        };
        return files.export_pair(&root, &manifest);
    }
    if action == "release" {
        let id = field(args, "audition_id")?;
        let (files, root) = {
            let mut locked = state.lock().unwrap();
            ensure!(
                locked.saved_auditions[slot]
                    .as_ref()
                    .is_some_and(|(manifest, _)| is_bound(manifest) && manifest.id == id),
                "A/B 试听已更换"
            );
            (
                locked.saved_auditions[slot].take().unwrap(),
                locked.store.root.clone(),
            )
        };
        drop(files);
        forget_range(&root, id.to_owned());
        return Ok(json!({"released":true,"id":id}));
    }
    let (project, root) = {
        let locked = state.lock().unwrap();
        let project = locked.store.read(project_id)?;
        ensure!(
            project.revision == expected,
            "REVISION_CONFLICT: 请重新选择试听范围"
        );
        if action == "status" {
            let current = locked.saved_auditions[slot]
                .as_ref()
                .filter(|(manifest, _)| is_bound(manifest));
            return Ok(json!({"pair":current.map(|(manifest,_)|manifest.view())}));
        }
        (project, locked.store.root.clone())
    };
    ensure!(
        action == "prepare",
        "Choose prepare, status, export or release"
    );
    let _worker = PREPARATION
        .try_lock()
        .map_err(|_| anyhow::anyhow!("正在准备另一个 A/B 试听"))?;
    let beat_ranges: [[f64; 2]; 2] = serde_json::from_value(args["beat_ranges"].clone())?;
    let level_match = args
        .get("level_match")
        .map(|value| value.as_bool().context("level_match must be boolean"))
        .transpose()?
        .unwrap_or(true);
    let method = args
        .get("match_method")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or(crate::audition::MatchMethod::Lufs);
    let (files, manifest) = if let Some(recordings) = args.get("recordings") {
        ensure!(
            args.get("job_id").is_none()
                && args.get("renders").is_none()
                && args.get("outputs").is_none(),
            "原录音对照只需片段 ID 和拍区间"
        );
        ensure!(beat_ranges[0] == beat_ranges[1], "两遍录音使用相同的拍区间");
        crate::audition::prepare_recordings(
            &root,
            &project,
            serde_json::from_value(recordings.clone())?,
            beat_ranges[0],
            level_match,
            method,
        )?
    } else {
        let job_id = args
            .get("job_id")
            .map(|value| value.as_str().context("job_id must be a string"))
            .transpose()?;
        let renders: Option<[crate::audition::SavedRender; 2]> = args
            .get("renders")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        ensure!(
            job_id.is_some() != renders.is_some(),
            "使用 job_id 或 renders 中的一项"
        );
        if let Some(sides) = &renders {
            ensure!(
                sides
                    .iter()
                    .any(|side| side.project_id == project.id && side.revision == expected),
                "A/B 至少一边需要匹配当前工程版本"
            );
        }
        let saved = renders
            .as_ref()
            .map(|[a, b]| Ok::<_, anyhow::Error>([a.snapshot(&root)?, b.snapshot(&root)?]))
            .transpose()?;
        let source = crate::audition::SavedSource {
            job_id: job_id.map(str::to_owned),
            renders,
            beat_ranges,
            outputs: args
                .get("outputs")
                .cloned()
                .map(serde_json::from_value)
                .transpose()?,
        };
        if let Some([(first, a), (second, b)]) = &saved {
            crate::audition::prepare_saved(
                &root,
                &project,
                source,
                [(first, a), (second, b)],
                level_match,
                method,
            )?
        } else {
            let result = crate::audition::completed_result(&root, &project, job_id.unwrap())?;
            crate::audition::prepare_saved(
                &root,
                &project,
                source,
                [(&project, &result); 2],
                level_match,
                method,
            )?
        }
    };
    let result = manifest.view();
    let old = {
        let mut locked = state.lock().unwrap();
        ensure!(
            locked.store.read(project_id)?.revision == expected,
            "REVISION_CONFLICT: 工程在准备试听期间更新"
        );
        locked.saved_auditions[slot].replace((manifest, Arc::new(files)))
    };
    if let Some((previous, files)) = old {
        drop(files);
        forget_range(&root, previous.id);
    }
    Ok(result)
}

fn tuning_audition(
    state: &Shared,
    action: &str,
    args: &Value,
    source: &str,
    public: bool,
) -> Result<Value> {
    let slot = usize::from(source != "user");
    let binding = public
        .then(|| Ok::<_, anyhow::Error>((field(args, "project_id")?, revision(args)?)))
        .transpose()?;
    let mut s = state.lock().unwrap();
    let root = s.store.root.clone();
    if action != "prepare" {
        ensure!(
            matches!(action, "status" | "cancel" | "release" | "measure"),
            "Choose prepare, status, cancel, release or measure"
        );
        let id = if public && action == "status" && args.get("audition_id").is_none() {
            None
        } else {
            Some(field(args, "audition_id")?)
        };
        let current = s.tuning_auditions[slot].as_ref().filter(|job| {
            id.is_none_or(|id| job.id == id)
                && binding.is_none_or(|(project, rev)| job.bound(project, rev))
        });
        if action == "status" && id.is_none() && current.is_none() {
            return Ok(json!({"job":null}));
        }
        let current = current.context("TUNING_AUDITION_NOT_FOUND: 调声试听已结束或更换")?;
        match action {
            "status" => {
                let mut status = current.snapshot();
                if let Some((project, rev)) = binding {
                    status["stale"] = json!(s.store.read(project)?.revision != rev);
                }
                return Ok(status);
            }
            "cancel" => return current.control.cancel(),
            "measure" => {
                let ready = current.ready.clone().context("TUNING_AUDITION_NOT_READY")?;
                let base = s.store.read(&current.request.selection.project_id)?;
                ensure!(
                    base.revision == ready.manifest.revision,
                    "REVISION_CONFLICT"
                );
                drop(s);
                let range = args
                    .get("beat_range")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?;
                let (candidate, _) =
                    Store::preview_edit_with_refs(&base, &ready.preview["operations"])?;
                let result =
                    ready
                        .files
                        .measure(&ready.manifest, [&base, &candidate], range, None)?;
                let s = state.lock().unwrap();
                ensure!(
                    s.store.read(&base.id)?.revision == base.revision,
                    "REVISION_CONFLICT: 工程在测量时更新"
                );
                ensure!(
                    s.tuning_auditions[slot]
                        .as_ref()
                        .is_some_and(|job| job.id == ready.manifest.id),
                    "TUNING_AUDITION_NOT_FOUND: 调声试听在测量时更换"
                );
                return Ok(result);
            }
            "release" => {
                let id = current.id.clone();
                let retired = s.tuning_auditions[slot].take();
                drop(s);
                drop(retired);
                forget_range(&root, id.clone());
                return Ok(json!({"released":true,"id":id}));
            }
            _ => unreachable!(),
        }
    }

    let engine = field(args, "engine")?.to_owned();
    ensure!(
        engine::RENDER_ENGINES.contains(&engine.as_str()),
        "选择可用的试听音源"
    );
    let selected = s
        .selections
        .saved
        .get(field(args, "selection_id")?)
        .context("选区已过期")?
        .clone();
    let mut base = s.store.read(&selected.project_id)?;
    ensure!(
        base.revision == revision(args)? && base.revision == selected.revision,
        "REVISION_CONFLICT: 工程已更新，请重新预览"
    );
    ensure!(
        binding.is_none_or(|(project, _)| project == base.id),
        "TUNING_SELECTION_MISMATCH: 选区属于另一个工程"
    );
    crate::selection::selected_vocal_notes(&base, &selected)?;
    let transform = args["transform"].clone();
    let kind = crate::selection::tuning_kind(&transform)?;
    let reference = kind == "pitch_reference";
    let local_voice = args
        .get("local_voice")
        .map(|value| value.as_bool().context("local_voice must be a boolean"))
        .transpose()?
        .unwrap_or(public || reference || kind == "vibrato_envelope");
    let track_id = selected.notes[0].0.clone();
    let intent = args
        .get("intent_id")
        .map(|_| {
            ensure!(slot == 1, "intent_id is for Agent tasks");
            s.journal
                .intent(field(args, "intent_id")?, &base.id, base.revision)
                .context("INTENT_MISMATCH: announce a focus for this project revision")
        })
        .transpose()?;
    let request = TuningRequest {
        selection: selected,
        transform,
        engine,
        local_voice,
    };
    if public {
        if let Some(active) = s.tuning_auditions[slot]
            .as_ref()
            .filter(|job| job.request.same_as(&request))
        {
            let status = active.snapshot();
            if matches!(
                status["status"].as_str(),
                Some("running" | "committing" | "cancelling" | "complete")
            ) {
                return Ok(status);
            }
        }
    }
    ensure!(!s.busy, "RENDER_BUSY: 当前有音频正在渲染");
    if reference && !local_voice {
        for track in &mut base.tracks {
            track.mute = track.id != track_id;
            track.solo = false;
        }
    }

    let jobs = Arc::clone(&s.job_history);
    let id = uid(if slot == 1 {
        "agent_tuning"
    } else {
        "pitch_ab"
    });
    let control = Arc::new(crate::jobs::Control::ephemeral(
        &id,
        &base.id,
        base.revision,
        &request.engine,
    ));
    let status = {
        let job = TuningAudition {
            request: request.clone(),
            id: id.clone(),
            control: Arc::clone(&control),
            ready: None,
        };
        let status = job.snapshot();
        let previous = s.tuning_auditions[slot].replace(job);
        s.busy = true;
        drop(s);
        if let Some(previous) = previous {
            let old_id = previous.id.clone();
            drop(previous);
            forget_range(&root, old_id);
        }
        status
    };

    let state = Arc::clone(state);
    thread::spawn(move || {
        let progress = |phase: &str, reason: &str, range: Option<[f64; 2]>| {
            if slot != 1 {
                return;
            }
            let mut s = state.lock().unwrap();
            if !s.tuning_auditions[slot]
                .as_ref()
                .is_some_and(|job| job.id == id)
            {
                return;
            }
            s.journal.push_with_intent(
                "tuning_audition",
                json!({"project_id":base.id,"revision":base.revision,"track_id":track_id,
                    "note_ids":request.selection.notes.iter().map(|(_, note)|note).collect::<Vec<_>>(),
                    "beat_range":range,"region":"pitch","phase":phase,"reason":reason,"audition_id":id}),
                intent.as_ref(),
            );
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
            progress("rendering", "核对所选音符与调声草稿", None);
            control.check()?;
            let plan = crate::selection::pitch_audition(
                &root,
                &base,
                &request.selection,
                &request.transform,
            )?;
            ensure!(
                state.lock().unwrap().store.read(&base.id)?.revision == base.revision,
                "REVISION_CONFLICT: 工程在准备试听时更新"
            );
            let (files, manifest, _) = crate::audition::prepare(
                crate::audition::Request {
                    audio: crate::audition::AudioRequest {
                        root: &root,
                        base: &base,
                        candidate: &plan.candidate,
                        engine: &request.engine,
                        range: plan.range,
                        compare: reference,
                        jobs: &jobs,
                        output: None,
                        local_voice_track: request.local_voice.then_some(track_id.as_str()),
                    },
                    id: &id,
                    level_match: false,
                    match_method: Default::default(),
                },
                None,
                &control,
                |side| {
                    control.check()?;
                    progress("rendering", side, Some(plan.range));
                    Ok(())
                },
            )?;
            Ok(PreparedTuning {
                files,
                manifest,
                preview: plan.preview,
            })
        }))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("调声试听中断")));

        let mut s = state.lock().unwrap();
        s.busy = false;
        if !s.tuning_auditions[slot]
            .as_ref()
            .is_some_and(|active| active.id == id)
        {
            return;
        }
        let result = result.and_then(|ready| {
            ensure!(
                s.store.read(&base.id)?.revision == base.revision,
                "REVISION_CONFLICT: 工程在试听准备期间更新"
            );
            Ok(ready)
        });
        let mut range = None;
        let delivered = result.map(|ready| {
            range = Some(ready.manifest.beat_range);
            let receipt = ready.manifest.view();
            s.tuning_auditions[slot].as_mut().unwrap().ready = Some(Arc::new(ready));
            receipt
        });
        let _ = control.finish(delivered);
        let completed = control.snapshot();
        drop(s);
        match completed["status"].as_str() {
            Some("complete") => progress("review", "当前歌声和调声草稿已可对照", range),
            Some("cancelled") => progress("cancelled", "调声试听已停止", None),
            _ => progress("failed", "调声试听准备失败", None),
        }
    });
    Ok(status)
}

fn arrangement_audition(state: &Shared, args: &Value, source: &str) -> Result<Value> {
    let slot = usize::from(source != "user");
    let project_id = field(args, "project_id")?;
    let expected = revision(args)?;
    let action = field(args, "action")?;
    let mut s = state.lock().unwrap();
    let root = s.store.root.clone();
    if action != "prepare" {
        let id = if action == "status" && args.get("audition_id").is_none() {
            None
        } else {
            Some(field(args, "audition_id")?)
        };
        let active = s.arrangement_auditions[slot]
            .as_ref()
            .filter(|job| job.bound(project_id, expected) && id.is_none_or(|id| job.id == id));
        if action == "status" && id.is_none() && active.is_none() {
            return Ok(json!({"job":null}));
        }
        let job = active.context("ARRANGEMENT_AUDITION_NOT_FOUND: 编排试听已结束或更换")?;
        return match action {
            "status" => {
                let mut report = job.snapshot();
                report["stale"] = json!(s.store.read(project_id)?.revision != expected);
                Ok(report)
            }
            "cancel" => job.control.cancel(),
            "release" | "apply" => {
                let id = job.id.clone();
                let mut result = json!({"id":id,"released":true});
                if action == "apply" {
                    ensure!(
                        field(args, "preview_id")? == job.request.preview_id,
                        "PREVIEW_MISMATCH: 应用试听对应的准确预览"
                    );
                    let ready = job
                        .ready
                        .as_ref()
                        .context("ARRANGEMENT_AUDITION_NOT_READY")?;
                    let project = s.store.commit_preview(ready.candidate.clone())?;
                    result = ready.summary.clone();
                    result["project"] = json!(project);
                    result["dry_run"] = json!(false);
                    result["audition_id"] = json!(id);
                    result["audition_released"] = json!(true);
                    if slot == 1 {
                        s.journal.push("arrangement_audition", json!({
                            "project_id":project_id,"revision":project.revision,"audition_id":id,
                            "phase":"done","region":"arrangement","reason":"已应用确认的编排试听草稿",
                        }));
                    }
                }
                let retired = s.arrangement_auditions[slot].take();
                drop(s);
                drop(retired);
                forget_range(&root, id);
                Ok(result)
            }
            "measure" => {
                let ready = job
                    .ready
                    .clone()
                    .context("ARRANGEMENT_AUDITION_NOT_READY")?;
                let base = s.store.read(project_id)?;
                ensure!(base.revision == expected, "REVISION_CONFLICT");
                drop(s);
                let range = args
                    .get("beat_range")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?;
                let edges = ready.summary["transition"]
                    .get("frame_range")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?;
                let report = ready.files.measure(
                    &ready.manifest,
                    [&base, &ready.candidate],
                    range,
                    edges,
                )?;
                let s = state.lock().unwrap();
                ensure!(
                    s.store.read(project_id)?.revision == expected,
                    "REVISION_CONFLICT"
                );
                ensure!(
                    s.arrangement_auditions[slot]
                        .as_ref()
                        .is_some_and(|job| job.id == ready.manifest.id),
                    "ARRANGEMENT_AUDITION_NOT_FOUND: 试听在测量期间更换"
                );
                Ok(report)
            }
            _ => bail!("Choose prepare, status, measure, apply, cancel or release"),
        };
    }

    let base = s.store.read(project_id)?;
    ensure!(base.revision == expected, "REVISION_CONFLICT");
    let engine = field(args, "engine")?;
    ensure!(
        engine::RENDER_ENGINES.contains(&engine),
        "选择可用的试听音源"
    );
    let preview_id = field(args, "preview_id")?;
    ensure!(
        preview_id.len() == 64 && preview_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid arrangement preview ID"
    );
    let edit = args["edit"].clone();
    let capture = args
        .get("box_id")
        .map(|_| s.selections.frozen_box(field(args, "box_id")?))
        .transpose()?;
    if let Some(capture) = &capture {
        capture.bound(&base)?;
    }
    let request = ArrangementRequest {
        project_id: project_id.to_owned(),
        revision: expected,
        engine: engine.to_owned(),
        preview_id: preview_id.to_owned(),
        box_id: capture.as_ref().map(|item| item.selection.id.clone()),
        edit: edit.clone(),
        beat_range: args
            .get("beat_range")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?,
        level_match: args
            .get("level_match")
            .map(|value| value.as_bool().context("level_match must be boolean"))
            .transpose()?
            .unwrap_or(false),
    };
    let edit: crate::arrange_audition::Edit = serde_json::from_value(edit)?;
    let intent = args
        .get("intent_id")
        .map(|_| {
            ensure!(slot == 1, "intent_id is for Agent tasks");
            s.journal
                .intent(field(args, "intent_id")?, project_id, expected)
                .context("INTENT_MISMATCH: announce a focus for this project revision")
        })
        .transpose()?;
    if let Some(active) = s.arrangement_auditions[slot]
        .as_ref()
        .filter(|job| job.request == request)
    {
        let snapshot = active.snapshot();
        if matches!(
            snapshot["status"].as_str(),
            Some("running" | "committing" | "cancelling" | "complete")
        ) {
            return Ok(snapshot);
        }
    }
    ensure!(!s.busy, "RENDER_BUSY: 当前有音频正在渲染");
    let id = uid(if slot == 1 {
        "agent_arrange"
    } else {
        "arrange_ab"
    });
    let jobs = Arc::clone(&s.job_history);
    let control = Arc::new(crate::jobs::Control::ephemeral(
        &id, &base.id, expected, engine,
    ));
    let job = ArrangementAudition {
        request: request.clone(),
        id: id.clone(),
        control: Arc::clone(&control),
        ready: None,
    };
    let snapshot = job.snapshot();
    let retired = s.arrangement_auditions[slot].replace(job);
    s.busy = true;
    drop(s);
    if let Some(retired) = retired {
        let old_id = retired.id.clone();
        drop(retired);
        forget_range(&root, old_id);
    }

    let state = Arc::clone(state);
    thread::spawn(move || {
        let progress = |phase: &str, reason: &str, range: [f64; 2], regions: &[Value]| {
            if slot == 1 {
                let mut s = state.lock().unwrap();
                if s.arrangement_auditions[slot]
                    .as_ref()
                    .is_some_and(|job| job.id == id)
                {
                    s.journal.push_with_intent(
                        "arrangement_audition",
                        json!({
                            "project_id":base.id,"revision":base.revision,"regions":regions,
                            "beat_range":range,"region":"arrangement","phase":phase,
                            "reason":reason,"audition_id":id,
                        }),
                        intent.as_ref(),
                    );
                }
            }
        };
        let mut focus = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
            control.check()?;
            let plan = edit.plan(
                &root,
                &base,
                &request.preview_id,
                request.beat_range,
                capture.as_deref(),
            )?;
            ensure!(
                state.lock().unwrap().store.read(&base.id)?.revision == base.revision,
                "REVISION_CONFLICT: 工程在准备试听时更新"
            );
            let (files, manifest, _) = crate::audition::prepare(
                crate::audition::Request {
                    audio: crate::audition::AudioRequest {
                        root: &root,
                        base: &base,
                        candidate: &plan.candidate,
                        engine: &request.engine,
                        range: plan.range,
                        compare: false,
                        jobs: &jobs,
                        output: None,
                        local_voice_track: None,
                    },
                    id: &id,
                    level_match: request.level_match,
                    match_method: crate::audition::MatchMethod::Lufs,
                },
                None,
                &control,
                |label| {
                    control.check()?;
                    progress("rendering", label, plan.range, &plan.regions);
                    Ok(())
                },
            )?;
            focus = Some((plan.range, plan.regions));
            Ok(PreparedArrangement {
                files,
                manifest,
                candidate: plan.candidate,
                summary: plan.summary,
            })
        }))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("编排试听中断")));

        let mut s = state.lock().unwrap();
        s.busy = false;
        if !s.arrangement_auditions[slot]
            .as_ref()
            .is_some_and(|job| job.id == id)
        {
            return;
        }
        let result = result.and_then(|ready| {
            ensure!(
                s.store.read(&base.id)?.revision == base.revision,
                "REVISION_CONFLICT: 工程在试听准备期间更新"
            );
            Ok(ready)
        });
        let delivered = result.map(|ready| {
            let mut result = ready.manifest.view();
            result["preview_id"] = json!(request.preview_id);
            result["transition"] = ready.summary["transition"].clone();
            s.arrangement_auditions[slot].as_mut().unwrap().ready = Some(Arc::new(ready));
            result
        });
        let _ = control.finish(delivered);
        let complete = control.snapshot()["status"] == "complete";
        drop(s);
        if let Some((range, regions)) = focus.filter(|_| complete) {
            progress("review", "当前编排和草稿可 A/B 试听", range, &regions);
        }
    });
    Ok(snapshot)
}

fn edit_heard_tuning(state: &Shared, args: &Value, source: &str) -> Result<Value> {
    let id = field(args, "audition_id")?;
    let selection_id = field(args, "selection_id")?;
    let expected = revision(args)?;
    let slot = usize::from(source != "user");
    let mut s = state.lock().unwrap();
    let job = s.tuning_auditions[slot]
        .as_ref()
        .filter(|job| job.id == id)
        .context("TUNING_AUDITION_NOT_FOUND: 调声试听已结束或更换")?;
    let selection = &job.request.selection;
    ensure!(
        selection.id == selection_id,
        "TUNING_SELECTION_MISMATCH: 请选择试听回执绑定的音符"
    );
    let project = s.store.read(&selection.project_id)?;
    ensure!(
        expected == selection.revision && expected == project.revision,
        "REVISION_CONFLICT"
    );
    ensure!(
        args.get("transform")
            .is_none_or(|transform| *transform == job.request.transform),
        "TUNING_TRANSFORM_MISMATCH: 提交时使用已试听的草稿"
    );
    let ready = job.ready.as_ref().context("TUNING_AUDITION_NOT_READY")?;
    let dry_run = args
        .get("dry_run")
        .map(|value| value.as_bool().context("dry_run must be boolean"))
        .transpose()?
        .unwrap_or(true);
    if dry_run {
        let mut preview = ready.preview.clone();
        preview["project_id"] = json!(project.id);
        preview["audition_id"] = json!(id);
        return Ok(preview);
    }

    let new = s
        .store
        .edit(&project.id, expected, &ready.preview["operations"])?;
    let result = json!({"project":new,"selection":selection.value(),
        "change_count":ready.preview["change_count"],"dry_run":false,
        "audition_id":id,"audition_released":true});
    let root = s.store.root.clone();
    let retired = s.tuning_auditions[slot].take();
    drop(s);
    drop(retired);
    forget_range(&root, id.to_owned());
    Ok(result)
}

#[cfg(windows)]
fn desktop_transport(state: &Shared, args: &Value) -> Result<Value> {
    use crate::transport_link::{self, AgentCommand};
    let action = field(args, "action")?;
    let root = state.lock().unwrap().store.root.clone();
    if action == "status" {
        return transport_link::send(&root, &AgentCommand::Status);
    }
    if action == "devices" {
        return transport_link::send(&root, &AgentCommand::Devices);
    }
    let ticket = field(args, "ticket")?.to_owned();
    let mut event = json!({"region":"transport","transport_action":action});
    let command = match action {
        "play" => {
            let project_id = field(args, "project_id")?;
            let revision = args["expected_revision"]
                .as_u64()
                .context("Missing expected_revision")?;
            let job_id = field(args, "job_id")?;
            let reason = field(args, "reason")?.trim();
            ensure!(
                !reason.is_empty() && reason.chars().count() <= 160,
                "Audition reason must be 1..160 characters"
            );
            let [start, end]: [f64; 2] = serde_json::from_value(args["beat_range"].clone())?;
            ensure!(
                start >= 0. && end > start && end <= 5_000.,
                "Invalid audition beat range"
            );
            let repeat = match args.get("repeat") {
                None => false,
                Some(v) => v.as_bool().context("repeat must be boolean")?,
            };
            let (project, live) = {
                let s = state.lock().unwrap();
                (s.store.read(project_id)?, s.jobs.get(job_id).cloned())
            };
            ensure!(
                project.revision == revision,
                "REVISION_CONFLICT: read the current project before playing"
            );
            let job = match live {
                Some(live) => live.snapshot(),
                None => crate::jobs::read(&root, job_id)?,
            };
            ensure!(
                job["status"] == "complete",
                "RENDER_NOT_COMPLETE: wait for the bound render job"
            );
            let result = &job["result"];
            ensure!(
                result["project_id"] == project_id && result["revision"] == revision,
                "RENDER_REVISION_MISMATCH: choose a render of the current revision"
            );
            let duration = result["seconds"]
                .as_f64()
                .context("Missing render duration")?;
            let timeline = project.timeline();
            let seconds = [timeline.seconds(start), timeline.seconds(end)];
            let frame = |s: f64| (s * 48_000.).round();
            ensure!(
                frame(seconds[0]) < frame(seconds[1]) && frame(seconds[1]) <= frame(duration),
                "PREVIEW_RANGE_OUTSIDE_MIX: select a nonempty range in the rendered mix"
            );
            event["reason"] = json!(reason);
            event["beat_range"] = json!([start, end]);
            event["repeat"] = json!(repeat);
            AgentCommand::Play {
                ticket,
                project_id: project_id.to_owned(),
                revision,
                url: field(result, "mix_url")?.to_owned(),
                seconds,
                repeat,
            }
        }
        "pause" => {
            event["reason"] = json!("暂停 Agent 试听");
            AgentCommand::Pause { ticket }
        }
        "audition_side" => {
            let id = field(args, "audition_id")?;
            let project_id = field(args, "project_id")?;
            let expected = revision(args)?;
            let variant = match args["variant"].as_str() {
                Some("A") => 0,
                Some("B") => 1,
                _ => bail!("Choose A or B"),
            };
            let reason = field(args, "reason")?.trim();
            ensure!(
                !reason.is_empty() && reason.chars().count() <= 160,
                "Audition reason must be 1..160 characters"
            );
            let locked = state.lock().unwrap();
            ensure!(
                locked.store.read(project_id)?.revision == expected,
                "REVISION_CONFLICT: read the current project before playing"
            );
            let manifest = locked.saved_auditions[1]
                .as_ref()
                .map(|(manifest, _)| manifest)
                .filter(|manifest| {
                    manifest.id == id
                        && manifest.project_id == project_id
                        && manifest.revision == expected
                })
                .or_else(|| {
                    locked.tuning_auditions[1]
                        .as_ref()
                        .filter(|job| job.id == id && job.bound(project_id, expected))
                        .and_then(|job| job.ready.as_ref())
                        .map(|ready| &ready.manifest)
                })
                .or_else(|| {
                    locked.arrangement_auditions[1]
                        .as_ref()
                        .filter(|job| job.id == id && job.bound(project_id, expected))
                        .and_then(|job| job.ready.as_ref())
                        .map(|ready| &ready.manifest)
                })
                .context("A/B 试听已更换")?;
            event["reason"] = json!(reason);
            event["beat_range"] = json!(manifest.ranges()[variant]);
            AgentCommand::AuditionSide {
                ticket,
                id: id.to_owned(),
                project_id: project_id.to_owned(),
                revision: expected,
                variant,
            }
        }
        "stop" => {
            event["reason"] = json!("停止 Agent 试听");
            AgentCommand::Stop { ticket }
        }
        _ => bail!("Unknown desktop transport action"),
    };
    let status = transport_link::send(&root, &command)?;
    event["project_id"] = status["project_id"].clone();
    event["revision"] = status["revision"].clone();
    state
        .lock()
        .unwrap()
        .journal
        .push("desktop_transport", event);
    Ok(status)
}
fn ui_clip_refs<'a>(
    project: &Project,
    selections: &'a crate::selection::Workspace,
) -> Result<&'a [crate::clips::ClipRef]> {
    selections.current_ui(project)?;
    Ok(&selections.ui_clips)
}

fn execute(
    state: &Shared,
    name: &str,
    a: &Value,
    source: &str,
    intent: Option<crate::presence::Intent>,
) -> Result<Value> {
    if name == "audio_import" {
        if let Some(path) = a["path"].as_str().map(std::path::Path::new).filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        }) {
            return import_stem_zip(state, a, path);
        }
        ensure!(
            a.get("dry_run").is_none() && a.get("expected_source_sha256").is_none(),
            "Audio preview and source hash are supported for stem ZIP"
        );
        return import_audio(state, a);
    }
    if name == "motif_returns" {
        let project = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        ensure!(
            project.revision == revision(a)?,
            "REVISION_CONFLICT: 请重新选择动机"
        );
        let window = crate::music_review::Window::read(&project, a)?;
        return crate::motif::report(&project, a, &window, None);
    }
    if name == "voicing_preview" {
        let project = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        ensure!(
            project.revision == revision(a)?,
            "REVISION_CONFLICT: 重新读取和弦再试听配置"
        );
        let request = crate::voicing::Request::read(&a["request"])?;
        return crate::voicing::preview(&project, &request, None);
    }
    if name == "harmony_query" {
        let project = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        ensure!(
            project.revision == revision(a)?,
            "REVISION_CONFLICT: 和弦标记已更新"
        );
        let bounds = a.get("beat_range").map(crate::harmony::range).transpose()?;
        return Ok(crate::harmony::query(&project, bounds));
    }
    if name == "rhythm_interplay" {
        let project = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        ensure!(
            project.revision == revision(a)?,
            "REVISION_CONFLICT: 请重新打开鼓贝斯复核"
        );
        let request = crate::rhythm::Request::read(&project, a)?;
        return crate::rhythm::report(&project, &request);
    }
    if name == "section_contrast" {
        let (root, project) = {
            let s = state.lock().unwrap();
            (s.store.root.clone(), s.store.read(field(a, "project_id")?)?)
        };
        ensure!(
            project.revision == revision(a)?,
            "REVISION_CONFLICT: 请重新打开段落对照"
        );
        let request = crate::section_review::Request::read(&project, a)?;
        let mut report = crate::section_review::report(&project, &request);
        let cue = crate::section_review::audio_index(a)?;
        ensure!(
            cue.is_some() == a.get("job_id").is_some(),
            "实音测量需要音频回执和听点编号"
        );
        if let Some(cue) = cue {
            ensure!(
                request
                    .track_ids
                    .as_ref()
                    .is_none_or(|ids| project.tracks.iter().all(|track| ids.contains(&track.id))),
                "整份混音测量须包含全部工程音轨"
            );
            let render = crate::audition::completed_result(&root, &project, field(a, "job_id")?)?;
            crate::section_review::attach_audio(&root, &project, &render, cue, &mut report)?;
        }
        return Ok(report);
    }
    if name == "composition_review" {
        let project = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        if a.get("expected_revision").is_some() {
            ensure!(
                project.revision == revision(a)?,
                "REVISION_CONFLICT: 请重新打开曲谱复核"
            );
        }
        let scope = crate::music_review::Window::read(&project, a)?;
        let mut value = if scope.bounds.is_none() && scope.track_ids.is_none() {
            crate::music::review(&project)
        } else {
            json!({"project_id":project.id,"revision":project.revision,"has_design":project.composition.is_some(),
                "musical_quality_approved":false,"interpretation_not_verdict":true})
        };
        value["score_detail"] = crate::music_review::report(&project, &scope, None, 24);
        return Ok(value);
    }
    #[cfg(windows)]
    if name == "recording_attach" {
        ensure!(source == "user", "录音须由桌面用户确认导入");
        let id = field(a, "project_id")?;
        let rev = revision(a)?;
        let (root, project) = {
            let s = state.lock().unwrap();
            (s.store.root.clone(), s.store.read(id)?)
        };
        let crate::recording::Prepared {
            raw,
            asset,
            track_id,
            start,
            title,
            offset_frames,
        } = crate::recording::prepared(&root, a)?;
        if project
            .tracks
            .iter()
            .any(|track| track.media_clips().iter().any(|clip| clip.asset == asset))
        {
            return Ok(serde_json::to_value(project)?);
        }
        ensure!(
            project.revision == rev,
            "REVISION_CONFLICT: 工程已更新，请确认录音拍位后重试"
        );
        if let Some(track) = &track_id {
            ensure!(
                project
                    .tracks
                    .iter()
                    .any(|item| &item.id == track && item.is_audio()),
                "录音绑定的音轨已不存在"
            );
        }
        let directory = root.join("assets");
        fs::create_dir_all(&directory)?;
        let path = directory.join(&asset);
        let temporary = directory.join(format!("{}.tmp", uid("take_work")));
        let result = (|| -> Result<Value> {
            let seconds = crate::audio::recording_asset(&raw, &temporary, offset_frames)?;
            let clip = AudioClip {
                asset,
                start,
                seconds,
            };
            let s = state.lock().unwrap();
            ensure!(
                s.store.read(id)?.revision == rev,
                "REVISION_CONFLICT: 工程已更新，请确认录音拍位后重试"
            );
            // Replacing an unattached file also recovers a prior interrupted publish.
            crate::model::move_file_windows(&temporary, &path, true)?;
            let saved = match &track_id {
                Some(track) => s.store.append_clip(id, rev, track, clip, &title),
                None => s.store.add_audio(id, rev, clip, &title),
            };
            if saved.is_err() {
                let _ = fs::remove_file(&path);
            }
            Ok(serde_json::to_value(saved?)?)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    if name == "gain_automation_query" {
        let project = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        ensure!(
            project.revision == revision(a)?,
            "REVISION_CONFLICT: 重新读取音量曲线"
        );
        let range = a
            .get("beat_range")
            .map(crate::automation::beat_range)
            .transpose()?;
        return crate::automation::query(&project, field(a, "track_id")?, range);
    }
    if name == "pronunciation_query" {
        let project = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        ensure!(
            project.revision == revision(a)?,
            "REVISION_CONFLICT: 工程已更新，请重新分析子音"
        );
        let ids = crate::engine::pronunciation_note_ids(a)?;
        return crate::engine::pronunciation(&project, field(a, "track_id")?, &ids);
    }
    if name == "audio_clips" || name == "audio_waveform" {
        let (project, root, selected) = {
            let locked = state.lock().unwrap();
            let project = locked.store.read(field(a, "project_id")?)?;
            let selected = if name == "audio_clips" && a["from_ui"] == true {
                Some(ui_clip_refs(&project, &locked.selections)?.to_vec())
            } else {
                None
            };
            (project, locked.store.root.clone(), selected)
        };
        if name == "audio_clips" {
            let mut value = crate::clips::query(&project);
            if let Some(selected) = selected {
                value["ui_clips"] = json!(selected);
            }
            return Ok(value);
        }
        let track = project
            .tracks
            .iter()
            .find(|track| Some(track.id.as_str()) == a["track_id"].as_str())
            .context("Unknown track")?;
        let clips = track.media_clips();
        let selected = match a.get("clip_id") {
            Some(field) => clips
                .iter()
                .find(|clip| Some(clip.id.as_str()) == field.as_str()),
            None if clips.len() == 1 => clips.first(),
            None => None,
        }
        .context("Select a WAV clip explicitly")?;
        if a.get("expected_revision").is_some() {
            ensure!(revision(a)? == project.revision, "REVISION_CONFLICT");
        }
        let range = a
            .get("frame_range")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        return crate::audio::clip_waveform(
            &root,
            selected,
            &project.timeline(),
            range,
            a["include_transients"] == true,
        );
    }
    if name == "audio_clip_group" {
        let action: crate::clips::GroupAction = serde_json::from_value(a["action"].clone())?;
        let id = field(a, "project_id")?;
        let revision = revision(a)?;
        let approved = a
            .get("preview_id")
            .map(|v| v.as_str().context("preview_id must be a string"))
            .transpose()?;
        let mut s = state.lock().unwrap();
        let clips: Vec<crate::clips::ClipRef> = if a["from_ui"] == true {
            ensure!(
                approved.is_none() && a.get("clips").is_none(),
                "Commit with the explicit clips returned by the preview"
            );
            ui_clip_refs(&s.store.read(id)?, &s.selections)?.to_vec()
        } else {
            serde_json::from_value(a["clips"].clone())?
        };
        let result = s
            .store
            .clip_transaction(id, revision, approved, |project| {
                crate::clips::plan_group(&s.store.root, project, &clips, &action)
            })?;
        if source == "agent" {
            let regions =
                crate::clips::focus_regions(&result, approved.is_some()).collect::<Vec<_>>();
            let label = match action {
                crate::clips::GroupAction::Move { .. } => "移动",
                crate::clips::GroupAction::Split { .. } => "切分",
                crate::clips::GroupAction::Copy { .. } => "复制",
                crate::clips::GroupAction::Gain { .. } => "调整增益",
                crate::clips::GroupAction::Slip { .. } => "滑移片段内容",
                crate::clips::GroupAction::Crossfade { .. } => "交叉淡化",
                crate::clips::GroupAction::Tempo { .. } => "设置跟速",
                crate::clips::GroupAction::Warp { .. } => "对齐内部拍点",
                crate::clips::GroupAction::Extract { .. } => "收录选段",
                crate::clips::GroupAction::Remove => "移除",
            };
            let stage = if approved.is_none() { "预览" } else { "已" };
            s.journal.push("audio_clip_group", json!({"project_id":id,"region":"arrangement","regions":regions,"reason":format!("{stage}{label} {} 个音频片段",clips.len())}));
        }
        return Ok(result);
    }
    if name == "audio_clip_edit" {
        let edit: crate::clips::Edit = serde_json::from_value(
            json!({"track_id":a["track_id"],"clip_id":a["clip_id"],"action":a["action"]}),
        )?;
        let preview = a
            .get("preview_id")
            .map(|value| value.as_str().context("preview_id must be a string"))
            .transpose()?;
        let mut locked = state.lock().unwrap();
        let result =
            locked
                .store
                .clip_edit(field(a, "project_id")?, revision(a)?, &edit, preview)?;
        if source == "agent" {
            let target = if preview.is_some() && result["after"][0].is_object() {
                &result["after"][0]
            } else {
                &result["before"]
            };
            let reason = if preview.is_none() {
                "预览音频片段编辑"
            } else {
                match result["action"].as_str() {
                    Some("split") => "切分音频片段",
                    Some("move") => "移动音频片段",
                    Some("copy") => "复制音频片段",
                    Some("trim") => "裁剪音频片段",
                    Some("set") => "调整片段增益与淡化",
                    _ => "移除音频片段",
                }
            };
            locked.journal.push("audio_clip_edit",json!({"project_id":result["project_id"],"region":"arrangement","track_id":target["track_id"],"clip_id":target["clip"]["id"],"beat_range":target["beat_range"],"reason":reason}));
        }
        return Ok(result);
    }
    if name == "arrangement_query" {
        let mut s = state.lock().unwrap();
        let project = s.store.read(field(a, "project_id")?)?;
        if a.get("expected_revision").is_some() {
            ensure!(project.revision == revision(a)?, "REVISION_CONFLICT");
        }
        let capture = if a["from_ui"] == true {
            ensure!(a.get("box_id").is_none(), "Choose from_ui or box_id");
            Some(s.selections.capture_box(&project)?)
        } else {
            a.get("box_id")
                .map(|_| s.selections.frozen_box(field(a, "box_id")?))
                .transpose()?
        };
        drop(s);
        if let Some(capture) = capture {
            capture.bound(&project)?;
            let range = crate::arrangement::BoxRange::read(&project, a, Some(&capture.range))?;
            return capture.view(&project, &range);
        }
        if a.get("beat_range").is_some() || a.get("track_ids").is_some() {
            let range = crate::arrangement::BoxRange::read(&project, a, None)?;
            return Ok(crate::arrangement::inventory(&project, &range, None));
        }
        return Ok(crate::arrangement::query(&project));
    }
    if name == "arrangement_edit" {
        let edit: crate::arrangement::RangeEdit = serde_json::from_value(
            json!({"action":a["action"],"track_ids":a["track_ids"],"source":a["source"],"target_start":a["target_start"],"slice_audio":a.get("slice_audio").cloned().unwrap_or(Value::Bool(false)),"semitones":a["semitones"],"quantize":a["quantize"]}),
        )?;
        let approved = a
            .get("preview_id")
            .map(|v| v.as_str().context("preview_id must be a string"))
            .transpose()?;
        let mut state = state.lock().unwrap();
        let capture = a
            .get("box_id")
            .map(|_| state.selections.frozen_box(field(a, "box_id")?))
            .transpose()?;
        let result = state.store.arrangement_edit(
            field(a, "project_id")?,
            revision(a)?,
            &edit,
            approved,
            capture.as_deref(),
        )?;
        if source == "agent" {
            let window = if approved.is_some() && !result["destination"].is_null() {
                &result["destination"]
            } else {
                &result["source"]
            };
            let reason = match (result["dry_run"].as_bool(), result["action"].as_str()) {
                (Some(true), Some("quantize")) => "检查框内各轨的起音偏差和摇摆",
                (Some(true), Some("transpose")) => "核对所选歌声与乐器的移调音域",
                (Some(true), _) => "核对多轨音符与音频片段",
                (_, Some("copy")) => "复制多轨乐句",
                (_, Some("move")) => "移动多轨乐句",
                (_, Some("quantize")) => "对齐所选歌声、乐器和鼓的起音",
                (_, Some("transpose")) => "移调所选歌声与乐器",
                _ => "清除所选音符与音频片段",
            };
            let regions = result["tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|track| json!({"track_id":track["track_id"],"beat_range":window}))
                .collect::<Vec<_>>();
            state.journal.push("arrangement_edit",json!({"project_id":result["project_id"],"region":"arrangement","track_id":result["tracks"][0]["track_id"],"beat_range":window,"regions":regions,"reason":reason}));
        }
        return Ok(result);
    }
    if name == "timeline_query" {
        let p = state.lock().unwrap().store.read(field(a, "project_id")?)?;
        let clock = p.timeline();
        let mut beats = vec![];
        let mut seconds = vec![];
        for (key, out) in [("beats", &mut beats), ("seconds", &mut seconds)] {
            if let Some(values) = a.get(key) {
                let values = values
                    .as_array()
                    .context("Timeline query values must be arrays")?;
                ensure!(values.len() <= 256, "Timeline query exceeds 256 values");
                for v in values {
                    let n = v.as_f64().context("Timeline query needs numbers")?;
                    ensure!(
                        n.is_finite() && n >= 0. && n <= if key == "beats" { 50000. } else { 900. },
                        "Timeline query outside supported range"
                    );
                    out.push(clock.query(if key == "beats" { n } else { clock.beat(n) }));
                }
            }
        }
        let mut result = json!({"project_id":p.id,"revision":p.revision,"base_bpm":p.bpm,"base_meter":p.meter,"tempo_map":p.tempo_map,"meter_map":p.meter_map,"beat_positions":beats,"second_positions":seconds,"time_unit":"quarter-note beats","meter_changes_start_new_bar":true});
        if let Some(range) = a.get("pulse_range") {
            let range: [f64; 2] = serde_json::from_value(range.clone())?;
            let [start, end] = range;
            ensure!(
                -8. <= start
                    && start < end
                    && end <= 50_000.
                    && clock.seconds_with_preroll(end) - clock.seconds_with_preroll(start) <= 45.,
                "节拍范围须在 -8 到 50000 拍内，最长 45 秒"
            );
            result["metronome_pulses"] = json!(clock.pulses(range));
        }
        return Ok(result);
    }
    if name == "instrument_catalog" {
        let root = state.lock().unwrap().store.root.clone();
        return crate::instruments::catalog(&root);
    }
    if name == "lyrics_reading" {
        return crate::lyrics::reading(a);
    }
    if name == "selection_edit" {
        if a.get("audition_id").is_some() {
            return edit_heard_tuning(state, a, source);
        }
        let (selected, p, root) = {
            let s = state.lock().unwrap();
            let selected = s
                .selections
                .saved
                .get(field(a, "selection_id")?)
                .context("Unknown/expired selection")?
                .clone();
            let p = s.store.read(&selected.project_id)?;
            ensure!(p.revision == revision(a)?, "REVISION_CONFLICT");
            (selected, p, s.store.root.clone())
        };
        let mut pre = crate::selection::preview_with_audio(&root, &p, &selected, &a["transform"])?;
        pre["project_id"] = json!(p.id);
        if a["dry_run"].as_bool().unwrap_or(true) {
            return Ok(pre);
        }
        let s = state.lock().unwrap();
        let new = if pre["change_count"].as_u64() == Some(0) {
            let current = s.store.read(&p.id)?;
            ensure!(current.revision == p.revision, "REVISION_CONFLICT");
            current
        } else {
            s.store.edit(&p.id, p.revision, &pre["operations"])?
        };
        return Ok(
            json!({"project":new,"selection":selected.value(),"change_count":pre["change_count"],"dry_run":false}),
        );
    }
    if ["selection_resolve", "ui_selection_set"].contains(&name) {
        ensure!(
            name != "ui_selection_set" || source == "user",
            "Only the editor can publish a user selection"
        );
        let mut s = state.lock().unwrap();
        let p = s.store.read(field(a, "project_id")?)?;
        if name == "ui_selection_set" {
            if p.revision != revision(a)? {
                return Ok(
                    json!({"accepted":false,"current_revision":p.revision,"reason":"stale view selection ignored"}),
                );
            }
            let clip_refs = a
                .get("clip_refs")
                .cloned()
                .map(serde_json::from_value::<Vec<crate::clips::ClipRef>>)
                .transpose()?;
            if let Some(refs) = &clip_refs {
                crate::clips::validate_refs(&p, refs)?;
            }
            let arrangement_range = a
                .get("arrangement_range")
                .map(|value| {
                    let range: crate::arrangement::BoxRange =
                        serde_json::from_value(value.clone())?;
                    range.validate(&p)?;
                    Ok::<_, anyhow::Error>(range)
                })
                .transpose()?;
            let mut selected = s.selections.set_ui(&p, &a["note_ids"])?;
            s.selections.ui_clips = clip_refs.unwrap_or_default();
            s.selections.ui_range = arrangement_range;
            selected["clip_refs"] = json!(s.selections.ui_clips);
            selected["arrangement_range"] = json!(s.selections.ui_range);
            selected["accepted"] = json!(true);
            return Ok(selected);
        }
        return Ok(s.selections.resolve(&p, a)?.value());
    }
    if name == "studio_status" {
        let s = state.lock().unwrap();
        let root = s.store.root.clone();
        let busy = s.busy;
        drop(s);
        return Ok(
            json!({"application":"Agent Vocal Studio","version":env!("CARGO_PKG_VERSION"),"language":"Rust","engine":engine::status(),"openutau":crate::openutau::status(&root),"instruments":crate::instruments::status(&root),"root":root,"busy":busy,"tools":tools(),"limits":["MIDI 渐变速度离散输出；原生工程精确保留","歌声每轨单声部，采样乐器支持复音","Piapro 专有调声未迁移","VOICEVOX 是其他歌手，不是初音"]}),
        );
    }
    if name == "project_export" && matches!(a["format"].as_str(), Some("wav" | "wav_stems")) {
        let expected = revision(a)?;
        let job_id = field(a, "mix_job_id")?;
        let (root, project, span) = {
            let current = state.lock().unwrap();
            let project = current.store.read(field(a, "project_id")?)?;
            ensure!(project.revision == expected, "REVISION_CONFLICT");
            ensure!(
                a.get("box_id").is_none() || a.get("beat_range").is_none(),
                "Choose box_id or beat_range"
            );
            let beats = if a.get("box_id").is_some() {
                let box_capture = current.selections.frozen_box(field(a, "box_id")?)?;
                box_capture.bound(&project)?;
                Some(box_capture.range.beat_range)
            } else {
                a.get("beat_range")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?
            };
            let span = beats
                .map(|range| crate::wave_delivery::BeatSpan::new(&project, range))
                .transpose()?;
            (current.store.root.clone(), project, span)
        };
        let render = crate::audition::completed_result(&root, &project, job_id)?;
        if a["format"] == "wav_stems" {
            let delivery = crate::wave_delivery::StemDelivery::new(&root, render, a, span)?;
            let mut s = state.lock().unwrap();
            ensure!(
                s.store.read(&project.id)?.revision == expected,
                "REVISION_CONFLICT"
            );
            let is_delivery = |id: &str| id.starts_with("delivery_");
            s.jobs.retain(|id, job| {
                !is_delivery(id)
                    || !crate::jobs::terminal(job.snapshot()["status"].as_str().unwrap_or(""))
            });
            ensure!(
                !s.jobs.keys().any(|id| is_delivery(id)),
                "DELIVERY_BUSY: 已有分轨正在打包"
            );
            let id = uid("delivery");
            let control = Arc::new(crate::jobs::Control::new(
                &root,
                &id,
                &project.id,
                expected,
                crate::wave_delivery::ENGINE,
            )?);
            s.jobs.insert(id.clone(), control.clone());
            let is_agent = source == "agent";
            if is_agent {
                s.journal.push(
                    "stem_delivery",
                    json!({"project_id":project.id,"revision":expected,"job_id":id,
                    "region":"transport","phase":"rendering","reason":"正在打包对齐分轨"}),
                );
            }
            let history = s.job_history.clone();
            drop(s);
            history.record(&control);
            let snapshot = control.snapshot();
            let state = state.clone();
            thread::spawn(move || {
                let output = std::panic::catch_unwind(|| delivery.run(&root, &id, &control))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("Stem delivery worker failed")));
                if let Err(error) = control.finish(output) {
                    eprintln!("Could not persist stem delivery outcome: {error:#}");
                }
                if is_agent {
                    let done = control.snapshot()["status"] == "complete";
                    state.lock().unwrap().journal.push(
                        "stem_delivery",
                        json!({"project_id":project.id,
                        "job_id":id,"region":"transport","phase":if done{"done"}else{"failed"},
                        "reason":if done{"分轨 ZIP 已就绪"}else{"分轨打包已结束"}}),
                    );
                }
            });
            return Ok(snapshot);
        }
        return crate::wave_delivery::export(&root, &render, a, span);
    }
    if name == "singer_list" {
        return match a["engine"].as_str().unwrap_or("voicevox") {
            "voicevox" => engine::singers(),
            "openutau" => {
                Ok(json!({"profiles":crate::openutau::catalog(&state.lock().unwrap().store.root)}))
            }
            _ => bail!("Unsupported singer catalog"),
        };
    }
    if ["render_status", "render_cancel", "render_jobs"].contains(&name) {
        let s = state.lock().unwrap();
        let root = s.store.root.clone();
        if name == "render_jobs" {
            let busy = s.busy;
            let history = s.job_history.clone();
            let deliveries = a["deliveries_only"] == true;
            ensure!(
                !deliveries
                    || (a.get("engine").is_none()
                        && a.get("expected_revision").is_none()
                        && a["archived_only"] != true),
                "Delivery lookup accepts project_id and active_only"
            );
            if a["archived_only"] == true {
                ensure!(
                    a.get("engine").is_none() && a["active_only"] != true,
                    "Archived reference lookup does not take engine or active_only"
                );
                let project = s.store.read(field(a, "project_id")?)?;
                ensure!(project.revision == revision(a)?, "REVISION_CONFLICT");
                drop(s);
                let job = crate::archived_mix::existing(&root, &project)?;
                return Ok(json!({"busy":busy,"jobs":job.into_iter().collect::<Vec<_>>()}));
            }
            if a.get("expected_revision").is_some() {
                drop(s);
                let job = history.existing_mix(
                    field(a, "project_id")?,
                    revision(a)?,
                    field(a, "engine")?,
                )?;
                return Ok(json!({"busy":busy,"jobs":job.into_iter().collect::<Vec<_>>()}));
            }
            let live = s
                .jobs
                .iter()
                .map(|(id, j)| (id.clone(), j.clone()))
                .collect::<Vec<_>>();
            drop(s);
            let project = a["project_id"].as_str();
            let mut jobs = if a["active_only"] == true {
                vec![]
            } else {
                history.recent(project, deliveries)?
            };
            for (id, control) in live {
                let value = control.snapshot();
                if (value["engine"] == crate::wave_delivery::ENGINE) != deliveries
                    || project.is_some_and(|p| value["project_id"] != p)
                {
                    continue;
                }
                if let Some(existing) = jobs.iter_mut().find(|v| v["job_id"] == id) {
                    *existing = value;
                } else if !crate::jobs::terminal(value["status"].as_str().unwrap_or("")) {
                    jobs.insert(0, value);
                }
            }
            let busy = if deliveries {
                jobs.iter()
                    .any(|job| !crate::jobs::terminal(job["status"].as_str().unwrap_or("")))
            } else {
                busy
            };
            return Ok(json!({"busy":busy,"jobs":jobs}));
        }
        let id = field(a, "job_id")?;
        identifier(id)?;
        let control = s.jobs.get(id).cloned();
        drop(s);
        return if let Some(control) = control {
            if name == "render_cancel" {
                let value = control.cancel()?;
                if source == "agent" && value["accepted"] == true {
                    let reason = if value["job"]["engine"] == crate::wave_delivery::ENGINE {
                        "等待分轨打包停止"
                    } else {
                        "等待渲染停止"
                    };
                    state.lock().unwrap().journal.push("render_cancel",json!({"project_id":value["job"]["project_id"],"job_id":id,"region":"transport","phase":"rendering","reason":reason}));
                }
                Ok(value)
            } else {
                Ok(control.snapshot())
            }
        } else {
            let value = crate::jobs::read(&root, id)?;
            if name == "render_cancel" {
                Ok(json!({"accepted":false,"job":value,"reason":"No live worker owns this job"}))
            } else {
                Ok(value)
            }
        };
    }
    if name == "render_start" {
        let prepare_monitor = a["prepare_monitor"] == true;
        let mut s = state.lock().unwrap();
        ensure!(!s.busy, "RENDER_BUSY: wait for current job");
        let p = s.store.read(field(a, "project_id")?)?;
        ensure!(p.revision == revision(a)?, "REVISION_CONFLICT");
        ensure!(p.seconds() > 0., "Project contains no notes");
        let mode = field(a, "engine")?.to_string();
        ensure!(
            engine::RENDER_ENGINES.contains(&mode.as_str()),
            "Unsupported engine"
        );
        let id = uid("render");
        let control = Arc::new(crate::jobs::Control::new(
            &s.store.root,
            &id,
            &p.id,
            p.revision,
            &mode,
        )?);
        if s.jobs.len() >= 128 {
            s.jobs.retain(|_, j| {
                !crate::jobs::terminal(j.snapshot()["status"].as_str().unwrap_or(""))
            });
        }
        s.jobs.insert(id.clone(), control.clone());
        s.busy = true;
        let root = s.store.root.clone();
        let history = s.job_history.clone();
        let is_agent = source == "agent";
        if is_agent {
            s.journal.push_with_intent("render_start", json!({"project_id":p.id,"revision":p.revision,"job_id":id,"region":"transport","phase":"rendering","reason":"开始离线渲染"}), intent.as_ref());
        }
        drop(s);
        history.record(&control);
        let st = state.clone();
        let jid = id.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                engine::render(&root, &p, &jid, &mode, prepare_monitor, &control)
            });
            let result = match result {
                Ok(value) => value,
                Err(_) => Err(anyhow::anyhow!("Renderer panicked")),
            };
            if let Err(error) = control.finish(result) {
                eprintln!("Could not persist render outcome: {error:#}");
            }
            if let Some(published) = control.snapshot().get("result") {
                if let Err(error) = crate::jobs::remember_mix(&root, published) {
                    eprintln!("Could not update the mix cache: {error:#}");
                }
            }
            let mut s = st.lock().unwrap();
            s.busy = false;
            if is_agent {
                let status = control.snapshot()["status"]
                    .as_str()
                    .unwrap_or("failed")
                    .to_string();
                s.journal.push_with_intent("render_finished",json!({"project_id":p.id,"revision":p.revision,"region":"transport","reason":match status.as_str(){"complete"=>"试听音频已渲染","cancelled"=>"渲染已停止",_=>"渲染未完成"},"phase":if ["complete","cancelled"].contains(&status.as_str()){"done"}else{"failed"},"job_id":jid,"status":status}),intent.as_ref());
            }
        });
        return Ok(json!({"job_id":id,"status":"running"}));
    }
    if name == "project_list" {
        let store = state.lock().unwrap().store.clone();
        let query: crate::project_index::Query = serde_json::from_value(a.clone())?;
        return store.list(&query);
    }
    if name == "project_import" {
        let dry_run = a
            .get("dry_run")
            .map(|value| value.as_bool().context("dry_run must be a boolean"))
            .transpose()?
            .unwrap_or(false);
        let expected_source_sha256 = a
            .get("expected_source_sha256")
            .map(|value| {
                value
                    .as_str()
                    .context("expected_source_sha256 must be a string")
            })
            .transpose()?;
        let reference_audio = a
            .get("reference_audio")
            .map(|value| serde_json::from_value::<exchange::PpsfAudio>(value.clone()))
            .transpose()?;
        let (filename, bytes) = input_file(a, 128_000_000)?;
        let store = state.lock().unwrap().store.clone();
        return exchange::import(
            &store,
            &filename,
            &bytes,
            dry_run,
            expected_source_sha256,
            reference_audio,
        );
    }
    let s = state.lock().unwrap();
    match name {
        "composition_design" => {
            crate::music::validate_plan(&a["plan"])?;
            Ok(serde_json::to_value(s.store.edit(
                field(a, "project_id")?,
                revision(a)?,
                &json!([{"op":"set_project","values":{"composition":a["plan"]}}]),
            )?)?)
        }
        "project_create" => Ok(serde_json::to_value(s.store.create(
            a["title"].as_str().unwrap_or("未命名工程"),
            a["bpm"].as_f64().unwrap_or(120.),
            if a.get("meter").is_some() {
                serde_json::from_value(a["meter"].clone())?
            } else {
                [4, 4]
            },
            vec![Track::new("主唱")],
        )?)?),
        "project_read" => Ok(serde_json::to_value(
            s.store.read(field(a, "project_id")?)?,
        )?),
        "project_preview" => {
            let base = s.store.read(field(a, "project_id")?)?;
            ensure!(base.revision == revision(a)?, "REVISION_CONFLICT");
            let window = crate::music_review::Window::read(&base, a)?;
            let candidate = Store::preview_edit(&base, &a["operations"])?;
            let mut result = json!({"project_id":base.id,"revision":base.revision,"dry_run":true,
                "score_changes":crate::draft_changes::report(&base,&candidate,&window,None)});
            if a["operations"].as_array().is_some_and(|ops| {
                ops.iter().any(|op| {
                    op["op"] == "set_project" && op["values"].get("section_markers").is_some()
                })
            }) {
                result["section_changes"] = json!({
                    "before":crate::arrangement::section_markers(&base),
                    "after":crate::arrangement::section_markers(&candidate),
                    "from_design":candidate.section_markers.is_none()
                });
            }
            Ok(result)
        }
        "project_edit" => Ok(serde_json::to_value(s.store.edit(
            field(a, "project_id")?,
            revision(a)?,
            &a["operations"],
        )?)?),
        "project_undo" | "project_redo" => Ok(serde_json::to_value(s.store.undo(
            field(a, "project_id")?,
            revision(a)?,
            name == "project_redo",
        )?)?),
        "project_recover" => Ok(serde_json::to_value(
            s.store
                .recover(field(a, "project_id")?, a["expected_revision"].as_u64())?,
        )?),
        "project_export" => {
            let p = s.store.read(field(a, "project_id")?)?;
            let format = field(a, "format")?;
            ensure!(
                ["avpack", "avproj", "mid"].contains(&format),
                "Unsupported format"
            );
            let mix_job_id = a
                .get("mix_job_id")
                .map(|id| id.as_str().context("mix_job_id must be a string"))
                .transpose()?;
            if mix_job_id.is_some() {
                ensure!(format == "avpack", "A reference mix requires avpack");
                ensure!(p.revision == revision(a)?, "REVISION_CONFLICT");
            }
            let bytes = match format {
                "mid" => exchange::midi(&p)?,
                "avpack" => exchange::bundle(&s.store, &p, mix_job_id)?,
                _ => serde_json::to_vec_pretty(&p)?,
            };
            let id = uid("export");
            let dir = s.store.root.join("exports").join(&id);
            fs::create_dir_all(&dir)?;
            let filename = format!("工程_r{}.{}", p.revision, format);
            let path = dir.join(&filename);
            fs::write(&path, bytes)?;
            Ok(
                json!({"path":path,"url":format!("/exports/{id}/{filename}"),"revision":p.revision,"warning":if format=="mid"{"MIDI 保留变速、拍号和段落标记的起点。段落终点、调声、跨音拖腔、音频和混音效果器请用工程包保存。"}else if format=="avproj"{"JSON 工程的伴奏仍引用本机素材，跨电脑请导出 avpack。"}else{""}}),
            )
        }
        _ => bail!("Unknown tool: {name}"),
    }
}
fn revision(a: &Value) -> Result<u64> {
    a["expected_revision"]
        .as_u64()
        .context("expected_revision is required")
}
fn input_file(a: &Value, limit: u64) -> Result<(String, Vec<u8>)> {
    let (name, bytes) = if let Some(path) = a["path"].as_str() {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute(), "Import path must be absolute");
        ensure!(
            fs::metadata(&path)?.len() <= limit,
            "File exceeds {} MB",
            limit / 1_000_000
        );
        (
            path.file_name()
                .context("Missing filename")?
                .to_string_lossy()
                .to_string(),
            fs::read(path)?,
        )
    } else {
        (
            field(a, "filename")?.to_string(),
            STANDARD.decode(field(a, "data_base64")?)?,
        )
    };
    ensure!(bytes.len() as u64 <= limit, "Import file too large");
    Ok((name, bytes))
}

fn import_stem_zip(state: &Shared, args: &Value, path: &std::path::Path) -> Result<Value> {
    ensure!(
        ["track_id", "filename", "data_base64", "name"]
            .iter()
            .all(|key| args.get(key).is_none()),
        "分轨 ZIP 会为每条音频新建音轨"
    );
    let id = field(args, "project_id")?;
    let expected = revision(args)?;
    let preview = args
        .get("dry_run")
        .map(|flag| flag.as_bool().context("dry_run must be boolean"))
        .transpose()?
        .unwrap_or(false);
    let explicit = args
        .get("start")
        .map(|beat| beat.as_f64().context("start must be a number"))
        .transpose()?;
    let sha = args
        .get("expected_source_sha256")
        .map(|hash| hash.as_str().context("Hash must be a string"))
        .transpose()?;
    let (root, project) = {
        let locked = state.lock().unwrap();
        let project = locked.store.read(id)?;
        ensure!(project.revision == expected, "REVISION_CONFLICT");
        (locked.store.root.clone(), project)
    };
    let mut archive = crate::stem_import::StemArchive::open(path, preview || sha.is_some())?;
    if let Some(hash) = sha {
        archive.verify_sha256(hash)?;
    }
    ensure!(
        project.tracks.len() + archive.len() <= TRACK_LIMIT,
        "导入分轨后最多可有 32 条音轨"
    );
    let start = archive.start(explicit);
    ensure!(
        (0. ..=50_000.).contains(&start)
            && project.timeline().seconds(start) + archive.frames() as f64 / 48000. <= 900.,
        "导入分轨超出工程时间范围"
    );
    if preview {
        let mut info = archive.preview(start)?;
        info["project_id"] = json!(id);
        info["revision"] = json!(expected);
        return Ok(info);
    }
    let staged = archive.decode(&root, start)?;
    let saved = state
        .lock()
        .unwrap()
        .store
        .add_stem_tracks(id, expected, &staged.tracks)?;
    staged.committed();
    Ok(serde_json::to_value(saved)?)
}

fn import_audio(state: &Shared, args: &Value) -> Result<Value> {
    use symphonia::core::io::MediaSource;
    let project_id = field(args, "project_id")?;
    let expected = revision(args)?;
    let track = args
        .get("track_id")
        .map(|value| value.as_str().context("track_id must be a string"))
        .transpose()?;
    let start = args
        .get("start")
        .map(|value| value.as_f64().context("start must be a number"))
        .transpose()?
        .unwrap_or(0.);
    ensure!(start.is_finite() && start >= 0., "Invalid audio placement");
    let root = {
        let locked = state.lock().unwrap();
        let project = locked.store.read(project_id)?;
        ensure!(expected == project.revision, "REVISION_CONFLICT");
        if let Some(id) = track {
            ensure!(
                project.tracks.iter().any(|t| t.id == id && t.is_audio()),
                "Target must be an existing audio track"
            );
        }
        locked.store.root.clone()
    };
    let (filename, source): (String, Box<dyn MediaSource>) =
        if let Some(path) = args["path"].as_str() {
            let path = std::path::Path::new(path);
            ensure!(path.is_absolute(), "Import path must be absolute");
            let name = path
                .file_name()
                .context("Missing filename")?
                .to_string_lossy()
                .into_owned();
            let file = fs::File::open(path)?;
            ensure!(file.metadata()?.is_file(), "Select an audio file");
            (name, Box::new(file))
        } else {
            let (filename, bytes) = input_file(args, 128_000_000)?;
            (filename, Box::new(std::io::Cursor::new(bytes)))
        };
    let clip = crate::audio::ingest(&root, source, &filename, start)?;
    let name = args["name"]
        .as_str()
        .or_else(|| {
            std::path::Path::new(&filename)
                .file_stem()
                .and_then(|stem| stem.to_str())
        })
        .unwrap_or(&filename);
    let asset = clip.asset.clone();
    let result = {
        let locked = state.lock().unwrap();
        if let Some(track_id) = track {
            locked
                .store
                .append_clip(project_id, expected, track_id, clip, name)
        } else {
            locked.store.add_audio(project_id, expected, clip, name)
        }
    };
    if result.is_err() {
        let _ = fs::remove_file(root.join("assets").join(asset));
    }
    Ok(serde_json::to_value(result?)?)
}
pub(crate) fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).unwrap()
}
pub(crate) fn response(data: Vec<u8>, mime: &str, status: u16) -> Response<CursorReader> {
    Response::new(StatusCode(status),vec![header("Content-Type",mime),header("Cache-Control","no-store"),header("X-Content-Type-Options","nosniff"),header("Content-Security-Policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; media-src 'self' blob:; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'")],std::io::Cursor::new(data.clone()),Some(data.len()),None)
}
type CursorReader = std::io::Cursor<Vec<u8>>;
pub(crate) fn unescape(s: &str) -> Result<String> {
    let b = s.as_bytes();
    let mut v = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            ensure!(i + 2 < b.len(), "Bad URL");
            v.push(u8::from_str_radix(
                std::str::from_utf8(&b[i + 1..i + 3])?,
                16,
            )?);
            i += 3
        } else {
            v.push(b[i]);
            i += 1
        }
    }
    Ok(String::from_utf8(v)?)
}
pub(crate) fn editor_asset(url: &str, token: &str) -> Result<(Vec<u8>, &'static str)> {
    Ok(match url {
        "/" => (
            include_str!("../web/index.html")
                .replace("__TOKEN__", token)
                .into_bytes(),
            "text/html; charset=utf-8",
        ),
        "/timeline.js" => (
            include_bytes!("../web/timeline.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/studio.js" => (
            include_bytes!("../web/studio.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/effects.js" => (
            include_bytes!("../web/effects.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/pico-grey.css" => (
            include_bytes!("../web/pico-grey.css").to_vec(),
            "text/css; charset=utf-8",
        ),
        "/studio.css" => (
            include_bytes!("../web/studio.css").to_vec(),
            "text/css; charset=utf-8",
        ),
        "/fonts/InterVariable.woff2" => (
            include_bytes!("../web/fonts/InterVariable.woff2").to_vec(),
            "font/woff2",
        ),
        "/fonts/NotoSansSC.woff2" => (
            include_bytes!("../web/fonts/NotoSansSC.woff2").to_vec(),
            "font/woff2",
        ),
        "/workspace.js" => (
            include_bytes!("../web/workspace.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/workspace.css" => (
            include_bytes!("../web/workspace.css").to_vec(),
            "text/css; charset=utf-8",
        ),
        "/transport.js" => (
            include_bytes!("../web/transport.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/timeline-ui.js" => (
            include_bytes!("../web/timeline-ui.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/app.js" => (
            include_bytes!("../web/app.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/pro.js" => (
            include_bytes!("../web/pro.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/agent.js" => (
            include_bytes!("../web/agent.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/mixer.js" => (
            include_bytes!("../web/mixer.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/routing.js" => (
            include_bytes!("../web/routing.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/gain-automation.js" => (
            include_bytes!("../web/gain-automation.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/harmony.js" => (
            include_bytes!("../web/harmony.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/recording.js" => (
            include_bytes!("../web/recording.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/agent.css" => (
            include_bytes!("../web/agent.css").to_vec(),
            "text/css; charset=utf-8",
        ),
        "/apple.css" => (
            include_bytes!("../web/apple.css").to_vec(),
            "text/css; charset=utf-8",
        ),
        "/design.js" => (
            include_bytes!("../web/design.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/motion.js" => (
            include_bytes!("../web/motion.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/instruments.js" => (
            include_bytes!("../web/instruments.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/jobs.js" => (
            include_bytes!("../web/jobs.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/clip-view.js" => (
            include_bytes!("../web/clip-view.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/clip-ui.js" => (
            include_bytes!("../web/clip-ui.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/warp.js" => (
            include_bytes!("../web/warp.js").to_vec(),
            "text/javascript; charset=utf-8",
        ),
        "/take-review.js" => (
            include_bytes!("../web/take-review.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/arrangement.js" => (
            include_bytes!("../web/arrangement.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/arrangement.css" => (
            include_bytes!("../web/arrangement.css").to_vec(),
            "text/css; charset=utf-8",
        ),
        "/commands.js" => (
            include_bytes!("../web/commands.js").to_vec(),
            "application/javascript; charset=utf-8",
        ),
        "/style.css" => (
            include_bytes!("../web/style.css").to_vec(),
            "text/css; charset=utf-8",
        ),
        "/favicon.ico" => (
            include_bytes!("../resources/app.ico").to_vec(),
            "image/x-icon",
        ),
        _ => anyhow::bail!("Not found"),
    })
}

fn serve(root: PathBuf, development_ui: bool) -> Result<()> {
    fs::create_dir_all(root.join("runtime"))?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("runtime/service.lock"))?;
    lock.try_lock()
        .context("Studio already serves this workspace")?;
    let bind = if development_ui {
        DEVELOPMENT_ADDRESS
    } else {
        "127.0.0.1:0"
    };
    let server = Server::http(bind).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let Some(std::net::SocketAddr::V4(address)) = server.server_addr().to_ip() else {
        bail!("Studio requires an IPv4 loopback address");
    };
    let token = uid("session");
    let instance_id = uid("server");
    let job_history = Arc::new(crate::jobs::History::new(&root));
    let shared = Arc::new(Mutex::new(State {
        store: Store::new(root.clone())?,
        job_history: job_history.clone(),
        jobs: HashMap::new(),
        busy: false,
        journal: crate::presence::Journal::default(),
        selections: crate::selection::Workspace::default(),
        saved_auditions: [None, None],
        tuning_auditions: [None, None],
        arrangement_auditions: [None, None],
    }));
    let session = ServiceSession {
        address,
        token: token.clone(),
        pid: std::process::id(),
        instance_id: Some(instance_id.clone()),
    };
    let phase = root.join("runtime").join(format!("{instance_id}.json.tmp"));
    fs::write(&phase, serde_json::to_vec(&session)?)?;
    let published = root.join(SESSION_FILE);
    #[cfg(windows)]
    crate::model::replace_windows(&phase, &published)?;
    #[cfg(not(windows))]
    fs::rename(&phase, &published)?;
    thread::spawn(move || {
        if let Err(error) = job_history.warm() {
            eprintln!("Could not warm render history: {error:#}");
        }
    });
    eprintln!(
        "Agent Vocal Studio {} http://{}",
        env!("CARGO_PKG_VERSION"),
        address
    );
    let address = address.to_string();
    let localhost = format!("localhost:{}", session.address.port());
    let origin = format!("http://{address}");
    let localhost_origin = format!("http://{localhost}");
    for mut req in server.incoming_requests() {
        let host = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Host"))
            .map(|h| h.value.as_str())
            .unwrap_or("");
        if host != address && host != localhost {
            let _ = req.respond(response(b"Forbidden host".to_vec(), "text/plain", 403));
            continue;
        }
        let url = req.url().split('?').next().unwrap_or("").to_string();
        if req.method().as_str() == "GET" || req.method().as_str() == "HEAD" {
            let root = root.clone();
            let token = token.clone();
            let instance_id = instance_id.clone();
            thread::spawn(move || {
                if url.starts_with("/exports/") {
                    crate::http_audio::serve(req, &root, &url);
                    return;
                }
                let result = if url == "/health" {
                    serde_json::to_vec(&json!({"application":"Agent Vocal Studio","version":env!("CARGO_PKG_VERSION"),"root":root,"pid":std::process::id(),"instance_id":instance_id})).map(|bytes| (bytes,"application/json")).map_err(Into::into)
                } else if development_ui {
                    editor_asset(&url, &token)
                } else {
                    Err(anyhow::anyhow!("Open the desktop application"))
                };
                let _ = req.respond(match result {
                    Ok((d, m)) => response(d, m, 200),
                    Err(_) => response(b"Not found".to_vec(), "text/plain", 404),
                });
            });
            continue;
        }
        if url != "/rpc" || req.method().as_str() != "POST" {
            let _ = req.respond(response(vec![], "text/plain", 404));
            continue;
        }
        let auth = req
            .headers()
            .iter()
            .any(|h| h.field.equiv("X-Agent-Vocal-Token") && h.value.as_str() == token);
        let content = req.headers().iter().any(|h| {
            h.field.equiv("Content-Type") && h.value.as_str().starts_with("application/json")
        });
        let origin_ok = req
            .headers()
            .iter()
            .filter(|h| h.field.equiv("Origin"))
            .all(|h| h.value.as_str() == origin || h.value.as_str() == localhost_origin);
        if !auth || !content || !origin_ok {
            let _ = req.respond(response(b"Forbidden".to_vec(), "text/plain", 403));
            continue;
        }
        let st = shared.clone();
        thread::spawn(move || {
            let result: Result<Value> = (|| {
                let mut body = String::new();
                req.as_reader()
                    .take(180_000_001)
                    .read_to_string(&mut body)?;
                ensure!(body.len() <= 180_000_000, "Request too large");
                let v: Value = serde_json::from_str(&body)?;
                call(
                    &st,
                    field(&v, "name")?,
                    &v["arguments"],
                    v["source"].as_str().unwrap_or("agent"),
                )
            })();
            let (v, status) = match result {
                Ok(data) => (json!({"ok":true,"data":data}), 200),
                Err(e) => {
                    let msg = format!("{e:#}");
                    let status = if is_revision_conflict(&e) { 409 } else { 400 };
                    (json!({"ok":false,"error":msg}), status)
                }
            };
            let _ = req.respond(response(
                serde_json::to_vec(&v).unwrap(),
                "application/json; charset=utf-8",
                status,
            ));
        });
    }
    Ok(())
}
fn ensure_server(root: &PathBuf) -> Result<ServiceSession> {
    let ready = || -> Result<ServiceSession> {
        let session: ServiceSession =
            serde_json::from_reader(fs::File::open(root.join(SESSION_FILE))?.take(4096))?;
        ensure!(
            *session.address.ip() == std::net::Ipv4Addr::LOCALHOST && session.address.port() != 0,
            "Studio session has an invalid loopback address"
        );
        let v: Value = ureq::get(&format!("http://{}/health", session.address))
            .config()
            .timeout_global(Some(Duration::from_secs(2)))
            .build()
            .call()?
            .body_mut()
            .read_json()?;
        ensure!(
            v["application"] == "Agent Vocal Studio",
            "Studio session address belongs to another application"
        );
        let server_root =
            PathBuf::from(v["root"].as_str().context("Missing server root")?).canonicalize()?;
        ensure!(
            server_root == root.canonicalize()?,
            "Another Studio instance is serving a different home"
        );
        if let Some(instance) = &session.instance_id {
            ensure!(
                v["instance_id"] == instance.as_str() && v["pid"] == session.pid,
                "Studio session was replaced"
            );
        } else {
            ensure!(
                session.address.to_string() == DEVELOPMENT_ADDRESS && v["instance_id"].is_null(),
                "Invalid legacy Studio session"
            );
        }
        Ok(session)
    };
    if let Ok(v) = ready() {
        return Ok(v);
    };
    let exe = std::env::current_exe()?.with_file_name(format!(
        "agent-vocal-studio{}",
        std::env::consts::EXE_SUFFIX
    ));
    let mut cmd = Command::new(exe);
    cmd.args(["--serve", "--home"])
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.spawn()?;
    let start = Instant::now();
    loop {
        if let Ok(v) = ready() {
            return Ok(v);
        };
        ensure!(
            start.elapsed() < Duration::from_secs(15),
            "Could not start Studio for {}",
            root.display()
        );
        thread::sleep(Duration::from_millis(100));
    }
}
pub(crate) fn proxy(root: &PathBuf, name: &str, args: &Value) -> Result<Value> {
    proxy_source(root, name, args, "agent")
}

const REVISION_CONFLICT: &str = "REVISION_CONFLICT";

fn is_revision_conflict(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message == REVISION_CONFLICT || message.starts_with("REVISION_CONFLICT:")
    })
}

#[derive(Debug)]
struct RemoteRevisionConflict(String);

impl std::fmt::Display for RemoteRevisionConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RemoteRevisionConflict {}

pub(crate) fn proxy_source(
    root: &PathBuf,
    name: &str,
    args: &Value,
    source: &str,
) -> Result<Value> {
    let session = ensure_server(root)?;
    let timeout = if matches!(name, "audio_import" | "recording_attach")
        || (name == "project_export" && args["format"] == "wav")
    {
        180
    } else {
        30
    };
    let mut response = ureq::post(&format!("http://{}/rpc", session.address))
        .header("X-Agent-Vocal-Token", &session.token)
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(timeout)))
        .build()
        .send_json(json!({"name":name,"arguments":args,"source":source}))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let status = response.status();
    let v: Value = response
        .body_mut()
        .with_config()
        .limit(44_000_000)
        .read_json()?;
    if status == 409 && v["ok"] == false {
        let message = v["error"].as_str().unwrap_or(REVISION_CONFLICT);
        return Err(RemoteRevisionConflict(message.to_owned()).into());
    }
    ensure!(v["ok"] == true, "{}", v["error"]);
    Ok(v["data"].clone())
}
fn mcp(root: PathBuf) -> Result<()> {
    // MCP stdio uses newline-delimited JSON-RPC. stdout is protocol-only.
    for line in io::stdin().lock().lines() {
        let line = line?;
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(id) = req.get("id") else { continue };
        let method = req["method"].as_str().unwrap_or("");
        let reply = match method {
            "initialize" => {
                json!({"protocolVersion":req["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"agent-vocal-studio","version":env!("CARGO_PKG_VERSION")},"instructions":"Local Rust vocal editor. Read project ids/revision before editing. For a new song, read music_knowledge and instrument_catalog, save a concrete composition_design, then audition the core phrase with simple accompaniment before developing sections. Do not create random template songs; score metrics do not certify musical quality. Use agent_focus for visible intent, then pass its intent_id to the matching project_edit, selection_edit, or render_start at the same revision. The app draws its own cursor and never needs the OS mouse. Use selection_resolve (from_ui for the user's box selection) and selection_edit preview/commit for targeted changes. A changed revision invalidates the selection; never silently retarget or replay. Preserve other projects. Native avpack preserves tuning and backing assets; MIDI does not. VOICEVOX and OpenUtau use independently installed singers. OpenUtau renders only with an installed Japanese Classic bank. guide is oscillator-only. Report actual tests and listening separately."})
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools":tools()}),
            "tools/call" => {
                let result = (|| {
                    proxy(
                        &root,
                        field(&req["params"], "name")?,
                        req["params"].get("arguments").unwrap_or(&json!({})),
                    )
                })();
                match result {
                    Ok(v) => {
                        json!({"content":[{"type":"text","text":serde_json::to_string(&v)?}],"structuredContent":if v.is_object(){v.clone()}else{json!({"items":v})},"isError":false})
                    }
                    Err(e) => {
                        let message = format!("{e:#}");
                        let mut reply =
                            json!({"content":[{"type":"text","text":message}],"isError":true});
                        if e.is::<RemoteRevisionConflict>() {
                            reply["structuredContent"] =
                                json!({"code":REVISION_CONFLICT,"message":message});
                        }
                        reply
                    }
                }
            }
            _ => {
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}})
                );
                io::stdout().flush()?;
                continue;
            }
        };
        println!("{}", json!({"jsonrpc":"2.0","id":id,"result":reply}));
        io::stdout().flush()?;
    }
    Ok(())
}
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = if let Some(i) = args.iter().position(|x| x == "--home") {
        PathBuf::from(args.get(i + 1).context("--home needs a path")?)
    } else {
        let exe = std::env::current_exe()?;
        exe.parent()
            .context("No executable parent")?
            .parent()
            .context("No Studio home")?
            .to_path_buf()
    };
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    if args.iter().any(|a| a == "--mcp") {
        mcp(root)
    } else if let Some(i) = args.iter().position(|a| a == "--call") {
        let name = args.get(i + 1).context("--call needs a tool name")?;
        let params = if let Some(s) = args.get(i + 2) {
            serde_json::from_str(s)?
        } else {
            json!({})
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&proxy(&root, name, &params)?)?
        );
        Ok(())
    } else if args.len() == 1 || args.iter().any(|a| a == "--open") {
        let exe = root.join("bin/agent-vocal-desktop.exe");
        ensure!(exe.is_file(), "Desktop application missing; run build.ps1");
        let mut cmd = Command::new(exe);
        cmd.arg("--home").arg(&root);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.spawn()?;
        Ok(())
    } else {
        serve(root, args.iter().any(|a| a == "--development-ui"))
    }
}

pub(crate) fn prepare_desktop(root: &PathBuf) -> Result<()> {
    let exe = root.join("runtime/voicevox/windows-cpu/run.exe");
    if exe.exists()
        && engine::get("/version").is_err()
        && std::net::TcpStream::connect_timeout(
            &"127.0.0.1:50021".parse().unwrap(),
            Duration::from_millis(250),
        )
        .is_err()
    {
        let mut cmd = Command::new(&exe);
        cmd.args([
            "--host",
            "127.0.0.1",
            "--port",
            "50021",
            "--cpu_num_threads",
            "4",
        ])
        .current_dir(exe.parent().unwrap())
        .stdin(std::process::Stdio::null())
        .stdout(fs::File::create(root.join("runtime/voicevox-out.log"))?)
        .stderr(fs::File::create(root.join("runtime/voicevox-error.log"))?);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.spawn()?;
    }
    ensure_server(root)?;
    Ok(())
}
