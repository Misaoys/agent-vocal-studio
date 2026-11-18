//! Bounded prefetch for transport playback of immutable rendered PCM.
use crate::resample::playback::{Window, LOOKAHEAD, TAPS};
use anyhow::{ensure, Context, Result};
use std::{
    fs::File,
    io::BufReader,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

pub(crate) const RATE: u64 = 48_000;
pub(crate) const MAX_FRAMES: u64 = (15 * 60 + 12) * RATE;
pub(crate) const STREAM_AFTER: usize = 30 * RATE as usize;
const PAGE: usize = 24_576;
const CACHE_PAGES: usize = 5;
const NONE: usize = usize::MAX;
type Reader = hound::WavReader<BufReader<File>>;

pub(crate) fn open_mix(path: &Path) -> Result<Reader> {
    let wav = hound::WavReader::open(path).context("无法读取试听 WAV")?;
    let spec = wav.spec();
    ensure!(
        spec.sample_rate as u64 == RATE
            && spec.channels == 2
            && spec.bits_per_sample == 24
            && spec.sample_format == hound::SampleFormat::Int
            && (1..=MAX_FRAMES).contains(&(wav.duration() as u64)),
        "试听 WAV 必须为 48 kHz / 24-bit 双声道混音"
    );
    Ok(wav)
}

pub(crate) fn read_frames(wav: &mut Reader, count: usize) -> Result<Vec<[f32; 2]>> {
    let mut frames = Vec::new();
    frames.try_reserve_exact(count)?;
    let mut samples = wav.samples::<i32>();
    for _ in 0..count {
        let left = samples.next().context("试听 WAV 样本不完整")??;
        let right = samples.next().context("试听 WAV 双声道不完整")??;
        frames.push([left as f32 / 8_388_608., right as f32 / 8_388_608.]);
    }
    Ok(frames)
}

struct Page {
    number: usize,
    start: usize,
    per_track: usize,
    frames: Vec<[f32; 2]>,
}

#[derive(Clone, Copy)]
struct AtFrame<'a> {
    index: usize,
    stride: usize,
    samples: &'a [[f32; 2]],
}

impl AtFrame<'_> {
    pub fn channel(&self, track: usize) -> [f32; 2] {
        self.samples[track * self.stride + self.index]
    }
}

#[derive(Default)]
pub(crate) struct Pages([Option<Page>; CACHE_PAGES]);

impl Pages {
    fn page(&self, number: usize) -> Option<&Page> {
        self.0.iter().flatten().find(|page| page.number == number)
    }

    #[cfg(test)]
    fn sample(&self, frame: usize) -> Option<[f32; 2]> {
        self.frame(frame).map(|frame| frame.channel(0))
    }

    fn frame(&self, frame: usize) -> Option<AtFrame<'_>> {
        let page = self.page(frame / PAGE).or_else(|| {
            self.0
                .iter()
                .flatten()
                .find(|page| (page.start..page.start + page.per_track).contains(&frame))
        })?;
        let index = frame - page.start;
        (index < page.per_track).then_some(AtFrame {
            index,
            stride: page.per_track,
            samples: &page.frames,
        })
    }

    pub fn read_inputs(&self, window: &Window<'_>, inputs: &mut [[f32; 2]]) -> Option<()> {
        if window.direct() {
            let frame = self.frame(window.index)?;
            for (track, input) in inputs.iter_mut().enumerate() {
                *input = frame.channel(track);
            }
        } else if let Some(range) = window.contiguous() {
            let page = self.page(window.index / PAGE)?;
            let offset = range.start - page.start;
            for (track, input) in inputs.iter_mut().enumerate() {
                let start = track * page.per_track + offset;
                *input = window.filter(page.frames[start..start + TAPS].try_into().ok()?);
            }
        } else {
            let indices = window.indices();
            let mut frames = [self.frame(indices[0])?; TAPS];
            for (frame, index) in frames.iter_mut().zip(indices).skip(1) {
                *frame = self.frame(index)?;
            }
            for (track, input) in inputs.iter_mut().enumerate() {
                *input = window.filter(&frames.each_ref().map(|frame| frame.channel(track)));
            }
        }
        Some(())
    }

    pub fn resident_frames(&self) -> usize {
        self.0.iter().flatten().map(|page| page.frames.len()).sum()
    }
}

struct Shared {
    pages: Arc<Mutex<Pages>>,
    current: AtomicUsize,
    loop_head: AtomicUsize,
    loop_tail: AtomicUsize,
    stopped: AtomicBool,
    failed: AtomicBool,
    buffered_frames: AtomicUsize,
}

pub(crate) struct Stream {
    shared: Arc<Shared>,
    len: usize,
    worker: thread::Thread,
}

fn wanted(len: usize, current: usize, head: usize, tail: usize) -> [usize; CACHE_PAGES] {
    let next = |page: usize| {
        if page != NONE && (page + 1) * PAGE < len {
            page + 1
        } else {
            NONE
        }
    };
    let after_head = next(head);
    let after_head = if after_head <= tail { after_head } else { NONE };
    [current, head, tail, next(current), after_head]
}

impl Stream {
    pub fn open(mut wav: Reader, first: usize, repeat: Option<(usize, usize)>) -> Result<Self> {
        let len = wav.duration() as usize;
        Self::from_reader(len, 1, first, repeat, move |start, count| {
            wav.seek(start as u32)?;
            read_frames(&mut wav, count)
        })
    }

    pub fn from_reader(
        len: usize,
        tracks: usize,
        first: usize,
        repeat: Option<(usize, usize)>,
        mut read: impl FnMut(usize, usize) -> Result<Vec<[f32; 2]>> + Send + 'static,
    ) -> Result<Self> {
        ensure!(len > 0 && tracks > 0, "Empty prefetch source");
        let mut decode = move |number: usize| {
            let first = number * PAGE;
            let start = first.saturating_sub(LOOKAHEAD);
            let end = (first + PAGE + LOOKAHEAD).min(len);
            let per_track = end - start;
            let frames = read(start, per_track)?;
            ensure!(
                frames.len() == per_track * tracks,
                "Prefetch returned an incomplete block"
            );
            Ok::<Page, anyhow::Error>(Page {
                number,
                start,
                per_track,
                frames,
            })
        };
        let current = first.min(len - 1) / PAGE;
        let (head, tail) = repeat_pages(len, repeat);
        let mut pages = Pages::default();
        let initial = wanted(len, current, head, tail);
        for (i, page) in initial.into_iter().enumerate() {
            if page != NONE && pages.page(page).is_none() {
                pages.0[i] = Some(decode(page)?);
            }
        }
        let buffered_frames = pages.resident_frames();
        let shared = Arc::new(Shared {
            pages: Arc::new(Mutex::new(pages)),
            current: AtomicUsize::new(current),
            loop_head: AtomicUsize::new(head),
            loop_tail: AtomicUsize::new(tail),
            stopped: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            buffered_frames: AtomicUsize::new(buffered_frames),
        });
        let reader = shared.clone();
        let handle = thread::Builder::new()
            .name("studio-audio-prefetch".into())
            .spawn(move || {
                let desired = || {
                    wanted(
                        len,
                        reader.current.load(Relaxed),
                        reader.loop_head.load(Relaxed),
                        reader.loop_tail.load(Relaxed),
                    )
                };
                let mut ready = Some(initial);
                while !reader.stopped.load(Relaxed) {
                    let keys = desired();
                    if Some(keys) == ready {
                        thread::park_timeout(Duration::from_millis(4));
                        continue;
                    }
                    ready = None;
                    let missing = {
                        let pages = reader.pages.lock().unwrap();
                        keys.into_iter()
                            .find(|&key| key != NONE && pages.page(key).is_none())
                    };
                    let Some(number) = missing else {
                        ready = Some(keys);
                        continue;
                    };
                    let page = match decode(number) {
                        Ok(page) => page,
                        Err(_) => {
                            reader.failed.store(true, Relaxed);
                            break;
                        }
                    };
                    let now = desired();
                    if !now.contains(&number) || reader.stopped.load(Relaxed) {
                        continue;
                    }
                    let retired = {
                        let mut pages = reader.pages.lock().unwrap();
                        let slot = pages.0.iter_mut().find(|slot| {
                            slot.as_ref().is_none_or(|page| !now.contains(&page.number))
                        });
                        let retired = slot.and_then(|slot| slot.replace(page));
                        reader
                            .buffered_frames
                            .store(pages.resident_frames(), Relaxed);
                        retired
                    };
                    drop(retired);
                }
            })?;
        Ok(Self {
            shared,
            len,
            worker: handle.thread().clone(),
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn pages(&self) -> Arc<Mutex<Pages>> {
        self.shared.pages.clone()
    }

    pub fn failed(&self) -> bool {
        self.shared.failed.load(Relaxed)
    }

    pub fn buffered_frames(&self) -> usize {
        self.shared.buffered_frames.load(Relaxed)
    }

    pub fn request(&self, position: usize, repeat: Option<(usize, usize)>) {
        let (head, tail) = repeat_pages(self.len, repeat);
        self.shared.loop_head.store(head, Relaxed);
        self.shared.loop_tail.store(tail, Relaxed);
        self.shared
            .current
            .store(position.min(self.len - 1) / PAGE, Relaxed);
    }
}

fn repeat_pages(len: usize, repeat: Option<(usize, usize)>) -> (usize, usize) {
    repeat
        .filter(|&(start, end)| start < end && end <= len)
        .map_or((NONE, NONE), |(start, end)| {
            (start / PAGE, (end - 1) / PAGE)
        })
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Relaxed);
        self.worker.unpark();
    }
}

#[cfg(test)]
pub(crate) fn fixture_wav(path: &Path, frames: usize) -> Result<()> {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE as u32,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for frame in 0..frames {
        let value = frame as i32 - 48_000;
        writer.write_sample(value)?;
        writer.write_sample(-value)?;
    }
    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn grouped_pages_expose_all_tracks_at_the_same_frame_and_follow_latest_seek() -> Result<()> {
        let len = PAGE * 12 + 177;
        let tracks = 16;
        let signal = |track: usize, frame: usize| {
            let value = track as f32 / 32. + frame as f32 / 1_000_000.;
            [value, -value]
        };
        let paged = Stream::from_reader(
            len,
            tracks,
            PAGE - 2,
            Some((PAGE - 2, PAGE * 4 + 5)),
            move |start, count| {
                Ok((0..tracks)
                    .flat_map(|slot| (start..start + count).map(move |frame| signal(slot, frame)))
                    .collect())
            },
        )?;
        let cache = paged.pages();
        let final_frame = PAGE * 12 + 172;
        paged.request(PAGE * 7 + 9, Some((PAGE - 2, PAGE * 4 + 5)));
        paged.request(final_frame, Some((PAGE - 2, PAGE * 4 + 5)));
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let pages = cache.lock().unwrap();
            if let Some(last) = pages.frame(final_frame) {
                let loop_head = pages.frame(PAGE - 2).unwrap();
                for slot in 0..tracks {
                    assert_eq!(last.channel(slot), signal(slot, final_frame));
                    assert_eq!(loop_head.channel(slot), signal(slot, PAGE - 2));
                }
                assert!(pages.frame(len).is_none());
                assert!(paged.buffered_frames() <= CACHE_PAGES * (PAGE + 2 * LOOKAHEAD) * tracks);
                break;
            }
            drop(pages);
            assert!(Instant::now() < until && !paged.failed());
            thread::sleep(Duration::from_millis(4));
        }
        Ok(())
    }

    #[test]
    fn far_seek_prefetches_exact_pcm_and_preserves_loop_head_in_fixed_cache() -> Result<()> {
        let path = std::env::temp_dir().join(format!("{}.wav", crate::model::uid("paged_mix")));
        fixture_wav(&path, PAGE * 11 + 19)?;
        let first = PAGE * 2 + 59;
        let head = PAGE * 2 + 1;
        let seek = PAGE * 9 + 316;
        let repeat = Some((head, PAGE * 5 + 2));
        let stream = Stream::open(open_mix(&path)?, first, repeat)?;
        let pages = stream.pages();
        let expected = |frame: usize| {
            let value = (frame as i32 - 48_000) as f32 / 8_388_608.;
            [value, -value]
        };
        assert_eq!(pages.lock().unwrap().sample(first), Some(expected(first)));
        stream.request(PAGE * 6 + 88, repeat);
        stream.request(seek, repeat);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let sample = pages.lock().unwrap().sample(seek + PAGE);
            if let Some(sample) = sample {
                assert_eq!(sample, expected(seek + PAGE));
                break;
            }
            assert!(Instant::now() < deadline && !stream.failed());
            thread::sleep(Duration::from_millis(4));
        }
        let cached = pages.lock().unwrap();
        assert_eq!(cached.sample(seek), Some(expected(seek)));
        assert_eq!(cached.sample(head), Some(expected(head)));
        assert_eq!(cached.sample(PAGE * 5 + 1), Some(expected(PAGE * 5 + 1)));
        assert_eq!(
            cached.resident_frames(),
            CACHE_PAGES * (PAGE + 2 * LOOKAHEAD) - (LOOKAHEAD - 19)
        );
        drop(cached);
        drop(stream);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
