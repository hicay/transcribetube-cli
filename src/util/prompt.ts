import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

/** Best-effort open a URL in the user's default browser. Never throws. */
export function openBrowser(url: string): void {
  const cmd =
    process.platform === 'darwin'
      ? 'open'
      : process.platform === 'win32'
        ? 'cmd'
        : 'xdg-open';
  const args = process.platform === 'win32' ? ['/c', 'start', '', url] : [url];
  try {
    const child = spawn(cmd, args, { stdio: 'ignore', detached: true });
    child.on('error', () => undefined);
    child.unref();
  } catch {
    // Headless / no browser — the caller already printed the URL.
  }
}

/**
 * Prompt on stderr and read a line from stdin. When `mask` is set the typed
 * characters are not echoed (for secrets like API keys).
 */
export function prompt(
  question: string,
  opts: { mask?: boolean } = {},
): Promise<string> {
  return new Promise((resolve) => {
    const rl = createInterface({
      input: process.stdin,
      output: process.stderr,
      terminal: true,
    });

    if (opts.mask) {
      const out = process.stderr;
      // Swallow echo while the user types the secret.
      (rl as any)._writeToOutput = (s: string) => {
        if (s.includes('\n') || s.includes(question)) out.write(s);
      };
    }

    rl.question(question, (answer) => {
      if (opts.mask) process.stderr.write('\n');
      rl.close();
      resolve(answer.trim());
    });
  });
}
