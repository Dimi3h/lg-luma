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
    time::{Duration, Instant},
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
/// How often LG's memory state and the running apps are polled for the diagnostics.
pub const WATCH: Duration = Duration::from_secs(30);
/// How long after a hidden launch the foreground is checked.
pub const FOREGROUND_CHECK: Duration = Duration::from_secs(3);
/// How long after a hidden launch the app is still watched for raising itself (Apple TV came
/// forward later than the first check). Bounded, because the remote's own app buttons
/// (Netflix, Prime Video, Disney+) open an app without passing through Luma: a press on a
/// preloaded app after this would otherwise be taken for the app raising itself.
pub const RAISE_WINDOW: Duration = Duration::from_secs(90);
/// After Luma loses focus inside that window, the foreground is checked this soon...
pub const RAISE_FIRST_CHECK: Duration = Duration::from_secs(1);
/// ...and again this often while Luma stays out of focus.
pub const RAISE_CHECK: Duration = Duration::from_secs(5);
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

/// The memory LG's policy reserves per preloaded app (`requiredMemory` in its entries, MB).
pub const REQUIRED_MB: u64 = 100;
/// Memory pressure (`/proc/pressure/memory`, "some avg10", %) above which a preload waits.
pub const PRESSURE_WAIT: f64 = 10.;

/// LG's memory manager's view (`com.webos.memorymanager/getCurrentMemState`): its level
/// ("normal", "medium", "low", "critical") and the memory it counts usable, in MB.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LgMemory {
    pub level: String,
    pub usable: u64,
}

impl LgMemory {
    pub fn from_reply(reply: &serde_json::Value) -> Option<Self> {
        Some(Self {
            level: reply["currentLevel"].as_str()?.to_owned(),
            usable: reply["usable"].as_u64()?,
        })
    }
}

/// `memBoundary.medium.enter` from `com.webos.memorymanager/getPolicy` (MB): below it LG's
/// memory manager enters "medium".
pub fn medium_enter(policy: &serde_json::Value) -> Option<u64> {
    policy["memBoundary"]["medium"]["enter"]
        .as_u64()
        .or_else(|| {
            policy
                .as_object()?
                .values()
                .find_map(|value| value["memBoundary"]["medium"]["enter"].as_u64())
        })
}

/// "some avg10" from `/proc/pressure/memory`, in %.
pub fn pressure_some_avg10(pressure: &str) -> Option<f64> {
    pressure
        .lines()
        .find(|line| line.starts_with("some"))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// Whether one more preload fits, deciding on LG's memory manager where it answers: its level
/// must be "normal" and its usable memory at least the medium boundary plus LG's own
/// `requiredMemory` (350 MB on the TV measured). Only without an answer does `MemAvailable` (350
/// MB) decide. Swap must be a third free either way, and high memory pressure waits. Returns
/// what allowed it, for the diagnostics.
pub fn memory_check(
    lg: Option<&LgMemory>,
    medium_enter: Option<u64>,
    meminfo: Option<&str>,
    pressure: Option<f64>,
) -> Result<String, Outcome> {
    let allowed = match (lg, medium_enter) {
        (Some(lg), Some(enter)) => {
            let needed = enter + REQUIRED_MB;
            if lg.level != "normal" {
                return Err(Outcome::LgLevel {
                    level: lg.level.clone(),
                    usable: lg.usable,
                });
            }
            if lg.usable < needed {
                return Err(Outcome::LgLow {
                    usable: lg.usable,
                    needed,
                });
            }
            format!(
                "LG level normal, {} MB usable of {needed} needed",
                lg.usable
            )
        }
        _ => {
            let available = memory_allows(meminfo)?;
            format!("{available} MB available; LG's memory state unknown")
        }
    };
    if let Some(meminfo) = meminfo
        && let (Some(total), Some(free)) = (
            meminfo_mb(meminfo, "SwapTotal"),
            meminfo_mb(meminfo, "SwapFree"),
        )
        && total > 0
        && free * SWAP_FREE_SHARE < total
    {
        return Err(Outcome::LowSwap { free, total });
    }
    if let Some(pressure) = pressure
        && pressure > PRESSURE_WAIT
    {
        return Err(Outcome::Pressure(pressure));
    }
    Ok(allowed)
}

/// What Luma watches to judge memory compression after a few days: LG's memory level now, how
/// often it entered medium, low and critical since Luma started, and how many apps of the
/// user's preload list disappeared from the running list without Luma closing them.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Evidence {
    pub level: Option<String>,
    pub usable_mb: Option<u64>,
    pub medium_enter_mb: Option<u64>,
    pub samples: u64,
    pub entered_medium: u64,
    pub entered_low: u64,
    pub entered_critical: u64,
    /// Listed apps that were running and then were not, without a close from Luma.
    pub listed_apps_gone: BTreeMap<String, u32>,
    #[serde(skip)]
    running: Option<Vec<String>>,
    /// Apps Luma itself closed (Recents, or one that came forward): not counted as gone.
    #[serde(skip)]
    pub closed_by_luma: HashSet<String>,
}

impl Evidence {
    /// One poll: LG's state (when it answered) and the running apps (when known).
    pub fn sample(
        &mut self,
        lg: Option<&LgMemory>,
        medium_enter: Option<u64>,
        running: Option<Vec<String>>,
        listed: &[String],
    ) {
        self.samples += 1;
        if medium_enter.is_some() {
            self.medium_enter_mb = medium_enter;
        }
        if let Some(lg) = lg {
            if self.level.as_deref() != Some(lg.level.as_str()) {
                match lg.level.as_str() {
                    "medium" => self.entered_medium += 1,
                    "low" => self.entered_low += 1,
                    "critical" => self.entered_critical += 1,
                    _ => {}
                }
            }
            self.level = Some(lg.level.clone());
            self.usable_mb = Some(lg.usable);
        }
        let Some(running) = running else {
            return;
        };
        if let Some(before) = &self.running {
            for id in before
                .iter()
                .filter(|id| listed.contains(id) && !running.contains(id))
            {
                if !self.closed_by_luma.remove(id) {
                    *self.listed_apps_gone.entry(id.clone()).or_default() += 1;
                }
            }
        }
        self.running = Some(running);
    }
}

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
    /// What allowed it (LG's numbers, or available memory).
    Preloaded {
        memory: String,
    },
    /// LG's memory manager is not at "normal" (an app starting, say): wait and try again.
    LgLevel {
        level: String,
        usable: u64,
    },
    /// LG counts less memory usable than its medium boundary plus `requiredMemory`.
    LgLow {
        usable: u64,
        needed: u64,
    },
    /// Memory pressure (some avg10, %) is high: wait and try again.
    Pressure(f64),
    AlreadyRunning,
    LowMemory(u64),
    LowSwap {
        free: u64,
        total: u64,
    },
    /// Enough of the user's apps are alive already (preloaded or used).
    EnoughRunning(usize),
    MemoryUnknown,
    /// It came to the front although launched hidden (`after` seconds after the launch):
    /// closed, never preloaded again.
    CameForward {
        after: u64,
    },
    Failed(String),
}

impl Outcome {
    /// A reason to stop preloading until Luma comes back to the front.
    pub fn pauses(&self) -> bool {
        matches!(
            self,
            Self::LowMemory(_)
                | Self::LowSwap { .. }
                | Self::LgLow { .. }
                | Self::EnoughRunning(_)
                | Self::MemoryUnknown
        )
    }

    /// A passing condition (LG's level during an app start, pressure): the same app is tried
    /// again after the usual spacing.
    pub fn waits(&self) -> bool {
        matches!(self, Self::LgLevel { .. } | Self::Pressure(_))
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Preloaded { memory } => format!("preloaded hidden ({memory})"),
            Self::LgLevel { level, usable } => {
                format!("waiting: LG memory level {level} ({usable} MB usable)")
            }
            Self::LgLow { usable, needed } => {
                format!("skipped: LG counts {usable} MB usable, {needed} MB needed")
            }
            Self::Pressure(avg10) => {
                format!("waiting: memory pressure {avg10:.1}% (above {PRESSURE_WAIT:.0}%)")
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
            Self::CameForward { after } => format!(
                "came to the front {after} s after the hidden launch: closed, never preloaded again"
            ),
            Self::Failed(error) => format!("failed: {error}"),
        }
    }
}

/// A hidden launch inside its watch window (`RAISE_WINDOW`). No other preload starts until it
/// ends, so only one app is ever watched.
#[derive(Clone, Debug, PartialEq)]
pub struct HiddenLaunch {
    pub id: String,
    pub at: Instant,
    /// When the foreground is next checked; set when Luma loses focus.
    check: Option<Instant>,
}

impl HiddenLaunch {
    pub fn new(id: String, at: Instant) -> Self {
        Self {
            id,
            at,
            check: None,
        }
    }

    pub fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) >= RAISE_WINDOW
    }

    /// Seconds since the hidden launch.
    pub fn after(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.at).as_secs()
    }

    /// Luma lost focus or was hidden: check the foreground soon.
    pub fn focus_lost(&mut self, now: Instant) {
        let soon = now + RAISE_FIRST_CHECK;
        self.check = Some(self.check.map_or(soon, |check| check.min(soon)));
    }

    /// Whether a foreground check is due now. With Luma in focus nothing else is in front, so
    /// no check is due and none stays scheduled.
    pub fn due(&mut self, now: Instant, focused: bool) -> bool {
        if focused {
            self.check = None;
            return false;
        }
        if self.check.is_some_and(|check| check <= now) {
            self.check = None;
            return true;
        }
        false
    }

    /// A check found it hidden: check again while Luma stays out of focus.
    pub fn checked(&mut self, now: Instant, focused: bool) {
        if !focused && !self.expired(now) {
            self.check = Some(now + RAISE_CHECK);
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
    /// A poll of LG's memory state and the running apps, for the diagnostics.
    Watch,
    /// Inside a hidden launch's watch window, Luma is out of focus: if `id` is in front, close
    /// it and bring `own` back.
    Check {
        id: String,
        own: &'static str,
        after: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolicyChange {
    pub off: Vec<String>,
    pub on: Vec<String>,
    pub listed: Vec<String>,
    /// The entries LG keeps warm itself ("periodic"); Luma does not launch those.
    pub periodic: Vec<String>,
}

pub enum Reply {
    Preloaded {
        id: String,
        outcome: Outcome,
    },
    /// The ids Luma switched off and on in LG's policy, and every id LG's policy lists.
    Policy(Result<PolicyChange, String>),
    MemoryCompression(Result<String, String>),
    Watch {
        lg: Option<LgMemory>,
        medium_enter: Option<u64>,
        running: Option<Vec<String>>,
    },
    /// `raised`: it was in front and was closed.
    Checked {
        id: String,
        after: u64,
        raised: bool,
    },
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
                        Job::Watch => Reply::Watch {
                            lg: crate::platform::lg_memory(),
                            medium_enter: crate::platform::lg_medium_enter(),
                            running: crate::platform::running_apps().ok(),
                        },
                        Job::Check { id, own, after } => Reply::Checked {
                            raised: crate::platform::close_if_in_front(&id, own),
                            id,
                            after,
                        },
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
