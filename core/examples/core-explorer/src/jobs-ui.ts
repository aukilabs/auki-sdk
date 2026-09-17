import { JobsRecords } from './jobs-records.ts';
import { FleetController } from './fleet.ts';
import { renderFleet } from './fleet-ui.ts';
import type { Connection } from './sdk';
import { JobsController, HISTORY_WARNING, type JobsContext, type Role } from './jobs.ts';
import { inspect, redact, uuid } from './safety.ts';
import { jobsShouldPoll } from './screens.ts';

/** Public configuration stays in memory; creating this UI performs no jobs calls. */
export function jobsUI(connection: Connection, getContext: () => JobsContext | undefined, options: {
  domainName?: () => string; navigate: (screen: string) => void; openRecord: (id: string) => Promise<void>;
}): { open(inputId?: string): void; close(retainSession?: object): Promise<void>; refreshContext(): void } {
  const section = document.querySelector<HTMLElement>('#view-jobs')!;
  section.innerHTML = `<div class="jobs-workspace">
    <div class="jobs-toolbar"><button id="jobs-back" type="button">← Jobs</button><h1 id="jobs-heading" tabindex="-1">Jobs</h1><button id="jobs-new" class="primary" type="button">New job</button></div>
    <p id="jobs-context" class="footnote"></p>
    <div class="jobs-worker-strip"><button id="jobs-configure" type="button">Workers</button><span id="jobs-workers" hidden></span><button id="jobs-discover" type="button" hidden>Refresh workers</button></div>
    <p id="jobs-status" role="status" aria-live="polite"></p>
    <div id="jobs-screen"></div>
    <button id="jobs-history" type="button">Recent jobs</button>
  </div>`;
  const get = (id: string) => section.querySelector<HTMLElement>(`#${id}`)!;
  const field = (id: string) => get(id) as HTMLInputElement;
  const button = (id: string) => get(id) as HTMLButtonElement;
  const text = (id: string, value: unknown) => { get(id).textContent = String(redact(value ?? '')); };
  let context = getContext(), screen = '', override: 'setup' | 'choose' | 'dashboard' | undefined;
  let draftInput = '', draftRole: Role = 'compute', busy = false, generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined, polls = 0;
  let closing: Promise<void> = Promise.resolve();
  let entered = false, ready = true, discoveryReady = false, pickerVersion = 0, previewVersion = 0, pickerStarted = false;
  const previewStates = new Map<string, string>();
  const names = new Map<string, string>();
  const previews = new Map<string, string>();
  let historyPage: { items: NonNullable<JobsController['state']['items']>; nextCursor?: string; config: string; scroll: number; selected: string } | undefined;
  let restoreHistory = false;
  const summaries = new Map<string, { action: string; input: string; result: string; evidence: string }>();
  const records = new JobsRecords(() => {
    const current = getContext(), data = connection.data;
    return current && data ? { domainId: current.domainId, data } : undefined;
  });
  function invalidateReads(preservePreview = false) {
    pickerVersion++; previewVersion++; pickerStarted = false; previewStates.clear();
    if (preservePreview) {
      for (const [id, preview] of previews) previewStates.set(id, preview);
      pickerStarted = !!draftInput && previews.has(draftInput);
    }
    else previews.clear();
    void records.close();
  }
  function evidence(job: unknown) { return JSON.stringify([job, controller.state.config]); }
  function selectedInput() {
    if (screen !== 'choose') return;
    get('jobs-picker').hidden = !!draftInput;
    get('jobs-input-preview').hidden = !draftInput;
    get('jobs-change-input').hidden = !draftInput;
    text('jobs-selected-name', draftInput ? names.get(draftInput) || 'Selected record' : '');
  }
  function activateReads() {
    if (section.hidden || !ready || busy) return;
    if (screen === 'choose') {
      if (!pickerStarted) { pickerStarted = true; void loadPicker(); }
      if (draftInput && !previewStates.has(draftInput)) void previewRecord(draftInput);
      selectedInput();
    } else if (screen === 'detail' && controller.state.executorMatch) {
      const id = controller.state.outputs?.[0];
      if (id && !previewStates.has(id)) void previewRecord(id, true);
    }
  }
  async function loadPicker(name = '') {
    if (section.hidden || screen !== 'choose' || !ready) return;
    const version = ++pickerVersion, epoch = generation;
    text('jobs-record-status', 'Loading records…');
    try {
      const items = await records.list(name);
      if (epoch !== generation || version !== pickerVersion || screen !== 'choose' || section.hidden || !items) return;
      const list = get('jobs-records'); list.replaceChildren();
      for (const record of items) {
        names.set(record.id, record.name);
        const entry = document.createElement('button'); entry.type = 'button'; entry.dataset.recordId = record.id; entry.id = `jobs-record-${record.id}`;
        entry.textContent = String(redact(`${record.name || 'Unnamed record'} · ${record.size} bytes`));
        entry.disabled = record.size > 65536;
        entry.onclick = () => { draftInput = record.id; field('jobs-input').value = record.id; previewVersion++; previewStates.delete(record.id); selectedInput(); void previewRecord(record.id); };
        list.append(entry);
      }
      text('jobs-record-status', items.length ? 'Up to 100 records · 64 KiB maximum' : 'No matching records. Try another name.');
    } catch { if (epoch === generation && version === pickerVersion && screen === 'choose') text('jobs-record-status', 'Unable to read records. Check Domain data permission and retry search.'); }
  }
  async function previewRecord(id: string, output = false) {
    if (section.hidden || !ready || previewStates.has(id)) return;
    const epoch = generation, preview = ++previewVersion;
    const target = output ? 'jobs-output-preview' : 'jobs-input-preview';
    previewStates.set(id, 'Loading preview…'); text(target, 'Loading preview…');
    button(output ? 'jobs-output-retry' : 'jobs-preview-retry').hidden = true;
    const current = () => preview === previewVersion && epoch === generation && !section.hidden
      && (output ? screen === 'detail' && controller.state.outputs?.includes(id) : screen === 'choose' && id === draftInput);
    try {
      const result = await records.preview(id);
      if (!result || !current()) return;
      names.set(id, result.record.name); previews.set(id, result.text);
      previewStates.set(id, result.text); text(target, result.text);
      if (!output) selectedInput();
    } catch {
      if (current()) { previewStates.set(id, 'Preview unavailable. Check permission and size (64 KiB maximum).'); text(target, previewStates.get(id)); button(output ? 'jobs-output-retry' : 'jobs-preview-retry').hidden = false; }
    }
  }
  async function dashboard(cached = false) {
    restoreHistory = cached && !!historyPage && historyPage.config === JSON.stringify(controller.state.config) && !controller.state.reconciliation;
    if (!restoreHistory) historyPage = undefined;
    invalidateReads(); draftInput = ''; override = 'dashboard'; controller.choose(); screen = ''; render();
    if (controller.state.config && !restoreHistory) await controller.list();
  }

  const controller = new JobsController(getContext, render);
  const fleet = new FleetController(() => {
    const current = getContext(), session = connection.session;
    return current && session ? { ...current, createFleet: () => session.fleet(current.domainId) } : undefined;
  }, () => { if (screen === 'setup') { screen = ''; render(); } });
  async function discover() {
    historyPage = undefined; restoreHistory = false;
    const version = generation;
    controller.invalidateDiscovery();
    await fleet.refresh();
    if (version !== generation) return;
    discoveryReady = true;
    await finishDiscovery();
  }
  async function finishDiscovery() {
    if (!discoveryReady || section.hidden || !ready) return;
    const version = generation;
    discoveryReady = false;
    if (controller.state.reconciliation || fleet.state.installations.length !== 1) return;
    const choice = fleet.state.installations[0];
    if (choice.config.computeId || choice.config.robotId) {
      screen = ''; draftRole = choice.config.computeId ? 'compute' : 'robot';
      await controller.configure(choice.config);
      if (version !== generation) return;
      // Navigation can also occur while configuration drains an old context.
      if (section.hidden) { discoveryReady = true; return; }
      if (override !== 'choose') await dashboard();
    }
  }
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
    finally { if (version === generation) { busy = false; render(); activateReads(); schedule(); if (failed) text('jobs-status', 'Unable to complete this action. Check configuration and permissions.'); } }
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
    const state = restoreHistory && historyPage ? { ...controller.state, items: historyPage.items, nextCursor: historyPage.nextCursor } : controller.state;
    const next = state.phase === 'uncertain' && override !== 'dashboard' ? 'uncertain' : override ?? (state.phase === 'submitting' ? 'review' : state.phase === 'history' || state.phase === 'setup' ? 'dashboard' : state.phase);
    const changed = screen !== next; screen = next;
    const focusId = section.contains(document.activeElement) ? (document.activeElement as HTMLElement).id : '';
    const technicalOpen = section.querySelector('details')?.open ?? false;
    text('jobs-context', context ? '' : 'Choose a Domain to view jobs.');
    text('jobs-status', state.reconciliation ? `Submission uncertain. A job may exist and consume credits. Reconcile the saved label before submitting again. ${state.message}` : state.message !== HISTORY_WARNING && state.message !== 'Observed executor matches the configured worker.' && (state.errorCode || state.reconciliation || ['detail', 'uncertain', 'submitting'].includes(state.phase) || /fail|denied|unresolved|exceeds|must|invalid|changed|missing|ambiguous/i.test(state.message)) ? state.message : '');
    const choice = fleet.state.installations.find(item => item.config.installationId === state.config?.installationId);
    text('jobs-workers', fleet.state.loading ? 'Discovering workers…' : choice ? [...choice.compute, ...choice.robot].map(m => `${m.association === 'assigned' ? 'Robot' : 'Compute'}: ${m.presence} · ${m.work_state}`).join(' / ') : fleet.state.message || 'Workers not yet observed');
    button('jobs-discover').disabled = busy || !context || !!state.reconciliation;
    button('jobs-back').hidden = screen === 'dashboard';
    get('jobs-workers').parentElement!.hidden = ['setup', 'choose', 'review', 'detail', 'uncertain'].includes(screen);
    button('jobs-history').hidden = screen !== 'uncertain' && !state.reconciliation;
    button('jobs-configure').disabled = busy || !context || state.phase === 'submitting' || !!state.reconciliation;
    button('jobs-history').disabled = busy || !connection.session || !context || !state.config;
    button('jobs-back').disabled = busy;
    button('jobs-new').hidden = !!state.reconciliation || screen !== 'dashboard';
    button('jobs-new').disabled = busy || !context;
    const host = get('jobs-screen');
    // One local screen exists at a time; no hidden duplicate controls or stale results.
    if (changed || !host.firstElementChild || !['setup', 'choose'].includes(screen)) {
      const pane = document.createElement('section'); pane.dataset.jobsScreen = screen; host.replaceChildren(pane);
      if (screen === 'setup') {
        text('jobs-heading', 'Workers');
        renderFleet(pane, fleet, choice => {
          override = undefined; screen = ''; draftRole = choice.config.computeId ? 'compute' : 'robot';
          void act(async () => { await controller.configure(choice.config); await dashboard(); });
        }, () => { void act(discover); });
      } else if (screen === 'choose') {
        text('jobs-heading', 'New job');
        pane.innerHTML = `<form id="jobs-choose-form" class="jobs-card">
          <div class="jobs-action-cards"><button type="button" data-role="compute">Uppercase text</button><button type="button" data-role="robot">Inspect file <small>Byte count · SHA-256</small></button></div>
          <select id="jobs-role" aria-label="Action" hidden><option value="compute">Uppercase text</option><option value="robot">Inspect file</option></select>
          <p id="jobs-action-help" class="footnote"></p>
          <div id="jobs-picker"><label>Exact record name<input id="jobs-search" type="search" placeholder="Exact record name" autocomplete="off"></label>
          <button type="button" id="jobs-search-button">Search</button>
          <p id="jobs-record-status" role="status"></p><div id="jobs-records" class="jobs-record-picker"></div></div>
          <strong id="jobs-selected-name"></strong><button id="jobs-change-input" type="button" hidden>Change input</button>
          <pre id="jobs-input-preview" class="jobs-preview">Select a record to preview.</pre><button id="jobs-preview-retry" type="button" hidden>Retry preview</button>
          <details><summary>Record UUID</summary><label>Input record UUID<input id="jobs-input" autocomplete="off" spellcheck="false"></label></details>
          <div class="actions"><button id="jobs-estimate" class="primary">Get estimate</button></div></form>`;
        button('jobs-change-input').onclick = changeInput;
        button('jobs-preview-retry').onclick = () => { previewStates.delete(draftInput); if (draftInput) void previewRecord(draftInput); };
        button('jobs-search-button').onclick = () => { void loadPicker(field('jobs-search').value.trim()); };
        field('jobs-search').onkeydown = event => { if (event.key === 'Enter') { event.preventDefault(); void loadPicker(field('jobs-search').value.trim()); } };
        field('jobs-input').value = draftInput;
        if (!state.config?.[draftRole === 'compute' ? 'computeId' : 'robotId']) draftRole = state.config?.computeId ? 'compute' : 'robot';
        field('jobs-role').value = draftRole;
        for (const role of ['compute', 'robot'] as const) {
          const option = section.querySelector<HTMLOptionElement>(`#jobs-role option[value="${role}"]`)!;
          option.disabled = !!state.discoveryRequired || !state.config?.[role === 'compute' ? 'computeId' : 'robotId'];
          if (option.disabled) option.textContent += ' · unavailable';
        }
        const help = () => text('jobs-action-help', state.discoveryRequired ? 'Rediscover workers to enable this action.' : draftRole === 'robot' ? 'Simulated robot · up to 64 KiB' : 'UTF-8 · up to 64 KiB');
        field('jobs-role').onchange = () => { draftRole = field('jobs-role').value as Role; help(); };
        for (const card of section.querySelectorAll<HTMLButtonElement>('[data-role]')) {
          const role = card.dataset.role as Role;
          card.disabled = !!state.discoveryRequired || !state.config?.[role === 'compute' ? 'computeId' : 'robotId'];
          card.setAttribute('aria-pressed', String(role === draftRole));
          card.onclick = () => { draftRole = role; field('jobs-role').value = role; for (const other of section.querySelectorAll('[data-role]')) other.setAttribute('aria-pressed', String(other === card)); help(); };
        }
        field('jobs-input').oninput = () => { previewVersion++; draftInput = field('jobs-input').value; text('jobs-input-preview', 'Use Retry preview to inspect this record.'); button('jobs-preview-retry').hidden = false; selectedInput(); };
        help();
        if (draftInput) text('jobs-input-preview', previewStates.get(draftInput) || 'Loading preview…');
        selectedInput();
        get('jobs-choose-form').onsubmit = event => { event.preventDefault(); override = undefined; polls = 0; void act(() => controller.prepare(draftRole, draftInput.trim())); };
      } else if (screen === 'review') {
        text('jobs-heading', state.phase === 'submitting' ? 'Submitting…' : 'Confirm job');
        pane.append(facts({ Action: state.role === 'robot' ? 'Inspect file · simulated robot' : 'Uppercase text', Input: names.get(state.inputId ?? '') || state.inputId,
          Domain: options.domainName?.() && options.domainName?.() !== 'Known Domain' ? options.domainName() : context?.domainId, Environment: context?.environment, 'Estimated credits': state.estimate?.total ?? 'Unavailable' }));
        pane.insertAdjacentHTML('beforeend', `<pre id="jobs-review-preview" class="jobs-preview"></pre><p class="footnote">May lock credits; worker availability is not reserved. A lost response requires reconciliation.</p><div class="actions"><button id="jobs-confirm" type="button" class="primary">Confirm &amp; run</button><button id="jobs-edit" type="button">Change action or input</button></div><details id="jobs-review-technical"><summary>Details</summary></details>`);
        if (state.role === 'compute') {
          const warning = document.createElement('p'); warning.className = 'footnote';
          warning.textContent = 'Unicode expansion can exceed the 64 KiB output limit. Failure billing is unknown.'; pane.append(warning);
        }
        text('jobs-review-preview', previews.get(state.inputId ?? '') || 'Input preview unavailable. Return to the input to inspect its bytes before confirming.');
        get('jobs-review-technical').append(facts({ 'Domain ID': context?.domainId, 'Input ID': state.inputId,
          Capability: state.spec?.tasks[0].capability, 'Output name': `sdk-${state.config?.installationId}-${state.role}-{taskId}`,
          'Output format': state.role === 'robot' ? 'JSON byte-count / SHA-256 report' : 'UTF-8 text',
          'Expected worker': state.expectedWorkerId, Mode: 'Dedicated · one task · maximum one attempt' }));
        // These are local protocol constants, not provider strings or credentials.
        const outputType = document.createElement('p');
        outputType.textContent = `Output SDK type: ${state.role === 'robot' ? 'example.report.v1' : 'example.text.v1'}`;
        get('jobs-review-technical').append(outputType);
        (get('jobs-review-technical') as HTMLDetailsElement).open = !changed && technicalOpen;
        button('jobs-confirm').disabled = busy || state.phase !== 'review' || !state.estimate;
        button('jobs-edit').disabled = busy || state.phase === 'submitting';
        button('jobs-confirm').onclick = () => { void act(() => controller.submit()); };
        button('jobs-edit').onclick = edit;
      } else if (screen === 'dashboard') {
        text('jobs-heading', 'Jobs');
        pane.innerHTML = `<div id="jobs-history-items" class="jobs-list" aria-label="Recent jobs"></div><div class="actions"><button id="jobs-refresh-history" type="button">Refresh</button><button id="jobs-next" type="button">Next page</button></div><details><summary>History limits</summary><p class="footnote">History may be incomplete · <a href="https://github.com/aukilabs/auki-sdk/issues/396" target="_blank" rel="noreferrer">Provider issue</a></p></details>`;
        if (state.reconciliation) pane.prepend(facts({ 'Reconcile submission label': state.reconciliation.label, 'Original Domain': state.reconciliation.domainId }));
        for (const item of state.items ?? []) {
          const entry = document.createElement('button'); entry.type = 'button'; entry.dataset.jobId = item.job.id; entry.id = `jobs-row-${item.job.id}`;
          const cached = summaries.get(item.job.id), summary = cached?.evidence === evidence(item.job) ? cached : undefined;
          const date = item.job.created_at ? new Date(item.job.created_at) : undefined;
          const time = date && Number.isFinite(date.getTime()) ? date.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : 'Time unavailable';
          entry.textContent = String(redact(`${summary?.action || 'Demo job'} · ${item.job.status}\n${summary?.input ? summary.input + ' · ' : ''}${time}\n${summary?.result || 'Inspect details →'}`)); entry.disabled = busy;
          entry.onclick = () => {
            historyPage = { items: state.items ?? [], nextCursor: state.nextCursor,
              config: JSON.stringify(state.config), scroll: section.scrollTop, selected: entry.id };
            restoreHistory = false; invalidateReads(); override = undefined; polls = 0;
            void act(() => controller.inspect(item.job.id));
          };
          get('jobs-history-items').append(entry);
        }
        if (!state.items?.length) text('jobs-history-items', busy ? 'Loading recent jobs…' : state.reconciliation ? 'No matching jobs on this page. Use Recent jobs to check again for the saved submission label.' : state.config ? 'No matching jobs on this page. Start with New job.' : 'Choose an available worker installation to see recent jobs.');
        button('jobs-next').disabled = busy || !state.nextCursor;
        button('jobs-next').onclick = () => { historyPage = undefined; restoreHistory = false; void act(() => controller.list(state.nextCursor ?? undefined)); };
        button('jobs-refresh-history').disabled = busy;
        button('jobs-refresh-history').onclick = () => { void act(dashboard); };
      } else if (screen === 'uncertain') {
        text('jobs-heading', 'Check submission');
        pane.append(facts({ 'Original Domain': state.reconciliation?.domainId ?? state.domainId, 'Original environment': state.reconciliation?.environment ?? state.environment, 'Submission label': state.reconciliation?.label ?? state.spec?.label }));
        pane.insertAdjacentHTML('beforeend', '<p>The response was lost or could not be verified. The job may already exist. Use Recent jobs to reconcile this label in the original Domain before another submission. Only one unresolved submission is retained per session; new jobs are blocked until it is reconciled. Labels are not idempotency keys. Logout or reload erases this in-memory recovery.</p>');
      } else {
        text('jobs-heading', state.details?.job.status ?? 'Job details');
        const details = state.details;
        if (details) summaries.set(details.job.id, { action: state.role === 'compute' ? 'Uppercase text' : state.role === 'robot' ? 'Inspect file' : 'Demo job', evidence: evidence(details.job), input: names.get(String(details.tasks[0]?.meta.input_id)) || '', result: state.outputs?.length ? 'Verified output available' : 'No verified output' });
        const active = !['completed', 'failed', 'canceled'].includes(details?.job.status ?? '');
        const label = document.createElement('p');
        label.textContent = String(redact(`${state.role === 'compute' ? 'Uppercase text' : state.role === 'robot' ? 'Inspect file · simulated robot' : 'Demo job'} · ${names.get(String(details?.tasks[0]?.meta.input_id)) || 'Input details below'} · ${state.executorMatch ? 'Worker verified' : 'Unverified — outputs withheld'}`)); pane.append(label);
        pane.insertAdjacentHTML('beforeend', `<pre id="jobs-output-preview" class="jobs-preview" hidden></pre><div id="jobs-outputs" class="actions"></div><button id="jobs-output-retry" type="button" hidden>Retry preview</button><div class="actions"><button id="jobs-refresh" type="button">Refresh status</button><button id="jobs-cancel" type="button" ${active ? '' : 'hidden'}>Cancel whole job…</button></div>${active ? '<p class="footnote">Cancellation does not prove task shutdown or credit release. Automatic refresh pauses after 60 checks.</p>' : ''}<details><summary id="jobs-details">Details · redacted</summary><div id="jobs-tasks" class="jobs-list"></div><pre id="jobs-json"></pre></details>`);
        get('jobs-tasks').append(facts({ 'Input ID': details?.tasks[0]?.meta.input_id, 'Job label': details?.job.label, 'Locked credits': details?.job.credit_lock_amount, 'Credits released at': details?.job.credit_released_at }));
        for (const task of details?.tasks ?? []) get('jobs-tasks').append(facts({ Task: task.label, Status: task.status, Attempts: `${task.attempts} / ${task.max_attempts}`, Phase: phaseText((task.meta.progress as { phase?: unknown } | undefined)?.phase), 'Recent events': Array.isArray(task.meta.events) ? task.meta.events.slice(-3).map(event => typeof event === 'object' && event ? phaseText((event as { phase?: unknown }).phase) : 'Event').join(' → ') : 'Not reported' }));
        // Defense in depth: only completed tasks with matching actual receipts expose UUID links.
        if (state.executorMatch === true && state.expectedWorkerId) for (const receipt of details?.receipts ?? []) {
          const task = details?.tasks.find(task => task.id === receipt.task_id);
          if (task?.status !== 'completed' || receipt.node_id !== state.expectedWorkerId) continue;
          if (receipt.meta.status !== undefined && receipt.meta.status !== 'completed') continue;
          for (const ref of receipt.outputs) {
            if (!state.outputs?.includes(ref.toLowerCase())) continue;
            let id: string; try { id = uuid(ref); } catch { continue; }
            const output = document.createElement('button'); output.type = 'button'; output.dataset.jobOutput = id; output.className = 'primary'; output.textContent = 'Open in Data →'; output.disabled = busy;
            output.onclick = () => { void act(() => options.openRecord(id)); }; get('jobs-outputs').append(output); get('jobs-output-preview').hidden = false; text('jobs-output-preview', previewStates.get(id) || 'Loading preview…');
            button('jobs-output-retry').hidden = !previewStates.get(id)?.startsWith('Preview unavailable'); button('jobs-output-retry').onclick = () => { previewStates.delete(id); void previewRecord(id, true); };
          }
        }
        text('jobs-json', inspect({ domain: context?.domainId, config: state.config, details }));
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
    if (screen === 'dashboard' && restoreHistory && historyPage && !busy && !section.hidden) {
      section.querySelector<HTMLElement>(`#${historyPage.selected}`)?.focus({ preventScroll: true });
      section.scrollTop = historyPage.scroll;
    }
    const estimateButton = section.querySelector<HTMLButtonElement>('#jobs-estimate');
    if (estimateButton) estimateButton.disabled = busy || !context || !!state.discoveryRequired || !state.config?.[draftRole === 'compute' ? 'computeId' : 'robotId'];
  }
  function changeInput() {
    previewVersion++; draftInput = ''; field('jobs-input').value = '';
    selectedInput(); text('jobs-input-preview', 'Select a record to preview.');
    void loadPicker();
  }
  function edit() {
    const state = controller.state;
    draftInput = state.inputId ?? draftInput; draftRole = state.role ?? draftRole;
    invalidateReads();
    if (state.config) { override = 'choose'; screen = ''; void act(() => controller.choose()); } else { override = 'setup'; screen = ''; render(); }
  }
  button('jobs-new').onclick = () => { historyPage = undefined; restoreHistory = false; draftInput = ''; invalidateReads(); override = 'choose'; screen = ''; if (controller.state.config) void act(() => controller.choose()); else { override = 'setup'; render(); } };
  button('jobs-configure').onclick = () => { invalidateReads(); override = 'setup'; controller.choose(); screen = ''; render(); };
  button('jobs-discover').onclick = () => { void act(discover); };
  button('jobs-history').onclick = () => { void act(dashboard); };
  button('jobs-back').onclick = () => { void act(() => dashboard(true)); };
  function enterVisible() {
    if (section.hidden || !context || !ready) return;
    if (discoveryReady && !busy) { void act(finishDiscovery); return; }
    if (!entered && !busy) {
      entered = true;
      if (controller.state.reconciliation) { override = undefined; controller.restoreContext(); render(); }
      else { if (override !== 'choose') override = 'dashboard'; void act(discover); }
    } else { render(); activateReads(); schedule(); }
  }
  new MutationObserver(() => {
    if (section.hidden) {
      stopPolling();
      const auxiliary = section.dataset?.auxiliary === 'true';
      invalidateReads(auxiliary);
      if (auxiliary) {
        if (['review', 'choose'].includes(screen) && ['review', 'choose'].includes(controller.state.phase)) {
          draftInput = controller.state.inputId ?? draftInput; draftRole = controller.state.role ?? draftRole;
          override = 'choose'; controller.choose(); screen = '';
        }
      } else {
        draftInput = ''; override = 'dashboard'; historyPage = undefined; restoreHistory = false;
        if (['review', 'choose'].includes(controller.state.phase)) controller.choose();
      }
      render();
    } else enterVisible();
  }).observe(section, { attributes: true, attributeFilter: ['hidden'] });
  document.addEventListener('visibilitychange', schedule);
  function close(retainSession?: object) {
    historyPage = undefined; restoreHistory = false;
    ++generation; ready = false; entered = false; discoveryReady = false; invalidateReads(); names.clear(); previews.clear(); summaries.clear(); busy = false; stopPolling(); override = undefined; draftInput = ''; draftRole = 'compute'; screen = ''; polls = 0;
    context = getContext();
    const pending = Promise.allSettled([controller.close(retainSession), fleet.close(), records.close()]).then(results => { if (results.some(r => r.status === 'rejected')) throw Error('Jobs or Fleet cleanup failed.'); });
    closing = Promise.allSettled([closing, pending]).then(results => { if (results.some(result => result.status === 'rejected')) throw new Error('Jobs cleanup failed.'); });
    render(); return closing;
  }
  render();
  return {
    open(inputId) {
      if (inputId && !busy && !['uncertain', 'submitting'].includes(controller.state.phase)) { draftInput = uuid(inputId); override = 'choose'; screen = ''; if (controller.state.config) { override = 'choose'; void act(() => controller.choose()); } }
      options.navigate('jobs'); render(); activateReads(); get('jobs-heading').focus({ preventScroll: true });
    },
    close,
    refreshContext() {
      const next = getContext();
      if (next?.domainId !== context?.domainId || next?.session !== context?.session || next?.environment !== context?.environment || next?.data !== context?.data) {
        context = next; const pending = close(connection.session), version = generation;
        void pending.then(() => { if (version !== generation) return; ready = true; controller.restoreContext(); enterVisible(); }).catch(() => text('jobs-status', 'Jobs cleanup failed. Reload before reconnecting.'));
      } else { const version = generation; void closing.then(() => { if (version !== generation) return; ready = true; controller.restoreContext(); enterVisible(); render(); }).catch(() => text('jobs-status', 'Jobs cleanup failed. Reload before reconnecting.')); }
    },
  };
}
