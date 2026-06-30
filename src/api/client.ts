import { loadConfig, TubeConfig } from '../config';

/** Project lifecycle states, mirrors core/domain Project.entity ProjectState. */
export type ProjectState = 'UPLOAD' | 'TRANSCRIPTION' | 'DONE' | 'ERROR';

export interface Project {
  id: string;
  name: string;
  state: ProjectState;
  type: string;
  language?: string;
  sourceUrl?: string;
  videoId?: string;
  createdAt: string;
  updatedAt: string;
}

export interface TranscriptParagraph {
  speaker?: number | string;
  start: number;
  end: number;
  text: string;
}

export interface ProjectWithTranscript extends Project {
  transcription: TranscriptParagraph[];
  speakers: Record<string, string>;
}

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly body?: unknown,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

/**
 * Thin HTTP client over the TranscribeTube REST API. It injects whichever
 * credential is configured: an `api-key` header (CI / MCP) or a Bearer JWT
 * (interactive `tube login`).
 */
export class TubeClient {
  private readonly cfg: TubeConfig;

  constructor(cfg: TubeConfig = loadConfig()) {
    this.cfg = cfg;
  }

  get authenticated(): boolean {
    return Boolean(this.cfg.apiKey || this.cfg.token);
  }

  private headers(extra: Record<string, string> = {}): Record<string, string> {
    const h: Record<string, string> = {
      accept: 'application/json',
      ...extra,
    };
    if (this.cfg.apiKey) h['api-key'] = this.cfg.apiKey;
    else if (this.cfg.token) h['authorization'] = `Bearer ${this.cfg.token}`;
    return h;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const url = `${this.cfg.apiUrl}${path}`;
    const res = await fetch(url, {
      method,
      headers: this.headers(
        body ? { 'content-type': 'application/json' } : {},
      ),
      body: body ? JSON.stringify(body) : undefined,
    });

    const text = await res.text();
    const parsed = text ? safeJson(text) : undefined;

    if (!res.ok) {
      throw new ApiError(
        messageFor(res.status, parsed),
        res.status,
        parsed,
      );
    }
    // The API wraps payloads in ApiResponseModel<T> ({ result }); unwrap it.
    return (parsed && (parsed as any).result !== undefined
      ? (parsed as any).result
      : parsed) as T;
  }

  me(): Promise<{ email: string; id: string }> {
    return this.request('GET', '/account/me');
  }

  currentPlan(): Promise<{ currentPlan: string; availableCredit: number; totalCredit: number }> {
    return this.request('GET', '/account/get-current-plan');
  }

  /** Fetch the account's existing API key (requires JWT auth). */
  getApiKey(): Promise<{ apiKey: string | null }> {
    return this.request('GET', '/account/api-key');
  }

  /** Generate (rotate) the account's API key (requires JWT auth). */
  createApiKey(): Promise<{ apiKey: string }> {
    return this.request('POST', '/account/api-key');
  }

  listProjects(opts: { limit?: number } = {}): Promise<Project[]> {
    const q = new URLSearchParams();
    if (opts.limit) q.set('limit', String(opts.limit));
    const qs = q.toString();
    return this.request('GET', `/api/list${qs ? `?${qs}` : ''}`);
  }

  getProject(id: string): Promise<ProjectWithTranscript> {
    return this.request('GET', `/api/detail/${encodeURIComponent(id)}`);
  }

  transcribeYoutube(input: {
    youtubeVideoId: string;
    language?: string;
    webhookUrl?: string;
  }): Promise<Project> {
    return this.request('POST', '/api/transcribeVideo', input);
  }
}

/** Resolve a YouTube watch URL or bare id into the 11-char video id. */
export function youtubeId(input: string): string | undefined {
  const trimmed = input.trim();
  if (/^[\w-]{11}$/.test(trimmed)) return trimmed;
  const m = trimmed.match(
    /(?:youtu\.be\/|v=|\/shorts\/|\/embed\/)([\w-]{11})/,
  );
  return m?.[1];
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function messageFor(status: number, body: unknown): string {
  const apiMsg =
    body && typeof body === 'object'
      ? (body as any).message || (body as any).error
      : undefined;
  switch (status) {
    case 401:
    case 403:
      return apiMsg || 'Authentication failed — run `tube login` or set TUBE_API_KEY.';
    case 402:
      return apiMsg || 'Insufficient credit for this transcription.';
    case 404:
      return apiMsg || 'Not found.';
    default:
      return apiMsg || `Request failed with status ${status}.`;
  }
}
