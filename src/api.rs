//! Thin HTTP client over the TranscribeTube REST API.
//!
//! The API has two credential types, each valid for its own routes:
//! `/api/*` accepts only the `api-key` header, `/account/*` accepts only a
//! Bearer session token. The client sends whichever it has.
use std::fmt;
use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub message: String,
    /// HTTP status, or a status-like code for local failures (422 failed
    /// transcription, 408 timed out). `None` for everything else.
    pub status: Option<u16>,
    pub body: Option<Value>,
}

impl ApiError {
    pub fn new(message: impl Into<String>, status: u16) -> Self {
        ApiError {
            message: message.into(),
            status: Some(status),
            body: None,
        }
    }

    /// A failure that did not come from an HTTP response.
    pub fn local(message: impl Into<String>) -> Self {
        ApiError {
            message: message.into(),
            status: None,
            body: None,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

/// Shown when the API reports a project as done but returns no transcript.
pub const NO_TRANSCRIPT: &str =
    "The project is marked done but the API returned no transcript for it.";

pub type Result<T> = std::result::Result<T, ApiError>;

pub struct Client {
    cfg: Config,
    agent: ureq::Agent,
    /// Sent as `x-tt-source` so the backend can tell CLI from MCP usage.
    source: &'static str,
}

impl Client {
    pub fn new(cfg: Config, source: &'static str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .user_agent(&format!("tube/{}", crate::VERSION))
            .build();
        Client { cfg, agent, source }
    }

    pub fn has_api_key(&self) -> bool {
        self.cfg.api_key.is_some()
    }

    pub fn has_token(&self) -> bool {
        self.cfg.token.is_some()
    }

    pub fn authenticated(&self) -> bool {
        self.has_api_key() || self.has_token()
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{}{}", self.cfg.api_url, path);
        let mut req = self
            .agent
            .request(method, &url)
            .set("accept", "application/json")
            .set("x-tt-source", self.source);
        if let Some(key) = &self.cfg.api_key {
            req = req.set("api-key", key);
        }
        if let Some(token) = &self.cfg.token {
            req = req.set("authorization", &format!("Bearer {token}"));
        }

        // No content-type without a body: the API rejects an empty JSON body.
        let sent = match body {
            Some(b) => req
                .set("content-type", "application/json")
                .send_bytes(b.to_string().as_bytes()),
            None if method == "POST" => req.send_bytes(&[]),
            None => req.call(),
        };

        let (status, text) = match sent {
            Ok(res) => (res.status(), res.into_string().unwrap_or_default()),
            Err(ureq::Error::Status(code, res)) => (code, res.into_string().unwrap_or_default()),
            Err(ureq::Error::Transport(t)) => {
                return Err(ApiError::local(format!(
                    "Could not reach {}: {t}",
                    self.cfg.api_url
                )));
            }
        };

        let parsed = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };

        if !(200..300).contains(&status) {
            return Err(ApiError {
                message: message_for(status, &parsed),
                status: Some(status),
                body: Some(parsed),
            });
        }
        // The API wraps payloads in { path, statusCode, timestamp, result }.
        Ok(match parsed {
            Value::Object(mut map) if map.contains_key("result") => {
                map.remove("result").unwrap_or(Value::Null)
            }
            other => other,
        })
    }

    /// `{ id, email }`. Needs a session token.
    pub fn me(&self) -> Result<Value> {
        self.request("GET", "/account/me", None)
    }

    /// `{ currentPlan, availableCredit, totalCredit }`. Needs a session token.
    pub fn current_plan(&self) -> Result<Value> {
        self.request("GET", "/account/get-current-plan", None)
    }

    /// The account's existing API key, or `None`. Needs a session token.
    pub fn get_api_key(&self) -> Result<Option<String>> {
        let res = self.request("GET", "/account/api-key", None)?;
        Ok(res
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    /// Generates (rotates) the account's API key. Needs a session token.
    pub fn create_api_key(&self) -> Result<Option<String>> {
        let res = self.request("POST", "/account/api-key", None)?;
        Ok(res
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    /// One page (100 rows, newest first) of `{ id, name, youtubeId, state, createdAt }`.
    pub fn list_projects(&self, page: u32) -> Result<Vec<Value>> {
        let path = if page > 1 {
            format!("/api/list?page={page}")
        } else {
            "/api/list".to_string()
        };
        match self.request("GET", &path, None)? {
            Value::Array(rows) => Ok(rows),
            Value::Null => Ok(Vec::new()),
            other => Err(ApiError::local(format!(
                "Unexpected project list response: {other}"
            ))),
        }
    }

    /// One project. A finished project carries its transcript as `text`, with
    /// a `[HH:MM:SS.mmm] Speaker:` line before every paragraph; that is the
    /// only transcript shape the API key routes return.
    pub fn get_project(&self, id: &str) -> Result<Value> {
        let path = format!(
            "/api/detail/{}?showTimecode=true&showSpeaker=true",
            encode_component(id)
        );
        self.request("GET", &path, None)
    }

    /// Starts a YouTube transcription and returns the new project's id.
    pub fn transcribe_youtube(
        &self,
        youtube_video_id: &str,
        language: Option<&str>,
        webhook_url: Option<&str>,
    ) -> Result<String> {
        let mut body = json!({
            "youtubeVideoId": youtube_video_id,
            // The API requires the field; empty means "no webhook".
            "webhookUrl": webhook_url.unwrap_or(""),
        });
        if let Some(lang) = language {
            body["language"] = json!(lang);
        }
        let res = self.request("POST", "/api/transcribeVideo", Some(&body))?;
        res.get("projectId")
            .or_else(|| res.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ApiError::local(format!("The API did not return a project id: {res}")))
    }
}

/// `upload` | `transcription` | `done` | `error`, lower-cased. A finished
/// project comes back without a `state` field, so the presence of `text`
/// marks it done.
pub fn project_state(project: &Value) -> String {
    match project.get("state").and_then(Value::as_str) {
        Some(state) => state.to_lowercase(),
        None if project.get("text").is_some() => "done".to_string(),
        None => "unknown".to_string(),
    }
}

/// Resolves a YouTube watch URL or bare id into the 11-character video id.
pub fn youtube_id(input: &str) -> Option<String> {
    const MARKERS: [&str; 4] = ["youtu.be/", "v=", "/shorts/", "/embed/"];
    let is_id_char = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-';

    let trimmed = input.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() == 11 && bytes.iter().all(|&c| is_id_char(c)) {
        return Some(trimmed.to_string());
    }
    for start in 0..bytes.len() {
        for marker in MARKERS {
            let rest = &bytes[start..];
            if !rest.starts_with(marker.as_bytes()) {
                continue;
            }
            let id = &rest[marker.len()..];
            if id.len() >= 11 && id[..11].iter().all(|&c| is_id_char(c)) {
                return Some(String::from_utf8_lossy(&id[..11]).into_owned());
            }
        }
    }
    None
}

/// Percent-encodes a path segment (same set as JavaScript's encodeURIComponent).
fn encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn message_for(status: u16, body: &Value) -> String {
    let text = |v: &Value| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(items) if !items.is_empty() => Some(
            items
                .iter()
                .map(|i| {
                    i.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| i.to_string())
                })
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ => None,
    };
    // API errors: { error: { message } }. Gateway errors: { message } or { Message }.
    let api_message = ["message", "Message", "error"]
        .iter()
        .filter_map(|key| body.get(key))
        .find_map(|v| text(v).or_else(|| v.get("message").and_then(text)));

    match status {
        // The gateway's own text ("Unauthorized", "User is not authorized to
        // access this resource with an explicit deny...") tells the user
        // nothing about what to do.
        401 | 403 => "Authentication failed. Run `tube login` or set TUBE_API_KEY.".to_string(),
        402 => {
            api_message.unwrap_or_else(|| "Insufficient credit for this transcription.".to_string())
        }
        404 => api_message.unwrap_or_else(|| "Not found.".to_string()),
        _ => api_message.unwrap_or_else(|| format!("Request failed with status {status}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_id_passes_through_an_11_char_id() {
        assert_eq!(youtube_id("dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn youtube_id_parses_a_watch_url() {
        assert_eq!(
            youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn youtube_id_parses_a_watch_url_with_extra_params() {
        assert_eq!(
            youtube_id("https://www.youtube.com/watch?t=42&v=dQw4w9WgXcQ&list=PL1").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn youtube_id_parses_a_short_url() {
        assert_eq!(
            youtube_id("https://youtu.be/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn youtube_id_parses_a_shorts_url() {
        assert_eq!(
            youtube_id("https://www.youtube.com/shorts/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn youtube_id_parses_an_embed_url() {
        assert_eq!(
            youtube_id("https://www.youtube.com/embed/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn youtube_id_rejects_other_sites_and_empty_input() {
        assert_eq!(youtube_id("https://vimeo.com/123456"), None);
        assert_eq!(youtube_id(""), None);
        assert_eq!(youtube_id("https://youtu.be/short"), None);
    }

    #[test]
    fn youtube_id_trims_whitespace() {
        assert_eq!(
            youtube_id("  dQw4w9WgXcQ  ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn youtube_id_survives_non_ascii_input() {
        assert_eq!(youtube_id("vidéo ünïcödé çok uzun bir metin"), None);
    }

    #[test]
    fn state_is_lower_cased_and_text_means_done() {
        assert_eq!(project_state(&json!({"state": "UPLOAD"})), "upload");
        assert_eq!(
            project_state(&json!({"state": "transcription"})),
            "transcription"
        );
        assert_eq!(project_state(&json!({"id": "p", "text": "hello"})), "done");
        assert_eq!(project_state(&json!({"id": "p", "text": null})), "done");
        assert_eq!(project_state(&json!({"id": "p"})), "unknown");
    }

    #[test]
    fn error_message_is_read_from_every_known_shape() {
        assert_eq!(
            message_for(
                404,
                &json!({"error": {"message": "[404] Project not found"}})
            ),
            "[404] Project not found"
        );
        assert_eq!(
            message_for(
                400,
                &json!({"message": ["a must be a string", "b is required"]})
            ),
            "a must be a string, b is required"
        );
        assert_eq!(message_for(500, &json!({"error": "boom"})), "boom");
        assert_eq!(
            message_for(500, &Value::Null),
            "Request failed with status 500."
        );
        assert_eq!(
            message_for(402, &Value::String("<html>".into())),
            "Insufficient credit for this transcription."
        );
    }

    #[test]
    fn auth_errors_always_say_what_to_do() {
        let expected = "Authentication failed. Run `tube login` or set TUBE_API_KEY.";
        assert_eq!(
            message_for(401, &json!({"message": "Unauthorized"})),
            expected
        );
        assert_eq!(
            message_for(
                403,
                &json!({"Message": "User is not authorized to access this resource with an explicit deny"})
            ),
            expected
        );
        assert_eq!(message_for(403, &Value::Null), expected);
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_component("abc-123_~.x"), "abc-123_~.x");
        assert_eq!(encode_component("a/b?c d"), "a%2Fb%3Fc%20d");
    }
}
