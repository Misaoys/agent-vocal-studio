use crate::{
    audio::{asset_reader, visit_clip},
    clips::Clip,
    timeline::Timeline,
};
use anyhow::{ensure, Context, Result};
use std::{
    f32::consts::FRAC_PI_2,
    io::{Read, Seek},
    path::Path,
};

/// Waveform-similarity overlap-add. The project clock gives the nominal input
/// position at each output hop. Search shifts a window by at most half a hop
/// to keep its overlap coherent. Both stereo channels use the same shift.
pub(crate) fn visit(
    root: &Path,
    clip: &Clip,
    clock: &Timeline,
    mut write: impl FnMut(u32, [f32; 2]) -> Result<()>,
) -> Result<()> {
    let Some(beats) = clip.beat_length else {
        return visit_clip(root, clip, write);
    };
    let first = clip.start_frame(clock);
    let length = (clip.end_frame(clock) - first) as usize;
    let input = clip.frames as usize;
    if length == input
        && clock.constant_tempo(clip.start, clip.start + beats)
        && clip
            .warp_points
            .iter()
            .all(|pin| clip.frame_at_source(clock, pin.frame) == first + i64::from(pin.frame))
    {
        return visit_clip(root, clip, write);
    }
    let mut reader = asset_reader(root, &clip.asset)?;
    ensure!(
        clip.offset + clip.frames <= reader.duration(),
        "Clip source exceeds its WAV asset"
    );
    let hop = 960.min((input.min(length) / 2).max(1));
    let window = hop * 2;
    let search = hop / 2;
    let mut previous = vec![[0.; 2]; window];
    let mut candidates = vec![[0.; 2]; window + 2 * search];
    let fade: Vec<_> = (0..hop)
        .map(|index| {
            let angle = FRAC_PI_2 * index as f32 / hop.saturating_sub(1).max(1) as f32;
            angle.sin().powi(2)
        })
        .collect();
    // End the overlap at the pinned sample. At that point the new source
    // window has full gain and its next sample carries through to the next hop.
    let mut pins = clip
        .warp_points
        .iter()
        .map(|point| {
            (
                (clip.frame_at_source(clock, point.frame) - first) as usize + 1,
                point.frame as usize + 1,
            )
        })
        .peekable();
    let mut previous_count = hop
        .min(length)
        .min(pins.peek().map_or(length, |pin| pin.0 - 1));
    read_window(&mut reader, clip, 0, &mut previous)?;
    for (index, &sample) in previous.iter().take(previous_count).enumerate() {
        write(index as u32, sample)?;
    }
    let mut offset = previous_count;
    let mut source_floor = 0;
    while offset < length {
        let pin = pins.peek().copied();
        let count = pin.map_or(hop.min(length - offset), |(output, source)| {
            hop.min(output - offset).min(source - source_floor)
        });
        let forced = pin
            .filter(|&(output, _)| offset + count == output)
            .map(|(_, source)| source - count);
        let tail = &previous[previous_count..previous_count + count];
        let next = if let Some(source) = forced {
            read_window(&mut reader, clip, source, &mut candidates[..window])?;
            &candidates[..window]
        } else {
            let required = (length - offset).min(window);
            let latest = input.saturating_sub(required).max(source_floor);
            let ideal = (clip
                .source_at_render_frame(clock, first + offset as i64)
                .round() as usize)
                .clamp(source_floor, latest);
            let low = ideal.saturating_sub(search).max(source_floor);
            let high = (ideal + search).min(latest);
            let scan = &mut candidates[..high - low + window];
            read_window(&mut reader, clip, low, scan)?;
            let best = align(tail, scan, ideal - low, high - low);
            &scan[best..best + window]
        };
        for index in 0..count {
            let alpha = if forced.is_some() && index + 1 == count {
                1.
            } else if count == hop || clip.warp_points.is_empty() {
                fade[index]
            } else if count == 1 {
                1.
            } else {
                (FRAC_PI_2 * index as f32 / (count - 1) as f32)
                    .sin()
                    .powi(2)
            };
            let sample =
                [0, 1].map(|side| tail[index][side] * (1. - alpha) + next[index][side] * alpha);
            write((offset + index) as u32, sample)?;
        }
        previous.copy_from_slice(next);
        previous_count = count;
        offset += count;
        if forced.is_some() {
            source_floor = pins.next().unwrap().1;
        }
    }
    Ok(())
}

fn read_window<R: Read + Seek>(
    reader: &mut hound::WavReader<R>,
    clip: &Clip,
    start: usize,
    out: &mut [[f32; 2]],
) -> Result<()> {
    let count = out.len().min(clip.frames as usize - start);
    reader.seek(clip.offset + start as u32)?;
    let mut samples = reader.samples::<f32>();
    for item in &mut out[..count] {
        let left = samples.next().context("Truncated audio asset")??;
        let right = samples.next().context("Truncated audio asset")??;
        ensure!(
            left.is_finite() && right.is_finite(),
            "Non-finite audio asset sample"
        );
        *item = [left, right];
    }
    out[count..].fill([0.; 2]);
    Ok(())
}

fn align(tail: &[[f32; 2]], scan: &[[f32; 2]], expected: usize, limit: usize) -> usize {
    let score = |offset: usize, stride: usize| {
        let mut xy = 0.;
        let mut xx = 0.;
        let mut yy = 0.;
        for (&a, &b) in tail.iter().zip(&scan[offset..]).step_by(stride) {
            xy += a[0] * b[0] + a[1] * b[1];
            xx += a[0] * a[0] + a[1] * a[1];
            yy += b[0] * b[0] + b[1] * b[1];
        }
        let similarity = if xx > 1e-12 && yy > 1e-12 {
            xy / (xx * yy).sqrt()
        } else {
            0.
        };
        similarity - 0.04 * offset.abs_diff(expected) as f32 / limit.max(1) as f32
    };
    let coarse = (0..=limit)
        .step_by(8)
        .chain([expected, limit])
        .map(|offset| (offset, score(offset, 8)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
        .0;
    (coarse.saturating_sub(7)..=(coarse + 7).min(limit))
        .map(|offset| (offset, score(offset, 4)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
        .0
}
