//! Profiles: one switch that changes the mic and the headphones together.
//!
//! # What a profile is, and deliberately is not
//!
//! The GoXLR already has profiles, and they already store the whole mic chain --
//! gain, gate, compressor, equaliser. Duplicating that here would mean two
//! places storing the same settings, drifting apart, and an operator having to
//! know which one won.
//!
//! So an Attune profile stores only what the device cannot: the per-bus
//! headphone corrections, which live in Equalizer APO and are invisible to the
//! GoXLR. It *references* a GoXLR mic profile by name rather than copying its
//! contents.
//!
//! Applying a profile therefore does two things: asks the device to load its own
//! mic profile, and writes the headphone configuration. Each half stays owned by
//! whichever system is actually authoritative for it.

use std::collections::HashMap;
use std::path::PathBuf;

use actix_web::{HttpResponse, Responder, get, post, web};
use attune_control::client::DaemonClient;
use attune_eq::{BusSetup, Curve, apo, profiles as voicings};
use serde::{Deserialize, Serialize};

use crate::eq;

pub fn services(cfg: &mut web::ServiceConfig) {
    cfg.service(list)
        .service(save_profile)
        .service(apply_profile)
        .service(delete_profile);
}

fn profiles_file() -> PathBuf {
    eq::settings_path().join("profiles.json")
}

/// What a profile holds for one bus.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BusChoice {
    pub voicing: String,
    /// The measured correction, stored by value.
    ///
    /// Kept rather than referenced because a profile should survive the
    /// correction being changed on the live bus -- otherwise switching away and
    /// back would silently pick up whatever is current rather than what was
    /// saved, which is the opposite of what a saved profile is for.
    pub correction: Option<Curve>,
}

/// A saved combination.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// The GoXLR mic profile to load, if the operator wants one loaded.
    ///
    /// Referenced by name, not copied: the device owns the mic chain.
    #[serde(default)]
    pub mic_profile: Option<String>,
    #[serde(default)]
    pub buses: HashMap<String, BusChoice>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    profiles: Vec<Profile>,
    #[serde(default)]
    active: Option<String>,
}

fn load_store() -> Store {
    std::fs::read_to_string(profiles_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_store(store: &Store) -> std::io::Result<()> {
    std::fs::create_dir_all(eq::settings_path())?;
    let text = serde_json::to_string_pretty(store)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(profiles_file(), text)
}

#[derive(Serialize)]
struct ProfileView {
    name: String,
    mic_profile: Option<String>,
    /// Bus name to voicing, for a one-line summary in the UI.
    buses: HashMap<String, String>,
    has_corrections: bool,
}

#[derive(Serialize)]
struct ListResponse {
    profiles: Vec<ProfileView>,
    active: Option<String>,
    /// Mic profiles the device reports, so one can be chosen when saving.
    mic_profiles: Vec<String>,
}

#[get("/api/attune/profiles")]
async fn list() -> impl Responder {
    let store = load_store();

    // The device is the authority on which mic profiles exist. If it cannot be
    // reached the list is empty rather than stale or invented.
    let mic_profiles = match DaemonClient::default().status().await {
        Ok(s) => s.files.mic_profiles,
        Err(_) => Vec::new(),
    };

    HttpResponse::Ok().json(ListResponse {
        profiles: store
            .profiles
            .iter()
            .map(|p| ProfileView {
                name: p.name.clone(),
                mic_profile: p.mic_profile.clone(),
                buses: p
                    .buses
                    .iter()
                    .map(|(k, v)| (k.clone(), v.voicing.clone()))
                    .collect(),
                has_corrections: p.buses.values().any(|b| b.correction.is_some()),
            })
            .collect(),
        active: store.active,
        mic_profiles,
    })
}

#[derive(Deserialize)]
struct SaveRequest {
    name: String,
    /// The GoXLR mic profile this profile should load, if any.
    #[serde(default)]
    mic_profile: Option<String>,
}

/// Snapshot the current headphone configuration under a name.
#[post("/api/attune/profiles/save")]
async fn save_profile(req: web::Json<SaveRequest>) -> impl Responder {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return error("a profile needs a name");
    }

    let current = eq::load();
    let buses: HashMap<String, BusChoice> = eq::BUSES
        .iter()
        .map(|bus| {
            (
                bus.to_string(),
                BusChoice {
                    voicing: current
                        .voicings
                        .get(*bus)
                        .cloned()
                        .unwrap_or_else(|| "neutral".to_string()),
                    correction: current.corrections.get(*bus).cloned(),
                },
            )
        })
        .collect();

    let mut store = load_store();
    let profile = Profile {
        name: name.clone(),
        mic_profile: req.mic_profile.clone(),
        buses,
    };

    // Saving over a name replaces it rather than making a second entry with the
    // same name, which would be unreachable by name afterwards.
    match store.profiles.iter_mut().find(|p| p.name == name) {
        Some(existing) => *existing = profile,
        None => store.profiles.push(profile),
    }
    store.active = Some(name.clone());

    match save_store(&store) {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "saved": name })),
        Err(e) => error(&format!("could not save: {e}")),
    }
}

#[derive(Deserialize)]
struct NameRequest {
    name: String,
}

/// Apply a profile: load the device's mic profile and write the headphone config.
#[post("/api/attune/profiles/apply")]
async fn apply_profile(req: web::Json<NameRequest>) -> impl Responder {
    let mut store = load_store();
    let Some(profile) = store.profiles.iter().find(|p| p.name == req.name).cloned() else {
        return error(&format!("no profile named '{}'", req.name));
    };

    let mut applied: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    // --- The device's half ------------------------------------------------

    if let Some(mic) = &profile.mic_profile {
        let client = DaemonClient::default();
        match client.first_serial().await {
            Ok(serial) => {
                let cmd = goxlr_ipc::GoXLRCommand::LoadMicProfile(mic.clone(), false);
                match client.command(&serial, cmd).await {
                    Ok(()) => applied.push(format!("mic profile '{mic}'")),
                    Err(e) => failed.push(format!("mic profile '{mic}': {e}")),
                }
            }
            Err(e) => failed.push(format!("mic profile '{mic}': {e}")),
        }
    }

    // --- Attune's half ----------------------------------------------------

    let mut settings = eq::load();
    for (bus, choice) in &profile.buses {
        settings
            .voicings
            .insert(bus.clone(), choice.voicing.clone());
        match &choice.correction {
            Some(c) => {
                settings.corrections.insert(bus.clone(), c.clone());
            }
            None => {
                settings.corrections.remove(bus);
            }
        }
    }

    if let Err(e) = eq::save(&settings) {
        return error(&format!("could not save headphone settings: {e}"));
    }

    match apo::detect() {
        Some(install) => {
            let devices = eq::render_devices();
            let buses: Vec<apo::BusCurve> = eq::BUSES
                .iter()
                .filter_map(|name| {
                    let device = eq::device_for(name, &devices)?;
                    let voicing = settings
                        .voicings
                        .get(*name)
                        .and_then(|v| voicings::by_name(v))
                        .unwrap_or(voicings::NEUTRAL);
                    let correction = settings.corrections.get(*name).cloned();
                    if correction.is_none() && voicing.name == "neutral" {
                        return None;
                    }
                    let managed = attune_eq::build(&BusSetup {
                        device: device.clone(),
                        correction,
                        voicing,
                    });
                    Some(apo::BusCurve::from_composite(&device, managed.curve))
                })
                .collect();

            match apo::apply(&install, &buses) {
                Ok(_) => applied.push(format!("headphone curves on {} bus(es)", buses.len())),
                Err(e) => failed.push(format!("headphone curves: {e}")),
            }
        }
        None => failed.push(
            "headphone curves: Equalizer APO is not installed, so they were saved \
             but not applied"
                .to_string(),
        ),
    }

    store.active = Some(profile.name.clone());
    let _ = save_store(&store);

    HttpResponse::Ok().json(serde_json::json!({
        "profile": profile.name,
        "applied": applied,
        "failed": failed,
    }))
}

#[post("/api/attune/profiles/delete")]
async fn delete_profile(req: web::Json<NameRequest>) -> impl Responder {
    let mut store = load_store();
    let before = store.profiles.len();
    store.profiles.retain(|p| p.name != req.name);

    if store.profiles.len() == before {
        return error(&format!("no profile named '{}'", req.name));
    }
    if store.active.as_deref() == Some(req.name.as_str()) {
        store.active = None;
    }

    match save_store(&store) {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "deleted": req.name })),
        Err(e) => error(&format!("could not save: {e}")),
    }
}

fn error(message: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({ "error": message }))
}
