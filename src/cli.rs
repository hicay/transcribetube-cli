//! The `tube` command line.
use std::io::{BufRead, IsTerminal};
use std::process::{Command, Stdio};
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use crate::api::{project_state, ApiError, Client, Result};
use crate::config;
use crate::format::{format_project, Format, FormatOptions};
use crate::output::{fail, info, json as print_json, out};
use crate::transcribe::{
    classify, start_file, start_youtube, wait_for_project, Source, WaitOptions,
};

#[derive(Parser)]
#[command(
    name = "tube",
    version,
    about = "TranscribeTube CLI: transcribe YouTube videos and audio or video files from the terminal or from AI assistants."
)]
struct Cli {
    /// API key (overrides config and TUBE_API_KEY)
    #[arg(long, global = true, value_name = "KEY")]
    api_key: Option<String>,
    /// Machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,
    /// Override the API base URL
    #[arg(long, global = true, value_name = "URL")]
    api_url: Option<String>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Sign in. Opens the dashboard to copy your API key, or pass --api-key.
    Login {
        /// Store a session token (needed only for `account api-key`)
        #[arg(long, value_name = "JWT")]
        token: Option<String>,
    },
    /// Remove stored credentials.
    Logout,
    /// Show which credentials are in use and whether they work.
    Whoami,
    /// Account and API key management.
    Account {
        #[command(subcommand)]
        command: AccountCmd,
    },
    /// Transcribe a local audio or video file, or a YouTube URL. Prints the transcript with --wait.
    Transcribe {
        /// Path to an audio or video file, a YouTube URL, or an 11-character video id
        #[arg(value_name = "PATH_OR_URL")]
        input: String,
        /// Language code (default: auto-detect)
        #[arg(short, long, value_name = "CODE")]
        lang: Option<String>,
        #[arg(short, long, value_enum, default_value = "txt")]
        format: Format,
        /// Wait for completion and print the transcript
        #[arg(short, long)]
        wait: bool,
        /// With --wait: give up waiting after this many minutes
        #[arg(long, value_name = "MINUTES", default_value = "15", requires = "wait", value_parser = parse_minutes)]
        timeout: Duration,
        /// POST to this URL when the transcription completes (YouTube only)
        #[arg(long, value_name = "URL")]
        webhook: Option<String>,
    },
    /// Manage projects.
    Project {
        #[command(subcommand)]
        command: ProjectCmd,
    },
    /// Fetch a finished transcript in a chosen format.
    Transcript {
        id: String,
        #[arg(short, long, value_enum, default_value = "txt")]
        format: Format,
        /// Omit speaker labels
        #[arg(long)]
        no_speakers: bool,
        /// Omit timecodes
        #[arg(long)]
        no_timecodes: bool,
    },
    /// Run the Model Context Protocol server (stdio) for AI clients.
    Mcp,
}

#[derive(Subcommand)]
enum AccountCmd {
    /// Show your API key, or generate a new one with --create. Needs a session token.
    ApiKey {
        /// Generate (rotate) a new API key
        #[arg(long)]
        create: bool,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// List your projects, newest first.
    List {
        /// Max rows to print (the API returns 100 per page)
        #[arg(long, default_value_t = 20, value_name = "N")]
        limit: usize,
        #[arg(long, default_value_t = 1, value_name = "N")]
        page: u32,
    },
    /// Show a project as JSON.
    Get { id: String },
    /// Poll a project until it is done or has failed.
    Watch {
        id: String,
        /// Give up waiting after this many minutes
        #[arg(long, value_name = "MINUTES", default_value = "15", value_parser = parse_minutes)]
        timeout: Duration,
    },
}

/// `--timeout`: minutes, fractions allowed (0.5 is 30 seconds), up to a day.
fn parse_minutes(text: &str) -> std::result::Result<Duration, String> {
    match text.trim().parse::<f64>() {
        Ok(minutes) if minutes > 0.0 && minutes <= 1440.0 => {
            Ok(Duration::from_secs_f64(minutes * 60.0))
        }
        _ => Err("expected a number of minutes, above 0 and at most 1440".to_string()),
    }
}

const NEEDS_TOKEN: &str = "This needs a session token, which an API key cannot replace. \
Copy your key from the dashboard instead (Settings → API access), or pass a token with `tube login --token <jwt>`.";

pub fn run() -> i32 {
    let cli = Cli::parse();
    match dispatch(&cli) {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

fn client_for(cli: &Cli, source: &'static str) -> Client {
    let mut cfg = config::load();
    if let Some(key) = cli.api_key.as_ref().filter(|k| !k.is_empty()) {
        cfg.api_key = Some(key.clone());
    }
    if let Some(url) = cli.api_url.as_ref().filter(|u| !u.is_empty()) {
        cfg.api_url = url.trim_end_matches('/').to_string();
    }
    Client::new(cfg, source)
}

fn dispatch(cli: &Cli) -> Result<()> {
    let quiet = cli.json;
    match &cli.command {
        Cmd::Login { token } => login(cli, token.as_deref()),
        Cmd::Logout => {
            save(&[("apiKey", None), ("token", None)])?;
            info("Logged out.", false);
            Ok(())
        }
        Cmd::Whoami => whoami(cli),
        Cmd::Account {
            command: AccountCmd::ApiKey { create },
        } => {
            let client = client_for(cli, "cli");
            if !client.has_token() {
                return Err(ApiError::new(NEEDS_TOKEN, 401));
            }
            let key = if *create {
                client.create_api_key()?
            } else {
                client.get_api_key()?
            };
            let Some(key) = key else {
                return Err(ApiError::local(
                    "No API key on this account yet. Run `tube account api-key --create` to generate one.",
                ));
            };
            if cli.json {
                print_json(&json!({ "apiKey": key }));
            } else {
                // stdout only, so `export TUBE_API_KEY=$(tube account api-key)` works
                out(&key);
                if *create {
                    info("New API key generated.", false);
                }
            }
            Ok(())
        }
        Cmd::Transcribe {
            input,
            lang,
            format,
            wait,
            timeout,
            webhook,
        } => {
            let client = client_for(cli, "cli");
            let id = match classify(input)? {
                Source::File(_) => {
                    if webhook.is_some() {
                        return Err(ApiError::local(
                            "--webhook works for YouTube URLs only; the API has no webhook for uploaded files. \
Use --wait, or `tube project watch <id>`.",
                        ));
                    }
                    let mut report = |bytes: u64| {
                        info(&format!("uploading {input} ({})", human_size(bytes)), quiet)
                    };
                    start_file(&client, input, lang.as_deref(), Some(&mut report))?
                }
                Source::Youtube => {
                    start_youtube(&client, input, lang.as_deref(), webhook.as_deref())?
                }
            };
            if !*wait {
                if cli.json {
                    print_json(&json!({ "id": id }));
                } else {
                    out(&id);
                    info(
                        &format!("Submitted. Track with: tube project watch {id}"),
                        false,
                    );
                }
                return Ok(());
            }
            let project = wait_with_progress(&client, &id, *timeout, quiet)?;
            if cli.json {
                print_json(&project);
            } else {
                require_text(&project)?;
                out(&format_project(&project, *format, FormatOptions::default()));
            }
            Ok(())
        }
        Cmd::Project { command } => {
            let client = client_for(cli, "cli");
            match command {
                ProjectCmd::List { limit, page } => {
                    let mut rows = client.list_projects(*page)?;
                    rows.truncate(*limit);
                    if cli.json {
                        print_json(&Value::Array(rows));
                    } else {
                        let cell = |row: &Value, key: &str| {
                            row.get(key)
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string()
                        };
                        for row in &rows {
                            out(&format!(
                                "{}\t{}\t{}\t{}",
                                cell(row, "id"),
                                cell(row, "state"),
                                cell(row, "createdAt"),
                                cell(row, "name")
                            ));
                        }
                    }
                }
                ProjectCmd::Get { id } => print_json(&client.get_project(id)?),
                ProjectCmd::Watch { id, timeout } => {
                    let project = wait_with_progress(&client, id, *timeout, quiet)?;
                    if cli.json {
                        print_json(&project);
                    } else {
                        out(&format!("done {id}"));
                    }
                }
            }
            Ok(())
        }
        Cmd::Transcript {
            id,
            format,
            no_speakers,
            no_timecodes,
        } => {
            let client = client_for(cli, "cli");
            let project = client.get_project(id)?;
            match project_state(&project).as_str() {
                "done" => {}
                "error" => return Err(failed(&project)),
                state => {
                    return Err(ApiError::local(format!(
                        "The transcript is not ready yet (state: {state}). Wait for it with: tube project watch {id}"
                    )))
                }
            }
            require_text(&project)?;
            out(&format_project(
                &project,
                *format,
                FormatOptions {
                    speakers: !no_speakers,
                    timecodes: !no_timecodes,
                },
            ));
            Ok(())
        }
        Cmd::Mcp => {
            crate::mcp::run_stdio(client_for(cli, "mcp"));
            Ok(())
        }
    }
}

/// A finished project should carry its transcript. Two things can be wrong:
/// no text at all (the API marked it done without one), or empty text (a
/// video without speech). Empty output with exit code 0 would hide both.
fn require_text(project: &Value) -> Result<()> {
    match project.get("text").and_then(Value::as_str) {
        None => Err(ApiError::new(crate::api::NO_TRANSCRIPT, 422)),
        Some(text) => {
            if text.trim().is_empty() {
                info(
                    "note: the transcript is empty (no speech was detected).",
                    false,
                );
            }
            Ok(())
        }
    }
}

fn failed(project: &Value) -> ApiError {
    let message = project
        .get("errorMessage")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(|m| format!("Transcription failed: {m}"))
        .unwrap_or_else(|| "Transcription failed.".to_string());
    ApiError {
        message,
        status: Some(422),
        body: None,
    }
}

fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let bytes = bytes as f64;
    if bytes >= 1024.0 * MB {
        format!("{:.1} GB", bytes / (1024.0 * MB))
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else {
        format!("{:.0} kB", (bytes / 1024.0).max(1.0))
    }
}

fn wait_with_progress(client: &Client, id: &str, timeout: Duration, quiet: bool) -> Result<Value> {
    let mut report = |state: &str| info(&format!("state: {state}"), quiet);
    wait_for_project(
        client,
        id,
        WaitOptions {
            timeout,
            on_state: Some(&mut report),
            ..WaitOptions::default()
        },
    )
}

fn save(patch: &[(&str, Option<&str>)]) -> Result<()> {
    config::save(patch).map_err(|e| {
        ApiError::local(format!(
            "Could not write {}: {e}",
            config::config_path().display()
        ))
    })
}

fn login(cli: &Cli, token: Option<&str>) -> Result<()> {
    let mut api_key = cli.api_key.clone().filter(|k| !k.is_empty());

    // Interactive onboarding: walk a new user to their key.
    if api_key.is_none() && token.is_none() {
        let key_url = format!("{}/settings/api-access", config::load().dashboard_url);
        info("Welcome to TranscribeTube", false);
        info("", false);
        info(
            "1. Sign in or create an account, then copy your API key from:",
            false,
        );
        info(&format!("   {key_url}"), false);
        if std::io::stdin().is_terminal() {
            info("   (Opening it in your browser...)", false);
            open_browser(&key_url);
        }
        info("", false);
        let entered = prompt_secret("2. Paste your API key: ");
        if entered.is_empty() {
            return Err(ApiError::local("No API key entered."));
        }
        api_key = Some(entered);
    }

    // Verify before storing, so a typo never replaces a working credential.
    let mut cfg = config::load();
    if api_key.is_some() {
        cfg.api_key = api_key.clone();
    }
    if token.is_some() {
        cfg.token = token.map(str::to_string);
    }
    if let Some(url) = cli.api_url.as_ref().filter(|u| !u.is_empty()) {
        cfg.api_url = url.trim_end_matches('/').to_string();
    }
    let client = Client::new(cfg, "cli");
    let summary = verify(&client, api_key.is_some(), token.is_some())?;

    let mut patch: Vec<(&str, Option<&str>)> = Vec::new();
    if let Some(key) = api_key.as_deref() {
        patch.push(("apiKey", Some(key)));
    }
    if let Some(token) = token {
        patch.push(("token", Some(token)));
    }
    save(&patch)?;

    info(&format!("Logged in. {summary}"), false);
    info("", false);
    info(
        "Try it:  tube transcribe \"https://youtu.be/<id>\" --wait",
        false,
    );
    Ok(())
}

/// Checks each given credential against a route that accepts it.
fn verify(client: &Client, check_key: bool, check_token: bool) -> Result<String> {
    let mut parts = Vec::new();
    if check_token {
        let me = client.me()?;
        let email = me.get("email").and_then(Value::as_str).unwrap_or("unknown");
        parts.push(format!("Account: {email}."));
        if let Ok(plan) = client.current_plan() {
            if let Some(credit) = plan.get("availableCredit") {
                parts.push(format!("Credit available: {credit}."));
            }
        }
    }
    if check_key {
        client.list_projects(1)?;
        parts.push("API key works.".to_string());
    }
    Ok(parts.join(" "))
}

fn whoami(cli: &Cli) -> Result<()> {
    let client = client_for(cli, "cli");
    if !client.authenticated() {
        return Err(ApiError::new(
            "Not logged in. Run `tube login` or set TUBE_API_KEY.",
            401,
        ));
    }
    // The API key route returns the whole account; a session token needs two
    // requests for the same facts. Either way the report has the same shape.
    let (mut report, plan) = if client.has_api_key() {
        let mut account = match client.account()? {
            Value::Object(fields) => fields,
            _ => serde_json::Map::new(),
        };
        let mut plan = serde_json::Map::new();
        for field in ["currentPlan", "availableCredit", "totalCredit"] {
            if let Some(value) = account.remove(field) {
                plan.insert(field.into(), value);
            }
        }
        account.insert("apiKey".into(), json!({ "valid": true }));
        (account, Some(Value::Object(plan)))
    } else {
        let me = match client.me()? {
            Value::Object(fields) => fields,
            _ => serde_json::Map::new(),
        };
        (me, client.current_plan().ok())
    };

    let mut lines = vec![report
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()];
    if let Some(plan) = &plan {
        if let Some(name) = plan.get("currentPlan").and_then(Value::as_str) {
            lines.push(format!("plan: {name}"));
        }
        match (plan.get("availableCredit"), plan.get("totalCredit")) {
            (Some(left), Some(total)) if !total.is_null() => {
                lines.push(format!("credit: {left} of {total} minutes"))
            }
            (Some(left), _) => lines.push(format!("credit: {left} minutes")),
            _ => {}
        }
    }
    report.insert("plan".into(), plan.unwrap_or(Value::Null));

    if cli.json {
        print_json(&Value::Object(report));
    } else {
        out(&lines.join("\n"));
    }
    Ok(())
}

/// Best-effort open of a URL in the default browser. Never fails.
fn open_browser(url: &str) {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/c", "start", "", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    // Headless or no browser: the caller already printed the URL.
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Reads a secret without echo on a terminal, or one line from a pipe.
fn prompt_secret(question: &str) -> String {
    if std::io::stdin().is_terminal() {
        rpassword::prompt_password(question)
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        eprint!("{question}");
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        eprintln!();
        line.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_work_before_and_after_the_subcommand() {
        let before = Cli::try_parse_from(["tube", "--json", "--api-key", "k", "whoami"]).unwrap();
        assert!(before.json);
        assert_eq!(before.api_key.as_deref(), Some("k"));
        let after =
            Cli::try_parse_from(["tube", "project", "list", "--json", "--api-key", "k"]).unwrap();
        assert!(after.json);
        assert_eq!(after.api_key.as_deref(), Some("k"));
    }

    #[test]
    fn an_unknown_format_is_refused() {
        assert!(Cli::try_parse_from(["tube", "transcript", "p1", "--format", "docx"]).is_err());
        assert!(Cli::try_parse_from(["tube", "transcript", "p1", "-f", "srt"]).is_ok());
    }

    #[test]
    fn transcribe_accepts_its_short_flags() {
        let cli = Cli::try_parse_from([
            "tube",
            "transcribe",
            "dQw4w9WgXcQ",
            "-w",
            "-f",
            "vtt",
            "-l",
            "nl",
        ])
        .unwrap();
        match cli.command {
            Cmd::Transcribe {
                wait, format, lang, ..
            } => {
                assert!(wait);
                assert_eq!(format, Format::Vtt);
                assert_eq!(lang.as_deref(), Some("nl"));
            }
            _ => panic!("expected transcribe"),
        }
    }

    #[test]
    fn the_wait_timeout_defaults_to_15_minutes_and_takes_minutes() {
        let timeout_of = |args: &[&str]| match Cli::try_parse_from(args).unwrap().command {
            Cmd::Transcribe { timeout, .. } => timeout,
            Cmd::Project {
                command: ProjectCmd::Watch { timeout, .. },
            } => timeout,
            _ => panic!("expected transcribe or project watch"),
        };
        let minutes = |m: u64| Duration::from_secs(m * 60);
        assert_eq!(
            timeout_of(&["tube", "transcribe", "x", "--wait"]),
            minutes(15)
        );
        assert_eq!(timeout_of(&["tube", "project", "watch", "p1"]), minutes(15));
        assert_eq!(
            timeout_of(&["tube", "transcribe", "x", "-w", "--timeout", "45"]),
            minutes(45)
        );
        assert_eq!(
            timeout_of(&["tube", "project", "watch", "p1", "--timeout", "0.5"]),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn a_bad_timeout_is_refused() {
        for bad in ["0", "-1", "soon", "NaN", "inf", "1441"] {
            assert!(
                Cli::try_parse_from(["tube", "project", "watch", "p1", "--timeout", bad]).is_err(),
                "--timeout {bad} should be rejected"
            );
        }
        // Without --wait there is nothing to time out.
        assert!(Cli::try_parse_from(["tube", "transcribe", "x", "--timeout", "5"]).is_err());
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human_size(10), "1 kB");
        assert_eq!(human_size(300 * 1024), "300 kB");
        assert_eq!(human_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
