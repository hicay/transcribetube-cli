//! Start a transcription and wait for it. Shared by the CLI command and the
//! MCP tool so both behave identically.
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::api::{project_state, youtube_id, ApiError, Client, Result};

pub struct WaitOptions<'a> {
    pub timeout: Duration,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    /// Called once per state change, for progress output.
    pub on_state: Option<&'a mut dyn FnMut(&str)>,
}

impl Default for WaitOptions<'_> {
    fn default() -> Self {
        WaitOptions {
            timeout: Duration::from_secs(15 * 60),
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(15),
            on_state: None,
        }
    }
}

/// Starts a YouTube transcription and returns the project id.
pub fn start_youtube(
    client: &Client,
    url: &str,
    language: Option<&str>,
    webhook_url: Option<&str>,
) -> Result<String> {
    let id = youtube_id(url).ok_or_else(|| {
        ApiError::local(format!(
            "Not a recognizable YouTube URL or video id: \"{url}\""
        ))
    })?;
    let language = language.filter(|l| !l.is_empty() && *l != "auto");
    client.transcribe_youtube(&id, language, webhook_url)
}

/// Polls until the project is done and returns it. A failed project is an
/// error with status 422, a timeout one with status 408.
pub fn wait_for_project(client: &Client, project_id: &str, mut opts: WaitOptions) -> Result<Value> {
    let deadline = Instant::now() + opts.timeout;
    let mut delay = opts.initial_delay;
    let mut last_state = String::new();

    loop {
        let project = client.get_project(project_id)?;
        let state = project_state(&project);
        if state != last_state {
            if let Some(on_state) = opts.on_state.as_mut() {
                on_state(&state);
            }
            last_state = state.clone();
        }
        match state.as_str() {
            "done" => return Ok(project),
            "error" => {
                let reason = project
                    .get("errorMessage")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .map(|m| format!("Transcription failed: {m}"))
                    .unwrap_or_else(|| "Transcription failed.".to_string());
                return Err(ApiError {
                    message: reason,
                    status: Some(422),
                    body: Some(project),
                });
            }
            _ => {}
        }
        if Instant::now() + delay > deadline {
            return Err(ApiError::new(
                format!("Timed out waiting for transcription. Check later with: tube project watch {project_id}"),
                408,
            ));
        }
        sleep(delay);
        delay = delay.mul_f64(1.5).min(opts.max_delay);
    }
}
