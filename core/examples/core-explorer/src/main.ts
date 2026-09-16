import './style.css';
import { facts, technical } from './presentation';
import { networkingUI } from './network-ui';
import { Connection, login, type DataMetadata, type DataQuery, type DomainSummary } from './sdk';
import { endpoint, inspect, isLoopback, ReadLane, previewBytes, safeError, uuid, redact } from './safety';
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const input = (id: string) => $<HTMLInputElement>(id).value.trim();
const text = (id: string, value: string) => { $(id).textContent = value; };
const disabled = (id: string, value: boolean) => { $<HTMLButtonElement>(id).disabled = value; };
function showView(view: string, focus = false) {
  for (const name of ['overview', 'data', 'networking']) {
    $(`view-${name}`).hidden = name !== view;
    document.querySelector(`[data-view="${name}"]`)!.setAttribute('aria-current', name === view ? 'page' : 'false');
  }
  if (focus) $(`view-${view}`).querySelector<HTMLElement>('h1')!.focus();
}
document.querySelectorAll<HTMLButtonElement>('[data-view]').forEach(button => {
  button.onclick = () => showView(button.dataset.view!);
});
document.querySelectorAll<HTMLButtonElement>('[data-go]').forEach(button => {
  button.onclick = () => showView(button.dataset.go!, true);
});
const connection = new Connection();
const lanes = { domains: new ReadLane(), portals: new ReadLane(), poses: new ReadLane(), records: new ReadLane(), record: new ReadLane(), bytes: new ReadLane() };
let selection = 0, auth = 0, offset = 0;
let domainId = '', recordId = '';
let summaries: DomainSummary[] = [];
const networking = networkingUI(connection, () => domainId);
function clearRecord() {
  lanes.record.cancel(); lanes.bytes.cancel(); recordId = ''; $('record-jump').hidden = true;
  text('record', ''); text('content', 'Select a record.'); text('record-name', 'Choose a record'); $('record-facts').replaceChildren();
  $('records').querySelectorAll('button').forEach(button => button.setAttribute('aria-pressed', 'false'));
  for (const id of ['copy-record', 'preview', 'download']) disabled(id, true);
}
function clearSelection() {
  selection++;
  for (const key of ['portals', 'poses', 'records', 'record', 'bytes'] as const) lanes[key].cancel();
  domainId = ''; clearRecord();
  for (const id of ['metadata', 'portals', 'poses', 'records', 'data-status']) text(id, '');
  text('selected', 'Choose a Domain'); text('domain-name', 'Choose a Domain'); $('domain-facts').replaceChildren();
  $<HTMLDetailsElement>('domain-controls').open = true;
  networking.refresh();
}
async function logout() {
  auth++; for (const lane of Object.values(lanes)) lane.cancel(); clearSelection();
  $('workspace').hidden = true; $('access').hidden = false;
  text('domains', ''); summaries = []; text('session-status', 'Closing…');
  $<HTMLInputElement>('password').value = ''; disabled('signin', true);
  let message = 'Session closed.';
  try { await connection.close(); } catch { message = 'Session closed with a cleanup error. Reload before reconnecting.'; }
  text('session-status', 'Signed out'); text('login-status', message); disabled('signin', false);
}
$('logout').onclick = () => { void logout(); };
$('login').onsubmit = async event => {
  event.preventDefault();
  const attempt = ++auth;
  const password = $<HTMLInputElement>('password').value;
  $<HTMLInputElement>('password').value = '';
  try {
    const urls = ['api', 'dds', 'dms'].map(id => endpoint(input(id)));
    disabled('signin', true); text('login-status', 'Signing in…');
    const timer = setTimeout(() => { if (attempt === auth) void logout(); }, 30_000);
    try {
      if (!await connection.accept(login(urls, input('email'), password)) || attempt !== auth) return;
    } finally { clearTimeout(timer); }
    $('access').hidden = true; $('workspace').hidden = false;
    text('session-status', urls.every(url => isLoopback(new URL(url))) ? 'Local fixtures / synthetic test data' : 'Connected · configured environment');
    showView('overview'); offset = 0; loadDomains();
  } catch { if (attempt === auth) text('login-status', 'Sign-in failed. Check credentials and environment, then retry.'); }
  finally { if (attempt === auth) disabled('signin', false); }
};
for (const id of ['api', 'dds', 'dms']) $(id).oninput = () => {
  try { text('environment', ['api', 'dds', 'dms'].every(key => isLoopback(new URL(endpoint(input(key))))) ? 'Local fixtures / synthetic test data' : 'Explicit environment configuration · sign in to contact these services'); }
  catch { text('environment', 'Invalid endpoint: use HTTPS outside loopback; no URL credentials.'); }
};
function loadDomains() {
  const session = connection.session; if (!session) return;
  text('domain-status', 'Loading…'); text('domains', ''); text('page', ''); disabled('previous', true); disabled('next', true);
  void lanes.domains.run(signal => session.domains().list({ limit: 10, offset }, signal), page => {
    summaries = page.domains;
    text('domain-status', page.domains.length ? 'Select a Domain or enter a known UUID.' : 'Empty — no Domains on this page.');
    text('page', `${page.domains.length ? page.offset + 1 : 0}–${page.domains.length ? page.offset + page.domains.length : 0} / ${page.total}`);
    disabled('previous', page.offset === 0); disabled('next', page.offset + page.limit >= page.total);
    for (const domain of page.domains) {
      const button = document.createElement('button'); button.className = 'domain';
      button.textContent = String(redact(`${domain.name}\n${domain.id}`)); button.onclick = () => { void selectDomain(domain.id); };
      $('domains').append(button);
    }
  }, error => text('domain-status', safeError(error)));
}
$('refresh-domains').onclick = loadDomains;
$('previous').onclick = () => { offset = Math.max(0, offset - 10); loadDomains(); };
$('next').onclick = () => { offset += 10; loadDomains(); };
$('manual').onsubmit = event => { event.preventDefault(); try { void selectDomain(uuid(input('domain-id'))); } catch { text('domain-status', 'Enter a complete Domain UUID.'); } };
async function selectDomain(id: string) {
  clearSelection(); const version = selection; domainId = id;
  for (const key of ['name', 'type', 'ids']) $<HTMLInputElement>(key).value = '';
  const summary = summaries.find(item => item.id === id);
  text('selected', String(redact(summary?.name ?? 'Known Domain')));
  text('domain-name', String(redact(summary?.name ?? 'Known Domain')));
  $('domain-facts').replaceChildren(facts(summary ? { Name: summary.name, Organization: summary.organization_id } : { Metadata: 'Unavailable unless present in the current Domain page.' }));
  const picker = $<HTMLDetailsElement>('domain-controls');
  const focusPicker = picker.contains(document.activeElement); picker.open = false;
  if (focusPicker) picker.querySelector('summary')!.focus();
  text('metadata', inspect(summary ?? { id, note: 'Metadata unavailable unless present in the current Domain page.' }));
  const data = await connection.select(id);
  if (version !== selection) { await data?.close(); return; }
  connection.data = data;
  if (data) refreshSelected();
  networking.refresh();
}
function refreshSelected() {
  const data = connection.data, session = connection.session, id = domainId;
  if (!data || !session) return;
  text('portals', 'Loading…'); text('poses', 'Loading…');
  void lanes.portals.run(signal => session.domains().portals(id, signal), portals => {
    text('portals', portals.length ? '' : 'Empty — no portals.');
    for (const portal of portals) {
      const item = document.createElement('section'), title = document.createElement('h4'), copy = document.createElement('button');
      title.textContent = String(redact(portal.name)); copy.textContent = 'Copy portal ID'; copy.onclick = () => { void copyId(portal.id); };
      item.className = 'spatial-item';
      item.append(title, facts({ 'Short ID': portal.short_id, Size: portal.size, Created: portal.created_at, Updated: portal.updated_at }), copy, technical(portal));
      $('portals').append(item);
    }
  }, error => text('portals', safeError(error)));
  void lanes.poses.run(signal => data.poses(signal), poses => {
    text('poses', poses.length ? '' : 'Empty — no poses.');
    for (const pose of poses) {
      const item = document.createElement('section'); item.className = 'spatial-item';
      item.append(facts({ 'Short ID': pose.short_id, Position: { x: pose.px, y: pose.py, z: pose.pz }, Orientation: { x: pose.rx, y: pose.ry, z: pose.rz, w: pose.rw }, 'Placed at': pose.placed_at }), technical(pose));
      $('poses').append(item);
    }
  }, error => text('poses', safeError(error)));
  loadRecords();
}
$('refresh-selected').onclick = refreshSelected;
function loadRecords() {
  const data = connection.data; if (!data) return;
  clearRecord(); text('records', '');
  const query: DataQuery = {};
  if (input('name')) query.name = input('name');
  if (input('type')) query.dataType = input('type');
  try { if (input('ids')) query.ids = input('ids').split(',').map(uuid); }
  catch { lanes.records.cancel(); text('data-status', 'Enter comma-separated UUIDs.'); return; }
  text('data-status', 'Loading…');
  void lanes.records.run(signal => data.list(query, signal), records => {
    text('data-status', records.length ? `${records.length} records · permissions checked per read` : 'Empty — no matching records.');
    for (const record of records) {
      const button = document.createElement('button'), name = document.createElement('strong'), detail = document.createElement('small');
      button.className = 'record'; button.setAttribute('aria-pressed', 'false');
      name.textContent = String(redact(record.name)); detail.textContent = String(redact(`${record.data_type} · ${record.size} bytes`));
      button.append(name, detail);
      button.onclick = () => { selectRecord(record); button.setAttribute('aria-pressed', 'true'); };
      $('records').append(button);
    }
  }, error => text('data-status', safeError(error)));
}
$('filters').onsubmit = event => { event.preventDefault(); loadRecords(); };
for (const id of ['name', 'type', 'ids']) $(id).oninput = () => { lanes.records.cancel(); clearRecord(); text('records', ''); text('data-status', 'Apply filters to read matching records.'); };
function selectRecord(record: DataMetadata) {
  clearRecord(); recordId = record.id; $('record-jump').hidden = false;
  text('content', 'Choose Preview to read this record, or Download for original bytes.');
  for (const id of ['copy-record', 'preview', 'download']) disabled(id, false);
  const data = connection.data!; text('record', 'Loading…'); text('record-name', 'Loading…');
  void lanes.record.run(signal => data.get(record.id, signal), value => {
    text('record', inspect(value)); text('record-name', String(redact(value.name)));
    $('record-facts').replaceChildren(facts({ Name: value.name, Type: value.data_type, Bytes: value.size, Created: value.created_at, Updated: value.updated_at }));
  }, error => { text('record', safeError(error)); text('record-name', 'Metadata unavailable'); text('record-facts', safeError(error)); });
}
function readBytes(download: boolean) {
  const data = connection.data, id = recordId; if (!data || !id) return;
  text('content', 'Loading…');
  void lanes.bytes.run(signal => data.read(id, signal), bytes => {
    if (download) {
      const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'application/octet-stream' }));
      const anchor = document.createElement('a'); anchor.href = url; anchor.download = `${id}.bin`; anchor.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
      text('content', `Downloaded ${bytes.length} bytes.`);
    } else {
      text('content', previewBytes(bytes));
    }
  }, error => text('content', safeError(error)));
}
$('preview').onclick = () => readBytes(false); $('download').onclick = () => readBytes(true);
async function copyId(id: string) { try { await navigator.clipboard.writeText(id); text('notice', 'ID copied.'); } catch { text('notice', 'Copy unavailable. Select the visible ID to copy.'); } }
$('copy-domain').onclick = () => { if (domainId) void copyId(domainId); };
$('record-jump').onclick = () => { $('record-detail').focus(); };
$('copy-record').onclick = () => { if (recordId) void copyId(recordId); };
window.addEventListener('pagehide', () => { void logout(); });
