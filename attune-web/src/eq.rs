//! Headphone correction endpoints.
//!
//! State lives in a JSON file beside the daemon's own configuration rather than
//! in Equalizer APO's config directory. APO's directory is under Program Files
//! and usually needs administrator rights to write; keeping Attune's own choices
//! somewhere writable means the app still remembers them when applying fails.

use std::path::PathBuf;

use actix_web::{HttpResponse, Responder, get, post, web};
use attune_eq::{BusSetup, Curve, apo, profiles};
use serde::{Deserialize, Serialize};

/// The GoXLR's Windows endpoints, in the order the mixer shows them.
///
/// Matched by substring against what Windows reports, because the vendor
/// decorates device names and that decoration changes between driver versions.
const BUSES: &[&str] = &["Game", "Music", "Chat", "System"];

pub fn services(cfg: &mut web::ServiceConfig) {
    cfg.service(state)
        .service(apply)
        .service(import)
        .service(search)
        .service(import_autoeq);
}

/// AutoEQ's index, fetched once and kept.
///
/// It is ~830 KB and changes when new measurements are published, which is not
/// often enough to justify re-fetching per keystroke. A process restart picks up
/// anything new.
static INDEX: tokio::sync::OnceCell<Vec<attune_eq::autoeq::Entry>> =
    tokio::sync::OnceCell::const_new();

async fn index() -> Result<&'static Vec<attune_eq::autoeq::Entry>, String> {
    INDEX
        .get_or_try_init(|| async {
            let text = reqwest::get(attune_eq::autoeq::INDEX_URL)
                .await
                .map_err(|e| format!("could not reach AutoEQ: {e}"))?
                .text()
                .await
                .map_err(|e| format!("could not read AutoEQ's index: {e}"))?;

            let entries = attune_eq::autoeq::parse_index(&text);
            if entries.is_empty() {
                return Err("AutoEQ's index was empty or its format changed".to_string());
            }
            Ok(entries)
        })
        .await
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Serialize)]
struct SearchHit {
    name: String,
    provenance: String,
    path: String,
}

#[get("/api/attune/eq/search")]
async fn search(query: web::Query<SearchQuery>) -> impl Responder {
    let entries = match index().await {
        Ok(e) => e,
        Err(e) => return error(&e),
    };

    let hits: Vec<SearchHit> = attune_eq::autoeq::search(entries, &query.q, 25)
        .into_iter()
        .map(|e| SearchHit {
            name: e.name.clone(),
            provenance: e.provenance(),
            path: e.path.clone(),
        })
        .collect();

    HttpResponse::Ok().json(hits)
}

#[derive(Deserialize)]
struct AutoEqImport {
    bus: String,
    /// Path within AutoEQ's results, as returned by the search.
    path: String,
}

#[post("/api/attune/eq/import-autoeq")]
async fn import_autoeq(req: web::Json<AutoEqImport>) -> impl Responder {
    let entries = match index().await {
        Ok(e) => e,
        Err(e) => return error(&e),
    };

    // Resolve against the index rather than trusting the path from the request:
    // this endpoint fetches a URL built from it, and an arbitrary path would
    // make that a request-controlled fetch.
    let Some(entry) = entries.iter().find(|e| e.path == req.path) else {
        return error("that measurement is not in AutoEQ's index");
    };

    let text = match reqwest::get(entry.parametric_url()).await {
        Ok(r) if r.status().is_success() => match r.text().await {
            Ok(t) => t,
            Err(e) => return error(&format!("could not read the curve: {e}")),
        },
        Ok(r) => {
            return error(&format!(
                "AutoEQ returned {} for that measurement -- it may not publish a \
                 parametric export for it",
                r.status()
            ));
        }
        Err(e) => return error(&format!("could not reach AutoEQ: {e}")),
    };

    let name = format!("{} ({})", entry.name, entry.provenance());
    let curve = match Curve::parse(&name, &text) {
        Ok(c) => c,
        Err(e) => return error(&e.to_string()),
    };

    let filters = curve.filters.len();
    let mut settings = load();
    settings.corrections.insert(req.bus.clone(), curve);

    if let Err(e) = save(&settings) {
        return error(&format!("could not save correction: {e}"));
    }

    HttpResponse::Ok().json(serde_json::json!({
        "imported": true,
        "name": name,
        "filters": filters,
    }))
}

/// Where Attune keeps its own settings.
fn settings_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("Attune")
}

fn settings_file() -> PathBuf {
    settings_path().join("headphones.json")
}

/// What the operator has chosen for each bus.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Settings {
    /// Bus name to voicing name.
    #[serde(default)]
    voicings: std::collections::HashMap<String, String>,
    /// Bus name to imported headphone correction.
    #[serde(default)]
    corrections: std::collections::HashMap<String, Curve>,
}

fn load() -> Settings {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(settings: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(settings_path())?;
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_file(), text)
}

#[derive(Serialize)]
struct ApoStatus {
    installed: bool,
    config_dir: Option<String>,
    /// Whether APO's main config currently pulls Attune's file in.
    included: bool,
    /// What to do about it, in the operator's terms.
    guidance: String,
}

#[derive(Serialize)]
struct BusState {
    name: String,
    /// The Windows endpoint, if one matching this bus was found.
    device: Option<String>,
    voicing: String,
    correction_name: Option<String>,
    /// Peak boost the composed curve asks for, before headroom, in dB.
    peak_boost_db: f32,
    /// How much preamp headroom was applied, in dB.
    headroom_db: f32,
    /// The composed response, as (Hz, dB) points for drawing.
    response: Vec<(f32, f32)>,
}

#[derive(Serialize)]
struct VoicingView {
    name: &'static str,
    description: &'static str,
}

#[derive(Serialize)]
struct EqState {
    apo: ApoStatus,
    buses: Vec<BusState>,
    voicings: Vec<VoicingView>,
}

/// Windows render endpoints, so a bus can be matched to a real device.
fn render_devices() -> Vec<String> {
    // The daemon already enumerates these for its own purposes, but going
    // through cpal here keeps this module independent of daemon internals.
    attune_analysis::capture::list_output_devices()
}

fn device_for(bus: &str, devices: &[String]) -> Option<String> {
    devices
        .iter()
        .find(|d| d.to_lowercase().contains(&bus.to_lowercase()))
        .cloned()
}

fn build_bus(name: &str, settings: &Settings, devices: &[String]) -> BusState {
    let voicing_name = settings
        .voicings
        .get(name)
        .cloned()
        .unwrap_or_else(|| "neutral".to_string());
    let voicing = profiles::by_name(&voicing_name).unwrap_or(profiles::NEUTRAL);
    let correction = settings.corrections.get(name);

    let managed = attune_eq::build(&BusSetup {
        device: name.to_string(),
        correction: correction.cloned(),
        voicing,
    });

    // A third-octave sweep is plenty for a plot and keeps the payload small.
    let mut response = Vec::new();
    let mut hz = 20.0_f32;
    while hz <= 20_000.0 {
        response.push((hz, managed.curve.response_at(hz)));
        hz *= 1.259_921; // 2^(1/3)
    }

    BusState {
        name: name.to_string(),
        device: device_for(name, devices),
        voicing: voicing.name.to_string(),
        correction_name: correction.map(|c| c.name.clone()),
        peak_boost_db: managed.original_peak_db,
        headroom_db: managed.headroom_applied_db,
        response,
    }
}

#[get("/api/attune/eq/state")]
async fn state() -> impl Responder {
    let settings = load();
    let devices = render_devices();
    let install = apo::detect();

    let apo_status = match &install {
        Some(i) => ApoStatus {
            installed: true,
            config_dir: Some(i.config_dir.display().to_string()),
            included: i.is_included(),
            guidance: if i.is_included() {
                "Equalizer APO is installed and loading Attune's configuration.".to_string()
            } else {
                "Equalizer APO is installed. Applying a curve will add one \
                 Include line to its configuration."
                    .to_string()
            },
        },
        None => ApoStatus {
            installed: false,
            config_dir: None,
            included: false,
            guidance: "Equalizer APO is not installed, so headphone correction \
                       cannot be applied. It is free and open source. Attune does \
                       not install it for you: it needs administrator rights, a \
                       reboot, and it changes the system audio pipeline, so that \
                       is your decision rather than this app's. Curves can still \
                       be chosen and previewed here in the meantime."
                .to_string(),
        },
    };

    HttpResponse::Ok().json(EqState {
        apo: apo_status,
        buses: BUSES
            .iter()
            .map(|b| build_bus(b, &settings, &devices))
            .collect(),
        voicings: profiles::ALL
            .iter()
            .map(|v| VoicingView {
                name: v.name,
                description: v.description,
            })
            .collect(),
    })
}

#[derive(Deserialize)]
struct ApplyRequest {
    bus: String,
    voicing: String,
    /// False previews only. Default false so a missing field cannot be read as
    /// permission to write to the system audio configuration.
    #[serde(default)]
    write: bool,
}

#[post("/api/attune/eq/apply")]
async fn apply(req: web::Json<ApplyRequest>) -> impl Responder {
    if profiles::by_name(&req.voicing).is_none() {
        return error(&format!("unknown voicing '{}'", req.voicing));
    }

    let mut settings = load();
    settings
        .voicings
        .insert(req.bus.clone(), req.voicing.clone());

    if let Err(e) = save(&settings) {
        return error(&format!("could not save settings: {e}"));
    }

    if !req.write {
        return HttpResponse::Ok().json(serde_json::json!({ "saved": true, "written": false }));
    }

    let Some(install) = apo::detect() else {
        return error(
            "Equalizer APO is not installed, so there is nothing to write to. \
             The choice has been saved and will apply once it is.",
        );
    };

    let devices = render_devices();
    let buses: Vec<apo::BusCurve> = BUSES
        .iter()
        .filter_map(|name| {
            let device = device_for(name, &devices)?;
            let voicing = settings
                .voicings
                .get(*name)
                .and_then(|v| profiles::by_name(v))
                .unwrap_or(profiles::NEUTRAL);

            // A bus set to neutral with no correction has nothing to say; a
            // Device section with no filters would only add noise to the file.
            let correction = settings.corrections.get(*name).cloned();
            if correction.is_none() && voicing.name == "neutral" {
                return None;
            }

            let managed = attune_eq::build(&BusSetup {
                device: device.clone(),
                correction,
                voicing,
            });
            Some(apo::BusCurve {
                device,
                curve: managed.curve,
            })
        })
        .collect();

    match apo::apply(&install, &buses) {
        Ok(result) => HttpResponse::Ok().json(serde_json::json!({
            "saved": true,
            "written": true,
            "added_include": result == apo::Applied::WrittenAndIncluded,
            "buses": buses.len(),
        })),
        Err(e) => error(&e.to_string()),
    }
}

#[derive(Deserialize)]
struct ImportRequest {
    bus: String,
    /// A name for the correction, usually the headphone model.
    name: String,
    /// The text of an AutoEQ ParametricEQ export.
    text: String,
}

#[post("/api/attune/eq/import")]
async fn import(req: web::Json<ImportRequest>) -> impl Responder {
    let curve = match Curve::parse(&req.name, &req.text) {
        Ok(c) => c,
        Err(e) => return error(&e.to_string()),
    };

    let filters = curve.filters.len();
    let mut settings = load();
    settings.corrections.insert(req.bus.clone(), curve);

    if let Err(e) = save(&settings) {
        return error(&format!("could not save correction: {e}"));
    }

    HttpResponse::Ok().json(serde_json::json!({
        "imported": true,
        "filters": filters,
    }))
}

fn error(message: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({ "error": message }))
}
