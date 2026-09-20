//! Bypass, crossfeed, loudness, meters and profile auto-switching.
//!
//! Five features that share one property: they are all about the audio
//! *around* the correction rather than the correction itself, so they live
//! together rather than growing `eq.rs` by half again.
//!
//! Persistence follows the same rule as everything else here -- crossfeed and
//! the plugin choice are per bus and per GoXLR profile, because they are
//! tuning decisions and tuning decisions belong to the profile that made them.
//! The auto-switch rules are deliberately *not*: a rule that says "load the
//! Competitive profile for cs2.exe" cannot itself live inside a profile
//! without becoming unreachable the moment another one is loaded.

use std::collections::HashMap;
use std::sync::Mutex;

use actix_web::{HttpResponse, Responder, get, post, web};
use attune_control::autoswitch::{Rules, foreground_executable};
use attune_control::client::DaemonClient;
use attune_eq::crossfeed::{self, Crossfeed};
use attune_eq::apo;
use serde::{Deserialize, Serialize};

use crate::eq::BUSES;

pub fn services(cfg: &mut web::ServiceConfig) {
    cfg.service(state)
        .service(set_bypass)
        .service(set_crossfeed)
        .service(set_plugin)
        .service(meters)
        .service(silence_interference)
        .service(autoswitch_state)
        .service(set_autoswitch);
}

fn error(message: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({ "error": message }))
}

// ------------------------------------------------------------------ bypass

/// Whether Attune is currently bypassed.
///
/// Process state rather than a file on disk. Bypass is a momentary thing --
/// you hold it, listen, and let go -- and a bypass that survived a daemon
/// restart would be a silent way to have no correction at all.
static BYPASSED: Mutex<bool> = Mutex::new(false);

pub fn is_bypassed() -> bool {
    BYPASSED.lock().map(|b| *b).unwrap_or(false)
}

fn set_bypassed(value: bool) {
    if let Ok(mut bypassed) = BYPASSED.lock() {
        *bypassed = value;
    }
}

#[derive(Deserialize)]
struct BypassRequest {
    on: bool,
}

#[post("/api/attune/extras/bypass")]
async fn set_bypass(req: web::Json<BypassRequest>) -> impl Responder {
    if apo::detect().is_none() {
        return error("Equalizer APO is not installed, so there is nothing to bypass");
    }

    if req.on {
        set_bypassed(true);
        match crate::eq::bypass().await {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "bypassed": true })),
            Err(e) => {
                set_bypassed(false);
                error(&e)
            }
        }
    } else {
        set_bypassed(false);
        // Re-derive rather than remembering the old file: the settings may
        // have changed while bypassed, and the settings are the truth.
        match crate::eq::reapply().await {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "bypassed": false })),
            Err(e) => error(&e),
        }
    }
}

// -------------------------------------------------------------- crossfeed

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct BusExtras {
    #[serde(default)]
    pub crossfeed: Option<Crossfeed>,
    #[serde(default)]
    pub plugin: Option<String>,
}

#[derive(Deserialize)]
struct CrossfeedRequest {
    bus: String,
    /// `None` turns it off.
    #[serde(default)]
    crossfeed: Option<Crossfeed>,
    /// Apply to every bus. On by default in the UI: you have one head, and
    /// crossfeed on Game but not Music is a change you hear when you alt-tab.
    #[serde(default)]
    all_buses: bool,
}

#[post("/api/attune/extras/crossfeed")]
async fn set_crossfeed(req: web::Json<CrossfeedRequest>) -> impl Responder {
    let sane = req.crossfeed.map(|c| c.sane());
    let targets: Vec<String> = if req.all_buses {
        BUSES.iter().map(|b| b.to_string()).collect()
    } else {
        vec![req.bus.clone()]
    };

    match crate::eq::update_extras(&targets, |extras| extras.crossfeed = sane).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "crossfeed": sane,
            "buses": targets,
        })),
        Err(e) => error(&e),
    }
}

// --------------------------------------------------------------- loudness

#[derive(Deserialize)]
struct PluginRequest {
    bus: String,
    /// A file name inside APO's `VSTPlugins` directory. `None` removes it.
    #[serde(default)]
    plugin: Option<String>,
    #[serde(default)]
    all_buses: bool,
}

#[post("/api/attune/extras/plugin")]
async fn set_plugin(req: web::Json<PluginRequest>) -> impl Responder {
    let targets: Vec<String> = if req.all_buses {
        BUSES.iter().map(|b| b.to_string()).collect()
    } else {
        vec![req.bus.clone()]
    };

    let plugin = req.plugin.clone();
    if let Err(e) = crate::eq::update_extras(&targets, |extras| extras.plugin = plugin.clone()).await
    {
        return error(&e);
    }

    // Ask APO whether it actually loaded the thing, because it will not say so
    // otherwise: an unrecognised directive and a plugin that failed to load
    // both produce silence, no error, and no effect.
    let verified = match (apo::detect(), &req.plugin) {
        (Some(install), Some(_)) => {
            let devices = crate::eq::render_devices();
            match crate::eq::device_for(&req.bus, &devices) {
                Some(device) => {
                    let bus = apo::BusCurve::from_composite(&device, Default::default());
                    apo::plugin_loaded(&install, &bus.endpoint, bus.connection.as_deref())
                }
                None => None,
            }
        }
        _ => None,
    };

    HttpResponse::Ok().json(serde_json::json!({
        "plugin": req.plugin,
        "buses": targets,
        // true: APO reported loading it. false: it did not. null: could not be
        // determined, which is not the same as either and is not reported as
        // one.
        "loaded": verified,
    }))
}

/// The VST plugins sitting in APO's plugin directory.
fn available_plugins() -> Vec<String> {
    let Some(install) = apo::detect() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(install.root.join("VSTPlugins")) else {
        return Vec::new();
    };

    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.to_lowercase().ends_with(".dll").then_some(name)
        })
        .collect();
    out.sort();
    out
}

/// Comment out whatever Equalizer APO loads before Attune.
///
/// In the app because the alternative is telling someone to open a text file
/// in Program Files and edit it, which is both worse and the thing this
/// project exists to stop.
#[post("/api/attune/extras/silence-interference")]
async fn silence_interference() -> impl Responder {
    let Some(install) = apo::detect() else {
        return error("Equalizer APO is not installed");
    };

    match apo::silence_interference(&install) {
        Ok(lines) => HttpResponse::Ok().json(serde_json::json!({
            "commented": lines,
            "backup": install.main_config().with_extension("txt.attune-backup"),
        })),
        Err(e) => error(&e.to_string()),
    }
}

// ----------------------------------------------------------------- meters

/// The running meters, started on first read and stopped when nobody looks.
static METERS: Mutex<Option<attune_analysis::meters::Meters>> = Mutex::new(None);

#[get("/api/attune/extras/meters")]
async fn meters() -> impl Responder {
    let mut running = match METERS.lock() {
        Ok(m) => m,
        Err(poisoned) => poisoned.into_inner(),
    };

    // Meters time themselves out, so a stale set is replaced rather than
    // reused -- its capture threads have already gone.
    let stale = running.as_ref().is_some_and(|m| !m.wanted());
    if running.is_none() || stale {
        let (started, _names, failures) = attune_analysis::meters::Meters::start(BUSES);
        for failure in &failures {
            log::debug!("meter: {failure}");
        }
        *running = Some(started);
    }

    let levels = running
        .as_ref()
        .map(|m| m.read())
        .unwrap_or_else(HashMap::new);

    HttpResponse::Ok().json(serde_json::json!({ "levels": levels }))
}

// ------------------------------------------------------------- autoswitch

fn rules_path() -> std::path::PathBuf {
    crate::eq::settings_path().join("autoswitch.json")
}

pub(crate) fn load_rules() -> Rules {
    std::fs::read_to_string(rules_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_rules(rules: &Rules) -> Result<(), String> {
    let path = rules_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

#[get("/api/attune/extras/autoswitch")]
async fn autoswitch_state() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "rules": load_rules(),
        // What is in front right now, so adding a rule for it is one click
        // rather than a hunt through Task Manager for the executable name.
        "foreground": foreground_executable(),
    }))
}

#[derive(Deserialize)]
struct AutoswitchRequest {
    #[serde(default)]
    enabled: Option<bool>,
    /// Add or replace a rule.
    #[serde(default)]
    set: Option<(String, String)>,
    /// Remove the rule for this executable.
    #[serde(default)]
    remove: Option<String>,
    /// Enable or disable one rule without deleting it.
    #[serde(default)]
    toggle: Option<(String, bool)>,
}

#[post("/api/attune/extras/autoswitch")]
async fn set_autoswitch(req: web::Json<AutoswitchRequest>) -> impl Responder {
    let mut rules = load_rules();

    if let Some(enabled) = req.enabled {
        rules.enabled = enabled;
    }
    if let Some((executable, profile)) = &req.set {
        rules.set(executable, profile);
    }
    if let Some(executable) = &req.remove {
        rules.remove(executable);
    }
    if let Some((executable, enabled)) = &req.toggle
        && let Some(rule) = rules
            .rules
            .iter_mut()
            .find(|r| r.executable.eq_ignore_ascii_case(executable))
    {
        rule.enabled = *enabled;
    }

    match save_rules(&rules) {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "rules": rules })),
        Err(e) => error(&format!("could not save: {e}")),
    }
}

// ---------------------------------------------------------------- state

#[get("/api/attune/extras/state")]
async fn state() -> impl Responder {
    let profiles = match DaemonClient::default().status().await {
        Ok(status) => status.files.profiles,
        Err(_) => Vec::new(),
    };

    HttpResponse::Ok().json(serde_json::json!({
        "bypassed": is_bypassed(),
        "crossfeed_presets": crossfeed::PRESETS
            .iter()
            .map(|(name, settings, description)| serde_json::json!({
                "name": name,
                "settings": settings,
                "description": description,
            }))
            .collect::<Vec<_>>(),
        "crossfeed_limits": {
            "level_db": crossfeed::LEVEL_RANGE_DB,
            "cutoff_hz": crossfeed::CUTOFF_RANGE_HZ,
            "delay_us": crossfeed::DELAY_RANGE_US,
        },
        "plugins": available_plugins(),
        // Said plainly rather than left for someone to discover: APO's config
        // language has no dynamics processing, so this needs a plugin and
        // Attune does not ship one.
        "plugin_guidance": "Loudness levelling needs a compressor, and Equalizer \
                            APO's configuration language has no dynamics \
                            processing -- only fixed filters, gain and delays. So \
                            it runs a VST plugin, and Attune ships none: put a \
                            compressor .dll in Equalizer APO's VSTPlugins folder \
                            and it will appear here. Whether APO actually loaded \
                            it is checked and reported, because APO will not say \
                            so by itself.",
        "profiles": profiles,
        "interference": apo::detect().map(|i| apo::interference(&i)).unwrap_or_default(),
    }))
}

// ------------------------------------------------------------ the watcher

/// Start the profile watcher, once.
///
/// Called from `services()`, which actix runs once per worker thread -- hence
/// the guard. Two watchers would race each other into loading the same profile
/// twice.
pub(crate) fn spawn_autoswitch() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        tokio::spawn(watch());
    });
}

/// Poll the foreground application and load the profile its rule names.
///
/// Deliberately quiet. It logs at debug, never surfaces a notification, and
/// does nothing at all until someone turns it on -- software that changes your
/// audio unasked is software you uninstall. A failure to reach the daemon is
/// not worth reporting either: the device is simply not there yet, which
/// happens every time the machine boots.
async fn watch() {
    let client = DaemonClient::default();
    let mut ticker = tokio::time::interval(attune_control::autoswitch::POLL_INTERVAL);

    loop {
        ticker.tick().await;

        let rules = load_rules();
        if !rules.enabled {
            continue;
        }

        let Some(foreground) = foreground_executable() else {
            continue;
        };

        let Ok(serial) = client.first_serial().await else {
            continue;
        };
        let Ok(mixer) = client.mixer(&serial).await else {
            continue;
        };

        let Some(wanted) =
            attune_control::autoswitch::decide(&rules, Some(&foreground), &mixer.profile_name)
        else {
            continue;
        };

        match client.load_profile(&serial, &wanted).await {
            Ok(()) => log::info!("autoswitch: {foreground} -> profile {wanted}"),
            Err(e) => log::debug!("autoswitch could not load {wanted}: {e}"),
        }
    }
}