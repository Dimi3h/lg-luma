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
//! switched off, through the controller's reconnects. The pad offers force feedback (rumble,
//! gain) like xpad; effects an app plays on it go to the controller's hidraw node as its rumble
//! report.
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

pub const EV_FF: u16 = 0x15;
/// uinput's own events on the uinput descriptor (force-feedback requests).
pub const EV_UINPUT: u16 = 0x0101;
pub const UI_FF_UPLOAD: u16 = 1;
pub const UI_FF_ERASE: u16 = 2;
pub const FF_RUMBLE: u16 = 0x50;
pub const FF_GAIN: u16 = 0x60;
/// Effects the virtual pad accepts at once, as the kernel's xpad driver allows.
pub const FF_EFFECTS_MAX: u32 = 16;

/// The Stadia controller's rumble output report: id 5, then the strong and the weak magnitude
/// as little-endian 16-bit values (as the kernel's hid-google-stadiaff sends it; the browser's
/// writes to handle 0x0044 on the TV, such as `006e0000`, fit that layout).
pub fn rumble_report(strong: u16, weak: u16) -> [u8; 5] {
    let [strong_low, strong_high] = strong.to_le_bytes();
    let [weak_low, weak_high] = weak.to_le_bytes();
    [0x05, strong_low, strong_high, weak_low, weak_high]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Effect {
    strong: u16,
    weak: u16,
    /// Replay length; 0 plays until stopped.
    length: std::time::Duration,
}

/// The virtual pad's force-feedback effects and what the controller currently plays: uploads,
/// erases, play and stop requests, the gain, and the replay lengths. Every change of what
/// should play yields the report to write; several effects at once play their strongest
/// magnitudes.
#[derive(Debug)]
pub struct Rumble {
    effects: std::collections::BTreeMap<i16, Effect>,
    /// Playing effects and when each ends (`None`: until stopped).
    playing: std::collections::BTreeMap<i16, Option<std::time::Instant>>,
    gain: u16,
    /// The magnitudes last sent.
    sent: (u16, u16),
    pub uploaded: u64,
    pub played: u64,
}

impl Default for Rumble {
    fn default() -> Self {
        Self {
            effects: Default::default(),
            playing: Default::default(),
            gain: u16::MAX,
            sent: (0, 0),
            uploaded: 0,
            played: 0,
        }
    }
}

impl Rumble {
    /// A rumble effect uploaded (new, or changed in place). Other effect types are refused.
    pub fn upload(&mut self, id: i16, kind: u16, strong: u16, weak: u16, length_ms: u16) -> bool {
        if kind != FF_RUMBLE || id < 0 {
            return false;
        }
        self.effects.insert(
            id,
            Effect {
                strong,
                weak,
                length: std::time::Duration::from_millis(length_ms.into()),
            },
        );
        self.uploaded += 1;
        true
    }

    /// An effect erased; stopped first if it plays.
    pub fn erase(&mut self, id: i16) -> Option<[u8; 5]> {
        self.effects.remove(&id);
        self.playing.remove(&id);
        self.output()
    }

    /// An EV_FF event: `value` > 0 starts effect `code`, 0 stops it; FF_GAIN sets the gain.
    pub fn event(&mut self, code: u16, value: i32, now: std::time::Instant) -> Option<[u8; 5]> {
        if code == FF_GAIN {
            self.gain = value.clamp(0, i32::from(u16::MAX)) as u16;
            return self.output();
        }
        let id = code as i16;
        if value > 0 {
            if let Some(effect) = self.effects.get(&id) {
                let end = (!effect.length.is_zero()).then(|| now + effect.length);
                self.playing.insert(id, end);
                self.played += 1;
            }
        } else {
            self.playing.remove(&id);
        }
        self.output()
    }

    /// Ends effects whose replay length passed.
    pub fn tick(&mut self, now: std::time::Instant) -> Option<[u8; 5]> {
        self.playing
            .retain(|_, end| end.is_none_or(|end| end > now));
        self.output()
    }

    /// When the next playing effect ends.
    pub fn next_end(&self) -> Option<std::time::Instant> {
        self.playing.values().flatten().min().copied()
    }

    /// Everything stops (the controller went away, the bridge ends): zeros, if anything played.
    pub fn stop_all(&mut self) -> Option<[u8; 5]> {
        self.playing.clear();
        self.output()
    }

    /// The controller came back: whatever plays is sent again to the new node.
    pub fn resend(&self) -> [u8; 5] {
        rumble_report(self.sent.0, self.sent.1)
    }

    fn output(&mut self) -> Option<[u8; 5]> {
        let scale = |value: u16| (u32::from(value) * u32::from(self.gain) / 0xffff) as u16;
        let (strong, weak) = self
            .playing
            .keys()
            .filter_map(|id| self.effects.get(id))
            .fold((0, 0), |(strong, weak), effect| {
                (strong.max(effect.strong), weak.max(effect.weak))
            });
        let next = (scale(strong), scale(weak));
        (next != self.sent).then(|| {
            self.sent = next;
            rumble_report(next.0, next.1)
        })
    }
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
    //! The bridge itself: the prototype's behaviour, including its TV workarounds, plus
    //! force feedback from the virtual pad to the controller's hidraw node.
    use super::*;
    use std::{
        ffi::CString,
        fs,
        io::Write,
        mem,
        sync::atomic::{AtomicBool, Ordering},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const fn ioc(dir: u32, kind: u32, number: u32, size: u32) -> u32 {
        (dir << 30) | (size << 16) | (kind << 8) | number
    }
    const UI_DEV_CREATE: u32 = ioc(0, b'U' as u32, 1, 0);
    const UI_SET_EVBIT: u32 = ioc(1, b'U' as u32, 100, 4);
    const UI_SET_KEYBIT: u32 = ioc(1, b'U' as u32, 101, 4);
    const UI_SET_ABSBIT: u32 = ioc(1, b'U' as u32, 103, 4);
    const UI_SET_FFBIT: u32 = ioc(1, b'U' as u32, 107, 4);
    const UPLOAD_SIZE: u32 = mem::size_of::<libc::uinput_ff_upload>() as u32;
    const ERASE_SIZE: u32 = mem::size_of::<libc::uinput_ff_erase>() as u32;
    const UI_BEGIN_FF_UPLOAD: u32 = ioc(3, b'U' as u32, 200, UPLOAD_SIZE);
    const UI_END_FF_UPLOAD: u32 = ioc(1, b'U' as u32, 201, UPLOAD_SIZE);
    const UI_BEGIN_FF_ERASE: u32 = ioc(3, b'U' as u32, 202, ERASE_SIZE);
    const UI_END_FF_ERASE: u32 = ioc(1, b'U' as u32, 203, ERASE_SIZE);
    /// Kernel 3.15 and later; the SDK's headers are older than the TV's kernel.
    const fn ui_get_sysname(length: u32) -> u32 {
        ioc(2, b'U' as u32, 44, length)
    }
    const EVIOCGRAB: u32 = ioc(1, b'E' as u32, 0x90, 4);
    /// Our own nodes for the controller (see `open_node`).
    const NODE: &str = "/tmp/.luma-pad-source";
    const HIDRAW_NODE: &str = "/tmp/.luma-pad-hidraw";
    /// Set by SIGTERM (the unit's stop): the loop sends zeros and ends. The handler only sets
    /// this; the loop waits in poll(), which a signal interrupts (the prototype's handler left
    /// it blocked in read()).
    static STOP: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_term(_: libc::c_int) {
        STOP.store(true, Ordering::SeqCst);
    }

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
        // Read and write: force-feedback requests arrive on the same descriptor.
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(format!("/dev/uinput: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: plain ioctls on the uinput descriptor with integer arguments.
        unsafe {
            libc::ioctl(fd, UI_SET_EVBIT as _, EV_KEY as libc::c_int);
            libc::ioctl(fd, UI_SET_EVBIT as _, EV_ABS as libc::c_int);
            libc::ioctl(fd, UI_SET_EVBIT as _, EV_FF as libc::c_int);
            libc::ioctl(fd, UI_SET_FFBIT as _, FF_RUMBLE as libc::c_int);
            libc::ioctl(fd, UI_SET_FFBIT as _, FF_GAIN as libc::c_int);
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
        device.ff_effects_max = FF_EFFECTS_MAX;
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

    /// A device number (major, minor).
    type Number = (u32, u32);
    /// The controller's `eventN` and number, and its hidraw node's name and number.
    type Source = (String, Number, Option<(String, Number)>);

    /// A device number from a sysfs `dev` file ("major:minor").
    fn number(path: &str) -> Option<Number> {
        let text = fs::read_to_string(path).ok()?;
        let (major, minor) = text.trim().split_once(':')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    }

    /// The controller's `eventN`, its device number, and its HID device's hidraw node (the
    /// same uhid device; every repair recreates both).
    fn find_source() -> Option<Source> {
        let read = |path: String| fs::read_to_string(path).unwrap_or_default();
        for index in 0..64 {
            let base = format!("/sys/class/input/event{index}");
            let id = |field: &str| read(format!("{base}/device/id/{field}"));
            if !is_stadia(&id("bustype"), &id("vendor"), &id("product")) {
                continue;
            }
            let event = number(&format!("{base}/dev"))?;
            let hidraw = fs::read_dir(format!("{base}/device/device/hidraw"))
                .ok()
                .and_then(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .find(|name| name.starts_with("hidraw"))
                })
                .and_then(|name| {
                    let dev = number(&format!("{base}/device/device/hidraw/{name}/dev"))?;
                    Some((name, dev))
                });
            return Some((format!("event{index}"), event, hidraw));
        }
        None
    }

    /// Opens a device through a node of its own: on the TV the static /dev/input/eventN node
    /// could keep pointing at a previous, dead instance after a reconnect (ENODEV).
    fn open_node(path: &str, (major, minor): Number, flags: libc::c_int) -> Option<i32> {
        let node = CString::new(path).unwrap();
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
            let fd = libc::open(node.as_ptr(), flags);
            libc::unlink(node.as_ptr());
            (fd >= 0).then_some(fd)
        }
    }

    fn close(fd: i32) {
        // SAFETY: closing a descriptor this process opened.
        unsafe { libc::close(fd) };
    }

    /// Writes a rumble report to the controller's hidraw node, if open.
    fn send(hidraw: Option<i32>, report: Option<[u8; 5]>) {
        if let (Some(fd), Some(report)) = (hidraw, report) {
            // SAFETY: writes the live 5-byte report.
            unsafe { libc::write(fd, report.as_ptr().cast(), report.len()) };
        }
    }

    /// Serves the virtual pad's force-feedback requests: uploads and erases (answered at once,
    /// so the app's ioctl never waits) and play/stop/gain events.
    fn serve_pad(ui: i32, rumble: &mut Rumble, hidraw: Option<i32>) {
        let size = mem::size_of::<libc::input_event>();
        loop {
            // SAFETY: input_event is plain data; zeroed is a valid value.
            let mut event: libc::input_event = unsafe { mem::zeroed() };
            // SAFETY: reads at most one input_event into the live value (non-blocking).
            let read =
                unsafe { libc::read(ui, (&mut event as *mut libc::input_event).cast(), size) };
            if read != size as isize {
                return;
            }
            match (event.type_, event.code) {
                (EV_UINPUT, UI_FF_UPLOAD) => {
                    // SAFETY: plain data; the kernel fills it for this request id.
                    let mut upload: libc::uinput_ff_upload = unsafe { mem::zeroed() };
                    upload.request_id = event.value as u32;
                    // SAFETY: ioctls with a live, correctly sized struct.
                    unsafe {
                        if libc::ioctl(ui, UI_BEGIN_FF_UPLOAD as _, &mut upload) < 0 {
                            continue;
                        }
                        let effect = &upload.effect;
                        // A rumble effect's union starts with strong, then weak magnitude.
                        let accepted = rumble.upload(
                            effect.id,
                            effect.type_,
                            (effect.u[0] & 0xffff) as u16,
                            ((effect.u[0] >> 16) & 0xffff) as u16,
                            effect.replay.length,
                        );
                        upload.retval = if accepted { 0 } else { -libc::EINVAL };
                        libc::ioctl(ui, UI_END_FF_UPLOAD as _, &mut upload);
                    }
                }
                (EV_UINPUT, UI_FF_ERASE) => {
                    // SAFETY: plain data; the kernel fills it for this request id.
                    let mut erase: libc::uinput_ff_erase = unsafe { mem::zeroed() };
                    erase.request_id = event.value as u32;
                    // SAFETY: ioctls with a live, correctly sized struct.
                    unsafe {
                        if libc::ioctl(ui, UI_BEGIN_FF_ERASE as _, &mut erase) < 0 {
                            continue;
                        }
                        send(hidraw, rumble.erase(erase.effect_id as i16));
                        erase.retval = 0;
                        libc::ioctl(ui, UI_END_FF_ERASE as _, &mut erase);
                    }
                }
                (EV_FF, code) => send(hidraw, rumble.event(code, event.value, Instant::now())),
                _ => {}
            }
        }
    }

    /// The bridge, single-threaded: one poll over the virtual pad (force-feedback requests,
    /// served even while the controller is away) and the controller. Ends on the unit's stop.
    pub fn run() -> Result<Value> {
        // SAFETY: installs a handler that only stores to an atomic.
        unsafe {
            libc::signal(
                libc::SIGTERM,
                on_term as *const () as usize as libc::sighandler_t,
            )
        };
        let mut status = json!({"running": true, "pid": std::process::id(), "since": now(),
            "source": null, "pad": null, "pad_nodes": [], "connections": 0,
            "rumble": false, "hidraw": null, "effects_uploaded": 0, "effects_played": 0});
        write_status(&status);
        log("waiting for a Stadia controller");
        let mut pad: Option<i32> = None;
        let mut source: Option<i32> = None;
        let mut hidraw: Option<i32> = None;
        let mut rumble = Rumble::default();
        let mut search_at = Instant::now();
        let mut counted = (0, 0);
        let mut written = Instant::now();
        let input_size = mem::size_of::<libc::input_event>();
        loop {
            if STOP.load(Ordering::SeqCst) {
                send(hidraw, rumble.stop_all());
                status["running"] = json!(false);
                write_status(&status);
                log("stopped");
                return Ok(json!({"returnValue": true}));
            }
            if source.is_none() && Instant::now() >= search_at {
                search_at = Instant::now() + Duration::from_secs(1);
                if let Some((event, number, hid)) = find_source() {
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
                                pad = Some(fd);
                            }
                            Err(error) => {
                                log(&format!("cannot create the virtual pad: {error}"));
                                status["error"] = json!(error);
                                write_status(&status);
                                search_at = Instant::now() + Duration::from_secs(10);
                            }
                        }
                    }
                    if pad.is_some()
                        && let Some(fd) = open_node(NODE, number, libc::O_RDONLY)
                    {
                        // SAFETY: an ioctl with an integer argument on the controller's node.
                        if unsafe { libc::ioctl(fd, EVIOCGRAB as _, 1 as libc::c_int) } < 0 {
                            log(&format!(
                                "could not take the controller exclusively: {}",
                                std::io::Error::last_os_error()
                            ));
                        }
                        source = Some(fd);
                        hidraw = hid.as_ref().and_then(|(_, number)| {
                            open_node(HIDRAW_NODE, *number, libc::O_WRONLY | libc::O_NONBLOCK)
                        });
                        // Whatever plays continues on the new instance.
                        send(hidraw, Some(rumble.resend()));
                        log(&format!(
                            "controller found at {event}; rumble through {}",
                            hid.as_ref()
                                .filter(|_| hidraw.is_some())
                                .map_or("nothing (no hidraw node)", |(name, _)| name.as_str())
                        ));
                        status["source"] = json!(event);
                        status["source_since"] = json!(now());
                        status["connections"] =
                            json!(status["connections"].as_u64().unwrap_or(0) + 1);
                        status["rumble"] = json!(hidraw.is_some());
                        status["hidraw"] =
                            json!(hid.filter(|_| hidraw.is_some()).map(|(name, _)| name));
                        if let Some(fields) = status.as_object_mut() {
                            fields.remove("error");
                        }
                        write_status(&status);
                    }
                }
            }
            let mut fds: Vec<libc::pollfd> = [pad, source]
                .into_iter()
                .flatten()
                .map(|fd| libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            let wait = rumble.next_end().map_or(1000, |end| {
                end.saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(1000) as i32
            });
            // SAFETY: live pollfds; a signal interrupts the wait.
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, wait) };
            send(hidraw, rumble.tick(Instant::now()));
            if ready <= 0 {
                continue;
            }
            for poll in &fds {
                if poll.revents == 0 {
                    continue;
                }
                if Some(poll.fd) == pad {
                    serve_pad(poll.fd, &mut rumble, hidraw);
                    continue;
                }
                // The controller: one event per readiness, or it went away.
                // SAFETY: input_event is plain data; zeroed is a valid value.
                let mut event: libc::input_event = unsafe { mem::zeroed() };
                // SAFETY: reads at most one input_event into the live value.
                let read = unsafe {
                    libc::read(
                        poll.fd,
                        (&mut event as *mut libc::input_event).cast(),
                        input_size,
                    )
                };
                if read == input_size as isize {
                    if let (Some(ui), Some((kind, code, value))) =
                        (pad, translate(event.type_, event.code, event.value))
                    {
                        emit(ui, kind, code, value);
                    }
                    continue;
                }
                if read < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                send(hidraw, rumble.stop_all());
                close(poll.fd);
                source = None;
                if let Some(fd) = hidraw.take() {
                    close(fd);
                }
                if let Some(ui) = pad {
                    for (kind, code, value) in rest() {
                        emit(ui, kind, code, value);
                    }
                }
                log("controller gone, virtual pad set to rest");
                status["source"] = Value::Null;
                status["rumble"] = json!(false);
                status["hidraw"] = Value::Null;
                write_status(&status);
                search_at = Instant::now() + Duration::from_secs(1);
            }
            // The effect counters, at most every two seconds.
            if (rumble.uploaded, rumble.played) != counted
                && written.elapsed() >= Duration::from_secs(2)
            {
                counted = (rumble.uploaded, rumble.played);
                written = Instant::now();
                status["effects_uploaded"] = json!(rumble.uploaded);
                status["effects_played"] = json!(rumble.played);
                write_status(&status);
            }
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
