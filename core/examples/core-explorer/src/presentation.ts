import { inspect, redact } from './safety.ts';

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
  summary.textContent = 'Technical details · JSON'; pre.textContent = inspect(value);
  details.append(summary, pre); return details;
}
