# TranscribeTube CLI

Transcribe **YouTube videos** to text, subtitles (`.srt`/`.vtt`), markdown or
JSON from your terminal, your CI pipeline, or directly from an AI assistant
via MCP.

Two small native binaries, no runtime to install:

- `tube`: the command line
- `transcribetube-mcp`: the MCP server (the same server as `tube mcp`)

## Install

### macOS and Linux: one line

```bash
curl -fsSL https://raw.githubusercontent.com/hicay/transcribetube-cli/main/install.sh | sh
```

This downloads the right binary for your machine (Apple silicon or Intel Mac,
x86-64 or ARM Linux) into `~/.local/bin`. The Linux build is static, so it
runs on any distribution, including Alpine. If the script says that
`~/.local/bin` is not on your `PATH`, add it:

```bash
# macOS (zsh)
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
# Linux (bash)
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc && source ~/.bashrc
```

To install somewhere else, or a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/hicay/transcribetube-cli/main/install.sh | TUBE_INSTALL_DIR=/usr/local/bin sudo -E sh
curl -fsSL https://raw.githubusercontent.com/hicay/transcribetube-cli/main/install.sh | TUBE_VERSION=0.1.1 sh
```

### npm (macOS, Linux, Windows)

```bash
npm install -g @transcribetube/cli
tube --version
```

Or without installing anything:

```bash
npx -y @transcribetube/cli transcribe "https://youtu.be/dQw4w9WgXcQ" --wait
```

The npm package is a small launcher: it downloads the native binary for your
platform from the GitHub release of the same version and checks its SHA-256.
Node 18 or newer.

### By hand

Download the archive for your platform from the
[releases page](https://github.com/hicay/transcribetube-cli/releases), unpack
it, and put `tube` and `transcribetube-mcp` on your `PATH`:

```bash
# Apple silicon Mac; use x86_64-apple-darwin for an Intel Mac
curl -fsSLO https://github.com/hicay/transcribetube-cli/releases/download/v0.1.1/tube-v0.1.1-aarch64-apple-darwin.tar.gz
tar -xzf tube-v0.1.1-aarch64-apple-darwin.tar.gz
sudo mv tube-v0.1.1-aarch64-apple-darwin/tube tube-v0.1.1-aarch64-apple-darwin/transcribetube-mcp /usr/local/bin/
```

```bash
# x86-64 Linux; use aarch64-unknown-linux-musl on ARM
curl -fsSLO https://github.com/hicay/transcribetube-cli/releases/download/v0.1.1/tube-v0.1.1-x86_64-unknown-linux-musl.tar.gz
tar -xzf tube-v0.1.1-x86_64-unknown-linux-musl.tar.gz
sudo mv tube-v0.1.1-x86_64-unknown-linux-musl/tube tube-v0.1.1-x86_64-unknown-linux-musl/transcribetube-mcp /usr/local/bin/
```

On Windows, download `tube-v0.1.1-x86_64-pc-windows-msvc.zip`, unpack it, and
add the folder to your `Path`.

### From source

```bash
cargo install --git https://github.com/hicay/transcribetube-cli
```

### Update or remove

Run the install command again to update. To remove: delete `tube` and
`transcribetube-mcp` from the install folder (or `npm uninstall -g
@transcribetube/cli`), and `~/.config/tube` if you want the stored key gone.

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

**Use from Claude or any MCP client.** Add to `claude_desktop_config.json`
(if the client cannot find the command, give the full path, for example
`/Users/you/.local/bin/transcribetube-mcp`; with nothing installed, use
`"command": "npx"` and `"args": ["-y", "@transcribetube/cli", "mcp"]`):

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

## How to use it

### 1. Sign in once

```bash
tube login
```

This opens the API access page of the dashboard. Copy the key, paste it at the
prompt, and the CLI checks it against the API before storing it. To check
later that it still works:

```bash
tube whoami
# API key works (97 projects)
```

### 2. Transcribe a video and wait for the text

```bash
tube transcribe "https://www.youtube.com/watch?v=zhWDdy_5v2w" --wait
```

```text
state: upload
state: transcription
state: done
[00:00:00] Speaker 0: In a single minute your body produces 120 to 180 million red blood cells, ...
[00:00:34] Speaker 0: An average of 1.38 micrometers of rain fall around the world, ...
```

The `state:` lines go to stderr, so redirecting stdout gives a clean file. A
watch URL, a `youtu.be` link, a Shorts link or the bare 11-character id all
work. Transcribing uses your minutes: one per started minute of video.

### 3. Save subtitles or another format

```bash
tube transcribe "<url>" --wait --format srt > video.srt
tube transcribe "<url>" --wait --format vtt > video.vtt
tube transcribe "<url>" --wait --format md  > video.md
```

### 4. Start now, fetch later

Without `--wait` the command prints only the project id and returns at once:

```bash
id=$(tube transcribe "<url>")
tube project watch "$id"                 # waits until it is done
tube transcript "$id" --format srt > video.srt
```

`tube transcript` works for any finished project, including ones made in the
web app (uploads, recordings), not only YouTube ones:

```bash
tube project list --limit 5
tube transcript <id> --no-speakers --no-timecodes > plain.txt
```

### 5. Use it in scripts

```bash
# Every video in a list, one .srt each; stop on the first real failure.
while read -r url; do
  id=$(tube transcribe "$url") || exit $?
  tube project watch "$id" && tube transcript "$id" -f srt > "$id.srt"
done < urls.txt
```

```bash
# JSON for other tools (no progress output in --json mode)
tube project list --json | jq -r '.[] | select(.state == "done") | .id'
tube transcript <id> --format json | jq '.transcription[0]'
```

In CI, skip `tube login` and set `TUBE_API_KEY` as a secret instead. The exit
code tells a script what went wrong (see below). To be called back when a
transcription finishes, pass `--webhook https://your.server/hook`.

### 6. Let an AI assistant do it

Add the MCP server (config under *Quick start*), then ask in plain words:

> Get me the transcript of https://youtu.be/zhWDdy_5v2w and summarise it.

The assistant calls `transcribe_youtube`, waits for the result, and works
with the text. `get_transcript` and `list_projects` let it reuse transcripts
you already have instead of spending minutes again.

### Good to know

- A video without speech finishes with an empty transcript; the CLI says so
  on stderr.
- `--wait` gives up after 15 minutes. The transcription keeps running; pick
  it up with `tube project watch <id>`.
- `--lang` is accepted, but the API currently detects the language itself.
- `tube logout` removes the stored key.

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
sh tests/npm-wrapper.sh   # the npm launcher, against a local archive server
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Layout: `src/api.rs` (HTTP client), `src/format.rs` (transcript parsing and
output formats), `src/transcribe.rs` (start and wait), `src/mcp.rs` (MCP
server), `src/cli.rs` (commands), `src/config.rs` (credentials).
