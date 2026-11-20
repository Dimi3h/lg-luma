//! The Stadia controller as a virtual Xbox 360 pad (Settings › System › Stadia controller as
//! Xbox controller).
//!
//! In LG's browser (Chromium 120, where GeForce NOW runs) the Stadia controller's buttons,
//! sticks and D-pad worked but its analog triggers did not. The kernel reports them as ABS_GAS
//! (right) and ABS_BRAKE (left), with the right stick on ABS_Z/ABS_RZ. This bridge reads the
//! controller exclusively and replays it on a uinput device laid out like the kernel's xpad
//! driver: triggers on ABS_Z/ABS_RZ, right stick on ABS_RX/ABS_RY. With it the user could
//! accelerate in a game on the measured TV (LG OLED55B46LA, webOS 25). Only the Stadia
//! controller (vendor 0x18d1, product 0x9400) is bridged; other controllers are untested.
//!
//! It runs as its own systemd unit, started with the Home takeover's apply (cold boot, enable)
//! so the virtual pad exists before a game starts: when the browser already knew the real
//! controller, the virtual pad became a second player and the game ignored it. The pad is
//! created when the controller is first seen and kept until a reboot or until the setting is
//! switched off, through the controller's reconnects. Rumble is not forwarded.
use crate::{
    Result,
    system::{self, System},
};
use serde_json::{Value, json};
use std::path::Path;

/// Present when the user switched the bridge off (Settings › System).
pub const OFF: &str = "/var/lib/luma-home/pad-bridge.off";
/// Written by the running bridge for Luma's diagnostics; /var/run starts empty at each boot.
pub const STATUS: &str = "/var/run/luma-pad-bridge.json";
pub const UNIT: &str = "luma-pad-bridge";

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;
pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_Z: u16 = 0x02;
pub const ABS_RX: u16 = 0x03;
pub const ABS_RY: u16 = 0x04;
pub const ABS_RZ: u16 = 0x05;
pub const ABS_GAS: u16 = 0x09;
pub const ABS_BRAKE: u16 = 0x0a;
pub const ABS_HAT0X: u16 = 0x10;
pub const ABS_HAT0Y: u16 = 0x11;
/// BTN_A, B, X, Y, TL, TR, SELECT, START, MODE, THUMBL, THUMBR: passed on unchanged. The
/// Stadia controller's four BTN_TRIGGER_HAPPY keys are not.
pub const KEYS: [u16; 11] = [
    0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e,
];
/// The Stadia controller over Bluetooth.
pub const STADIA: (u16, u16) = (0x18d1, 0x9400);
/// The virtual pad: what the kernel's xpad driver reports for a wired Xbox 360 pad.
pub const PAD_NAME: &str = "Microsoft X-Box 360 pad";
pub const PAD_ID: (u16, u16, u16, u16) = (0x03, 0x045e, 0x028e, 0x0114);

/// A Stadia stick axis (0..255, rest 128) on the Xbox range (-32768..32767, rest 0).
pub fn stick(value: i32) -> i32 {
    if value <= 0 {
        return -32768;
    }
    if value >= 255 {
        return 32767;
    }
    ((value - 128) * 258).clamp(-32768, 32767)
}

/// One controller event as the virtual pad's event (type, code, value), or `None` when it is
/// not passed on.
pub fn translate(kind: u16, code: u16, value: i32) -> Option<(u16, u16, i32)> {
    match kind {
        EV_SYN => Some((EV_SYN, 0, 0)),
        EV_KEY => KEYS.contains(&code).then_some((EV_KEY, code, value)),
        EV_ABS => match code {
            ABS_X => Some((EV_ABS, ABS_X, stick(value))),
            ABS_Y => Some((EV_ABS, ABS_Y, stick(value))),
            ABS_Z => Some((EV_ABS, ABS_RX, stick(value))),
            ABS_RZ => Some((EV_ABS, ABS_RY, stick(value))),
            ABS_BRAKE => Some((EV_ABS, ABS_Z, value)),
            ABS_GAS => Some((EV_ABS, ABS_RZ, value)),
            ABS_HAT0X | ABS_HAT0Y => Some((EV_ABS, code, value)),
            _ => None,
        },
        _ => None,
    }
}

/// The events that set the virtual pad to rest (when the controller goes away).
pub fn rest() -> Vec<(u16, u16, i32)> {
    KEYS.iter()
        .map(|key| (EV_KEY, *key, 0))
        .chain(
            [
                ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ, ABS_HAT0X, ABS_HAT0Y,
            ]
            .into_iter()
            .map(|axis| (EV_ABS, axis, 0)),
        )
        .chain([(EV_SYN, 0, 0)])
        .collect()
}

/// Whether sysfs ids (`id/bustype`, `id/vendor`, `id/product`, hex text) are the Stadia
/// controller over Bluetooth (bus 0x05).
pub fn is_stadia(bustype: &str, vendor: &str, product: &str) -> bool {
    let hex = |text: &str| u16::from_str_radix(text.trim(), 16).ok();
    hex(bustype) == Some(0x05) && (hex(vendor), hex(product)) == (Some(STADIA.0), Some(STADIA.1))
}

/// A runtime udev rule that hides the real Stadia controller from apps while the bridge runs.
/// The bridge grabs the controller, so its joystick node only ever shows its untouched start
/// state (on the TV both sticks read -32767, pinned in a corner): the browser treated that as a
/// second, live pad. The bridge opens the controller through its own node as root, so it is
/// unaffected. /run is not persistent, so the rule is written at every start.
pub const HIDE_RULE: &str = "/run/udev/rules.d/99-luma-stadia-hide.rules";
pub const HIDE_RULE_TEXT: &str = "SUBSYSTEM==\"input\", ATTRS{id/vendor}==\"18d1\", ATTRS{id/product}==\"9400\", ENV{ID_INPUT_JOYSTICK}=\"\", ENV{ID_INPUT_KEY}=\"\", ENV{ID_INPUT}=\"\", MODE=\"0600\"\n";

/// The controller's present `eventN` and `jsN` under /sys/class/input.
fn controller_nodes(system: &impl System) -> Vec<String> {
    // sysfs attributes report a size of 4096 bytes whatever they hold.
    let id = |name: &str, field: &str| {
        system::read(
            system,
            format!("/sys/class/input/{name}/device/id/{field}"),
            4096,
        )
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
    };
    let mut nodes: Vec<String> = std::fs::read_dir(system.path(Path::new("/sys/class/input")))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("event") || name.starts_with("js"))
                .filter(|name| {
                    is_stadia(
                        &id(name, "bustype"),
                        &id(name, "vendor"),
                        &id(name, "product"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
}

/// Reloads udev's rules and re-evaluates an already present controller, so the change applies
/// at once. An app that already opened the controller keeps its old view until it restarts.
fn reapply(system: &impl System) -> Vec<String> {
    let _ = system::command(system, "udevadm", &["control", "--reload"]);
    let nodes = controller_nodes(system);
    for node in &nodes {
        let path = format!("/sys/class/input/{node}");
        let _ = system::command(system, "udevadm", &["trigger", "--action=change", &path]);
    }
    nodes
}

/// Hides the real controller from apps (the rule above).
pub fn hide(system: &impl System) -> Result<Vec<String>> {
    system::mkdir(system, "/run/udev")?;
    system::mkdir(system, "/run/udev/rules.d")?;
    system::write(system, HIDE_RULE, HIDE_RULE_TEXT.as_bytes(), 0o644)?;
    Ok(reapply(system))
}

/// Gives the real controller back to apps.
pub fn show(system: &impl System) -> Vec<String> {
    let _ = system::remove(system, HIDE_RULE);
    reapply(system)
}

/// Starts the bridge as its own systemd unit (replacing a running one), unless switched off,
/// and hides the real controller from apps. `helper` is the `luma-helper` the unit runs; when
/// the unit ends for any reason it gives the controller back (`ExecStopPost`), where the TV's
/// systemd accepts that property.
pub fn start(system: &impl System, helper: &Path) -> Result<String> {
    if system::exists(system, OFF) {
        return Ok("off in Settings; not started".into());
    }
    let _ = system::command(system, "systemctl", &["stop", UNIT]);
    let unit = format!("--unit={UNIT}");
    let helper = helper.to_string_lossy();
    let stop_post = format!("--property=ExecStopPost={helper} pad-bridge show");
    let started = system::command(
        system,
        "systemd-run",
        &[&unit, "--collect", &stop_post, &helper, "pad-bridge", "run"],
    );
    let note = if started.is_err() {
        system::command(
            system,
            "systemd-run",
            &[&unit, "--collect", &helper, "pad-bridge", "run"],
        )?;
        "; ExecStopPost refused, controller shown again only on stop"
    } else {
        ""
    };
    let hidden = match hide(system) {
        Ok(nodes) => format!("controller hidden from apps ({})", nodes.join(", ")),
        Err(error) => format!("controller not hidden: {error}"),
    };
    Ok(format!("started ({helper}); {hidden}{note}"))
}

/// Stops the bridge (the kernel removes the virtual pad with the process) and gives the real
/// controller back to apps.
pub fn stop(system: &impl System) {
    let _ = system::command(system, "systemctl", &["stop", UNIT]);
    show(system);
}

/// `on` (start now and at every cold boot), `off` (stop now and at boot), `status`, `hide` or
/// `show` (the real controller for apps), or `run` (the bridge itself, as the unit runs it).
pub fn dispatch(system: &impl System, command: &str, helper: &Path) -> Result<Value> {
    system::root(system)?;
    match command {
        "on" => {
            system::remove(system, OFF)?;
            let result = start(system, helper)?;
            Ok(json!({"returnValue": true, "result": result}))
        }
        "off" => {
            system::write(system, OFF, b"", 0o644)?;
            stop(system);
            Ok(json!({"returnValue": true, "result": "stopped"}))
        }
        "show" => Ok(json!({"returnValue": true, "shown": show(system)})),
        "hide" => Ok(json!({"returnValue": true, "hidden": hide(system)?})),
        "status" => Ok(system::json(system, STATUS)
            .unwrap_or_else(|_| json!({"running": false}))
            .as_object()
            .map(|status| {
                let mut status = Value::Object(status.clone());
                status["returnValue"] = json!(true);
                status
            })
            .unwrap_or_else(|| json!({"returnValue": true, "running": false}))),
        "run" => device::run(),
        _ => Err("Unknown pad bridge command".into()),
    }
}

#[cfg(all(feature = "webos", target_os = "linux"))]
mod device {
    //! The bridge itself: the prototype's behaviour, including its four TV workarounds.
    use super::*;
    use std::{
        ffi::CString,
        fs,
        io::Write,
        mem, thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    const fn ioc(dir: u32, kind: u32, number: u32, size: u32) -> u32 {
        (dir << 30) | (size << 16) | (kind << 8) | number
    }
    const UI_DEV_CREATE: u32 = ioc(0, b'U' as u32, 1, 0);
    const UI_SET_EVBIT: u32 = ioc(1, b'U' as u32, 100, 4);
    const UI_SET_KEYBIT: u32 = ioc(1, b'U' as u32, 101, 4);
    const UI_SET_ABSBIT: u32 = ioc(1, b'U' as u32, 103, 4);
    /// Kernel 3.15 and later; the SDK's headers are older than the TV's kernel.
    const fn ui_get_sysname(length: u32) -> u32 {
        ioc(2, b'U' as u32, 44, length)
    }
    const EVIOCGRAB: u32 = ioc(1, b'E' as u32, 0x90, 4);
    /// Our own node for the controller (see `open_source`).
    const NODE: &str = "/tmp/.luma-pad-source";

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_secs())
            .unwrap_or(0)
    }

    fn log(message: &str) {
        eprintln!("Luma pad bridge: {message}");
    }

    fn write_status(status: &Value) {
        let text = status.to_string();
        let temporary = format!("{STATUS}.tmp");
        if fs::write(&temporary, text).is_ok() {
            let _ = fs::rename(&temporary, STATUS);
        }
    }

    fn emit(fd: i32, kind: u16, code: u16, value: i32) {
        // SAFETY: input_event is plain data; zeroed is a valid value.
        let mut event: libc::input_event = unsafe { mem::zeroed() };
        event.type_ = kind;
        event.code = code;
        event.value = value;
        // SAFETY: writes exactly one input_event from a live value.
        unsafe {
            libc::write(
                fd,
                (&event as *const libc::input_event).cast(),
                mem::size_of::<libc::input_event>(),
            )
        };
    }

    fn create_pad() -> std::result::Result<i32, String> {
        let path = CString::new("/dev/uinput").unwrap();
        // SAFETY: a valid C string; the descriptor is owned by this process until it exits.
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(format!("/dev/uinput: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: plain ioctls on the uinput descriptor with integer arguments.
        unsafe {
            libc::ioctl(fd, UI_SET_EVBIT as _, EV_KEY as libc::c_int);
            libc::ioctl(fd, UI_SET_EVBIT as _, EV_ABS as libc::c_int);
            for key in KEYS {
                libc::ioctl(fd, UI_SET_KEYBIT as _, key as libc::c_int);
            }
            for axis in [
                ABS_X, ABS_Y, ABS_Z, ABS_RX, ABS_RY, ABS_RZ, ABS_HAT0X, ABS_HAT0Y,
            ] {
                libc::ioctl(fd, UI_SET_ABSBIT as _, axis as libc::c_int);
            }
        }
        // SAFETY: uinput_user_dev is plain data; zeroed is a valid value.
        let mut device: libc::uinput_user_dev = unsafe { mem::zeroed() };
        for (slot, byte) in device.name.iter_mut().zip(PAD_NAME.bytes()) {
            *slot = byte as libc::c_char;
        }
        device.id.bustype = PAD_ID.0;
        device.id.vendor = PAD_ID.1;
        device.id.product = PAD_ID.2;
        device.id.version = PAD_ID.3;
        for axis in [ABS_X, ABS_Y, ABS_RX, ABS_RY] {
            let axis = axis as usize;
            device.absmin[axis] = -32768;
            device.absmax[axis] = 32767;
            device.absfuzz[axis] = 16;
            device.absflat[axis] = 128;
        }
        device.absmax[ABS_Z as usize] = 255;
        device.absmax[ABS_RZ as usize] = 255;
        for hat in [ABS_HAT0X, ABS_HAT0Y] {
            device.absmin[hat as usize] = -1;
            device.absmax[hat as usize] = 1;
        }
        let size = mem::size_of::<libc::uinput_user_dev>();
        // SAFETY: writes the whole live struct, then creates the device.
        let created = unsafe {
            libc::write(fd, (&device as *const libc::uinput_user_dev).cast(), size) == size as isize
                && libc::ioctl(fd, UI_DEV_CREATE as _) >= 0
        };
        if !created {
            let error = std::io::Error::last_os_error();
            // SAFETY: closing the descriptor opened above.
            unsafe { libc::close(fd) };
            return Err(format!("uinput: {error}"));
        }
        Ok(fd)
    }

    /// The pad's `inputNN` under /sys/devices/virtual/input.
    fn sysname(fd: i32) -> Option<String> {
        let mut name = [0u8; 64];
        // SAFETY: the kernel writes at most the given length into the buffer.
        let result = unsafe {
            libc::ioctl(
                fd,
                ui_get_sysname(name.len() as u32 - 1) as _,
                name.as_mut_ptr(),
            )
        };
        (result >= 0).then(|| {
            String::from_utf8_lossy(&name)
                .trim_end_matches('\0')
                .to_owned()
        })
    }

    /// The pad's event and joystick nodes.
    fn nodes(sysname: &str) -> Vec<String> {
        let mut nodes: Vec<String> = fs::read_dir(format!("/sys/devices/virtual/input/{sysname}"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with("event") || name.starts_with("js"))
                    .collect()
            })
            .unwrap_or_default();
        nodes.sort();
        nodes
    }

    /// Announces the pad's joystick node a second time: the browser opens it on the first
    /// "add" event, sometimes before the node is readable for it, and then never tries again.
    fn announce(sysname: &str) {
        thread::sleep(Duration::from_secs(1));
        for node in nodes(sysname).iter().filter(|node| node.starts_with("js")) {
            let path = format!("/sys/devices/virtual/input/{sysname}/{node}/uevent");
            if fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|mut file| file.write_all(b"add"))
                .is_ok()
            {
                log(&format!("announced again: {path}"));
            }
        }
    }

    /// The controller's `eventN` and device number, from sysfs.
    fn find_source() -> Option<(String, u32, u32)> {
        let read = |path: String| fs::read_to_string(path).unwrap_or_default();
        for index in 0..64 {
            let base = format!("/sys/class/input/event{index}");
            let id = |field: &str| read(format!("{base}/device/id/{field}"));
            if !is_stadia(&id("bustype"), &id("vendor"), &id("product")) {
                continue;
            }
            let number = read(format!("{base}/dev"));
            let (major, minor) = number.trim().split_once(':')?;
            return Some((
                format!("event{index}"),
                major.parse().ok()?,
                minor.parse().ok()?,
            ));
        }
        None
    }

    /// Opens the controller through a node of its own: on the TV the static /dev/input/eventN
    /// node could keep pointing at a previous, dead instance after a reconnect (ENODEV).
    fn open_source(major: u32, minor: u32) -> Option<i32> {
        let node = CString::new(NODE).unwrap();
        // SAFETY: valid C string; the node is removed again right after opening.
        unsafe {
            libc::unlink(node.as_ptr());
            if libc::mknod(
                node.as_ptr(),
                libc::S_IFCHR | 0o600,
                libc::makedev(major, minor),
            ) < 0
            {
                return None;
            }
            let fd = libc::open(node.as_ptr(), libc::O_RDONLY);
            libc::unlink(node.as_ptr());
            (fd >= 0).then_some(fd)
        }
    }

    /// Never returns while it works; the unit's stop ends it (no signal handler: a handler
    /// left the prototype blocked in read()), and the kernel removes the pad with it.
    pub fn run() -> Result<Value> {
        let started = now();
        let mut pad: Option<(i32, Option<String>)> = None;
        let mut status = json!({"running": true, "pid": std::process::id(), "since": started,
            "source": null, "pad": null, "pad_nodes": [], "connections": 0});
        write_status(&status);
        log("waiting for a Stadia controller");
        loop {
            let Some((event, major, minor)) = find_source() else {
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            if pad.is_none() {
                match create_pad() {
                    Ok(fd) => {
                        let name = sysname(fd);
                        log(&format!("virtual Xbox 360 pad created ({name:?})"));
                        if let Some(name) = &name {
                            announce(name);
                            status["pad"] = json!(name);
                            status["pad_nodes"] = json!(nodes(name));
                        }
                        pad = Some((fd, name));
                    }
                    Err(error) => {
                        log(&format!("cannot create the virtual pad: {error}"));
                        status["error"] = json!(error);
                        write_status(&status);
                        thread::sleep(Duration::from_secs(10));
                        continue;
                    }
                }
            }
            let Some((ui, _)) = &pad else { continue };
            let ui = *ui;
            let Some(source) = open_source(major, minor) else {
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            // SAFETY: an ioctl with an integer argument on the controller's descriptor.
            if unsafe { libc::ioctl(source, EVIOCGRAB as _, 1 as libc::c_int) } < 0 {
                log(&format!(
                    "could not take the controller exclusively: {}",
                    std::io::Error::last_os_error()
                ));
            }
            log(&format!("controller found at {event}"));
            status["source"] = json!(event);
            status["source_since"] = json!(now());
            status["connections"] = json!(status["connections"].as_u64().unwrap_or(0) + 1);
            if let Some(fields) = status.as_object_mut() {
                fields.remove("error");
            }
            write_status(&status);
            let size = mem::size_of::<libc::input_event>();
            loop {
                let mut poll = libc::pollfd {
                    fd: source,
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: one live pollfd.
                let ready = unsafe { libc::poll(&mut poll, 1, 1000) };
                if ready == 0 {
                    continue;
                }
                if ready < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                // SAFETY: input_event is plain data; zeroed is a valid value.
                let mut event: libc::input_event = unsafe { mem::zeroed() };
                // SAFETY: reads at most one input_event into the live value.
                let read = unsafe {
                    libc::read(source, (&mut event as *mut libc::input_event).cast(), size)
                };
                if read != size as isize {
                    if read < 0
                        && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    break;
                }
                if let Some((kind, code, value)) = translate(event.type_, event.code, event.value) {
                    emit(ui, kind, code, value);
                }
            }
            // SAFETY: closing the controller's descriptor opened above.
            unsafe { libc::close(source) };
            for (kind, code, value) in rest() {
                emit(ui, kind, code, value);
            }
            log("controller gone, virtual pad set to rest");
            status["source"] = Value::Null;
            write_status(&status);
            thread::sleep(Duration::from_secs(1));
        }
    }
}

#[cfg(not(all(feature = "webos", target_os = "linux")))]
mod device {
    use super::*;
    pub fn run() -> Result<Value> {
        Err("The pad bridge needs a webOS build".into())
    }
}
