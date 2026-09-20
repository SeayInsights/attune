//! Attune's page and endpoints, mounted into the daemon's existing web server.
//!
//! # Why a separate page rather than edits to the existing UI
//!
//! The daemon serves a prebuilt Vite bundle whose source lives in another
//! repository. Editing it here is not possible, and vendoring the built output
//! would put a megabyte of compiled JavaScript in every diff.
//!
//! So Attune serves its own page at `/attune`, from its own crate, against its
//! own endpoints. Upstream's UI is left exactly as it is. The entire coupling to
//! upstream is one `.configure(attune_web::services)` call in the daemon's HTTP
//! server -- which is about as small as a merge conflict surface gets.
//!
//! # Why the endpoints talk to the daemon over HTTP
//!
//! These handlers run *inside* the daemon but reach it through its own public
//! API on loopback rather than its internal device channels. That keeps this
//! crate free of daemon internals, so upstream can restructure them without
//! breaking Attune. These are configuration operations measured in seconds; the
//! loopback round trip is irrelevant next to a fifteen second recording.

use std::time::Duration;

use actix_web::{HttpResponse, Responder, get, post, web};
use attune_analysis::capture;
use attune_analysis::measure::measure;
use attune_control::client::DaemonClient;
use attune_control::diagnose::diagnose;
use attune_tuner::{apply, derive, targets};
use serde::{Deserialize, Serialize};

/// The page itself. Single file, no build step, no bundler.
const PAGE: &str = include_str!("page.html");

/// Register Attune's routes.
pub fn services(cfg: &mut web::ServiceConfig) {
    cfg.service(page)
        .service(state)
        .service(list_targets)
        .service(devices)
        .service(tune);
}

#[get("/attune")]
async fn page() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(PAGE)
}

#[derive(Serialize)]
struct StateResponse {
    serial: String,
    device_type: String,
    profile: String,
    chain: attune_control::settings::MicChain,
    findings: Vec<FindingView>,
}

#[derive(Serialize)]
struct FindingView {
    severity: String,
    stage: String,
    summary: String,
    detail: String,
}

#[get("/api/attune/state")]
async fn state() -> impl Responder {
    let client = DaemonClient::default();

    let serial = match client.first_serial().await {
        Ok(s) => s,
        Err(e) => return error(&e.to_string()),
    };

    let mixer = match client.mixer(&serial).await {
        Ok(m) => m,
        Err(e) => return error(&e.to_string()),
    };

    let chain = match client.mic_chain(&serial).await {
        Ok(c) => c,
        Err(e) => return error(&e.to_string()),
    };

    let findings = diagnose(&chain)
        .into_iter()
        .map(|f| FindingView {
            severity: f.severity.label().to_string(),
            stage: f.stage.to_string(),
            summary: f.summary,
            detail: f.detail,
        })
        .collect();

    HttpResponse::Ok().json(StateResponse {
        serial,
        device_type: format!("{:?}", mixer.hardware.device_type),
        profile: mixer.profile_name,
        chain,
        findings,
    })
}

#[derive(Serialize)]
struct TargetView {
    name: &'static str,
    description: &'static str,
    speech_level_dbfs: f32,
    peak_ceiling_dbfs: f32,
    gate_margin_db: f32,
    compressor_depth_db: f32,
}

#[get("/api/attune/targets")]
async fn list_targets() -> impl Responder {
    let list: Vec<TargetView> = targets::ALL
        .iter()
        .map(|t| TargetView {
            name: t.name,
            description: t.description,
            speech_level_dbfs: t.speech_level_dbfs,
            peak_ceiling_dbfs: t.peak_ceiling_dbfs,
            gate_margin_db: t.gate_margin_db,
            compressor_depth_db: t.compressor_depth_db,
        })
        .collect();

    HttpResponse::Ok().json(list)
}

#[get("/api/attune/devices")]
async fn devices() -> impl Responder {
    HttpResponse::Ok().json(capture::list_input_devices())
}

#[derive(Deserialize)]
struct TuneRequest {
    #[serde(default = "default_seconds")]
    seconds: u64,
    #[serde(default = "default_target")]
    target: String,
    #[serde(default = "default_device")]
    device: String,
    /// False means dry run. Defaults to false on purpose: this writes to the
    /// device the operator is speaking into, and a missing field must never be
    /// read as consent.
    #[serde(default)]
    apply: bool,
}

fn default_seconds() -> u64 {
    15
}
fn default_target() -> String {
    "streaming".to_string()
}
fn default_device() -> String {
    "Chat Mic".to_string()
}

#[derive(Serialize)]
struct TuneResponse {
    measurement: attune_analysis::Measurement,
    usable: bool,
    usability: String,
    recommendation: attune_tuner::Recommendation,
    applied: Option<AppliedView>,
}

#[derive(Serialize)]
struct AppliedView {
    confirmed: Vec<String>,
    failed: Vec<FailedWrite>,
}

#[derive(Serialize)]
struct FailedWrite {
    setting: String,
    reason: String,
}

#[post("/api/attune/tune")]
async fn tune(req: web::Json<TuneRequest>) -> impl Responder {
    let Some(target) = targets::by_name(&req.target) else {
        return error(&format!("unknown target '{}'", req.target));
    };

    // Cap the recording so a bad request cannot hold a worker for an hour.
    let seconds = req.seconds.clamp(3, 60);

    let client = DaemonClient::default();

    let serial = match client.first_serial().await {
        Ok(s) => s,
        Err(e) => return error(&e.to_string()),
    };

    let chain = match client.mic_chain(&serial).await {
        Ok(c) => c,
        Err(e) => return error(&e.to_string()),
    };

    // cpal capture blocks. Keep it off the async workers.
    let device = req.device.clone();
    let captured = match tokio::task::spawn_blocking(move || {
        capture::record(&device, Duration::from_secs(seconds))
    })
    .await
    {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return error(&e.to_string()),
        Err(e) => return error(&format!("capture task failed: {e}")),
    };

    let measurement = measure(&captured.samples, captured.sample_rate);
    let usability = measurement.usability();
    let recommendation = derive(&measurement, &chain, target);

    let applied = if req.apply && !recommendation.is_empty() {
        match apply::apply(&client, &serial, &recommendation).await {
            Ok(a) => Some(AppliedView {
                confirmed: a.confirmed,
                failed: a
                    .failed
                    .into_iter()
                    .map(|(setting, reason)| FailedWrite { setting, reason })
                    .collect(),
            }),
            Err(e) => return error(&e.to_string()),
        }
    } else {
        None
    };

    HttpResponse::Ok().json(TuneResponse {
        usable: usability == attune_analysis::measure::Usability::Usable,
        usability: usability.explain(),
        measurement,
        recommendation,
        applied,
    })
}

/// A consistent error shape, so the page has one thing to render.
fn error(message: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({ "error": message }))
}
