use crate::{
    effects::{db_gain, Effect, CHAIN_LIMIT},
    model::{identifier, Project, Track},
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub(crate) const AUX_LIMIT: usize = 4;
pub(crate) const GROUP_LIMIT: usize = 8;
pub(crate) const BUS_LIMIT: usize = GROUP_LIMIT + AUX_LIMIT;
// The first slot is the direct master path. The rest follow the bus list.
pub(crate) type Levels = [f64; BUS_LIMIT + 1];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bus {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub gain_db: f64,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Send {
    pub bus_id: String,
    pub gain_db: f64,
}

pub(crate) fn buses(project: &Project) -> impl Iterator<Item = &Bus> {
    project.mix_groups.iter().chain(&project.aux_buses)
}

pub(crate) fn validate_buses(groups: &[Bus], returns: &[Bus]) -> Result<()> {
    ensure!(
        groups.len() <= GROUP_LIMIT,
        "At most {GROUP_LIMIT} mix groups"
    );
    ensure!(
        returns.len() <= AUX_LIMIT,
        "At most {AUX_LIMIT} auxiliary returns"
    );
    let mut seen = HashSet::new();
    for bus in groups.iter().chain(returns) {
        identifier(&bus.id)?;
        ensure!(seen.insert(&bus.id), "Duplicate mix bus ID");
        ensure!(
            !bus.name.trim().is_empty() && bus.name.chars().count() <= 100,
            "Invalid mix bus name"
        );
        ensure!(
            (-60.0..=12.).contains(&bus.gain_db),
            "Bus gain must be -60..12 dB"
        );
        ensure!(
            bus.effects.len() <= CHAIN_LIMIT,
            "Bus effect chain exceeds insert limit"
        );
        for effect in &bus.effects {
            effect.validate()?;
        }
    }
    Ok(())
}

pub(crate) fn validate_sends(sends: &[Send], buses: &[Bus]) -> Result<()> {
    let mut seen = HashSet::new();
    for send in sends {
        ensure!(
            seen.insert(&send.bus_id),
            "Duplicate send to auxiliary return"
        );
        ensure!(
            buses.iter().any(|b| b.id == send.bus_id),
            "Send refers to an unknown auxiliary return"
        );
        ensure!(
            (-60.0..=0.).contains(&send.gain_db),
            "Send gain must be -60..0 dB"
        );
    }
    Ok(())
}

pub(crate) fn group_open(output_group: Option<&str>, groups: &[Bus]) -> bool {
    !groups
        .iter()
        .any(|group| output_group == Some(&group.id) && group.mute)
}

pub(crate) fn levels(
    output_group: Option<&str>,
    sends: &[Send],
    groups: &[Bus],
    returns: &[Bus],
) -> Levels {
    let mut levels = [0.; BUS_LIMIT + 1];
    let group = output_group.and_then(|id| groups.iter().position(|bus| bus.id == id));
    levels[group.map_or(0, |index| index + 1)] = 1.;
    if group.is_none_or(|index| !groups[index].mute) {
        for (index, bus) in returns.iter().enumerate() {
            if let Some(send) = sends.iter().find(|send| send.bus_id == bus.id) {
                levels[1 + groups.len() + index] = db_gain(send.gain_db);
            }
        }
    }
    levels
}

pub(crate) fn validate_track(track: &Track, project: &Project) -> Result<()> {
    ensure!(
        !buses(project).any(|bus| bus.id == track.id),
        "A track and mix bus share an ID"
    );
    if let Some(id) = &track.output_group {
        ensure!(
            project.mix_groups.iter().any(|group| &group.id == id),
            "Track output refers to an unknown mix group"
        );
    }
    validate_sends(&track.sends, &project.aux_buses)
}

impl Bus {
    pub(crate) fn gain(&self) -> f64 {
        if self.mute {
            0.
        } else {
            db_gain(self.gain_db)
        }
    }
}

#[cfg(windows)]
pub(crate) fn same_buses(old: &[Bus], new: &[Bus]) -> bool {
    old.iter()
        .map(|bus| &bus.id)
        .eq(new.iter().map(|bus| &bus.id))
}

pub(crate) fn add(frame: [f32; 2], gain: f64, sum: &mut [f32; 2]) {
    for (input, output) in frame.into_iter().zip(sum) {
        *output += (f64::from(input) * gain) as f32;
    }
}
