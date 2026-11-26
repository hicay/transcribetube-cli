# TranscribeTube CLI

Transcribe **YouTube videos** to text, subtitles (`.srt`/`.vtt`), markdown or
JSON from your terminal, your CI pipeline, or directly from an AI assistant
via MCP.

Two small native binaries, no runtime to install:

- `tube`: the command line
- `transcribetube-mcp`: the MCP server (the same server as `tube mcp`)

## Install

```bash
cargo install --git https://github.com/hicay/transcribetube-cli
```

Or download the archive for your platform from the
[releases page](https://github.com/hicay/transcribetube-cli/releases) and put
`tube` and `transcribetube-mcp` on your `PATH`.

## Quick start

```bash
tube login          # opens the dashboard, paste your API key, done
tube transcribe "https://youtu.be/dQw4w9WgXcQ" --wait --format srt > out.srt
```

`tube login` opens `app.transcribetube.com/settings/api-access` so you can
sign in and copy your key, asks you to paste it, and stores it only after the
API accepts it. Scripted setups can skip the prompt:

```bash
tube login --api-key <key>
# or, with nothing stored at all:
TUBE_API_KEY=<key> tube transcribe "<url>" --wait
```

**Use from Claude or any MCP client.** Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "transcribetube": {
      "command": "transcribetube-mcp",
      "env": { "TUBE_API_KEY": "<key>" }
    }
  }
}
```

Claude Code one-liner:

```bash
claude mcp add transcribetube --env TUBE_API_KEY=<key> -- transcribetube-mcp
```

Tools exposed: `transcribe_youtube`, `get_transcript`, `list_projects`.
See [`AI_INTEGRATION.md`](./AI_INTEGRATION.md) for how discovery works.

## Commands

| Command | Description |
|---|---|
| `tube transcribe <url>` | Start a transcription and print the project id. `--wait` waits and prints the transcript. `--format`, `--lang`, `--webhook`. |
| `tube transcript <id>` | Print a finished transcript. `--format txt\|srt\|vtt\|md\|json`, `--no-speakers`, `--no-timecodes`. |
| `tube project list` | List your projects: id, state, created, name. `--limit`, `--page`. |
| `tube project get <id>` | Show one project as JSON, as the API returns it. |
| `tube project watch <id>` | Wait until the project is done or has failed. |
| `tube whoami` | Check that the stored credentials work. |
| `tube login` / `tube logout` | Store or remove credentials. |
| `tube account api-key` | Print your API key, `--create` to rotate it. Needs a session token (see below). |
| `tube mcp` | Run the MCP server on stdio. |

Global flags, accepted before or after the command: `--json`
(machine-readable, no progress output), `--api-key`, `--api-url`.

### Output and exit codes

Data is written to **stdout**; progress and errors go to **stderr**, so pipes
stay clean. Exit codes: `0` ok · `2` authentication · `3` insufficient credit ·
`4` the transcription failed · `5` not found · `1` anything else.

## Authentication

Order of precedence: `--api-key` flag, then the `TUBE_API_KEY` environment
variable, then the stored config (`~/.config/tube/config.json`, readable only
by you).

The API has two kinds of credential, and each works only for its own routes:

| Credential | Sent as | Works for |
|---|---|---|
| API key | `api-key` header | transcribing, listing, reading transcripts (everything most people need) |
| Session token | `Authorization: Bearer` | account email, remaining credit, reading or rotating the API key |

So with an API key alone, `tube whoami` can confirm that the key works but
cannot show the account email or credit, and `tube account api-key` is not
available. A session token (`tube login --token <jwt>` or `TUBE_TOKEN`) adds
those; it expires after about an hour.

## Subtitles: what to expect

The API returns a finished transcript as paragraphs with a start time and a
speaker. `srt` and `vtt` output therefore has **one cue per paragraph**, and a
cue ends where the next one starts. That is fine for reading along and for
search, but too coarse for burned-in captions.

## Development

```bash
cargo test            # unit tests, plus the real binaries against a mock API
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Layout: `src/api.rs` (HTTP client), `src/format.rs` (transcript parsing and
output formats), `src/transcribe.rs` (start and wait), `src/mcp.rs` (MCP
server), `src/cli.rs` (commands), `src/config.rs` (credentials).
