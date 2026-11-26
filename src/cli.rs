//! The `tube` command line.
use std::io::{BufRead, IsTerminal};
use std::process::{Command, Stdio};

use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use crate::api::{project_state, ApiError, Client, Result};
use crate::config;
use crate::format::{format_project, Format, FormatOptions};
use crate::output::{fail, info, json as print_json, out};
use crate::transcribe::{start_youtube, wait_for_project, WaitOptions};

#[derive(Parser)]
#[command(
    name = "tube",
    version,
    about = "TranscribeTube CLI: transcribe YouTube videos from the terminal or from AI assistants."
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
        /// Store a session token (needed only for `whoami` credit and `account api-key`)
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
    /// Transcribe a YouTube URL. Prints the transcript with --wait.
    Transcribe {
        /// YouTube URL or 11-character video id
        url: String,
        /// Language code (default: auto-detect)
        #[arg(short, long, value_name = "CODE")]
        lang: Option<String>,
        #[arg(short, long, value_enum, default_value = "txt")]
        format: Format,
        /// Wait for completion and print the transcript
        #[arg(short, long)]
        wait: bool,
        /// POST to this URL when the transcription completes
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
    Watch { id: String },
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
            url,
            lang,
            format,
            wait,
            webhook,
        } => {
            let client = client_for(cli, "cli");
            let id = start_youtube(&client, url, lang.as_deref(), webhook.as_deref())?;
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
            let project = wait_with_progress(&client, &id, quiet)?;
            if cli.json {
                print_json(&project);
            } else {
                note_if_empty(&project);
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
                ProjectCmd::Watch { id } => {
                    let project = wait_with_progress(&client, id, quiet)?;
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
            if project.get("text").and_then(Value::as_str).is_none() {
                return Err(ApiError::local(
                    "The project is done but has no transcript text.",
                ));
            }
            note_if_empty(&project);
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

/// A video without speech finishes with an empty transcript; say so, since
/// empty output with exit code 0 otherwise looks like a bug.
fn note_if_empty(project: &Value) {
    let text = project.get("text").and_then(Value::as_str).unwrap_or("");
    if text.trim().is_empty() {
        info(
            "note: the transcript is empty (no speech was detected).",
            false,
        );
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

fn wait_with_progress(client: &Client, id: &str, quiet: bool) -> Result<Value> {
    let mut report = |state: &str| info(&format!("state: {state}"), quiet);
    wait_for_project(
        client,
        id,
        WaitOptions {
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
    let mut report = serde_json::Map::new();
    let mut lines = Vec::new();

    if client.has_token() {
        let me = client.me()?;
        let plan = client.current_plan().ok();
        lines.push(
            me.get("email")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
        );
        if let Some(credit) = plan.as_ref().and_then(|p| p.get("availableCredit")) {
            lines.push(format!("credit: {credit}"));
        }
        if let Value::Object(fields) = me {
            report.extend(fields);
        }
        report.insert("plan".into(), plan.unwrap_or(Value::Null));
    }
    if client.has_api_key() {
        // No API key route returns the account, so prove the key by using it.
        let count = client.list_projects(1)?.len();
        let shown = if count >= 100 {
            "100+".to_string()
        } else {
            count.to_string()
        };
        let noun = if count == 1 { "project" } else { "projects" };
        lines.push(format!("API key works ({shown} {noun})"));
        report.insert("apiKey".into(), json!({ "valid": true, "projects": count }));
    }

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
}
