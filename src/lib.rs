mod archived_mix;
mod arrange_audition;
mod arrangement;
mod audio;
mod audition;
mod automation;
mod balance;
#[cfg(windows)]
mod capture_clock;
mod clips;
#[cfg(windows)]
mod desktop;
mod draft_changes;
mod drum_kit;
mod effects;
mod engine;
mod exchange;
#[cfg(windows)]
mod file_browser;
mod harmony;
mod http_audio;
mod instruments;
mod jobs;
#[cfg(windows)]
mod live_mix;
mod loudness;
mod lyric_lines;
mod lyrics;
mod lyrics_fit;
#[cfg(windows)]
mod metronome;
mod midi_exchange;
mod model;
#[cfg(windows)]
mod monitor_stream;
mod monitor_voice;
mod motif;
mod music;
mod music_review;
mod openutau;
mod pitch_join;
mod pitch_reference;
mod presence;
mod project_index;
#[cfg(windows)]
mod recording;
mod resample;
mod rhythm;
mod routing;
mod section_review;
mod selection;
mod service;
mod source_cache;
mod stem_import;
mod stretch;
mod timeline;
mod transient;
#[cfg(windows)]
mod transport;
#[cfg(windows)]
mod transport_link;
mod vibrato;
mod voicing;
mod wave_delivery;
pub fn run_cli() -> anyhow::Result<()> {
    service::run()
}
#[cfg(windows)]
pub fn run_desktop() -> anyhow::Result<()> {
    desktop::run()
}
