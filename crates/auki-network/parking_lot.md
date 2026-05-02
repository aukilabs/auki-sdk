# Parking lot — auki-network

---

## mDNS coexistence — `_p2p._udp.local.` vs `_auki._tcp.local.`

The SDK's existing operator-control surface advertises daemons under `_auki._tcp.local.` (per [`docs/control-api.md`](../../docs/control-api.md)). libp2p's mDNS behaviour advertises peers under `_p2p._udp.local.` for LAN peer discovery.

Three options, none obviously right:

1. **Dual-channel coexistence.** Daemons advertise both. Park's existing `_auki._tcp.local.` browse keeps working; libp2p mDNS is purely additive for peer discovery. Cost: two service records per daemon, two browse paths in Park.

2. **Migration.** Drop `_auki._tcp.local.`; everything goes through libp2p mDNS + `ReachabilityRecord` lookup. Cleaner long-term, breaks Park's existing mDNS code, breaks any LAN-only consumer that doesn't want to wire up libp2p.

3. **Disable libp2p mDNS.** Use `_auki._tcp.local.` for everything; advertise a `ReachabilityRecord`-shaped TXT record alongside the existing `name=` / `app=` ones. Keeps one service record, but means our LAN discovery isn't off-the-shelf libp2p.

Resolution shapes the M1 swarm default. Leaning toward (1) for the demo (additive, low-risk) with (2) as the long-term direction once Park has its libp2p path.

---

## Wallet → peer-key derivation label evolution

`PEER_DERIVATION_LABEL = "peer/v1"` is shipped. The `/v1` suffix is deliberate — when libp2p's PeerId encoding changes, or when we want to rotate the peer key without invalidating the wallet, we can ship `"peer/v2"` and have new code derive from the new label while old `ReachabilityRecord`s under the v1 PeerId still resolve.

Open: should the label format be richer? `"peer/<protocol>/<version>"` (e.g. `"peer/libp2p/v1"`) leaves room for future non-libp2p networking layers. `"peer/v1"` is shorter and assumes libp2p is the only networking substrate we'll ever have. Defer until a second substrate appears (or doesn't).

Related, in [`auki-identity/parking_lot.md`](../auki-identity/parking_lot.md): BIP32-style HD vs labeled-hash derivation. If the SDK ever switches to BIP32 paths, the peer label becomes a derivation path like `m/44'/auki'/peer'/0'`. Re-derivation invalidates existing peer ids regardless of label format; this is a coordinated SDK + consumer change.

---

## Park-from-home access pattern

Reid milestone-2 needs Park to dial K1's daemon from outside the K1's LAN. Three approaches:

1. **Discovery Service query.** Park asks an Auki Labs–hosted directory "what's the current `ReachabilityRecord` for peer-id X?". Service is authoritative; daemons publish to it. Closest fit to the long-term Domain/Cluster architecture.
2. **Capability recruitment.** Park asks the Discovery Service "what relay knows peer-id X?", dials through that relay using libp2p's circuit-relay-v2. Same Discovery Service shape, different query.
3. **Manual peer-id paste for v1.** Operator pastes the K1's peer-id (and a relay multiaddr) into Park's UI; Park dials through. No Discovery Service dependency.

(3) ships fastest and unblocks the demo without a Discovery Service. (1) and (2) need the Discovery Service shape pinned first. Probably do (3) for the milestone-2 demo and pull (1)/(2) in alongside Domain participation.

---

## Off-by-default for relay-server on consumer daemons

The Reid decision stipulates `relay-server` is off by default for BoosterApp / Sentinel; opt-in via `--relay-server` (or equivalent). The dedicated `aukilabs/relay` app is what runs as relay infrastructure.

Open in code: how does the off-by-default flag get plumbed? Three paths:

1. **Boolean argument to a swarm builder** in `auki-network` — `SwarmConfig { enable_relay_server: bool, ... }`. M1a's `SwarmConfig` would gain the field in M1b alongside the `relay::Behaviour` wrapped in `Toggle`.
2. **Per-capability advertisement gate** — `Capability::SFU` / `Capability::TURN` etc. only get included in the `ReachabilityRecord` if the corresponding behaviour is enabled.
3. **Both.** Boolean controls the libp2p behaviour; capability list reflects what's actually offered.

Lean toward (3); decide concretely when M1b starts. The M1a `SwarmConfig` is intentionally minimal (only `listen_addresses` + `agent_version`) to leave the relay-server field shape unspecified for now.

---

## `ReachabilityRecord` extensibility

Today it's `{peer_id, addresses, capabilities, last_seen_ns}`. Likely additions over time:

- **Operator metadata.** Friendly name, owning wallet id, geographic hint.
- **Health / load.** Open-circuit count, recent error rate, advertised capacity.
- **Auth.** Signed by the peer's wallet to prove ownership of the peer key — needs `auki-identity::CreationCert` shape extended for signing arbitrary structs.

Append-fields-with-`#[serde(default)]` is the easy path; a versioned wire format is the honest one. Decide before any consumer relies on the shape being stable.

---

## `SwarmConfig` minimalism — when do we add knobs?

M1a's `SwarmConfig` has only `listen_addresses` and `agent_version`. Many libp2p knobs are baked-in: idle connection timeout (60s), identify protocol id (`/auki/identify/1.0.0`), ping defaults. This is deliberate — fewer knobs means fewer ways to mis-configure.

Knobs that consumers will likely want eventually:

- **Idle timeout.** Long-lived idle connections vs aggressive eviction. Daemons probably want longer; gateways shorter.
- **Ping interval / timeout.** Keepalive cadence vs liveness sensitivity.
- **Allowed transports.** Force-TCP-only or force-QUIC-only for testing or networks that block UDP.
- **Connection limits.** `libp2p::connection_limits::Behaviour` parameters.

Default: don't expose any of these until a real consumer asks. Adding fields with `Default` impls is non-breaking; removing them is. Stay minimal.

---

## `BuildError::Transport(String)` — structured vs prose

`BuildError::Transport` currently wraps a `String` (formatted from libp2p's transport-setup errors). This loses type information — callers can't programmatically distinguish "tcp listen failed" from "noise key generation failed." It also doesn't surface the underlying `std::io::Error` cleanly.

Three options:

1. **Keep `String`.** Simple; transport setup failures are rare and operator-facing anyway.
2. **Box the underlying error.** `Transport(Box<dyn std::error::Error + Send + Sync>)`. Preserves the chain via `Error::source()`.
3. **Enum the failure modes.** `Tcp`, `Quic`, `Noise`, `Yamux`, `Behaviour` variants. Most type information; most maintenance burden as libp2p evolves.

Lean toward (2) once a consumer wants programmatic dispatch. (1) is fine for M1a.
