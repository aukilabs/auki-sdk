import { AukiChatConnection } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import { AukiDiscoveryMode, AukiPeerReachabilityMode, type AukiPeer, type Connection, drainCleanup } from './sdk';
import { Chat, chatText } from './chat.ts';
import { ChatDiscovery, exactChatRoute } from './chat-discovery.ts';
export function chatUI(connection: Connection, domain: () => string, echo: HTMLElement) {
  const root = document.querySelector('#view-networking')!;
  const tabs = document.createElement('div'); tabs.className = 'actions';
  const echoTab = document.createElement('button'), chatTab = document.createElement('button');
  echoTab.textContent = 'Echo'; chatTab.textContent = 'Chat'; tabs.append(echoTab, chatTab); root.prepend(tabs);
  const panel = document.createElement('section'); panel.className = 'task-body chat-panel'; panel.hidden = true;
  panel.innerHTML = `<div class="network-bar"><h2>Chat</h2><button id="chat-close">Disconnect</button></div><p id="chat-state" role="status"></p>
    <div id="chat-connect-view"><button id="chat-find" class="primary">Find peers</button><p id="chat-discovery-state" role="status"></p><div id="chat-candidates"></div><button id="chat-connect-selected" class="primary">Connect</button><p class="footnote">Advertised candidates · availability unverified · operator approval required.</p><details id="chat-advanced"><summary>Advanced · manual connection</summary><label>Host Peer ID<input id="chat-peer" autocomplete="off"></label><label>Host WSS route<input id="chat-route" autocomplete="off"></label><button id="chat-connect" class="primary">Connect Chat</button></details></div>
    <div id="chat-pair-view" hidden><p>Ask the operator to approve this session and your authenticated Peer ID.</p><p>Session ID</p><pre id="chat-session"></pre><p>Your Chat Peer ID</p><pre id="chat-local-peer"></pre><p>Pending approval expires after 5 minutes.</p></div>
    <div id="chat-conversation" hidden><p class="footnote">Received by peer is a transport receipt, not human-read or AI execution. Session limit: 1 hour.</p><div id="chat-messages" role="log"></div><form id="chat-form"><label>Message · up to 2048 UTF-8 bytes<textarea id="chat-text" rows="3"></textarea></label><button id="chat-send" class="primary">Send</button></form></div><p id="chat-error" role="status"></p>`;
  root.append(panel);
  const get = (id: string) => panel.querySelector<HTMLElement>('#' + id)!;
  const field = (id: string) => get(id) as HTMLInputElement;
  const button = (id: string) => get(id) as HTMLButtonElement;
  const chat = new Chat(render);
  const discovery = new ChatDiscovery(render);
  let resetting = false;
  function render() {
    get('chat-state').textContent = chat.state;
    const idle = ['stopped', 'failed'].includes(chat.state);
    get('chat-connect-view').hidden = !idle;
    get('chat-pair-view').hidden = chat.state !== 'pending';
    get('chat-conversation').hidden = chat.state !== 'paired';
    get('chat-session').textContent = chat.sessionId;
    get('chat-local-peer').textContent = chat.peerId;
    const available = !!connection.session && !!domain() && idle && !resetting && discovery.state !== 'closing';
    button('chat-connect').disabled = !available || discovery.state === 'searching';
    button('chat-find').disabled = !available || discovery.state === 'searching';
    button('chat-find').textContent = discovery.state === 'stopped' ? 'Find peers' : 'Refresh peers';
    button('chat-connect-selected').disabled = !available || !discovery.selected || discovery.selected.expiresAt <= Date.now();
    button('chat-close').disabled = resetting || chat.state === 'closing' || (idle && discovery.state === 'stopped');
    const states = { stopped: '', searching: 'Finding Chat peers…', ready: 'Select a Chat peer.', empty: 'No Chat peers found.', denied: 'Discovery access denied.', offline: 'Discovery unavailable or offline. Retry when connected.', error: 'Discovery failed.', closing: 'Closing discovery…' };
    get('chat-discovery-state').textContent = states[discovery.state];
    get('chat-candidates').replaceChildren(...discovery.candidates.map((target, index) => {
      const item = document.createElement('div'), row = document.createElement('button');
      const routeParts = target.route.split('/');
      row.textContent = `${target.peerId.slice(0, 12)}…${target.peerId.slice(-8)} · ${routeParts[2]}:${routeParts[4]}`;
      row.dataset.peerId = target.peerId; row.dataset.route = target.route;
      row.style.overflowWrap = 'anywhere'; row.style.whiteSpace = 'normal';
      row.setAttribute('aria-pressed', String(discovery.selected === target));
      row.onclick = () => { discovery.select(index); };
      const details = document.createElement('details'), summary = document.createElement('summary'), exact = document.createElement('pre');
      summary.textContent = 'Technical details';
      exact.textContent = `Peer ID\n${target.peerId}\n\nWSS route\n${target.route}`;
      details.append(summary, exact); item.append(row, details);
      return item;
    }));
    let valid = false; try { chatText(field('chat-text').value); valid = true; } catch { /* incomplete */ }
    button('chat-send').disabled = chat.state !== 'paired' || !valid || chat.messages.length >= 128;
    get('chat-messages').replaceChildren(...chat.messages.map(message => {
      const row = document.createElement('p'); row.className = 'chat-message';
      row.textContent = `${message.direction === 'out' ? 'You' : 'Host'} · ${message.status === 'received' && message.direction === 'out' ? 'Received by peer' : message.status}\n${message.text}`; return row;
    }));
  }
  echoTab.onclick = () => { echo.hidden = false; panel.hidden = true; };
  chatTab.onclick = () => { echo.hidden = true; panel.hidden = false; render(); };
  const startPeer = () => {
    const session = connection.session, selected = domain();
    if (!session || !selected || resetting) throw new Error('Select a Domain.');
    return session.startPeerWithDiscovery(selected, AukiDiscoveryMode.DiscoverOnly, AukiPeerReachabilityMode.OutboundOnly);
  };
  button('chat-find').onclick = () => {
    get('chat-error').textContent = '';
    void discovery.find(startPeer).catch(() => { get('chat-error').textContent = 'Chat discovery cleanup failed.'; });
  };
  function connect(remote: string, route: string) {
    if (resetting || !connection.session || !domain() || discovery.state === 'searching' || !exactChatRoute(remote, route)) {
      get('chat-error').textContent = 'Enter the exact host Peer ID and WSS relay route.'; return;
    }
    get('chat-error').textContent = '';
    void chat.connect(startPeer, peer => AukiChatConnection.connect(peer as AukiPeer, remote, route));
  }
  button('chat-connect-selected').onclick = () => {
    const target = discovery.selected;
    if (!target || target.expiresAt <= Date.now()) { get('chat-error').textContent = 'Refresh peers and select a current candidate.'; return; }
    connect(target.peerId, target.route);
  };
  button('chat-connect').onclick = () => connect(field('chat-peer').value.trim(), field('chat-route').value.trim());
  async function close() {
    resetting = true;
    try { await drainCleanup(() => discovery.close(), () => chat.close()); }
    finally { resetting = false; render(); }
  }
  button('chat-close').onclick = () => { void close().catch(() => { get('chat-error').textContent = 'Chat cleanup failed.'; }); };
  field('chat-text').oninput = render;
  get('chat-form').onsubmit = event => {
    event.preventDefault(); const value = field('chat-text').value; field('chat-text').value = '';
    void chat.send(value).catch(() => { get('chat-error').textContent = 'Message was not sent. Check the connection and message size.'; });
  };
  const earlier = connection.beforeClose;
  connection.beforeClose = async () => {
    field('chat-text').value = field('chat-peer').value = field('chat-route').value = ''; get('chat-error').textContent = '';
    await drainCleanup(close, earlier);
  };
  render(); return { refresh: render };
}
