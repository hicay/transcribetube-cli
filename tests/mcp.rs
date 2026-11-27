//! Speaks MCP to the real `transcribetube-mcp` binary and to `tube mcp`.
mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::*;
use serde_json::{json, Value};

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Server {
    fn start(binary: &str, args: &[&str], api: &MockApi, key: Option<&str>) -> Server {
        let mut cmd = Command::new(binary);
        cmd.env_clear()
            // Windows cannot open sockets without SystemRoot.
            .envs(std::env::vars().filter(|(k, _)| k.eq_ignore_ascii_case("SYSTEMROOT")))
            .env(
                "XDG_CONFIG_HOME",
                std::env::temp_dir().join("tube-mcp-test-empty"),
            )
            .env("TUBE_API_URL", &api.url)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(key) = key {
            cmd.env("TUBE_API_KEY", key);
        }
        let mut child = cmd.spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server {
            child,
            stdin,
            stdout,
        }
    }

    fn mcp(api: &MockApi) -> Server {
        Server::start(
            env!("CARGO_BIN_EXE_transcribetube-mcp"),
            &[],
            api,
            Some("test-key"),
        )
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("not JSON: {line:?}"))
    }

    fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.read()
    }

    fn tool(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.call(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn route(req: &Request) -> (u16, String) {
    // The upload to storage carries no API key; everything else must.
    if req.method != "PUT" && req.header("api-key") != Some("test-key") {
        return (401, json!({"message": "Unauthorized"}).to_string());
    }
    if let Some(response) = upload_routes(req) {
        return response;
    }
    match (req.method.as_str(), req.path.split('?').next().unwrap()) {
        ("GET", "/api/list") => ok(json!([
            {"id": "p1", "name": "First", "state": "done"},
            {"id": "p2", "name": "Second", "state": "upload"},
        ])),
        ("GET", "/api/detail/p1") => ok(done_project("p1")),
        ("GET", "/api/detail/p2") => ok(in_progress("p2", "upload")),
        ("GET", "/api/detail/notext") => ok(json!({"id": "notext", "name": "X", "text": null})),
        ("GET", "/api/detail/silent") => {
            ok(json!({"id": "silent", "name": "TEST VIDEO", "text": ""}))
        }
        ("POST", "/api/transcribeVideo") => ok(json!({"success": true, "projectId": "p1"})),
        _ => error(404, "[404] Project not found"),
    }
}

fn text_of(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text in {response}"))
}

#[test]
fn handshake_then_tool_listing() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);

    let init = s.call(1, "initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}));
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "transcribetube");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // A notification gets no answer; the next response must belong to tools/list.
    s.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let tools = s.call(2, "tools/list", json!({}));
    assert_eq!(tools["id"], 2);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "transcribe_youtube",
            "transcribe_file",
            "get_transcript",
            "list_projects"
        ]
    );
}

#[test]
fn an_unknown_protocol_version_gets_the_newest_supported_one() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let init = s.call(1, "initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn transcribe_youtube_returns_the_transcript_and_tags_the_source() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let res = s.tool(
        7,
        "transcribe_youtube",
        json!({"url": "https://youtu.be/dQw4w9WgXcQ", "language": "en"}),
    );
    assert_eq!(res["id"], 7);
    assert!(res["result"].get("isError").is_none(), "{res}");
    assert_eq!(
        text_of(&res),
        "[00:00:00] Speaker 0: Hello world.\n[00:00:05] Speaker 1: Hi there."
    );

    let post = api
        .requests()
        .into_iter()
        .find(|q| q.method == "POST")
        .unwrap();
    assert_eq!(post.header("x-tt-source"), Some("mcp"));
    assert_eq!(post.json()["language"], "en");
}

#[test]
fn get_transcript_honours_the_format() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let res = s.tool(
        1,
        "get_transcript",
        json!({"projectId": "p1", "format": "srt"}),
    );
    assert!(text_of(&res).starts_with("1\n00:00:00,000 --> 00:00:05,250\n"));
}

#[test]
fn tool_failures_are_results_marked_as_errors() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let not_ready = s.tool(1, "get_transcript", json!({"projectId": "p2"}));
    assert_eq!(not_ready["result"]["isError"], true);
    assert!(text_of(&not_ready).contains("not ready yet"));

    let missing = s.tool(2, "get_transcript", json!({"projectId": "nope"}));
    assert_eq!(missing["result"]["isError"], true);
    assert_eq!(text_of(&missing), "[404] Project not found");

    let bad_url = s.tool(
        3,
        "transcribe_youtube",
        json!({"url": "https://vimeo.com/1"}),
    );
    assert_eq!(bad_url["result"]["isError"], true);
}

#[test]
fn invalid_arguments_and_unknown_methods_are_protocol_errors() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let res = s.tool(1, "transcribe_youtube", json!({}));
    assert_eq!(res["error"]["code"], -32602);
    let res = s.tool(2, "list_projects", json!({"limit": 1000}));
    assert_eq!(res["error"]["code"], -32602);
    let res = s.tool(3, "no_such_tool", json!({}));
    assert_eq!(res["error"]["code"], -32602);
    let res = s.call(4, "resources/list", json!({}));
    assert_eq!(res["error"]["code"], -32601);
    assert!(api.requests().is_empty());
}

#[test]
fn a_broken_line_does_not_stop_the_server() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    writeln!(s.stdin.as_mut().unwrap(), "{{not json").unwrap();
    let res = s.read();
    assert_eq!(res["error"]["code"], -32700);
    assert_eq!(res["id"], Value::Null);
    assert_eq!(s.call(1, "ping", json!({}))["result"], json!({}));
}

#[test]
fn list_projects_applies_the_limit() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let res = s.tool(1, "list_projects", json!({"limit": 1}));
    let rows: Value = serde_json::from_str(text_of(&res)).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
}

#[test]
fn a_missing_api_key_surfaces_as_a_tool_error() {
    let api = MockApi::start(route);
    let mut s = Server::start(env!("CARGO_BIN_EXE_transcribetube-mcp"), &[], &api, None);
    let res = s.tool(1, "list_projects", json!({}));
    assert_eq!(res["result"]["isError"], true);
    assert!(text_of(&res).contains("TUBE_API_KEY"));
}

#[test]
fn the_server_answers_pings_while_a_transcription_is_running() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polls);
    let api = MockApi::start(move |req| {
        if req.method == "POST" {
            return ok(json!({"success": true, "projectId": "p1"}));
        }
        // First poll says "still working", so the tool call sleeps ~2 s.
        match counter.fetch_add(1, Ordering::SeqCst) {
            0 => ok(in_progress("p1", "transcription")),
            _ => ok(done_project("p1")),
        }
    });
    let mut s = Server::mcp(&api);
    s.send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "transcribe_youtube", "arguments": {"url": "dQw4w9WgXcQ"}}}));
    s.send(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));

    let first = s.read();
    assert_eq!(
        first["id"], 2,
        "the ping must not wait for the transcription"
    );
    let second = s.read();
    assert_eq!(second["id"], 1);
    assert!(text_of(&second).contains("Hello world."));
}

#[test]
fn closing_stdin_ends_the_server_after_the_running_call_answers() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    s.send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_projects", "arguments": {}}}));
    // Close stdin right away; the answer must still arrive and the process exit.
    s.stdin = None;
    let res = s.read();
    assert_eq!(res["id"], 1);
    assert!(s.child.wait().unwrap().success());
}

#[test]
fn tube_mcp_is_the_same_server() {
    let api = MockApi::start(route);
    let mut s = Server::start(env!("CARGO_BIN_EXE_tube"), &["mcp"], &api, Some("test-key"));
    let tools = s.call(1, "tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 4);
    let res = s.tool(2, "list_projects", json!({}));
    assert!(text_of(&res).contains("\"p1\""));
    assert_eq!(api.requests()[0].header("x-tt-source"), Some("mcp"));
}

#[test]
fn a_video_without_speech_is_explained_not_returned_empty() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let res = s.tool(
        1,
        "get_transcript",
        json!({"projectId": "silent", "format": "srt"}),
    );
    assert!(res["result"].get("isError").is_none());
    assert!(text_of(&res).contains("no speech was detected"));
}

#[test]
fn a_done_project_without_a_transcript_is_a_tool_error() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let res = s.tool(1, "get_transcript", json!({"projectId": "notext"}));
    assert_eq!(res["result"]["isError"], true);
    assert!(text_of(&res).contains("returned no transcript"));
}

#[test]
fn transcribe_file_uploads_the_file_and_returns_the_transcript() {
    let api = MockApi::start(route);
    let mut s = Server::mcp(&api);
    let bytes = media_bytes();
    let file = media_file("Interview 1.wav", &bytes);
    let res = s.tool(
        5,
        "transcribe_file",
        json!({"path": file.to_str().unwrap(), "format": "srt"}),
    );
    assert_eq!(res["id"], 5);
    assert!(res["result"].get("isError").is_none(), "{res}");
    assert!(text_of(&res).starts_with("1\n00:00:00,000 --> 00:00:05,250\n"));

    let seen = api.requests();
    let key = format!("{UPLOAD_PREFIX}Interview_1.wav");
    assert_eq!(
        seen[0].json(),
        json!({"fileName": "Interview_1.wav", "fileType": "audio/wav"})
    );
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(seen[1].header("content-type"), Some("audio/wav"));
    assert!(
        seen[1].raw == bytes,
        "the upload must be byte for byte the file"
    );
    assert_eq!(seen[2].path, "/api/transcribeFile");
    assert_eq!(seen[2].header("x-tt-source"), Some("mcp"));
    assert_eq!(
        seen[2].json(),
        json!({
            "name": "Interview 1.wav",
            "fileUrl": format!("https://videoven-data.s3.amazonaws.com/{key}::{key}"),
        })
    );
}

#[test]
fn transcribe_file_failures_are_tool_errors() {
    let api = MockApi::start(|req| {
        if req.method == "PUT" {
            return s3_denied();
        }
        route(req)
    });
    let mut s = Server::mcp(&api);

    let missing = temp_dir().join("nope.mp3");
    let res = s.tool(
        1,
        "transcribe_file",
        json!({"path": missing.to_str().unwrap()}),
    );
    assert_eq!(res["result"]["isError"], true);
    assert!(text_of(&res).starts_with("No such file"), "{res}");
    assert!(api.requests().is_empty());

    let file = media_file("talk.mp3", b"abc");
    let res = s.tool(
        2,
        "transcribe_file",
        json!({"path": file.to_str().unwrap()}),
    );
    assert_eq!(res["result"]["isError"], true);
    assert!(text_of(&res).contains("rejected with status 403"), "{res}");
    assert!(api
        .requests()
        .iter()
        .all(|q| q.path != "/api/transcribeFile"));

    // A call without a path is a protocol error, like any bad argument.
    let res = s.tool(3, "transcribe_file", json!({"format": "srt"}));
    assert_eq!(res["error"]["code"], -32602);
}
