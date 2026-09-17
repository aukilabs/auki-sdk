import { endpoint, inspect, isLoopback, redact } from './safety.ts';

export function shortPeerId(value: string): string {
  const safe = String(redact(value));
  return safe.length > 24 ? `${safe.slice(0, 12)}…${safe.slice(-8)}` : safe;
}

// Only render supplied fields. No inferred schema, links, or provider HTML.
export function facts(values: Record<string, unknown>): HTMLDListElement {
  const list = document.createElement('dl');
  list.className = 'facts';
  for (const [key, value] of Object.entries(values)) {
    if (value === undefined || value === null) continue;
    const row = document.createElement('div');
    const term = document.createElement('dt'), description = document.createElement('dd');
    term.textContent = key;
    description.textContent = typeof value === 'object' ? inspect(value) : String(redact(value));
    row.append(term, description); list.append(row);
  }
  return list;
}

export function technical(value: unknown): HTMLDetailsElement {
  const details = document.createElement('details'), summary = document.createElement('summary'), pre = document.createElement('pre');
  summary.textContent = 'Details · JSON'; pre.textContent = inspect(value);
  summary.onclick = event => {
    event.preventDefault();
    details.dispatchEvent(new CustomEvent('explorer:technical', { bubbles: true, detail: redact(value) }));
  };
  details.append(summary, pre); return details;
}

/** A shared, inert list row. Event ownership remains with the caller. */
export function recordRow(name: unknown, description: unknown): HTMLButtonElement {
  const button = document.createElement('button');
  const title = document.createElement('strong'), detail = document.createElement('small');
  button.type = 'button'; button.className = 'record';
  title.textContent = String(redact(name)); detail.textContent = String(redact(description));
  button.append(title, detail);
  return button;
}

/** Describe only the configured bases; this does not probe service availability. */
export function environmentLabel(values: readonly string[]): string {
  const urls = values.map(endpoint);
  if (urls.length !== 3) throw new Error('API, DDS and DMS are required.');
  if (urls.every(url => isLoopback(new URL(url)))) return 'Local fixtures / synthetic test data';
  const dev = ['https://api.dev.aukiverse.com', 'https://dds.dev.aukiverse.com', 'https://dms.dev.aukiverse.com/v1'];
  return urls.every((url, index) => url === dev[index]) ? 'Dev' : 'Custom / mixed environment';
}
