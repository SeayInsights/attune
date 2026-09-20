//! Headphone correction endpoints.
//!
//! # Settings belong to a GoXLR profile
//!
//! There is one list of profiles in this application, and it is the device's.
//! Attune keys its headphone settings by the active profile's name, so they
//! follow it: switch profile and the correction switches with it. There is
//! deliberately no second profile list and no second save button -- two
//! competing lists of profiles is exactly the confusion worth avoiding.
//!
//! What the device cannot store -- the headphone curves, which live in Equalizer
//! APO and are invisible to it -- is kept here and looked up by name.
//!
//! # Three layers, composed
//!
//! A bus's final curve is the sum of three things, kept separate because they
//! answer different questions:
//!
//! 1. **Correction** -- measured, headphone-specific, imported from AutoEQ.
//!    "What is wrong with these headphones?"
//! 2. **Voicing** -- an opinion about a use case, not headphone-specific.
//!    "What do I want them to do?"
//! 3. **Manual** -- per-band trim set by ear. "I disagree."
//!
//! Collapsing them into one editable curve would mean re-importing a correction
//! wipes the person's own adjustments, which is the wrong trade.

use std::collections::HashMap;
use std::path::PathBuf;

use actix_web::{HttpResponse, Responder, get, post, web};
use attune_control::client::DaemonClient;
use attune_eq::curve::{Filter, FilterKind};
use attune_eq::{Curve, apo, profiles};
use serde::{Deserialize, Serialize};

/// The GoXLR's Windows output endpoints, in the order the mixer shows them.
pub(crate) const BUSES: &[&str] = &["Game", "Music", "Chat", "System"];

/// Centres for the manual trim, matching the device's own equaliser labels so
/// the two read the same way.
pub(crate) const BAND_CENTRES: [f32; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// How far a manual band may be pushed, in dB.
const MANUAL_LIMIT_DB: f32 = 12.0;

/// How far a macro tone control may be pushed, in dB.
const MACRO_LIMIT_DB: f32 = 10.0;

pub fn services(cfg: &mut web::ServiceConfig) {
    cfg.service(state)
        .service(set_bus)
        .service(apply)
        .service(search)
        .service(import_autoeq)
        .service(clear_correction)
        .service(verify);
}

// ---------------------------------------------------------------- storage

pub(crate) fn settings_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("Attune")
}

fn settings_file() -> PathBuf {
    settings_path().join("headphones.json")
}

/// Broad tone controls, the three most people actually reach for.
///
/// Separate from the per-band trim because they answer a different question:
/// "a bit more bass" is not the same request as "cut 3 dB at 125 Hz", and
/// collapsing them would mean nudging a macro scribbles over bands somebody set
/// deliberately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Macros {
    #[serde(default)]
    pub bass: f32,
    #[serde(default)]
    pub voice: f32,
    #[serde(default)]
    pub treble: f32,
}

impl Macros {
    fn filters(&self) -> Vec<Filter> {
        let mut out = Vec::new();
        let clamp = |v: f32| v.clamp(-MACRO_LIMIT_DB, MACRO_LIMIT_DB);

        if self.bass.abs() >= 0.05 {
            out.push(Filter {
                kind: FilterKind::LowShelf,
                freq_hz: 200.0,
                gain_db: clamp(self.bass),
                q: 0.7,
            });
        }
        if self.voice.abs() >= 0.05 {
            // Wide rather than narrow: this is meant to shift the whole
            // presence region, not carve a notch in it.
            out.push(Filter {
                kind: FilterKind::Peaking,
                freq_hz: 1800.0,
                gain_db: clamp(self.voice),
                q: 0.8,
            });
        }
        if self.treble.abs() >= 0.05 {
            out.push(Filter {
                kind: FilterKind::HighShelf,
                freq_hz: 5000.0,
                gain_db: clamp(self.treble),
                q: 0.7,
            });
        }
        out
    }

    fn is_flat(&self) -> bool {
        self.filters().is_empty()
    }
}

/// One bus's layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BusSettings {
    #[serde(default = "neutral")]
    pub voicing: String,
    #[serde(default)]
    pub correction: Option<Curve>,
    /// Per-band trim in dB, one entry per [`BAND_CENTRES`].
    #[serde(default)]
    pub manual: Vec<f32>,
    /// Broad bass / voice / treble tilt.
    #[serde(default)]
    pub macros: Macros,
}

fn neutral() -> String {
    "neutral".to_string()
}

impl Default for BusSettings {
    fn default() -> Self {
        Self {
            voicing: neutral(),
            correction: None,
            manual: vec![0.0; BAND_CENTRES.len()],
            macros: Macros::default(),
        }
    }
}

impl BusSettings {
    /// The manual trim as filters, skipping bands left at zero.
    fn manual_filters(&self) -> Vec<Filter> {
        self.manual
            .iter()
            .zip(BAND_CENTRES.iter())
            .filter(|(gain, _)| gain.abs() >= 0.05)
            .map(|(gain, hz)| Filter {
                kind: FilterKind::Peaking,
                freq_hz: *hz,
                gain_db: gain.clamp(-MANUAL_LIMIT_DB, MANUAL_LIMIT_DB),
                // One octave wide, so adjacent bands overlap into a smooth
                // shape rather than ten isolated spikes.
                q: 1.41,
            })
            .collect()
    }
}

/// Everything, keyed by GoXLR profile name then bus name.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    profiles: HashMap<String, HashMap<String, BusSettings>>,
}

/// The shape settings had before they were keyed by profile.
///
/// Read so that an upgrade does not silently discard a correction somebody
/// searched for and imported. Schema changes are the author's problem, not the
/// operator's.
#[derive(Debug, Default, Deserialize)]
struct LegacyStore {
    #[serde(default)]
    voicings: HashMap<String, String>,
    #[serde(default)]
    corrections: HashMap<String, Curve>,
}

/// Load, folding any pre-profile settings into `profile` on first sight.
fn load_store_for(profile: &str) -> Store {
    let Ok(text) = std::fs::read_to_string(settings_file()) else {
        return Store::default();
    };

    let mut store: Store = serde_json::from_str(&text).unwrap_or_default();
    if !store.profiles.is_empty() {
        return store;
    }

    let legacy: LegacyStore = serde_json::from_str(&text).unwrap_or_default();
    if legacy.voicings.is_empty() && legacy.corrections.is_empty() {
        return store;
    }

    let target = store.profiles.entry(profile.to_string()).or_default();
    for (bus, voicing) in legacy.voicings {
        target.entry(bus).or_default().voicing = voicing;
    }
    for (bus, correction) in legacy.corrections {
        target.entry(bus).or_default().correction = Some(correction);
    }

    // Persist immediately so the migration happens once rather than on every
    // read, and so the file on disk matches what the app believes.
    let _ = save_store(&store);
    store
}

fn save_store(store: &Store) -> std::io::Result<()> {
    std::fs::create_dir_all(settings_path())?;
    let text = serde_json::to_string_pretty(store)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_file(), text)
}

/// The device's active profile name, which is the key everything hangs off.
async fn active_profile() -> Result<String, String> {
    let client = DaemonClient::default();
    let serial = client.first_serial().await.map_err(|e| e.to_string())?;
    let mixer = client.mixer(&serial).await.map_err(|e| e.to_string())?;
    Ok(mixer.profile_name)
}

// ---------------------------------------------------------------- shared

pub(crate) fn render_devices() -> Vec<String> {
    attune_analysis::capture::list_output_devices()
}

pub(crate) fn device_for(bus: &str, devices: &[String]) -> Option<String> {
    devices
        .iter()
        .find(|d| d.to_lowercase().contains(&bus.to_lowercase()))
        .cloned()
}

/// Build the final curve for a bus: correction, then voicing, then manual trim.
fn compose(settings: &BusSettings) -> attune_eq::headroom::Managed {
    let voicing = profiles::by_name(&settings.voicing).unwrap_or(profiles::NEUTRAL);
    let mut composed = profiles::compose(settings.correction.as_ref(), voicing);
    composed.filters.extend(settings.manual_filters());
    composed.filters.extend(settings.macros.filters());
    attune_eq::headroom::manage(&composed, attune_eq::headroom::DEFAULT_MARGIN_DB)
}

fn is_empty(settings: &BusSettings) -> bool {
    settings.correction.is_none()
        && settings.voicing == "neutral"
        && settings.manual_filters().is_empty()
        && settings.macros.is_flat()
}

fn write_config(store: &Store, profile: &str) -> Result<usize, String> {
    let Some(install) = apo::detect() else {
        return Err(
            "Equalizer APO is not installed, so there is nothing to write to. \
                    The curves are saved and will apply once it is."
                .to_string(),
        );
    };

    let devices = render_devices();
    let default = BusSettings::default();
    let empty = HashMap::new();
    let per_bus = store.profiles.get(profile).unwrap_or(&empty);

    let buses: Vec<apo::BusCurve> = BUSES
        .iter()
        .filter_map(|name| {
            let device = device_for(name, &devices)?;
            let settings = per_bus.get(*name).unwrap_or(&default);

            // A bus with nothing set contributes nothing; an empty Device
            // section would only add noise to the file.
            if is_empty(settings) {
                return None;
            }

            Some(apo::BusCurve::from_composite(
                &device,
                compose(settings).curve,
            ))
        })
        .collect();

    apo::apply(&install, &buses)
        .map(|_| buses.len())
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- state

#[derive(Serialize)]
struct ApoStatus {
    installed: bool,
    included: bool,
    guidance: String,
    /// Whether APO is still in the audio path everywhere it was set up.
    /// False means the curves are not running, whatever the rest of this says.
    attached: bool,
    /// The endpoints it is no longer attached to, by name.
    detached: Vec<String>,
}

#[derive(Serialize)]
struct BusView {
    name: String,
    device: Option<String>,
    voicing: String,
    correction_name: Option<String>,
    manual: Vec<f32>,
    macros: Macros,
    headroom_db: f32,
    /// The composed response as (Hz, dB) for drawing.
    response: Vec<(f32, f32)>,
}

#[derive(Serialize)]
struct VoicingView {
    name: &'static str,
    description: &'static str,
}

#[derive(Serialize)]
struct StateResponse {
    /// The GoXLR profile these settings belong to.
    profile: String,
    apo: ApoStatus,
    buses: Vec<BusView>,
    voicings: Vec<VoicingView>,
    band_centres: Vec<f32>,
    manual_limit_db: f32,
    macro_limit_db: f32,
}

#[get("/api/attune/eq/state")]
async fn state() -> impl Responder {
    let profile = match active_profile().await {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    let store = load_store_for(&profile);
    let empty = HashMap::new();
    let per_bus = store.profiles.get(&profile).unwrap_or(&empty);
    let devices = render_devices();
    let default = BusSettings::default();

    let install = apo::detect();
    let attachment = attune_eq::attachment::check();
    let apo_status = match &install {
        Some(i) => ApoStatus {
            installed: true,
            included: i.is_included(),
            // A detached endpoint outranks everything else this message could
            // say. The curves are still on disk, the UI still draws them, and
            // the audio is flat -- a failure that looks exactly like success,
            // so it has to be the sentence a person reads first.
            guidance: if !attachment.healthy() {
                attachment.explain()
            } else if i.is_included() {
                "Equalizer APO is installed and loading these curves.".to_string()
            } else {
                "Equalizer APO is installed. Pressing Apply adds one line to its \
                 configuration so it loads these curves."
                    .to_string()
            },
            attached: attachment.healthy(),
            detached: attachment.detached.clone(),
        },
        None => ApoStatus {
            installed: false,
            included: false,
            guidance: "Equalizer APO is not installed, so nothing here can reach \
                       your headphones yet. It is free and open source. Attune \
                       does not install it for you -- it needs administrator \
                       rights, a reboot, and it changes the system audio \
                       pipeline. You can still set curves up here meanwhile."
                .to_string(),
            attached: false,
            detached: Vec::new(),
        },
    };

    let buses = BUSES
        .iter()
        .map(|name| {
            let settings = per_bus.get(*name).unwrap_or(&default);
            let managed = compose(settings);

            // The response is reported WITHOUT the preamp, so the plot shows the
            // tonal shape rather than the shape plus a large constant offset.
            //
            // Headroom management can drop the preamp by 10 dB or more, which
            // would push the whole curve to the floor of a +/-12 dB plot and read
            // as "this does nothing" exactly where it does the most. The offset
            // is reported separately as headroom_db, which is where it belongs:
            // it is a level decision, not a tonal one.
            let preamp = managed.curve.preamp_db;

            let mut response = Vec::new();
            let mut hz = 20.0_f32;
            while hz < 20_000.0 {
                response.push((hz, managed.curve.response_at(hz) - preamp));
                // Sixth-octave: fine enough that a narrow band reads as a bump
                // rather than a corner, still a small payload.
                hz *= 1.122_462;
            }
            // The geometric walk lands at 18.2 kHz and the next step overshoots,
            // so the last sixth of an octave would be missing and the plotted
            // line would stop short of the right-hand edge -- which reads as
            // "nothing happens up here" rather than "the sample grid ended".
            response.push((20_000.0, managed.curve.response_at(20_000.0) - preamp));

            BusView {
                name: name.to_string(),
                device: device_for(name, &devices),
                voicing: settings.voicing.clone(),
                correction_name: settings.correction.as_ref().map(|c| c.name.clone()),
                manual: if settings.manual.len() == BAND_CENTRES.len() {
                    settings.manual.clone()
                } else {
                    vec![0.0; BAND_CENTRES.len()]
                },
                macros: settings.macros,
                headroom_db: managed.headroom_applied_db,
                response,
            }
        })
        .collect();

    HttpResponse::Ok().json(StateResponse {
        profile,
        apo: apo_status,
        buses,
        voicings: profiles::ALL
            .iter()
            .map(|v| VoicingView {
                name: v.name,
                description: v.description,
            })
            .collect(),
        band_centres: BAND_CENTRES.to_vec(),
        manual_limit_db: MANUAL_LIMIT_DB,
        macro_limit_db: MACRO_LIMIT_DB,
    })
}

// ---------------------------------------------------------------- edit

#[derive(Deserialize)]
struct SetRequest {
    bus: String,
    #[serde(default)]
    voicing: Option<String>,
    /// Full manual trim, one entry per band.
    #[serde(default)]
    manual: Option<Vec<f32>>,
    /// Broad bass / voice / treble tilt.
    #[serde(default)]
    macros: Option<Macros>,
}

/// Change one bus. Saves against the active profile; does not write to APO.
#[post("/api/attune/eq/set")]
async fn set_bus(req: web::Json<SetRequest>) -> impl Responder {
    let profile = match active_profile().await {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    if let Some(v) = &req.voicing
        && profiles::by_name(v).is_none()
    {
        return error(&format!("unknown voicing '{v}'"));
    }

    let mut store = load_store_for(&profile);
    let bus = store
        .profiles
        .entry(profile.clone())
        .or_default()
        .entry(req.bus.clone())
        .or_default();

    if let Some(v) = &req.voicing {
        bus.voicing = v.clone();
    }
    if let Some(m) = &req.manual {
        bus.manual = (0..BAND_CENTRES.len())
            .map(|i| {
                m.get(i)
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(-MANUAL_LIMIT_DB, MANUAL_LIMIT_DB)
            })
            .collect();
    }
    if let Some(m) = req.macros {
        bus.macros = Macros {
            bass: m.bass.clamp(-MACRO_LIMIT_DB, MACRO_LIMIT_DB),
            voice: m.voice.clamp(-MACRO_LIMIT_DB, MACRO_LIMIT_DB),
            treble: m.treble.clamp(-MACRO_LIMIT_DB, MACRO_LIMIT_DB),
        };
    }

    match save_store(&store) {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "saved": true, "profile": profile })),
        Err(e) => error(&format!("could not save: {e}")),
    }
}

#[derive(Deserialize)]
struct BusRequest {
    bus: String,
}

#[post("/api/attune/eq/clear-correction")]
async fn clear_correction(req: web::Json<BusRequest>) -> impl Responder {
    let profile = match active_profile().await {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    let mut store = load_store_for(&profile);
    if let Some(bus) = store.profiles.entry(profile).or_default().get_mut(&req.bus) {
        bus.correction = None;
    }

    match save_store(&store) {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "cleared": req.bus })),
        Err(e) => error(&format!("could not save: {e}")),
    }
}

/// Write the active profile's curves to Equalizer APO.
#[post("/api/attune/eq/apply")]
async fn apply() -> impl Responder {
    let profile = match active_profile().await {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    let store = load_store_for(&profile);
    match write_config(&store, &profile) {
        Ok(count) => HttpResponse::Ok()
            .json(serde_json::json!({ "written": true, "buses": count, "profile": profile })),
        Err(e) => error(&e),
    }
}

// ---------------------------------------------------------------- AutoEQ

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
    path: String,
    /// Apply the same correction to every bus, which is usually what is wanted:
    /// one pair of headphones is on the person's head regardless of which bus
    /// the sound came from.
    #[serde(default)]
    all_buses: bool,
}

#[post("/api/attune/eq/import-autoeq")]
async fn import_autoeq(req: web::Json<AutoEqImport>) -> impl Responder {
    let profile = match active_profile().await {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    let entries = match index().await {
        Ok(e) => e,
        Err(e) => return error(&e),
    };

    // Resolved against the index rather than trusted: this builds a URL from it.
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
                "AutoEQ returned {} -- it may not publish a parametric export for \
                 that measurement",
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
    let filter_count = curve.filters.len();

    let mut store = load_store_for(&profile);
    let profile_map = store.profiles.entry(profile).or_default();

    let targets: Vec<String> = if req.all_buses {
        BUSES.iter().map(|b| b.to_string()).collect()
    } else {
        vec![req.bus.clone()]
    };

    for bus in &targets {
        profile_map.entry(bus.clone()).or_default().correction = Some(curve.clone());
    }

    if let Err(e) = save_store(&store) {
        return error(&format!("could not save: {e}"));
    }

    HttpResponse::Ok().json(serde_json::json!({
        "imported": true,
        "name": name,
        "filters": filter_count,
        "buses": targets,
    }))
}

// ---------------------------------------------------------------- verify

#[derive(Deserialize)]
struct VerifyRequest {
    bus: String,
    #[serde(default = "default_return")]
    return_device: String,
    #[serde(default = "default_verify_seconds")]
    seconds: u64,
}

fn default_return() -> String {
    "Stream Mix".to_string()
}
fn default_verify_seconds() -> u64 {
    6
}

/// Play pink noise through a bus and measure what comes back.
///
/// The only way to know whether Equalizer APO is actually applying a correction:
/// a directive that matches no device fails silently, so reading the
/// configuration back proves nothing.
#[post("/api/attune/eq/verify")]
async fn verify(req: web::Json<VerifyRequest>) -> impl Responder {
    let seconds = req.seconds.clamp(3, 20);
    let bus = req.bus.clone();
    let return_device = req.return_device.clone();

    let devices = render_devices();
    let Some(out_device) = device_for(&bus, &devices) else {
        return error(&format!("no output device matching '{bus}'"));
    };

    let play_device = out_device.clone();
    let player = tokio::task::spawn_blocking(move || {
        let samples = attune_analysis::playback::pink_noise(48_000 * seconds as usize, 1);
        attune_analysis::playback::play(&play_device, &samples, 48_000)
    });

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let capture_seconds = seconds.saturating_sub(2).max(2);
    let recorder = tokio::task::spawn_blocking(move || {
        attune_analysis::capture::record(
            &return_device,
            std::time::Duration::from_secs(capture_seconds),
        )
    });

    let captured = match recorder.await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return error(&e.to_string()),
        Err(e) => return error(&format!("capture task failed: {e}")),
    };
    let _ = player.await;

    let m = attune_analysis::measure::measure(&captured.samples, captured.sample_rate);

    HttpResponse::Ok().json(serde_json::json!({
        "bus": bus,
        "played_to": out_device,
        "captured_from": captured.device_name,
        "peak_dbfs": m.peak_dbfs,
        "bands": m.bands,
    }))
}

fn error(message: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({ "error": message }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bus_with_nothing_set_produces_a_flat_curve() {
        let managed = compose(&BusSettings::default());
        assert!(managed.curve.response_at(1000.0).abs() < 0.01);
        assert!(is_empty(&BusSettings::default()));
    }

    #[test]
    fn manual_bands_left_at_zero_produce_no_filters() {
        assert!(BusSettings::default().manual_filters().is_empty());
    }

    #[test]
    fn a_manual_band_lands_at_its_own_frequency() {
        let mut s = BusSettings::default();
        s.manual[5] = 6.0; // 1 kHz

        let filters = s.manual_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].freq_hz, 1000.0);
        assert_eq!(filters[0].gain_db, 6.0);
    }

    /// A manual boost must not be able to clip: headroom management has to see
    /// it like any other layer.
    #[test]
    fn a_manual_boost_gets_headroom_and_cannot_clip() {
        let mut s = BusSettings::default();
        s.manual[2] = 10.0;

        let managed = compose(&s);
        assert!(managed.headroom_applied_db > 5.0);

        let mut hz = 20.0_f32;
        while hz < 20_000.0 {
            assert!(managed.curve.response_at(hz) <= 0.01, "clips at {hz:.0} Hz");
            hz *= 1.059_463_1;
        }
    }

    #[test]
    fn manual_gains_are_clamped_to_the_stated_limit() {
        let mut s = BusSettings::default();
        s.manual[0] = 999.0;
        s.manual[1] = -999.0;

        let filters = s.manual_filters();
        assert_eq!(filters[0].gain_db, MANUAL_LIMIT_DB);
        assert_eq!(filters[1].gain_db, -MANUAL_LIMIT_DB);
    }

    /// The three layers stay separate, so re-importing a correction cannot wipe
    /// the person's own adjustments.
    #[test]
    fn the_layers_compose_without_overwriting_each_other() {
        let mut manual = vec![0.0; BAND_CENTRES.len()];
        manual[8] = 4.0;

        let s = BusSettings {
            voicing: "competitive".to_string(),
            manual,
            correction: Some(Curve {
                name: "test".into(),
                preamp_db: 0.0,
                filters: vec![Filter {
                    kind: FilterKind::Peaking,
                    freq_hz: 500.0,
                    gain_db: -3.0,
                    q: 1.0,
                }],
            }),
            macros: Macros::default(),
        };

        let managed = compose(&s);
        let expected = 1 + profiles::COMPETITIVE.curve().filters.len() + 1;
        assert_eq!(managed.curve.filters.len(), expected);
        assert!(!is_empty(&s));
    }

    /// Settings are stored per profile, so two profiles do not share a bus.
    /// Settings are stored per profile, so two profiles do not share a bus.
    #[test]
    fn two_profiles_hold_independent_settings() {
        let mut store = Store::default();

        let pc = BusSettings {
            voicing: "competitive".into(),
            ..BusSettings::default()
        };
        let sleep = BusSettings {
            voicing: "music".into(),
            ..BusSettings::default()
        };

        store
            .profiles
            .entry("PC".into())
            .or_default()
            .insert("Game".into(), pc);
        store
            .profiles
            .entry("Sleep".into())
            .or_default()
            .insert("Game".into(), sleep);

        assert_eq!(store.profiles["PC"]["Game"].voicing, "competitive");
        assert_eq!(store.profiles["Sleep"]["Game"].voicing, "music");
    }
}
