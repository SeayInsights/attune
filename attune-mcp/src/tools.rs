//! The tools Attune exposes, and how each maps onto the daemon's HTTP API.
//!
//! # Why these are declarative
//!
//! Each tool is a name, a description, a schema, and a route. Keeping them as
//! data rather than a match arm per tool means the list a model sees and the
//! call that actually happens cannot drift apart -- there is only one place a
//! tool exists.
//!
//! # Why writes are opt-in
//!
//! Anything that changes the device takes an explicit flag that defaults to
//! false. A model deciding on its own to rewrite someone's microphone settings
//! mid-stream is a bad outcome, and the protection against it should not be the
//! model's judgement. The description says so too, so a model reading the tool
//! list knows it is asking permission rather than discovering a refusal.

use serde_json::{Value, json};

/// How a tool reaches the daemon.
pub enum Route {
    Get(&'static str),
    Post(&'static str),
}

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub route: Route,
    /// JSON Schema for the arguments.
    pub schema: fn() -> Value,
}

fn no_args() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

pub fn all() -> Vec<Tool> {
    vec![
        Tool {
            name: "attune_status",
            description: "Read the connected GoXLR: model, serial, active profile, the full \
                 microphone chain (mic type, preamp gain, gate, compressor, \
                 equaliser), and rule-based findings about how it is configured. \
                 Read-only.",
            route: Route::Get("/api/attune/state"),
            schema: no_args,
        },
        Tool {
            name: "attune_measure",
            description: "Record the microphone and measure it: noise floor, speech level, \
                 peak, signal-to-noise, crest factor, clipping, and the spectrum \
                 as octave bands. The recording starts immediately and runs for \
                 the given number of seconds -- the person must speak throughout \
                 or the result is rejected as unusable. Changes nothing.",
            route: Route::Post("/api/attune/tune"),
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "seconds": {
                            "type": "integer", "minimum": 3, "maximum": 60, "default": 15,
                            "description": "How long to record. Shorter than 10 rarely captures enough speech."
                        },
                        "target": {
                            "type": "string", "default": "streaming",
                            "description": "Tuning target: streaming, podcast, or broadcast."
                        }
                    },
                    "additionalProperties": false
                })
            },
        },
        Tool {
            name: "attune_tune_mic",
            description: "Measure the microphone and derive settings for a target, then \
                 optionally write them to the device. Every write is read back and \
                 verified. Set apply=true ONLY when the person has asked for the \
                 change to be made -- it alters the microphone they are speaking \
                 into. With apply=false it reports what it would change and why, \
                 which is the right default. Expect several passes: corrections \
                 are bounded per pass so a bad measurement cannot swing a setting \
                 to its limit.",
            route: Route::Post("/api/attune/tune"),
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "seconds": { "type": "integer", "minimum": 3, "maximum": 60, "default": 15 },
                        "target": { "type": "string", "default": "streaming" },
                        "apply": {
                            "type": "boolean", "default": false,
                            "description": "Write the derived settings to the device. Requires the person's explicit agreement."
                        }
                    },
                    "additionalProperties": false
                })
            },
        },
        Tool {
            name: "attune_targets",
            description: "List the microphone tuning targets and what each one aims for: \
                 speech level, peak ceiling, gate margin, compression depth, and \
                 the voice curve it shapes toward.",
            route: Route::Get("/api/attune/targets"),
            schema: no_args,
        },
        Tool {
            name: "attune_headphones",
            description: "Read the per-bus headphone correction: which voicing and imported \
                 measurement each GoXLR output bus (Game, Music, Chat, System) is \
                 using, the resulting response curve, and whether Equalizer APO is \
                 installed and loading it. Read-only.",
            route: Route::Get("/api/attune/eq/state"),
            schema: no_args,
        },
        Tool {
            name: "attune_set_voicing",
            description: "Set the voicing for one output bus. Voicings are opinions about a \
                 use case rather than headphone corrections: neutral, competitive \
                 (cuts bass so quiet detail like footsteps is not masked), music, \
                 voice. With write=false the choice is saved but not applied to the \
                 system audio configuration.",
            route: Route::Post("/api/attune/eq/apply"),
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "bus": {
                            "type": "string", "enum": ["Game", "Music", "Chat", "System"],
                            "description": "Which GoXLR output bus."
                        },
                        "voicing": {
                            "type": "string", "enum": ["neutral", "competitive", "music", "voice"]
                        },
                        "write": {
                            "type": "boolean", "default": false,
                            "description": "Also write to Equalizer APO, changing what the person hears. Requires their explicit agreement."
                        }
                    },
                    "required": ["bus", "voicing"],
                    "additionalProperties": false
                })
            },
        },
        Tool {
            name: "attune_search_headphone",
            description: "Search AutoEQ's published headphone measurements by model name. \
                 Results include who measured it and on what rig -- the same \
                 headphone measured on different rigs gives different corrections, \
                 and which to use is a real choice, so present the options rather \
                 than picking silently.",
            route: Route::Get("/api/attune/eq/search"),
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "q": { "type": "string", "description": "Model name, e.g. 'DT 990 Pro'." }
                    },
                    "required": ["q"],
                    "additionalProperties": false
                })
            },
        },
        Tool {
            name: "attune_import_correction",
            description: "Import an AutoEQ measurement as the headphone correction for one \
                 bus, using a path from attune_search_headphone. This saves the \
                 correction; it does not write to the system audio configuration \
                 until a voicing is applied with write=true.",
            route: Route::Post("/api/attune/eq/import-autoeq"),
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "bus": { "type": "string", "enum": ["Game", "Music", "Chat", "System"] },
                        "path": { "type": "string", "description": "The path field from a search result." }
                    },
                    "required": ["bus", "path"],
                    "additionalProperties": false
                })
            },
        },
        Tool {
            name: "attune_verify_eq",
            description: "Prove whether the headphone correction is actually being applied, \
                 by playing pink noise through a bus and measuring what returns on \
                 the GoXLR's Stream Mix. Reading the configuration back cannot \
                 answer this: a device directive that matches nothing fails \
                 silently. Audible to the person for a few seconds -- tell them \
                 first.",
            route: Route::Post("/api/attune/eq/verify"),
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "bus": { "type": "string", "enum": ["Game", "Music", "Chat", "System"] },
                        "seconds": { "type": "integer", "minimum": 3, "maximum": 20, "default": 6 }
                    },
                    "required": ["bus"],
                    "additionalProperties": false
                })
            },
        },
        Tool {
            name: "attune_profiles",
            description: "List saved Attune profiles and which is active. A profile holds \
                 the per-bus headphone curves and references a GoXLR mic profile by \
                 name. Read-only.",
            route: Route::Get("/api/attune/profiles"),
            schema: no_args,
        },
        Tool {
            name: "attune_apply_profile",
            description: "Apply a saved profile: load its GoXLR mic profile and write its \
                 headphone curves. This changes both the microphone and what the \
                 person hears, so only call it when they have asked for that \
                 profile. Reports the two halves separately, because one can land \
                 while the other fails.",
            route: Route::Post("/api/attune/profiles/apply"),
            schema: || {
                json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"],
                    "additionalProperties": false
                })
            },
        },
    ]
}

/// The tool list in the shape MCP expects.
pub fn describe() -> Value {
    let tools: Vec<Value> = all()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(),
            })
        })
        .collect();
    json!({ "tools": tools })
}
