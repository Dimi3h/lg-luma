//! Repairing a Bluetooth gamepad whose input LG's Bluetooth stack stopped delivering
//! (Settings › System › Repair Bluetooth controller).
//!
//! Measured on an LG OLED55B46LA (webOS 25, MediaTek Fluoride) with a Stadia controller (BLE
//! HID): at its first button press after every new LE connection the controller sends a GATT
//! Service Changed indication. LG's stack rediscovers the services, drops the HID notification
//! registration and never restores it, so input reaches the radio but never `/dev/uhid`. The
//! stack rewrites the device's GATT cache file right after the indication, which is the trigger.
//! A plain HID reconnect does not help (a new LE connection, a new indication at the next press).
//! What worked: keep the LE link up through a second GATT client, then restart only the HID
//! layer, so the HID host registers for notifications again without a new LE connection.
use crate::{
    Result, accepted,
    system::{self, System},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const SERVICE: &str = "com.webos.service.bluetooth2";
/// The Bluetooth class of device of a gamepad (the Stadia controller reports 1288).
pub const GAMEPAD_CLASS: u64 = 1288;
const BTSVC: &str = "/mnt/lg/cmn_data/btsvc";
/// How long the HID profile may take to drop after `hid/disconnect`, in seconds.
pub const DROP_SECONDS: u32 = 6;
/// The measured repair waited about this long between the disconnect and the connect.
const GAP_SECONDS: u32 = 3;
/// How long the HID profile may take to come back after `hid/connect`, in seconds.
pub const RECONNECT_SECONDS: u32 = 10;

/// A connected gamepad worth watching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gamepad {
    pub address: String,
    pub name: String,
}

/// A Bluetooth address as LG's service writes it (`fb:ff:bc:f7:9c:36`).
pub fn valid_address(address: &str) -> bool {
    let parts: Vec<&str> = address.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
}

fn profiles(device: &Value) -> Vec<&str> {
    device["connectedProfiles"]
        .as_array()
        .map(|profiles| profiles.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The BLE gamepads with HID connected in a `device/getStatus` reply, wherever LG nests the
/// device entries. LG's own remote ("LGE MR…", on the same stack) is never one of them.
pub fn gamepads(status: &Value) -> Vec<Gamepad> {
    fn walk(value: &Value, found: &mut Vec<Gamepad>) {
        match value {
            Value::Object(fields) => {
                if let (Some(address), Some(kind)) = (
                    fields.get("address").and_then(Value::as_str),
                    fields.get("typeOfDevice").and_then(Value::as_str),
                ) {
                    let name = fields.get("name").and_then(Value::as_str).unwrap_or("");
                    if kind == "ble"
                        && valid_address(address)
                        && fields.get("classOfDevice").and_then(Value::as_u64)
                            == Some(GAMEPAD_CLASS)
                        && profiles(value).contains(&"hid")
                        && !name.starts_with("LGE")
                        && !found.iter().any(|pad| pad.address == address)
                    {
                        found.push(Gamepad {
                            address: address.to_ascii_lowercase(),
                            name: name.to_owned(),
                        });
                    }
                }
                fields.values().for_each(|child| walk(child, found));
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, found)),
            _ => {}
        }
    }
    let mut found = vec![];
    walk(status, &mut found);
    found
}

/// Whether `address` has the profile connected in a `device/getStatus` reply.
pub fn has_profile(status: &Value, address: &str, profile: &str) -> bool {
    fn walk(value: &Value, address: &str, profile: &str) -> bool {
        match value {
            Value::Object(fields) => {
                fields
                    .get("address")
                    .and_then(Value::as_str)
                    .is_some_and(|found| found.eq_ignore_ascii_case(address))
                    && profiles(value).contains(&profile)
                    || fields.values().any(|child| walk(child, address, profile))
            }
            Value::Array(items) => items.iter().any(|item| walk(item, address, profile)),
            _ => false,
        }
    }
    walk(status, address, profile)
}

/// The device's GATT cache file: `fluoride-<vendor>` per `module.conf` (`vendor=mtk` on the
/// measured TV; a `fluoride-rtk` tree exists for the other chip), else whichever tree has it.
pub fn cache_path(system: &impl System, address: &str) -> Option<PathBuf> {
    if !valid_address(address) {
        return None;
    }
    let file = format!(
        "gatt_cache_{}",
        address.replace(':', "").to_ascii_lowercase()
    );
    let tree = |vendor: &str| {
        Path::new(BTSVC)
            .join(format!("fluoride-{vendor}"))
            .join("rec/bluetooth")
            .join(&file)
    };
    let configured = system::read(system, format!("{BTSVC}/module.conf"), 4096)
        .ok()
        .and_then(|conf| {
            String::from_utf8_lossy(&conf).lines().find_map(|line| {
                line.trim()
                    .strip_prefix("vendor=")
                    .map(|vendor| vendor.trim().to_owned())
            })
        })
        .filter(|vendor| !vendor.is_empty() && vendor.chars().all(|c| c.is_ascii_alphanumeric()));
    configured
        .iter()
        .map(String::as_str)
        .chain(["mtk", "rtk"])
        .map(tree)
        .find(|path| system::exists(system, path))
}

/// The cache file's modification time in milliseconds since the epoch; `None` when absent.
pub fn cache_mtime(system: &impl System, address: &str) -> Option<u128> {
    let path = cache_path(system, address)?;
    fs::metadata(system.path(&path))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_millis())
}

/// Decides when to repair: one baseline per HID connection, a repair when the cache file changes
/// while HID stays connected. A new connection (the controller switched on) only takes a new
/// baseline: a plain connect was not seen to rewrite the cache.
#[derive(Debug, Default)]
pub struct Watch {
    baselines: BTreeMap<String, Option<u128>>,
}

impl Watch {
    /// `connected`: the gamepads with HID connected now, with their cache file's time. Returns
    /// the addresses to repair.
    pub fn observe(&mut self, connected: &[(String, Option<u128>)]) -> Vec<String> {
        self.baselines
            .retain(|address, _| connected.iter().any(|(other, _)| other == address));
        let mut repair = vec![];
        for (address, mtime) in connected {
            match self.baselines.get_mut(address) {
                None => {
                    self.baselines.insert(address.clone(), *mtime);
                }
                // A cache file that only appears now (none before, or unreadable) is a
                // baseline too: only a change of a file seen before triggers.
                Some(baseline @ None) => *baseline = *mtime,
                Some(baseline) => {
                    if mtime.is_some() && *mtime != *baseline {
                        // Once per change, whatever the repair's outcome: a failed repair
                        // never turns into a loop.
                        *baseline = *mtime;
                        repair.push(address.clone());
                    }
                }
            }
        }
        repair
    }

    /// After a repair: its own cache time is the new baseline, so the repair never triggers
    /// itself (it was not seen to rewrite the cache, but a stack that does would loop).
    pub fn repaired(&mut self, address: &str, mtime: Option<u128>) {
        if let Some(baseline) = self.baselines.get_mut(address) {
            *baseline = mtime.or(*baseline);
        }
    }

    pub fn watching(&self) -> Vec<String> {
        self.baselines.keys().cloned().collect()
    }
}

fn status(system: &impl System) -> Result<Value> {
    accepted(system::luna(
        system,
        SERVICE,
        "device/getStatus",
        &json!({}),
    )?)
}

/// Waits up to `seconds` for `address`'s HID profile to be `connected`; the seconds it took.
fn wait_hid(system: &impl System, address: &str, connected: bool, seconds: u32) -> Option<u32> {
    for waited in 0..=seconds {
        if status(system).is_ok_and(|reply| has_profile(&reply, address, "hid") == connected) {
            return Some(waited);
        }
        if waited < seconds {
            system.pause();
        }
    }
    None
}

/// The repair as measured on the TV: a second GATT client keeps the LE link up
/// (`gatt/connect`, idempotent), then only the HID layer restarts (`hid/disconnect`, about 3 s,
/// `hid/connect`). `gatt/disconnect` is never called: on the TV it did not answer.
pub fn repair(system: &impl System, address: &str) -> Result<Value> {
    if !valid_address(address) {
        return Err("Invalid Bluetooth address".into());
    }
    let started = SystemTime::now();
    let payload = json!({"address": address});
    accepted(system::luna(system, SERVICE, "gatt/connect", &payload)?)?;
    accepted(system::luna(system, SERVICE, "hid/disconnect", &payload)?)?;
    let dropped = wait_hid(system, address, false, DROP_SECONDS);
    for _ in dropped.unwrap_or(0)..GAP_SECONDS {
        system.pause();
    }
    accepted(system::luna(system, SERVICE, "hid/connect", &payload)?)?;
    let back = wait_hid(system, address, true, RECONNECT_SECONDS);
    let outcome = match (dropped, back) {
        (_, Some(_)) => "repaired: HID connected again",
        (Some(_), None) => "failed: HID did not come back",
        (None, None) => "failed: HID neither dropped nor came back",
    };
    Ok(json!({
        "returnValue": true,
        "address": address,
        "outcome": outcome,
        "repaired": back.is_some(),
        "dropped": dropped.is_some(),
        "seconds": started.elapsed().map(|time| time.as_secs()).unwrap_or(0),
    }))
}

/// `status` (the connected gamepads with their cache file's time) or `repair ADDRESS`.
pub fn dispatch(system: &impl System, command: &str, address: Option<&str>) -> Result<Value> {
    system::root(system)?;
    match command {
        "status" => {
            let pads: Vec<Value> = gamepads(&status(system)?)
                .into_iter()
                .map(|pad| {
                    json!({
                        "address": pad.address,
                        "name": pad.name,
                        "cache_mtime": cache_mtime(system, &pad.address).map(|ms| ms as u64),
                    })
                })
                .collect();
            Ok(json!({"returnValue": true, "gamepads": pads}))
        }
        "repair" => repair(system, address.ok_or("Missing Bluetooth address")?),
        _ => Err("Unknown Bluetooth command".into()),
    }
}
