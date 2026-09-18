import type { AukiDomainData, DataMetadata } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';

export const DEFAULT_UPLOAD_TYPE = 'core-explorer.file.v1';
export const MAX_UPLOAD_BYTES = 8 * 1024 * 1024;
export type UploadContext = { domainId: string; domainName: string; environment: string };
export type UploadClient = Pick<AukiDomainData, 'write' | 'get'>;
export type UploadBinding = UploadContext & { data: UploadClient };
export type UploadFile = Pick<File, 'name' | 'size' | 'arrayBuffer'>;
export type UploadReview = Readonly<UploadContext & { target: string; fileName: string; size: number; dataType: string }>;
export type UploadState = {
  step: 'choose' | 'review' | 'sending' | 'result';
  message: string;
  review?: UploadReview;
  outcome?: 'verified' | 'uncertain' | 'denied' | 'collision' | 'cancelled' | 'error';
  metadata?: DataMetadata;
  recordId?: string;
};

export class UploadValidationError extends Error {}

export function validateUpload(file: UploadFile, dataType: string): void {
  if (!Number.isSafeInteger(file.size) || file.size < 1 || file.size > MAX_UPLOAD_BYTES) {
    throw new UploadValidationError('Choose a nonempty file no larger than 8 MiB (8,388,608 bytes).');
  }
  if (!dataType.trim() || new TextEncoder().encode(dataType).length > 128 || /[\u0000-\u001f\u007f-\u009f]/u.test(dataType)) {
    throw new UploadValidationError('Data type must contain 1–128 UTF-8 bytes without control characters.');
  }
  // Match the existing SDK buffered named-write and Domain Server contract.
  if ([...dataType].some(c => ';!?<>[]{}()/\\"$#@*^&|~%=+'.includes(c))) {
    throw new UploadValidationError('Named uploads reject punctuation including / in data types. Use a type such as core-explorer.file.v1.');
  }
}

const uncertain = 'Completion uncertain. A server record may exist. Cancellation does not roll back a write. Check the saved target in this Domain before uploading again.';

/** One explicit buffered write per review; retain ownership until all work settles. */
export class UploadController {
  state: UploadState = { step: 'choose', message: 'Choose one file, then review its destination.' };
  private generation = 0;
  private reviewed?: { file: UploadFile; binding: UploadBinding; review: UploadReview };
  private operation?: { controller: AbortController; sent: boolean; generation: number };
  private pending?: Promise<void>;
  private getBinding: () => UploadBinding | undefined;
  private changed: () => void;
  private uploaded: () => void;
  private uniqueId: () => string;
  private timeout: number;

  constructor(getBinding: () => UploadBinding | undefined, changed: () => void = () => {}, uploaded: () => void = () => {}, uniqueId: () => string = () => crypto.randomUUID(), timeout = 30_000) {
    this.getBinding = getBinding; this.changed = changed; this.uploaded = uploaded;
    this.uniqueId = uniqueId; this.timeout = timeout;
  }

  get busy(): boolean { return !!this.pending; }

  review(file: UploadFile, dataType: string): UploadReview {
    if (this.busy) throw new UploadValidationError('Wait for cancellation and pending work to settle.');
    this.reviewed = undefined;
    this.state = { step: 'choose', message: 'Choose one file, then review its destination.' };
    validateUpload(file, dataType);
    const binding = this.getBinding();
    if (!binding?.domainId || !binding.environment) throw new UploadValidationError('Select a connected Domain before uploading.');
    const id = this.uniqueId();
    if (!/^[a-f0-9-]{36}$/i.test(id)) throw new UploadValidationError('Unable to generate a unique target.');
    const review = Object.freeze({ domainId: binding.domainId, domainName: binding.domainName, environment: binding.environment,
      target: `core-explorer-${id}`, fileName: file.name, size: file.size, dataType });
    this.reviewed = { file, binding: { ...binding }, review };
    this.state = { step: 'review', review, message: 'Confirm this exact destination to send one buffered upload. Read access does not imply write permission.' };
    this.changed();
    return review;
  }

  private matches(binding: UploadBinding): boolean {
    const current = this.getBinding();
    return !!current && current.data === binding.data && current.domainId === binding.domainId && current.environment === binding.environment;
  }

  confirm(): Promise<void> {
    if (this.pending) return this.pending;
    const reviewed = this.reviewed;
    if (!reviewed || this.state.step !== 'review') return Promise.resolve();
    this.reviewed = undefined; // A confirmed review cannot be retried or double-submitted.
    if (!this.matches(reviewed.binding)) {
      this.state = { step: 'result', review: reviewed.review, outcome: 'cancelled', message: 'Destination changed. Nothing was sent. Review again in the selected Domain.' };
      this.changed(); return Promise.resolve();
    }
    const operation = { controller: new AbortController(), sent: false, generation: ++this.generation };
    this.operation = operation;
    this.state = { step: 'sending', review: reviewed.review, message: 'Reading the file into memory… No upload progress is available.' };
    // Start in a microtask so pending ownership exists before any callbacks run.
    const pending = Promise.resolve().then(() => this.send(reviewed, operation)).finally(() => {
      if (this.pending === pending) { this.pending = undefined; this.operation = undefined; this.changed(); }
    });
    this.pending = pending;
    this.changed();
    return pending;
  }

  private async send({ file, binding, review }: NonNullable<UploadController['reviewed']>, operation: NonNullable<UploadController['operation']>): Promise<void> {
    const current = () => operation.generation === this.generation && !operation.controller.signal.aborted && this.matches(binding);
    const publish = (state: UploadState) => { this.state = state; this.changed(); };
    let verifying = false;
    const timer = setTimeout(() => {
      if (operation.generation !== this.generation) return;
      ++this.generation;
      operation.controller.abort();
      publish({ step: 'result', review, recordId: this.state.recordId, outcome: operation.sent ? 'uncertain' : 'cancelled',
        message: operation.sent ? `Timed out. ${uncertain}` : 'Timed out while reading the local file. Nothing was sent.' });
    }, this.timeout);
    try {
      if (!current()) return;
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (!current()) return;
      if (bytes.byteLength !== review.size || bytes.byteLength > MAX_UPLOAD_BYTES) {
        publish({ step: 'result', review, outcome: 'error', message: 'File size changed while buffering. Nothing was sent.' }); return;
      }
      operation.sent = true;
      publish({ step: 'sending', review, message: 'Sending buffered upload… Byte progress is unavailable. Cancelling may leave a server record.' });
      if (!current()) return;
      const saved = await binding.data.write({ name: review.target, dataType: review.dataType }, bytes, operation.controller.signal);
      if (!current()) return;
      verifying = true;
      publish({ step: 'sending', review, recordId: saved.id, message: 'Write returned. Reading back metadata before marking the upload verified…' });
      if (!current()) return;
      const metadata = await binding.data.get(saved.id, operation.controller.signal);
      if (!current()) return;
      if (metadata.id !== saved.id || metadata.domain_id.toLowerCase() !== review.domainId.toLowerCase() || metadata.name !== review.target || metadata.data_type !== review.dataType || metadata.size !== review.size) {
        publish({ step: 'result', review, recordId: saved.id, outcome: 'uncertain', message: `Metadata readback did not match the reviewed upload. ${uncertain}` }); return;
      }
      publish({ step: 'result', review, recordId: metadata.id, metadata, outcome: 'verified', message: 'Upload verified by metadata readback (Domain, target, type and byte size). File contents were not downloaded for comparison.' });
      // UI refresh is downstream of verification and is fenced independently.
      if (current()) {
        try { this.uploaded(); } catch { /* A failed record-list refresh cannot undo verified metadata. */ }
      }
    } catch (error) {
      if (!current()) return;
      const failure = error as { status?: number; code?: string };
      const denied = failure?.status === 401 || failure?.status === 403 || failure?.code === 'authorization_denied';
      if (verifying) publish({ step: 'result', review, recordId: this.state.recordId, outcome: 'uncertain', message: `${denied ? 'Metadata readback denied.' : 'Metadata readback failed.'} ${uncertain}` });
      else if (denied) publish({ step: 'result', review, outcome: 'denied', message: 'Upload denied. Write permission is separate from read permission. No automatic retry.' });
      else if (failure?.status === 409) publish({ step: 'result', review, outcome: 'collision', message: 'Target name already exists. Nothing was overwritten. No automatic retry; check the target.' });
      else publish({ step: 'result', review, outcome: operation.sent ? 'uncertain' : 'error', message: operation.sent ? `Upload request failed. ${uncertain}` : 'Could not read the local file. Nothing was sent.' });
    } finally {
      clearTimeout(timer);
      // Also fence context changes when the host has not yet called clear().
      if (operation.generation === this.generation && !this.matches(binding)) {
        ++this.generation;
        operation.controller.abort();
        publish({ step: 'result', review, outcome: operation.sent ? 'uncertain' : 'cancelled', message: operation.sent ? `Destination changed. ${uncertain}` : 'Destination changed. Nothing was sent.' });
      }
    }
  }

  /** Synchronously invalidate first; callers can separately await cancel(). */
  clear(): void {
    ++this.generation;
    this.reviewed = undefined;
    const operation = this.operation;
    operation?.controller.abort();
    if (operation && this.state.step === 'sending') {
      this.state = { step: 'result', review: this.state.review, recordId: this.state.recordId,
        outcome: operation.sent ? 'uncertain' : 'cancelled', message: operation.sent ? `Cancelled. ${uncertain}` : 'Cancelled before sending. Nothing was sent.' };
    } else if (this.state.step === 'review') {
      this.state = { step: 'choose', message: 'Review cancelled. Nothing was sent.' };
    }
    // Keep uncertain target details available after Domain changes and reopening.
    this.changed();
  }

  /** Authentication boundary: forget all prior account details, including late results. */
  reset(): void {
    ++this.generation;
    this.reviewed = undefined;
    this.operation?.controller.abort();
    this.state = { step: 'choose', message: 'Choose one file, then review its destination.' };
    this.changed();
  }

  async cancel(): Promise<void> { this.clear(); await this.pending; }
}
