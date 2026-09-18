import { AukiChatConnection } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import { AukiDiscoveryMode, AukiPeerReachabilityMode, type AukiPeer, type Connection, drainCleanup } from './sdk';
import { Chat, chatText } from './chat.ts';
export function chatUI(connection: Connection, domain: () => string, echo: HTMLElement) {
  const root = document.querySelector('#view-networking')!;
  const tabs = document.createElement('div'); tabs.className = 'actions';
  const echoTab = document.createElement('button'), chatTab = document.createElement('button');
  echoTab.textContent = 'Echo'; chatTab.textContent = 'Chat'; tabs.append(echoTab, chatTab); root.prepend(tabs);
  const panel = document.createElement('section'); panel.className = 'task-body chat-panel'; panel.hidden = true;
  panel.innerHTML = `<div class="network-bar"><h2>Chat</h2><button id="chat-close">Disconnect</button></div><p id="chat-state" role="status"></p>
    <div id="chat-connect-view"><label>Host Peer ID<input id="chat-peer" autocomplete="off"></label><label>Host WSS route<input id="chat-route" autocomplete="off"></label><button id="chat-connect" class="primary">Connect Chat</button></div>
    <div id="chat-pair-view" hidden><p>Ask the operator to approve this session and your authenticated Peer ID.</p><p>Session ID</p><pre id="chat-session"></pre><p>Your Chat Peer ID</p><pre id="chat-local-peer"></pre><p>Pending approval expires after 5 minutes.</p></div>
    <div id="chat-conversation" hidden><p class="footnote">Received by peer is a transport receipt, not human-read or AI execution. Session limit: 1 hour.</p><div id="chat-messages" role="log"></div><form id="chat-form"><label>Message · up to 2048 UTF-8 bytes<textarea id="chat-text" rows="3"></textarea></label><button id="chat-send" class="primary">Send</button></form></div><p id="chat-error" role="status"></p>`;
  root.append(panel);
  const get = (id: string) => panel.querySelector<HTMLElement>('#' + id)!;
  const field = (id: string) => get(id) as HTMLInputElement;
  const button = (id: string) => get(id) as HTMLButtonElement;
  const chat = new Chat(render);
  function render() {
    get('chat-state').textContent = chat.state;
    const idle = ['stopped', 'failed'].includes(chat.state);
    get('chat-connect-view').hidden = !idle;
    get('chat-pair-view').hidden = chat.state !== 'pending';
    get('chat-conversation').hidden = chat.state !== 'paired';
    get('chat-session').textContent = chat.sessionId;
    get('chat-local-peer').textContent = chat.peerId;
    button('chat-connect').disabled = !connection.session || !domain() || !idle;
    button('chat-close').disabled = idle || chat.state === 'closing';
    let valid = false; try { chatText(field('chat-text').value); valid = true; } catch { /* incomplete */ }
    button('chat-send').disabled = chat.state !== 'paired' || !valid || chat.messages.length >= 128;
    get('chat-messages').replaceChildren(...chat.messages.map(message => {
      const row = document.createElement('p'); row.className = 'chat-message';
      row.textContent = `${message.direction === 'out' ? 'You' : 'Host'} · ${message.status === 'received' && message.direction === 'out' ? 'Received by peer' : message.status}\n${message.text}`; return row;
    }));
  }
  echoTab.onclick = () => { echo.hidden = false; panel.hidden = true; };
  chatTab.onclick = () => { echo.hidden = true; panel.hidden = false; render(); };
  button('chat-connect').onclick = () => {
    const session = connection.session, selected = domain(), remote = field('chat-peer').value.trim(), route = field('chat-route').value.trim();
    if (!session || !selected || !remote || !/^\/dns4\/[^/]+\/tcp\/\d+\/wss\/p2p\/[^/]+\/p2p-circuit\/p2p\/[^/]+$/.test(route) || !route.endsWith('/p2p/' + remote)) {
      get('chat-error').textContent = 'Enter the exact host Peer ID and WSS relay route.'; return;
    }
    get('chat-error').textContent = '';
    void chat.connect(() => session.startPeerWithDiscovery(selected, AukiDiscoveryMode.DiscoverOnly, AukiPeerReachabilityMode.OutboundOnly), peer => AukiChatConnection.connect(peer as AukiPeer, remote, route));
  };
  button('chat-close').onclick = () => { void chat.close().catch(() => { get('chat-error').textContent = 'Chat cleanup failed.'; }); };
  field('chat-text').oninput = render;
  get('chat-form').onsubmit = event => {
    event.preventDefault(); const value = field('chat-text').value; field('chat-text').value = '';
    void chat.send(value).catch(() => { get('chat-error').textContent = 'Message was not sent. Check the connection and message size.'; });
  };
  const earlier = connection.beforeClose;
  connection.beforeClose = async () => {
    field('chat-text').value = field('chat-peer').value = field('chat-route').value = ''; get('chat-error').textContent = '';
    await drainCleanup(() => chat.close(), earlier);
  };
  render(); return { refresh: render };
}
