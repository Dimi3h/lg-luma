//! Optional takeover of LG's system screensaver (`com.webos.app.screensaver`) by Luma's clock
//! page. Like the Home manifest method: a staged web app is bind-mounted over the system app's
//! directory and SAM is made to read its manifest again. The stage holds copies of the page,
//! never links into the installed package, so an app update cannot leave a stale mount; each
//! apply (enable, cold boot, update) copies the installed page again when it changed.
use crate::{
    APP, Result, home,
    system::{self, System},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const STATE: &str = "/var/lib/luma-screensaver";
const CONFIG: &str = "/var/lib/luma-screensaver/state.json";
pub const STAGE: &str = "/var/lib/luma-screensaver/app";
/// The cold-boot hook, right after `10-luma-home` in Homebrew Channel's name order. It is its
/// own hook so the screensaver survives with or without the Home takeover.
pub const HOOK: &str = "/var/lib/webosbrew/init.d/10-luma-screensaver";
/// Present while the takeover is applied this boot: Luma then leaves its own timer off.
pub const MARKER: &str = "/var/run/luma-screensaver";
pub const ID: &str = "com.webos.app.screensaver";
pub const TARGET: &str = "/usr/palm/applications/com.webos.app.screensaver";
/// The page's files inside the installed package's `screensaver/` directory.
pub const PAGE: [&str; 2] = ["index.html", "inter-clock.ttf"];
const SCRIPTS: [&str; 2] = ["screensaver.sh", "luma-helper"];
const SAM_WAIT_POLLS: usize = 200;
const LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub enabled: bool,
    pub app_dir: String,
    /// LG's own manifest, for status and to recognise a manifest that is already the stage's.
    pub stock: Value,
}

/// The firmware SAM can reread a system app's manifest on (blockSystemApp and
/// releaseBlockSystemApp): webOS 25/26, as for the Home manifest method.
pub fn supported(release: &str) -> Result<()> {
    let version = release
        .split("release")
        .nth(1)
        .and_then(|s| s.trim_start().split('.').next())
        .and_then(|s| s.parse::<u32>().ok());
    if matches!(version, Some(10 | 11)) {
        Ok(())
    } else {
        Err("The screensaver takeover needs webOS 25/26 (10.x/11.x), where SAM can reread a system app's manifest".into())
    }
}

/// The staged manifest: LG's identity and title, as a hidden web app in a screen-saver window.
pub fn manifest(stock: &Value) -> Value {
    let mut manifest = json!({
        "id": ID,
        "version": stock["version"].as_str().unwrap_or("1.0.0"),
        "vendor": "Luma",
        "title": stock["title"].as_str().unwrap_or("Screensaver"),
        "type": "web",
        "main": "index.html",
        "defaultWindowType": "screenSaver",
        "visible": false,
        "noSplashOnLaunch": true,
        "handlesRelaunch": true,
        "disableBackHistoryAPI": true,
    });
    if let Some(icon) = stock_icon(stock) {
        manifest["icon"] = json!(icon);
    }
    manifest
}

/// LG's icon when it is a plain relative file name, which the stage carries a copy of.
fn stock_icon(stock: &Value) -> Option<&str> {
    stock["icon"].as_str().filter(|icon| {
        let path = Path::new(icon);
        !icon.is_empty()
            && path.is_relative()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && path.components().count() == 1
    })
}

fn is_stage(manifest: &Value) -> bool {
    manifest["type"] == "web" && manifest["main"] == "index.html" && manifest["vendor"] == "Luma"
}

pub fn mounted(system: &impl System) -> bool {
    system::same_file(system, STAGE, TARGET)
}

fn foreign_mount(system: &impl System) -> Result<bool> {
    if mounted(system) {
        return Ok(false);
    }
    let info = String::from_utf8(system::read(system, "/proc/self/mountinfo", LIMIT)?)
        .map_err(|e| e.to_string())?;
    Ok(info.lines().any(|line| {
        line.split_whitespace().nth(4).is_some_and(|point| {
            let point = home::unescape(point);
            point == TARGET || point.starts_with(&format!("{TARGET}/"))
        })
    }))
}

pub fn hook_owned(system: &impl System) -> bool {
    std::fs::read_link(system.path(Path::new(HOOK)))
        .is_ok_and(|p| p == system.path(Path::new(&format!("{STATE}/screensaver.sh"))))
}

pub fn load(system: &impl System) -> Result<Option<Config>> {
    if !system::exists(system, CONFIG) {
        return Ok(None);
    }
    let config: Config =
        serde_json::from_value(system::json(system, CONFIG)?).map_err(|e| e.to_string())?;
    let app = Path::new(&config.app_dir);
    if !app.is_absolute()
        || app
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Invalid installed directory".into());
    }
    Ok(Some(config))
}

fn wait_for_sam(system: &impl System) -> Result<()> {
    let mut last = String::new();
    for _ in 0..SAM_WAIT_POLLS {
        match home::app_manager(system, "getForegroundAppInfo", json!({})) {
            Ok(reply) if reply["returnValue"] == true => return Ok(()),
            Ok(reply) => last = reply.to_string(),
            Err(error) => last = error,
        }
        system.poll();
    }
    Err(format!("SAM did not answer: {last}"))
}

/// Writes `bytes` unless the file already holds them (the stage is rewritten at every boot).
fn update(system: &impl System, path: &str, bytes: &[u8], mode: u32) -> Result<bool> {
    if system::read(system, path, LIMIT).ok().as_deref() == Some(bytes) {
        return Ok(false);
    }
    system::write(system, path, bytes, mode)?;
    Ok(true)
}

/// Copies the installed page into the stage and writes the staged manifest. Without an
/// installed page (Luma removed) the previous copy stays in use.
fn stage(system: &impl System, config: &Config) -> Result<()> {
    system::mkdir(system, STATE)?;
    system::mkdir(system, STAGE)?;
    let page = Path::new(&config.app_dir).join("screensaver");
    if system::exists(system, page.join(PAGE[0])) {
        for name in PAGE {
            let bytes = system::read(system, page.join(name), LIMIT)?;
            update(system, &format!("{STAGE}/{name}"), &bytes, 0o644)?;
        }
    } else if !system::exists(system, format!("{STAGE}/{}", PAGE[0])) {
        return Err("Luma's screensaver page is not installed".into());
    }
    let manifest = manifest(&config.stock);
    update(
        system,
        &format!("{STAGE}/appinfo.json"),
        serde_json::to_string_pretty(&manifest)
            .map_err(|e| e.to_string())?
            .as_bytes(),
        0o644,
    )?;
    update(
        system,
        &format!("{STAGE}/.luma-screensaver"),
        format!("{APP}\n").as_bytes(),
        0o644,
    )?;
    Ok(())
}

/// Copies the installed app's newer `screensaver.sh` and `luma-helper` for the boot hook, only
/// when root owns them and nobody else can write them (root runs them at boot).
fn refresh_scripts(system: &impl System, app: &Path) {
    let differs = SCRIPTS.iter().any(|name| {
        system::read(system, app.join(name), 16 * 1024 * 1024).ok()
            != system::read(system, format!("{STATE}/{name}"), 16 * 1024 * 1024).ok()
    });
    if !differs
        || !SCRIPTS
            .iter()
            .all(|name| system::exists(system, app.join(name)))
    {
        return;
    }
    if !SCRIPTS.iter().all(|name| system.trusted(&app.join(name))) {
        eprintln!(
            "Luma screensaver: the installed scripts are newer but not root-owned; run screensaver.sh enable to use them"
        );
        return;
    }
    if let Err(error) = install_scripts(system, app) {
        eprintln!("Luma screensaver: boot scripts not refreshed: {error}");
    }
}

fn install_scripts(system: &impl System, app: &Path) -> Result<()> {
    system::mkdir(system, STATE)?;
    for name in SCRIPTS {
        let bytes = system::read(system, app.join(name), 16 * 1024 * 1024)?;
        system::write(system, format!("{STATE}/{name}"), &bytes, 0o755)?;
    }
    Ok(())
}

fn install_hook(system: &impl System) -> Result<()> {
    system::mkdir(system, "/var/lib/webosbrew")?;
    system::mkdir(system, "/var/lib/webosbrew/init.d")?;
    if !hook_owned(system) {
        system::link(system, format!("{STATE}/screensaver.sh"), HOOK)?;
    }
    Ok(())
}

fn unmount(system: &impl System) -> Result<()> {
    if let Err(error) = system::command(system, "/bin/umount", &[TARGET]) {
        eprintln!("Luma screensaver: {error}; detaching mount");
        system::command(system, "/bin/umount", &["-l", TARGET])?;
    }
    if mounted(system) {
        return Err("The screensaver bind mount is still active".into());
    }
    Ok(())
}

/// Puts LG's screensaver back: no mount, SAM reading LG's own manifest again, no marker.
pub fn restore(system: &impl System) -> Result<()> {
    system::remove(system, MARKER)?;
    if foreign_mount(system)? {
        return Err("Another integration is mounted over LG's screensaver".into());
    }
    if mounted(system) {
        unmount(system)?;
    }
    home::reread(
        system,
        ID,
        &system::json(system, format!("{TARGET}/appinfo.json"))?,
    )
}

/// Mounts the stage (refreshed from the installed package) and makes SAM read it. A failure
/// after the mount restores LG's screensaver.
pub fn apply(system: &impl System, config: &Config) -> Result<()> {
    wait_for_sam(system)?;
    let app = Path::new(&config.app_dir);
    if system::exists(system, app.join("screensaver.sh")) {
        refresh_scripts(system, app);
    }
    if foreign_mount(system)? {
        return Err("Another integration is mounted over LG's screensaver".into());
    }
    stage(system, config)?;
    let result = (|| {
        if !mounted(system) {
            system::command(system, "/bin/mount", &["--bind", STAGE, TARGET])?;
        }
        if !mounted(system) {
            return Err("The screensaver bind mount did not take".into());
        }
        home::reread(system, ID, &manifest(&config.stock))?;
        system::write(system, MARKER, format!("{APP}\n").as_bytes(), 0o644)
    })();
    if let Err(error) = result {
        return match restore(system) {
            Ok(()) => Err(error),
            Err(recovery) => Err(format!("{error}\nRestore also failed: {recovery}")),
        };
    }
    if let Err(error) = install_hook(system) {
        eprintln!("Luma screensaver: boot hook not updated: {error}");
    }
    Ok(())
}

/// The installed package's path as the TV names it (a test root maps back onto `/`).
fn installed(system: &impl System, app: &Path) -> Result<std::path::PathBuf> {
    let physical = std::fs::canonicalize(system.path(app)).map_err(|e| e.to_string())?;
    let base = system.path(Path::new("/"));
    if base == Path::new("/") {
        return Ok(physical);
    }
    Ok(Path::new("/").join(
        physical
            .strip_prefix(base)
            .map_err(|_| "Installation escaped test root")?,
    ))
}

pub fn enable(system: &impl System, app: &Path) -> Result<()> {
    let release = String::from_utf8(system::read(system, "/etc/starfish-release", 64 * 1024)?)
        .map_err(|e| e.to_string())?;
    supported(&release)?;
    let app = installed(system, app)?;
    let package = system::json(system, app.join("appinfo.json"))?;
    if package["id"] != APP || !system::exists(system, app.join("screensaver").join(PAGE[0])) {
        return Err("Enable this from an installed Luma package with its screensaver page".into());
    }
    if foreign_mount(system)? {
        return Err("Another integration is mounted over LG's screensaver".into());
    }
    if system::exists(system, HOOK) && !hook_owned(system) {
        return Err("10-luma-screensaver is already in use".into());
    }
    let previous = load(system)?;
    let stock = if mounted(system) {
        previous
            .as_ref()
            .map(|c| c.stock.clone())
            .ok_or("Missing saved screensaver manifest")?
    } else {
        system::json(system, format!("{TARGET}/appinfo.json"))
            .map_err(|_| "LG's screensaver app was not found".to_string())?
    };
    if stock["id"] != ID || is_stage(&stock) {
        return Err("Could not identify LG's original screensaver manifest".into());
    }
    system::mkdir(system, STATE)?;
    system::mkdir(system, STAGE)?;
    if !mounted(system)
        && let Some(icon) = stock_icon(&stock)
        && let Ok(bytes) = system::read(system, format!("{TARGET}/{icon}"), LIMIT)
    {
        update(system, &format!("{STAGE}/{icon}"), &bytes, 0o644)?;
    }
    install_scripts(system, &app)?;
    let mut config = Config {
        enabled: false,
        app_dir: app.to_string_lossy().into_owned(),
        stock,
    };
    system::write_json(system, CONFIG, &config)?;
    apply(system, &config)?;
    config.enabled = true;
    system::write_json(system, CONFIG, &config)
}

pub fn disable(system: &impl System) -> Result<()> {
    if let Some(mut config) = load(system)? {
        config.enabled = false;
        system::write_json(system, CONFIG, &config)?;
    }
    restore(system)?;
    if hook_owned(system) {
        system::remove(system, HOOK)?;
    }
    Ok(())
}

pub fn status(system: &impl System) -> Result<Value> {
    let saved = load(system)?;
    let release = system::read(system, "/etc/starfish-release", 64 * 1024)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    Ok(json!({
        "enabled": saved.as_ref().is_some_and(|c| c.enabled),
        "mounted": mounted(system),
        "applied": system::exists(system, MARKER),
        "bootHook": hook_owned(system),
        "supported": supported(&release).is_ok(),
        "foreignMount": foreign_mount(system).unwrap_or(false),
        "appDir": saved.as_ref().map(|c| c.app_dir.clone()),
        "stockType": saved.as_ref().map(|c| c.stock["type"].clone()),
    }))
}

pub fn dispatch(system: &impl System, command: &str, app: Option<&str>) -> Result<Value> {
    system::root(system)?;
    match command {
        "status" => status(system),
        "enable" => {
            enable(
                system,
                Path::new(app.ok_or("Missing installation directory")?),
            )?;
            Ok(json!({"returnValue":true}))
        }
        "apply" => {
            match load(system)? {
                Some(config) if config.enabled => apply(system, &config)?,
                Some(_) => disable(system)?,
                None => return Err("The screensaver takeover has not been enabled".into()),
            }
            Ok(json!({"returnValue":true}))
        }
        "disable" | "restore" => {
            disable(system)?;
            Ok(json!({"returnValue":true}))
        }
        _ => Err("Unknown screensaver command".into()),
    }
}
