import { Command } from 'commander';

import { TubeClient } from './api/client';
import { loadConfig, saveConfig } from './config';
import { formatTranscript, TranscriptFormat } from './formatters';
import { transcribeYoutube, waitForProject } from './core/transcribe';
import { fail, info, json, out } from './util/output';
import { openBrowser, prompt } from './util/prompt';

export function buildProgram(): Command {
  const program = new Command();

  program
    .name('tube')
    .description(
      'TranscribeTube CLI — transcribe YouTube videos, podcasts and media files.',
    )
    .version('0.1.0')
    .option('--api-key <key>', 'API key (overrides config / env)')
    .option('--json', 'machine-readable JSON output')
    .option('--api-url <url>', 'override API base URL');

  // Build a client honoring global flags.
  const clientFromOpts = (): TubeClient => {
    const g = program.opts();
    const cfg = loadConfig();
    if (g.apiKey) cfg.apiKey = g.apiKey;
    if (g.apiUrl) cfg.apiUrl = g.apiUrl;
    return new TubeClient(cfg);
  };
  const isJson = (): boolean => Boolean(program.opts().json);

  // ---- auth -------------------------------------------------------------
  program
    .command('login')
    .description('Sign in. Opens the dashboard to grab your API key, or use flags.')
    .option('--api-key <key>', 'store an API key non-interactively (CI / MCP)')
    .option('--token <jwt>', 'store a Supabase JWT non-interactively')
    .action(async (opts) => {
      const cfg = loadConfig();

      // Non-interactive path: flags or piped env, just save + verify.
      let apiKey: string | undefined = opts.apiKey;
      const token: string | undefined = opts.token;

      // Interactive onboarding: walk a new user to their key.
      if (!apiKey && !token) {
        const keyUrl = `${cfg.dashboardUrl}/account/api-key`;
        info('Welcome to TranscribeTube 👋');
        info('');
        info('1. Sign in or create an account, then copy your API key from:');
        info(`   ${keyUrl}`);
        info('   (Opening it in your browser…)');
        openBrowser(keyUrl);
        info('');
        apiKey = await prompt('2. Paste your API key: ', { mask: true });
        if (!apiKey) fail(new Error('No API key entered.'));
      }

      saveConfig({ apiKey, token });

      // Verify the credential actually works before declaring success.
      try {
        const client = clientFromOpts();
        const me = await client.me();
        const plan = await client.currentPlan().catch(() => undefined);
        info(`✓ Logged in as ${me.email}`);
        if (plan) info(`  credit available: ${plan.availableCredit ?? 'n/a'}`);
        info('');
        info('Try it:  tube transcribe "https://youtu.be/<id>" --wait');
      } catch (e) {
        saveConfig({ apiKey: undefined, token: undefined });
        fail(e);
      }
    });

  program
    .command('logout')
    .description('Remove stored credentials.')
    .action(() => {
      saveConfig({ apiKey: undefined, token: undefined });
      info('Logged out.');
    });

  program
    .command('whoami')
    .description('Show the authenticated account and remaining credit.')
    .action(async () => {
      const client = clientFromOpts();
      try {
        const me = await client.me();
        const plan = await client.currentPlan().catch(() => undefined);
        if (isJson()) return json({ ...me, plan });
        out(`${me.email}`);
        if (plan) info(`credit: ${plan.availableCredit ?? 'n/a'}`);
      } catch (e) {
        fail(e);
      }
    });

  // ---- account ----------------------------------------------------------
  const account = program
    .command('account')
    .description('Account and API key management.');

  account
    .command('api-key')
    .description('Show your API key, or mint a new one with --create.')
    .option('--create', 'generate (rotate) a new API key')
    .action(async (opts) => {
      const client = clientFromOpts();
      try {
        const { apiKey } = opts.create
          ? await client.createApiKey()
          : await client.getApiKey();
        if (!apiKey) fail(new Error('No API key on this account yet. Run `tube account api-key --create` to generate one.'));
        if (isJson()) return json({ apiKey });
        out(apiKey); // stdout only, so `export TUBE_API_KEY=$(tube account api-key)` works
        if (opts.create) info('New API key generated.');
      } catch (e) {
        fail(e);
      }
    });

  // ---- transcribe (the headline command) --------------------------------
  program
    .command('transcribe <url>')
    .description('Transcribe a YouTube URL. Prints the transcript with --wait.')
    .option('-l, --lang <code>', 'language code (default: auto-detect)')
    .option('-f, --format <fmt>', 'txt|srt|vtt|md|json', 'txt')
    .option('-w, --wait', 'wait for completion and print the transcript')
    .option('--webhook <url>', 'POST to this URL when transcription completes')
    .action(async (url, opts) => {
      const client = clientFromOpts();
      const quiet = isJson();
      try {
        const project = await transcribeYoutube(client, url, {
          language: opts.lang,
          webhookUrl: opts.webhook,
          wait: opts.wait,
          onState: (s) => info(`state: ${s}`, quiet),
        });

        if (!opts.wait) {
          if (isJson()) return json(project);
          out(project.id);
          info('Submitted. Track with: tube project watch ' + project.id, quiet);
          return;
        }
        if (isJson()) return json(project);
        out(
          formatTranscript(project as any, opts.format as TranscriptFormat),
        );
      } catch (e) {
        fail(e);
      }
    });

  // ---- project ----------------------------------------------------------
  const project = program.command('project').description('Manage projects.');

  project
    .command('list')
    .description('List your projects.')
    .option('--limit <n>', 'max items', (v) => parseInt(v, 10), 20)
    .action(async (opts) => {
      const client = clientFromOpts();
      try {
        const projects = await client.listProjects({ limit: opts.limit });
        if (isJson()) return json(projects);
        for (const p of projects) {
          out(`${p.id}\t${p.state}\t${p.type}\t${p.name ?? ''}`);
        }
      } catch (e) {
        fail(e);
      }
    });

  project
    .command('get <id>')
    .description('Show a project.')
    .action(async (id) => {
      const client = clientFromOpts();
      try {
        json(await client.getProject(id));
      } catch (e) {
        fail(e);
      }
    });

  project
    .command('watch <id>')
    .description('Poll a project until it is DONE or ERROR.')
    .action(async (id) => {
      const client = clientFromOpts();
      try {
        const done = await waitForProject(client, id, {
          onState: (s) => info(`state: ${s}`, isJson()),
        });
        if (isJson()) return json(done);
        out(`DONE ${done.id}`);
      } catch (e) {
        fail(e);
      }
    });

  // ---- transcript -------------------------------------------------------
  program
    .command('transcript <id>')
    .description('Fetch a finished transcript in a chosen format.')
    .option('-f, --format <fmt>', 'txt|srt|vtt|md|json', 'txt')
    .option('--no-speakers', 'omit speaker labels')
    .option('--no-timecodes', 'omit timecodes')
    .action(async (id, opts) => {
      const client = clientFromOpts();
      try {
        const p = await client.getProject(id);
        out(
          formatTranscript(p, opts.format as TranscriptFormat, {
            speakers: opts.speakers,
            timecodes: opts.timecodes,
          }),
        );
      } catch (e) {
        fail(e);
      }
    });

  // ---- mcp ---------------------------------------------------------------
  program
    .command('mcp')
    .description('Run the Model Context Protocol server (stdio) for AI clients.')
    .action(async () => {
      // Lazy import so normal CLI usage never loads the MCP SDK.
      const { buildServer } = await import('./mcp/server');
      const { StdioServerTransport } = await import(
        '@modelcontextprotocol/sdk/server/stdio.js'
      );
      await buildServer(clientFromOpts()).connect(new StdioServerTransport());
      info('transcribetube MCP server ready (stdio).');
    });

  return program;
}

if (require.main === module) {
  buildProgram().parseAsync(process.argv);
}
