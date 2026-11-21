//! Settings › System › Screen Share listening off: keeps the TV's Wi-Fi Direct listening
//! (`listenState` of `com.webos.service.wifi/p2p`) disabled, because while it listens the radio
//! leaves the access point's channel twice a second (see `luma_tv_helper::wifi` for what was
//! measured). Runs on its own thread, also while Luma is in the background: a game or stream is
//! what runs in front. LG's own Screen Share app is never fought, and switching the setting off
//! only enables listening again when Luma was the one that disabled it.
use serde::Serialize;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// LG's Screen Share (Miracast) app: while it is in front the state is left alone.
pub const SCREEN_SHARE: &str = "com.webos.app.miracast";
/// How often the state is read besides the subscription's replies.
pub const POLL: Duration = Duration::from_secs(60);
/// At least this long between two changes by Luma, so a TV that switches listening straight
/// back on is never fought in a tight loop.
pub const MIN_GAP: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Disable,
    Enable,
}

/// What to do for the setting (`on`), the current `listen_state`, the foreground app and
/// whether Luma disabled listening itself.
pub fn decide(
    on: bool,
    listen_state: &str,
    front: Option<&str>,
    disabled_by_luma: bool,
) -> Option<Action> {
    match (on, listen_state) {
        (true, "enabled") if front != Some(SCREEN_SHARE) => Some(Action::Disable),
        (false, "disabled") if disabled_by_luma => Some(Action::Enable),
        _ => None,
    }
}

/// The last change Luma made.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Change {
    pub at: String,
    pub action: &'static str,
    pub result: String,
}

/// `wifi_direct` in `luma-diagnostics.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Evidence {
    /// `listenState` as last read.
    pub listen_state: Option<String>,
    /// `listenState` at the first read since Luma started (whether a disable survived a
    /// reboot or a restart).
    pub listen_state_at_start: Option<String>,
    pub disabled_by_luma: bool,
    /// How often Luma had to disable listening again after it had disabled it.
    pub disabled_again: u32,
    pub last_change: Option<Change>,
    /// Whether the TV accepted a subscription to `p2p/getstate` (otherwise only the poll).
    pub subscribed: Option<bool>,
    /// The last read or change that failed.
    pub error: Option<String>,
}

/// What the keeper needs from the TV, so its decisions can be tested without one.
pub trait P2p {
    /// The current `listenState`.
    fn state(&self) -> Result<String, String>;
    /// Sets `listenState`; returns it as read afterwards.
    fn set(&self, listen: bool) -> Result<String, String>;
    fn foreground(&self) -> Option<String>;
}

pub struct Keeper {
    pub on: bool,
    /// Luma's earlier disable held at the first read: a later disable is one "again".
    held: bool,
    last_set: Option<Instant>,
    pub evidence: Evidence,
}

impl Keeper {
    pub fn new(on: bool, disabled_by_luma: bool) -> Self {
        Self {
            on,
            held: false,
            last_set: None,
            evidence: Evidence {
                disabled_by_luma,
                ..Default::default()
            },
        }
    }

    /// As if the last change was longer than `MIN_GAP` ago.
    #[cfg(test)]
    pub fn after_gap(&mut self) {
        self.last_set = None;
    }

    /// One check, with the state a subscription reply carried or read now.
    pub fn check(&mut self, p2p: &impl P2p, known: Option<String>, now: impl Fn() -> String) {
        let evidence = &mut self.evidence;
        let listen = match known.map_or_else(|| p2p.state(), Ok) {
            Ok(listen) => listen,
            Err(error) => {
                evidence.error = Some(error);
                return;
            }
        };
        evidence.error = None;
        if evidence.listen_state_at_start.is_none() {
            evidence.listen_state_at_start = Some(listen.clone());
            self.held = listen == "disabled" && evidence.disabled_by_luma;
        }
        evidence.listen_state = Some(listen.clone());
        // Enabled again by someone else while the setting is off: no longer Luma's to restore.
        if !self.on && listen == "enabled" {
            evidence.disabled_by_luma = false;
        }
        let front = (self.on && listen == "enabled")
            .then(|| p2p.foreground())
            .flatten();
        let Some(action) = decide(
            self.on,
            &listen,
            front.as_deref(),
            evidence.disabled_by_luma,
        ) else {
            return;
        };
        if self.last_set.is_some_and(|last| last.elapsed() < MIN_GAP) {
            return;
        }
        self.last_set = Some(Instant::now());
        let at = now();
        let result = p2p.set(action == Action::Enable);
        let (name, outcome) = match (&result, action) {
            (Ok(state), Action::Disable) => {
                if self.held {
                    evidence.disabled_again += 1;
                }
                self.held = true;
                evidence.disabled_by_luma = true;
                ("disable", format!("listenState {state}"))
            }
            (Ok(state), Action::Enable) => {
                self.held = false;
                evidence.disabled_by_luma = false;
                ("enable", format!("listenState {state}"))
            }
            (Err(error), Action::Disable) => ("disable", format!("failed: {error}")),
            (Err(error), Action::Enable) => ("enable", format!("failed: {error}")),
        };
        match &result {
            Ok(state) => evidence.listen_state = Some(state.clone()),
            Err(error) => evidence.error = Some(error.clone()),
        }
        eprintln!("Luma Wi-Fi Direct: {at} listening was {listen}; {name}: {outcome}");
        evidence.last_change = Some(Change {
            at,
            action: name,
            result: outcome,
        });
    }
}

enum Command {
    Setting(bool),
    State {
        listen: Option<String>,
        subscribed: Option<bool>,
    },
}

pub struct Service {
    sender: mpsc::Sender<Command>,
    pub receiver: mpsc::Receiver<Evidence>,
}

impl Service {
    /// `subscribe` starts a subscription to `p2p/getstate` that hands each reply's
    /// `listenState` and `subscribed` to the given function.
    pub fn spawn<S>(
        on: bool,
        disabled_by_luma: bool,
        p2p: impl P2p + Send + 'static,
        subscribe: S,
    ) -> Self
    where
        S: FnOnce(Box<dyn FnMut(Option<String>, Option<bool>) + Send>) -> Result<(), String>
            + Send
            + 'static,
    {
        let (sender, commands) = mpsc::channel();
        let (replies, receiver) = mpsc::channel();
        let subscription = sender.clone();
        let _ = thread::Builder::new()
            .name("luma-wifi-direct".into())
            .spawn(move || {
                let mut keeper = Keeper::new(on, disabled_by_luma);
                // Read once at start (cold boot, enable, update), then on every change.
                keeper.check(&p2p, None, now);
                let subscribed = subscribe(Box::new(move |listen, subscribed| {
                    let _ = subscription.send(Command::State { listen, subscribed });
                }));
                if let Err(error) = subscribed {
                    eprintln!("Luma Wi-Fi Direct: no subscription ({error}); polling every minute");
                    keeper.evidence.subscribed = Some(false);
                }
                let mut sent = None;
                let mut polled = Instant::now();
                loop {
                    if sent.as_ref() != Some(&keeper.evidence) {
                        sent = Some(keeper.evidence.clone());
                        if replies.send(keeper.evidence.clone()).is_err() {
                            return;
                        }
                    }
                    let wait = POLL.saturating_sub(polled.elapsed());
                    match commands.recv_timeout(wait) {
                        Ok(Command::Setting(on)) => {
                            if keeper.on != on {
                                keeper.on = on;
                                keeper.check(&p2p, None, now);
                            }
                        }
                        Ok(Command::State { listen, subscribed }) => {
                            if subscribed.is_some() {
                                keeper.evidence.subscribed = subscribed;
                            }
                            keeper.check(&p2p, listen, now);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            polled = Instant::now();
                            keeper.check(&p2p, None, now);
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
        Self { sender, receiver }
    }

    pub fn set_enabled(&self, on: bool) {
        let _ = self.sender.send(Command::Setting(on));
    }
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
