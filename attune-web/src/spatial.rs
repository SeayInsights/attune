//! Spatial audio endpoints.
//!
//! Thin. All the thinking is in `attune_eq::spatial`; this turns it into JSON
//! and keeps the per-bus shape the Headphones tab already works in.
//!
//! On anything that is not Windows these return a flat "not available here"
//! rather than disappearing, so the UI has one shape to render and does not
//! have to guess why a section is missing.

use actix_web::{HttpResponse, Responder, get, post, web};
use serde::Deserialize;

use crate::eq::BUSES;

#[derive(Deserialize)]
struct SetRequest {
    bus: String,
    /// A subtype string from a previous `state` call. Opaque to the UI.
    subtype: String,
    /// Apply to every bus. On by default in the UI for the same reason the
    /// headphone correction is: there is one pair of headphones, and having
    /// Game virtualised while Music is not is a difference you would hear as
    /// the mix changing when you alt-tab.
    #[serde(default)]
    all_buses: bool,
}

pub fn services(cfg: &mut web::ServiceConfig) {
    cfg.service(state).service(set);
}

fn error(message: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({ "error": message }))
}

#[cfg(windows)]
#[get("/api/attune/spatial/state")]
async fn state() -> impl Responder {
    use attune_eq::spatial;

    // One entry per bus, each carrying its own active format. They are
    // genuinely independent -- Windows tracks this per endpoint -- so
    // reporting a single global value would be a lie that happens to be true
    // most of the time.
    let mut buses = Vec::new();
    let mut first_error: Option<String> = None;

    for bus in BUSES {
        match spatial::status(bus) {
            Ok(status) => buses.push(serde_json::json!({
                "name": bus,
                "device": status.device,
                "active": status.active,
                "active_label": status.active_label,
                "formats": status.formats,
            })),
            Err(e) => {
                // A missing bus is normal -- not everyone has all four, and
                // the device may be unplugged. Record why and carry on rather
                // than failing the whole request over one endpoint.
                log::debug!("spatial status for {bus}: {e}");
                first_error.get_or_insert_with(|| e.to_string());
            }
        }
    }

    HttpResponse::Ok().json(serde_json::json!({
        "supported": true,
        "buses": buses,
        "note": first_error,
    }))
}

#[cfg(not(windows))]
#[get("/api/attune/spatial/state")]
async fn state() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "supported": false,
        "buses": [],
        "note": "Windows spatial audio is a Windows feature; there is no \
                 equivalent to switch on this platform.",
    }))
}

#[cfg(windows)]
#[post("/api/attune/spatial/set")]
async fn set(req: web::Json<SetRequest>) -> impl Responder {
    use attune_eq::spatial;

    let targets: Vec<&str> = if req.all_buses {
        BUSES.to_vec()
    } else {
        vec![req.bus.as_str()]
    };

    let mut changed = Vec::new();
    let mut failures = Vec::new();

    for bus in targets {
        match spatial::set(bus, &req.subtype) {
            Ok(status) => changed.push(serde_json::json!({
                "name": bus,
                "active_label": status.active_label,
            })),
            Err(e) => failures.push(format!("{bus}: {e}")),
        }
    }

    // Partial success is the realistic case -- four endpoints, any of which
    // may be absent -- so report both halves rather than picking one.
    if changed.is_empty() {
        return error(&failures.join("; "));
    }

    HttpResponse::Ok().json(serde_json::json!({
        "changed": changed,
        "failed": failures,
    }))
}

#[cfg(not(windows))]
#[post("/api/attune/spatial/set")]
async fn set(_req: web::Json<SetRequest>) -> impl Responder {
    error("Windows spatial audio is not available on this platform")
}
