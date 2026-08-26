# TODO: Integrate `auki-p2p` into `auki-domain`

**Status:** Ready for implementation.

**Last updated:** 2026-08-26.

**Locked decisions:**
[`PLAN_AUKI_P2P_INTEGRATION.md`](./PLAN_AUKI_P2P_INTEGRATION.md).

**Approved design:**
[`2026-08-25-auki-p2p-integration-and-cluster-removal-design.md`](./2026-08-25-auki-p2p-integration-and-cluster-removal-design.md).

This is the implementation ledger for D01–D16. It does not reopen those
decisions. Each numbered prompt is intended to be reviewed and committed on its
own unless it explicitly says that two repositories must move together.

## 1. Rules for executing this TODO

- Work in order. A later prompt may assume every earlier prompt is merged.
- Start each prompt from a green tree and finish it with the listed proof.
- Keep commits narrow. Do not mix formatting or unrelated cleanup into a
  migration commit.
- Add the failing test or fixed vector before changing the behavior when the
  prompt changes a security or lifecycle contract.
- Do not run the old and new swarms in one executable, even temporarily.
- Do not add an unauthenticated fallback, legacy protocol retry, Manager
  compatibility mode, fake leader, or synthetic membership.
- Do not put DDS HTTP, SIWE, registration, retry policy, or token refresh loops
  in `auki-p2p`, `auki-domain`, or a protocol crate.
- Do not use `peer_type`, scopes, known peers, route provenance, or transport
  connection as base protocol authorization.
- Do not choose or implement a discovery system in this migration.
- Do not put relay-provider selection or DMS booking HTTP in `auki-domain`.
  `auki-p2p` owns CRv2 reservation mechanics after a host supplies an
  authorized `RelayProvider`.
- Do not create a general plugin framework. The only extension boundary is the
  restricted `DomainProtocols` handle locked in D08.
- Preserve existing payload codecs and business bounds unless D09 explicitly
  says the payload changes.
- If implementation reveals a contradiction with D01–D16, stop and amend the
  plan first. Do not silently choose a different product rule in code.

Repository names below mean:

- **SDK:** this `auki-sdk` repository; and
- **Posemesh:** the sibling `posemesh/core` workspace that currently owns the
  source copies of `auki-p2p` and `auki-p2p-dataset`.

## 2. Frozen implementation contracts

These constants and public shapes complete the choices intentionally left to
the TODO. Changing one is a reviewed contract change, not an incidental coding
choice.

### 2.1 Credential and key bounds

- Authentication exchange timeout: 10 seconds per direction.
- Maximum encoded token: 64 KiB.
- Token lifetime: exactly 30 minutes.
- Verifier clock skew: 60 seconds. Skew may tolerate a future `iat`/`nbf` at
  verification, but literal `exp` wins for local authority, known-peer state,
  and permission to open a new stream.
- Verification keys: one current and at most one previous ES256 key, each at
  most 64 KiB encoded.
- Previous-key overlap: at least 31 minutes (token lifetime plus skew).
- Verification-key maximum staleness without an exact host refresh: 60 minutes.
- A key update carries a monotonic `u64` generation. A lower generation or the
  same generation with different key bytes is stale and rejected; the same
  generation with identical bytes refreshes last-seen time atomically.
- Diagnostic `peer_type`, when present, is at most 64 visible ASCII bytes.
  Diagnostic scopes are at most 32 unique strings of at most 128 visible ASCII
  bytes each. They are never authorization.

The public host boundary uses redacting owned wrappers with these meanings:

```rust
DdsVerificationKeys { generation, current_es256_pem, previous_es256_pem }
SignedP2pCredential  // owns the compact token; Debug never prints it

DomainAuthority::install_verification_keys(keys)
DomainAuthority::install_credential(credential)
DomainAuthority::sign_peer_challenge(challenge)
```

The implementation may use borrowing internally, but it must not expose a raw
private identity key or parsed-but-unverified claims.

### 2.2 Domain and route bounds

- One `Domain` owns one node and exactly one DDS Domain UUID.
- Maximum listen addresses: 16.
- Maximum expected peers with configured routes: 1,024.
- Maximum candidates per expected peer: 16.
- Maximum configured route candidates across one Domain: 4,096.
- Maximum encoded multiaddr: 1,024 bytes before parsing.
- Duplicate canonical routes do not consume a second slot.
- Direct routes use exact `ip4|ip6|dns|dns4|dns6 / tcp(nonzero)` grammar with an
  optional matching terminal `/p2p/<expected-peer>`.
- Circuit routes are complete CRv2 routes ending
  `/p2p/<relay>/p2p-circuit/p2p/<expected-peer>`.
- Domain peer-observation events use a bounded channel of 256 entries and an
  authoritative snapshot for lag recovery.
- Explicit `Domain::leave()` has a 30-second cleanup deadline. At the deadline
  it cancels/aborts all owned tasks, closes listeners, and returns a typed
  cleanup error; it does not detach work.

The intended host-facing route surface is deliberately small:

```rust
DomainRoutes::replace(expected_peer, candidates)
DomainRoutes::remove(expected_peer)
DomainRoutes::snapshot()
```

Routes remain dial hints. None of these methods grants authority.

### 2.3 Protocol authoring surface

`DomainProtocols` fixes the owning Domain requirement and exposes only:

```rust
DomainProtocols::register(spec, handler)
DomainProtocols::open(expected_peer, protocol)
DomainProtocols::open_exact(expected_peer, route, protocol)
```

`register` returns an RAII registration. Duplicate IDs fail. The registration
ends on Drop or Domain leave. Frame/concurrency values are explicit bounded
fields of `spec`; protocol implementations select smaller limits appropriate
to their existing wire contracts.

## 3. Stage 1 completion target

Stage 1 is complete when native Rust and Python use one Domain-owned
authenticated node and:

- `auki-sdk` is the canonical source of publishable `auki-p2p` and
  `auki-p2p-dataset` crates;
- Posemesh consumes those crates at one exact revision or pinned release and no
  longer contains source copies;
- the retained info, resources, registries, blobs, message, and stream
  protocols use the D11 authenticated IDs and the D02 Domain-token rule;
- `Domain`, `DomainBuilder`, `join`, `leave`, catalogs, registries, blobs,
  messages, streams, routes, and peer observation remain understandable public
  APIs;
- Manager, membership, election, heartbeat, Domain time, central allow-list,
  and Manager-owned Discovery/control paths are absent from the active native
  runtime;
- Python exposes the new Domain lifecycle without `ClusterManager`;
- the diagnostic app proves the direct two-peer vertical slice; and
- there is no discovery requirement or SDK relay-booking service.

Swift and browser work are separate stages at the end of this ledger. Their
existing released package lines remain available while Stage 1 changes the
native/Python line.

## 4. Implementation sequence

### P00 — Record the migration baseline

This is a preflight, not a product-code commit.

**Work**

- Record the SDK revision and the Posemesh revision from which the two P2P
  crates are imported.
- Run the current SDK native test suite and record any pre-existing failures.
- Run the current Posemesh `auki-p2p`, `auki-p2p-dataset`, and compute-node
  suites.
- Record the current public Rust, Python, Swift, and browser package versions
  before the breaking native line begins.
- Confirm that neither worktree contains unrelated edits that overlap the
  migration files.

**Proof**

- A short baseline note in the first PR description identifies both source
  revisions and any accepted pre-existing gate failure.
- No file is changed solely to make the baseline look green.

---

### P01 — Import the canonical P2P crates into `auki-sdk`

**Goal:** Move the already-tested implementation without changing its behavior.

**SDK files**

- `Cargo.toml`
- `Cargo.lock`
- `crates/auki-p2p/**` (new)
- `crates/auki-p2p-dataset/**` (new)

**Work**

- Import `posemesh/core/auki-p2p` and `posemesh/core/auki-p2p-dataset`, including
  their tests and READMEs, from the exact P00 revision.
- Add both crates to the SDK workspace and centralize their libp2p `0.56` family
  dependencies in the workspace manifest.
- Preserve the exact `libp2p-stream` version paired with libp2p `0.56`.
- Make the SDK copies publishable and give them SDK-owned package metadata.
- Do not modify the Posemesh copies or switch any consumer in this prompt.
- Do not fold either crate into `auki-domain`; both remain useful low-level
  public crates.

**Required proof**

- All imported unit and integration tests pass unchanged.
- Strict Clippy passes for both crates.
- A reviewable import diff identifies every intentional manifest-only change
  from the recorded Posemesh source revision.

**Commit:** `feat(p2p): import authenticated runtime crates`

---

### P02 — Unify stable P2P identity

**Goal:** Make `auki_p2p::Identity` the only native P2P key owner without
changing existing SDK wallet-derived Peer IDs.

**Primary files**

- `crates/auki-p2p/src/identity.rs`
- `crates/auki-p2p/src/lib.rs`
- `crates/auki-network/src/lib.rs`
- the SDK wallet/peer identity adapter and its tests

**Work**

- Keep canonical libp2p protobuf private-key import/export for host-owned key
  files.
- Add the minimum constructor needed to create an Ed25519 identity from the
  existing `Wallet::derive_child("peer/v1")` result. Do not add a wallet
  dependency to `auki-p2p`.
- Keep explicit random generation for tests and deliberate ephemeral tools.
- Make production load/parse failures fail instead of generating a new key.
- Replace `auki_network::PeerIdentity` with a one-release deprecated adapter or
  alias that owns no second keypair. Preserve source compatibility only where
  it does not preserve the old runtime.
- Keep challenge signing on the narrow public identity/authority surface;
  never expose raw secret bytes to credential acquisition code.

**Required proof**

- A fixed wallet seed produces the exact same Peer ID before and after the
  change.
- Protobuf write/read round-trips the same Peer ID.
- Wrong key type, corrupt bytes, and missing production key all fail closed.
- A test proves the deprecated adapter and `auki_p2p::Identity` cannot diverge.

**Commit:** `refactor(identity): make auki-p2p identity canonical`

---

### P03 — Generalize the DDS Domain-token verifier

**Goal:** Implement D02 and D03 in the canonical crate, removing Posemesh-only
role/scope authorization from the transport boundary.

**Primary files**

- `crates/auki-p2p/src/token.rs`
- `crates/auki-p2p/src/authority.rs`
- `crates/auki-p2p/src/transport.rs`
- `crates/auki-p2p/src/runtime.rs`
- `crates/auki-p2p/tests/authenticated_transport.rs`
- `crates/auki-p2p-dataset/src/lib.rs`

**Work**

- Accept the exact D02 ES256 `p2p-access` profile: `iss=dds`, only
  `aud=auki-p2p`, canonical principal UUID, exact Noise `peer_id`, `1..=25`
  unique canonical Domain UUIDs, and the locked 30-minute lifetime with the
  Section 2.1 60-second verifier skew and literal-expiry rule.
- Replace the single verification key with the Section 2.1 atomically
  replaceable current/previous key ring. Enforce generation, overlap, and
  staleness rules; reject unknown algorithms, duplicate keys, unbounded
  structures, malformed dates, and stale replacement attempts.
- Retain bounded signed `peer_type`, scopes, and application metadata only as
  diagnostics. Do not require or consult them for base SDK authorization.
- Make session requirements express the exact Domain and, when applicable, the
  expected Peer ID. Remove the generic expected-role gate.
- Keep every new stream dependent on a current local credential. Existing
  bounded authenticated streams may finish after local expiry.
- Allow the generic protocol ID value type to carry both canonical SDK
  `/auki/auth/1/...` IDs and other valid application namespaces such as the
  Posemesh dataset ID. This is validation of a versioned ID, not legacy SDK
  fallback.
- Move the dataset's Robot/Compute rule, if Posemesh still requires it, into an
  explicit dataset-level metadata check performed after mutual authentication
  and before dataset request bytes. The generic transport must not know that
  rule.

**Required proof**

- Fixed tests cover valid auth, wrong signature, algorithm, issuer, audience,
  Peer ID, Domain, lifetime, duplicate Domain, missing local authority, and
  expired local/remote authority.
- Unknown bounded `peer_type` and extra diagnostic scopes authenticate under
  the same valid Domain token.
- Key rotation accepts the documented overlap and rejects stale/unknown keys.
- No application payload byte is exposed on any negative vector.
- Dataset role-policy tests, if retained, fail before dataset request bytes and
  do not change base `auki-p2p` authorization.

**Commit:** `feat(auth): make domain tokens the p2p authorization baseline`

---

### P04 — Add connection/authentication observation to `auki-p2p`

**Goal:** Give `auki-domain` enough facts to implement D07 without moving peer
authority into the Domain facade.

**Primary files**

- `crates/auki-p2p/src/transport.rs`
- `crates/auki-p2p/src/runtime.rs`
- `crates/auki-p2p/src/lib.rs`
- new focused observation tests

**Work**

- Expose a bounded, lag-recoverable observation surface for:
  successful mutual authentication, connection establishment/closure, and
  fatal node termination.
- Authentication observations contain the exact Peer ID, Domain UUID,
  verified-until time, connection identity, and bounded diagnostic participant
  metadata. They never contain the signed token.
- Ensure parallel connections can be distinguished so a Domain can remove a
  peer only after its final connection closes.
- Make subscriber lag explicit. The authoritative local snapshot must be
  readable after lag; events themselves are not state or authority.
- Do not add a public membership map, reachability database, grace period, or
  authorization shortcut to `Node`.

**Required proof**

- Deterministic tests cover first auth, repeated auth, two connections, one
  closure, final closure, credential expiry with a live connection, reconnect,
  and subscriber lag recovery.
- Noise-only and Identify-only connections never produce an authenticated-peer
  observation.

**Commit:** `feat(p2p): expose authenticated connection observations`

---

### P05 — Switch Posemesh to the canonical SDK crates

**Goal:** End source divergence before `auki-domain` starts building on the
canonical implementation.

This is the first coordinated two-repository prompt.

**SDK work**

- Publish a temporary pinned release, or push a reviewable SDK revision that
  contains P01–P04.
- Record that exact version/revision in the Posemesh change.

**Posemesh work**

- Replace path/workspace ownership of `auki-p2p` and `auki-p2p-dataset` with
  exact pinned SDK dependencies.
- Update call sites for the role-neutral session API and identity changes.
- Remove `posemesh/core/auki-p2p` and
  `posemesh/core/auki-p2p-dataset` only after all consumers compile against the
  canonical crates.
- Keep the compute node's DMS relay booking coordinator. It selects/authorizes
  providers and constructs `RelayProvider`; the canonical `auki-p2p::Node`
  continues to perform the CRv2 reservation lifecycle.

**Required proof**

- Posemesh has no path or copied source dependency for either canonical crate.
- The authenticated transport, real relay, dataset, compute-node relay booking,
  and relay-file end-to-end suites pass.
- `rg` finds no second `pub struct Identity`, `pub struct Node`, or copied
  `RelayReservationNode` implementation in Posemesh.
- SDK crate tests still pass at the exact revision consumed by Posemesh.

**Commits**

- SDK release/revision commit if needed.
- Posemesh: `refactor(p2p): consume canonical auki-sdk crates`

---

### P06 — Freeze retained codecs and authenticated protocol IDs

**Goal:** Separate reusable application wire logic from the old central swarm
before replacing Domain runtime ownership.

**Primary files**

- `crates/auki-network/src/info_protocol.rs`
- `crates/auki-network/src/resources_protocol.rs`
- `crates/auki-network/src/resources_v3_protocol.rs`
- `crates/auki-network/src/resources_v4_protocol.rs`
- `crates/auki-network/src/registries_protocol.rs`
- `crates/auki-network/src/blobs_protocol.rs`
- `crates/auki-network/src/message_protocol.rs`
- `crates/auki-network/src/stream_protocol.rs`
- new shared protocol-ID and preservation-vector modules/tests

**Work**

- Define the exact D11 authenticated IDs once and reuse those constants from
  clients, servers, and tests.
- Extract or expose codecs, bounded request/response functions, and pure
  business validation without importing `NetworkRuntime`, `Swarm`, Discovery,
  membership, heartbeat, or a peer allow-list.
- Preserve resources `0.2/0.3/0.4`, registries `0.2/0.3`, blobs `0.1`, message
  `0.1`, and native stream `0.2` payload bytes and bounds.
- Define info `1.0.0` without Manager, membership, authority-role, or route
  fields. Keep useful participant/session/application and clock-descriptor
  metadata.
- Do not yet cut the public `Domain` over. This prompt creates clean adapters
  that the new internal runtime can call.

**Required proof**

- Existing fixed payload vectors remain byte-identical for every unchanged
  codec.
- Info has new explicit vectors.
- Every legacy protocol ID is absent from the new constants and fails protocol
  negotiation in a focused test.
- Pure codec tests compile without the `auki-network` `swarm` feature.

**Commit:** `refactor(protocols): isolate retained domain codecs`

---

### P07 — Build the private Domain-owned runtime

**Goal:** Implement the replacement engine behind `auki-domain` without
shipping a second public runtime.

**Primary files**

- `crates/auki-domain/src/domain.rs`
- new private modules such as `runtime.rs`, `authority.rs`, `routes.rs`,
  `peers.rs`, and `protocols.rs`
- `crates/auki-domain/src/lib.rs`
- `crates/auki-domain/Cargo.toml`

**Work**

- Add the D06 `DomainConfig`: one DDS Domain UUID, one canonical `Identity`,
  bounded listeners, and bounded explicit routes.
- Add the narrow cloneable authority handle for signed credential/key
  installation and safe challenge signing. It performs no HTTP.
- Make one joined Domain own exactly one internal `auki_p2p::Node`.
- Add the restricted `DomainProtocols` registration/open surface from D08.
- Implement a per-Domain route catalog keyed by expected Peer ID. Stage 1
  accepts canonical direct TCP and complete CRv2 routes; sources remain
  untrusted metadata.
- Build the D07 known-peer snapshot/events from P04 observations. Expire records
  at the verified deadline or final connection closure, with no grace cache.
- Implement `Ready`, `CredentialUnavailable`, `Failed`, and `Stopped` status
  plus a lag-recoverable subscription.
- Keep this engine private until the retained adapters are present. No
  executable or feature may start it beside `NetworkRuntime`.

**Required proof**

- Unit tests cover zero listener/zero route readiness, route validation and
  bounds, duplicate protocol registration, authority rotation/expiry,
  observation state, fatal child failure, and bounded cleanup.
- API review proves `DomainProtocols` cannot access token strings, raw keys,
  listener ownership, the swarm, or node shutdown.
- A test proves a route can establish transport but cannot expose application
  bytes with invalid Domain authority.

**Commit:** `feat(domain): add the authenticated domain runtime`

---

### P08 — Complete the first resource-catalog vertical slice

**Goal:** Satisfy D16 through the new internal runtime before migrating the
remaining protocols.

**Primary files**

- resource adapter modules from P06
- the new private Domain protocol host from P07
- `crates/auki-domain/tests/` focused authenticated resource tests

**Work**

- Register and open `/auki/auth/1/resources/0.2.0` through
  `DomainProtocols`.
- Reuse the existing provider, catalog, owner, size, and payload behavior.
- Bind outbound catalog requests to the expected authenticated Peer ID.
- Build a two-native-Domain test with explicit direct TCP routes and signed
  same-Domain credentials.
- Observe both peers through the D07 snapshot/events.
- Exercise explicit ordered leave and prove no node/protocol task remains.
- Keep the path private if other public Domain methods still depend on the old
  runtime; this is an implementation harness, not a dual shipping mode.

**Required proof**

- Bidirectional resource `0.2.0` catalog fetch succeeds.
- Wrong Peer ID, wrong Domain, expired token, anonymous stream, and legacy
  `/auki/resources/0.2.0` expose zero catalog bytes.
- Zero-route join remains locally ready.
- The test has a bounded timeout and leaves no task/listener leak.

**Commit:** `feat(domain): serve resource catalogs over authenticated p2p`

---

### P09 — Migrate info and all resource versions

**Goal:** Finish participant info and the complete resource family on the new
engine.

**Work**

- Add `/auki/auth/1/info/1.0.0` using the P06 adapted payload.
- Add resources `0.3.0` and `0.4.0` beside the proven `0.2.0` adapter.
- Preserve current provider selection, ownership, catalog merging, pagination,
  and bounds where those semantics already exist.
- Remove any info/resource dependency on Manager, membership, role authority,
  Discovery, or Domain time.

**Required proof**

- Positive and D09 negative authorization tables run for all four handlers.
- Preservation vectors pass for all resource payload versions.
- Peer observation metadata is refreshed from authenticated info only where it
  is bounded diagnostic metadata; it never changes authority or routes.

**Commit:** `feat(domain): migrate info and resource protocols`

---

### P10 — Migrate registry and blob operations

**Goal:** Move registry listing/fetching and content-addressed blob transfer
onto authenticated streams without changing their integrity rules.

**Work**

- Register and open registry `0.2.0` and `0.3.0` under their D11 IDs.
- Register and open blob `0.1.0` under its D11 ID.
- Preserve owner matching, registry kind checks, hash-pinned fetch, content
  hash verification, response-size limits, and cancellation cleanup.
- Move useful operations currently reachable only through `ClusterManager`
  onto plain internal Domain services in preparation for P12.

**Required proof**

- Existing preservation vectors and positive behavior pass.
- Wrong expected peer, wrong Domain, expired authority, wrong owner, bad hash,
  oversized response, and cancellation cases fail without partial data escape.
- No registry/blob code imports Manager, membership, known-peer authorization,
  or the old runtime.

**Commit:** `feat(domain): migrate registry and blob protocols`

---

### P11 — Migrate messaging and typed streams

**Goal:** Preserve the two stateful protocol families on one authenticated
Domain node.

**Work**

- Register/open message `0.1.0` and native stream `0.2.0` under D11 IDs.
- Preserve addressed receiver/expected producer checks, manifests, typed
  payload validation, ACK behavior, bounded queues, backpressure, frame/stream
  bounds, and cancellation ownership.
- Keep channel declarations/providers on `DomainBuilder`; do not turn
  `DomainProtocols` into a service locator.
- Remove browser stream `0.1.0` from the new native product surface.
- Do not recreate Manager broadcast, heartbeat, diagnostic, or browser session
  semantics using messaging.

**Required proof**

- Existing message and stream business tests pass through the new authenticated
  adapters.
- Positive and D09 negative authorization tables pass for both protocols.
- Slow consumer, full queue, dropped receiver, interrupted stream, oversized
  frame, wrong producer, and leave-during-I/O tests prove bounded cleanup.
- No application byte is sent before expected-peer and Domain authentication.

**Commit:** `feat(domain): migrate messages and typed streams`

---

### P12 — Cut the public `Domain` API over and delete Manager ownership

**Goal:** Make the new engine the only native runtime and remove the obsolete
product model honestly.

**Primary files**

- `crates/auki-domain/src/domain.rs`
- `crates/auki-domain/src/lib.rs`
- `crates/auki-domain/src/cluster_manager.rs` (remove)
- `crates/auki-domain/src/cluster_membership.rs` (remove)
- Manager-era `auki-network` runtime/control modules
- affected SDK tests and READMEs

**Work**

- Expose D06 `DomainBuilder`, `DomainConfig`, `Domain::join`, authority, routes,
  protocols, status, known peers, catalogs, registries, blobs, messages,
  streams, and ordered `leave` over the P07 runtime.
- Remove `Domain::cluster_manager()`, `ClusterManager`, `ClusterTarget`, cluster
  create/join/delete, membership/election/successor state, Manager relay
  control, heartbeat, synchronized Domain-time APIs, and diagnostic broadcast.
- Remove native runtime dependencies on Discovery HTTP, peer allow/block-list
  authorization, join/membership protocols, and `NetworkRuntime`.
- Retain `auki-network` only for codecs/plain types still used after P06. Do not
  move pure codecs solely for cosmetic crate boundaries.
- Before deleting runtime symbols needed only by the old Swift and Rust/WASM
  browser bindings, tag their final Manager-compatible package lines and remove
  those unsupported crates from the active Stage 1 workspace. Do not partly
  port them or preserve an old runtime feature just to keep current-main
  bindings compiling; P16 and P17 introduce their authenticated replacements.
- Remove all legacy SDK protocol handlers. Do not leave a feature flag that can
  start them.
- Make `leave(self)` bounded, ordered, and error-reporting; Drop remains only a
  best-effort backstop.

**Required proof**

- The complete native Domain test matrix passes on the new engine.
- The seven-step D16 scenario is rerun through the public `Domain` facade, not
  only the private P08 integration harness.
- `rg` finds no public Manager/membership/election/heartbeat/Domain-time API and
  no active import of old swarm, Discovery, or allow-list ownership from
  `auki-domain`.
- `rg` finds none of the removed protocol IDs in native production code.
- No executable constructs both `NetworkRuntime` and `auki_p2p::Node`.
- Existing useful Domain operations have migration examples or compile tests.

**Commit:** `refactor(domain)!: replace cluster manager with authenticated p2p`

---

### P13 — Replace the Python `ClusterManager` facade

**Goal:** Ship the Stage 1 binding over the same native Domain owner.

**Primary files**

- `bindings/python/auki-domain-py/**`
- Python examples/tests/docs
- any now-unused `auki-network-py` runtime bridge

**Work**

- Expose builder/join, authority installation, status, explicit routes, known
  peers, catalogs, registries, blobs, messages, streams, and leave with Python
  ownership matching Rust.
- Remove Python `ClusterManager`, cluster targets, membership/election,
  heartbeat/Domain-time, Manager relay controls, and Discovery-owned startup.
- Keep credential acquisition in the Python host/application layer. The binding
  accepts signed token/key material and never performs hidden DDS HTTP.
- Preserve cancellation and object-lifetime ownership across Python async task
  cancellation and garbage collection.

**Required proof**

- Python-to-Rust and Rust-to-Python resource `0.2.0` vertical tests pass.
- Negative auth vectors are shared with Rust.
- Python leave is bounded and no native task survives interpreter object
  cleanup.
- Removed Manager symbols are absent from the new package API and documented as
  breaking changes.

**Commit:** `refactor(python)!: expose authenticated domain lifecycle`

---

### P14 — Refactor the diagnostic app and publish Stage 1 guidance

**Goal:** Provide one understandable manual proof and an honest migration path
for early SDK consumers.

**Primary files**

- `examples/diagnostic-app/**`
- `crates/auki-domain/README.md`
- SDK root documentation and changelog/migration guide

**Work**

- Make the diagnostic app start one Domain with host-supplied identity,
  credentials, listeners, and explicit routes.
- Show local status, authenticated known peers, and a resource catalog fetch.
- Remove cluster create/join, leader, membership, heartbeat, synchronized time,
  diagnostic broadcast, and Manager relay UI.
- Document that explicit routes can later be supplied by configuration, an app,
  DDS/DMS adapters, or discovery without changing authentication.
- Document the relay split with one concrete Posemesh example:
  DMS/compute-node assigns a provider; `auki-p2p` reserves it; Domain publishes
  the confirmed route. The SDK performs no relay booking HTTP.
- Publish a breaking migration table for Rust and Python.

**Required proof**

- Two local app instances complete the D16 resource exchange over direct TCP.
- Invalid/wrong-Domain credentials show no resource data.
- The Stage 1 workspace examples compile from documented commands.

**Commit:** `docs(domain): document authenticated p2p migration`

---

### P15 — Stage 1 release gate

This is a release checklist, not an architecture prompt.

- [ ] The active Stage 1 SDK workspace format, strict Clippy, unit, integration,
      doctest, and Python suites pass. Old Swift/Rust-WASM packages are tested
      at their pinned prior tags, not against the new native runtime.
- [ ] Posemesh exact-revision tests and the relay-file dev end-to-end pass.
- [ ] One cross-repository fixed vector proves the DDS Domain UUID, Peer ID,
      signed claims, mutual-auth framing, and resource `0.2.0` payload.
- [ ] Every retained D09 handler has positive, anonymous, wrong-Peer-ID,
      wrong-Domain, expired-authority, and legacy-ID coverage.
- [ ] Shutdown/task-leak tests pass under cancellation and runtime failure.
- [ ] No core P2P/Domain/protocol crate performs DDS or DMS HTTP.
- [ ] No Manager compatibility feature or unauthenticated fallback ships.
- [ ] Crate/package versions and MSRV are pinned and documented.
- [ ] Prior Swift and browser package lines/tags remain retrievable before the
      native/Python breaking release is published.

## 5. Later platform stages

These prompts do not block P15. They inherit the same D01–D13 wire and product
rules; they do not reopen them.

### P16 — Bind the native Domain owner for Swift/iOS

- Replace the old Swift `NetworkRuntime` binding with the same native Rust
  `Domain` owner used in Stage 1.
- Use host-owned Keychain/file persistence for canonical identity material and
  host-owned DDS credential acquisition.
- Expose Domain lifecycle and narrow protocol APIs, not a raw node or swarm.
- Share Rust authentication/protocol vectors and run native-to-Swift resource,
  message, and stream tests.
- Remove Manager, Discovery-owned startup, membership, heartbeat, and old
  liveness APIs from the new Swift package line.

### P17 — Replace the browser experiment with one TypeScript engine

- Keep `auki-domain-browser` as the browser product facade.
- Use one js-libp2p engine with WebSocket plus stock circuit relay for the first
  route slice.
- Implement the exact D02 token/Peer-ID/Domain verification and D11 IDs.
- Retire the Rust/WASM browser Domain runtime and browser Manager/session/probe
  product protocols after preserving reusable vectors.
- Prove native-to-browser authenticated resources and stream `0.2.0`; do not
  revive browser stream `0.1.0`.

## 6. Explicitly deferred work

The following items require separate product decisions and must not expand a
prompt above:

- automatic discovery or route exchange;
- a general SDK relay-provider booking service;
- browser WebRTC Direct and native QUIC/WebSocket listener parity;
- synchronized Domain time;
- concurrent multiple Domains sharing one process/runtime;
- a generic application authorization framework beyond D02;
- a generic plugin/dependency-injection system; and
- extraction of every retained codec into a new crate.

## 7. Final deletion audit

Before declaring the whole migration complete, production code and public docs
must contain no active concept equivalent to:

- `ClusterManager`, Manager election, successor handoff, or cluster admission;
- authoritative membership or `known_peers` as an allow-list;
- Manager heartbeat or synchronized Domain time;
- Manager-created relay ownership or SDK-embedded relay booking;
- unauthenticated legacy protocol fallback;
- two swarms/Peer IDs for one Domain; or
- DDS/DMS HTTP inside the P2P, Domain, or protocol cores.

Names may differ, so this is a semantic audit, not only an `rg` checklist.
