/** Data goes to stdout, everything else to stderr, so pipes stay clean. */
export function out(text: string): void {
  process.stdout.write(text.endsWith('\n') ? text : text + '\n');
}

export function json(value: unknown): void {
  out(JSON.stringify(value, null, 2));
}

/** Progress / status — stderr only, suppressed when not a TTY or in --json. */
export function info(text: string, quiet = false): void {
  if (quiet) return;
  process.stderr.write(text + '\n');
}

export function fail(err: unknown): never {
  const e = err as { message?: string; status?: number };
  process.stderr.write(`error: ${e.message ?? String(err)}\n`);
  process.exit(exitCodeFor(e.status));
}

/** Script-friendly exit codes (see README). */
export function exitCodeFor(status?: number): number {
  switch (status) {
    case 401:
    case 403:
      return 2;
    case 402:
      return 3;
    case 422:
      return 4;
    case 404:
      return 5;
    default:
      return 1;
  }
}
