//! Presentation pacing is measured only in forced-redraw profiling mode. Idle
//! wakeups are not frames, and CPU submission time is not a GPU timestamp.
use serde::Serialize;
use std::time::Instant;

const WARMUP: usize = 10;

#[derive(Default)]
pub struct Profile {
    presented: usize,
    previous: Option<Instant>,
    intervals: Vec<f64>,
    cpu: Vec<f64>,
    timeline: Vec<Frame>,
}
/// One profiled frame: where its time went (milliseconds) and how much of it was redrawn.
#[derive(Clone, Copy, Serialize)]
pub struct Frame {
    pub interval_ms: f64,
    pub compose_ms: f64,
    pub render_ms: f64,
    pub swap_ms: f64,
    pub redrawn: f64,
    /// First-use work inside render_ms: glyph outlines, logo meshes, images.
    pub glyph_ms: f64,
    pub vector_ms: f64,
    pub image_ms: f64,
}
#[derive(Serialize)]
pub struct Timing {
    pub samples: usize,
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}
#[derive(Serialize)]
pub struct Summary {
    pub presented: usize,
    pub warmup_frames: usize,
    pub pacing: Option<Timing>,
    pub cpu_submission: Option<Timing>,
    pub timeline: Vec<Frame>,
}
impl Profile {
    pub fn record(&mut self, now: Instant, cpu_ms: f64, exclude: bool, mut frame: Frame) {
        self.presented += 1;
        if self.presented > WARMUP
            && !exclude
            && let Some(previous) = self.previous
        {
            frame.interval_ms = now.duration_since(previous).as_secs_f64() * 1000.;
            self.intervals.push(frame.interval_ms);
            self.cpu.push(cpu_ms);
            self.timeline.push(frame);
        }
        self.previous = Some(now);
    }
    pub fn pause(&mut self) {
        self.previous = None;
    }
    pub fn summary(&self) -> Summary {
        Summary {
            presented: self.presented,
            warmup_frames: WARMUP,
            pacing: summarize(&self.intervals),
            cpu_submission: summarize(&self.cpu),
            timeline: self.timeline.clone(),
        }
    }
}
fn summarize(samples: &[f64]) -> Option<Timing> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(Timing {
        samples: sorted.len(),
        mean_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p95_ms: sorted[(sorted.len() * 95).div_ceil(100) - 1],
        max_ms: *sorted.last()?,
    })
}

/// A gap longer than this between two presentations is idle time, not a frame interval:
/// idle screens draw on demand, so seconds without a frame are expected.
pub const IDLE_GAP_MS: f64 = 100.;
/// Interval histogram resolution for the 95th percentile (0.25 ms buckets up to the idle gap).
const BUCKETS_PER_MS: f64 = 4.;
const BUCKETS: usize = (IDLE_GAP_MS * BUCKETS_PER_MS) as usize + 1;

/// Rolling frame statistics while Luma is in front, kept in constant memory: every
/// presentation is O(1) (a few additions and one histogram bucket), with nothing sorted or
/// allocated on the SDL thread.
pub struct FrameStats {
    presented: u64,
    partial: u64,
    full: u64,
    intervals: u64,
    sum_ms: f64,
    max_ms: f64,
    over_20: u64,
    over_33: u64,
    histogram: [u32; BUCKETS],
    foreground_ms: f64,
    idle_ms: f64,
    /// While in front: when foreground time was last added up.
    counted: Option<Instant>,
    /// While in front: the last presentation, or the moment Luma came to the front.
    anchor: Option<Instant>,
    /// Whether `anchor` is a presentation (so the next one completes an interval).
    anchor_presented: bool,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            presented: 0,
            partial: 0,
            full: 0,
            intervals: 0,
            sum_ms: 0.,
            max_ms: 0.,
            over_20: 0,
            over_33: 0,
            histogram: [0; BUCKETS],
            foreground_ms: 0.,
            idle_ms: 0.,
            counted: None,
            anchor: None,
            anchor_presented: false,
        }
    }
}

/// What `luma-diagnostics.json` records under `frame_stats`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FrameSummary {
    pub presented: u64,
    pub partial_redraws: u64,
    pub full_redraws: u64,
    pub partial_share: f64,
    pub intervals: u64,
    pub interval_mean_ms: f64,
    pub interval_p95_ms: f64,
    pub interval_max_ms: f64,
    pub intervals_over_20ms: u64,
    pub intervals_over_33ms: u64,
    /// Time in front, and the part of it in gaps of more than `IDLE_GAP_MS` without a frame.
    pub foreground_s: f64,
    pub idle_s: f64,
}

fn millis(from: Instant, to: Instant) -> f64 {
    to.saturating_duration_since(from).as_secs_f64() * 1000.
}

impl FrameStats {
    /// Luma came to the front or left it. Time behind other apps is not counted.
    pub fn set_foreground(&mut self, now: Instant, foreground: bool) {
        match (foreground, self.counted.is_some()) {
            (true, false) => {
                self.counted = Some(now);
                self.anchor = Some(now);
                self.anchor_presented = false;
            }
            (false, true) => {
                self.count(now);
                self.idle_ms += self.pending_idle(now);
                self.counted = None;
                self.anchor = None;
            }
            _ => {}
        }
    }

    fn count(&mut self, now: Instant) {
        if let Some(counted) = self.counted {
            self.foreground_ms += millis(counted, now);
            self.counted = Some(now);
        }
    }

    /// The current gap without a frame, once it is long enough to be idle time.
    fn pending_idle(&self, now: Instant) -> f64 {
        self.anchor
            .map(|anchor| millis(anchor, now))
            .filter(|gap| *gap > IDLE_GAP_MS)
            .unwrap_or(0.)
    }

    /// One presentation; `partial` when only a damaged region was redrawn.
    pub fn presented(&mut self, now: Instant, partial: bool) {
        if self.counted.is_none() {
            return;
        }
        self.count(now);
        self.presented += 1;
        if partial {
            self.partial += 1;
        } else {
            self.full += 1;
        }
        if let Some(anchor) = self.anchor {
            let gap = millis(anchor, now);
            if gap > IDLE_GAP_MS {
                self.idle_ms += gap;
            } else if self.anchor_presented {
                self.intervals += 1;
                self.sum_ms += gap;
                self.max_ms = self.max_ms.max(gap);
                self.over_20 += u64::from(gap > 20.);
                self.over_33 += u64::from(gap > 33.);
                let bucket = ((gap * BUCKETS_PER_MS).ceil() as usize).min(BUCKETS - 1);
                self.histogram[bucket] += 1;
            }
        }
        self.anchor = Some(now);
        self.anchor_presented = true;
    }

    /// The figures up to `now`, including a gap without frames that is still running.
    pub fn summary(&self, now: Instant) -> FrameSummary {
        let foreground_ms = self.foreground_ms + self.counted.map_or(0., |at| millis(at, now));
        let p95 = if self.intervals == 0 {
            0.
        } else {
            let rank = (self.intervals * 95).div_ceil(100);
            let mut seen = 0;
            let bucket = self
                .histogram
                .iter()
                .position(|count| {
                    seen += u64::from(*count);
                    seen >= rank
                })
                .unwrap_or(BUCKETS - 1);
            // A bucket's upper edge, never above the largest interval actually seen.
            (bucket as f64 / BUCKETS_PER_MS).min(self.max_ms)
        };
        let round = |value: f64| (value * 100.).round() / 100.;
        FrameSummary {
            presented: self.presented,
            partial_redraws: self.partial,
            full_redraws: self.full,
            partial_share: round(self.partial as f64 / self.presented.max(1) as f64),
            intervals: self.intervals,
            interval_mean_ms: round(self.sum_ms / self.intervals.max(1) as f64),
            interval_p95_ms: round(p95),
            interval_max_ms: round(self.max_ms),
            intervals_over_20ms: self.over_20,
            intervals_over_33ms: self.over_33,
            foreground_s: round(foreground_ms / 1000.),
            idle_s: round((self.idle_ms + self.pending_idle(now)) / 1000.),
        }
    }
}

/// How often the running statistics reach the file (and once more at exit).
pub const WRITE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

enum Message {
    Facts(serde_json::Value),
    Frames(FrameSummary),
}

/// Writes `luma-diagnostics.json` on its own thread: the SDL thread only sends the startup
/// facts (at start and on a drawable change) and a small copy of the frame statistics (at
/// most once a minute and at exit). Serialising and file I/O never happen on the SDL thread.
pub struct Writer {
    sender: Option<std::sync::mpsc::Sender<Message>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    pub fn spawn(path: std::path::PathBuf) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel::<Message>();
        let thread = std::thread::Builder::new()
            .name("luma-diagnostics".into())
            .spawn(move || {
                let mut facts = serde_json::json!({});
                let mut frames = None;
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Facts(value) => facts = value,
                        Message::Frames(summary) => frames = Some(summary),
                    }
                    // Only the latest state matters: skip to it when messages queued up.
                    while let Ok(message) = receiver.try_recv() {
                        match message {
                            Message::Facts(value) => facts = value,
                            Message::Frames(summary) => frames = Some(summary),
                        }
                    }
                    let mut value = facts.clone();
                    if let (Some(object), Some(summary)) = (value.as_object_mut(), frames) {
                        object.insert(
                            "frame_stats".into(),
                            serde_json::to_value(summary).unwrap_or_default(),
                        );
                    }
                    if let Err(error) = write_startup(&path, &value) {
                        eprintln!("Luma diagnostics file: {error}");
                    }
                }
            })
            .ok();
        Self {
            sender: thread.is_some().then_some(sender),
            thread,
        }
    }

    pub fn facts(&self, facts: serde_json::Value) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Message::Facts(facts));
        }
    }

    pub fn frames(&self, summary: FrameSummary) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Message::Frames(summary));
        }
    }

    /// Waits for the queued writes (at exit, so the final statistics are on disk).
    pub fn finish(mut self) {
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Writes the startup facts (text mode, MSAA samples obtained, drawable, GL strings) as JSON,
/// replacing the previous file. Best effort: a failure is only logged.
pub fn write_startup(path: &std::path::Path, facts: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(facts).map_err(|error| error.to_string())?;
    // Written beside and renamed over, so a reader never sees half a file.
    let partial = path.with_extension("json.tmp");
    std::fs::write(&partial, text + "\n")
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn profiling_excludes_warmup_and_capture_stalls() {
        let mut profile = Profile::default();
        let start = Instant::now();
        for index in 0..20 {
            profile.record(
                start + Duration::from_millis(index * 16),
                2.,
                index == 19,
                Frame {
                    interval_ms: 0.,
                    compose_ms: 0.,
                    render_ms: 2.,
                    swap_ms: 0.,
                    redrawn: 1.,
                    glyph_ms: 0.,
                    vector_ms: 0.,
                    image_ms: 0.,
                },
            );
        }
        let summary = profile.summary();
        let pacing = summary.pacing.unwrap();
        assert_eq!(pacing.samples, 9);
        assert_eq!(pacing.mean_ms, 16.);
        assert_eq!(pacing.p95_ms, 16.);
        assert_eq!(summary.cpu_submission.unwrap().mean_ms, 2.);
    }

    #[test]
    fn frame_stats_count_intervals_slow_frames_redraws_and_idle_time_in_front_only() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut stats = FrameStats::default();
        // Behind another app nothing counts.
        stats.presented(at(0), false);
        stats.set_foreground(at(1000), true);
        // 400 ms in front before the first frame: idle, and no interval yet.
        stats.presented(at(1400), false);
        // Eighteen 16 ms intervals, then one of 25 ms and one of 40 ms (all partial redraws).
        let mut now = 1400;
        for _ in 0..18 {
            now += 16;
            stats.presented(at(now), true);
        }
        now += 25;
        stats.presented(at(now), true);
        now += 40;
        stats.presented(at(now), true);
        // Two seconds without a frame, then one more frame: idle, not an interval.
        now += 2000;
        stats.presented(at(now), false);
        let summary = stats.summary(at(now));
        assert_eq!(summary.presented, 22);
        assert_eq!((summary.partial_redraws, summary.full_redraws), (20, 2));
        assert_eq!(summary.partial_share, 0.91);
        assert_eq!(summary.intervals, 20);
        assert_eq!(summary.interval_max_ms, 40.);
        assert_eq!(summary.interval_p95_ms, 25.);
        assert!((summary.interval_mean_ms - 17.65).abs() < 1e-9);
        assert_eq!(
            (summary.intervals_over_20ms, summary.intervals_over_33ms),
            (2, 1)
        );
        assert_eq!(summary.idle_s, 2.4);
        assert_eq!(summary.foreground_s, 2.75);
        // A gap still running counts as idle; leaving the front stops every clock.
        assert_eq!(stats.summary(at(now + 500)).idle_s, 2.9);
        stats.set_foreground(at(now + 500), false);
        stats.presented(at(now + 600), false);
        let behind = stats.summary(at(now + 60_000));
        assert_eq!(behind.presented, 22);
        assert_eq!(behind.idle_s, 2.9);
        assert_eq!(behind.foreground_s, 3.25);
        // Back in front, the first frame starts a new run instead of a long interval.
        stats.set_foreground(at(now + 70_000), true);
        stats.presented(at(now + 70_010), false);
        assert_eq!(stats.summary(at(now + 70_010)).intervals, 20);
    }

    #[test]
    fn the_writer_thread_keeps_the_startup_facts_and_adds_the_latest_frame_stats() {
        let directory =
            std::env::temp_dir().join(format!("luma-frame-stats-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("luma-diagnostics.json");
        let writer = Writer::spawn(path.clone());
        writer.facts(serde_json::json!({"version": "test", "obtained_samples": 4}));
        let mut stats = FrameStats::default();
        let start = Instant::now();
        stats.set_foreground(start, true);
        stats.presented(start, true);
        stats.presented(start + Duration::from_millis(16), false);
        writer.frames(stats.summary(start + Duration::from_millis(16)));
        writer.finish();
        let read: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read["version"], "test");
        assert_eq!(read["obtained_samples"], 4);
        assert_eq!(read["frame_stats"]["presented"], 2);
        assert_eq!(read["frame_stats"]["intervals"], 1);
        assert_eq!(read["frame_stats"]["interval_mean_ms"], 16.0);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
