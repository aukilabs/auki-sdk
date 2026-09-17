import type { AukiDomainData, DataMetadata } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import { previewBytes, uuid } from './safety.ts';
export type RecordContext = { domainId: string; data: Pick<AukiDomainData, 'list' | 'get' | 'readTo'> };
/** Owns only reads, never the shared Domain client. close aborts and drains every read. */
export class JobsRecords {
  private epoch = 0;
  private pending = new Set<Promise<unknown>>();
  private aborts = new Set<AbortController>();
  private context: () => RecordContext | undefined;
  constructor(context: () => RecordContext | undefined) { this.context = context; }
  private run<T>(work: (context: RecordContext, signal: AbortSignal) => Promise<T>): Promise<T | undefined> {
    const context = this.context(), epoch = this.epoch;
    if (!context) return Promise.resolve(undefined);
    const abort = new AbortController(); this.aborts.add(abort);
    const timer = setTimeout(() => abort.abort(), 15000);
    const pending = (async () => {
      const value = await work(context, abort.signal);
      const now = this.context();
      if (!abort.signal.aborted && epoch === this.epoch && now?.data === context.data && now.domainId === context.domainId) return value;
    })().finally(() => { clearTimeout(timer); this.aborts.delete(abort); this.pending.delete(pending); });
    this.pending.add(pending); return pending;
  }
  list(name = '') { return this.run(async (context, signal) => {
    const records = await context.data.list(name ? { name } : {}, signal);
    return records.filter(record => record.domain_id.toLowerCase() === context.domainId.toLowerCase()).slice(0, 100);
  }); }
  preview(id: string): Promise<{ record: DataMetadata; text: string } | undefined> {
    return this.run(async (context, signal) => {
      const key = uuid(id), record = await context.data.get(key, signal);
      if (record.id.toLowerCase() !== key || record.domain_id.toLowerCase() !== context.domainId.toLowerCase() || !Number.isSafeInteger(record.size) || record.size < 0 || record.size > 65536) throw Error('Invalid record');
      if (signal.aborted) return Promise.reject(Error('Cancelled'));
      const bytes = new Uint8Array(record.size); let length = 0;
      const received = await context.data.readTo(key, chunk => {
        if (signal.aborted || length + chunk.length > bytes.length) throw Error('Invalid size');
        bytes.set(chunk, length); length += chunk.length;
      }, { maxBytes: 65536, maxChunkBytes: 65536 }, signal);
      if (received !== length || length !== record.size) throw Error('Changed size');
      return { record, text: previewBytes(bytes) };
    });
  }
  async close() { this.epoch++; for (const abort of this.aborts) abort.abort(); await Promise.allSettled([...this.pending]); }
}
