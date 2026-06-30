import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

export interface TubeConfig {
  apiUrl: string;
  /** Web dashboard base URL, used by `tube login` onboarding. */
  dashboardUrl?: string;
  /** Long-lived API key (header: `api-key`). Preferred for CI / MCP. */
  apiKey?: string;
  /** Supabase JWT obtained via `tube login`. */
  token?: string;
  /** Default transcription language, ISO code or "auto". */
  defaultLanguage?: string;
}

const DEFAULT_API_URL = 'https://api.transcribetube.com';
const DEFAULT_DASHBOARD_URL = 'https://app.transcribetube.com';

export function configPath(): string {
  const base =
    process.env.XDG_CONFIG_HOME || join(homedir(), '.config');
  return join(base, 'tube', 'config.json');
}

export function loadConfig(): TubeConfig {
  const cfg: TubeConfig = {
    apiUrl: process.env.TUBE_API_URL || DEFAULT_API_URL,
    dashboardUrl: process.env.TUBE_DASHBOARD_URL || DEFAULT_DASHBOARD_URL,
    defaultLanguage: 'auto',
  };

  const path = configPath();
  if (existsSync(path)) {
    try {
      Object.assign(cfg, JSON.parse(readFileSync(path, 'utf8')));
    } catch {
      // Corrupt config file is non-fatal; env/flags can still drive auth.
    }
  }

  // Environment always wins over the stored file so CI stays stateless.
  if (process.env.TUBE_API_KEY) cfg.apiKey = process.env.TUBE_API_KEY;
  if (process.env.TUBE_TOKEN) cfg.token = process.env.TUBE_TOKEN;
  if (process.env.TUBE_API_URL) cfg.apiUrl = process.env.TUBE_API_URL;

  return cfg;
}

export function saveConfig(patch: Partial<TubeConfig>): TubeConfig {
  const current = loadFileOnly();
  const next = { ...current, ...patch };
  const path = configPath();
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(next, null, 2) + '\n', { mode: 0o600 });
  return next;
}

function loadFileOnly(): TubeConfig {
  const path = configPath();
  if (!existsSync(path)) return { apiUrl: DEFAULT_API_URL };
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch {
    return { apiUrl: DEFAULT_API_URL };
  }
}
