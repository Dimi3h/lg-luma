//! LG's preload policy for apps the user hid in Luma. LG's preload manager starts the apps in its
//! policy hidden; its `setPreloadPolicy` (API group "system") can switch an entry off or on, but
//! cannot add other apps. Luma only switches off entries of apps the user hid, and only switches
//! on again the entries it switched off.
use crate::{
    Result, accepted,
    system::{self, System},
    valid_app_id,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const SERVICE: &str = "com.webos.service.preloadmanager";
const APP_MANAGER: &str = "com.webos.applicationManager";

/// Every `{"id": …, "isEnabled": …}` entry in a `getPreloadPolicy` reply, wherever it is nested.
pub fn policy_entries(reply: &Value) -> BTreeMap<String, bool> {
    fn walk(value: &Value, entries: &mut BTreeMap<String, bool>) {
        match value {
            Value::Object(fields) => {
                if let (Some(id), Some(enabled)) = (
                    fields.get("id").and_then(Value::as_str),
                    fields.get("isEnabled").and_then(Value::as_bool),
                ) {
                    entries.insert(id.to_owned(), enabled);
                }
                for child in fields.values() {
                    walk(child, entries);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, entries);
                }
            }
            _ => {}
        }
    }
    let mut entries = BTreeMap::new();
    walk(reply, &mut entries);
    entries
}

fn ids(request: &Value, key: &str) -> Result<Vec<String>> {
    let list = request[key]
        .as_array()
        .ok_or("Invalid preload policy request")?;
    if list.len() > 64 {
        return Err("Too many apps in a preload policy request".into());
    }
    list.iter()
        .map(|id| {
            id.as_str()
                .filter(|id| valid_app_id(id))
                .map(str::to_owned)
                .ok_or_else(|| "Invalid app id in a preload policy request".to_string())
        })
        .collect()
}

fn set(system: &impl System, id: &str, enabled: bool) -> Result<()> {
    accepted(system::luna(
        system,
        SERVICE,
        "setPreloadPolicy",
        &json!({"application": {"id": id, "isEnabled": enabled}}),
    )?)
    .map(|_| ())
}

/// `{"disable": [ids], "enable": [ids]}`: switches LG's entries for those apps off or on (ids
/// LG's policy does not list are skipped) and closes an instance LG already preloaded hidden of
/// an app it switched off. Replies with the ids it switched off (`disabled`) and on (`enabled`),
/// so Luma only ever switches on again what it switched off itself.
pub fn dispatch(system: &impl System, request: &str) -> Result<Value> {
    system::root(system)?;
    let request: Value = serde_json::from_str(request).map_err(|e| e.to_string())?;
    let (disable, enable) = (ids(&request, "disable")?, ids(&request, "enable")?);
    let entries = policy_entries(&accepted(system::luna(
        system,
        SERVICE,
        "getPreloadPolicy",
        &json!({}),
    )?)?);
    let running = |id: &str| {
        system::luna(system, APP_MANAGER, "running", &json!({}))
            .ok()
            .is_some_and(|reply| {
                reply["running"]
                    .as_array()
                    .is_some_and(|apps| apps.iter().any(|app| app["id"] == id))
            })
    };
    let front = system::luna(system, APP_MANAGER, "getForegroundAppInfo", &json!({}))
        .ok()
        .and_then(|reply| reply["appId"].as_str().map(str::to_owned));
    let mut disabled = vec![];
    for id in &disable {
        // Only entries that are on: one LG already had off is not Luma's to switch on later.
        if entries.get(id) == Some(&true) {
            set(system, id, false)?;
            // LG may already have preloaded it hidden: close it unless it is on screen.
            if front.as_deref() != Some(id.as_str()) && running(id) {
                let _ = system::luna(system, APP_MANAGER, "closeByAppId", &json!({"id": id}));
            }
            disabled.push(id.clone());
        }
    }
    let mut enabled = vec![];
    for id in &enable {
        if entries.get(id) == Some(&false) {
            set(system, id, true)?;
            enabled.push(id.clone());
        }
    }
    Ok(json!({"returnValue": true, "disabled": disabled, "enabled": enabled}))
}
