//! Control Center (tvOS style): Sleep, volume and mute, sound output and network status.
//! Every Luna call runs on this module's own worker, never on the SDL thread, and failures
//! come back as a quiet notice. Desktop builds simulate the TV so the panel can be tested.
use serde::Serialize;
use serde_json::{Value, json};
use std::{sync::mpsc, thread};

/// Sound outputs offered, in order. LG names them in `com.webos.service.apiadapter`.
pub const OUTPUTS: [&str; 5] = [
    "tv_speaker",
    "external_arc",
    "external_optical",
    "bt_soundbar",
    "headphone",
];

/// Turns the TV off (the panel's Sleep); `reason` as LG's own remote sends it.
pub const POWER_OFF: &str = "luna://com.webos.service.tvpower/power/turnOff";
pub const GET_VOLUME: &str = "luna://com.webos.audio/getVolume";
pub const SET_VOLUME: &str = "luna://com.webos.audio/setVolume";
pub const SET_MUTED: &str = "luna://com.webos.audio/setMuted";
pub const GET_OUTPUT: &str = "luna://com.webos.service.apiadapter/audio/getSoundOutput";
pub const SET_OUTPUT: &str = "luna://com.webos.service.apiadapter/audio/changeSoundOutput";
pub const NETWORK: &str = "luna://com.webos.service.connectionmanager/getStatus";

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Read volume, mute, sound output and network.
    Status,
    SetVolume(u8),
    SetMuted(bool),
    SetOutput(String),
    Sleep,
}

impl Command {
    /// The Luna request that performs a change (none for `Status`).
    pub fn request(&self) -> Option<(&'static str, Value)> {
        match self {
            Self::Status => None,
            Self::SetVolume(volume) => Some((SET_VOLUME, json!({"volume": (*volume).min(100)}))),
            Self::SetMuted(muted) => Some((SET_MUTED, json!({"muted": muted}))),
            Self::SetOutput(output) => OUTPUTS
                .contains(&output.as_str())
                .then(|| (SET_OUTPUT, json!({"output": output}))),
            Self::Sleep => Some((POWER_OFF, json!({"reason": "remoteKey"}))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Network {
    Wired,
    Wifi(String),
    Offline,
}

/// What the panel shows; fields the TV did not answer stay `None` and are not drawn.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub volume: Option<u8>,
    pub muted: Option<bool>,
    pub output: Option<String>,
    pub network: Option<Network>,
}

pub struct Reply {
    pub command: Command,
    /// The status read after the command, or why the command failed.
    pub result: Result<Status, String>,
}

/// `getVolume`: webOS 5 and later nest it in `volumeStatus`; older firmware answers flat.
pub fn parse_volume(value: &Value) -> (Option<u8>, Option<bool>, Option<String>) {
    let status = value.get("volumeStatus").unwrap_or(value);
    let volume = status
        .get("volume")
        .and_then(Value::as_u64)
        .filter(|volume| *volume <= 100)
        .map(|volume| volume as u8);
    let muted = status
        .get("muteStatus")
        .or_else(|| status.get("muted"))
        .and_then(Value::as_bool);
    let output = status
        .get("soundOutput")
        .and_then(Value::as_str)
        .map(str::to_owned);
    (volume, muted, output)
}

pub fn parse_network(value: &Value) -> Option<Network> {
    let connected = |key: &str| value[key]["state"].as_str() == Some("connected");
    if connected("wired") {
        Some(Network::Wired)
    } else if connected("wifi") {
        Some(Network::Wifi(
            value["wifi"]["ssid"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect(),
        ))
    } else if value.get("wired").is_some() || value.get("wifi").is_some() {
        Some(Network::Offline)
    } else {
        None
    }
}

fn tv_status() -> Status {
    let (volume, muted, mut output) = crate::platform::system_call(GET_VOLUME, json!({}))
        .map(|value| parse_volume(&value))
        .unwrap_or_default();
    if let Ok(value) = crate::platform::system_call(GET_OUTPUT, json!({}))
        && let Some(current) = value.get("soundOutput").and_then(Value::as_str)
    {
        output = Some(current.to_owned());
    }
    let network = crate::platform::system_call(NETWORK, json!({}))
        .ok()
        .and_then(|value| parse_network(&value));
    Status {
        volume,
        muted,
        output,
        network,
    }
}

/// A simulated TV for desktop previews: changes apply to memory only.
fn desktop(status: &mut Status, command: &Command) -> Result<Status, String> {
    match command {
        Command::Status => {}
        Command::SetVolume(volume) => status.volume = Some((*volume).min(100)),
        Command::SetMuted(muted) => status.muted = Some(*muted),
        Command::SetOutput(output) => status.output = Some(output.clone()),
        Command::Sleep => return Err("Desktop preview · the TV stays on".into()),
    }
    Ok(status.clone())
}

pub struct Service {
    sender: mpsc::Sender<Command>,
    pub receiver: mpsc::Receiver<Reply>,
}

impl Service {
    pub fn new(webos: bool) -> Self {
        let (sender, commands) = mpsc::channel::<Command>();
        let (replies, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut simulated = Status {
                volume: Some(12),
                muted: Some(false),
                output: Some("tv_speaker".into()),
                network: Some(Network::Wifi("Luma".into())),
            };
            while let Ok(command) = commands.recv() {
                let result = if webos {
                    match command.request() {
                        Some((uri, payload)) => {
                            crate::platform::system_call(uri, payload).map(|_| tv_status())
                        }
                        None if command == Command::Status => Ok(tv_status()),
                        None => Err("Unsupported sound output".into()),
                    }
                } else {
                    desktop(&mut simulated, &command)
                };
                if let Err(error) = &result {
                    eprintln!("Luma Control Center {command:?}: {error}");
                }
                if replies.send(Reply { command, result }).is_err() {
                    break;
                }
            }
        });
        Self { sender, receiver }
    }

    pub fn send(&self, command: Command) {
        let _ = self.sender.send(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_replies_parse_on_old_and_new_firmware_and_reject_nonsense() {
        let new = json!({"volumeStatus": {"volume": 17, "muteStatus": true, "soundOutput": "external_arc"}});
        assert_eq!(
            parse_volume(&new),
            (Some(17), Some(true), Some("external_arc".into()))
        );
        assert_eq!(
            parse_volume(&json!({"volume": 5, "muted": false})),
            (Some(5), Some(false), None)
        );
        assert_eq!(parse_volume(&json!({"volume": 900})).0, None);
    }

    #[test]
    fn network_replies_parse_wired_wifi_and_offline() {
        assert_eq!(
            parse_network(&json!({"wired": {"state": "connected"}})),
            Some(Network::Wired)
        );
        assert_eq!(
            parse_network(
                &json!({"wired": {"state": "disconnected"}, "wifi": {"state": "connected", "ssid": "Home"}})
            ),
            Some(Network::Wifi("Home".into()))
        );
        assert_eq!(
            parse_network(&json!({"wired": {"state": "disconnected"}})),
            Some(Network::Offline)
        );
        assert_eq!(parse_network(&json!({})), None);
    }

    #[test]
    fn only_allowlisted_outputs_and_bounded_volumes_reach_the_tv() {
        assert_eq!(Command::SetOutput("rm -rf".into()).request(), None);
        assert_eq!(
            Command::SetVolume(250).request(),
            Some((SET_VOLUME, json!({"volume": 100})))
        );
        assert_eq!(
            Command::Sleep.request(),
            Some((POWER_OFF, json!({"reason": "remoteKey"})))
        );
        for output in OUTPUTS {
            assert!(Command::SetOutput(output.into()).request().is_some());
        }
    }

    #[test]
    fn desktop_simulates_changes_and_never_sleeps() {
        let service = Service::new(false);
        service.send(Command::SetVolume(30));
        let reply = service
            .receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(reply.result.unwrap().volume, Some(30));
        service.send(Command::Sleep);
        let reply = service
            .receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(reply.result.is_err());
    }
}
