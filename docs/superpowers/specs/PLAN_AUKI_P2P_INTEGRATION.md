# PLAN: Decide the `auki-p2p` integration into `auki-domain`

**Status:** D01–D16 locked. Companion design refreshed and implementation TODO
ready.

**Companion design:**
[`2026-08-25-auki-p2p-integration-and-cluster-removal-design.md`](./2026-08-25-auki-p2p-integration-and-cluster-removal-design.md)

**Implementation ledger:**
[`TODO_AUKI_P2P_INTEGRATION.md`](./TODO_AUKI_P2P_INTEGRATION.md).

**Last updated:** 2026-08-26.

Locked decisions in this plan refine and, where necessary, supersede the
companion design. The companion design and implementation ledger have both been
refreshed from these decisions.

## 1. Purpose of this plan

The design document captures the direction we agree with:

- keep `auki-domain` as the product-facing SDK;
- replace its networking and trust internals with `auki-p2p`;
- preserve useful Domain, resource, registry, blob, message, and stream behavior;
- remove Manager, election, membership, and Manager-owned authorization;
- treat known peers as observed authenticated and reachable peers; and
- defer discovery.

That direction alone was not precise enough to produce an implementation
ledger. The locked decisions below prevent public APIs, credential contracts,
wire protocols, browser support, or the meaning of a Domain from being chosen
accidentally while editing code.

This plan has one job: record the resolution of those choices. It does not contain
implementation prompts, commit-sized tasks, or a migration checklist. The TODO
may be written only after this plan's decision gate is satisfied.

## 2. What is already locked

These are product decisions from the design discussion. Reopening one requires
an explicit design change, not an implementation convenience.

| ID | Locked decision |
| --- | --- |
| L01 | `auki-domain` remains the main application-facing networking facade. |
| L02 | We adapt the existing SDK; we do not build a replacement SDK beside it. |
| L03 | Manager, leader election, successor handoff, Manager admission, and authoritative cluster membership are removed. |
| L04 | Useful resource, registry, blob, messaging, and typed-stream business logic is reused rather than rewritten. |
| L05 | Application protocols receive mutually DDS-authenticated streams before seeing application bytes. |
| L06 | A configured address, discovered address, Noise connection, or cached peer record never grants application authorization. |
| L07 | Known-peer state is observational: authenticated, reachable peers rather than an authoritative global roster. |
| L08 | Discovery selection is deferred. The runtime accepts explicit bounded route candidates from any future source. |
| L09 | We do not create one new crate per existing protocol up front. Extraction must follow a demonstrated ownership or reuse boundary. |
| L10 | The SDK uses the upstream crates.io libp2p `0.56` family; no private rust-libp2p fork is part of this migration. |
| L11 | One process must not run two competing swarms or Peer IDs during the cutover. |
| L12 | Manager APIs are removed honestly; they are not retained with fake leaders, synthetic memberships, or renamed election concepts. |

## 3. Current audit baseline

This section records the facts the decisions must account for. It is not a
proposal for preserving the current architecture.

### 3.1 Current crate responsibilities

| Crate or package | Current responsibility | Consequence for this migration |
| --- | --- | --- |
| `auki-domain` | `Domain` plus a large `ClusterManager` that owns Discovery, Manager state, membership, protocol services, clocks, and shutdown | Keep the Domain facade and feature logic; replace the engine. |
| `auki-network` | Wallet-derived P2P identity, swarm, allow-list, central command runtime, protocols, Discovery client, and Swift annotations | Its responsibilities must be split; it cannot remain the authorization/runtime center. |
| `auki-domain-relay` | Generic CRv2 relay server used for Manager/browser reachability | Manager ownership disappears; its future must be decided separately from discovery. |
| `auki-network-browser-wasm` | Experimental browser libp2p runtime and browser Domain session | It is Manager/Discovery-shaped and is not automatically compatible with native `auki-p2p`. |
| `auki-domain-browser` | Separate TypeScript Domain contract and partial implementation | We must choose whether this or the Rust/WASM path is the canonical browser SDK. |
| `auki-domain-py` | Large Python `ClusterManager` facade plus catalogs, streams, messages, and registry access | Useful protocol APIs need a non-Manager owner and an explicit breaking-API plan. |
| `auki-network-swift` | Public `NetworkRuntime`, Discovery, liveness, and typed streams | It exposes the layer we intend to replace, so Swift cannot be treated as a mechanical rename. |
| Posemesh `auki-p2p` | Native DDS-authenticated runtime, exact routes, relay primitives, protocol supervision, and route catalog | Strong starting implementation, but its current credential and Domain assumptions are narrower than the SDK. |
| Posemesh `auki-p2p-dataset` | First narrow application protocol built on `auki-p2p` | Reference pattern for authenticated protocol ownership, not a template that every protocol must copy verbatim. |

The Auki SDK already depends on upstream libp2p `0.56`. The dependency upgrade
is therefore not an open architectural question in this repository.

### 3.2 Mismatches that block a mechanical transplant

1. **Domain identity differs.** The SDK describes a Domain ID as a
   wallet/public-key-derived identifier and currently networks using a free-form
   `cluster_name`. Posemesh `auki-p2p` requires a DDS UUID Domain claim.
2. **The credential audience is too narrow.** Current `auki-p2p` accepts only
   `robot`, `compute`, and `domain_server` roles with the exact
   `domain-data:r` scope. General SDK daemons, mobile apps, and browsers do not
   fit that profile.
3. **Runtime ownership differs.** Current `DomainConfig` receives a prebuilt
   swarm and each `Domain` owns a `ClusterManager`. D05 replaces that with one
   Domain-owned `auki-p2p` node and defers concurrent multi-Domain hosting.
4. **Transport support differs.** Posemesh `auki-p2p` is a native Tokio
   TCP/DNS/relay runtime. The SDK currently carries native TCP/QUIC/WebSocket
   paths plus separate WebRTC/WebSocket browser work.
5. **Authentication changes wire compatibility.** Existing protocols expose
   application framing directly after negotiation. `auki-p2p` performs mutual
   credential exchange before yielding the stream.
6. **The public extension boundary is undecided.** Posemesh protocol crates
   compose directly from `Node`, `ProtocolSpec`, `P2pCredentialStore`, and
   `RouteCatalog`; normal product code receives a narrow service. We must decide
   what SDK protocol authors can access without exposing raw libp2p internals to
   every application.

### 3.3 Existing protocol inventory

This is the final high-level disposition locked by D09–D13. Exact file movement
and test order belong in the TODO.

| Current protocol | Current purpose | Locked disposition |
| --- | --- | --- |
| `/auki/join/0.0.1` | Manager admission and membership bootstrap | Remove. |
| `/auki/membership/0.0.1` | Manager-authored roster convergence | Remove. |
| `/auki/heartbeat/0.0.1` | Manager liveness plus time samples | Remove entirely under D10. |
| `/auki/info/0.0.1` | Participant metadata | Adapt to the authenticated D09 metadata contract. |
| `/auki/resources/0.2.0` | Legacy resource catalog | Reuse its payload behind a D11 authenticated ID. |
| `/auki/resources/0.3.0` | Message-channel resource catalog | Reuse behind authenticated streams. |
| `/auki/resources/0.4.0` | Map Log resource catalog | Reuse behind authenticated streams. |
| `/auki/registries/0.2.0` and `0.3.0` | Registry listing and hash-pinned fetch | Reuse both behind authenticated streams. |
| `/auki/blobs/0.1.0` | Bounded content-addressed blob fetch | Reuse behind authenticated streams. |
| `/auki/message/0.1.0` | Live bounded typed messaging | Reuse behind the common D02 rule. |
| `/auki/stream/0.2.0` | Native typed sensor/map streams | Reuse as the one cross-platform application stream contract. |
| `/auki/stream/0.1.0` | Browser stream path | Remove; the browser later implements `0.2.0`. |
| `/auki/diagnostic/0.0.1` | Cluster diagnostic broadcast | Remove. |
| `/auki/browser-session/0.0.1` | Browser-to-Manager control/session | Remove. |
| `/auki/browser-probe/0.0.1` | Browser transport probing | Remove from the product wire surface; private transport tests may remain. |
| `/auki/identify/0.0.1`, ping, Noise | Transport identity and health | Internal transport behavior; never application authorization. |
| stock Circuit Relay v2 plus `auki-domain-relay` | Manager/browser reachability | `auki-p2p` owns CRv2 reserve/connect/cancel mechanics; provider booking and relay-server operation stay outside `Domain`. |

### 3.4 Public surface inventory

The implementation TODO cannot say "preserve the API" generally. It needs an
approved, language-by-language diff.

| Surface | Clearly useful | Clearly removed | Locked replacement direction |
| --- | --- | --- | --- |
| Rust `auki-domain` | `Domain`, `DomainBuilder`, catalogs, registry/blob fetches, messages, typed streams, `leave` | `ClusterManager`, `ClusterTarget`, membership, elections, `admit_peer`, Manager relay ownership | D05–D08 Domain-owned runtime, authority handle, observed peers, and protocol-author handle. |
| Python `auki-domain` | Catalogs, registry/blob access, message channels, stream types/openers | Cluster creation/join policy, membership mutation, Manager state | Ship the retained Domain facade with native Stage 1. |
| Swift `auki-network` | Stable identity and useful typed streams | Manager/Discovery semantics | Stage 2 binds the native Domain owner; old `NetworkRuntime` is removed. |
| Browser TypeScript/WASM | Stable peer identity, Domain participation, sensor/media behavior | Manager control and Manager Discovery dependency | Stage 3 uses `auki-domain-browser` plus one TypeScript/js-libp2p engine; Rust/WASM engine is retired. |

## 4. Decision rules

A decision belongs in this plan when changing its answer would materially alter
one or more of:

- a public SDK API;
- a DDS credential or authorization contract;
- a protocol ID or wire handshake;
- runtime/key/task ownership;
- supported platforms or transports;
- compatibility and release strategy; or
- the crate dependency graph.

Exact module names, private structs, channel capacities, retry constants,
metrics names, and commit order do not belong here unless a product guarantee
depends on them. Those are TODO decisions.

Every open decision below is resolved only when the plan records:

1. one selected answer;
2. the public or wire consequence;
3. rejected alternatives that would otherwise reappear during review; and
4. the smallest proof needed before implementation begins.

## 5. Decision ledger

| ID | Decision | Status | Depends on |
| --- | --- | --- | --- |
| D01 | Canonical Domain identity and credential representation | **LOCKED** | — |
| D02 | DDS P2P Domain-token authorization baseline | **LOCKED** | D01 |
| D03 | Credential acquisition, refresh, and verification ownership | **LOCKED** | D01, D02, D04 |
| D04 | Stable P2P identity derivation, storage, and proof | **LOCKED** | — |
| D05 | Domain-owned runtime and one active Domain context | **LOCKED** | D01, D03, D04 |
| D06 | Exact `Domain` construction, `join`, readiness, `leave`, and API delta | **LOCKED** | D05 |
| D07 | Known-peer state, events, counts, and public snapshot | **LOCKED** | D01, D02, D05 |
| D08 | Authenticated protocol extension boundary | **LOCKED** | D02, D05 |
| D09 | Existing protocol disposition and authorization matrix | **LOCKED** | D01, D02, D08 |
| D10 | Domain time and heartbeat replacement | **LOCKED** | D07, D09 |
| D11 | Wire protocol IDs, compatibility, and cutover | **LOCKED** | D03, D09 |
| D12 | Native/browser transport and browser SDK target | **LOCKED** | D03, D05, D11 |
| D13 | Route input, direct reachability, and relay ownership | **LOCKED** | D05, D12 |
| D14 | Canonical repository/crate ownership and fate of `auki-network` | **LOCKED** | D05, D08, D09, D12, D13 |
| D15 | Rust/Python/Swift/browser release and breaking-change policy | **LOCKED** | D06, D11, D12, D14 |
| D16 | First vertical slice and migration/cutover boundary | **LOCKED** | D06–D15 |

## 6. Locked decisions

### D01 — Canonical Domain identity

**Status:** LOCKED.

**Decision:** The canonical online Domain identity is the DDS Domain UUID.

- `auki-domain`, `DomainConfig`, DDS P2P claims, mutual authentication,
  protocol authorization, peer snapshots, and route associations use the same
  canonical lowercase/hyphenated UUID representation.
- One `Domain` object represents exactly one DDS Domain UUID.
- A human-readable Domain name is display metadata and never authority.
- Free-form cluster names are not Domain identities and disappear with cluster
  bootstrap.
- No wallet-derived identifier is translated into, aliased to, or accepted as
  a DDS Domain UUID.
- If wallet-derived Domain/tag identifiers remain useful to data products or
  ownership claims, they retain a separately named and typed concept. They do
  not participate in P2P authentication.
- Whether one credential may contain several DDS Domain UUIDs, and its bound,
  is decided with the credential profile in D02. It does not change the
  one-Domain-per-`Domain` rule.

**Why it blocks the TODO:** The transport currently parses UUIDs, the SDK uses
cluster names at runtime, and product documentation defines a wallet-derived
Domain ID. Every credential, `SessionRequirements`, route association,
protocol policy, and `DomainConfig` depends on this answer.

**Rejected alternatives:**

- preserving free-form `cluster_name` as authority;
- deriving the online Domain identity from a wallet;
- maintaining a signed or implicit mapping between two authoritative Domain
  identifiers; and
- accepting both DDS UUIDs and legacy identifiers in protocol policy.

**Resolution proof:** One cross-language fixed vector and one example showing
the same Domain value in `DomainConfig`, a DDS claim, mutual authentication,
and a protocol authorization check.

### D02 — DDS P2P Domain-token authorization baseline

**Status:** LOCKED.

**Decision:** Possession of one valid DDS P2P Domain access token is the SDK's
complete baseline P2P authorization.

For an application stream to be exposed, `auki-p2p` verifies that:

- DDS signed the token under the accepted P2P token profile;
- the token is currently valid for the exact `auki-p2p` audience;
- the token's Peer ID exactly equals the remote Noise Peer ID;
- the token contains the exact DDS Domain UUID required by the local `Domain`;
  and
- the local peer also has a current token authorizing that same Domain.

Once those checks pass, the peer may use the standard authenticated SDK
protocols for that Domain. There is no second generic authorization layer based
on:

- Robot, Compute, Domain Server, browser, daemon, producer, or consumer roles;
- generic per-protocol scopes;
- Manager approval or cluster membership;
- `known_peers` or cached authentication;
- app name, capability advertisement, or transport type.

`peer_type`, subject kind, app metadata, or legacy scopes may remain signed
claims for diagnostics or wire compatibility, but `auki-domain` and its base
protocols do not use them as authorization inputs.

Existing protocol business rules still apply after authentication. Examples
include matching a discovered resource's owner to the authenticated Peer ID,
pinning registry hashes, addressing the expected producer, respecting queue and
size bounds, and validating message or stream payloads. Those are protocol
integrity/ownership rules, not a generic role system.

If a future protocol truly needs additional authority, that protocol must
define and version one explicit application-level rule. We do not design that
framework preemptively as part of this migration.

A token may authorize a bounded list of unique DDS Domain UUIDs; each
application stream still selects and proves one exact Domain. Keep the current
`1..=25` bound.

The accepted signed claim profile is also locked:

| Claim | Requirement |
| --- | --- |
| JWT algorithm | ES256 under the current bounded DDS verification-key set. |
| `type` | Exactly `p2p-access`. |
| `iss` | Exactly `dds`. |
| `aud` | Exactly the single audience `auki-p2p`. |
| `sub` | Canonical DDS principal UUID; it is attribution, not protocol authorization. |
| `peer_id` | Canonical libp2p Peer ID and exact match for the Noise peer. |
| `domain_ids` | `1..=25` unique canonical DDS Domain UUIDs. |
| `iat`, `exp` | Required numeric dates with the current exact 30-minute lifetime, subject to the bounded verifier skew fixed in the TODO. |

`peer_type`, legacy `scopes`, application name/version, and other bounded
signed metadata may be present for diagnostics and compatibility. They are not
required authorization inputs and the base SDK does not reject an otherwise
valid token merely because its principal is not Robot, Compute, or Domain
Server. Unknown or unbounded token structures still fail the verifier's input
bounds.

**Why it blocks the TODO:** Current `auki-p2p` knows only Robot, Compute, and
Domain Server and hard-codes `domain-data:r`. The SDK includes native daemons,
mobile applications, browser participants, producers, and consumers.

**Issuance boundary:** DDS decides which upstream principals may receive a P2P
Domain access token. The SDK does not reproduce or second-guess that issuance
policy. D03 defines how a host supplies an issued token to the runtime; neither
the host's principal type nor its acquisition method changes how the presented
token is authorized.

**Rejected alternatives:**

- a generic SDK role hierarchy;
- a scope matrix for every existing Domain protocol;
- restoring cluster membership as a second authorization gate;
- treating a prior authentication or known-peer record as reusable authority;
  and
- allowing protocol handlers to accept tokens for a different Domain.

**Resolution proof:** An approved claim schema plus a protocol authorization
table showing the same Domain-token rule for every base protocol, with
anonymous, wrong-Peer-ID, wrong-Domain, and expired examples. Any exceptional
application-level policy must be called out explicitly rather than inferred
from `peer_type`.

### D03 — Credential acquisition, refresh, and verification

**Status:** LOCKED.

**Decision:** `auki-p2p`, `auki-domain`, and application protocol crates make
no DDS HTTP requests. The host application—or an optional adapter outside
those core crates—obtains and refreshes DDS material and pushes it into the
runtime through a narrow credential-installation API.

The host supplies:

- the current peer-bound DDS P2P Domain access token; and
- the current bounded DDS verification-key set and its updates.

The host owns all environment-specific acquisition concerns, including DDS
URLs, upstream user/machine authentication, SIWE or registration state, HTTP
timeouts/retries, browser sessions, and refresh scheduling.

The core runtime still owns the security boundary:

- parse and verify the token itself rather than trusting host-parsed claims;
- validate the accepted signing algorithm/key, issuer, audience, timestamps,
  canonical DDS Domain UUIDs, and Peer-ID binding;
- bind the token Peer ID to the runtime's stable local identity at install;
- retain only validated credential state;
- atomically replace credentials and verification keys;
- reject stale, malformed, mismatched, or expired updates;
- expose no token string or private identity key to application protocols; and
- fail closed for every new inbound or outbound application stream when no
  current local credential exists.

An application installs a refreshed token before expiry. If refresh fails, an
already-authenticated bounded application stream may finish according to that
protocol's existing lifetime rules, but no new stream opens after the installed
authority expires. Domain readiness reports unavailable until the host installs
a new valid credential.

The runtime exposes a safe Peer-ID challenge-signing capability so the host can
complete DDS binding without receiving the raw libp2p private key. The host
still owns the HTTP challenge ceremony.

We may later ship standard native or browser DDS adapters for convenience.
Those adapters use the same installation surface and are not dependencies of
`auki-p2p`, `auki-domain`, or the first migration TODO.

**Why it blocks the TODO:** Mutual authentication is unusable without a
credential source. Native wallet holders, headless daemons, mobile apps, and
browsers have different security and storage constraints.

**Deferred host-integration details:** Each product selects its upstream DDS
authentication method and may share helper adapters. Those choices do not
change the core push/verify/fail-closed contract. Exact skew, refresh margin,
key-overlap bounds, and retry defaults belong in the TODO once DDS's live
signing-key contract is recorded.

**Rejected alternatives:**

- embedding DDS HTTP endpoints or auth flows in `auki-p2p`;
- making every protocol depend on a DDS client;
- letting the host submit already-parsed claims without the signed token;
- exposing the private P2P identity to arbitrary credential code;
- allowing an expired cached token to open a new stream; and
- requiring one universal HTTP authentication mechanism for native, machine,
  mobile, and browser hosts.

**Resolution proof:** A fake host installs a token and verification key, opens
an authenticated stream, rotates both, then proves malformed, wrong-Peer-ID,
wrong-Domain, stale-key, and expired installations fail closed without exposing
application bytes. A separate host-adapter test owns any real DDS HTTP flow.

### D04 — Stable P2P identity

**Status:** LOCKED.

**Decision:** `auki_p2p::Identity` is the one canonical P2P identity type for
the SDK and Posemesh.

It owns exactly one Ed25519 libp2p keypair and derived Peer ID. The same
identity is used for:

- libp2p Noise authentication;
- DDS Peer-ID challenge signing;
- every `auki-domain` context and authenticated application protocol in the
  process; and
- direct and relayed routes.

The canonical persisted secret representation is libp2p's protobuf-encoded
Ed25519 private key. Loading rejects malformed, non-canonical, or non-Ed25519
keys. Generating an identity is explicit; a missing or invalid configured
production identity never silently falls back to a new random Peer ID.

Different hosts may obtain the canonical identity through input adapters
without creating different identity models:

- an existing SDK wallet derives the unchanged
  `Wallet::derive_child("peer/v1")` Ed25519 seed and constructs
  `auki_p2p::Identity`, preserving every current wallet-to-Peer-ID vector;
- a headless service loads canonical protobuf bytes from its host-owned secret
  store, file, environment integration, or platform key storage; and
- tests and explicit development tools may generate a new identity and export
  its canonical representation for persistence.

Secret-file, environment, wallet, and platform-storage I/O remain outside
`auki-p2p`; they all produce the same `Identity`. After construction, protocols
receive neither the raw private key nor an alternate signing implementation.
The runtime exposes only the bounded challenge-signing capability locked in
D03.

`auki_network::PeerIdentity` stops being a separate implementation. During the
agreed D15 compatibility window it may be a deprecated alias/re-export or a
thin constructor adapter returning `auki_p2p::Identity`; it must not retain a
second keypair type, Peer-ID derivation, or runtime identity owner.

**Why it blocks the TODO:** Existing SDK Peer IDs derive deterministically from
`Wallet::derive_child("peer/v1")`; Posemesh `auki-p2p::Identity` loads or
generates a canonical protobuf Ed25519 key. Replacing one with the other could
change deployed Peer IDs.

**Rejected alternatives:**

- retaining independent `PeerIdentity` and `Identity` keypair implementations;
- changing the existing `peer/v1` derivation label or wallet-to-Peer-ID result;
- using the wallet's signing key directly as the libp2p key;
- making protocol crates load files, environment variables, or wallets;
- exposing raw private keys to protocol handlers or DDS HTTP adapters; and
- automatically creating an ephemeral replacement when a stable identity fails
  to load.

**Resolution proof:** Existing seed-to-Peer-ID vectors plus a round-trip through
the canonical protobuf representation, the new runtime, Noise, and DDS
challenge signing. Wallet-derived and restored-protobuf constructions for the
same Ed25519 seed must produce the same Peer ID.

### D05 — Domain-owned runtime and Domain cardinality

**Status:** LOCKED.

**Decision:** One joined `Domain` owns exactly one internal `auki_p2p::Node`
and represents exactly one DDS Domain UUID.

The first migration does not support several concurrently joined Domain
contexts sharing one P2P node. It therefore needs no public SDK `Runtime`
composition object, cloneable runtime handle, Domain registry, reference
counting, or process-global singleton.

- `Domain::join()` creates and starts the internal node from the identity and
  host-provided configuration approved in D04 and D06.
- `Domain` owns the listener, credential installation state, verification
  state, route catalog, authenticated protocol servers, and their supervised
  tasks.
- All protocols exposed through that `Domain` share its one node, one Peer ID,
  and one selected DDS Domain UUID.
- A token containing other Domain claims does not implicitly create or join
  those Domains.
- `Domain::leave()` performs orderly protocol shutdown and then shuts down the
  node. `Drop` remains a fail-safe cancellation path; it is not the normal
  graceful-shutdown API.
- After leaving, an application may construct and join another `Domain`, using
  the same stable identity if it is still the same logical peer. There is no
  hidden runtime retained between the two instances.
- Tests may run several independent `Domain` instances in one test process to
  represent several logical peers. They do not share an internal node.

This preserves the existing product shape: applications own a `Domain`, and
`Domain` owns its networking. `auki-p2p::Node` remains independently usable by
lower-level Posemesh applications, but it does not become another required
public object in the `auki-domain` API.

Supporting one logical peer in several DDS Domains concurrently is deferred.
If a concrete product later requires it, that work must explicitly design a
shared-node owner and its credential, protocol, leave, and shutdown semantics;
the first migration does not pre-build that machinery.

**Why it blocks the TODO:** Without this boundary, implementation could
accidentally introduce a second public lifecycle, a hidden singleton, or
multi-Domain reference counting merely to replace the current `ClusterManager`.

**Rejected alternatives:**

- a new public process-wide SDK runtime that callers must construct and pass to
  every `Domain`;
- several Domain contexts sharing one node in the first migration;
- an implicit global node selected by whichever `Domain` joins first;
- one node or Peer ID per protocol; and
- automatically joining every Domain listed in a credential.

**Resolution proof:** One lifecycle test and ownership diagram showing
`Domain::join()` starting one node and all retained protocol servers,
authenticated protocol use through that node, `Domain::leave()` stopping the
servers before the node, and a later join restoring the same Peer ID from the
same D04 identity.

### D06 — Exact `Domain` API and lifecycle semantics

**Status:** LOCKED.

**Decision:** Preserve `Domain`, `DomainBuilder`, `Domain::join()`, and
`Domain::leave()` as the application lifecycle. Change their networking
meaning; do not add a second public runtime lifecycle.

`DomainConfig` contains only the inputs needed to create the D05-owned node:

- the exact DDS Domain UUID;
- the D04 `auki_p2p::Identity`;
- zero or more local listen addresses; and
- zero or more initial D13 peer routes.

The builder also receives the initial signed P2P credential and DDS
verification-key set obtained by the host under D03. Existing protocol-specific
builder inputs, such as message-channel declarations and providers, remain on
the builder rather than being folded into `DomainConfig`.

Conceptually, the Rust surface remains this small:

```rust
let domain = Domain::builder(peer, session, config)
    .authority(verification_keys, signed_token)
    .message_channel(channel, capacity)?
    .join()
    .await?;

domain.authority().install_verification_keys(next_keys).await?;
domain.authority().install_credential(next_token).await?;
domain.leave().await?;
```

The exact owned token/key wrapper names may be selected in the TODO, but the
public boundary is fixed: `Domain::authority()` is a narrow cloneable handle
for credential/key installation and DDS challenge signing. It cannot open
streams, access raw private-key material, control the node, or perform DDS
HTTP.

`join()` validates the Peer/Session/P2P identity chain and the initial
credential, starts the node, binds every configured listener, and installs all
built-in protocol handlers. It then returns without waiting for a remote peer.
Zero listeners and zero peer routes are valid; such a Domain is locally ready
but not presently reachable or able to dial anyone.

Readiness stays simple and local:

- `Ready`: the node and handlers are running and local Domain authority is
  current;
- `CredentialUnavailable`: the credential is missing or expired, so no new
  authenticated stream may open; and
- `Failed` or `Stopped`: the runtime terminated or `leave()` completed.

`Domain::status()` and a status subscription expose those states. Route count,
peer count, and Internet reachability are not part of readiness.

`leave(self)` stops accepting new work, stops protocol services, and shuts down
the node. It is bounded and returns cleanup failure. Dropping `Domain` is only a
best-effort cancellation fallback.

Keep catalogs, registry/blob fetches, map and typed streams, message channels,
peer observation, and other non-Manager Domain methods. Remove
`Domain::cluster_manager()` and all Manager/election/membership methods. Any
useful operation currently reachable only through `ClusterManager` moves to a
plain `Domain` method with the same product meaning.

**Rejected alternatives:** a public SDK runtime object, preserving
`ClusterTarget` or Discovery URL fields, making peer discovery part of
`join()`, treating zero peers as not ready, and keeping `cluster_manager()` as a
miscellaneous service locator.

**Resolution proof:** Native and Python API examples plus lifecycle tests for
zero-route join, authority expiry/reinstallation, fatal runtime failure, and
ordered explicit leave.

### D07 — Known-peer semantics and public observation

**Status:** LOCKED.

**Decision:** `known_peers` is an exact per-`Domain` observation of peers that
are both transport-connected and recently authenticated under a still-current
credential for that exact Domain.

- A peer first appears after one application stream completes mutual D02
  authentication. Noise or Identify alone is insufficient.
- A peer remains present while at least one underlying connection is live and
  the most recently verified remote credential has not expired.
- Every successful new authentication refreshes its observed authority
  deadline and metadata.
- The peer disappears immediately when its last connection closes or its
  observed credential expires. There is no disconnected grace cache.
- Already-open bounded streams follow D03 when authority expires, but the peer
  disappears and no new stream opens.
- Multiple connections and streams collapse into one record keyed by Peer ID.

The public record contains only the Peer ID, authenticated-until timestamp, and
authenticated participant metadata retained by D09. It does not expose tokens,
scopes as authority, raw observed addresses, Manager state, or a claim that the
peer is globally online.

`peer_count()` is exactly the current snapshot length. The public event stream
has `Appeared`, `Updated`, and `Disappeared` events, and subscribers can always
recover from lag by reading a fresh snapshot. Events and snapshots never grant
authorization; every stream authenticates again.

Applications may address an operation by an observed Peer ID. The operation
uses an existing connection or a route held by the separate D13 route catalog;
the known-peer record is not itself a dial hint.

**Rejected alternatives:** Manager-like membership, an eventually consistent
global roster, indefinite recently-seen caching, counting Noise-only peers,
using `known_peers.contains()` as authorization, and publishing connection
addresses as trusted peer data.

**Resolution proof:** One deterministic state table/test covering first auth,
parallel connections, refresh, expiry with an open connection, last-connection
closure, reconnect, and subscriber lag recovery.

### D08 — Authenticated protocol extension boundary

**Status:** LOCKED.

**Decision:** Keep two simple public layers.

1. `auki-p2p::Node` is the supported low-level API for standalone protocol
   crates such as `auki-p2p-dataset` and for Posemesh applications that own
   their node directly.
2. `Domain::protocols()` returns a cloneable `DomainProtocols` handle for code
   that must add a protocol to the Domain-owned node.

`DomainProtocols` is deliberately smaller than `Node`. It can:

- register one unique versioned protocol ID with concurrency/frame bounds and
  an authenticated-stream handler;
- open an authenticated stream to an expected Peer ID using the Domain route
  catalog; and
- open one exact validated D13 route when a protocol requires route pinning.

The Domain UUID and mutual-authentication requirements are fixed by the owning
`Domain`; protocol authors cannot weaken or replace them. The handle exposes no
token strings, verification keys, identity secret, raw swarm, listener control,
peer allow-list, or node shutdown.

Registration returns an RAII handle. Dropping it removes that handler, and
`Domain::leave()` cancels all registrations even if a clone remains elsewhere.
Duplicate protocol IDs fail rather than replace an existing handler. SDK-owned
and third-party protocol crates use the same boundary.

Normal applications should receive narrow protocol-specific clients and
servers, not `DomainProtocols`. No generic plugin framework, dependency
injection container, or new protocol trait hierarchy is required.

**Rejected alternatives:** exposing the raw swarm, making every new protocol a
method inside a central runtime loop, giving protocols credential access,
restricting registration to Auki-owned crates, and inventing a plugin system
before a second external protocol needs one.

**Resolution proof:** `auki-p2p-dataset` plus one retained Domain protocol both
serve and open authenticated streams, while compile-time/API inspection shows
the Domain extension handle cannot read credentials or stop the node.

### D09 — Existing protocol disposition and authorization matrix

**Status:** LOCKED.

**Decision:** Every retained base protocol uses exactly D02. There are no
protocol-specific role, scope, Manager, membership, or known-peer gates.
Existing owner/target/hash/size/queue/payload checks remain because they protect
the operation itself.

| Current protocol | Decision | Payload/business rules |
| --- | --- | --- |
| join and membership | Remove | Manager admission and roster semantics have no replacement. |
| heartbeat | Remove | D10 removes Manager liveness and Domain-time sampling. |
| info | Adapt | Keep Peer ID, application/version, session, display metadata, and useful clock descriptors; remove Manager, membership, role-authority, and route fields. |
| resources `0.2.0`, `0.3.0`, `0.4.0` | Keep | Reuse the existing payload codecs, bounds, resource ownership, and provider behavior byte-for-byte behind D11 IDs. |
| registries `0.2.0`, `0.3.0` | Keep | Preserve listing, owner checks, kind checks, and hash-pinned fetches byte-for-byte. |
| blobs `0.1.0` | Keep | Preserve bounded, content-addressed fetch and hash validation byte-for-byte. |
| message `0.1.0` | Keep | Preserve addressed receiver ownership, bounded queues, ACK behavior, and typed payload validation. |
| native stream `0.2.0` | Keep | Preserve expected producer, manifest, bounds, and typed stream validation. |
| browser stream `0.1.0` | Remove | The later D12 browser implementation uses the retained `0.2.0` application contract instead of a second stream model. |
| diagnostic `0.0.1` | Remove | Cluster-wide diagnostic broadcast is Manager-era control, not a base Domain data protocol. Local metrics/logging remain local. |
| browser session `0.0.1` | Remove | Browser-to-Manager control disappears. |
| browser probe `0.0.1` and browser-full-peer experiments | Remove from the product wire surface | Transport experiments may retain private tests without becoming Domain protocols. |
| libp2p Identify, ping, Noise, Yamux, CRv2 | Keep as transport internals | They provide identity, health, multiplexing, or reachability and never authorize application data. |

For outbound operations that name a peer or resource owner, the authenticated
Noise Peer ID must match that expected Peer ID before the existing payload
rules run. Authority expiry follows D03: a bounded authenticated stream may
finish, but no new stream opens.

The adapted info payload is the only retained protocol whose application bytes
change in this migration. All other retained codecs stay unchanged; D11 changes
their negotiated protocol IDs so authenticated and unauthenticated peers cannot
be confused.

**Rejected alternatives:** retaining control protocols with new names, adding a
role matrix, authorizing message or registry access from cached membership, and
redesigning retained payloads merely because their transport changed.

**Resolution proof:** The table above becomes executable coverage: one positive
same-Domain case and anonymous, wrong-Peer-ID, wrong-Domain, and expired cases
for every retained handler, plus preservation vectors for unchanged codecs.

### D10 — Domain time and heartbeat

**Status:** LOCKED.

**Decision:** Remove Manager heartbeat and synchronized Domain-time behavior.
Do not design a replacement time protocol in this migration.

`auki-time`, local monotonic clocks, session clocks, clock descriptors, and
timestamped product data remain. What disappears is the claim that
`auki-domain` can derive one shared current time from a Manager or arbitrary
peer samples.

Remove `domain_time_now`, `domain_clock_estimate`, heartbeat source selection,
clock-quality state tied to cluster membership, and diagnostic behavior that
depends on them. libp2p ping/connection observation is sufficient for transport
health and does not become a public heartbeat protocol.

If a product later demonstrates a need for synchronized Domain time, it gets a
separate authenticated protocol and explicit clock/source contract. That work
does not block authenticated resources, registries, blobs, messages, or
streams.

**Rejected alternatives:** preserving heartbeat only for convenience, electing
a new time leader, treating every peer as an equally trusted clock source, and
building a consensus/time-synchronization subsystem during this migration.

**Resolution proof:** Remove the heartbeat/Domain-time APIs and show that
session/log timestamping and every retained D09 protocol still operate from
their existing local/session clock inputs.

### D11 — Wire versions and compatibility window

**Status:** LOCKED.

**Decision:** Make one clean authenticated wire break. Do not negotiate,
serve, or fall back to an unauthenticated legacy protocol in the new runtime.

All retained Auki protocols use the prefix `/auki/auth/1/`. The `1` versions
the common mutual-authentication exchange; the remaining suffix identifies the
application payload codec:

| Legacy ID | Authenticated ID |
| --- | --- |
| `/auki/info/0.0.1` | `/auki/auth/1/info/1.0.0` |
| `/auki/resources/0.2.0` | `/auki/auth/1/resources/0.2.0` |
| `/auki/resources/0.3.0` | `/auki/auth/1/resources/0.3.0` |
| `/auki/resources/0.4.0` | `/auki/auth/1/resources/0.4.0` |
| `/auki/registries/0.2.0` | `/auki/auth/1/registries/0.2.0` |
| `/auki/registries/0.3.0` | `/auki/auth/1/registries/0.3.0` |
| `/auki/blobs/0.1.0` | `/auki/auth/1/blobs/0.1.0` |
| `/auki/message/0.1.0` | `/auki/auth/1/message/0.1.0` |
| `/auki/stream/0.2.0` | `/auki/auth/1/stream/0.2.0` |

The resource, registry, blob, message, and stream payload version remains in
the ID because those codecs remain byte-identical under D09. Info uses a new
application version because its Manager-era fields are removed.

Removed D09 protocols receive no authenticated replacement ID. Unsupported
protocol negotiation fails before the authentication exchange and before any
application bytes. Authentication failure closes that stream and never retries
an old ID.

There is no dual-stack window in one process. Existing deployed consumers stay
on the prior SDK release until upgraded as a coordinated group under D15.

**Rejected alternatives:** reusing old IDs, capability-negotiated downgrade,
serving authenticated and unauthenticated handlers together, and changing
unchanged payload codecs only to make their version numbers look newer.

**Resolution proof:** Fixed wire vectors for authentication followed by each
retained payload, plus tests proving old IDs are unsupported and an auth failure
never emits or retries legacy application bytes.

### D12 — Native and browser target

**Status:** LOCKED.

**Decision:** Ship the authenticated runtime in stages rather than making every
platform block the native migration.

| Stage | Product target | Transport/runtime choice |
| --- | --- | --- |
| 1 | Native Rust, then its Python binding | Upstream Rust libp2p `0.56`, Tokio, TCP/DNS, Noise, Yamux; stock CRv2 client for an explicitly supplied circuit route. |
| 2 | Swift/iOS | Bind the same native Rust implementation; do not retain the old Swift `NetworkRuntime`. |
| 3 | Browser | Keep `auki-domain-browser` as the product facade and use one TypeScript/js-libp2p engine implementing the same D02/D11 wire contract. |

The Rust/WASM browser runtime is not a second supported Domain engine. Retain
useful test vectors while replacing it, then remove it. The first browser
transport slice is WebSocket plus stock circuit relay; WebRTC Direct and other
browser transports may be added later as route mechanisms without changing
authentication or Domain APIs.

Native QUIC and WebSocket listeners are not required for Stage 1. They may be
added when a concrete route/platform needs them. Browser credential acquisition
and refresh remain host-owned under D03.

Every stage must use the same Peer-ID binding, DDS Domain UUID, authentication
exchange, protocol IDs, and application codec vectors. A stage is not released
with Manager authorization as a temporary substitute.

**Rejected alternatives:** blocking native work on all bindings, maintaining
both TypeScript and Rust/WASM browser Domain engines, porting DDS HTTP into the
runtime, and treating transport parity as permission to diverge on auth or
payload framing.

**Resolution proof:** Stage 1 native tests are required by D16. Before Stage 3
ships, one browser/native fixed-vector suite and one mutually authenticated
native-to-browser stream must pass.

### D13 — Routes, direct reachability, and relays

**Status:** LOCKED.

**Decision:** `Domain` owns a bounded catalog of explicit, untrusted route
candidates keyed by expected Peer ID. Discovery and relay-provider booking are
not part of `auki-domain`; CRv2 transport and reservation mechanics remain part
of `auki-p2p`.

The word "booking" is important here. The responsibilities are:

| Responsibility | Owner |
| --- | --- |
| Decide which relay provider this peer is authorized to reserve, with what limits and until when | An external product control-plane adapter. In Posemesh today this is the compute node's DMS relay-booking coordinator. |
| Validate `RelayProvider`, establish the exact direct relay connection, request/confirm/renew a CRv2 reservation, expose the confirmed circuit route, and cancel it safely | `auki-p2p::Node` and its relay reservation API. |
| Add/remove the confirmed route from Domain publication or dialing state | The host/Domain integration driving the Domain route catalog. |
| Connect to a target over an exact circuit and mutually authenticate the application stream | `auki-p2p`. |
| Run the relay server and choose its admission/capacity policy | Standalone relay infrastructure, not `auki-domain`. |

In other words, `auki-p2p` knows **how** to use and maintain an assigned relay;
it deliberately does not know **which** relay a product is entitled to use.

The Stage 1 route types are:

- a canonical direct TCP multiaddr whose optional terminal `/p2p/<peer>` must
  match the route's expected Peer ID; and
- a complete stock CRv2 circuit multiaddr ending
  `/p2p/<relay>/p2p-circuit/p2p/<expected-peer>`.

`DomainConfig` supplies initial routes. Plain `Domain` methods add, replace, and
remove routes later. Inputs are canonicalized, deduplicated, bounded, and never
treated as authorization. Exact numeric bounds are implementation constants in
the TODO, not public product policy.

An outbound operation may reuse an existing connection to the expected peer or
dial one of that peer's current candidates. It does not dial a cached known-peer
address, invent a route, or fall back to another target. D02 authentication is
still required after transport connection.

The host owns where candidates and relay-provider assignments come from and
decides their desired lifetime. `auki-p2p` owns the resulting transport
reservation until explicit cancellation. Removing an expired circuit candidate
prevents new use; existing bounded authenticated streams may finish.

When general SDK relay publication is implemented, `Domain` exposes only a
thin relay-reservation facade over its D05-owned `auki-p2p::Node`: the host
supplies an authorized `RelayProvider`, and `auki-p2p` returns the confirmed
circuit route/handle. That facade performs no DMS HTTP and contains no booking
policy.

Posemesh DMS relay booking remains one concrete external provider-assignment
adapter: `compute-node/src/relay_booking.rs` calls DMS, constructs the
`RelayProvider`, and drives the `auki-p2p` reservation lifecycle.
`auki-domain-relay` is no longer created or controlled by a Manager; a generic
standalone CRv2 relay binary may remain as independent infrastructure.

Relay booking and browser reachability are not required in D16. The first
vertical slice is direct TCP.

**Rejected alternatives:** selecting discovery now, embedding DMS provider
selection/booking HTTP in the SDK, moving CRv2 reservation mechanics out of
`auki-p2p`, restoring Manager relay ownership, trusting advertised addresses,
unbounded route caching, and requiring a relay before direct authenticated
protocols can ship.

**Resolution proof:** Canonical route vectors plus a two-peer direct test that
rejects wrong target suffixes, stale removal, Noise Peer-ID mismatch, and valid
transport with invalid Domain authority.

### D14 — Canonical repository and crate boundaries

**Status:** LOCKED.

**Decision:** `auki-sdk` becomes the canonical source repository for both
`auki-p2p` and `auki-p2p-dataset`.

```text
auki-p2p                       (identity, auth, routes, transport, streams)
    ↑
auki-p2p-dataset               (independent reusable protocol)

auki-network                   (temporary retained wire codecs/types only)
    ↑                         ↗
auki-domain  ─────────────── auki-p2p
    ↑
language bindings / applications

Posemesh ────────────────→ versioned auki-p2p crates
```

- Add publishable `crates/auki-p2p` and `crates/auki-p2p-dataset` packages to
  `auki-sdk`.
- Posemesh temporarily consumes an exact Git revision, then a normal pinned
  crate version after the first release.
- Delete the Posemesh source copies as soon as that dependency switch lands;
  fixes then happen only in `auki-sdk` and are consumed by version/revision.
- `auki-p2p` depends on neither `auki-domain` nor `auki-network`.
- `auki-p2p-dataset` depends only on `auki-p2p` plus its data dependencies.
- `auki-domain` owns Domain protocol handlers and depends on `auki-p2p`.

Do not move every existing codec merely for architectural aesthetics.
`auki-network` remains temporarily as the home of retained D09 wire codecs and
plain protocol types, but loses its swarm, runtime, allow-list, Discovery,
Manager-control, join, membership, heartbeat, and diagnostic responsibilities.
It is not an alternate engine. Later extraction or renaming requires a concrete
reuse reason and does not block this migration.

During the D15 compatibility release, `auki_network::PeerIdentity` may be one
deprecated alias/constructor adapter for `auki_p2p::Identity`. It contains no
separate key or derivation implementation and is removed in the following
breaking release.

**Rejected alternatives:** keeping canonical copies in two repositories,
making `auki-p2p` depend on the Domain facade, moving each codec into a new
crate, and retaining `auki-network` as a second runtime under a compatibility
feature.

**Resolution proof:** Package graphs in both repositories show one source of
`auki-p2p`; Posemesh tests run against the pinned SDK revision/version; a source
search finds no old swarm/runtime/allow-list owner in `auki-network` after
cutover.

### D15 — Bindings and breaking releases

**Status:** LOCKED.

**Decision:** Treat removal of Manager semantics as one honest breaking SDK
release. Keep product package names and useful Domain concepts; do not ship a
legacy Manager feature or runtime downgrade.

| Surface | Release decision |
| --- | --- |
| Rust `auki-domain` | First authenticated Domain release. Remove `ClusterManager`, `ClusterTarget`, elections, membership mutation, Manager relay ownership, and Domain-time APIs. Promote retained operations to `Domain`. |
| Python `auki-domain` | Ship with the same native release and expose the same retained Domain concepts. Remove Python Manager objects rather than emulate them. |
| Swift | Keep the previous package line until Stage 2. Its next release binds the new Domain owner and removes public `NetworkRuntime`/Manager semantics. |
| Browser | Keep the previous package line until Stage 3. Its next release preserves the `auki-domain-browser` product facade but replaces Manager join/control internally. |

Under the repository's current pre-`1.0` versioning, the authenticated Rust and
Python line is `0.1.0`; if versions advance before implementation, use the next
minor version and mark it breaking. Swift and browser increment their own
breaking minor when their stages ship.

There is no release in which the new Domain silently talks to legacy peers.
Unsupported bindings remain pinned to their prior package versions and are not
advertised as compatible with the new authenticated Domain line.

The only temporary source-compatibility aid is the honest D14
`PeerIdentity`-to-`Identity` alias/constructor for one release. Manager-shaped
APIs receive no aliases because their meaning is gone.

Migrate all in-repository consumers for a shipping stage in the same stage:
the Rust Domain tests, `examples/diagnostic-app` (despite its legacy name), and
the Python Domain binding/tests for Stage 1; Swift examples/tests in Stage 2;
browser examples/tests in Stage 3. External consumers receive a migration guide
from Manager methods to plain Domain methods.

**Rejected alternatives:** indefinite deprecation, a `legacy-manager` feature,
keeping old package versions while changing their wire behavior, forcing every
language to ship simultaneously, and exposing the raw new runtime as the
replacement for every removed API.

**Resolution proof:** A checked Rust/Python API diff and migration of all Stage
1 in-repository consumers, followed by equivalent independent gates for Swift
and browser.

### D16 — First vertical slice and cutover boundary

**Status:** LOCKED.

**Decision:** The first vertical slice is native Rust, direct TCP, and the
existing resource-catalog `0.2.0` request/response in both directions through
the D05 Domain facade.

The acceptance scenario is intentionally small:

1. Two peers create stable D04 identities and receive peer-bound DDS tokens for
   the same D01 Domain UUID.
2. Each calls `Domain::join()` with an explicit direct route and no Discovery
   or Manager.
3. Peer A serves its existing `0.2.0` catalog through
   `/auki/auth/1/resources/0.2.0`.
4. Peer B fetches it through the retained `Domain` method.
5. Both observe one authenticated known peer under D07.
6. Wrong-Peer-ID, wrong-Domain, expired-token, and legacy-ID attempts expose no
   catalog bytes.
7. Both call `Domain::leave()` and all protocol/runtime tasks terminate.

Use the refactored in-repository `examples/diagnostic-app` as the first manual
application proof; it no longer uses the removed diagnostic broadcast.

Implementation may land canonical crates and private adapters incrementally,
but no executable mode may start old and new swarms together. The public
`Domain` cutover ships only after every retained protocol needed by that release
uses the one canonical node. There is no mixed authorization mode and no
runtime fallback.

The old allow-list, join, and membership gates are removed when the new node
becomes the Domain owner, not after a later grace period. Rollback means
reverting the whole unreleased cutover or deploying the prior SDK release; it
does not mean negotiating legacy protocols at runtime.

After this slice passes, migrate the remaining D09 protocols one at a time on
the same node, then complete the Stage 1 Python surface. Relay and browser work
remain later stages.

**Rejected alternatives:** starting with discovery, relay booking, messaging,
or browser support; running two swarms; keeping the membership allow-list as a
backup; and calling an outbound-only mock a vertical slice.

**Resolution proof:** The seven-step scenario above runs against real libp2p
transports and signed test credentials, followed by the refactored manual
example and clean task-leak checks.

## 7. Decisions that do not block the TODO

Unless a blocking decision above pulls them into scope, the following remain
deferred or may be selected as implementation defaults in the TODO:

- mDNS, Kademlia, Rendezvous, Gossipsub, DDS, DMS, or another discovery source;
- global topology convergence or a replacement cluster roster;
- a generalized capability-advertisement protocol;
- extracting every existing protocol into its own crate;
- production relay booking for general SDK participants;
- redesigning resource, registry, blob, message, or stream payloads;
- performance tuning beyond preserving current documented bounds;
- exact internal channel sizes, retry intervals, and metric names; and
- unrelated cleanup in `auki-session`, registries, logs, maps, or layouts.

These items must not be smuggled into the integration TODO merely because the
old Manager happened to own them.

## 8. Resolution order

The decisions were resolved in four short passes. The implementation TODO may
refine private details, but may not silently change their product meaning.

### Pass A — Identity and trust

D01, D02, D04, then D03 locked:

- canonical Domain identifier;
- DDS P2P Domain-token authorization model;
- native/browser issuance and refresh ownership; and
- stable Peer ID/key compatibility.

### Pass B — Runtime and product API

D05–D08 locked:

- Domain-owned single-runtime lifecycle;
- exact Domain API/lifecycle;
- known-peer semantics; and
- the protocol-author extension boundary.

### Pass C — Protocols and reachability

D09–D13 locked:

- protocol disposition and authorization;
- Domain time;
- wire cutover;
- native/browser target; and
- routes/relay ownership.

### Pass D — Delivery boundary

D14–D16 locked:

- canonical crate ownership;
- binding/release policy; and
- the first vertical slice and cutover.

## 9. Gate before writing `TODO_AUKI_P2P_INTEGRATION.md`

The decision gate is closed:

- [x] D01–D16 are marked `LOCKED` with one explicit answer each.
- [x] The Domain identifier has one canonical cross-language representation.
- [x] Every platform in the first release has a documented host-side path for
      obtaining tokens and verification keys through the locked D03 installation
      boundary.
- [x] The credential claim schema and the D02 Domain-token rule across every
      retained protocol are frozen.
- [x] The Domain-owned one-node lifecycle diagram is approved.
- [x] Every supported host constructs the same canonical
      `auki_p2p::Identity`, and existing wallet-derived Peer-ID vectors remain
      unchanged.
- [x] The Rust `DomainConfig`, `Domain::join`, readiness, `leave`, peer
      observation, and shutdown APIs are sketched precisely.
- [x] Every current protocol has a keep/adapt/remove decision.
- [x] Every retained protocol has an old/new wire-ID and compatibility decision.
- [x] The native/browser transport and package matrix is explicit.
- [x] Direct-route and any in-scope relay ownership are explicit.
- [x] The canonical crate/repository graph and duplicate-removal cutover are approved.
- [x] Rust, Python, Swift, and browser API changes are classified by release.
- [x] The first vertical slice has one runtime, one trust model, and one
      end-to-end acceptance scenario.
- [x] Discovery remains outside the implementation critical path.

The companion design now reflects the locked decisions. The implementation
order, file boundaries, focused commits, tests, consumer migration, platform
stages, and final deletion audit are recorded in
[`TODO_AUKI_P2P_INTEGRATION.md`](./TODO_AUKI_P2P_INTEGRATION.md).
