import './fleet-screen.css';
import type { FleetMachine } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import { aggregateFleet, FleetScreenController, type FleetMode } from './fleet-screen.ts';
import type { FleetContext } from './fleet.ts';
import { fleetAssociation, fleetCapability, fleetObservedAt, fleetOverview, fleetSourceNote, fleetTime } from './fleet-presentation.ts';
import { inspect, redact } from './safety.ts';

type Icon = 'bot' | 'server' | 'refresh-cw' | 'arrow-up-right' | 'arrow-left' | 'x' | 'activity' | 'clock' | 'info';

export function fleetScreenUI(context: () => FleetContext | undefined, navigate: (screen: string) => void) {
  const section = document.getElementById('view-fleet')!;
  let mode: FleetMode = 'all', selected = '', scroll = 0, returnJobs = false;
  let showOffline = false;
  const controller = new FleetScreenController(context, render);

  function node<K extends keyof HTMLElementTagNameMap>(tag: K, text: string, parent: HTMLElement, className = '') {
    const el = document.createElement(tag);
    el.textContent = String(redact(text));
    el.className = className;
    parent.append(el);
    return el;
  }
  function button(text: string, parent: HTMLElement, action: () => void, className = '') {
    const el = node('button', text, parent, className);
    el.type = 'button';
    el.onclick = action;
    return el;
  }
  function icon(name: Icon, parent: HTMLElement) {
    const el = node('span', '', parent, `fleet-icon fleet-icon-${name}`);
    el.setAttribute('aria-hidden', 'true');
    return el;
  }
  function badge(text: string, state: string, parent: HTMLElement) {
    return node('span', text, parent, `fleet-badge fleet-badge-${state}`);
  }
  function modeLabel(value: string) {
    return value === 'dedicated' ? 'Dedicated' : value === 'public' ? 'Public' : 'Unknown mode';
  }

  // Keep the heading, refresh control and live region mounted across reads.
  // Rebuilding these controls used to discard keyboard focus after Refresh.
  section.classList.add('fleet-view');
  section.replaceChildren();
  const heading = node('div', '', section, 'fleet-heading');
  const title = node('div', '', heading);
  node('h1', 'Fleet', title).tabIndex = -1;
  node('p', 'Robots in this Domain and visible compute nodes.', title, 'fleet-subtitle');
  const actions = node('div', '', heading, 'fleet-heading-actions');
  const backJobs = button('← Jobs', actions, () => { returnJobs = false; navigate('jobs'); });
  const refresh = button('', actions, () => {
    if (controller.state.loading) return;
    const restore = document.activeElement === refresh;
    void controller.refresh().finally(() => {
      if (restore && !section.hidden && document.activeElement === document.body) refresh.focus({ preventScroll: true });
    });
  }, 'fleet-refresh');
  refresh.id = 'fleet-screen-refresh';
  icon('refresh-cw', refresh);
  const refreshLabel = node('span', 'Refresh', refresh);
  const status = node('p', '', section, 'fleet-status');
  status.id = 'fleet-screen-status';
  status.setAttribute('role', 'status');
  const content = node('div', '', section, 'fleet-content');

  function diagnostics(parent: HTMLElement, machine?: FleetMachine) {
    const details = node('details', '', parent, 'fleet-diagnostics');
    node('summary', 'Details · sources and IDs', details);
    node('pre', inspect({
      machine,
      observations: controller.state.snapshots.map(s => ({
        view: s.view, observed_at: s.observed_at, sources: s.sources,
        machines: machine ? s.machines.filter(m => m.id === machine.id) : undefined,
        unresolved_activity: s.unresolved_activity,
      })),
      failures: controller.state.failures,
    }), details);
  }

  function overview(parent: HTMLElement, machines: FleetMachine[], partial: boolean) {
    const counts = fleetOverview(machines);
    const region = node('section', '', parent, 'fleet-overview');
    region.setAttribute('aria-label', 'Fleet overview');
    const facts = [
      ['robots', 'Domain robots', counts.robots, 'In this Domain', 'bot'],
      ['compute', 'Compute nodes', counts.compute, 'Visible to this account', 'server'],
      ['online', 'Online', counts.online, `${counts.offline} offline · ${counts.presenceUnknown} unknown`, 'activity'],
      ['busy', 'Busy', counts.busy, `${counts.idle} idle · ${counts.workUnknown} unknown`, 'clock'],
    ] as const;
    for (const [key, label, value, note, symbol] of facts) {
      const item = node('div', '', region, 'fleet-stat');
      item.dataset.fleetStat = key;
      const caption = node('div', '', item, 'fleet-stat-label');
      icon(symbol, caption); node('span', label, caption);
      node('strong', String(value), item, 'fleet-stat-value');
      node('span', note, item, 'fleet-stat-note');
    }
    const caption = node('p', '', parent, 'fleet-overview-note');
    icon('info', caption);
    node('span', partial ? 'Visible observations · some sources are incomplete' : 'Snapshot observations · presence and work are separate states', caption);
  }

  function selectMachine(id: string) {
    scroll = section.scrollTop;
    selected = id;
    render();
    if (!window.matchMedia('(min-width: 1100px)').matches) section.scrollTop = 0;
    section.querySelector<HTMLElement>('#fleet-inspector-title')?.focus({ preventScroll: true });
  }

  function closeMachine() {
    const id = selected;
    selected = '';
    render();
    section.scrollTop = scroll;
    section.querySelectorAll<HTMLButtonElement>('[data-machine]').forEach(card => {
      if (card.dataset.machine === id) card.focus({ preventScroll: true });
    });
  }

  function machineCard(machine: FleetMachine, index: number, parent: HTMLElement) {
    const card = button('', parent, () => selectMachine(machine.id), 'fleet-machine');
    card.dataset.machine = machine.id;
    card.dataset.kind = machine.kind;
    card.setAttribute('aria-expanded', String(selected === machine.id));
    if (selected === machine.id) card.setAttribute('aria-controls', 'fleet-inspector');
    card.setAttribute('aria-label', String(redact(`View ${machine.name || 'unnamed machine'}`)));
    card.setAttribute('aria-describedby', `fleet-machine-presence-${index} fleet-machine-work-${index} fleet-machine-capability-${index}`);
    const top = node('span', '', card, 'fleet-card-top');
    const typeIcon = node('span', '', top, 'fleet-machine-icon');
    icon(machine.kind === 'robot' ? 'bot' : 'server', typeIcon);
    badge(machine.presence === 'unknown' ? 'Presence unknown' : machine.presence === 'online' ? 'Online' : 'Offline', machine.presence, top).id = `fleet-machine-presence-${index}`;
    const identity = node('span', '', card, 'fleet-card-identity');
    node('span', `${machine.kind === 'robot' ? 'Robot' : 'Compute'} · ${modeLabel(machine.mode)}`, identity, 'fleet-eyebrow');
    const name = node('strong', machine.name || 'Unnamed machine', identity, 'fleet-machine-name');
    name.id = `fleet-machine-name-${index}`;
    node('span', fleetAssociation(machine), identity, 'fleet-card-association');
    const capability = node('span', '', card, 'fleet-card-capability');
    capability.id = `fleet-machine-capability-${index}`;
    const first = machine.capabilities[0];
    const label = first ? fleetCapability(first).name : 'No capabilities reported';
    const purpose = node('span', label, capability);
    purpose.title = String(redact(label));
    if (machine.capabilities.length > 1) node('span', `+${machine.capabilities.length - 1}`, capability, 'fleet-capability-count');
    const footer = node('span', '', card, 'fleet-card-footer');
    badge(machine.work_state === 'unknown' ? 'Work unknown' : machine.work_state === 'busy' ? 'Busy' : 'Idle', machine.work_state, footer).id = `fleet-machine-work-${index}`;
    const affordance = node('span', selected === machine.id ? 'Selected' : 'View details', footer, 'fleet-card-action');
    icon('arrow-up-right', affordance);
  }

  function inspector(machine: FleetMachine, parent: HTMLElement) {
    const panel = node('aside', '', parent, 'fleet-inspector');
    panel.id = 'fleet-inspector';
    panel.setAttribute('aria-labelledby', 'fleet-inspector-title');
    const toolbar = node('div', '', panel, 'fleet-inspector-toolbar');
    node('span', 'Machine details', toolbar, 'fleet-eyebrow');
    const close = button('', toolbar, closeMachine, 'fleet-inspector-close');
    close.id = 'fleet-screen-back';
    close.setAttribute('aria-label', 'Back to fleet overview');
    icon('arrow-left', close).classList.add('fleet-mobile-back');
    node('span', 'Fleet', close, 'fleet-mobile-back');
    icon('x', close).classList.add('fleet-desktop-close');
    const identity = node('div', '', panel, 'fleet-inspector-identity');
    identity.dataset.kind = machine.kind;
    const typeIcon = node('div', '', identity, 'fleet-machine-icon');
    icon(machine.kind === 'robot' ? 'bot' : 'server', typeIcon);
    node('p', `${machine.kind === 'robot' ? 'Robot' : 'Compute node'} · ${modeLabel(machine.mode)}`, identity, 'fleet-eyebrow');
    const name = node('h2', machine.name || 'Unnamed machine', identity);
    name.id = 'fleet-inspector-title'; name.tabIndex = -1;
    node('p', fleetAssociation(machine), identity, 'fleet-card-association');
    const states = node('dl', '', panel, 'fleet-machine-states');
    for (const [label, value] of [['Presence', machine.presence], ['Work', machine.work_state]]) {
      const item = node('div', '', states);
      node('dt', label, item);
      badge(value[0].toUpperCase() + value.slice(1), value, node('dd', '', item));
    }
    const capabilities = node('section', '', panel, 'fleet-inspector-section');
    node('h3', 'Capabilities', capabilities);
    if (!machine.capabilities.length) node('p', 'No capabilities reported.', capabilities, 'fleet-muted');
    for (const capability of machine.capabilities) {
      const description = fleetCapability(capability);
      const item = node('div', '', capabilities, 'fleet-capability');
      node('strong', description.name, item);
      node('p', description.description, item, 'fleet-muted');
    }
    const activity = node('section', '', panel, 'fleet-inspector-section');
    node('h3', 'Active work', activity);
    for (const work of machine.activity) {
      const item = node('div', '', activity, 'fleet-activity');
      const heading = node('div', '', item, 'fleet-activity-heading');
      node('strong', fleetCapability(work.capability).name, heading);
      badge(work.task_status, 'neutral', heading);
      node('p', `Updated ${fleetTime(work.updated_at)}`, item, 'fleet-muted');
    }
    if (!machine.activity.length) {
      const empty = node('div', '', activity, 'fleet-work-empty');
      icon('activity', empty);
      node('p', machine.work_state === 'unknown' ? 'Work status is unknown.' : 'No active tasks visible.', empty);
      node('p', 'Only authorized activity in this Domain is shown.', empty, 'fleet-muted');
    }
    const observations = node('dl', '', panel, 'fleet-observation-times');
    for (const [label, value] of [['Presence observed', machine.presence_observed_at], ['Work observed', machine.work_observed_at], ...(machine.last_seen_at ? [['Last seen', machine.last_seen_at]] : [])]) {
      const row = node('div', '', observations);
      node('dt', label!, row);
      const time = node('dd', fleetTime(value), row);
      if (value) time.title = value;
    }
    node('p', 'Observed status does not reserve capacity or guarantee job eligibility.', panel, 'fleet-inspector-note');
    diagnostics(panel, machine);
  }

  function render() {
    const focused = document.activeElement as HTMLElement | null;
    const focusedMode = focused?.dataset.fleetMode;
    const focusedMachine = focused?.dataset.machine;
    const focusedOffline = focused?.id === 'fleet-show-offline';
    const ctx = context();
    const { snapshots, failures, loading } = controller.state;
    const initial = controller.state.message === 'Choose Refresh to read Fleet.';
    const cleanupFailed = controller.state.message.startsWith('Fleet cleanup failed');
    backJobs.hidden = !returnJobs;
    refresh.hidden = !ctx;
    refresh.disabled = loading || cleanupFailed;
    refreshLabel.textContent = loading ? 'Reading…' : 'Refresh';
    refresh.setAttribute('aria-busy', String(loading));
    content.replaceChildren();
    section.dataset.hasSelection = 'false';
    status.textContent = ctx ? controller.state.message : '';
    status.className = 'fleet-status';
    if (!ctx) {
      const empty = node('div', '', content, 'fleet-empty');
      icon('bot', empty);
      node('h2', 'Choose a Domain', empty);
      node('p', 'See its robots and the compute nodes visible to your account.', empty);
      button('Choose Domain', empty, () => navigate('domains'), 'primary');
      return;
    }
    const all = aggregateFleet(ctx.domainId, snapshots);
    const result = aggregateFleet(ctx.domainId, snapshots, mode);
    const visible = all.machines.filter(m => showOffline || m.presence === 'online');
    const machines = result.machines.filter(m => showOffline || m.presence === 'online');
    const machine = machines.find(m => m.id === selected);
    if (!loading && !machine) selected = '';
    section.dataset.hasSelection = String(Boolean(machine));
    const partial = snapshots.length > 0 && (!all.complete || failures.length > 0);
    if (partial) {
      status.textContent = `Partial observations · ${fleetSourceNote(snapshots, failures)}`;
      status.classList.add('fleet-status-partial');
      node('span', `Observed ${fleetTime(fleetObservedAt(snapshots))}`, status, 'fleet-status-time');
    }
    else if (snapshots.length) status.textContent = `Observed ${fleetTime(fleetObservedAt(snapshots))}`;
    else if (initial) status.textContent = 'Read-only inventory · refresh when you need a new snapshot';
    if (!snapshots.length) {
      const empty = node('div', '', content, 'fleet-empty');
      icon(loading ? 'refresh-cw' : 'bot', empty);
      node('h2', loading ? 'Reading your fleet' : cleanupFailed ? 'Fleet cleanup failed' : failures.length ? 'Fleet could not be loaded' : 'Your fleet at a glance', empty);
      node('p', loading ? 'Gathering machine inventory and authorized activity.' : cleanupFailed ? 'Reload before reconnecting.' : failures.length ? 'Check the source details, then use Refresh to try again.' : 'Refresh to see robots, compute nodes and their observed activity in one place.', empty);
      if (failures.length) diagnostics(content);
      return;
    }
    const summary = node('div', '', content, 'fleet-summary');
    overview(summary, all.machines, partial);
    const layout = node('div', '', content, 'fleet-layout');
    const inventory = node('section', '', layout, 'fleet-inventory');
    inventory.setAttribute('aria-labelledby', 'fleet-machines-title');
    const toolbar = node('div', '', inventory, 'fleet-toolbar');
    const title = node('h2', 'Machines', toolbar);
    title.id = 'fleet-machines-title';
    node('span', String(machines.length), title, 'fleet-list-count');
    const controls = node('div', '', toolbar, 'fleet-controls');
    controls.setAttribute('role', 'group'); controls.setAttribute('aria-label', 'Fleet provider mode');
    for (const value of ['all', 'dedicated', 'public'] as const) {
      const count = visible.filter(m => value === 'all' || m.mode === value).length;
      const control = button(value === 'all' ? 'All' : modeLabel(value), controls, () => { mode = value; selected = ''; render(); });
      control.dataset.fleetMode = value;
      control.setAttribute('aria-pressed', String(mode === value));
      node('span', String(count), control, 'fleet-filter-count').setAttribute('aria-hidden', 'true');
    }
    const filters = node('div', '', inventory, 'fleet-presence-filter');
    const label = node('label', '', filters, 'fleet-checkbox');
    const checkbox = node('input', '', label);
    checkbox.id = 'fleet-show-offline'; checkbox.type = 'checkbox'; checkbox.checked = showOffline;
    checkbox.setAttribute('aria-describedby', 'fleet-presence-note');
    checkbox.onchange = () => { showOffline = checkbox.checked; render(); };
    node('span', 'Show offline machines', label);
    node('span', showOffline ? 'Includes unknown presence' : 'Showing online machines', filters, 'fleet-muted').id = 'fleet-presence-note';
    const cards = node('div', '', inventory, 'fleet-grid');
    machines.forEach((m, index) => machineCard(m, index, cards));
    if (!machines.length) {
      const empty = node('div', '', inventory, 'fleet-empty fleet-empty-filtered');
      icon('server', empty);
      const hidden = !showOffline && result.machines.length > 0;
      node('h3', hidden ? 'No online machines in this view.' : result.complete && !failures.length ? 'No machines in this view.' : 'No machines visible in available sources.', empty);
      node('p', hidden ? `${result.machines.length} machines have offline or unknown presence. Select Show offline machines to include them.` : mode !== 'all' && visible.length ? 'Try All to see the rest of the visible fleet.' : 'Check the selected Domain or refresh after a worker is registered.', empty);
      if (mode !== 'all') button('Show all modes', empty, () => { mode = 'all'; render(); });
    }
    if (machine) inspector(machine, content);
    else diagnostics(inventory);
    if (!section.hidden) {
      if (focusedMode) section.querySelector<HTMLButtonElement>(`[data-fleet-mode="${focusedMode}"]`)?.focus({ preventScroll: true });
      else if (focusedOffline) section.querySelector<HTMLInputElement>('#fleet-show-offline')?.focus({ preventScroll: true });
      else if (focusedMachine) section.querySelectorAll<HTMLButtonElement>('[data-machine]').forEach(card => {
        if (card.dataset.machine === focusedMachine) card.focus({ preventScroll: true });
      });
    }
  }

  document.addEventListener('visibilitychange', () => {
    if (document.hidden) void controller.close(false).catch(() => {});
    else if (!section.hidden) void controller.enter();
  });
  render();
  return {
    openFromJobs() { returnJobs = true; navigate('fleet'); render(); },
    visibility(visible: boolean) { if (visible) void controller.enter(); else void controller.close(false).catch(() => {}); },
    reset() { selected = ''; mode = 'all'; showOffline = false; returnJobs = false; void controller.close().catch(() => {}); },
    close: () => controller.close(), render,
  };
}
