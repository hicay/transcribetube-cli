//! Start a transcription and wait for it. Shared by the CLI command and the
//! MCP tool so both behave identically.
use std::fs::File;
use std::path::Path;
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

/// What the argument of `tube transcribe` points at.
#[derive(Debug, PartialEq)]
pub enum Source {
    /// An existing local file, with its size in bytes.
    File(u64),
    Youtube,
}

/// A local file wins over a YouTube id of the same name. Anything that is
/// neither is refused here, before any request is made.
pub fn classify(input: &str) -> Result<Source> {
    if let Ok(meta) = std::fs::metadata(input) {
        if meta.is_dir() {
            return Err(ApiError::local(format!(
                "\"{input}\" is a directory. Pass one audio or video file."
            )));
        }
        if meta.len() == 0 {
            return Err(ApiError::local(format!("\"{input}\" is an empty file.")));
        }
        return Ok(Source::File(meta.len()));
    }
    if youtube_id(input).is_some() {
        return Ok(Source::Youtube);
    }
    let lower = input.trim().to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Err(ApiError::local(format!(
            "Not a recognizable YouTube URL or video id: \"{input}\""
        )));
    }
    Err(ApiError::local(format!(
        "No such file, and not a YouTube URL or video id: \"{input}\""
    )))
}

/// The MIME type sent when asking for the upload URL and again with the
/// upload itself; the two must match or S3 rejects the signature.
pub fn content_type_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "aif" | "aiff" => "audio/aiff",
        "caf" => "audio/x-caf",
        "wma" => "audio/x-ms-wma",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "mpg" | "mpeg" => "video/mpeg",
        _ => "application/octet-stream",
    }
}

/// The file name sent to the API, which makes it the last part of the
/// storage key. Kept to plain characters so the key survives URLs, storage
/// event notifications and the server's temp path unchanged; the project
/// keeps the real name.
pub fn upload_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    // At most 100 characters, keeping the end: that is where the extension is.
    let safe = &safe[safe.len().saturating_sub(100)..];
    if safe.trim_matches(|c| c == '.' || c == '_').is_empty() {
        "file".to_string()
    } else {
        safe.to_string()
    }
}

/// Uploads a local audio or video file and starts its transcription.
/// Returns the project id. `on_upload` is called with the file size just
/// before the upload starts, for progress output.
pub fn start_file(
    client: &Client,
    path: &str,
    language: Option<&str>,
    on_upload: Option<&mut dyn FnMut(u64)>,
) -> Result<String> {
    let Source::File(_) = classify(path)? else {
        return Err(ApiError::local(format!("No such file: \"{path}\"")));
    };
    let file_path = Path::new(path);
    // Open first: an unreadable file should fail before the API is asked.
    let unreadable = |e: std::io::Error| ApiError::local(format!("Could not read \"{path}\": {e}"));
    let file = File::open(file_path).map_err(unreadable)?;
    let size = file.metadata().map_err(unreadable)?.len();

    let display_name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let content_type = content_type_for(file_path);

    let target = client.create_upload_url(&upload_name(file_path), content_type)?;
    if let Some(on_upload) = on_upload {
        on_upload(size);
    }
    client.upload(&target.url, content_type, file, size)?;

    let language = language.filter(|l| !l.is_empty() && *l != "auto");
    client.transcribe_file(&display_name, &target.key, language)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_follow_the_extension_whatever_its_case() {
        assert_eq!(content_type_for(Path::new("a/talk.mp3")), "audio/mpeg");
        assert_eq!(content_type_for(Path::new("Talk.M4A")), "audio/mp4");
        assert_eq!(content_type_for(Path::new("clip.MOV")), "video/quicktime");
        assert_eq!(content_type_for(Path::new("clip.mp4")), "video/mp4");
        assert_eq!(
            content_type_for(Path::new("notes.xyz")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type_for(Path::new("noextension")),
            "application/octet-stream"
        );
    }

    #[test]
    fn upload_names_keep_only_plain_characters_and_the_extension() {
        assert_eq!(
            upload_name(Path::new("dir/talk-1_final.mp3")),
            "talk-1_final.mp3"
        );
        assert_eq!(
            upload_name(Path::new("Board meeting (março) #3.m4a")),
            "Board_meeting__mar_o___3.m4a"
        );
        assert_eq!(upload_name(Path::new("a::b.wav")), "a__b.wav");
        assert_eq!(upload_name(Path::new("日本語")), "file");

        let long = format!("{}.mp3", "x".repeat(300));
        let name = upload_name(Path::new(&long));
        assert_eq!(name.len(), 100);
        assert!(name.ends_with(".mp3"));
    }

    #[test]
    fn something_that_is_no_file_is_a_youtube_id_or_refused() {
        assert_eq!(classify("dQw4w9WgXcQ").unwrap(), Source::Youtube);
        assert_eq!(
            classify("https://youtu.be/dQw4w9WgXcQ").unwrap(),
            Source::Youtube
        );
        let err = classify("https://vimeo.com/1").unwrap_err();
        assert!(err.message.starts_with("Not a recognizable YouTube URL"));
        let err = classify("no-such-recording.mp3").unwrap_err();
        assert!(err.message.starts_with("No such file"), "{}", err.message);
    }
}
