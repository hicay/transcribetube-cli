import {
  ApiError,
  Project,
  ProjectState,
  ProjectWithTranscript,
  TubeClient,
  youtubeId,
} from '../api/client';

export interface TranscribeOptions {
  language?: string;
  webhookUrl?: string;
  /** Poll until the project reaches DONE/ERROR. */
  wait?: boolean;
  /** Max time to wait, ms. Default 15 min. */
  timeoutMs?: number;
  /** Progress callback for CLI spinners / logging. */
  onState?: (state: ProjectState) => void;
}

/**
 * Start a YouTube transcription and, when `wait` is set, poll until it
 * finishes. Shared by the CLI command and the MCP tool so both behave
 * identically.
 */
export async function transcribeYoutube(
  client: TubeClient,
  url: string,
  opts: TranscribeOptions = {},
): Promise<Project | ProjectWithTranscript> {
  const id = youtubeId(url);
  if (!id) {
    throw new Error(`Not a recognizable YouTube URL or video id: "${url}"`);
  }

  const project = await client.transcribeYoutube({
    youtubeVideoId: id,
    language: opts.language && opts.language !== 'auto' ? opts.language : undefined,
    webhookUrl: opts.webhookUrl,
  });

  if (!opts.wait) return project;
  return waitForProject(client, project.id, opts);
}

export async function waitForProject(
  client: TubeClient,
  projectId: string,
  opts: Pick<TranscribeOptions, 'timeoutMs' | 'onState'> = {},
): Promise<ProjectWithTranscript> {
  const deadline = Date.now() + (opts.timeoutMs ?? 15 * 60_000);
  let delay = 2_000;
  let lastState: ProjectState | undefined;

  while (Date.now() < deadline) {
    const project = await client.getProject(projectId);
    if (project.state !== lastState) {
      lastState = project.state;
      opts.onState?.(project.state);
    }
    if (project.state === 'DONE') return project;
    if (project.state === 'ERROR') {
      throw new ApiError('Transcription failed (state=ERROR).', 422, project);
    }
    await sleep(delay);
    delay = Math.min(delay * 1.5, 15_000); // capped backoff
  }
  throw new ApiError('Timed out waiting for transcription.', 408);
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
