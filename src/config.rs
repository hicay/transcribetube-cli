//! Credentials and endpoints: defaults, then `~/.config/tube/config.json`,
//! then environment. Environment always wins so CI stays stateless.
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde_json::{Map, Value};

pub const DEFAULT_API_URL: &str = "https://api.transcribetube.com";
pub const DEFAULT_DASHBOARD_URL: &str = "https://app.transcribetube.com";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub api_url: String,
    /// Web dashboard base URL, used by `tube login` onboarding.
    pub dashboard_url: String,
    /// Long-lived API key (header: `api-key`). Used for `/api/*`.
    pub api_key: Option<String>,
    /// Supabase JWT (header: `Authorization: Bearer`). Used for `/account/*`.
    pub token: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_url: DEFAULT_API_URL.to_string(),
            dashboard_url: DEFAULT_DASHBOARD_URL.to_string(),
            api_key: None,
            token: None,
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn config_path() -> PathBuf {
    let base = env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::home_dir().unwrap_or_default().join(".config"));
    base.join("tube").join("config.json")
}

/// The stored file as a JSON object; a missing or corrupt file is empty.
fn load_file() -> Map<String, Value> {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn load() -> Config {
    resolve(&load_file(), &env)
}

/// Pure merge of file and environment, separated from I/O for tests.
pub fn resolve(file: &Map<String, Value>, env: &dyn Fn(&str) -> Option<String>) -> Config {
    let from_file = |key: &str| {
        file.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let mut cfg = Config::default();
    if let Some(v) = from_file("apiUrl") {
        cfg.api_url = v;
    }
    if let Some(v) = from_file("dashboardUrl") {
        cfg.dashboard_url = v;
    }
    cfg.api_key = from_file("apiKey");
    cfg.token = from_file("token");

    if let Some(v) = env("TUBE_API_KEY") {
        cfg.api_key = Some(v);
    }
    if let Some(v) = env("TUBE_TOKEN") {
        cfg.token = Some(v);
    }
    if let Some(v) = env("TUBE_API_URL") {
        cfg.api_url = v;
    }
    if let Some(v) = env("TUBE_DASHBOARD_URL") {
        cfg.dashboard_url = v;
    }
    cfg.api_url = cfg.api_url.trim_end_matches('/').to_string();
    cfg.dashboard_url = cfg.dashboard_url.trim_end_matches('/').to_string();
    cfg
}

/// Merges `patch` into the stored file; `None` removes the key. Keys this
/// version does not know about are kept. The file is created owner-only
/// because it holds a credential.
pub fn save(patch: &[(&str, Option<&str>)]) -> std::io::Result<()> {
    let mut file = load_file();
    for (key, value) in patch {
        match value {
            Some(v) => {
                file.insert((*key).to_string(), Value::String((*v).to_string()));
            }
            None => {
                file.remove(*key);
            }
        }
    }
    let path = config_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(file))? + "\n";

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options.open(&path)?;
    handle.write_all(text.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `mode` only applies on creation; tighten a file that already existed.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn defaults_apply_with_no_file_and_no_env() {
        let cfg = resolve(&Map::new(), &|_| None);
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn file_values_override_defaults() {
        let cfg = resolve(
            &file(json!({"apiKey": "from-file", "apiUrl": "https://file.example/"})),
            &|_| None,
        );
        assert_eq!(cfg.api_key.as_deref(), Some("from-file"));
        assert_eq!(cfg.api_url, "https://file.example");
    }

    #[test]
    fn environment_wins_over_the_file() {
        let cfg = resolve(
            &file(json!({"apiKey": "from-file", "apiUrl": "https://file.example"})),
            &|name| match name {
                "TUBE_API_KEY" => Some("from-env".to_string()),
                "TUBE_API_URL" => Some("https://env.example".to_string()),
                _ => None,
            },
        );
        assert_eq!(cfg.api_key.as_deref(), Some("from-env"));
        assert_eq!(cfg.api_url, "https://env.example");
    }

    #[test]
    fn empty_strings_in_the_file_count_as_unset() {
        let cfg = resolve(&file(json!({"apiKey": "", "token": 5})), &|_| None);
        assert_eq!(cfg.api_key, None);
        assert_eq!(cfg.token, None);
    }
}
