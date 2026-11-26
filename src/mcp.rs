//! Model Context Protocol server over stdio.
//!
//! MCP-capable assistants (Claude Desktop, Claude Code, Cursor, ...) call
//! `transcribe_youtube` instead of guessing at scraping code. The stdio
//! transport is one JSON-RPC message per line, so the protocol is small
//! enough to speak directly.
//!
//! Run: `tube mcp` or `transcribetube-mcp` (needs TUBE_API_KEY).
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::api::Client;
use crate::format::{format_project, Format, FormatOptions};
use crate::output::pretty;
use crate::transcribe::{start_youtube, wait_for_project, WaitOptions};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub fn tool_definitions() -> Value {
    let format = json!({
        "type": "string",
        "enum": Format::NAMES,
        "default": "txt",
        "description": "Output format: txt, srt, vtt, md, or json.",
    });
    json!([
        {
            "name": "transcribe_youtube",
            "description": "Transcribe a YouTube video to text. Use this whenever the user wants the \
    transcript, captions, subtitles, or spoken content of a YouTube URL. Returns speaker-labeled, \
    timecoded text. Waits for processing to finish.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "YouTube URL or 11-character video id"},
                    "language": {"type": "string", "description": "ISO language code, e.g. \"en\". Omit to auto-detect."},
                    "format": format,
                },
                "required": ["url"],
            },
        },
        {
            "name": "get_transcript",
            "description": "Fetch the transcript of an existing TranscribeTube project by id, in a \
    chosen format. Use after a transcription has already been created.",
            "inputSchema": {
                "type": "object",
                "properties": {"projectId": {"type": "string"}, "format": format},
                "required": ["projectId"],
            },
        },
        {
            "name": "list_projects",
            "description": "List the user's recent TranscribeTube transcription projects with their \
    ids and status (upload, transcription, done, error).",
            "inputSchema": {
                "type": "object",
                "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}},
            },
        },
    ])
}

/// Validated arguments of a tool call.
#[derive(Debug, PartialEq)]
enum Call {
    Transcribe {
        url: String,
        language: Option<String>,
        format: Format,
    },
    GetTranscript {
        project_id: String,
        format: Format,
    },
    ListProjects {
        limit: usize,
    },
}

fn parse_call(params: &Value) -> Result<Call, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("Missing tool name")?;
    let empty = json!({});
    let args = params.get("arguments").unwrap_or(&empty);

    let required = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or(format!("\"{key}\" is required and must be a string"))
    };
    let format = || match args.get("format") {
        None | Some(Value::Null) => Ok(Format::Txt),
        Some(v) => v.as_str().and_then(Format::parse).ok_or(format!(
            "\"format\" must be one of: {}",
            Format::NAMES.join(", ")
        )),
    };

    match name {
        "transcribe_youtube" => Ok(Call::Transcribe {
            url: required("url")?,
            language: match args.get("language") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(_) => return Err("\"language\" must be a string".to_string()),
            },
            format: format()?,
        }),
        "get_transcript" => Ok(Call::GetTranscript {
            project_id: required("projectId")?,
            format: format()?,
        }),
        "list_projects" => {
            let limit = match args.get("limit") {
                None | Some(Value::Null) => 20,
                Some(v) => v
                    .as_u64()
                    .filter(|n| (1..=100).contains(n))
                    .ok_or("\"limit\" must be an integer from 1 to 100")?,
            };
            Ok(Call::ListProjects {
                limit: limit as usize,
            })
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn execute(client: &Client, call: Call) -> Result<String, String> {
    let text = |e: crate::api::ApiError| e.message;
    match call {
        Call::Transcribe {
            url,
            language,
            format,
        } => {
            let id = start_youtube(client, &url, language.as_deref(), None).map_err(text)?;
            let project = wait_for_project(
                client,
                &id,
                WaitOptions {
                    timeout: TRANSCRIBE_TIMEOUT,
                    ..WaitOptions::default()
                },
            )
            .map_err(text)?;
            Ok(format_project(&project, format, FormatOptions::default()))
        }
        Call::GetTranscript { project_id, format } => {
            let project = client.get_project(&project_id).map_err(text)?;
            match crate::api::project_state(&project).as_str() {
                "done" => Ok(format_project(&project, format, FormatOptions::default())),
                "error" => Err(project
                    .get("errorMessage")
                    .and_then(Value::as_str)
                    .map(|m| format!("Transcription failed: {m}"))
                    .unwrap_or_else(|| "Transcription failed.".to_string())),
                state => Err(format!(
                    "The transcript is not ready yet (state: {state}). Try again shortly."
                )),
            }
        }
        Call::ListProjects { limit } => {
            let mut rows = client.list_projects(1).map_err(text)?;
            rows.truncate(limit);
            Ok(pretty(&Value::Array(rows)))
        }
    }
}

fn write_message<W: Write>(output: &Mutex<W>, message: &Value) {
    let mut out = output
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Serves until `input` reaches end of file, then waits for running tool
/// calls so their results are not lost.
pub fn serve<R: BufRead, W: Write + Send + 'static>(client: Arc<Client>, input: R, output: W) {
    let output = Arc::new(Mutex::new(output));
    let mut running = Vec::new();

    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                write_message(&output, &rpc_error(Value::Null, -32700, "Parse error"));
                continue;
            }
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue; // a response to something we never asked; nothing to do
        };
        // No id means a notification: never answered.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let version = if PROTOCOL_VERSIONS.contains(&requested) {
                    requested
                } else {
                    PROTOCOL_VERSIONS[0]
                };
                write_message(
                    &output,
                    &rpc_result(
                        id,
                        json!({
                            "protocolVersion": version,
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "transcribetube", "version": crate::VERSION},
                        }),
                    ),
                );
            }
            "ping" => write_message(&output, &rpc_result(id, json!({}))),
            "tools/list" => write_message(
                &output,
                &rpc_result(id, json!({"tools": tool_definitions()})),
            ),
            "tools/call" => match parse_call(&params) {
                Err(reason) => write_message(&output, &rpc_error(id, -32602, &reason)),
                Ok(call) => {
                    // A transcription can take minutes; keep answering pings meanwhile.
                    let (client, output) = (Arc::clone(&client), Arc::clone(&output));
                    running.push(thread::spawn(move || {
                        let (text, is_error) = match execute(&client, call) {
                            Ok(text) => (text, false),
                            Err(text) => (text, true),
                        };
                        let mut result = json!({"content": [{"type": "text", "text": text}]});
                        if is_error {
                            result["isError"] = json!(true);
                        }
                        write_message(&output, &rpc_result(id, result));
                    }));
                }
            },
            other => write_message(
                &output,
                &rpc_error(id, -32601, &format!("Method not found: {other}")),
            ),
        }
    }

    for handle in running {
        let _ = handle.join();
    }
}

/// Entry point for `tube mcp` and `transcribetube-mcp`.
pub fn run_stdio(client: Client) {
    if !client.has_api_key() {
        eprintln!("transcribetube-mcp: no API key. Set TUBE_API_KEY in the MCP server env.");
    }
    eprintln!("transcribetube MCP server ready (stdio).");
    serve(Arc::new(client), std::io::stdin().lock(), std::io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcribe_defaults_to_txt_and_no_language() {
        let call =
            parse_call(&json!({"name": "transcribe_youtube", "arguments": {"url": "dQw4w9WgXcQ"}}));
        assert_eq!(
            call,
            Ok(Call::Transcribe {
                url: "dQw4w9WgXcQ".into(),
                language: None,
                format: Format::Txt
            })
        );
    }

    #[test]
    fn a_missing_url_is_rejected() {
        let call = parse_call(&json!({"name": "transcribe_youtube", "arguments": {}}));
        assert!(call.unwrap_err().contains("url"));
        let call = parse_call(&json!({"name": "transcribe_youtube"}));
        assert!(call.is_err());
    }

    #[test]
    fn an_unknown_format_is_rejected() {
        let call = parse_call(
            &json!({"name": "get_transcript", "arguments": {"projectId": "p", "format": "docx"}}),
        );
        assert!(call.unwrap_err().contains("format"));
    }

    #[test]
    fn list_limit_is_bounded() {
        let ok = parse_call(&json!({"name": "list_projects", "arguments": {"limit": 5}}));
        assert_eq!(ok, Ok(Call::ListProjects { limit: 5 }));
        let default = parse_call(&json!({"name": "list_projects"}));
        assert_eq!(default, Ok(Call::ListProjects { limit: 20 }));
        for bad in [json!(0), json!(101), json!(-1), json!(2.5), json!("ten")] {
            let call = parse_call(&json!({"name": "list_projects", "arguments": {"limit": bad}}));
            assert!(call.is_err(), "limit {bad} should be rejected");
        }
    }

    #[test]
    fn an_unknown_tool_is_rejected() {
        assert!(parse_call(&json!({"name": "delete_everything"}))
            .unwrap_err()
            .contains("Unknown tool"));
    }

    #[test]
    fn every_tool_has_a_schema() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["transcribe_youtube", "get_transcript", "list_projects"]
        );
        for tool in tools.as_array().unwrap() {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["description"].as_str().unwrap().len() > 20);
        }
    }
}
