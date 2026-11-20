//! Preloading the user's own apps (Settings › General › Preload apps).
//!
//! LG's preload manager starts some apps hidden so they open at once, but only apps in LG's own
//! policy, which cannot list other apps. Luma does the same for the user's most used apps: while
//! it is in front and idle, it starts one app at a time hidden (`launchHidden`, `preload:
//! partial`), only web apps (the only kind seen to stay hidden), only while enough memory is
//! available, and never one that is already running. An app that comes to the front anyway is
//! closed, Luma returns, and the app is never preloaded again. LG's own preloads of apps the user
//! hid in Luma are switched off through the elevated helper and switched on again when the app
//! is shown again or the setting is turned off. All TV calls run on this module's own thread.
use crate::model::App;
use serde::Serialize;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashSet},
    sync::mpsc,
    thread,
    time::Duration,
};

/// At most this many of the user's apps are kept preloaded.
pub const MAX_APPS: usize = 3;
/// No preload starts below this much available memory (`MemAvailable`, MB). On the TV this was
/// tuned on (1.7 GB RAM, swap in heavy use), one preloaded web app had 74 MB resident.
pub const MEMORY_FLOOR_MB: u64 = 350;
/// Luma must have been in front without input this long before a preload starts.
pub const IDLE: Duration = Duration::from_secs(8);
/// At least this long between two preloads.
pub const SPACING: Duration = Duration::from_secs(20);
/// How long after a hidden launch the foreground is checked.
pub const FOREGROUND_CHECK: Duration = Duration::from_secs(3);
/// Launch counts kept in the preferences (the most launched ones).
const MAX_COUNTS: usize = 64;

/// The apps worth keeping preloaded, best first: web apps only, never one that came forward
/// before, ranked by launches from Luma and then by their place in the grid.
pub fn chosen<'a>(
    apps: &[&'a App],
    counts: &BTreeMap<String, u32>,
    blocked: &[String],
) -> Vec<&'a App> {
    let mut ranked: Vec<(usize, &App)> = apps
        .iter()
        .enumerate()
        .filter(|(_, app)| app.web && !blocked.contains(&app.id))
        .map(|(index, app)| (index, *app))
        .collect();
    ranked.sort_by_key(|(index, app)| (Reverse(counts.get(&app.id).copied().unwrap_or(0)), *index));
    ranked
        .into_iter()
        .take(MAX_APPS)
        .map(|(_, app)| app)
        .collect()
}

/// The next app to preload: the best chosen one not tried since Luma last came to the front.
pub fn next(
    apps: &[&App],
    counts: &BTreeMap<String, u32>,
    blocked: &[String],
    tried: &HashSet<String>,
) -> Option<String> {
    chosen(apps, counts, blocked)
        .into_iter()
        .find(|app| !tried.contains(&app.id))
        .map(|app| app.id.clone())
}

/// Counts one launch from Luma, keeping only the most launched apps.
pub fn count_launch(counts: &mut BTreeMap<String, u32>, id: &str) {
    let count = counts.entry(id.to_owned()).or_default();
    *count = count.saturating_add(1);
    while counts.len() > MAX_COUNTS {
        // The least launched other app goes (never the one just counted).
        let Some(least) = counts
            .iter()
            .filter(|(other, _)| other.as_str() != id)
            .min_by_key(|(other, count)| (**count, (*other).clone()))
            .map(|(other, _)| other.clone())
        else {
            break;
        };
        counts.remove(&least);
    }
}

/// `MemAvailable` from `/proc/meminfo`, in MB.
pub fn mem_available_mb(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb / 1024)
}

/// Whether memory allows one more preload; the reason when it does not.
pub fn memory_allows(meminfo: Option<&str>) -> Result<u64, Outcome> {
    match meminfo.and_then(mem_available_mb) {
        None => Err(Outcome::MemoryUnknown),
        Some(mb) if mb < MEMORY_FLOOR_MB => Err(Outcome::LowMemory(mb)),
        Some(mb) => Ok(mb),
    }
}

/// LG's preload policy changes for the hidden-app rule: with the setting on, LG's preloads of
/// apps hidden in Luma go off and the ones Luma switched off earlier for apps shown again come
/// back on; with it off, everything Luma switched off comes back on. Entries for apps the user
/// did not hide are never touched (the helper also skips ids LG's policy does not list).
pub fn lg_policy_change(
    enabled: bool,
    hidden: &[String],
    disabled_by_luma: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut disable: Vec<String> = if enabled { hidden.to_vec() } else { vec![] };
    disable.sort();
    disable.dedup();
    let mut enable: Vec<String> = disabled_by_luma
        .iter()
        .filter(|id| !disable.contains(id))
        .cloned()
        .collect();
    enable.sort();
    enable.dedup();
    (disable, enable)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Preloaded {
        available_mb: u64,
    },
    AlreadyRunning,
    LowMemory(u64),
    MemoryUnknown,
    /// It came to the front although launched hidden: closed, never preloaded again.
    CameForward,
    Failed(String),
}

impl Outcome {
    pub fn describe(&self) -> String {
        match self {
            Self::Preloaded { available_mb } => {
                format!("preloaded hidden ({available_mb} MB available before)")
            }
            Self::AlreadyRunning => "skipped: already running".into(),
            Self::LowMemory(mb) => {
                format!("skipped: {mb} MB available, below {MEMORY_FLOOR_MB} MB")
            }
            Self::MemoryUnknown => "skipped: available memory unknown".into(),
            Self::CameForward => "came to the front: closed, never preloaded again".into(),
            Self::Failed(error) => format!("failed: {error}"),
        }
    }
}

/// One line of `preload` in `luma-diagnostics.json`.
#[derive(Clone, Debug, Serialize)]
pub struct Decision {
    pub at: String,
    pub id: String,
    pub result: String,
}

/// The most recent decisions shown in the diagnostics file.
pub const DECISIONS: usize = 24;

pub enum Job {
    Preload {
        id: String,
        own: &'static str,
    },
    Policy {
        disable: Vec<String>,
        enable: Vec<String>,
    },
}

pub enum Reply {
    Preloaded {
        id: String,
        outcome: Outcome,
    },
    /// The ids Luma switched off and on in LG's policy.
    Policy(Result<(Vec<String>, Vec<String>), String>),
}

/// A thread for preload work: a preload waits seconds to check the foreground, which must never
/// hold up the platform worker that launches the apps the user picks.
pub struct Service {
    sender: mpsc::Sender<Job>,
    pub receiver: mpsc::Receiver<Reply>,
}

impl Service {
    pub fn spawn() -> Self {
        let (sender, jobs) = mpsc::channel::<Job>();
        let (replies, receiver) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("luma-preload".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let reply = match job {
                        Job::Preload { id, own } => Reply::Preloaded {
                            outcome: crate::platform::preload_app(&id, own),
                            id,
                        },
                        Job::Policy { disable, enable } => {
                            Reply::Policy(crate::platform::preload_policy(&disable, &enable))
                        }
                    };
                    if replies.send(reply).is_err() {
                        break;
                    }
                }
            });
        Self { sender, receiver }
    }

    pub fn send(&self, job: Job) {
        let _ = self.sender.send(job);
    }
}
