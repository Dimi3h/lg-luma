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

/// Histogram resolution: 0.25 ms buckets up to `RANGE_MS`; longer samples share the last.
const BUCKETS_PER_MS: f64 = 4.;
const RANGE_MS: f64 = 250.;
const BUCKETS: usize = (RANGE_MS * BUCKETS_PER_MS) as usize + 1;

/// Count, mean, 95th percentile and maximum of millisecond samples in constant memory: a sample
/// is a few additions and one histogram bucket, with nothing sorted or allocated.
pub struct Distribution {
    count: u64,
    sum_ms: f64,
    max_ms: f64,
    histogram: Box<[u32; BUCKETS]>,
}

impl Default for Distribution {
    fn default() -> Self {
        Self {
            count: 0,
            sum_ms: 0.,
            max_ms: 0.,
            histogram: Box::new([0; BUCKETS]),
        }
    }
}

/// A distribution as written to the file.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Spread {
    pub count: u64,
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

fn round(value: f64) -> f64 {
    (value * 100.).round() / 100.
}

impl Distribution {
    pub fn add(&mut self, ms: f64) {
        let ms = ms.max(0.);
        self.count += 1;
        self.sum_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        let bucket = ((ms * BUCKETS_PER_MS).ceil() as usize).min(BUCKETS - 1);
        self.histogram[bucket] += 1;
    }

    pub fn timing(&self) -> Spread {
        let p95 = if self.count == 0 {
            0.
        } else {
            let rank = (self.count * 95).div_ceil(100);
            let mut seen = 0;
            let bucket = self
                .histogram
                .iter()
                .position(|count| {
                    seen += u64::from(*count);
                    seen >= rank
                })
                .unwrap_or(BUCKETS - 1);
            // A bucket's upper edge, never above the largest sample seen.
            (bucket as f64 / BUCKETS_PER_MS).min(self.max_ms)
        };
        Spread {
            count: self.count,
            mean_ms: round(self.sum_ms / self.count.max(1) as f64),
            p95_ms: round(p95),
            max_ms: round(self.max_ms),
        }
    }
}

/// Frame statistics while Luma is in front. Luma draws only when something changed, so the
/// gaps between frames say nothing about smoothness; what is measured instead is how long a
/// key or pointer event takes to show (from handling the event to the presented frame that
/// reflects it) and how long each frame takes (scene build, submission and swap).
#[derive(Default)]
pub struct FrameStats {
    presented: u64,
    partial: u64,
    full: u64,
    input_to_frame: Distribution,
    frame_time: Distribution,
    over_16_7: u64,
    over_33: u64,
    foreground_ms: f64,
    /// While in front: when foreground time was last added up.
    counted: Option<Instant>,
    /// The first input not yet shown in a presented frame.
    input: Option<Instant>,
}

/// What `luma-diagnostics.json` records under `frame_stats`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FrameSummary {
    pub presented: u64,
    pub partial_redraws: u64,
    pub full_redraws: u64,
    pub partial_share: f64,
    /// From a key or pointer event to the presented frame that shows its effect.
    pub input_to_frame: Spread,
    /// Scene build, submission and swap of each presented frame.
    pub frame_time: Spread,
    pub frames_over_16_7ms: u64,
    pub frames_over_33ms: u64,
    pub foreground_s: f64,
}

fn millis(from: Instant, to: Instant) -> f64 {
    to.saturating_duration_since(from).as_secs_f64() * 1000.
}

impl FrameStats {
    /// Luma came to the front or left it. Time behind other apps is not counted.
    pub fn set_foreground(&mut self, now: Instant, foreground: bool) {
        match (foreground, self.counted) {
            (true, None) => self.counted = Some(now),
            (false, Some(counted)) => {
                self.foreground_ms += millis(counted, now);
                self.counted = None;
                self.input = None;
            }
            _ => {}
        }
    }

    /// A key or pointer event was handled; the next presented frame shows its effect.
    pub fn input(&mut self, at: Instant) {
        if self.counted.is_some() && self.input.is_none() {
            self.input = Some(at);
        }
    }

    /// The loop is done with this round: an input that changed nothing on screen is not timed.
    pub fn settle(&mut self) {
        self.input = None;
    }

    /// One presented frame: `frame_ms` from the start of its scene build to the end of its swap,
    /// `partial` when only a damaged region was redrawn.
    pub fn presented(&mut self, now: Instant, frame_ms: f64, partial: bool) {
        if self.counted.is_none() {
            return;
        }
        self.presented += 1;
        if partial {
            self.partial += 1;
        } else {
            self.full += 1;
        }
        self.frame_time.add(frame_ms);
        self.over_16_7 += u64::from(frame_ms > 16.7);
        self.over_33 += u64::from(frame_ms > 33.);
        if let Some(input) = self.input.take() {
            self.input_to_frame.add(millis(input, now));
        }
    }

    /// The figures up to `now`.
    pub fn summary(&self, now: Instant) -> FrameSummary {
        let foreground_ms = self.foreground_ms + self.counted.map_or(0., |at| millis(at, now));
        FrameSummary {
            presented: self.presented,
            partial_redraws: self.partial,
            full_redraws: self.full,
            partial_share: round(self.partial as f64 / self.presented.max(1) as f64),
            input_to_frame: self.input_to_frame.timing(),
            frame_time: self.frame_time.timing(),
            frames_over_16_7ms: self.over_16_7,
            frames_over_33ms: self.over_33,
            foreground_s: round(foreground_ms / 1000.),
        }
    }
}

/// How often the running statistics reach the file (and once more at exit).
pub const WRITE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

enum Message {
    Facts(serde_json::Value),
    Frames(FrameSummary),
    /// The latest preload decisions (Settings › General › Preload apps).
    Preload(serde_json::Value),
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
                let mut preload = None;
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Facts(value) => facts = value,
                        Message::Frames(summary) => frames = Some(summary),
                        Message::Preload(value) => preload = Some(value),
                    }
                    // Only the latest state matters: skip to it when messages queued up.
                    while let Ok(message) = receiver.try_recv() {
                        match message {
                            Message::Facts(value) => facts = value,
                            Message::Frames(summary) => frames = Some(summary),
                            Message::Preload(value) => preload = Some(value),
                        }
                    }
                    let mut value = facts.clone();
                    if let (Some(object), Some(summary)) = (value.as_object_mut(), frames) {
                        object.insert(
                            "frame_stats".into(),
                            serde_json::to_value(summary).unwrap_or_default(),
                        );
                    }
                    if let (Some(object), Some(decisions)) = (value.as_object_mut(), &preload) {
                        object.insert("preload".into(), decisions.clone());
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

    pub fn preload(&self, decisions: serde_json::Value) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Message::Preload(decisions));
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
    fn frame_stats_time_input_to_frame_and_each_frame_in_front_only() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut stats = FrameStats::default();
        // Behind another app nothing counts.
        stats.input(at(0));
        stats.presented(at(5), 4., false);
        stats.set_foreground(at(1000), true);
        // A key at 1000 ms shows in the frame presented 30 ms later (frame time 12 ms).
        stats.input(at(1000));
        stats.input(at(1010));
        stats.presented(at(1030), 12., true);
        // A frame without input (an animation step) only adds its frame time.
        stats.presented(at(1047), 20., true);
        // An input that changes nothing is not timed: the round settles without a frame.
        stats.input(at(2000));
        stats.settle();
        stats.presented(at(5000), 40., false);
        // Nineteen more inputs, each shown 20 ms later in a 10 ms frame.
        for step in 0..19 {
            let t = 6000 + step * 100;
            stats.input(at(t));
            stats.presented(at(t + 20), 10., true);
        }
        let summary = stats.summary(at(8000));
        assert_eq!(summary.presented, 22);
        assert_eq!((summary.partial_redraws, summary.full_redraws), (21, 1));
        assert_eq!(summary.input_to_frame.count, 20);
        assert_eq!(summary.input_to_frame.max_ms, 30.);
        assert_eq!(summary.input_to_frame.p95_ms, 20.);
        assert!((summary.input_to_frame.mean_ms - 20.5).abs() < 1e-9);
        assert_eq!(summary.frame_time.count, 22);
        assert_eq!(summary.frame_time.max_ms, 40.);
        assert_eq!(
            (summary.frames_over_16_7ms, summary.frames_over_33ms),
            (2, 1)
        );
        assert_eq!(summary.foreground_s, 7.);
        // Leaving the front stops every clock and drops a pending input.
        stats.input(at(8000));
        stats.set_foreground(at(8000), false);
        stats.presented(at(8010), 5., false);
        let behind = stats.summary(at(60_000));
        assert_eq!(behind.presented, 22);
        assert_eq!(behind.input_to_frame.count, 20);
        assert_eq!(behind.foreground_s, 7.);
        // Long samples share the last bucket but keep their true maximum.
        let mut long = Distribution::default();
        long.add(900.);
        assert_eq!(long.timing().max_ms, 900.);
        assert_eq!(long.timing().p95_ms, 250.);
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
        stats.input(start);
        stats.presented(start + Duration::from_millis(16), 9., false);
        writer.frames(stats.summary(start + Duration::from_millis(16)));
        writer.finish();
        let read: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read["version"], "test");
        assert_eq!(read["obtained_samples"], 4);
        assert_eq!(read["frame_stats"]["presented"], 1);
        assert_eq!(read["frame_stats"]["input_to_frame"]["count"], 1);
        assert_eq!(read["frame_stats"]["input_to_frame"]["mean_ms"], 16.0);
        assert_eq!(read["frame_stats"]["frame_time"]["max_ms"], 9.0);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
