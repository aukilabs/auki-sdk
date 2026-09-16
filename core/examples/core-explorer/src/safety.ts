export function endpoint(value: string): string {
  const url = new URL(value);
  if (url.username || url.password || url.search || url.hash) throw new Error('Use a base URL without credentials, query or fragment.');
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && isLoopback(url))) throw new Error('HTTPS is required outside loopback.');
  return url.href.replace(/\/$/, '');
}
export function isLoopback(url: URL): boolean {
  return ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
}
export function uuid(value: string): string {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value.trim())) throw new Error('Enter a complete UUID.');
  return value.trim().toLowerCase();
}
const credentialLabel = /password|secret|token|credential|authorization|cookie|private.?key|api.?key/i;
// Keep escaped quoted values intact; consume an unquoted Bearer scheme and token together.
const credentialValue = /(["']?\b([\w.-]+)["']?\s*[:=]\s*)("(?:\\[\s\S]|[^"\\])*(?:"|\\?$)|'(?:\\[\s\S]|[^'\\])*(?:'|\\?$)|Bearer\s+[^\s"'<>]+|[^\s,;]+)/gi;
export function redact(value: unknown, depth = 0): unknown {
  if (depth > 20) return '[depth limit]';
  if (typeof value === 'string') return value
    .replace(credentialValue, (match, prefix: string, key: string) => credentialLabel.test(key) ? `${prefix}[redacted]` : match)
    .replace(/\bBearer\s+[^\s"'<>]+/gi, 'Bearer [redacted]')
    .replace(/\b[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, '[redacted token]');
  if (Array.isArray(value)) return value.map(item => redact(item, depth + 1));
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) =>
    [key, credentialLabel.test(key) ? '[redacted]' : redact(item, depth + 1)]));
  return value;
}
// Parse the full SDK-bounded buffer: truncation must never bypass JSON redaction.
export function previewBytes(bytes: Uint8Array): string {
  const decoded = new TextDecoder().decode(bytes);
  let preview: string;
  try { preview = inspect(JSON.parse(decoded)); }
  catch {
    if (/^\s*[{["']/.test(decoded)) return 'Preview withheld — malformed or incomplete JSON.';
    preview = String(redact(decoded));
  }
  const encoded = new TextEncoder().encode(preview);
  if (encoded.length <= 65536) return preview;
  const label = 'Truncated to 64 KiB.\n';
  // Streaming decode omits an incomplete trailing UTF-8 character.
  return label + new TextDecoder().decode(encoded.subarray(0, 65536 - label.length), { stream: true });
}
export function inspect(value: unknown): string { return JSON.stringify(redact(value), null, 2); }
export function safeError(error: unknown): string {
  const e = error as { status?: number; kind?: string };
  if (e?.status === 401 || e?.status === 403) return 'Denied — this read is not authorized.';
  if (e?.kind === 'limit') return 'Limit exceeded — buffered reads allow 8 MiB.';
  if (e?.kind === 'timeout') return 'Timed out — retry this read.';
  return 'Error — request failed. Check configuration and permissions, then retry.';
}
// Each UI lane owns its pending read. Invalidation fences late results even if
// the underlying operation completes at the same moment as cancellation.
export class ReadLane {
  private controller?: AbortController;
  private generation = 0;
  cancel() { this.generation++; this.controller?.abort(); }
  async run<T>(read: (signal: AbortSignal) => Promise<T>, success: (value: T) => void, failure: (error: unknown) => void, timeout = 15_000) {
    this.cancel();
    const generation = this.generation;
    const controller = this.controller = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => { timedOut = true; controller.abort(); }, timeout);
    try {
      const value = await read(controller.signal);
      if (generation === this.generation) {
        if (timedOut) failure({ kind: 'timeout' }); else success(value);
      }
    } catch (error) {
      if (generation === this.generation) failure(timedOut ? { kind: 'timeout' } : error);
    } finally { clearTimeout(timer); }
  }
}
