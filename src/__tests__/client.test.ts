import { ApiError, TubeClient, youtubeId } from '../api/client';
import { formatTranscript } from '../formatters';

// ── youtubeId() ──────────────────────────────────────────────────────────────

describe('youtubeId()', () => {
  it('passes through an 11-char video id', () => {
    expect(youtubeId('dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
  });

  it('parses a standard watch URL', () => {
    expect(youtubeId('https://www.youtube.com/watch?v=dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
  });

  it('parses a youtu.be short URL', () => {
    expect(youtubeId('https://youtu.be/dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
  });

  it('parses a Shorts URL', () => {
    expect(youtubeId('https://www.youtube.com/shorts/dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
  });

  it('parses an embed URL', () => {
    expect(youtubeId('https://www.youtube.com/embed/dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
  });

  it('returns undefined for a non-YouTube URL', () => {
    expect(youtubeId('https://vimeo.com/123456')).toBeUndefined();
  });

  it('returns undefined for an empty string', () => {
    expect(youtubeId('')).toBeUndefined();
  });

  it('trims whitespace before matching', () => {
    expect(youtubeId('  dQw4w9WgXcQ  ')).toBe('dQw4w9WgXcQ');
  });
});

// ── formatTranscript() ───────────────────────────────────────────────────────

const PARA = [
  { speaker: 0, start: 0, end: 5, text: 'Hello world.' },
  { speaker: 1, start: 5, end: 10, text: 'Hi there.' },
];

const PROJECT_WITH_TRANSCRIPT = {
  id: 'p1',
  name: 'Test Video',
  state: 'DONE' as const,
  type: 'API',
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
  transcription: PARA,
  speakers: { '0': 'Alice', '1': 'Bob' },
};

describe('formatTranscript()', () => {
  it('produces plain text with timecodes and speaker labels', () => {
    const out = formatTranscript(PROJECT_WITH_TRANSCRIPT, 'txt');
    expect(out).toContain('[00:00:00]');
    expect(out).toContain('Alice:');
    expect(out).toContain('Hello world.');
    expect(out).toContain('Bob:');
  });

  it('omits speakers when speakers=false', () => {
    const out = formatTranscript(PROJECT_WITH_TRANSCRIPT, 'txt', { speakers: false });
    expect(out).not.toContain('Alice');
    expect(out).toContain('Hello world.');
  });

  it('produces valid SRT with sequence numbers and timestamps', () => {
    const out = formatTranscript(PROJECT_WITH_TRANSCRIPT, 'srt');
    expect(out).toMatch(/^1\n/);
    expect(out).toContain('-->');
    expect(out).toContain(',');
  });

  it('produces valid VTT with WEBVTT header', () => {
    const out = formatTranscript(PROJECT_WITH_TRANSCRIPT, 'vtt');
    expect(out.startsWith('WEBVTT')).toBe(true);
    expect(out).toContain('-->');
  });

  it('produces markdown with title and speaker headings', () => {
    const out = formatTranscript(PROJECT_WITH_TRANSCRIPT, 'md');
    expect(out).toContain('# Test Video');
    expect(out).toContain('**Alice**');
  });

  it('produces parseable JSON', () => {
    const raw = formatTranscript(PROJECT_WITH_TRANSCRIPT, 'json');
    const parsed = JSON.parse(raw);
    expect(parsed).toHaveProperty('transcription');
    expect(parsed.transcription).toHaveLength(2);
  });
});

// ── TubeClient — response unwrap + error mapping ─────────────────────────────

function makeFetch(status: number, body: unknown) {
  return jest.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  });
}

describe('TubeClient', () => {
  const cfg = { apiUrl: 'https://api.example.com', apiKey: 'test-key' };

  afterEach(() => jest.resetAllMocks());

  it('unwraps { result } envelope', async () => {
    global.fetch = makeFetch(200, { result: { email: 'a@b.com', id: '123' } }) as any;
    const client = new TubeClient(cfg);
    const me = await client.me();
    expect(me).toEqual({ email: 'a@b.com', id: '123' });
  });

  it('returns raw object when no result wrapper', async () => {
    global.fetch = makeFetch(200, { email: 'a@b.com', id: '123' }) as any;
    const client = new TubeClient(cfg);
    const me = await client.me();
    expect(me).toEqual({ email: 'a@b.com', id: '123' });
  });

  it('throws ApiError on 401', async () => {
    global.fetch = makeFetch(401, { message: 'Unauthorized' }) as any;
    const client = new TubeClient(cfg);
    await expect(client.me()).rejects.toBeInstanceOf(ApiError);
    await expect(client.me()).rejects.toMatchObject({ status: 401 });
  });

  it('uses api-key header when apiKey is set', async () => {
    global.fetch = makeFetch(200, { result: { email: 'x@y.com', id: '1' } }) as any;
    const client = new TubeClient(cfg);
    await client.me();
    const [, init] = (global.fetch as jest.Mock).mock.calls[0];
    expect(init.headers['api-key']).toBe('test-key');
    expect(init.headers['authorization']).toBeUndefined();
  });

  it('sends youtubeVideoId (not youtubeId) to transcribeVideo', async () => {
    global.fetch = makeFetch(200, { result: { id: 'proj1', state: 'UPLOAD' } }) as any;
    const client = new TubeClient(cfg);
    await client.transcribeYoutube({ youtubeVideoId: 'dQw4w9WgXcQ' });
    const [, init] = (global.fetch as jest.Mock).mock.calls[0];
    const sent = JSON.parse(init.body);
    expect(sent).toHaveProperty('youtubeVideoId', 'dQw4w9WgXcQ');
    expect(sent).not.toHaveProperty('youtubeId');
  });

  it('getApiKey returns { apiKey }', async () => {
    global.fetch = makeFetch(200, { result: { apiKey: 'secret-key' } }) as any;
    const client = new TubeClient(cfg);
    const res = await client.getApiKey();
    expect(res).toHaveProperty('apiKey', 'secret-key');
  });

  it('currentPlan returns { currentPlan, availableCredit, totalCredit }', async () => {
    global.fetch = makeFetch(200, {
      result: { currentPlan: 'Pro', availableCredit: 300, totalCredit: 600 },
    }) as any;
    const client = new TubeClient(cfg);
    const plan = await client.currentPlan();
    expect(plan).toMatchObject({ currentPlan: 'Pro', availableCredit: 300, totalCredit: 600 });
  });
});
