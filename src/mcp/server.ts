/**
 * TranscribeTube MCP server.
 *
 * Exposes transcription as Model Context Protocol tools so MCP-capable
 * assistants (Claude Desktop, Claude Code, Cursor, …) can transcribe a
 * YouTube video natively — the user asks in natural language and the model
 * calls `transcribe_youtube` instead of guessing at scraping code.
 *
 * Run:  npx -y @transcribetube/cli mcp     (needs TUBE_API_KEY)
 */
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { z } from 'zod';

import { TubeClient } from '../api/client';
import { loadConfig } from '../config';
import { formatTranscript, TranscriptFormat } from '../formatters';
import { transcribeYoutube } from '../core/transcribe';

const FORMATS = ['txt', 'srt', 'vtt', 'md', 'json'] as const;

export function buildServer(client = new TubeClient(loadConfig())): McpServer {
  const server = new McpServer({
    name: 'transcribetube',
    version: '0.1.0',
  });

  // The SDK's generic inference over zod shapes trips TS2589 (excessively deep)
  // on this toolchain; bind to a looser signature and keep handler args typed.
  const tool = server.tool.bind(server) as (
    name: string,
    description: string,
    schema: Record<string, unknown>,
    handler: (args: any) => Promise<{ content: { type: 'text'; text: string }[] }>,
  ) => void;

  tool(
    'transcribe_youtube',
    'Transcribe a YouTube video to text. Use this whenever the user wants the ' +
      'transcript, captions, subtitles, or spoken content of a YouTube URL. ' +
      'Returns speaker-labeled, timecoded text. Waits for processing to finish.',
    {
      url: z.string().describe('YouTube URL or 11-character video id'),
      language: z
        .string()
        .optional()
        .describe('ISO language code, e.g. "en". Omit to auto-detect.'),
      format: z
        .enum(FORMATS)
        .default('txt')
        .describe('Output format: txt, srt, vtt, md, or json.'),
    },
    async ({ url, language, format }) => {
      const project = await transcribeYoutube(client, url, {
        language,
        wait: true,
        timeoutMs: 10 * 60_000,
      });
      const text = formatTranscript(
        project as any,
        format as TranscriptFormat,
      );
      return { content: [{ type: 'text', text }] };
    },
  );

  tool(
    'get_transcript',
    'Fetch the transcript of an existing TranscribeTube project by id, in a ' +
      'chosen format. Use after a transcription has already been created.',
    {
      projectId: z.string(),
      format: z.enum(FORMATS).default('txt'),
    },
    async ({ projectId, format }) => {
      const project = await client.getProject(projectId);
      const text = formatTranscript(project, format as TranscriptFormat);
      return { content: [{ type: 'text', text }] };
    },
  );

  tool(
    'list_projects',
    'List the user\'s recent TranscribeTube transcription projects with their ' +
      'ids and status (UPLOAD, TRANSCRIPTION, DONE, ERROR).',
    { limit: z.number().int().positive().max(100).default(20) },
    async ({ limit }) => {
      const projects = await client.listProjects({ limit });
      return {
        content: [{ type: 'text', text: JSON.stringify(projects, null, 2) }],
      };
    },
  );

  return server;
}

async function main(): Promise<void> {
  const client = new TubeClient(loadConfig());
  if (!client.authenticated) {
    process.stderr.write(
      'transcribetube-mcp: no credentials. Set TUBE_API_KEY in the MCP server env.\n',
    );
  }
  const server = buildServer(client);
  await server.connect(new StdioServerTransport());
  process.stderr.write('transcribetube MCP server ready (stdio).\n');
}

if (require.main === module) {
  main().catch((e) => {
    process.stderr.write(`fatal: ${(e as Error).message}\n`);
    process.exit(1);
  });
}
