import { FleetController, type Installation } from './fleet.ts';
import { inspect, redact } from './safety.ts';
/** All provider labels are inert, redacted text. IDs and source details are secondary. */
export function renderFleet(host: HTMLElement, fleet: FleetController, select: (choice: Installation) => void, refresh: () => void) {
  host.innerHTML = '<p class="footnote">Assigned robots and dedicated compute workers. Choose one installation.</p><button id="fleet-refresh" type="button">Refresh</button><p id="fleet-status" role="status"></p><div id="fleet-choices" class="jobs-list"></div><details><summary>Details · sources and public IDs</summary><pre id="fleet-diagnostics"></pre></details>';
  const state = fleet.state;
  const button = host.querySelector<HTMLButtonElement>('#fleet-refresh')!; button.disabled = state.loading; button.onclick = refresh;
  host.querySelector('#fleet-status')!.textContent = state.message;
  for (const [index, choice] of state.installations.entries()) {
    const card = document.createElement('div'); card.className = 'jobs-card';
    const label = document.createElement('p');
    const names = [...choice.compute, ...choice.robot].map(m => m.name).join(' + ');
    label.textContent = String(redact(`Installation ${index + 1} · ${names}`)); card.append(label);
    for (const role of ['compute', 'robot'] as const) {
      const observation = document.createElement('p'); observation.className = 'footnote';
      observation.textContent = String(redact(`${role === 'compute' ? 'Dedicated compute candidates' : 'Assigned robots'}: ${choice[role].map(m => `${m.name} · ${m.presence} · work ${m.work_state}`).join('; ') || 'Not discovered'}`)); card.append(observation);
    }
    for (const issue of choice.issues) { const p = document.createElement('p'); p.textContent = issue; card.append(p); }
    const use = document.createElement('button'); use.type = 'button'; use.dataset.fleetInstallation = choice.config.installationId;
    use.textContent = 'Use installation'; use.disabled = !choice.config.computeId && !choice.config.robotId;
    use.onclick = () => select(choice); card.append(use); host.querySelector('#fleet-choices')!.append(card);
  }
  host.querySelector('#fleet-diagnostics')!.textContent = inspect({ inventory: state.inventory, dedicated_pool: state.pool });
  const note = document.createElement('p'); note.className = 'footnote'; note.textContent = 'Online, offline, busy and unknown are observations, not reservations or scheduling guarantees. No workers are started.'; host.append(note);
}
