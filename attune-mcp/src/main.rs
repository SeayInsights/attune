//! `attune-mcp` -- Attune as an MCP server.
//!
//! # What this is for
//!
//! Attune ships no AI. It has no API keys, no model, and no vendor dependency,
//! and everything it does works with nothing configured -- the tuner derives
//! settings deterministically from measurement.
//!
//! This binary is the optional seam: it speaks MCP over stdio and translates
//! calls into the daemon's existing HTTP API, so somebody who wants a model to
//! drive Attune can point their own at it. Which model, and whether to use one
//! at all, is entirely theirs to decide.
//!
//! # Why it is a separate process
//!
//! MCP clients launch a server as a child process and talk to it over stdin and
//! stdout. Nothing is written to stdout that is not a protocol message --
//! diagnostics go to stderr, because a stray line on stdout corrupts the stream
//! and produces a failure that looks like the client's fault.
//!
//! # Setup
//!
//! Add to an MCP client's configuration:
//!
//! ```json
//! { "mcpServers": { "attune": { "command": "path/to/attune-mcp.exe" } } }
//! ```
//!
//! The Attune daemon must be running; this talks to it, it does not replace it.

mod tools;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The MCP revision this implements.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Where the daemon listens.
const DAEMON: &str = "http://127.0.0.1:14564";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = tokio::io::stdout();
    let http = reqwest::Client::new();

    while let Some(line) = lines.next_line().await? {
        // Strip a byte-order mark before trimming. Some writers put one at the
        // head of the stream, and a BOM makes JSON parsing fail at column 1 with
        // a message that says nothing about why -- which is a miserable thing to
        // debug for a character that carries no meaning here.
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("attune-mcp: could not parse a message: {e}");
                continue;
            }
        };

        // A notification has no id and takes no response. Replying to one is a
        // protocol error, not a harmless extra.
        let Some(id) = request.get("id").cloned() else {
            continue;
        };

        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(json!({}));

        let response = match handle(method, params, &http).await {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": e.to_string() }
            }),
        };

        stdout.write_all(format!("{response}\n").as_bytes()).await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn handle(method: &str, params: Value, http: &reqwest::Client) -> anyhow::Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "attune", "version": env!("CARGO_PKG_VERSION") }
        })),

        "tools/list" => Ok(tools::describe()),

        "tools/call" => call_tool(params, http).await,

        // Answering ping keeps a client from deciding the server is dead.
        "ping" => Ok(json!({})),

        other => Err(anyhow::anyhow!("unsupported method '{other}'")),
    }
}

async fn call_tool(params: Value, http: &reqwest::Client) -> anyhow::Result<Value> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("tools/call needs a name"))?;

    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    let tool = tools::all()
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| anyhow::anyhow!("no tool named '{name}'"))?;

    let outcome = match tool.route {
        tools::Route::Get(path) => {
            let mut url = format!("{DAEMON}{path}");
            // GET tools carry their arguments as query parameters.
            if let Some(object) = arguments.as_object()
                && !object.is_empty()
            {
                let query: Vec<String> = object
                    .iter()
                    .map(|(k, v)| {
                        let raw = match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        format!("{k}={}", urlencode(&raw))
                    })
                    .collect();
                url.push('?');
                url.push_str(&query.join("&"));
            }
            http.get(url).send().await
        }
        tools::Route::Post(path) => {
            http.post(format!("{DAEMON}{path}"))
                .json(&arguments)
                .send()
                .await
        }
    };

    let response = outcome.map_err(|e| {
        anyhow::anyhow!(
            "could not reach the Attune daemon at {DAEMON}: {e}. It has to be \
             running -- this server talks to it rather than replacing it."
        )
    })?;

    let status = response.status();
    let body: Value = response
        .json()
        .await
        .unwrap_or_else(|e| json!({ "error": format!("unreadable response: {e}") }));

    // A daemon-reported failure is surfaced as tool content with isError, not as
    // a protocol error: the model should read what went wrong and decide, rather
    // than being told the call itself was malformed.
    let failed = !status.is_success() || body.get("error").is_some();

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
        }],
        "isError": failed
    }))
}

/// Percent-encode a query value.
///
/// Written out rather than pulled in: one dependency for one function that has
/// to handle exactly the characters headphone model names contain.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_declares_an_object_schema() {
        for tool in tools::all() {
            let schema = (tool.schema)();
            assert_eq!(
                schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "{} has no object schema",
                tool.name
            );
            assert!(
                schema.get("properties").is_some(),
                "{} has no properties",
                tool.name
            );
        }
    }

    #[test]
    fn tool_names_are_unique() {
        let mut names: Vec<&str> = tools::all().iter().map(|t| t.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate tool name");
    }

    /// Anything that changes the device must default to not doing so.
    #[test]
    fn writing_tools_default_to_not_writing() {
        for tool in tools::all() {
            let schema = (tool.schema)();
            let props = schema.get("properties").unwrap();

            for flag in ["apply", "write"] {
                if let Some(field) = props.get(flag) {
                    assert_eq!(
                        field.get("default"),
                        Some(&json!(false)),
                        "{}'s {flag} must default to false",
                        tool.name
                    );
                    assert!(
                        !schema
                            .get("required")
                            .and_then(|r| r.as_array())
                            .map(|r| r.iter().any(|v| v == flag))
                            .unwrap_or(false),
                        "{}'s {flag} must not be required, or omitting it would be impossible",
                        tool.name
                    );
                }
            }
        }
    }

    /// A model reading the list should learn that a write needs agreement,
    /// rather than discovering it by being refused.
    #[test]
    fn writing_tools_say_so_in_their_description() {
        for tool in tools::all() {
            let schema = (tool.schema)();
            let props = schema.get("properties").unwrap();
            let writes = props.get("apply").is_some() || props.get("write").is_some();

            if writes {
                let d = tool.description.to_lowercase();
                assert!(
                    d.contains("agreement") || d.contains("asked"),
                    "{} can change the device but does not say consent is needed",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn initialize_reports_tool_capability() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let http = reqwest::Client::new();
        let result = rt.block_on(handle("initialize", json!({}), &http)).unwrap();

        assert_eq!(
            result.get("protocolVersion").and_then(|v| v.as_str()),
            Some(PROTOCOL_VERSION)
        );
        assert!(result.pointer("/capabilities/tools").is_some());
    }

    #[test]
    fn tools_list_matches_the_declared_tools() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let http = reqwest::Client::new();
        let result = rt.block_on(handle("tools/list", json!({}), &http)).unwrap();

        let listed = result.get("tools").and_then(|t| t.as_array()).unwrap();
        assert_eq!(listed.len(), tools::all().len());
        assert!(listed.iter().all(|t| t.get("inputSchema").is_some()));
    }

    #[test]
    fn an_unknown_method_is_an_error_not_a_silent_success() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let http = reqwest::Client::new();
        assert!(rt.block_on(handle("nonsense", json!({}), &http)).is_err());
    }

    /// A leading BOM must not make the first message unparseable.
    #[test]
    fn a_byte_order_mark_is_tolerated() {
        let raw = "\u{feff}{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
        let cleaned = raw.trim_start_matches('\u{feff}').trim();

        let parsed: Value = serde_json::from_str(cleaned).expect("should parse after stripping");
        assert_eq!(parsed.get("id"), Some(&json!(1)));

        // And without stripping it genuinely fails, so the guard is load-bearing.
        assert!(serde_json::from_str::<Value>(raw).is_err());
    }

    #[test]
    fn query_values_are_encoded_so_model_names_survive() {
        assert_eq!(urlencode("DT 990 Pro"), "DT%20990%20Pro");
        assert_eq!(
            urlencode("Beyerdynamic (250 Ohm)"),
            "Beyerdynamic%20%28250%20Ohm%29"
        );
        assert_eq!(urlencode("plain"), "plain");
    }
}
