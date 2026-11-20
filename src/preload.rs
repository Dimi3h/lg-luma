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

/// The automatic choice (no list of the user's own) keeps at most this many apps alive.
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

/// The apps worth keeping preloaded, best first.
///
/// - With the user's own list (`preload_list`): every listed app in grid order, web or native,
///   with no fixed number; memory, swap and the apps already running decide how many load.
/// - Without one: the automatic choice, up to `MAX_APPS` web apps ranked by launches from Luma
///   and then grid order.
///
/// Never an app that came forward when preloaded (`blocked`).
pub fn chosen<'a>(
    apps: &[&'a App],
    counts: &BTreeMap<String, u32>,
    blocked: &[String],
    list: &[String],
) -> Vec<&'a App> {
    if !list.is_empty() {
        return apps
            .iter()
            .filter(|app| list.contains(&app.id) && !blocked.contains(&app.id))
            .copied()
            .collect();
    }
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

/// How many of the user's apps may run before preloading stops: `MAX_APPS` for the automatic
/// choice; none for the user's own list, where memory and swap decide.
pub fn running_limit(list: &[String]) -> Option<usize> {
    list.is_empty().then_some(MAX_APPS)
}

/// The next app to preload: the best chosen one not tried since Luma last came to the front.
/// Apps in LG's own preload policy (`lg_listed`) are left to LG, which preloads them itself.
pub fn next(
    apps: &[&App],
    counts: &BTreeMap<String, u32>,
    blocked: &[String],
    list: &[String],
    tried: &HashSet<String>,
    lg_listed: &[String],
) -> Option<String> {
    chosen(apps, counts, blocked, list)
        .into_iter()
        .find(|app| !tried.contains(&app.id) && !lg_listed.contains(&app.id))
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
    meminfo_mb(meminfo, "MemAvailable")
}

/// How many of the user's apps run already (Luma not counted): they count towards `MAX_APPS`,
/// since a fourth or fifth app kept alive adds memory pressure for little gain.
pub fn running_user_apps(running: &[String], user_apps: &[String]) -> usize {
    running.iter().filter(|id| user_apps.contains(id)).count()
}

/// A `/proc/meminfo` field in MB.
fn meminfo_mb(meminfo: &str, field: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix(field)?.strip_prefix(':'))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb / 1024)
}

/// Whether memory allows one more preload; the reason when it does not. Besides available
/// memory, swap must not be mostly used: on a TV that swaps heavily a hidden app only pushes
/// others out. Luma never closes anything when memory runs low later (LG's memory manager does);
/// it just stops preloading.
pub fn memory_allows(meminfo: Option<&str>) -> Result<u64, Outcome> {
    let Some(meminfo) = meminfo else {
        return Err(Outcome::MemoryUnknown);
    };
    let available = match mem_available_mb(meminfo) {
        None => return Err(Outcome::MemoryUnknown),
        Some(mb) if mb < MEMORY_FLOOR_MB => return Err(Outcome::LowMemory(mb)),
        Some(mb) => mb,
    };
    if let (Some(total), Some(free)) = (
        meminfo_mb(meminfo, "SwapTotal"),
        meminfo_mb(meminfo, "SwapFree"),
    ) && total > 0
        && free * SWAP_FREE_SHARE < total
    {
        return Err(Outcome::LowSwap { free, total });
    }
    Ok(available)
}

/// Swap must be at least this share free (1/3) before a preload.
pub const SWAP_FREE_SHARE: u64 = 3;

/// LG's preload policy changes, as `(disable, enable)` for the helper (which skips ids LG's policy
/// does not list and only switches entries that differ):
///
/// - with the setting on, LG's preloads of apps hidden in Luma go off and LG's preloads of apps
///   in the user's list go on (hidden wins);
/// - entries Luma switched off before (`disabled_by_luma`) come back on once not wanted off, and
///   entries Luma switched on (`enabled_by_luma`) go back off once not wanted on;
/// - with the setting off, everything Luma changed goes back.
///
/// Entries Luma never changed and that are neither hidden nor listed are not touched.
pub fn lg_policy_change(
    enabled: bool,
    hidden: &[String],
    list: &[String],
    disabled_by_luma: &[String],
    enabled_by_luma: &[String],
) -> (Vec<String>, Vec<String>) {
    let want_off: Vec<&String> = if enabled {
        hidden.iter().collect()
    } else {
        vec![]
    };
    let want_on: Vec<&String> = if enabled {
        list.iter().filter(|id| !hidden.contains(id)).collect()
    } else {
        vec![]
    };
    let mut disable: Vec<String> = want_off
        .iter()
        .map(|id| (*id).clone())
        .chain(
            enabled_by_luma
                .iter()
                .filter(|id| !want_on.contains(id))
                .cloned(),
        )
        .collect();
    let mut enable: Vec<String> = want_on
        .iter()
        .map(|id| (*id).clone())
        .chain(
            disabled_by_luma
                .iter()
                .filter(|id| !want_off.contains(id))
                .cloned(),
        )
        .collect();
    disable.sort();
    disable.dedup();
    enable.sort();
    enable.dedup();
    (disable, enable)
}

/// Which LG policy entries Luma has changed, after the helper switched `off` and `on`: switching
/// back an entry Luma changed earlier restores LG's own value instead of being remembered.
pub fn record_policy_change(
    off: &[String],
    on: &[String],
    disabled_by_luma: &mut Vec<String>,
    enabled_by_luma: &mut Vec<String>,
) {
    for id in off {
        if let Some(index) = enabled_by_luma.iter().position(|other| other == id) {
            enabled_by_luma.remove(index);
        } else if !disabled_by_luma.contains(id) {
            disabled_by_luma.push(id.clone());
        }
    }
    for id in on {
        if let Some(index) = disabled_by_luma.iter().position(|other| other == id) {
            disabled_by_luma.remove(index);
        } else if !enabled_by_luma.contains(id) {
            enabled_by_luma.push(id.clone());
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Preloaded {
        available_mb: u64,
    },
    AlreadyRunning,
    LowMemory(u64),
    LowSwap {
        free: u64,
        total: u64,
    },
    /// Enough of the user's apps are alive already (preloaded or used).
    EnoughRunning(usize),
    MemoryUnknown,
    /// It came to the front although launched hidden: closed, never preloaded again.
    CameForward,
    Failed(String),
}

impl Outcome {
    /// A reason to stop preloading until Luma comes back to the front.
    pub fn pauses(&self) -> bool {
        matches!(
            self,
            Self::LowMemory(_)
                | Self::LowSwap { .. }
                | Self::EnoughRunning(_)
                | Self::MemoryUnknown
        )
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Preloaded { available_mb } => {
                format!("preloaded hidden ({available_mb} MB available before)")
            }
            Self::AlreadyRunning => "skipped: already running".into(),
            Self::LowMemory(mb) => {
                format!("skipped: {mb} MB available, below {MEMORY_FLOOR_MB} MB")
            }
            Self::LowSwap { free, total } => {
                format!("skipped: {free} of {total} MB swap free, below a third")
            }
            Self::EnoughRunning(count) => {
                format!("skipped: {count} of your apps already run (limit {MAX_APPS})")
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
    /// `user_apps`: the ids in Luma's grid, to count which of them run already, against
    /// `limit` (none for the user's own list).
    Preload {
        id: String,
        own: &'static str,
        user_apps: Vec<String>,
        limit: Option<usize>,
    },
    Policy {
        disable: Vec<String>,
        enable: Vec<String>,
    },
    /// Settings › System › Memory compression switched (through the elevated helper).
    MemoryCompression(bool),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolicyChange {
    pub off: Vec<String>,
    pub on: Vec<String>,
    pub listed: Vec<String>,
}

pub enum Reply {
    Preloaded {
        id: String,
        outcome: Outcome,
    },
    /// The ids Luma switched off and on in LG's policy, and every id LG's policy lists.
    Policy(Result<PolicyChange, String>),
    MemoryCompression(Result<String, String>),
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
                        Job::Preload {
                            id,
                            own,
                            user_apps,
                            limit,
                        } => Reply::Preloaded {
                            outcome: crate::platform::preload_app(&id, own, &user_apps, limit),
                            id,
                        },
                        Job::Policy { disable, enable } => {
                            Reply::Policy(crate::platform::preload_policy(&disable, &enable))
                        }
                        Job::MemoryCompression(on) => {
                            Reply::MemoryCompression(crate::platform::memory_compression(on))
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
