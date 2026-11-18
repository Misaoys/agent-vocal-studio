use std::f64::consts::PI;

pub(crate) fn blackman_sinc<const N: usize>(cutoff: f64, fraction: f64) -> [f64; N] {
    let mut weights = std::array::from_fn(|tap| {
        let x = tap as f64 - (N / 2 - 1) as f64 - fraction;
        let sinc = if x.abs() < 1e-12 {
            cutoff
        } else {
            (PI * x * cutoff).sin() / (PI * x)
        };
        let position = (x + (N / 2) as f64) / N as f64;
        let window = 0.42 - 0.5 * (2. * PI * position).cos() + 0.08 * (4. * PI * position).cos();
        sinc * window
    });
    let sum: f64 = weights.iter().sum();
    for value in &mut weights {
        *value /= sum;
    }
    weights
}

#[cfg(windows)]
pub(crate) mod playback {
    use super::blackman_sinc;
    use std::ops::Range;

    pub const TAPS: usize = 128;
    pub const LOOKAHEAD: usize = TAPS / 2;
    const LEFT: usize = LOOKAHEAD - 1;
    const SOURCE_RATE: u64 = crate::monitor_stream::RATE;

    pub fn phase_step(output_rate: u64) -> u64 {
        let (mut a, mut b) = (SOURCE_RATE, output_rate);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    }

    pub struct Kernel {
        table: Vec<[f32; TAPS]>,
        phases: u64,
        rate: u64,
    }

    impl Kernel {
        /// Prepare coefficients on the control thread. Common endpoint rates use exact phases.
        pub fn new(rate: u64) -> Option<Self> {
            if rate == SOURCE_RATE {
                return None;
            }
            let phases = (rate / phase_step(rate)).min(4096);
            let cutoff = 0.950 * (rate as f64 / SOURCE_RATE as f64).min(1.);
            let table = (0..=phases)
                .map(|phase| {
                    blackman_sinc::<TAPS>(cutoff, phase as f64 / phases as f64)
                        .map(|value| value as f32)
                })
                .collect();
            Some(Self {
                table,
                phases,
                rate,
            })
        }

        fn weights(&self, remainder: u64) -> &[f32; TAPS] {
            let phase = (remainder * self.phases + self.rate / 2) / self.rate;
            &self.table[phase as usize]
        }
    }

    pub struct Window<'a> {
        pub index: usize,
        pub position: f64,
        bounds: Range<usize>,
        repeat: bool,
        weights: Option<&'a [f32; TAPS]>,
    }

    impl<'a> Window<'a> {
        pub fn new(
            position: u64,
            rate: u64,
            bounds: Range<usize>,
            repeat: bool,
            kernel: Option<&'a Kernel>,
        ) -> Self {
            let index = (position / rate) as usize;
            let remainder = position % rate;
            Self {
                index,
                position: index as f64 + remainder as f64 / rate as f64,
                bounds,
                repeat,
                weights: kernel.map(|kernel| kernel.weights(remainder)),
            }
        }

        pub fn direct(&self) -> bool {
            self.weights.is_none()
        }

        pub fn contiguous(&self) -> Option<Range<usize>> {
            (self.index - self.bounds.start >= LEFT && self.bounds.end - self.index > LOOKAHEAD)
                .then(|| self.index - LEFT..self.index + LOOKAHEAD + 1)
        }

        pub fn indices(&self) -> [usize; TAPS] {
            let start = self.bounds.start as isize;
            let end = self.bounds.end as isize;
            std::array::from_fn(|tap| {
                let frame = self.index as isize + tap as isize - LEFT as isize;
                if self.repeat {
                    (start + (frame - start).rem_euclid(end - start)) as usize
                } else {
                    frame.clamp(start, end - 1) as usize
                }
            })
        }

        pub fn filter(&self, frames: &[[f32; 2]; TAPS]) -> [f32; 2] {
            let weights = self.weights.expect("window is band-limited");
            // Independent partial sums allow the compiler to vectorize the stereo dot products.
            let mut sums = [[0_f32; 2]; 8];
            for (frames, weights) in frames
                .as_chunks::<8>()
                .0
                .iter()
                .zip(weights.as_chunks::<8>().0)
            {
                for ((sum, frame), weight) in sums.iter_mut().zip(frames).zip(weights) {
                    sum[0] += frame[0] * weight;
                    sum[1] += frame[1] * weight;
                }
            }
            [
                sums.iter().map(|sum| sum[0]).sum(),
                sums.iter().map(|sum| sum[1]).sum(),
            ]
        }

        pub fn read(&self, frames: &[[f32; 2]]) -> [f32; 2] {
            if self.direct() {
                return frames[self.index];
            }
            if let Some(range) = self.contiguous() {
                return self.filter(frames[range].try_into().expect("one sinc window"));
            }
            self.filter(&self.indices().map(|frame| frames[frame]))
        }
    }
}
