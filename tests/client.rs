mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use serde_json::json;
use transcribetube_cli::api::Client;
use transcribetube_cli::config::Config;
use transcribetube_cli::transcribe::{start_file, start_youtube, wait_for_project, WaitOptions};

fn client(api: &MockApi, api_key: Option<&str>, token: Option<&str>) -> Client {
    Client::new(
        Config {
            api_url: api.url.clone(),
            api_key: api_key.map(str::to_string),
            token: token.map(str::to_string),
            ..Config::default()
        },
        "cli",
    )
}

fn fast() -> WaitOptions<'static> {
    WaitOptions {
        timeout: Duration::from_secs(5),
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(10),
        on_state: None,
    }
}

#[test]
fn unwraps_the_result_envelope() {
    let api = MockApi::start(|_| ok(json!({"email": "a@b.com", "id": "123"})));
    let me = client(&api, None, Some("jwt")).me().unwrap();
    assert_eq!(me, json!({"email": "a@b.com", "id": "123"}));
}

#[test]
fn returns_the_raw_body_when_there_is_no_envelope() {
    let api = MockApi::start(|_| (200, json!({"email": "a@b.com", "id": "123"}).to_string()));
    let me = client(&api, None, Some("jwt")).me().unwrap();
    assert_eq!(me["email"], "a@b.com");
}

#[test]
fn an_api_error_carries_status_and_message() {
    let api = MockApi::start(|_| error(404, "[404] Project not found"));
    let err = client(&api, Some("k"), None)
        .get_project("nope")
        .unwrap_err();
    assert_eq!(err.status, Some(404));
    assert_eq!(err.message, "[404] Project not found");
}

#[test]
fn a_gateway_401_tells_the_user_what_to_do() {
    let api = MockApi::start(|_| (401, json!({"message": "Unauthorized"}).to_string()));
    let err = client(&api, Some("bad"), None)
        .list_projects(1)
        .unwrap_err();
    assert_eq!(err.status, Some(401));
    assert!(err.message.contains("tube login"), "{}", err.message);
}

#[test]
fn a_non_json_error_body_falls_back_to_a_generic_message() {
    let api = MockApi::start(|_| (502, "<html>Bad Gateway</html>".to_string()));
    let err = client(&api, Some("k"), None).list_projects(1).unwrap_err();
    assert_eq!(err.status, Some(502));
    assert_eq!(err.message, "Request failed with status 502.");
}

#[test]
fn an_unreachable_api_is_a_local_error() {
    let unreachable = Client::new(
        Config {
            api_url: "http://127.0.0.1:1".into(),
            api_key: Some("k".into()),
            ..Config::default()
        },
        "cli",
    );
    let err = unreachable.list_projects(1).unwrap_err();
    assert_eq!(err.status, None);
    assert!(
        err.message.starts_with("Could not reach"),
        "{}",
        err.message
    );
}

#[test]
fn sends_each_credential_in_its_own_header() {
    let api = MockApi::start(|_| ok(json!([])));
    client(&api, Some("test-key"), None)
        .list_projects(1)
        .unwrap();
    client(&api, None, Some("jwt")).list_projects(1).unwrap();
    client(&api, Some("test-key"), Some("jwt"))
        .list_projects(1)
        .unwrap();
    let seen = api.requests();

    assert_eq!(seen[0].header("api-key"), Some("test-key"));
    assert_eq!(seen[0].header("authorization"), None);
    assert_eq!(seen[1].header("api-key"), None);
    assert_eq!(seen[1].header("authorization"), Some("Bearer jwt"));
    assert_eq!(seen[2].header("api-key"), Some("test-key"));
    assert_eq!(seen[2].header("authorization"), Some("Bearer jwt"));
}

#[test]
fn identifies_itself_to_the_api() {
    let api = MockApi::start(|_| ok(json!([])));
    client(&api, Some("k"), None).list_projects(1).unwrap();
    let seen = &api.requests()[0];
    assert_eq!(seen.header("x-tt-source"), Some("cli"));
    assert!(seen.header("user-agent").unwrap().starts_with("tube/"));
    // A GET has no body, so it must not claim a JSON one.
    assert_eq!(seen.header("content-type"), None);
}

#[test]
fn list_asks_for_the_page_only_past_the_first() {
    let api = MockApi::start(|_| ok(json!([{"id": "p1", "state": "done"}])));
    let c = client(&api, Some("k"), None);
    assert_eq!(c.list_projects(1).unwrap().len(), 1);
    c.list_projects(3).unwrap();
    let seen = api.requests();
    assert_eq!(seen[0].path, "/api/list");
    assert_eq!(seen[1].path, "/api/list?page=3");
}

#[test]
fn detail_asks_for_timecodes_and_speakers_and_encodes_the_id() {
    let api = MockApi::start(|_| ok(done_project("p1")));
    client(&api, Some("k"), None).get_project("a b/c").unwrap();
    assert_eq!(
        api.requests()[0].path,
        "/api/detail/a%20b%2Fc?showTimecode=true&showSpeaker=true"
    );
}

#[test]
fn transcribe_sends_the_fields_the_api_requires() {
    let api = MockApi::start(|_| ok(json!({"success": true, "projectId": "proj1"})));
    let c = client(&api, Some("k"), None);
    let id = c.transcribe_youtube("dQw4w9WgXcQ", None, None).unwrap();
    assert_eq!(id, "proj1");

    let sent = &api.requests()[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, "/api/transcribeVideo");
    assert_eq!(sent.header("content-type"), Some("application/json"));
    let body = sent.json();
    assert_eq!(body["youtubeVideoId"], "dQw4w9WgXcQ");
    // The API rejects a request without webhookUrl; empty means none.
    assert_eq!(body["webhookUrl"], "");
    assert!(body.get("youtubeId").is_none());
    assert!(body.get("language").is_none());
}

#[test]
fn transcribe_passes_language_and_webhook_through() {
    let api = MockApi::start(|_| ok(json!({"success": true, "projectId": "proj1"})));
    let c = client(&api, Some("k"), None);
    start_youtube(
        &c,
        "https://youtu.be/dQw4w9WgXcQ",
        Some("nl"),
        Some("https://hook.example/x"),
    )
    .unwrap();
    start_youtube(&c, "dQw4w9WgXcQ", Some("auto"), None).unwrap();
    let seen = api.requests();
    assert_eq!(seen[0].json()["language"], "nl");
    assert_eq!(seen[0].json()["webhookUrl"], "https://hook.example/x");
    assert!(
        seen[1].json().get("language").is_none(),
        "auto means: let the API detect"
    );
}

#[test]
fn a_bad_url_never_reaches_the_api() {
    let api = MockApi::start(|_| ok(json!({})));
    let err = start_youtube(
        &client(&api, Some("k"), None),
        "https://vimeo.com/1",
        None,
        None,
    )
    .unwrap_err();
    assert!(err.message.contains("Not a recognizable YouTube URL"));
    assert!(api.requests().is_empty());
}

#[test]
fn a_402_is_reported_as_insufficient_credit() {
    let api = MockApi::start(|_| (402, String::new()));
    let err = client(&api, Some("k"), None)
        .transcribe_youtube("dQw4w9WgXcQ", None, None)
        .unwrap_err();
    assert_eq!(err.status, Some(402));
    assert_eq!(err.message, "Insufficient credit for this transcription.");
}

#[test]
fn creating_an_api_key_posts_without_a_json_body() {
    let api = MockApi::start(|_| ok(json!({"apiKey": "new-key"})));
    let key = client(&api, None, Some("jwt")).create_api_key().unwrap();
    assert_eq!(key.as_deref(), Some("new-key"));
    let sent = &api.requests()[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.header("content-type"), None);
    assert_eq!(sent.header("content-length"), Some("0"));
}

#[test]
fn a_missing_api_key_is_none_not_an_error() {
    let api = MockApi::start(|_| ok(json!({"apiKey": null})));
    assert_eq!(client(&api, None, Some("jwt")).get_api_key().unwrap(), None);
}

#[test]
fn waiting_polls_through_every_state_until_done() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let api = MockApi::start(move |_| match counter.fetch_add(1, Ordering::SeqCst) {
        0 | 1 => ok(in_progress("p1", "upload")),
        2 => ok(in_progress("p1", "transcription")),
        _ => ok(done_project("p1")),
    });
    let mut states = Vec::new();
    let mut record = |s: &str| states.push(s.to_string());
    let project = wait_for_project(
        &client(&api, Some("k"), None),
        "p1",
        WaitOptions {
            on_state: Some(&mut record),
            ..fast()
        },
    )
    .unwrap();

    assert_eq!(project["text"], TRANSCRIPT);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    // One callback per change, not per poll.
    assert_eq!(states, ["upload", "transcription", "done"]);
}

#[test]
fn waiting_returns_at_once_for_a_project_that_is_already_done() {
    let api = MockApi::start(|_| ok(done_project("p1")));
    wait_for_project(&client(&api, Some("k"), None), "p1", fast()).unwrap();
    assert_eq!(api.requests().len(), 1);
}

#[test]
fn a_failed_project_stops_the_wait_with_its_reason() {
    let api = MockApi::start(|_| ok(failed_project("p1")));
    let err = wait_for_project(&client(&api, Some("k"), None), "p1", fast()).unwrap_err();
    assert_eq!(err.status, Some(422));
    assert_eq!(
        err.message,
        "Transcription failed: Download: video unavailable"
    );
}

#[test]
fn waiting_gives_up_after_the_timeout() {
    let api = MockApi::start(|_| ok(in_progress("p1", "upload")));
    let err = wait_for_project(
        &client(&api, Some("k"), None),
        "p1",
        WaitOptions {
            timeout: Duration::from_millis(60),
            ..fast()
        },
    )
    .unwrap_err();
    assert_eq!(err.status, Some(408));
    assert!(err.message.contains("tube project watch p1"));
    assert!(api.requests().len() >= 2);
}

#[test]
fn an_api_error_during_the_wait_is_passed_on() {
    let api = MockApi::start(|_| error(404, "[404] Project not found"));
    let err = wait_for_project(&client(&api, Some("k"), None), "gone", fast()).unwrap_err();
    assert_eq!(err.status, Some(404));
}

/// The upload flow, with one route swapped out.
fn upload_api_with(
    method: &'static str,
    prefix: &'static str,
    response: fn() -> (u16, String),
) -> MockApi {
    MockApi::start(move |req| {
        if req.method == method && req.path.starts_with(prefix) {
            return response();
        }
        upload_routes(req).unwrap_or_else(|| error(404, "no such route"))
    })
}

fn upload_api() -> MockApi {
    MockApi::start(|req| upload_routes(req).unwrap_or_else(|| error(404, "no such route")))
}

#[test]
fn a_file_is_transcribed_in_three_requests_with_the_fields_the_api_requires() {
    let api = upload_api();
    let bytes = media_bytes();
    let file = media_file("Board meeting #3.m4a", &bytes);
    let mut announced = Vec::new();
    let mut record = |size: u64| announced.push(size);

    let id = start_file(
        &client(&api, Some("test-key"), None),
        file.to_str().unwrap(),
        None,
        Some(&mut record),
    )
    .unwrap();
    assert_eq!(id, "p1");
    assert_eq!(announced, [bytes.len() as u64]);

    let seen = api.requests();
    assert_eq!(seen.len(), 3);
    let key = format!("{UPLOAD_PREFIX}Board_meeting__3.m4a");

    // 1. Ask where to upload. The name becomes part of the storage key, so
    //    only plain characters are sent.
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path.as_str()),
        ("POST", "/api/upload-url")
    );
    assert_eq!(seen[0].header("api-key"), Some("test-key"));
    assert_eq!(
        seen[0].json(),
        json!({"fileName": "Board_meeting__3.m4a", "fileType": "audio/mp4"})
    );

    // 2. PUT the bytes to the signed URL, exactly as given, with the same
    //    content type and without our credentials.
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(
        seen[1].path,
        format!("/s3/{key}?X-Amz-Signature=abc&X-Amz-Expires=3600")
    );
    assert_eq!(seen[1].header("content-type"), Some("audio/mp4"));
    assert_eq!(seen[1].header("content-length"), Some("200000"));
    assert_eq!(seen[1].header("transfer-encoding"), None);
    assert_eq!(seen[1].header("api-key"), None);
    assert_eq!(seen[1].header("authorization"), None);
    assert!(
        seen[1].raw == bytes,
        "the upload must be byte for byte the file"
    );

    // 3. Register the upload. The project keeps the real file name.
    assert_eq!(
        (seen[2].method.as_str(), seen[2].path.as_str()),
        ("POST", "/api/transcribeFile")
    );
    assert_eq!(seen[2].header("api-key"), Some("test-key"));
    assert_eq!(seen[2].header("x-tt-source"), Some("cli"));
    assert_eq!(
        seen[2].json(),
        json!({
            "name": "Board meeting #3.m4a",
            "fileUrl": format!("https://videoven-data.s3.amazonaws.com/{key}::{key}"),
        })
    );
}

#[test]
fn a_file_transcription_passes_the_language_through() {
    let api = upload_api();
    let file = media_file("talk.mp3", b"ID3 not really audio");
    let c = client(&api, Some("k"), None);
    start_file(&c, file.to_str().unwrap(), Some("nl"), None).unwrap();
    start_file(&c, file.to_str().unwrap(), Some("auto"), None).unwrap();
    let posts: Vec<_> = api
        .requests()
        .into_iter()
        .filter(|q| q.path == "/api/transcribeFile")
        .collect();
    assert_eq!(posts[0].json()["language"], "nl");
    assert!(posts[1].json().get("language").is_none());
}

#[test]
fn a_missing_file_a_directory_and_an_empty_file_never_reach_the_api() {
    let api = upload_api();
    let c = client(&api, Some("k"), None);
    let dir = temp_dir();

    let missing = dir.join("nope.mp3");
    let err = start_file(&c, missing.to_str().unwrap(), None, None).unwrap_err();
    assert!(err.message.starts_with("No such file"), "{}", err.message);
    assert_eq!(err.status, None);

    let err = start_file(&c, dir.to_str().unwrap(), None, None).unwrap_err();
    assert!(err.message.contains("is a directory"), "{}", err.message);

    let empty = media_file("empty.wav", b"");
    let err = start_file(&c, empty.to_str().unwrap(), None, None).unwrap_err();
    assert!(err.message.contains("empty file"), "{}", err.message);

    assert!(api.requests().is_empty());
}

#[test]
fn a_rejected_upload_stops_before_a_project_is_created() {
    let api = upload_api_with("PUT", "/s3/", s3_denied);
    let file = media_file("talk.mp3", &media_bytes());
    let err = start_file(
        &client(&api, Some("k"), None),
        file.to_str().unwrap(),
        None,
        None,
    )
    .unwrap_err();
    // Not an API status: a 403 from storage is not a bad API key.
    assert_eq!(err.status, None);
    assert_eq!(
        err.message,
        "The upload was rejected with status 403 (AccessDenied). Nothing was transcribed."
    );
    let seen = api.requests();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|q| q.path != "/api/transcribeFile"));
}

#[test]
fn an_upload_that_cannot_connect_does_not_print_the_signed_url() {
    let api = MockApi::start(|req| match req.path.as_str() {
        "/api/upload-url" => ok(json!({
            "url": "http://127.0.0.1:1/s3/k?X-Amz-Signature=secret-signature",
            "key": "raw/usr/u1/x/talk.mp3",
        })),
        _ => error(404, "no such route"),
    });
    let file = media_file("talk.mp3", b"abc");
    let err = start_file(
        &client(&api, Some("k"), None),
        file.to_str().unwrap(),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        err.message.starts_with("The upload failed"),
        "{}",
        err.message
    );
    assert!(!err.message.contains("secret-signature"), "{}", err.message);
    assert_eq!(api.requests().len(), 1);
}

#[test]
fn an_api_error_on_the_upload_url_is_passed_on_before_anything_is_uploaded() {
    let api = upload_api_with("POST", "/api/upload-url", || (402, String::new()));
    let file = media_file("talk.mp3", b"abc");
    let err = start_file(
        &client(&api, Some("k"), None),
        file.to_str().unwrap(),
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err.status, Some(402));
    assert_eq!(api.requests().len(), 1);
}

#[test]
fn not_enough_credit_when_registering_the_upload_is_a_402() {
    let api = upload_api_with("POST", "/api/transcribeFile", || {
        error(402, "Not enough credit for user: u1.")
    });
    let file = media_file("talk.mp3", b"abc");
    let err = start_file(
        &client(&api, Some("k"), None),
        file.to_str().unwrap(),
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err.status, Some(402));
    assert_eq!(err.message, "Not enough credit for user: u1.");
}

#[test]
fn an_upload_url_response_without_a_url_is_an_error() {
    let api = upload_api_with("POST", "/api/upload-url", || ok(json!({"key": "k"})));
    let file = media_file("talk.mp3", b"abc");
    let err = start_file(
        &client(&api, Some("k"), None),
        file.to_str().unwrap(),
        None,
        None,
    )
    .unwrap_err();
    assert!(err.message.contains("did not return an upload URL"));
    assert_eq!(api.requests().len(), 1);
}
