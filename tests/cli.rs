//! Runs the real `tube` binary against the mock API.
mod common;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::*;
use serde_json::{json, Value};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// A private config directory per test, so tests never see a real login.
fn config_home() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tube-test-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn finish(output: Output) -> Run {
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn tube(home: &PathBuf, api: Option<&MockApi>, env: &[(&str, &str)], args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tube"));
    cmd.env_clear()
        .env("XDG_CONFIG_HOME", home)
        .env(
            "TUBE_API_URL",
            api.map(|a| a.url.as_str()).unwrap_or("http://127.0.0.1:1"),
        )
        .envs(env.iter().copied())
        .args(args)
        .stdin(Stdio::null());
    cmd
}

fn run(api: &MockApi, env: &[(&str, &str)], args: &[&str]) -> Run {
    finish(tube(&config_home(), Some(api), env, args).output().unwrap())
}

const KEY: [(&str, &str); 1] = [("TUBE_API_KEY", "test-key")];

fn route(req: &Request) -> (u16, String) {
    match (req.method.as_str(), req.path.split('?').next().unwrap()) {
        ("GET", "/api/list") if req.header("api-key") == Some("test-key") => ok(json!([
            {"id": "p1", "name": "First", "youtubeId": "dQw4w9WgXcQ", "state": "done", "createdAt": "2026-01-02T00:00:00.000Z"},
            {"id": "p2", "name": "Second", "youtubeId": "aaaaaaaaaaa", "state": "upload", "createdAt": "2026-01-01T00:00:00.000Z"},
            {"id": "p3", "state": "error", "createdAt": "2025-12-31T00:00:00.000Z"},
        ])),
        ("GET", "/api/detail/p1") => ok(done_project("p1")),
        ("GET", "/api/detail/p2") => ok(in_progress("p2", "upload")),
        ("GET", "/api/detail/p3") => ok(failed_project("p3")),
        ("GET", p) if p.starts_with("/api/detail/") => error(404, "[404] Project not found"),
        ("POST", "/api/transcribeVideo") => ok(json!({"success": true, "projectId": "p1"})),
        ("GET", "/account/me") if req.header("authorization") == Some("Bearer jwt") => {
            ok(json!({"id": "u1", "email": "owner@example.com"}))
        }
        ("GET", "/account/get-current-plan") => {
            ok(json!({"currentPlan": "Pro", "availableCredit": 300, "totalCredit": 600}))
        }
        ("GET", "/account/api-key") => ok(json!({"apiKey": "stored-key"})),
        ("POST", "/account/api-key") => ok(json!({"apiKey": "rotated-key"})),
        _ => (401, json!({"message": "Unauthorized"}).to_string()),
    }
}

#[test]
fn transcribe_without_wait_prints_only_the_project_id() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["transcribe", "https://youtu.be/dQw4w9WgXcQ"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "p1\n");
    assert!(r.stderr.contains("tube project watch p1"));
}

#[test]
fn transcribe_with_wait_prints_the_transcript() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["transcribe", "dQw4w9WgXcQ", "--wait"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout,
        "[00:00:00] Speaker 0: Hello world.\n[00:00:05] Speaker 1: Hi there.\n"
    );
    assert_eq!(r.stderr, "state: done\n");
}

#[test]
fn transcribe_with_wait_can_write_srt() {
    let api = MockApi::start(route);
    let r = run(
        &api,
        &KEY,
        &["transcribe", "dQw4w9WgXcQ", "-w", "-f", "srt"],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r
        .stdout
        .starts_with("1\n00:00:00,000 --> 00:00:05,250\nSpeaker 0: Hello world.\n\n2\n"));
}

#[test]
fn json_mode_keeps_stderr_silent_and_stdout_parseable() {
    let api = MockApi::start(route);
    let r = run(
        &api,
        &KEY,
        &["--json", "transcribe", "dQw4w9WgXcQ", "--wait"],
    );
    assert_eq!(r.code, 0);
    assert_eq!(r.stderr, "");
    let parsed: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(parsed["id"], "p1");
    assert_eq!(parsed["text"], TRANSCRIPT);

    let r = run(&api, &KEY, &["transcribe", "dQw4w9WgXcQ", "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&r.stdout).unwrap(),
        json!({"id": "p1"})
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn a_bad_url_exits_1_without_calling_the_api() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["transcribe", "https://vimeo.com/1"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout, "");
    assert!(r
        .stderr
        .starts_with("error: Not a recognizable YouTube URL"));
    assert!(api.requests().is_empty());
}

#[test]
fn transcript_supports_every_format_and_the_no_flags() {
    let api = MockApi::start(route);
    let txt = run(
        &api,
        &KEY,
        &["transcript", "p1", "--no-speakers", "--no-timecodes"],
    );
    assert_eq!(txt.stdout, "Hello world.\nHi there.\n");
    let vtt = run(&api, &KEY, &["transcript", "p1", "--format", "vtt"]);
    assert!(vtt
        .stdout
        .starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:05.250\n<v Speaker 0>Hello world."));
    let md = run(&api, &KEY, &["transcript", "p1", "-f", "md"]);
    assert!(md.stdout.starts_with("# Test Video\n"));
    let js = run(&api, &KEY, &["transcript", "p1", "-f", "json"]);
    let parsed: Value = serde_json::from_str(&js.stdout).unwrap();
    assert_eq!(parsed["transcription"].as_array().unwrap().len(), 2);
}

#[test]
fn transcript_of_an_unfinished_project_is_an_error_not_an_empty_file() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["transcript", "p2"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout, "");
    assert!(
        r.stderr.contains("not ready yet (state: upload)"),
        "{}",
        r.stderr
    );
}

#[test]
fn exit_codes_distinguish_the_failures() {
    let api = MockApi::start(route);
    // 5: not found
    let r = run(&api, &KEY, &["transcript", "missing"]);
    assert_eq!(
        (r.code, r.stderr.as_str()),
        (5, "error: [404] Project not found\n")
    );
    // 4: the transcription itself failed
    let r = run(&api, &KEY, &["project", "watch", "p3"]);
    assert_eq!(r.code, 4);
    assert!(r
        .stderr
        .contains("Transcription failed: Download: video unavailable"));
    let r = run(&api, &KEY, &["transcript", "p3"]);
    assert_eq!(r.code, 4);
    // 2: bad credential
    let r = run(&api, &[("TUBE_API_KEY", "wrong")], &["project", "list"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("tube login"));
}

#[test]
fn insufficient_credit_exits_3() {
    let api = MockApi::start(|_| error(402, "[402] Not enough credit"));
    let r = run(&api, &KEY, &["transcribe", "dQw4w9WgXcQ"]);
    assert_eq!(
        (r.code, r.stderr.as_str()),
        (3, "error: [402] Not enough credit\n")
    );
}

#[test]
fn project_list_prints_tab_separated_rows_and_honours_limit() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["project", "list"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout,
        "p1\tdone\t2026-01-02T00:00:00.000Z\tFirst\n\
         p2\tupload\t2026-01-01T00:00:00.000Z\tSecond\n\
         p3\terror\t2025-12-31T00:00:00.000Z\t\n"
    );
    let r = run(&api, &KEY, &["project", "list", "--limit", "1", "--json"]);
    let parsed: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 1);
    assert_eq!(parsed[0]["id"], "p1");
}

#[test]
fn project_get_prints_the_api_response_as_is() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["project", "get", "p2"]);
    assert_eq!(
        serde_json::from_str::<Value>(&r.stdout).unwrap(),
        in_progress("p2", "upload")
    );
}

#[test]
fn project_watch_reports_done() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["project", "watch", "p1"]);
    assert_eq!(
        (r.code, r.stdout.as_str(), r.stderr.as_str()),
        (0, "done p1\n", "state: done\n")
    );
}

#[test]
fn the_api_key_flag_beats_the_environment() {
    let api = MockApi::start(route);
    let r = run(
        &api,
        &[("TUBE_API_KEY", "wrong")],
        &["--api-key", "test-key", "project", "list"],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(api.requests()[0].header("api-key"), Some("test-key"));
}

#[test]
fn whoami_with_an_api_key_proves_the_key_works() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["whoami"]);
    assert_eq!(
        (r.code, r.stdout.as_str()),
        (0, "API key works (3 projects)\n")
    );
    // The account routes reject API keys, so they must not be called with one.
    assert!(api
        .requests()
        .iter()
        .all(|q| !q.path.starts_with("/account")));
}

#[test]
fn whoami_with_a_token_shows_account_and_credit() {
    let api = MockApi::start(route);
    let r = run(&api, &[("TUBE_TOKEN", "jwt")], &["whoami"]);
    assert_eq!(
        (r.code, r.stdout.as_str()),
        (0, "owner@example.com\ncredit: 300\n")
    );
    let r = run(
        &api,
        &[("TUBE_TOKEN", "jwt"), ("TUBE_API_KEY", "test-key")],
        &["whoami", "--json"],
    );
    let parsed: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(parsed["email"], "owner@example.com");
    assert_eq!(parsed["plan"]["availableCredit"], 300);
    assert_eq!(parsed["apiKey"], json!({"valid": true, "projects": 3}));
}

#[test]
fn whoami_without_credentials_exits_2_without_a_request() {
    let api = MockApi::start(route);
    let r = run(&api, &[], &["whoami"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Not logged in"));
    assert!(api.requests().is_empty());
}

#[test]
fn account_api_key_needs_a_token_and_prints_only_the_key() {
    let api = MockApi::start(route);
    let r = run(&api, &KEY, &["account", "api-key"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("session token"));
    assert!(api.requests().is_empty());

    let r = run(&api, &[("TUBE_TOKEN", "jwt")], &["account", "api-key"]);
    assert_eq!(
        (r.code, r.stdout.as_str(), r.stderr.as_str()),
        (0, "stored-key\n", "")
    );
    let r = run(
        &api,
        &[("TUBE_TOKEN", "jwt")],
        &["account", "api-key", "--create"],
    );
    assert_eq!(r.stdout, "rotated-key\n");
}

#[test]
fn login_stores_a_verified_key_owner_only_and_logout_removes_it() {
    let api = MockApi::start(route);
    let home = config_home();
    let r = finish(
        tube(&home, Some(&api), &[], &["login", "--api-key", "test-key"])
            .output()
            .unwrap(),
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("Logged in. API key works."));

    let path = home.join("tube").join("config.json");
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(stored, json!({"apiKey": "test-key"}));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    // The stored key is used when nothing else is given.
    let r = finish(
        tube(&home, Some(&api), &[], &["project", "list", "--limit", "1"])
            .output()
            .unwrap(),
    );
    assert_eq!(r.code, 0, "{}", r.stderr);

    let r = finish(tube(&home, Some(&api), &[], &["logout"]).output().unwrap());
    assert_eq!(r.code, 0);
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(stored, json!({}));
}

#[test]
fn login_with_a_wrong_key_stores_nothing_and_keeps_the_old_one() {
    let api = MockApi::start(route);
    let home = config_home();
    let r = finish(
        tube(&home, Some(&api), &[], &["login", "--api-key", "wrong"])
            .output()
            .unwrap(),
    );
    assert_eq!(r.code, 2);
    assert!(!home.join("tube").join("config.json").exists());

    finish(
        tube(&home, Some(&api), &[], &["login", "--api-key", "test-key"])
            .output()
            .unwrap(),
    );
    let r = finish(
        tube(&home, Some(&api), &[], &["login", "--api-key", "wrong"])
            .output()
            .unwrap(),
    );
    assert_eq!(r.code, 2);
    let stored = std::fs::read_to_string(home.join("tube").join("config.json")).unwrap();
    assert!(
        stored.contains("test-key"),
        "a failed login must not wipe a working key"
    );
}

#[test]
fn login_reads_a_pasted_key_from_a_pipe() {
    let api = MockApi::start(route);
    let home = config_home();
    let mut child = tube(&home, Some(&api), &[], &["login"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"  test-key  \n")
        .unwrap();
    let r = finish(child.wait_with_output().unwrap());
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("/settings/api-access"));
    let stored = std::fs::read_to_string(home.join("tube").join("config.json")).unwrap();
    assert!(stored.contains("\"apiKey\": \"test-key\""));
}

#[test]
fn login_with_nothing_pasted_fails() {
    let api = MockApi::start(route);
    let r = run(&api, &[], &["login"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("No API key entered."));
}

#[test]
fn a_corrupt_config_file_is_ignored() {
    let api = MockApi::start(route);
    let home = config_home();
    std::fs::create_dir_all(home.join("tube")).unwrap();
    std::fs::write(home.join("tube").join("config.json"), "{not json").unwrap();
    let r = finish(
        tube(&home, Some(&api), &KEY, &["project", "list"])
            .output()
            .unwrap(),
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
}

#[test]
fn an_unreachable_api_exits_1_with_a_plain_message() {
    let r = finish(
        tube(&config_home(), None, &KEY, &["project", "list"])
            .output()
            .unwrap(),
    );
    assert_eq!(r.code, 1);
    assert!(
        r.stderr
            .starts_with("error: Could not reach http://127.0.0.1:1"),
        "{}",
        r.stderr
    );
}

#[test]
fn version_and_help_work_without_credentials() {
    let r = finish(
        tube(&config_home(), None, &[], &["--version"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        r.stdout.trim(),
        format!("tube {}", env!("CARGO_PKG_VERSION"))
    );
    let r = finish(
        tube(&config_home(), None, &[], &["--help"])
            .output()
            .unwrap(),
    );
    for command in [
        "login",
        "logout",
        "whoami",
        "account",
        "transcribe",
        "project",
        "transcript",
        "mcp",
    ] {
        assert!(r.stdout.contains(command), "help is missing {command}");
    }
}

#[test]
fn waiting_shows_each_state_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let api = MockApi::start(move |req| {
        if req.method == "POST" {
            return ok(json!({"success": true, "projectId": "p9"}));
        }
        match counter.fetch_add(1, Ordering::SeqCst) {
            0 => ok(in_progress("p9", "upload")),
            _ => ok(done_project("p9")),
        }
    });
    let r = run(&api, &KEY, &["transcribe", "dQw4w9WgXcQ", "--wait"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stderr, "state: upload\nstate: done\n");
    assert!(api
        .requests()
        .iter()
        .any(|q| q.path.starts_with("/api/detail/p9")));
}
