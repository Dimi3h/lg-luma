//! Settings › System › Repair Bluetooth controller: a watcher that repairs a BLE gamepad whose
//! input LG's Bluetooth stack stopped delivering (see `luma_tv_helper::bluetooth` for the
//! measured cause and repair). It runs on its own thread, also while Luma is in the background,
//! because the game is what runs in front. Decisions go to the main loop for the diagnostics.
use luma_tv_helper::bluetooth::{Gamepad, Watch};
use serde::Serialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// How often the connected devices are read.
pub const STATUS_EVERY: Duration = Duration::from_secs(3);
/// How often a connected gamepad's GATT cache file is checked (Luma running as root).
pub const CACHE_EVERY: Duration = Duration::from_secs(1);
/// Without root every read starts the elevated helper: with no gamepad connected, only this
/// often.
pub const HELPER_IDLE_EVERY: Duration = Duration::from_secs(10);

/// One repair, for the log and `gamepad_repair` in `luma-diagnostics.json`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Repair {
    pub at: String,
    pub address: String,
    pub name: String,
    pub outcome: String,
}

/// `gamepad_repair` in `luma-diagnostics.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Evidence {
    pub repairs: u32,
    pub last: Option<Repair>,
    /// Why the watcher cannot work on this TV, if it cannot (no GATT cache, no access).
    pub inactive: Option<String>,
    /// The gamepads currently watched.
    pub watching: Vec<String>,
}

impl Evidence {
    pub fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Repaired(repair) => {
                self.repairs += 1;
                self.last = Some(repair);
            }
            Reply::Inactive(reason) => self.inactive = reason,
            Reply::Watching(addresses) => self.watching = addresses,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Repaired(Repair),
    /// `None` once it works again.
    Inactive(Option<String>),
    Watching(Vec<String>),
}

/// What the watcher needs from the TV, so its decisions can be tested without one.
pub trait Tv {
    /// The BLE gamepads with HID connected, with their GATT cache file's time when it is
    /// readable in the same pass (the elevated helper reads both at once).
    fn gamepads(&self) -> Result<Vec<(Gamepad, Option<u128>)>, String>;
    /// The cache file's time read directly, when Luma runs as root; `None` otherwise.
    fn cache_mtime(&self, address: &str) -> Option<u128>;
    /// Whether `cache_mtime` reads the file (Luma runs as root).
    fn direct(&self) -> bool;
    fn repair(&self, address: &str) -> Result<String, String>;
}

/// One pass of the watcher: what it found and repaired. `pads` keeps the last device read
/// between status reads.
pub struct Pass<'a> {
    pub watch: &'a mut Watch,
    pub pads: &'a mut Vec<(Gamepad, Option<u128>)>,
    pub read_status: bool,
}

impl Pass<'_> {
    /// Returns the replies for the main loop; repairs run here, one at a time.
    pub fn run(self, tv: &impl Tv, now: impl Fn() -> String) -> Vec<Reply> {
        let mut replies = vec![];
        if self.read_status {
            match tv.gamepads() {
                Ok(pads) => *self.pads = pads,
                Err(error) => {
                    self.pads.clear();
                    replies.push(Reply::Inactive(Some(format!(
                        "connected devices unreadable: {error}"
                    ))));
                    return replies;
                }
            }
        } else if tv.direct() {
            for (pad, mtime) in self.pads.iter_mut() {
                *mtime = tv.cache_mtime(&pad.address);
            }
        }
        let missing: Vec<&str> = self
            .pads
            .iter()
            .filter(|(_, mtime)| mtime.is_none())
            .map(|(pad, _)| pad.address.as_str())
            .collect();
        if self.read_status {
            replies
                .push(Reply::Inactive((!missing.is_empty()).then(|| {
                    format!("no GATT cache file for {}", missing.join(", "))
                })));
        }
        let connected: Vec<(String, Option<u128>)> = self
            .pads
            .iter()
            .map(|(pad, mtime)| (pad.address.clone(), *mtime))
            .collect();
        for address in self.watch.observe(&connected) {
            let name = self
                .pads
                .iter()
                .find(|(pad, _)| pad.address == address)
                .map(|(pad, _)| pad.name.clone())
                .unwrap_or_default();
            let at = now();
            eprintln!(
                "Luma gamepad: {at} {name} {address}: input lost after Service Changed; repairing"
            );
            let outcome = tv
                .repair(&address)
                .unwrap_or_else(|error| format!("failed: {error}"));
            let done = now();
            eprintln!("Luma gamepad: {done} {name} {address}: {outcome}");
            let mtime = if tv.direct() {
                tv.cache_mtime(&address)
            } else {
                tv.gamepads().ok().and_then(|pads| {
                    pads.into_iter()
                        .find(|(pad, _)| pad.address == address)
                        .and_then(|(_, mtime)| mtime)
                })
            };
            self.watch.repaired(&address, mtime);
            replies.push(Reply::Repaired(Repair {
                at,
                address,
                name,
                outcome,
            }));
        }
        if self.read_status {
            replies.push(Reply::Watching(self.watch.watching()));
        }
        replies
    }
}

pub struct Service {
    enabled: Arc<AtomicBool>,
    pub receiver: mpsc::Receiver<Reply>,
}

impl Service {
    pub fn spawn(enabled: bool, tv: impl Tv + Send + 'static) -> Self {
        let flag = Arc::new(AtomicBool::new(enabled));
        let (replies, receiver) = mpsc::channel();
        let shared = flag.clone();
        let _ = thread::Builder::new()
            .name("luma-gamepad".into())
            .spawn(move || {
                let mut watch = Watch::default();
                let mut pads = vec![];
                let mut status_at: Option<Instant> = None;
                loop {
                    if !shared.load(Ordering::Relaxed) {
                        watch = Watch::default();
                        pads.clear();
                        status_at = None;
                        thread::sleep(STATUS_EVERY);
                        continue;
                    }
                    let read_status = status_at.is_none_or(|at| at.elapsed() >= STATUS_EVERY);
                    if read_status {
                        status_at = Some(Instant::now());
                    }
                    let pass = Pass {
                        watch: &mut watch,
                        pads: &mut pads,
                        read_status,
                    };
                    for reply in pass.run(&tv, now) {
                        if replies.send(reply).is_err() {
                            return;
                        }
                    }
                    // While a gamepad is connected, its cache file is checked every second
                    // when Luma can read it itself; otherwise every status read.
                    thread::sleep(match (pads.is_empty(), tv.direct()) {
                        (false, true) => CACHE_EVERY,
                        (true, false) => HELPER_IDLE_EVERY,
                        _ => STATUS_EVERY,
                    });
                }
            });
        Self {
            enabled: flag,
            receiver,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
