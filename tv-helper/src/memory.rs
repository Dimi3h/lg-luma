//! Memory compression (Settings › System): a compressed swap in RAM on the kernel's spare zram
//! device, applied by the Home takeover's cold-boot apply and when the setting is switched on.
//! LG's TVs boot with `zram0` present but unused (disksize 0). It is only ever set up when
//! unused: a zram device already in use is left alone, LG's own swap is never touched, and
//! switching the setting off only stops applying it at the next cold boot (a full zram is never
//! swapped off live).
use crate::{
    Result,
    system::{self, System},
};
use serde_json::{Value, json};
use std::path::Path;

/// Present when the user switched memory compression off (Settings › System).
pub const OFF: &str = "/var/lib/luma-home/memory-compression.off";
const DEVICE: &str = "/dev/zram0";
const BLOCK: &str = "/sys/block/zram0";
/// The compressed swap's size: 45% of the RAM (768 MB on a 1.7 GB TV).
const SHARE_PERCENT: u64 = 45;
/// Above LG's own swap, so compressed pages in RAM are used first.
const PRIORITY: &str = "100";
/// Reclaim tuning for a swap in RAM: no read-ahead of swapped pages, and earlier background
/// reclaim instead of stalls in the allocating app.
const VM: [(&str, &str); 2] = [
    ("/proc/sys/vm/page-cluster", "0"),
    ("/proc/sys/vm/watermark_scale_factor", "150"),
];

fn text(system: &impl System, path: impl AsRef<Path>) -> Option<String> {
    system::read(system, path, 64 * 1024)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn mem_total_kb(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kb| kb.parse().ok())
}

/// The swap size for `mem_total_kb` of RAM, in bytes, whole MiB.
pub fn size_bytes(mem_total_kb: u64) -> u64 {
    mem_total_kb * SHARE_PERCENT / 100 / 1024 * 1024 * 1024
}

/// The best algorithm the kernel offers (`comp_algorithm` lists them, the current in brackets).
pub fn algorithm(available: &str) -> Option<&'static str> {
    let names: Vec<&str> = available
        .split_whitespace()
        .map(|name| name.trim_matches(|c| c == '[' || c == ']'))
        .collect();
    ["zstd", "lz4", "lzo-rle", "lzo"]
        .into_iter()
        .find(|name| names.contains(name))
}

/// Swap devices from `/proc/swaps`: name, size and used (KiB), priority.
fn swaps(system: &impl System) -> Vec<Value> {
    text(system, "/proc/swaps")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            (fields.len() >= 5).then(|| {
                json!({
                    "device": fields[0],
                    "size_kb": fields[2].parse::<u64>().unwrap_or(0),
                    "used_kb": fields[3].parse::<u64>().unwrap_or(0),
                    "priority": fields[4].parse::<i64>().unwrap_or(0),
                })
            })
        })
        .collect()
}

/// The current state: every swap, every zram device with its size and compression, the VM
/// settings and whether the setting is on.
pub fn status(system: &impl System) -> Value {
    let mut zram = vec![];
    for index in 0..4 {
        let block = format!("/sys/block/zram{index}");
        let Some(disksize) = text(system, format!("{block}/disksize")) else {
            continue;
        };
        // mm_stat: original data, compressed data, memory used (bytes), ...
        let stat: Vec<u64> = text(system, format!("{block}/mm_stat"))
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|value| value.parse().ok())
            .collect();
        let (data, used) = (
            stat.first().copied().unwrap_or(0),
            stat.get(2).copied().unwrap_or(0),
        );
        zram.push(json!({
            "device": format!("/dev/zram{index}"),
            "disksize_mb": disksize.parse::<u64>().unwrap_or(0) / 1024 / 1024,
            "algorithm": text(system, format!("{block}/comp_algorithm"))
                .and_then(|value| {
                    value
                        .split_whitespace()
                        .find(|name| name.starts_with('['))
                        .map(|name| name.trim_matches(|c| c == '[' || c == ']').to_owned())
                }),
            "data_mb": data / 1024 / 1024,
            "ram_mb": used / 1024 / 1024,
            "ratio": if used > 0 { ((data as f64 / used as f64) * 10.).round() / 10. } else { 0. },
        }));
    }
    json!({
        "returnValue": true,
        "enabled": !system::exists(system, OFF),
        "swaps": swaps(system),
        "zram": zram,
        "page_cluster": text(system, VM[0].0),
        "watermark_scale_factor": text(system, VM[1].0),
    })
}

/// Sets up the compressed swap if the setting is on and `zram0` is unused. Returns what it did
/// (for boot.log), never failing the caller for a device it leaves alone.
pub fn apply(system: &impl System) -> Result<String> {
    if system::exists(system, OFF) {
        return Ok("off in Settings; nothing applied".into());
    }
    let Some(disksize) = text(system, format!("{BLOCK}/disksize")) else {
        return Ok("no zram0 on this TV; nothing applied".into());
    };
    let in_swaps = swaps(system).iter().any(|swap| swap["device"] == DEVICE);
    let mut done = vec![];
    if disksize != "0" || in_swaps {
        done.push("zram0 already in use; left alone".to_owned());
    } else {
        let available = text(system, format!("{BLOCK}/comp_algorithm")).unwrap_or_default();
        let algorithm = algorithm(&available).ok_or("zram0 offers no known algorithm")?;
        let total = text(system, "/proc/meminfo")
            .as_deref()
            .and_then(mem_total_kb)
            .ok_or("MemTotal unknown")?;
        let size = size_bytes(total);
        system::set_attribute(system, format!("{BLOCK}/comp_algorithm"), algorithm)?;
        system::set_attribute(system, format!("{BLOCK}/disksize"), &size.to_string())?;
        system::command(system, "/sbin/mkswap", &[DEVICE])?;
        system::command(system, "/sbin/swapon", &["-p", PRIORITY, DEVICE])?;
        done.push(format!(
            "zram0: {} MB {algorithm} swap at priority {PRIORITY}",
            size / 1024 / 1024
        ));
    }
    for (path, value) in VM {
        match system::set_attribute(system, path, value) {
            Ok(()) => done.push(format!("{path} = {value}")),
            Err(error) => done.push(format!("{path} not set: {error}")),
        }
    }
    Ok(done.join("; "))
}

/// `on` (apply now and at every cold boot), `off` (stop applying at the next cold boot; nothing
/// is swapped off live), `apply` (the cold-boot step) or `status`.
pub fn dispatch(system: &impl System, command: &str) -> Result<Value> {
    system::root(system)?;
    match command {
        "status" => Ok(status(system)),
        "on" | "apply" => {
            if command == "on" {
                system::remove(system, OFF)?;
            }
            let result = apply(system)?;
            let mut reply = status(system);
            reply["result"] = json!(result);
            Ok(reply)
        }
        "off" => {
            system::write(system, OFF, b"", 0o644)?;
            let mut reply = status(system);
            reply["result"] =
                json!("off from the next cold boot; the current swap stays until then");
            Ok(reply)
        }
        _ => Err("Unknown memory compression command".into()),
    }
}
