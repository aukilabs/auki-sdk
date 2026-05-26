# Full Network and Domain Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fully expose `auki-network` and `auki-domain` through the SDK binding standard so generated Python, Swift, and JavaScript/WebAssembly consumers can use the operational network and domain surfaces without falling back to raw Rust or the legacy PyO3 wrappers.

**Architecture:** Native Python and Swift use UniFFI facades over binding-safe Rust runtime objects. Browser JavaScript uses wasm-bindgen for pure protocol helpers and JavaScript-owned libp2p runtime facades where Rust `NetworkRuntime` cannot run in wasm. Binding APIs expose stable strings, JSON records, byte payloads, opaque runtime handles, callback interfaces, polling queues, and responder tokens rather than raw `libp2p`, Tokio, generic stream, or trait-object internals.

**Tech Stack:** Rust 2024, UniFFI 0.31, wasm-bindgen, wasm-pack, Tokio, libp2p, prost/generated `auki-proto` packages, SwiftPM/XCFramework generation, Python UniFFI package generation, JavaScript ESM with js-libp2p browser transport.

---

## Scope

This plan covers only `auki-network` and `auki-domain`.

`auki-ros-adapter` is intentionally excluded from this pass. Its ROS2 host constraints need a separate adapter-specific binding decision.

The existing PyO3 packages under `bindings/python/auki-*-py` are legacy compatibility surfaces. Do not expand them and do not use them as the target API shape.

## Definition of Full Exposure

Full exposure means every supported operation in the public Rust runtime surface has a binding-supported equivalent, or an explicit binding-safe replacement when the Rust type cannot cross the FFI boundary.

It does not mean a one-to-one export of raw Rust APIs. These Rust shapes are not binding contracts:

- `libp2p::PeerId`, `Multiaddr`, `Swarm`, `Stream`, and protocol handler internals.
- Generic stream APIs such as `open_stream<T>`.
- Trait objects and closures that cannot be modeled as stable UniFFI callback interfaces.
- Tokio channels, oneshots, or receivers.
- Rust-only error types that expose implementation-specific dependencies.

Bindings must use these boundary shapes:

- `String` for peer IDs, multiaddrs, protocol IDs, content IDs, and clock IDs.
- JSON strings for large protocol request/response DTOs whose schema is already protobuf-backed or registry-backed.
- `Vec<u8>` / `bytes` for protobuf payloads and stream frames.
- Opaque UniFFI objects for runtime handles, subscriptions, and provider registrations.
- Callback interfaces only where the host language owns dynamic provider behavior.
- Polling methods plus responder tokens for inbound async events that require host-language decisions.

## Current Surface Gap

`auki-network` is only partially bound today.

Native UniFFI currently exposes identity helpers, capability strings, peer derivation, and the message-node object behind the `message_node` feature. Browser wasm currently exposes identity/protocol helpers and browser-probe encode/decode helpers with a JavaScript wrapper for `AukiNetworkPeer` and `dialBrowserProbe`.

The native binding surface does not yet expose:

- Full `NetworkRuntime` lifecycle.
- `NetworkRuntimeHandle` control methods.
- `SwarmConfig` construction as a binding-safe builder or record.
- Allowed-peer updates.
- Heartbeat target updates.
- Join request/response flows.
- Participant info request/response flows.
- Sensor catalog, resource catalog, and registry request/response flows.
- Membership, liveness, diagnostic, and request event draining.
- Discovery client methods.
- App-instance derivation.
- Stream provider installation.
- Byte-oriented stream opening and stream entry consumption.

`auki-domain` is also only partially bound today.

Native UniFFI currently exposes a bounded `DomainClusterManager` with bootstrap, cluster identity facts, peer counts, membership JSON, participant-info JSON, domain time, participant-info fetch, and shutdown. Browser wasm currently exposes membership/election JSON helpers only.

The domain binding surface does not yet expose:

- Diagnostic broadcast and drain.
- Manager-side peer admission.
- Clock sync estimate inspection.
- Domain clock estimate inspection.
- Dynamic sensor and resource catalog providers.
- Registry app-root serving and registry entry fetches.
- Sensor catalog and resource catalog fetches.
- Stream opening and stream provider installation.
- Full event/error observability needed by generated language consumers.

## Binding Contract Strategy

The plan creates a small set of binding contracts that both crates can share:

- Binding DTOs live in crate-local `src/ffi.rs` modules unless they are reused across crates often enough to justify a later `auki-bindings-core` crate.
- Rust conversion helpers use `TryFrom` / `From` and are covered by Rust tests before generated-language tests are added.
- JSON payloads are canonical enough for tests: fields are stable, peer IDs are strings, byte fields use base64 only when JSON needs to carry raw bytes, and error responses include `code` and `message`.
- Stream payloads are protobuf bytes produced or consumed by the generated per-language `auki-proto` packages.
- Inbound events that require a host-language response return a numeric `responder_id`; the opaque runtime object stores the Rust responder until the host calls a matching `respond_*` method or the event times out.

## Phase 1: Make Coverage Measurable

- [x] Add `crates/auki-network/bindings/surface.md` listing the required native and browser binding operations.

  Include these sections:

  ```markdown
  # auki-network Binding Surface

  ## Native UniFFI Required

  - Peer identity and derivation.
  - Runtime lifecycle.
  - Runtime control.
  - Event draining.
  - Request/response protocols.
  - Discovery client.
  - App-instance derivation.
  - Byte streams.
  - Diagnostics.

  ## Browser JavaScript Required

  - Peer identity and derivation.
  - Protocol constants.
  - Browser probe.
  - Message protocol.
  - Request/response DTO encoding helpers.
  - JavaScript-owned libp2p peer facade.
  ```

- [x] Add `crates/auki-domain/bindings/surface.md` listing the required native and browser binding operations.

  Include these sections:

  ```markdown
  # auki-domain Binding Surface

  ## Native UniFFI Required

  - Cluster lifecycle.
  - Manager admission.
  - Membership inspection.
  - Participant info.
  - Domain time and clock estimates.
  - Diagnostics.
  - Catalog and registry providers.
  - Catalog and registry fetches.
  - Byte streams.

  ## Browser JavaScript Required

  - Membership validation helpers.
  - Manager election helpers.
  - Domain DTO validation helpers.
  - JavaScript domain client facade over `auki-network` browser transport.
  ```

- [x] Add a binding surface verification script at `scripts/bindings/check-full-surface.py`.

  The script reads both `bindings/surface.md` files and asserts that each bullet has a matching Rust test marker in the crate tests. Use markers in this form:

  ```rust
  // binding-surface: native runtime lifecycle
  ```

  Expected command:

  ```bash
  python3 scripts/bindings/check-full-surface.py
  ```

  Expected output:

  ```text
  full binding surface markers present for auki-network
  full binding surface markers present for auki-domain
  ```

- [x] Add `crates/auki-network/tests/full_binding_surface.rs` and `crates/auki-domain/tests/full_binding_surface.rs` with one ignored smoke marker test per required surface item.

  Each test starts as `#[ignore = "implemented in later phase"]` and becomes active when the phase implementing that surface lands.

  The first marker test in `crates/auki-network/tests/full_binding_surface.rs`:

  ```rust
  #[test]
  #[ignore = "implemented in later phase"]
  fn native_runtime_lifecycle_is_exposed() {
      // binding-surface: native runtime lifecycle
  }
  ```

  The first marker test in `crates/auki-domain/tests/full_binding_surface.rs`:

  ```rust
  #[test]
  #[ignore = "implemented in later phase"]
  fn native_cluster_lifecycle_is_exposed() {
      // binding-surface: native cluster lifecycle
  }
  ```

## Phase 2: Expose `auki-network` Native Runtime Control

- [x] Extend `crates/auki-network/src/ffi.rs` with binding-safe runtime configuration records.

  Add these UniFFI records:

  ```rust
  #[derive(Debug, Clone, uniffi::Record)]
  pub struct BindingAllowedPeer {
      pub peer_id: String,
      pub multiaddrs: Vec<String>,
  }

  #[derive(Debug, Clone, uniffi::Record)]
  pub struct BindingSwarmConfig {
      pub wallet_seed: Vec<u8>,
      pub listen_multiaddrs: Vec<String>,
      pub agent_version: String,
      pub allowed_peers: Vec<BindingAllowedPeer>,
      pub heartbeat_clock_id: Option<String>,
      pub heartbeat_clock_hash_hex: Option<String>,
  }

  #[derive(Debug, Clone, uniffi::Record)]
  pub struct BindingUpdateReport {
      pub accepted: Vec<String>,
      pub rejected_json: String,
  }
  ```

- [x] Add typed binding errors in `crates/auki-network/src/ffi.rs`.

  Use one UniFFI error enum for runtime construction and API calls:

  ```rust
  #[derive(Debug, thiserror::Error, uniffi::Error)]
  pub enum BindingNetworkError {
      #[error("invalid peer id: {message}")]
      InvalidPeerId { message: String },
      #[error("invalid multiaddr: {message}")]
      InvalidMultiaddr { message: String },
      #[error("runtime error: {message}")]
      Runtime { message: String },
      #[error("timeout waiting for response")]
      Timeout,
      #[error("closed")]
      Closed,
      #[error("unsupported on this target: {message}")]
      Unsupported { message: String },
  }
  ```

- [x] Add an opaque UniFFI object `AukiNetworkRuntime` in `crates/auki-network/src/ffi.rs`.

  Required methods:

  ```rust
  #[uniffi::export]
  impl AukiNetworkRuntime {
      pub fn spawn(config: BindingSwarmConfig) -> Result<Arc<Self>, BindingNetworkError>;
      pub fn local_peer_id(&self) -> String;
      pub fn listen_multiaddrs(&self) -> Vec<String>;
      pub fn connected_peers(&self) -> Vec<String>;
      pub fn set_allowed_peers(&self, peers: Vec<BindingAllowedPeer>) -> Result<BindingUpdateReport, BindingNetworkError>;
      pub fn set_heartbeat_targets(&self, peer_ids: Vec<String>) -> Result<(), BindingNetworkError>;
      pub fn shutdown(&self) -> Result<(), BindingNetworkError>;
  }
  ```

- [x] Add active tests in `crates/auki-network/tests/full_binding_surface.rs` for runtime lifecycle and control.

  Required tests:

  - `native_runtime_lifecycle_is_exposed`
  - `native_runtime_control_is_exposed`
  - `native_allowed_peer_updates_are_exposed`
  - `native_heartbeat_targets_are_exposed`

  Expected command:

  ```bash
  cargo test -p auki-network --features message_node,swarm native_runtime_
  ```

  Expected result: all matching tests pass without needing the legacy PyO3 packages.

## Phase 3: Expose `auki-network` Events and Request/Response Protocols

- [x] Add binding event records to `crates/auki-network/src/ffi.rs`.

  Required event records:

  ```rust
  #[derive(Debug, Clone, uniffi::Record)]
  pub struct BindingRuntimeEvent {
      pub kind: String,
      pub peer_id: Option<String>,
      pub payload_json: String,
      pub responder_id: Option<u64>,
  }

  #[derive(Debug, Clone, uniffi::Record)]
  pub struct BindingProtocolResponse {
      pub peer_id: String,
      pub payload_json: String,
  }
  ```

- [x] Add responder registries inside `AukiNetworkRuntime`.

  Store pending oneshot responders behind `Mutex<HashMap<u64, PendingResponder>>`. Every inbound event that needs a host-language decision returns a `responder_id`. A response method removes the responder and sends the Rust response exactly once. A second response returns `BindingNetworkError::Closed`.

- [x] Add event draining methods.

  Required methods:

  ```rust
  pub fn drain_runtime_events(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_membership_events(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_liveness_events(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_diagnostic_events(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_join_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_participant_info_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_sensor_catalog_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_resource_catalog_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn drain_registry_entry_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  ```

- [x] Add outbound request methods.

  Required methods:

  ```rust
  pub fn send_join_request_json(&self, peer_id: String, request_json: String, timeout_ms: u64) -> Result<BindingProtocolResponse, BindingNetworkError>;
  pub fn request_participant_info_json(&self, peer_id: String, request_json: String, timeout_ms: u64) -> Result<BindingProtocolResponse, BindingNetworkError>;
  pub fn request_sensor_catalog_json(&self, peer_id: String, request_json: String, timeout_ms: u64) -> Result<BindingProtocolResponse, BindingNetworkError>;
  pub fn request_resource_catalog_json(&self, peer_id: String, request_json: String, timeout_ms: u64) -> Result<BindingProtocolResponse, BindingNetworkError>;
  pub fn request_registry_entry_json(&self, peer_id: String, request_json: String, timeout_ms: u64) -> Result<BindingProtocolResponse, BindingNetworkError>;
  ```

- [x] Add inbound response methods.

  Required methods:

  ```rust
  pub fn respond_join_json(&self, responder_id: u64, response_json: String) -> Result<(), BindingNetworkError>;
  pub fn respond_participant_info_json(&self, responder_id: u64, response_json: String) -> Result<(), BindingNetworkError>;
  pub fn respond_sensor_catalog_json(&self, responder_id: u64, response_json: String) -> Result<(), BindingNetworkError>;
  pub fn respond_resource_catalog_json(&self, responder_id: u64, response_json: String) -> Result<(), BindingNetworkError>;
  pub fn respond_registry_entry_json(&self, responder_id: u64, response_json: String) -> Result<(), BindingNetworkError>;
  ```

- [x] Add two-runtime Rust tests that use only binding facades.

  Required tests in `crates/auki-network/tests/full_binding_surface.rs`:

  - `native_join_request_response_is_exposed`
  - `native_participant_info_request_response_is_exposed`
  - `native_catalog_request_response_is_exposed`
  - `native_registry_request_response_is_exposed`
  - `native_diagnostics_are_exposed`

  Expected command:

  ```bash
  cargo test -p auki-network --features message_node,swarm full_binding_surface -- --ignored
  ```

  As each ignored test is implemented, remove its ignore attribute and keep the command passing.

## Phase 4: Expose `auki-network` Streams as Bytes

- [x] Add stream binding records to `crates/auki-network/src/ffi.rs`.

  Required records:

  ```rust
  #[derive(Debug, Clone, uniffi::Record)]
  pub struct BindingStreamRequest {
      pub peer_id: String,
      pub request_json: String,
      pub payload_kind: String,
      pub timeout_ms: u64,
  }

  #[derive(Debug, Clone, uniffi::Record)]
  pub struct BindingStreamEntry {
      pub sequence: u64,
      pub timestamp_ns: u64,
      pub payload_kind: String,
      pub payload: Vec<u8>,
  }
  ```

- [x] Add opaque UniFFI object `AukiStreamSubscription`.

  Required methods:

  ```rust
  #[uniffi::export]
  impl AukiStreamSubscription {
      pub fn manifest_json(&self) -> String;
      pub fn next_entry(&self, timeout_ms: u64) -> Result<Option<BindingStreamEntry>, BindingNetworkError>;
      pub fn close(&self) -> Result<(), BindingNetworkError>;
  }
  ```

- [x] Add host-driven stream provider events.

  Required runtime methods:

  ```rust
  pub fn drain_stream_open_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent>;
  pub fn accept_stream_open(&self, responder_id: u64, manifest_json: String) -> Result<u64, BindingNetworkError>;
  pub fn decline_stream_open(&self, responder_id: u64, reason: String) -> Result<(), BindingNetworkError>;
  pub fn push_stream_entry(&self, stream_id: u64, entry: BindingStreamEntry) -> Result<(), BindingNetworkError>;
  pub fn finish_stream(&self, stream_id: u64) -> Result<(), BindingNetworkError>;
  pub fn open_stream_bytes(&self, request: BindingStreamRequest) -> Result<Arc<AukiStreamSubscription>, BindingNetworkError>;
  ```

- [x] Add stream tests that exchange protobuf bytes.

  Required tests:

  - `native_camera_stream_bytes_are_exposed`
  - `native_detection_stream_bytes_are_exposed`
  - `native_stream_decline_is_exposed`

  Test payloads must be generated with the Rust `auki-proto` types and consumed as raw bytes through the binding facade. Do not create parallel hand-written binding payload structs.

## Phase 5: Expose `auki-network` Discovery, App Instance, and Browser JS Runtime

- [x] Add UniFFI discovery client object in `crates/auki-network/src/ffi.rs`.

  Required object and methods:

  ```rust
  pub struct AukiDiscoveryClient;

  pub fn discovery_client(base_url: String) -> Result<Arc<AukiDiscoveryClient>, BindingNetworkError>;
  pub fn register_peer_json(&self, registration_json: String, timeout_ms: u64) -> Result<String, BindingNetworkError>;
  pub fn discover_peers_json(&self, query_json: String, timeout_ms: u64) -> Result<String, BindingNetworkError>;
  pub fn unregister_peer_json(&self, peer_id: String, timeout_ms: u64) -> Result<(), BindingNetworkError>;
  ```

- [x] Add UniFFI app-instance derivation helpers.

  Required functions:

  ```rust
  pub fn derive_app_instance_json(wallet_seed: Vec<u8>, app_id: String) -> Result<String, BindingNetworkError>;
  pub fn app_instance_peer_id(app_instance_json: String) -> Result<String, BindingNetworkError>;
  ```

- [x] Expand `crates/auki-network/src/wasm.rs` with pure browser-safe protocol helpers.

  Required wasm exports:

  - `auki_network_protocols_json()`
  - `encode_message_envelope_bytes(json: &str) -> Result<Vec<u8>, JsValue>`
  - `decode_message_envelope_json(bytes: &[u8]) -> Result<String, JsValue>`
  - `encode_join_request_bytes(json: &str) -> Result<Vec<u8>, JsValue>`
  - `decode_join_response_json(bytes: &[u8]) -> Result<String, JsValue>`
  - `encode_catalog_request_bytes(json: &str) -> Result<Vec<u8>, JsValue>`
  - `decode_catalog_response_json(bytes: &[u8]) -> Result<String, JsValue>`

- [x] Expand `crates/auki-network/bindings/javascript/index.js` with JavaScript-owned libp2p runtime methods.

  Required exports:

  ```javascript
  export class AukiNetworkPeer {
    static async create(options) {}
    get peerId() {}
    get multiaddrs() {}
    async stop() {}
    async dialBrowserProbe(peerMultiaddr) {}
    async sendMessageEnvelope(peerMultiaddr, envelopeBytes) {}
    async requestJoin(peerMultiaddr, requestBytes) {}
    async requestCatalog(peerMultiaddr, requestBytes) {}
  }
  ```

  This JavaScript class may use js-libp2p directly. Do not attempt to run Rust `NetworkRuntime` inside browser wasm.

- [x] Add JavaScript smoke tests under `crates/auki-network/bindings/javascript/test/`.

  Required tests:

  - `protocol-helpers.test.mjs`
  - `browser-message-smoke.test.mjs`
  - `browser-request-response-smoke.test.mjs`

  Expected command:

  ```bash
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test
  ```

## Phase 6: Expose `auki-domain` Native Cluster Control

- [x] Extend `crates/auki-domain/src/ffi.rs` with a full native domain error enum.

  Required enum:

  ```rust
  #[derive(Debug, thiserror::Error, uniffi::Error)]
  pub enum BindingDomainError {
      #[error("network error: {message}")]
      Network { message: String },
      #[error("domain error: {message}")]
      Domain { message: String },
      #[error("invalid peer id: {message}")]
      InvalidPeerId { message: String },
      #[error("invalid json: {message}")]
      InvalidJson { message: String },
      #[error("timeout waiting for response")]
      Timeout,
      #[error("closed")]
      Closed,
      #[error("unsupported on this target: {message}")]
      Unsupported { message: String },
  }
  ```

- [x] Extend the existing `DomainClusterManager` UniFFI object with manager and diagnostics methods.

  Required methods:

  ```rust
  pub fn admit_peer(&self, peer_id: String, multiaddrs: Vec<String>) -> Result<String, BindingDomainError>;
  pub fn broadcast_diagnostic_message_json(&self, message_json: String) -> Result<(), BindingDomainError>;
  pub fn drain_diagnostic_messages_json(&self, max_events: u32) -> Vec<String>;
  pub fn drain_membership_events_json(&self, max_events: u32) -> Vec<String>;
  ```

- [x] Add clock inspection methods.

  Required methods:

  ```rust
  pub fn clock_sync_estimate_json(&self, peer_id: String) -> Result<String, BindingDomainError>;
  pub fn clock_sync_estimates_json(&self) -> Result<String, BindingDomainError>;
  pub fn domain_clock_estimate_json(&self) -> Result<String, BindingDomainError>;
  ```

- [x] Add active tests in `crates/auki-domain/tests/full_binding_surface.rs`.

  Required tests:

  - `native_manager_admission_is_exposed`
  - `native_diagnostics_are_exposed`
  - `native_clock_estimates_are_exposed`

  Expected command:

  ```bash
  cargo test -p auki-domain --test full_binding_surface -- --test-threads=1
  ```

## Phase 7: Expose `auki-domain` Providers, Registry, Catalogs, and Streams

- [x] Add UniFFI provider callback interfaces in `crates/auki-domain/src/ffi.rs`.

  Required interfaces:

  ```rust
  #[uniffi::export(with_foreign)]
  pub trait BindingSensorCatalogProvider: Send + Sync {
      fn snapshot_json(&self) -> Result<String, BindingDomainError>;
  }

  #[uniffi::export(with_foreign)]
  pub trait BindingResourceCatalogProvider: Send + Sync {
      fn snapshot_json(&self) -> Result<String, BindingDomainError>;
  }

  #[uniffi::export(with_foreign)]
  pub trait BindingRegistryEntryProvider: Send + Sync {
      fn entry_json(&self, path: String) -> Result<Option<String>, BindingDomainError>;
  }
  ```

- [x] Add provider installation and static convenience methods to `DomainClusterManager`.

  Required methods:

  ```rust
  pub fn set_sensor_catalog_provider(&self, provider: Arc<dyn BindingSensorCatalogProvider>) -> Result<(), BindingDomainError>;
  pub fn set_resource_catalog_provider(&self, provider: Arc<dyn BindingResourceCatalogProvider>) -> Result<(), BindingDomainError>;
  pub fn set_registry_entry_provider(&self, provider: Arc<dyn BindingRegistryEntryProvider>) -> Result<(), BindingDomainError>;
  pub fn set_static_sensor_catalog_json(&self, catalog_json: String) -> Result<(), BindingDomainError>;
  pub fn set_static_resource_catalog_json(&self, catalog_json: String) -> Result<(), BindingDomainError>;
  pub fn set_static_registry_entries_json(&self, entries_json: String) -> Result<(), BindingDomainError>;
  ```

- [x] Add fetch methods to `DomainClusterManager`.

  Required methods:

  ```rust
  pub async fn fetch_sensor_catalog_json(&self, peer_id: String, timeout_ms: u64) -> Result<String, BindingDomainError>;
  pub async fn fetch_resource_catalog_json(&self, peer_id: String, timeout_ms: u64) -> Result<String, BindingDomainError>;
  pub async fn fetch_registry_entry_json(&self, peer_id: String, path: String, timeout_ms: u64) -> Result<String, BindingDomainError>;
  ```

- [x] Add stream methods that delegate to the `auki-network` byte stream facade.

  Required methods:

  ```rust
  pub fn drain_stream_open_requests(&self, max_events: u32) -> Vec<DomainRuntimeEvent>;
  pub fn accept_stream_open(&self, responder_id: u64, manifest_json: String) -> Result<u64, BindingDomainError>;
  pub fn decline_stream_open(&self, responder_id: u64, reason: String) -> Result<(), BindingDomainError>;
  pub fn push_stream_entry(&self, stream_id: u64, entry: DomainStreamEntry) -> Result<(), BindingDomainError>;
  pub fn finish_stream(&self, stream_id: u64) -> Result<(), BindingDomainError>;
  pub async fn open_stream_bytes(&self, peer_id: String, request_json: String, payload_kind: String, timeout_ms: u64) -> Result<Arc<DomainStreamSubscription>, BindingDomainError>;
  ```

  The implementation uses a domain-local opaque wrapper `DomainStreamSubscription` to avoid cross-crate UniFFI object coupling.

- [x] Add provider/fetch/stream tests in `crates/auki-domain/tests/full_binding_surface.rs`.

  Required marker tests:

  - `native_catalog_and_registry_providers_are_exposed`
  - `native_catalog_and_registry_fetches_are_exposed`
  - `native_byte_streams_are_exposed`

## Phase 8: Expose `auki-domain` Browser JavaScript Facade

- [x] Extend `crates/auki-domain/src/wasm.rs` with browser-safe pure helpers.

  Required wasm exports:

  - `validate_membership_json(json: &str) -> Result<String, JsValue>`
  - `validate_participant_info_json(json: &str) -> Result<String, JsValue>`
  - `validate_sensor_catalog_json(json: &str) -> Result<String, JsValue>`
  - `validate_resource_catalog_json(json: &str) -> Result<String, JsValue>`
  - `validate_registry_entry_json(json: &str) -> Result<String, JsValue>`
  - `domain_successor_json(membership_json: &str, peer_id: &str) -> Result<String, JsValue>`

- [x] Add `crates/auki-domain/bindings/javascript/index.js.tmpl`.

  Required class:

  ```javascript
  export class AukiDomainClient {
    constructor({ networkPeer, clusterName }) {}
    get clusterName() {}
    get localPeerId() {}
    validateMembership(membershipJson) {}
    successor(membershipJson, peerId) {}
    async requestJoin(peerMultiaddr, requestJson) {}
    async fetchParticipantInfo(peerMultiaddr, requestJson) {}
    async fetchSensorCatalog(peerMultiaddr, requestJson) {}
    async fetchResourceCatalog(peerMultiaddr, requestJson) {}
    async fetchRegistryEntry(peerMultiaddr, requestJson) {}
  }
  ```

  This client composes `AukiNetworkPeer` from `auki-network`, or any compatible peer object that exposes `requestFramed(peerMultiaddr, protocol, payload)`. It does not instantiate Rust `DomainClusterManager` in browser wasm.

- [x] Add JavaScript tests under `crates/auki-domain/bindings/javascript/test/`.

  Required tests:

  - `domain-helpers.test.mjs`
  - `domain-client-request-response.test.mjs`

  Expected command:

  ```bash
  just generate-javascript-bindings auki-domain
  npm --prefix bindings/javascript/auki-domain test
  ```

## Phase 9: Generated Language Smoke Tests

- [x] Add generated Python smoke scripts that use UniFFI packages only.

  Required files:

  - `crates/auki-network/bindings/python/smoke_full_network.py`
  - `crates/auki-domain/bindings/python/smoke_full_domain.py`

  Required coverage:

  - Same wallet seed derives the same peer ID as Rust.
  - `AukiNetworkRuntime.spawn` starts and shuts down.
  - Two native runtimes complete a join request/response.
  - Domain manager exposes membership JSON.
  - Domain manager serves and fetches participant info.
  - Domain manager serves and fetches sensor and resource catalogs.

  Expected command:

  ```bash
  AUKI_PYTHON_NATIVE_TARGETS="$(rustc -vV | awk '/host:/ {print $2}')" just generate-python-bindings auki-network
  AUKI_PYTHON_NATIVE_TARGETS="$(rustc -vV | awk '/host:/ {print $2}')" just generate-python-bindings auki-domain
  python3 crates/auki-network/bindings/python/smoke_full_network.py
  python3 crates/auki-domain/bindings/python/smoke_full_domain.py
  ```

- [x] Add Swift smoke targets that use generated Swift packages only.

  Required files:

  - `crates/auki-network/bindings/swift/SmokeFullNetwork/Package.swift`
  - `crates/auki-network/bindings/swift/SmokeFullNetwork/Sources/SmokeFullNetwork/main.swift`
  - `crates/auki-domain/bindings/swift/SmokeFullDomain/Package.swift`
  - `crates/auki-domain/bindings/swift/SmokeFullDomain/Sources/SmokeFullDomain/main.swift`

  Required coverage:

  - Import generated `AukiNetwork` and `AukiDomain` modules.
  - Start and shut down the native network runtime.
  - Build a domain cluster manager.
  - Assert membership JSON and local peer ID are non-empty.

  Expected command:

  ```bash
  just generate-swift-bindings auki-network
  just generate-swift-bindings auki-domain
  swift run --package-path crates/auki-network/bindings/swift/SmokeFullNetwork SmokeFullNetwork
  swift run --package-path crates/auki-domain/bindings/swift/SmokeFullDomain SmokeFullDomain
  ```

- [x] Add JavaScript smoke scripts for generated wasm packages.

  Required files:

  - `crates/auki-network/bindings/javascript/test/full-network-generated.test.mjs.tmpl`
  - `crates/auki-domain/bindings/javascript/test/full-domain-generated.test.mjs.tmpl`

  Required coverage:

  - Import generated wasm package.
  - Validate protocol constants.
  - Encode/decode message and join DTO bytes.
  - Validate domain membership and catalog JSON.
  - Compose `AukiDomainClient` with `AukiNetworkPeer` in a request/response smoke.

## Phase 10: Documentation and Changelog Propagation

- [x] Update `crates/auki-network/README.md` and `crates/auki-network/src/readme.md`.

  Required content:

  - Native Python/Swift users get the operational runtime through UniFFI.
  - Browser JavaScript users get protocol helpers plus the JavaScript-owned libp2p peer facade.
  - Streams use generated `auki-proto` bytes.
  - Legacy PyO3 wrappers are not the supported path for new bindings.

- [x] Update `crates/auki-domain/README.md` and `crates/auki-domain/src/readme.md`.

  Required content:

  - Native Python/Swift users get `DomainClusterManager` operational APIs through UniFFI.
  - Browser JavaScript users get pure helpers plus a JavaScript `AukiDomainClient` over `AukiNetworkPeer`.
  - Catalog, registry, participant-info, diagnostic, time, and stream behavior are exposed.

- [x] Update sprint files after each implementation phase.

  Required files:

  - `crates/auki-network/src/sprint.md`
  - `crates/auki-domain/src/sprint.md`

  Each completed phase should remove finished work from open tasks and add the next binding surface item still open.

- [x] Propagate changelog entries after each implementation phase.

  Required files for `auki-network` changes:

  - `crates/auki-network/changelog.md`
  - `crates/changelog.md`
  - `changelog.md`

  Required files for `auki-domain` changes:

  - `crates/auki-domain/changelog.md`
  - `crates/changelog.md`
  - `changelog.md`

  Required files for docs-only plan updates:

  - `docs/superpowers/plans/changelog.md`
  - `docs/superpowers/changelog.md`
  - `docs/changelog.md`
  - `changelog.md`

## Verification Gate

Run these commands before marking the full exposure complete:

```bash
python3 scripts/bindings/check-full-surface.py
cargo test -p auki-network --features message_node,swarm full_binding_surface
cargo test -p auki-domain full_binding_surface
cargo test -p auki-network --features browser_probe browser_probe
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p auki-domain --target wasm32-unknown-unknown --no-default-features --features wasm
AUKI_PYTHON_NATIVE_TARGETS="$(rustc -vV | awk '/host:/ {print $2}')" just generate-python-bindings auki-network
AUKI_PYTHON_NATIVE_TARGETS="$(rustc -vV | awk '/host:/ {print $2}')" just generate-python-bindings auki-domain
python3 crates/auki-network/bindings/python/smoke_full_network.py
python3 crates/auki-domain/bindings/python/smoke_full_domain.py
just generate-javascript-bindings auki-network
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-network test
npm --prefix bindings/javascript/auki-domain test
just generate-swift-bindings auki-network
just generate-swift-bindings auki-domain
swift run --package-path crates/auki-network/bindings/swift/SmokeFullNetwork SmokeFullNetwork
swift run --package-path crates/auki-domain/bindings/swift/SmokeFullDomain SmokeFullDomain
```

The final review should also confirm:

- No new public binding path depends on the legacy PyO3 wrappers.
- `auki-ros-adapter` remains out of this pass.
- Browser wasm does not claim to run the native Rust network/domain runtime.
- Native Python and Swift can perform runtime, domain, request/response, and stream operations through generated bindings.
- JavaScript can perform browser-supported protocol and domain flows through generated wasm helpers plus JavaScript-owned libp2p facades.
