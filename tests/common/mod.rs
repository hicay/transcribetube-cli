//! A tiny HTTP server that stands in for the TranscribeTube API in tests,
//! and for the storage that file uploads are PUT to.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Path with query string.
    pub path: String,
    /// Header names are lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// The body as received; `body` is lossy for binary uploads.
    pub raw: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("request body is JSON")
    }
}

pub struct MockApi {
    pub url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl MockApi {
    /// `handler` returns the status and the raw response body.
    pub fn start<F>(handler: F) -> MockApi
    where
        F: Fn(&Request) -> (u16, String) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let handler = Arc::new(handler);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (seen, handler) = (Arc::clone(&seen), Arc::clone(&handler));
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new();
                    if reader.read_line(&mut first).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = first.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts.next().unwrap_or("").to_string();

                    let mut headers = Vec::new();
                    let mut length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = line.trim().split_once(':') {
                            let (k, v) = (k.trim().to_lowercase(), v.trim().to_string());
                            if k == "content-length" {
                                length = v.parse().unwrap_or(0);
                            }
                            headers.push((k, v));
                        }
                    }
                    let mut body = vec![0u8; length];
                    reader.read_exact(&mut body).ok();

                    let request = Request {
                        method,
                        path,
                        headers,
                        body: String::from_utf8_lossy(&body).into_owned(),
                        raw: body,
                    };
                    let (status, payload) = handler(&request);
                    seen.lock().unwrap().push(request);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                });
            }
        });
        MockApi { url, requests }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

/// The API's response envelope.
pub fn ok(result: Value) -> (u16, String) {
    (200, json!({"path": "/x", "statusCode": 200, "timestamp": "2026-01-01T00:00:00.000Z", "result": result}).to_string())
}

/// The API's error envelope.
pub fn error(status: u16, message: &str) -> (u16, String) {
    (
        status,
        json!({"path": "/x", "statusCode": status, "timestamp": "2026-01-01T00:00:00.000Z", "error": {"message": message}})
            .to_string(),
    )
}

pub const TRANSCRIPT: &str =
    "[00:00:00.000] Speaker 0: \nHello world.\n\n[00:00:05.250] Speaker 1: \nHi there.";

/// A finished project exactly as `/api/detail/:id` returns it: no `state`.
pub fn done_project(id: &str) -> Value {
    json!({"id": id, "name": "Test Video", "youtubeId": "dQw4w9WgXcQ", "language": "en", "text": TRANSCRIPT})
}

pub fn in_progress(id: &str, state: &str) -> Value {
    json!({"id": id, "name": "Test Video", "state": state, "message": "Transcription is in progress", "videoLength": 12, "createdAt": "2026-01-01T00:00:00.000Z"})
}

pub fn failed_project(id: &str) -> Value {
    json!({"id": id, "name": "Test Video", "state": "error", "errorMessage": "Download: video unavailable", "transcription": [], "speakers": []})
}

/// The key `/api/upload-url` hands out is this plus the file name it was given.
pub const UPLOAD_PREFIX: &str = "raw/usr/u1/0b7e1f6e-5d43-4c0a-9a57-3f2f8d1c9e11/";

/// The file upload flow as the API serves it. The pre-signed URL points back
/// at this mock (under `/s3/`), which then plays the storage side too.
/// `None` for any other request, so a test's own routes can take over.
pub fn upload_routes(req: &Request) -> Option<(u16, String)> {
    match (req.method.as_str(), req.path.split('?').next().unwrap()) {
        ("POST", "/api/upload-url") => {
            let key = format!(
                "{UPLOAD_PREFIX}{}",
                req.json()["fileName"].as_str().unwrap_or("")
            );
            let host = req.header("host").expect("host header");
            Some(ok(json!({
                "url": format!("http://{host}/s3/{key}?X-Amz-Signature=abc&X-Amz-Expires=3600"),
                "key": key,
            })))
        }
        ("PUT", path) if path.starts_with("/s3/") => Some((200, String::new())),
        ("POST", "/api/transcribeFile") => Some(ok(json!({"id": "p1"}))),
        _ => None,
    }
}

/// What S3 answers to a PUT whose signature it does not accept.
pub fn s3_denied() -> (u16, String) {
    (
        403,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>AccessDenied</Code><Message>Request has expired</Message></Error>".to_string(),
    )
}

/// 200 kB that are not valid UTF-8, so a lossy or partial upload shows.
pub fn media_bytes() -> Vec<u8> {
    (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect()
}

static NEXT_MEDIA_DIR: AtomicUsize = AtomicUsize::new(0);

/// A fresh, empty directory under the system temp directory.
pub fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tube-media-{}-{}",
        std::process::id(),
        NEXT_MEDIA_DIR.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `bytes` to a new file called `name` and returns its path.
pub fn media_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = temp_dir().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}
