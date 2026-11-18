use cpal::StreamInstant;
use std::{
    sync::atomic::{AtomicU64, Ordering::*},
    time::Duration,
};

#[derive(Default)]
pub(crate) struct CaptureClock {
    input_ns: AtomicU64,
    punch_ns: AtomicU64,
}

fn nanos(instant: StreamInstant) -> Option<u64> {
    instant
        .duration_since(&StreamInstant::new(0, 0))?
        .as_nanos()
        .try_into()
        .ok()
}

impl CaptureClock {
    pub(crate) fn input(&self, instant: StreamInstant) {
        if let Some(ns) = nanos(instant).filter(|ns| *ns > 0) {
            let _ = self.input_ns.compare_exchange(0, ns, Release, Relaxed);
        }
    }

    pub(crate) fn punch(&self, callback: StreamInstant, seconds_after_callback: f64) {
        if self.punch_ns.load(Relaxed) != 0 {
            return;
        }
        if let Some(ns) = callback
            .add(Duration::from_secs_f64(seconds_after_callback))
            .and_then(nanos)
        {
            let _ = self.punch_ns.compare_exchange(0, ns, Release, Relaxed);
        }
    }

    pub(crate) fn punch_instant(&self) -> Option<StreamInstant> {
        let ns = self.punch_ns.load(Acquire);
        (ns != 0)
            .then(|| StreamInstant::new((ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as u32))
    }

    /// A positive offset discards input preroll. A negative one fills the missing start with silence.
    pub(crate) fn offset(&self, rate: u32, correction_ms: f64) -> Option<i64> {
        let input = self.input_ns.load(Acquire);
        let punch = self.punch_ns.load(Acquire);
        if input == 0 || punch == 0 {
            return None;
        }
        let seconds = (i128::from(punch) - i128::from(input)) as f64 / 1e9 + correction_ms / 1000.;
        Some((seconds * f64::from(rate)).round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_qpc_aligns_input_to_output_and_applies_signed_correction() {
        let timing = CaptureClock::default();
        assert_eq!(timing.offset(44_100, 0.), None);
        timing.input(StreamInstant::new(19, 990_000_000));
        timing.punch(StreamInstant::new(20, 0), 0.110);
        timing.punch(StreamInstant::new(22, 0), 0.10);
        assert_eq!(timing.offset(44_100, 0.), Some(5292));
        assert_eq!(timing.offset(48_000, 20.), Some(6720));
        assert_eq!(timing.offset(48_000, -150.), Some(-1440));
    }
}
