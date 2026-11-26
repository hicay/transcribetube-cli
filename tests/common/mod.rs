//! A tiny HTTP server that stands in for the TranscribeTube API in tests.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
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
