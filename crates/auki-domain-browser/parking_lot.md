# Parking Lot — auki-domain-browser

Open questions for the browser Domain peer adapter.

## Items

- **2026-05-19 — Browser transport.** Native SDK peers advertise TCP/QUIC multiaddrs that browsers cannot dial directly. Decide the first SDK-owned browser transport: WebSocket multiaddrs, WebTransport, WebRTC-as-transport, or SDK relay.
- **2026-05-19 — Browser Manager scope.** Decide whether browser `createDomain` makes the browser a Manager in v1, provisions/depends on a native Manager, or lands after leaf-peer join support.
