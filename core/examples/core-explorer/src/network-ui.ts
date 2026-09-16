import { AukiDiscoveryMode, AukiPeerReachabilityMode, AukiEchoClient, type AukiPeer, type Connection } from './sdk';
import { facts, technical, shortPeerId } from './presentation';
import { Networking, diagnosticBytes } from './networking.ts';
import { inspect, previewBytes, safeError } from './safety';
export function networkingUI(connection: Connection, selectedDomain: () => string) {
  const section = document.createElement('section');
  // Static markup only. Every user/provider value below uses textContent/value.
  section.className = 'chapter-body';
  section.innerHTML = `<div class="network-bar"><span>Local peer <span id="net-state" class="badge" role="status">stopped</span></span><button id="net-stop" disabled>Stop networking</button></div>
    <p id="net-status" role="status">Choose a Domain to enable networking.</p>
    <div class="network-step" data-step="connect"><h2><span class="step-number">1</span>Start a session</h2><div class="step-body">
    <p>Start an outbound peer in the selected Domain. Data reads work without networking.</p><button id="net-start" class="primary" disabled>Start networking</button></div></div>
    <div class="network-step" data-step="discover"><h2><span class="step-number">2</span>Choose a candidate</h2><div class="step-body">
    <p>Discovery does not prove a candidate is online or authorized.</p><button id="net-discover" disabled>Discover Echo candidates</button><div id="net-candidates"></div>
    <details id="manual-route"><summary>Advanced · enter a manual route</summary>
    <label>Target Peer ID<input id="net-peer-id" autocomplete="off"></label><label>Target WSS relay route<input id="net-route" autocomplete="off"></label><button id="net-use-manual" disabled>Use manual target</button></details>
    </div></div>
    <div class="network-step" data-step="diagnostic"><h2><span class="step-number">3</span>Send a diagnostic</h2><div class="step-body">
    <p id="net-target">Choose a target to continue.</p><button id="net-reselect" disabled>Choose another target</button>
    <label>Diagnostic · 1–1024 UTF-8 bytes<input id="net-payload" value="Core Explorer diagnostic" autocomplete="off"></label>
    <button id="net-send" class="primary" disabled>Send verified Echo</button></div></div>
    <div id="net-results" role="status"></div>
    <p class="footnote">Echo checks the authenticated roundtrip only. No robot controls, task dispatch or application authority.</p>
    <details id="network-technical"><summary>Technical details · local peer and target</summary><pre id="net-local"></pre><pre id="net-target-details"></pre></details>`;
  document.querySelector('#view-networking')!.append(section);
  const get = (id: string) => section.querySelector<HTMLElement>(`#${id}`)!;
  const field = (id: string) => get(id) as HTMLInputElement;
  const button = (id: string) => get(id) as HTMLButtonElement;
  let discovering = false, sending = false, resultVersion = 0;
  let resultKey = '';
  let targetConfirmed = false;
  const identity = () => JSON.stringify([selectedDomain(), field('net-peer-id').value.trim(), field('net-route').value.trim(), field('net-payload').value]);
  function invalidateResult() { resultVersion++; resultKey = ''; get('net-results').textContent = ''; }
  const net = new Networking<AukiPeer, AukiEchoClient>(render);
  function render() {
    get('net-state').textContent = net.state;
    button('net-start').disabled = !connection.session || !selectedDomain() || !['stopped', 'failed'].includes(net.state);
    button('net-stop').disabled = !['starting', 'ready'].includes(net.state);
    button('net-discover').disabled = net.state !== 'ready' || discovering;
    const fieldsComplete = !!field('net-peer-id').value.trim() && !!field('net-route').value.trim();
    const target = targetConfirmed && fieldsComplete;
    button('net-use-manual').disabled = net.state !== 'ready' || !fieldsComplete;
    let validPayload = false;
    try { diagnosticBytes(field('net-payload').value); validPayload = !!field('net-payload').value.trim(); } catch { /* Keep send disabled. */ }
    button('net-send').disabled = net.state !== 'ready' || !target || !validPayload || sending;
    button('net-reselect').disabled = net.state !== 'ready' || !target;
    for (const id of ['net-peer-id', 'net-route']) field(id).disabled = net.state !== 'ready';
    field('net-payload').disabled = net.state !== 'ready' || !target;
    const stage = net.state !== 'ready' ? 'connect' : target ? 'diagnostic' : 'discover';
    section.querySelectorAll<HTMLElement>('[data-step]').forEach(step => {
      step.classList.toggle('active', step.dataset.step === stage);
      step.setAttribute('aria-current', step.dataset.step === stage ? 'step' : 'false');
      step.querySelector<HTMLElement>('.step-body')!.hidden = step.dataset.step !== stage;
    });
    get('net-target').textContent = target ? `Selected target · ${shortPeerId(field('net-peer-id').value.trim())}` : 'Choose a target to continue.';
    get('net-target-details').textContent = fieldsComplete ? inspect({ targetPeerId: field('net-peer-id').value.trim(), route: field('net-route').value.trim() }) : '';
    if (resultKey && resultKey !== identity()) invalidateResult();
    get('net-local').textContent = net.peer && net.state === 'ready' ? inspect({
      domainId: net.peer.domainId, localPeerId: net.peer.peerId,
      reachability: 'Outbound only', wssRoute: net.peer.wssRoute ?? null, tcpRoute: net.peer.tcpRoute ?? null,
    }) : inspect({ domainId: selectedDomain() || null });
    if (net.state !== 'ready') {
      targetConfirmed = false;
      get('net-candidates').replaceChildren(); invalidateResult();
      field('net-peer-id').value = field('net-route').value = '';
      get('net-target-details').textContent = '';
      get('net-target').textContent = 'Choose a target to continue.';
      get('net-status').textContent = net.state === 'failed' ? safeError(undefined)
        : net.state === 'stopping' ? 'Awaiting pending work and peer cleanup…'
        : net.state === 'starting' ? 'Starting local peer…'
        : 'Networking stopped. Data reads remain available.';
    }
  }
  for (const id of ['net-peer-id', 'net-route', 'net-payload']) field(id).oninput = () => {
    if (id !== 'net-payload') targetConfirmed = false;
    invalidateResult();
    get('net-status').textContent = 'Diagnostic changed. Send to verify the current target and message.';
    render();
  };
  button('net-reselect').onclick = () => {
    targetConfirmed = false; resultVersion++; get('net-status').textContent = 'Choose a candidate or edit and confirm a manual route.'; render(); button('net-discover').focus();
  };
  button('net-use-manual').onclick = () => {
    if (button('net-use-manual').disabled) return;
    targetConfirmed = true; invalidateResult(); render(); field('net-payload').focus();
  };
  connection.beforeClose = () => net.stop();
  button('net-start').onclick = () => {
    const session = connection.session, domain = selectedDomain();
    if (!session || !domain) return;
    void net.start(() => session.startPeerWithDiscovery(domain, AukiDiscoveryMode.DiscoverOnly, AukiPeerReachabilityMode.OutboundOnly), peer => new AukiEchoClient(peer))
      .then(() => { if (net.state === 'ready') get('net-status').textContent = 'Local peer ready. Target availability and authorization are checked only by a successful Echo response.'; });
  };
  button('net-stop').onclick = () => { void net.stop(); };
  button('net-discover').onclick = async () => {
    if (discovering) return;
    discovering = true; render(); get('net-candidates').replaceChildren();
    await net.run((peer, client) => peer.discoverProtocol(client.protocol), candidates => {
      get('net-status').textContent = `${candidates.length} discovered candidates · availability and authorization unverified`;
      for (const candidate of candidates) {
        const row = document.createElement('div'), summary = document.createElement('p');
        summary.textContent = 'Discovered candidate · unverified';
        row.append(summary, facts({ 'Peer ID': shortPeerId(candidate.peerId) }), technical({ peerId: candidate.peerId, routes: candidate.routes, expiresAt: candidate.expiresAt, source: candidate.source }));
        for (const route of candidate.routes.filter(route => /^\/dns4\/[^/]+\/tcp\/\d+\/wss\/p2p\/[^/]+\/p2p-circuit\/p2p\/[^/]+$/.test(route))) {
          const choose = document.createElement('button'), peerId = candidate.peerId;
          choose.textContent = 'Use candidate route';
          choose.onclick = () => { if (net.state !== 'ready' || sending) return; field('net-peer-id').value = peerId; field('net-route').value = route; targetConfirmed = true; invalidateResult(); get('net-status').textContent = 'Candidate selected. Send a diagnostic to verify this target.'; render(); field('net-payload').focus(); };
          row.append(choose);
        }
        get('net-candidates').append(row);
      }
    }, () => { get('net-status').textContent = safeError(undefined); }, candidates => { for (const candidate of candidates) candidate.free(); });
    discovering = false; render();
  };
  button('net-send').onclick = async () => {
    if (sending || net.state !== 'ready' || !targetConfirmed) return;
    let payload: Uint8Array;
    try { if (!field('net-payload').value.trim()) throw new Error(); payload = diagnosticBytes(field('net-payload').value); }
    catch { get('net-status').textContent = 'Enter 1–1024 UTF-8 bytes for the diagnostic.'; return; }
    const peerId = field('net-peer-id').value.trim(), route = field('net-route').value.trim();
    if (!peerId || !route) { get('net-status').textContent = 'Enter a target Peer ID and WSS relay route.'; return; }
    invalidateResult(); const version = resultVersion, key = identity(), domain = selectedDomain();
    const current = () => version === resultVersion && key === identity();
    sending = true; render();
    await net.run((_peer, client) => client.sendExact(peerId, route, payload), receipt => {
      if (!current()) return;
      resultKey = key;
      const heading = document.createElement('h3'), count = document.createElement('p'), payload = document.createElement('pre');
      heading.textContent = 'Verified Echo response';
      count.textContent = `${receipt.payload.length} bytes echoed`;
      payload.textContent = previewBytes(receipt.payload);
      get('net-results').replaceChildren(heading, count, payload, technical({ remotePeerId: receipt.remotePeerId, domainId: domain, bytes: receipt.payload.length }));
      get('net-status').textContent = 'Authenticated diagnostic roundtrip completed. This grants no application or task authority.';
    }, () => { if (current()) get('net-status').textContent = safeError(undefined); }, receipt => receipt.free());
    sending = false; render();
  };
  render();
  return { refresh: render };
}
