//! Wi-Fi Direct (Screen Share) listening (Settings › System › Screen Share listening off).
//!
//! Measured on an LG OLED55B46LA (webOS 25) only: with `listenState` "enabled", `iw event` showed
//! `p2p0` remain on 2437 MHz for 100 ms every 500 ms while the TV was connected at 5300 MHz, so
//! the radio left the access point's channel twice a second; router pings had a p90 of 63 ms
//! (max 76–132 ms). With it "disabled": no such events, p90 7.1 ms, max 8.7 ms.
use crate::{
    Result, accepted,
    system::{self, System},
};
use serde_json::{Value, json};

const SERVICE: &str = "com.webos.service.wifi";

/// `listenState` from a `p2p/getstate` reply ("enabled" or "disabled").
pub fn listen_state(reply: &Value) -> Option<&str> {
    reply["listenState"].as_str()
}

/// `state` (the current `p2p/getstate` reply), `enable` or `disable` (`listenState`).
pub fn dispatch(system: &impl System, command: &str) -> Result<Value> {
    system::root(system)?;
    let state = match command {
        "state" => None,
        "enable" => Some("enabled"),
        "disable" => Some("disabled"),
        _ => return Err("Unknown Wi-Fi Direct command".into()),
    };
    if let Some(state) = state {
        accepted(system::luna(
            system,
            SERVICE,
            "p2p/setstate",
            &json!({"listenState": state}),
        )?)?;
    }
    accepted(system::luna(system, SERVICE, "p2p/getstate", &json!({}))?)
}
