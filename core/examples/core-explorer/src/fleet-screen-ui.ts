import { aggregateFleet, FleetScreenController, type FleetMode } from './fleet-screen.ts';
import type { FleetContext } from './fleet.ts';
import { inspect, redact } from './safety.ts';
export function fleetScreenUI(context: () => FleetContext | undefined, navigate: (screen: string) => void) {
  const section = document.getElementById('view-fleet')!;
  let mode: FleetMode = 'all', selected = '', scroll = 0, returnJobs = false;
  const controller = new FleetScreenController(context, render);
  function node(tag: string, text: string, parent: HTMLElement) {
    const el = document.createElement(tag); el.textContent = String(redact(text)); parent.append(el); return el;
  }
  function button(text: string, parent: HTMLElement, action: () => void) {
    const el = node('button', text, parent) as HTMLButtonElement; el.type = 'button'; el.onclick = action; return el;
  }
  function render() {
    const focusedMode = (document.activeElement as HTMLElement | null)?.dataset.fleetMode;
    section.replaceChildren();
    const heading = node('div', '', section); heading.className = 'view-head';
    node('h1', 'Fleet', heading).tabIndex = -1;
    if (returnJobs) button('← Jobs', heading, () => { returnJobs = false; navigate('jobs'); });
    node('p', 'Domain robots + visible compute candidates', section).className = 'footnote';
    const ctx = context();
    if (!ctx) { node('p', 'Choose a Domain to inspect Fleet.', section); button('Choose Domain', section, () => navigate('domains')); return; }
    const result = aggregateFleet(ctx.domainId, controller.state.snapshots, mode);
    const machine = result.machines.find(m => m.id === selected);
    if (machine) {
      const back = button('← Fleet', section, () => { const id = selected; selected = ''; render(); section.scrollTop = scroll; section.querySelectorAll<HTMLButtonElement>('[data-machine]').forEach(b => { if (b.dataset.machine === id) b.focus({ preventScroll: true }); }); }); back.id = 'fleet-screen-back';
      node('h2', machine.name || 'Unnamed machine', section).tabIndex = -1;
      node('p', `${machine.kind} · ${['dedicated', 'public'].includes(machine.mode) ? machine.mode : 'Unknown mode'} · ${machine.association === 'candidate' ? 'Visible compute candidate' : machine.association === 'assigned' ? 'Assigned to this Domain' : 'Observed task in this Domain'}`, section);
      node('p', `Presence: ${machine.presence === 'unknown' ? 'Unknown' : machine.presence} · Work: ${machine.work_state === 'unknown' ? 'Unknown' : machine.work_state}`, section);
      node('h3', 'Capabilities', section); const caps = node('ul', '', section);
      for (const cap of machine.capabilities) node('li', cap, caps);
      if (!machine.capabilities.length) node('p', 'No capabilities reported.', section);
      node('h3', 'Authorized active work', section);
      for (const work of machine.activity) node('p', `${work.capability} · ${work.task_status} · updated ${work.updated_at}`, section);
      if (!machine.activity.length) node('p', 'No active tasks visible in authorized Domain observations.', section);
    } else {
      selected = '';
      const controls = node('div', '', section); controls.className = 'fleet-controls'; controls.setAttribute('role', 'group'); controls.setAttribute('aria-label', 'Fleet provider mode');
      for (const value of ['all', 'dedicated', 'public'] as const) {
        const control = button(value[0].toUpperCase() + value.slice(1), controls, () => { mode = value; selected = ''; if (controller.state.loading) void controller.refresh(); else render(); });
        control.dataset.fleetMode = value; control.setAttribute('aria-pressed', String(mode === value));
      }
      const refresh = button(controller.state.loading ? 'Reading…' : 'Refresh', section, () => { void controller.refresh(); }); refresh.id = 'fleet-screen-refresh'; refresh.disabled = controller.state.loading;
      const status = node('p', controller.state.message, section); status.id = 'fleet-screen-status'; status.setAttribute('role', 'status');
      const list = node('div', '', section); list.className = 'jobs-list';
      for (const m of result.machines) {
        const entry = button(`${m.name || 'Unnamed machine'} · ${m.kind} / ${['dedicated', 'public'].includes(m.mode) ? m.mode : 'Unknown mode'} · ${m.presence === 'unknown' ? 'Unknown' : m.presence} · work ${m.work_state === 'unknown' ? 'Unknown' : m.work_state}`, list, () => { scroll = section.scrollTop; selected = m.id; render(); section.scrollTop = 0; section.querySelector('h2')?.focus(); });
        entry.dataset.machine = m.id;
      }
      if (!controller.state.loading && controller.state.snapshots.length && !result.machines.length) node('p', result.complete && !controller.state.failures.length ? 'No machines in this view.' : 'No machines visible in available sources.', section);
    }
    const details = node('details', '', section); node('summary', 'Details · sources and IDs', details);
    node('pre', inspect({ machine, observations: controller.state.snapshots.map(s => ({ view: s.view, observed_at: s.observed_at, sources: s.sources, machines: machine ? s.machines.filter(m => m.id === machine.id) : undefined, unresolved_activity: s.unresolved_activity })), failures: controller.state.failures }), details);
    if (focusedMode) section.querySelector<HTMLButtonElement>(`[data-fleet-mode="${focusedMode}"]`)?.focus({ preventScroll: true });
  }
  document.addEventListener('visibilitychange', () => { if (document.hidden) void controller.close(false).catch(() => {}); });
  render();
  return { openFromJobs() { returnJobs = true; navigate('fleet'); render(); },
    visibility(visible: boolean) { if (!visible) void controller.close(false).catch(() => {}); },
    reset() { selected = ''; mode = 'all'; returnJobs = false; void controller.close().catch(() => {}); },
    close: () => controller.close(), render };
}
