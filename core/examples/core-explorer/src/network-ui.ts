import { AukiDiscoveryMode, AukiPeerReachabilityMode, AukiEchoClient, type AukiPeer, type Connection } from './sdk';
import { facts, shortPeerId } from './presentation';
import { Networking, diagnosticBytes } from './networking.ts';
import { inspect, previewBytes, safeError } from './safety';
export function networkingUI(connection: Connection, selectedDomain: () => string) {
  const section = document.createElement('section');
  // Static markup only. Every user/provider value below uses textContent/value.
  section.className = 'task-body';
  section.innerHTML = `<div class="network-bar"><span>Local peer <span id="net-state" class="badge" role="status">stopped</span></span><button id="net-stop" disabled>Stop</button></div>
    <p id="net-status" role="status">Choose a Domain to enable networking.</p>
    <div class="actions"><button id="net-back" hidden>Back</button><button id="net-show-technical">Details</button><button id="net-show-result" hidden>View verified response</button></div>
    <div class="network-step" data-network-screen="connect" data-step="connect"><h2 tabindex="-1">Connect</h2><div class="step-body">
    <button id="net-start" class="primary" disabled>Connect</button></div></div>
    <div class="network-step" data-network-screen="discover" data-step="discover" hidden><h2 tabindex="-1">Choose a target</h2><div class="step-body">
    <p>Targets may be offline or unauthorized.</p><button id="net-discover" disabled>Find targets</button><button id="net-show-manual" disabled>Enter a manual route</button><div id="net-candidates"></div>
    </div></div>
    <div id="manual-route" class="network-step" data-network-screen="manual" hidden><h2 tabindex="-1">Enter a manual route</h2>
    <p>Check the Peer ID and WSS relay route.</p>
    <label>Target Peer ID<input id="net-peer-id" autocomplete="off"></label><label>Target WSS relay route<input id="net-route" autocomplete="off"></label><button id="net-use-manual" disabled>Use manual target</button></div>
    <div class="network-step" data-network-screen="diagnostic" data-step="diagnostic" hidden><h2 tabindex="-1">Send Echo</h2><div class="step-body">
    <p id="net-target">Choose a target to continue.</p><button id="net-reselect" disabled>Choose another target</button>
    <label>Diagnostic · 1–1024 UTF-8 bytes<input id="net-payload" value="Core Explorer diagnostic" autocomplete="off"></label>
    <button id="net-send" class="primary" disabled>Send verified Echo</button></div></div>
    <div class="network-step" data-network-screen="result" hidden><h2 tabindex="-1">Diagnostic result</h2><button id="net-result-back">Back</button><div id="net-results" role="status"></div></div>

    <div id="network-technical" class="network-step" data-network-screen="technical" hidden><h2 tabindex="-1">Connection details</h2><p class="footnote">Echo verifies an authenticated roundtrip. It grants no task or application authority.</p><h3>Local peer</h3><pre id="net-local"></pre><h3>Target</h3><pre id="net-target-details"></pre><div id="net-extra-details" hidden><h3 id="net-extra-title"></h3><pre id="net-extra-json"></pre></div></div>`;
  document.querySelector('#view-networking')!.append(section);
  const get = (id: string) => section.querySelector<HTMLElement>(`#${id}`)!;
  const field = (id: string) => get(id) as HTMLInputElement;
  const button = (id: string) => get(id) as HTMLButtonElement;
  let discovering = false, sending = false, resultVersion = 0;
  let resultKey = '';
  let targetConfirmed = false;
  type Screen = 'connect' | 'discover' | 'diagnostic' | 'manual' | 'technical' | 'result';
  let screen: Screen = 'connect';
  const history: { screen: Screen; focus: HTMLElement | null; scroll: [HTMLElement, number][] }[] = [];
  const panel = () => section.querySelector<HTMLElement>(`[data-network-screen="${screen}"]`)!;
  function focusScreen() {
    if (!section.closest('[hidden]')) panel().querySelector<HTMLElement>('h2')?.focus({ preventScroll: true });
  }
  function show(next: Screen, remember = true) {
    if (screen !== next && remember) {
      const scroll: [HTMLElement, number][] = [];
      for (let element: HTMLElement | null = panel(); element; element = element.parentElement) scroll.push([element, element.scrollTop]);
      history.push({ screen, focus: document.activeElement instanceof HTMLElement ? document.activeElement : null, scroll });
    }
    screen = next; render(); focusScreen();
  }
  function back() {
    const previous = history.pop();
    screen = previous?.screen ?? (net.state === 'ready' ? targetConfirmed ? 'diagnostic' : 'discover' : 'connect');
    render();
    if (previous?.focus?.isConnected && !previous.focus.closest('[hidden]')) previous.focus.focus({ preventScroll: true });
    else focusScreen();
    for (const [element, top] of previous?.scroll ?? []) element.scrollTop = top;
  }
  function showTechnical(title = '', json = '') {
    get('net-extra-title').textContent = title;
    get('net-extra-json').textContent = json;
    get('net-extra-details').hidden = !json;
    show('technical');
  }
  function technicalButton(title: string, value: unknown) {
    const control = document.createElement('button'), json = inspect(value);
    control.textContent = title;
    control.dataset.networkTechnical = '';
    control.onclick = () => showTechnical(title, json);
    return control;
  }
  const identity = () => JSON.stringify([selectedDomain(), field('net-peer-id').value.trim(), field('net-route').value.trim(), field('net-payload').value]);
  function invalidateResult() {
    resultVersion++; resultKey = ''; get('net-results').textContent = '';
    get('net-extra-json').textContent = ''; get('net-extra-details').hidden = true;
  }
  const net = new Networking<AukiPeer, AukiEchoClient>(render);
  let previousState = net.state;
  function render() {
    get('net-state').textContent = net.state;
    button('net-start').disabled = !connection.session || !selectedDomain() || !['stopped', 'failed'].includes(net.state);
    button('net-stop').disabled = !['starting', 'ready'].includes(net.state);
    button('net-discover').disabled = net.state !== 'ready' || discovering;
    button('net-show-manual').disabled = net.state !== 'ready';
    const fieldsComplete = !!field('net-peer-id').value.trim() && !!field('net-route').value.trim();
    const target = targetConfirmed && fieldsComplete;
    button('net-use-manual').disabled = net.state !== 'ready' || !fieldsComplete;
    let validPayload = false;
    try { diagnosticBytes(field('net-payload').value); validPayload = !!field('net-payload').value.trim(); } catch { /* Keep send disabled. */ }
    button('net-send').disabled = net.state !== 'ready' || !target || !validPayload || sending;
    button('net-reselect').disabled = net.state !== 'ready' || !target;
    for (const id of ['net-peer-id', 'net-route']) field(id).disabled = net.state !== 'ready';
    field('net-payload').disabled = net.state !== 'ready' || !target;
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
    // Primary-view refreshes keep the current local screen. Only lifecycle or
    // invalidated selections can make a screen unavailable.
    const oldScreen = screen;
    if (net.state !== previousState) {
      if (net.state !== 'ready') { screen = 'connect'; history.length = 0; }
      else screen = 'discover';
      previousState = net.state;
    }
    if (net.state !== 'ready' && screen !== 'technical') screen = 'connect';
    if (net.state === 'ready' && (screen === 'connect' || (screen === 'diagnostic' && !target))) screen = 'discover';
    if (screen === 'result' && !resultKey) screen = net.state === 'ready' ? target ? 'diagnostic' : 'discover' : 'connect';
    section.querySelectorAll<HTMLElement>('[data-network-screen]').forEach(step => {
      const active = step.dataset.networkScreen === screen;
      step.hidden = !active;
      step.classList.toggle('active', active);
      if (step.dataset.step) step.setAttribute('aria-current', active ? 'step' : 'false');
    });
    get('net-back').hidden = !['manual', 'technical'].includes(screen);
    get('net-show-technical').hidden = screen === 'technical';
    get('net-show-result').hidden = !resultKey || screen === 'result' || screen === 'technical';
    if (oldScreen !== screen) focusScreen();
  }
  button('net-back').onclick = back;
  button('net-result-back').onclick = back;
  button('net-show-manual').onclick = () => { if (net.state === 'ready') show('manual'); };
  button('net-show-technical').onclick = () => showTechnical();
  button('net-show-result').onclick = () => { if (resultKey) show('result'); };
  for (const id of ['net-peer-id', 'net-route', 'net-payload']) field(id).oninput = () => {
    if (id !== 'net-payload') targetConfirmed = false;
    invalidateResult();
    get('net-status').textContent = 'Diagnostic changed. Send to verify the current target and message.';
    render();
  };
  button('net-reselect').onclick = () => {
    targetConfirmed = false; resultVersion++; history.length = 0; get('net-status').textContent = 'Choose a candidate or edit and confirm a manual route.'; show('discover', false); button('net-discover').focus();
  };
  button('net-use-manual').onclick = () => {
    if (button('net-use-manual').disabled) return;
    targetConfirmed = true; invalidateResult(); history.length = 0; show('diagnostic', false); field('net-payload').focus();
  };
  connection.beforeClose = () => net.close();
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
        row.append(summary, facts({ 'Peer ID': shortPeerId(candidate.peerId) }), technicalButton('Candidate technical details', { peerId: candidate.peerId, routes: candidate.routes, expiresAt: candidate.expiresAt, source: candidate.source }));
        for (const route of candidate.routes.filter(route => /^\/dns4\/[^/]+\/tcp\/\d+\/wss\/p2p\/[^/]+\/p2p-circuit\/p2p\/[^/]+$/.test(route))) {
          const choose = document.createElement('button'), peerId = candidate.peerId;
          choose.textContent = 'Use candidate route';
          choose.onclick = () => { if (net.state !== 'ready' || sending) return; field('net-peer-id').value = peerId; field('net-route').value = route; targetConfirmed = true; invalidateResult(); history.length = 0; get('net-status').textContent = 'Candidate selected. Send a diagnostic to verify this target.'; show('diagnostic', false); field('net-payload').focus(); };
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
      get('net-results').replaceChildren(heading, count, payload, technicalButton('Receipt technical details', { remotePeerId: receipt.remotePeerId, domainId: domain, bytes: receipt.payload.length }));
      get('net-status').textContent = 'Authenticated diagnostic roundtrip completed. This grants no application or task authority.';
      if (screen === 'diagnostic') show('result');
    }, () => { if (current()) get('net-status').textContent = safeError(undefined); }, receipt => receipt.free());
    sending = false; render();
  };
  render();
  return { refresh: render };
}
