# Parking lot — auki-network

---

## Wallet → peer-key derivation label evolution

`PEER_DERIVATION_LABEL = "peer/v1"` is shipped. The `/v1` suffix is deliberate — when libp2p's PeerId encoding changes, or when we want to rotate the peer key without invalidating the wallet, we can ship `"peer/v2"` and have new code derive from the new label while old `ReachabilityRecord`s under the v1 PeerId still resolve.

Open: should the label format be richer? `"peer/<protocol>/<version>"` (e.g. `"peer/libp2p/v1"`) leaves room for future non-libp2p networking layers. `"peer/v1"` is shorter and assumes libp2p is the only networking substrate we'll ever have. Defer until a second substrate appears (or doesn't).

Related, in [`auki-identity/parking_lot.md`](../auki-identity/parking_lot.md): BIP32-style HD vs labeled-hash derivation. If the SDK ever switches to BIP32 paths, the peer label becomes a derivation path like `m/44'/auki'/peer'/0'`. Re-derivation invalidates existing peer ids regardless of label format; this is a coordinated SDK + consumer change.

---

## `ReachabilityRecord` extensibility

Today it's `{peer_id, addresses, capabilities, last_seen_ns}`. Likely additions over time:

- **Operator metadata.** Friendly name, owning wallet id, geographic hint.
- **Health / load.** Open-circuit count, recent error rate, advertised capacity.
- **Auth.** Signed by the peer's wallet to prove ownership of the peer key — needs `auki-identity::CreationCert` shape extended for signing arbitrary structs.

Append-fields-with-`#[serde(default)]` is the easy path; a versioned wire format is the honest one. Decide before any consumer relies on the shape being stable.

---

## `SwarmConfig` minimalism — when do we add knobs?

M1's `SwarmConfig` has `listen_addresses`, `agent_version`, `enable_mdns`, `enable_relay_server`. Many libp2p knobs are baked-in: idle connection timeout (60s), identify protocol id (`/auki/identify/1.0.0`), ping defaults. This is deliberate — fewer knobs means fewer ways to mis-configure.

Knobs that consumers will likely want eventually:

- **Idle timeout.** Long-lived idle connections vs aggressive eviction. Daemons probably want longer; gateways shorter.
- **Ping interval / timeout.** Keepalive cadence vs liveness sensitivity.
- **Allowed transports.** Force-TCP-only or force-QUIC-only for testing or networks that block UDP.
- **Connection limits.** `libp2p::connection_limits::Behaviour` parameters.
- **Configured relay multiaddrs** for relay-clients to auto-register reachability with on startup. Currently the relay-client behaviour is wired in but no auto-dial of any specific relay; consumer code does the dial explicitly.

Default: don't expose any of these until a real consumer asks. Adding fields with `Default` impls is non-breaking; removing them is. Stay minimal.

---

## `BuildError::Transport(String)` — structured vs prose

`BuildError::Transport` currently wraps a `String` (formatted from libp2p's transport-setup errors). This loses type information — callers can't programmatically distinguish "tcp listen failed" from "noise key generation failed" from "mdns init failed." It also doesn't surface the underlying `std::io::Error` cleanly.

Three options:

1. **Keep `String`.** Simple; transport setup failures are rare and operator-facing anyway.
2. **Box the underlying error.** `Transport(Box<dyn std::error::Error + Send + Sync>)`. Preserves the chain via `Error::source()`.
3. **Enum the failure modes.** `Tcp`, `Quic`, `RelayClient`, `Noise`, `Yamux`, `Mdns`, `Behaviour` variants. Most type information; most maintenance burden as libp2p evolves.

Lean toward (2) once a consumer wants programmatic dispatch. (1) is fine for M1.

---

## Loopback test workaround — `add_external_address` for relay reservation

The `relay_server_accepts_reservation` test calls `relay_swarm.add_external_address(relay_addr)` so the relay's reservation response includes the loopback listen address (which the swarm doesn't auto-discover as external).

On real networks, external addresses get learned via identify (the client tells the relay what address it dialed) or AutoNAT. This isn't really a parking-lot question — just documented here so a future reader doesn't strip the `add_external_address` call thinking it's redundant. Move to a comment-only note if/when AutoNAT lands.

---

## DCUtR / hole-punching — when?

Not in M1. libp2p `dcutr::Behaviour` upgrades a relayed connection to a direct one via simultaneous-open hole-punching. Small additive change to the `Behaviour` composition; not load-bearing for the M2 demo (Park dialing K1 through a relay works either way).

Ship it when (a) the M2 demo is end-to-end and (b) Park-from-home traffic volume warrants reducing relay load.
