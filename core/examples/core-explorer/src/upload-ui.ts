import type { Connection } from './sdk';
import { redact } from './safety.ts';
import { UploadController, UploadValidationError, DEFAULT_UPLOAD_TYPE, type UploadContext } from './upload.ts';

export function uploadUI(connection: Connection, getContext: () => UploadContext | undefined, options: {
  navigate: (screen: string) => void; onUploaded: () => void;
}): { open(): void; clear(): void; reset(): void; cancel(): Promise<void> } {
  const section = document.querySelector<HTMLElement>('#view-upload')!;
  // Static markup only; all file, environment and provider values use text nodes.
  section.innerHTML = `<div class="chapter-body">
    <button id="upload-back" type="button" data-back="data">Back to Data</button>
    <h1 id="upload-heading" tabindex="-1">Upload one file</h1>
    <p id="upload-intro">Buffered upload · maximum 8 MiB (8,388,608 bytes). Each upload uses a generated unique name. No overwrite or automatic retry.</p>
    <div id="upload-choose"><label>File<input id="upload-file" type="file"></label>
      <label>Data type<input id="upload-type" value="${DEFAULT_UPLOAD_TYPE}" autocomplete="off" aria-describedby="upload-type-help"></label>
      <p id="upload-type-help">SDK named data types allow 1–128 UTF-8 bytes and exclude punctuation such as /. The default core-explorer.file.v1 works for this example.</p>
      <button id="upload-review" type="button" class="primary">Review upload</button></div>
    <div id="upload-destination" hidden><h2 id="upload-destination-title">Destination</h2><dl id="upload-facts"></dl>
      <label>Generated target<input id="upload-target" readonly></label></div>
    <div id="upload-review-step" hidden><p>Confirm the Domain, connected environment, target, type and size above. Read access does not grant write permission.</p>
      <button id="upload-confirm" type="button" class="primary">Confirm upload</button>
      <button id="upload-edit" type="button">Change file or type</button></div>
    <div id="upload-sending" hidden><p>Keep this page open while the SDK completes the request. Cancelling after send may leave a server record.</p></div>
    <div id="upload-result" hidden><button id="upload-metadata" type="button">Returned metadata · redacted</button>
      <p>For uncertain completion, copy the generated target above and check it with the Data name filter in the original Domain before deciding to upload again.</p>
      <button id="upload-new" type="button">Choose another file</button></div>
    <p id="upload-status" role="status" aria-live="polite"></p>
    <button id="upload-cancel" type="button">Cancel upload</button>
  </div>`;
  const get = (id: string) => section.querySelector<HTMLElement>(`#${id}`)!;
  const field = (id: string) => get(id) as HTMLInputElement;
  const button = (id: string) => get(id) as HTMLButtonElement;
  let chooseNew = false;
  const controller = new UploadController(() => {
    const context = getContext(), data = connection.data;
    return context && data && connection.session ? { ...context, data } : undefined;
  }, render, options.onUploaded);
  function render() {
    const state = controller.state;
    const step = chooseNew && !controller.busy ? 'choose' : state.step;
    // Exactly one step marker denotes the active upload step.
    section.dataset.uploadStep = step;
    get('upload-choose').hidden = step !== 'choose';
    get('upload-review-step').hidden = step !== 'review';
    get('upload-sending').hidden = step !== 'sending';
    get('upload-result').hidden = step !== 'result';
    get('upload-destination').hidden = !state.review;
    button('upload-review').disabled = controller.busy || !connection.data || !getContext();
    button('upload-confirm').disabled = controller.busy || step !== 'review';
    button('upload-edit').disabled = controller.busy;
    button('upload-new').disabled = controller.busy;
    button('upload-cancel').disabled = !controller.busy && step !== 'review';
    get('upload-status').textContent = chooseNew ? 'Choose one file and review its destination. The previous target remains available below for checking.' : state.message;
    get('upload-destination-title').textContent = chooseNew ? 'Previous destination' : 'Destination';
    button('upload-metadata').disabled = !state.metadata;
    get('upload-heading').textContent = step === 'review' ? 'Review upload' : step === 'sending' ? 'Uploading' : step === 'result' ? 'Upload result' : 'Upload one file';
    get('upload-intro').hidden = step !== 'choose';
    get('upload-facts').replaceChildren();
    if (state.review) {
      const review = state.review;
      for (const [key, value] of Object.entries({ Domain: review.domainName || 'Known Domain', File: review.fileName, Size: `${review.size.toLocaleString('en-US')} bytes`,
        'Domain ID': review.domainId, Environment: review.environment, 'Data type': review.dataType,
        ...(state.recordId ? { 'Returned record ID': state.recordId } : {}) })) {
        const term = document.createElement('dt'), description = document.createElement('dd');
        term.textContent = key; description.textContent = String(redact(value)); get('upload-facts').append(term, description);
      }
      field('upload-target').value = review.target;
    } else field('upload-target').value = '';
  }
  button('upload-review').onclick = () => {
    if (controller.busy) return;
    const file = field('upload-file').files?.[0];
    if (!file) { get('upload-status').textContent = 'Choose one file before reviewing.'; return; }
    try { chooseNew = false; controller.review(file, field('upload-type').value); section.scrollTop = 0; get('upload-heading').focus({ preventScroll: true }); }
    catch (error) { render(); get('upload-status').textContent = error instanceof UploadValidationError ? error.message : 'Unable to review this upload.'; }
  };
  button('upload-metadata').onclick = () => {
    if (controller.state.metadata) document.dispatchEvent(new CustomEvent('explorer:technical', { detail: controller.state.metadata }));
  };
  button('upload-confirm').onclick = () => { void controller.confirm(); };
  button('upload-cancel').onclick = () => { void controller.cancel(); };
  button('upload-edit').onclick = () => { controller.clear(); chooseNew = true; render(); field('upload-type').focus(); };
  button('upload-new').onclick = () => { chooseNew = true; field('upload-file').value = ''; render(); field('upload-file').focus(); };
  button('upload-back').onclick = () => { controller.clear(); options.navigate('data'); };
  render();
  return {
    open() { render(); options.navigate('upload'); section.scrollTop = 0; get('upload-heading').focus({ preventScroll: true }); },
    clear() { chooseNew = false; field('upload-file').value = ''; controller.clear(); },
    reset() { chooseNew = false; field('upload-file').value = ''; field('upload-type').value = DEFAULT_UPLOAD_TYPE; controller.reset(); },
    cancel() { return controller.cancel(); },
  };
}
