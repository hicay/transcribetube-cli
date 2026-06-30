# @transcribetube/cli

Transcribe **YouTube videos, podcasts, and audio/video files** to text,
subtitles (`.srt`/`.vtt`), or markdown — from your terminal, your CI pipeline,
or directly from an AI assistant via MCP.

## Quick start

**CLI one-liner** (no install required):

```bash
npx -y @transcribetube/cli transcribe "https://youtu.be/dQw4w9WgXcQ" --wait --format srt > out.srt
```

**Use from Claude / any MCP client** — add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "transcribetube": {
      "command": "npx",
      "args": ["-y", "@transcribetube/cli", "mcp"],
      "env": { "TUBE_API_KEY": "tk_live_xxx" }
    }
  }
}
```

---

## Install & onboard

```bash
npm install -g @transcribetube/cli
tube login          # opens the dashboard, paste your API key, done
```

`tube login` (no flags) walks a new user through it: it opens
`app.transcribetube.com/account/api-key` in the browser to sign up / sign in
and copy the key, prompts you to paste it, then verifies the credential and
shows your remaining credit. Non-interactive setups can skip the prompts:

```bash
tube login --api-key tk_live_xxx     # CI / scripted
```

Already authenticated? Pull or rotate your key from the CLI:

```bash
tube account api-key            # print current key
tube account api-key --create   # generate a new one
export TUBE_API_KEY="$(tube account api-key)"
```

Or stay install-free with `npx -y @transcribetube/cli <command>`.

## Authentication

Order of precedence: `--api-key` flag → `TUBE_API_KEY` env → stored config
(`~/.config/tube/config.json`). API keys are sent as the `api-key` header;
`tube login --token <jwt>` uses a Supabase Bearer token instead.

## Commands

| Command | Description |
|---|---|
| `tube transcribe <url>` | Transcribe a YouTube URL. `--wait` prints the transcript. |
| `tube transcript <id>` | Fetch a finished transcript. `--format txt\|srt\|vtt\|md\|json`. |
| `tube project list` | List your projects (`id`, state, type, name). |
| `tube project get <id>` | Show one project as JSON. |
| `tube project watch <id>` | Poll until `DONE`/`ERROR`. |
| `tube whoami` | Show account + remaining credit. |
| `tube mcp` | Run the MCP server (see below). |

Global flags: `--json` (machine-readable, suppresses progress), `--api-key`,
`--api-url`.

### Output discipline & exit codes

Data is written to **stdout**; progress and logs go to **stderr**, so pipes stay
clean. Exit codes: `0` ok · `2` auth · `3` insufficient credit · `4`
transcription error · `5` not found · `1` other.

## Use it from AI assistants (MCP)

The package ships a [Model Context Protocol](https://modelcontextprotocol.io)
server, so Claude and other MCP clients can transcribe on their own.

**Claude Code one-liner:**

```bash
claude mcp add transcribetube --env TUBE_API_KEY=tk_live_xxx -- npx -y @transcribetube/cli mcp
```

Tools exposed: `transcribe_youtube`, `get_transcript`, `list_projects`.
See [`AI_INTEGRATION.md`](./AI_INTEGRATION.md) for how discovery works.

## Development

```bash
npm install
npm run build
npm test
node bin/run.js whoami
```
