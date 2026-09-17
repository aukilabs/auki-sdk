import type { Connection } from './sdk';
import { JobsController, type JobsContext, type Role } from './jobs.ts';
import { inspect, redact, uuid } from './safety.ts';
import { jobsShouldPoll } from './screens.ts';

/** Public configuration stays in memory; creating this UI performs no jobs calls. */
export function jobsUI(connection: Connection, getContext: () => JobsContext | undefined, options: {
  navigate: (screen: string) => void; openRecord: (id: string) => Promise<void>;
}): { open(inputId?: string): void; close(retainSession?: object): Promise<void>; refreshContext(): void } {
  const section = document.querySelector<HTMLElement>('#view-jobs')!;
  section.innerHTML = `<div class="jobs-workspace">
    <div class="jobs-toolbar"><button id="jobs-back" type="button">← Back</button><span class="eyebrow">06 / Jobs playground</span></div>
    <h1 id="jobs-heading" tabindex="-1">Configure your demo.</h1>
    <p id="jobs-context" class="footnote"></p>
    <div id="jobs-screen"></div>
    <p id="jobs-status" role="status" aria-live="polite"></p>
    <div class="jobs-footer"><button id="jobs-new" type="button">New job</button><button id="jobs-configure" type="button">Worker configuration</button><button id="jobs-history" type="button">Job history</button></div>
  </div>`;
  const get = (id: string) => section.querySelector<HTMLElement>(`#${id}`)!;
  const field = (id: string) => get(id) as HTMLInputElement;
  const button = (id: string) => get(id) as HTMLButtonElement;
  const text = (id: string, value: unknown) => { get(id).textContent = String(redact(value ?? '')); };
  let context = getContext(), screen = '', override: 'setup' | 'choose' | undefined;
  let draftInput = '', draftRole: Role = 'compute', busy = false, generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined, polls = 0;
  let closing: Promise<void> = Promise.resolve();
  const controller = new JobsController(getContext, render);
  function stopPolling() { clearTimeout(timer); timer = undefined; }
  function schedule() {
    stopPolling();
    const state = controller.state;
    if (busy || polls >= 60 || !jobsShouldPoll(!section.hidden && !document.hidden, screen, state.details?.tasks ?? [])) return;
    timer = setTimeout(() => { polls++; void act(() => controller.refresh()); }, 3000);
  }
  async function act(action: () => unknown | Promise<unknown>) {
    if (busy) return;
    const version = generation;
    let failed = false;
    busy = true; stopPolling();
    try { const pending = action(); render(); await pending; }
    catch { failed = true; }
    finally { if (version === generation) { busy = false; render(); if (failed) text('jobs-status', 'Unable to complete this action. Check configuration and permissions.'); } }
  }
  function facts(values: Record<string, unknown>) {
    const dl = document.createElement('dl'); dl.className = 'facts';
    for (const [key, value] of Object.entries(values)) {
      const row = document.createElement('div'), dt = document.createElement('dt'), dd = document.createElement('dd');
      dt.textContent = key; dd.textContent = String(redact(value ?? 'Not reported')); row.append(dt, dd); dl.append(row);
    }
    return dl;
  }
  const phaseText = (value: unknown): string => typeof value === 'string' ? String(redact(value)).slice(0, 120) : 'Not reported';
  function render() {
    const state = controller.state;
    const next = override ?? (state.phase === 'submitting' ? 'review' : state.phase);
    const changed = screen !== next; screen = next;
    const focusId = section.contains(document.activeElement) ? (document.activeElement as HTMLElement).id : '';
    const technicalOpen = section.querySelector('details')?.open ?? false;
    text('jobs-context', context ? `Domain ${context.domainId} · ${context.environment}` : 'Choose a Domain before configuring or running a job.');
    text('jobs-status', state.message);
    button('jobs-configure').disabled = busy || !context || state.phase === 'submitting' || !!state.reconciliation;
    button('jobs-history').disabled = busy || !connection.session || !context || !state.config;
    button('jobs-back').disabled = busy;
    button('jobs-new').hidden = !state.config || !!state.reconciliation || !['detail', 'history'].includes(state.phase);
    button('jobs-new').disabled = busy;
    const host = get('jobs-screen');
    // One local screen exists at a time; no hidden duplicate controls or stale results.
    if (changed || !host.firstElementChild || !['setup', 'choose'].includes(screen)) {
      const pane = document.createElement('section'); pane.dataset.jobsScreen = screen; host.replaceChildren(pane);
      if (screen === 'setup') {
        text('jobs-heading', 'Configure your demo.');
        pane.innerHTML = `<p>Use the public IDs of your activated demo workers. Configuration does not start workers or establish online status.</p>
          <form id="jobs-setup-form" class="jobs-card"><label>Installation UUID<input id="jobs-installation" required autocomplete="off" spellcheck="false"></label>
          <div class="jobs-fields"><label>Expected compute UUID<input id="jobs-compute-id" required autocomplete="off" spellcheck="false"></label><label>Expected robot UUID<input id="jobs-robot-id" required autocomplete="off" spellcheck="false"></label></div>
          <p class="footnote">Public identifiers only. Kept in memory for this Domain and session. Unresolved submissions survive Domain changes until reconciled; logout erases recovery. Activation is a separate operator step.</p><button id="jobs-save-config" class="primary">Use this configuration →</button></form>`;
        field('jobs-installation').value = state.config?.installationId ?? '';
        field('jobs-compute-id').value = state.config?.computeId ?? '';
        field('jobs-robot-id').value = state.config?.robotId ?? '';
        get('jobs-setup-form').onsubmit = event => {
          event.preventDefault();
          try {
            const config = { installationId: uuid(field('jobs-installation').value), computeId: uuid(field('jobs-compute-id').value), robotId: uuid(field('jobs-robot-id').value) };
            override = undefined; void act(() => controller.configure(config));
          } catch { text('jobs-status', 'Enter three complete public UUIDs. Do not enter credentials.'); }
        };
      } else if (screen === 'choose') {
        text('jobs-heading', 'One record. One action.');
        pane.innerHTML = `<p>Run a small, dedicated demo task. Choose an existing record or upload one from Data.</p><form id="jobs-choose-form" class="jobs-card">
          <label>Action<select id="jobs-role"><option value="compute">Compute · uppercase text</option><option value="robot">Robot · simulated inspection</option></select></label>
          <p id="jobs-action-help" class="footnote"></p><label>Input record UUID<input id="jobs-input" required autocomplete="off" spellcheck="false" aria-describedby="jobs-input-help"></label>
          <p id="jobs-input-help" class="footnote">Maximum 64 KiB (65,536 bytes). Compute requires valid UTF-8. Robot reports byte count and hash; no hardware actions.</p>
          <div class="actions"><button id="jobs-estimate" class="primary">Estimate &amp; review →</button><button type="button" id="jobs-browse-data">Choose from Data</button></div></form>`;
        field('jobs-input').value = draftInput; field('jobs-role').value = draftRole;
        const help = () => text('jobs-action-help', `Expected ${draftRole} worker: ${draftRole === 'compute' ? state.config?.computeId : state.config?.robotId}. Availability is checked only when you estimate.`);
        field('jobs-role').onchange = () => { draftRole = field('jobs-role').value as Role; help(); };
        field('jobs-input').oninput = () => { draftInput = field('jobs-input').value; };
        help();
        button('jobs-browse-data').onclick = () => options.navigate('data');
        get('jobs-choose-form').onsubmit = event => { event.preventDefault(); override = undefined; polls = 0; void act(() => controller.prepare(draftRole, draftInput.trim())); };
      } else if (screen === 'review') {
        text('jobs-heading', state.phase === 'submitting' ? 'Submitting your job…' : 'Review before running.');
        pane.append(facts({ Action: state.role === 'robot' ? 'Simulated inspection' : 'Uppercase text', Input: state.inputId,
          Capability: state.spec?.tasks[0].capability, 'Output name': `sdk-${state.config?.installationId}-${state.role}-{taskId}`, 'Output format': state.role === 'robot' ? 'JSON inspection report' : 'UTF-8 text', 'Expected worker': state.expectedWorkerId, 'Estimated credits': state.estimate?.total, Mode: 'Dedicated · one task · maximum one attempt' }));
        const outputType = document.createElement('p'); outputType.className = 'footnote';
        // Fixed application schema labels, never backend strings or credentials.
        outputType.textContent = state.role === 'robot' ? 'Output SDK type: example.report.v1' : 'Output SDK type: example.text.v1'; pane.append(outputType);
        pane.insertAdjacentHTML('beforeend', `<p>Confirm the Domain above, input and credit estimate. The output taskId is assigned on submission. An estimate is not a worker reservation. Submitting may lock credits.</p><div class="actions"><button id="jobs-confirm" type="button" class="primary">Confirm &amp; submit job</button><button id="jobs-edit" type="button">← Change action or input</button></div>`);
        button('jobs-confirm').disabled = busy || state.phase !== 'review' || !state.estimate;
        button('jobs-edit').disabled = busy || state.phase === 'submitting';
        button('jobs-confirm').onclick = () => { void act(() => controller.submit()); };
        button('jobs-edit').onclick = edit;
      } else if (screen === 'history') {
        text('jobs-heading', 'Recent demo jobs.');
        pane.innerHTML = `<p>One provider page, filtered to this installation’s capabilities. Listing can skip jobs: <a href="https://github.com/aukilabs/auki-sdk/issues/396" target="_blank" rel="noreferrer">#396 · incomplete pagination</a>. An absent job is not proof that submission failed.</p><div id="jobs-history-items" class="jobs-list"></div><button id="jobs-next" type="button">Next provider page →</button>`;
        if (state.reconciliation) pane.prepend(facts({ 'Reconcile submission label': state.reconciliation.label, 'Original Domain': state.reconciliation.domainId }));
        for (const item of state.items ?? []) {
          const entry = document.createElement('button'); entry.type = 'button'; entry.dataset.jobId = item.job.id;
          entry.textContent = String(redact(`${item.job.label} · ${item.job.status}`)); entry.disabled = busy;
          entry.onclick = () => { polls = 0; void act(() => controller.inspect(item.job.id)); }; get('jobs-history-items').append(entry);
        }
        if (!state.items?.length) text('jobs-history-items', 'No matching jobs on this page.');
        button('jobs-next').disabled = busy || !state.nextCursor;
        button('jobs-next').onclick = () => { void act(() => controller.list(state.nextCursor ?? undefined)); };
      } else if (screen === 'uncertain') {
        text('jobs-heading', 'Submission needs checking.');
        pane.append(facts({ 'Original Domain': state.reconciliation?.domainId ?? state.domainId, 'Original environment': state.reconciliation?.environment ?? state.environment, 'Submission label': state.reconciliation?.label ?? state.spec?.label }));
        pane.insertAdjacentHTML('beforeend', '<p>The response was lost or could not be verified. The job may already exist. Use Job history to reconcile this label in the original Domain before another submission. Only one unresolved submission is retained per session; new jobs are blocked until it is reconciled. Labels are not idempotency keys. Logout or reload erases this in-memory recovery.</p>');
      } else {
        text('jobs-heading', 'Follow your job.');
        const details = state.details;
        pane.append(facts({ Job: state.jobId, Status: details?.job.status ?? 'Awaiting details', 'Executor verification': state.executorMatch === true ? 'Expected worker verified' : 'Unverified — outputs withheld',
          'Locked credits': details?.job.credit_lock_amount, 'Credits released at': details?.job.credit_released_at }));
        pane.insertAdjacentHTML('beforeend', `<div id="jobs-tasks" class="jobs-list"></div><div id="jobs-outputs" class="actions"></div><div class="actions"><button id="jobs-refresh" type="button">Refresh status</button><button id="jobs-cancel" type="button">Cancel whole job…</button></div><p class="footnote">A canceled job can still have running tasks. Cancellation does not prove task shutdown or credit release. Automatic refresh pauses after 60 checks.</p><details><summary id="jobs-details">Technical details · redacted</summary><pre id="jobs-json"></pre></details>`);
        for (const task of details?.tasks ?? []) get('jobs-tasks').append(facts({ Task: task.label, Status: task.status, Attempts: `${task.attempts} / ${task.max_attempts}`, Phase: phaseText((task.meta.progress as { phase?: unknown } | undefined)?.phase), 'Recent events': Array.isArray(task.meta.events) ? task.meta.events.slice(-3).map(event => typeof event === 'object' && event ? phaseText((event as { phase?: unknown }).phase) : 'Event').join(' → ') : 'Not reported' }));
        // Defense in depth: only completed tasks with matching actual receipts expose UUID links.
        if (state.executorMatch === true && state.expectedWorkerId) for (const receipt of details?.receipts ?? []) {
          const task = details?.tasks.find(task => task.id === receipt.task_id);
          if (task?.status !== 'completed' || receipt.node_id !== state.expectedWorkerId) continue;
          if (receipt.meta.status !== undefined && receipt.meta.status !== 'completed') continue;
          for (const ref of receipt.outputs) {
            if (!state.outputs?.includes(ref.toLowerCase())) continue;
            let id: string; try { id = uuid(ref); } catch { continue; }
            const output = document.createElement('button'); output.type = 'button'; output.dataset.jobOutput = id; output.textContent = 'Open output in Data →'; output.disabled = busy;
            output.onclick = () => { void act(() => options.openRecord(id)); }; get('jobs-outputs').append(output);
          }
        }
        text('jobs-json', inspect(details));
        section.querySelector('details')!.open = technicalOpen;
        button('jobs-refresh').disabled = busy || !state.jobId;
        button('jobs-refresh').onclick = () => { polls = 0; void act(() => controller.refresh()); };
        button('jobs-cancel').disabled = busy || !state.jobId || ['completed', 'failed', 'canceled'].includes(details?.job.status ?? '');
        button('jobs-cancel').onclick = () => {
          if (window.confirm('Cancel this whole job? Running tasks may take time to stop, and locked credits may remain unreleased.')) void act(() => controller.cancel());
        };
      }
    }
    for (const id of ['jobs-save-config', 'jobs-estimate']) {
      const control = section.querySelector<HTMLButtonElement>(`#${id}`); if (control) control.disabled = busy || !context;
    }
    for (const control of section.querySelectorAll<HTMLInputElement | HTMLSelectElement>('#jobs-screen input, #jobs-screen select')) control.disabled = busy || !context;
    if (!changed && focusId && !section.hidden) section.querySelector<HTMLElement>(`#${focusId}`)?.focus({ preventScroll: true });
    if (changed && !section.hidden) { section.scrollTop = 0; get('jobs-heading').focus({ preventScroll: true }); }
    schedule();
  }
  function edit() {
    const state = controller.state;
    draftInput = state.inputId ?? draftInput; draftRole = state.role ?? draftRole;
    if (state.config) { override = 'choose'; void act(() => controller.configure(state.config!)); }
  }
  button('jobs-new').onclick = edit;
  button('jobs-configure').onclick = () => { override = 'setup'; if (controller.state.config) void act(() => controller.configure(controller.state.config!)); else render(); };
  button('jobs-history').onclick = () => { override = undefined; void act(() => controller.list()); };
  button('jobs-back').onclick = () => {
    if (screen === 'review') edit();
    else if (screen === 'setup' && controller.state.config) { override = 'choose'; render(); }
    else if (screen === 'history') { override = undefined; render(); options.navigate('data'); }
    else options.navigate('data');
  };
  new MutationObserver(schedule).observe(section, { attributes: true, attributeFilter: ['hidden'] });
  document.addEventListener('visibilitychange', schedule);
  function close(retainSession?: object) {
    ++generation; busy = false; stopPolling(); override = undefined; draftInput = ''; draftRole = 'compute'; screen = ''; polls = 0;
    context = getContext();
    const pending = controller.close(retainSession);
    closing = Promise.allSettled([closing, pending]).then(results => { if (results.some(result => result.status === 'rejected')) throw new Error('Jobs cleanup failed.'); });
    render(); return closing;
  }
  render();
  return {
    open(inputId) {
      if (inputId && !busy && !['uncertain', 'submitting'].includes(controller.state.phase)) { draftInput = uuid(inputId); if (controller.state.config) { override = 'choose'; void act(() => controller.configure(controller.state.config!)); } }
      options.navigate('jobs'); render(); get('jobs-heading').focus({ preventScroll: true });
    },
    close,
    refreshContext() {
      const next = getContext();
      if (next?.domainId !== context?.domainId || next?.session !== context?.session || next?.environment !== context?.environment || next?.data !== context?.data) {
        context = next; void close(connection.session).catch(() => text('jobs-status', 'Jobs cleanup failed. Reload before reconnecting.'));
      } else { controller.restoreContext(); render(); }
    },
  };
}
