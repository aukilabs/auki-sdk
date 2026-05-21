//! UniFFI Swift bindings for `auki-network`.
//!
//! ## Scope (v0 — post PR B)
//!
//! Full v0 networking surface for native iOS / Swift consumers:
//!
//! - **Discovery HTTP client** (`DiscoveryClient`, `ClusterEntry`,
//!   `CreateClusterOutcome`, `DiscoveryError`) — re-exported from the
//!   upstream-annotated `auki_network_rs::discovery_client`.
//! - **`NetworkRuntime`** + the [`spawn_for_swift`] orchestrator that
//!   builds the libp2p swarm and wires it to Swift callback interfaces.
//! - **Peer-liveness observation** via [`PeerLivenessListener`]
//!   (3-variant v0 surface — `Connected` / `Disconnected` /
//!   `HeartbeatStreamClosed`; the upstream heartbeat-detail variants are
//!   filtered out by `SwiftPeerLivenessEvent::is_v0_forwardable`).
//! - **Heartbeat source** via [`HeartbeatTimestampProvider`].
//! - **5-payload stream surface** — `StreamSubscriptionAudio` / …Camera
//!   / …PointCloud / …JointEncoders / …Detection upstream Objects
//!   plus matching `NetworkRuntime.open_*_stream` async methods. Producer
//!   side via the [`SwiftStreamProvider`] callback interface with a
//!   **two-call protocol**: `dispatch_decision` returns a
//!   [`SwiftStreamDecision`] (no trait-object fields — UniFFI 0.31
//!   constraint); on Accept, the runtime calls the matching `*_source`
//!   method to retrieve the per-payload `Box<dyn Swift*Source>`.
//!
//! Cluster lifecycle / peer enumeration (`ClusterManager` etc.) is the
//! future `auki-domain-swift` (PR C), mirroring the
//! `auki-network` / `auki-domain` split the Python bindings already
//! follow. PR A's `auki-identity-swift` ships `Wallet` and `PeerIdentity`.
//!
//! ## API shape
//!
//! Async. Unlike `auki-network-py` (which is deliberately sync-shaped
//! because Python callers live in a GIL world), this crate exports
//! `async`/`await` methods. UniFFI drives the exported futures on a
//! process-wide multi-thread tokio runtime via
//! `#[uniffi::export(async_runtime = "tokio")]`.
//!
//! `PeerId` and `Multiaddr` cross the FFI as their canonical strings via
//! `uniffi::custom_type!` registrations with the `remote` keyword (see
//! below). All prost-generated wire types (`StreamRequest`,
//! `StreamManifest`, `AudioFrame`, etc.) cross as opaque `Data`; Swift
//! decodes via swift-protobuf against `crates/auki-datatypes/proto/`.
//! Errors are typed `uniffi::Error` enums; the upstream
//! `auki_network::stream_runtime::StreamError`/`OpenStreamError`
//! variants that wrap non-FFI types are flattened to `message: String`.

pub use auki_network_rs::discovery_client::{
    ClusterEntry, CreateClusterOutcome, DiscoveryClient, DiscoveryError,
};
use auki_network_rs::HeartbeatTimestampSource;
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use std::sync::Arc;

uniffi::setup_scaffolding!();

// ─── Custom-type registrations ─────────────────────────────────────
//
// `PeerId` and `Multiaddr` are libp2p types defined in external crates
// (`libp2p-identity`, `multiaddr`); we can't annotate them directly. UniFFI's
// `custom_type!` with the `remote` keyword registers the conversion at the
// binding-crate level, anchored on this crate's `UniFfiTag` to satisfy the
// orphan rule. Every upstream `auki-network` method that takes or returns
// `PeerId` / `Multiaddr` is auto-exposed with `String` at the seam.
//
// `auki-domain-swift` (PR C) will pick up these registrations via its dep on
// this crate — no need to redeclare there.

// Cross-FFI representation: canonical libp2p peer-id string (`12D3KooW…`).
// Parse failures surface as a Rust `anyhow::Error` — UniFFI propagates the
// message to Swift as a thrown error on the affected method.
//
// The `remote` keyword is critical. Without it, the macro emits
// `impl<UT> FfiConverter<UT> for PeerId` which fails the orphan rule
// (PeerId and FfiConverter are both foreign). With `remote`, the impl
// becomes `impl FfiConverter<crate::UniFfiTag> for PeerId` — the
// binding crate's local UniFfiTag is the anchor.
uniffi::custom_type!(PeerId, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<PeerId>()
            .map_err(|e| anyhow::anyhow!("invalid peer-id {s:?}: {e}"))
    },
    lower: |p: PeerId| p.to_string(),
});

// Cross-FFI representation: canonical `/ip4/.../tcp/...` multiaddr
// string. Parse failures surface as Rust `anyhow::Error`. `remote`
// keyword for the same reason as the `PeerId` registration above.
uniffi::custom_type!(Multiaddr, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<Multiaddr>()
            .map_err(|e| anyhow::anyhow!("invalid multiaddr {s:?}: {e}"))
    },
    lower: |m: Multiaddr| m.to_string(),
});

// ─── Liveness events ───────────────────────────────────────────────
//
// `auki_network::PeerLivenessEvent` has 5 variants; two carry rich
// `Heartbeat*Observation` structs that aren't useful at v0 (iosapp's
// proof-bar UI only needs connect/disconnect/heartbeat-stream-closed).
// Translate to a 3-variant Swift-facing enum here and skip the two
// heartbeat-detail variants in the binding adapter — Rust callers
// continue to see the full upstream enum.

/// Peer connection-level liveness events surfaced to Swift consumers.
/// Each variant carries the affected peer-id as a canonical string.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum SwiftPeerLivenessEvent {
    /// A known peer connected at the libp2p connection layer.
    Connected { peer_id: String },
    /// A known peer disconnected at the libp2p connection layer.
    Disconnected { peer_id: String },
    /// A heartbeat substream closed or could not be opened. Useful for
    /// observers that want to distinguish transport-level disconnects
    /// from heartbeat-protocol failures.
    HeartbeatStreamClosed { peer_id: String },
}

impl SwiftPeerLivenessEvent {
    /// Translate an upstream `PeerLivenessEvent` into the Swift-facing
    /// 3-variant subset. The two heartbeat-detail upstream variants
    /// (`HeartbeatReceived`, `HeartbeatNtpSampleObserved`) get folded
    /// into `HeartbeatStreamClosed` as a placeholder; production callers
    /// of this function should pre-filter via `is_v0_forwardable` so
    /// those variants never reach this function.
    pub fn from_upstream(e: &auki_network_rs::PeerLivenessEvent) -> Self {
        use auki_network_rs::PeerLivenessEvent;
        match e {
            PeerLivenessEvent::Connected { peer_id } => Self::Connected {
                peer_id: peer_id.to_string(),
            },
            PeerLivenessEvent::Disconnected { peer_id } => Self::Disconnected {
                peer_id: peer_id.to_string(),
            },
            PeerLivenessEvent::HeartbeatStreamClosed { peer_id } => Self::HeartbeatStreamClosed {
                peer_id: peer_id.to_string(),
            },
            PeerLivenessEvent::HeartbeatReceived { peer_id, .. }
            | PeerLivenessEvent::HeartbeatNtpSampleObserved { peer_id, .. } => {
                Self::HeartbeatStreamClosed {
                    peer_id: peer_id.to_string(),
                }
            }
        }
    }

    /// True for upstream variants that should be forwarded to Swift
    /// listeners at v0 (filters out the two heartbeat-detail variants).
    pub fn is_v0_forwardable(upstream: &auki_network_rs::PeerLivenessEvent) -> bool {
        use auki_network_rs::PeerLivenessEvent;
        matches!(
            upstream,
            PeerLivenessEvent::Connected { .. }
                | PeerLivenessEvent::Disconnected { .. }
                | PeerLivenessEvent::HeartbeatStreamClosed { .. }
        )
    }
}

// ─── Peer liveness listener (Swift callback interface) ─────────────

/// Swift consumers implement this trait to receive peer liveness
/// events from the network runtime. Wrapped in `Arc<dyn ...>` and
/// passed into [`spawn_for_swift`]; the runtime's drain task calls
/// `on_event` for each forwardable upstream event.
///
/// `Send + Sync` per UniFFI callback-interface contract — Swift
/// implementations must be safe to call from a Rust tokio worker
/// thread. Swift compiler enforces this when adopting the protocol.
#[uniffi::export(callback_interface)]
pub trait PeerLivenessListener: Send + Sync {
    /// Invoked once per peer liveness event observed by the runtime.
    /// The drain task in `spawn_for_swift` runs on a tokio worker; long
    /// blocking work here will stall delivery of subsequent events.
    fn on_event(&self, event: SwiftPeerLivenessEvent);
}

// ─── Heartbeat timestamp provider (Swift callback interface) ───────

/// Swift consumers implement this trait to supply the heartbeat-source
/// timestamp readings and clock identity the runtime needs. Wrapped in
/// `Arc<dyn ...>`; the adapter [`heartbeat_source_from_provider`]
/// converts it into the upstream `HeartbeatTimestampSource` shape.
///
/// `clock_id` and `clock_hash` are read once at runtime spawn (they're
/// stable for the lifetime of the runtime). `now_ns` is invoked on
/// every outbound heartbeat frame; `domain_clock_bytes` is invoked the
/// same way and returns the JSON-encoded
/// `auki_network::heartbeat_protocol::HeartbeatDomainClock` or `None`.
///
/// Note: `HeartbeatDomainClock` uses JSON encoding (the heartbeat wire
/// format is length-prefixed JSON), so `domain_clock_bytes` must
/// carry a valid JSON object matching that struct's serde shape, or
/// `None` to signal "no domain clock to advertise".
#[uniffi::export(callback_interface)]
pub trait HeartbeatTimestampProvider: Send + Sync {
    /// Clock Registry id for the heartbeat `sent_at_clock_ns` values.
    /// Read once at spawn.
    fn clock_id(&self) -> String;
    /// Content-addressed hash of `clock_id`'s Clock Registry entry.
    /// Read once at spawn.
    fn clock_hash(&self) -> String;
    /// Current reading of `clock_id` in nanoseconds. Called per
    /// outbound heartbeat frame; must be fast (<1 ms).
    fn now_ns(&self) -> i64;
    /// JSON-encoded `auki_network::heartbeat_protocol::HeartbeatDomainClock`
    /// describing the domain clock this peer is currently advertising,
    /// or `None`. Called per outbound heartbeat frame.
    fn domain_clock_bytes(&self) -> Option<Vec<u8>>;
}

/// Adapter: build an upstream `HeartbeatTimestampSource` from a Swift
/// `HeartbeatTimestampProvider`. The closures wrap the trait-object
/// method calls; `domain_clock_bytes` results are decoded as
/// JSON-encoded `HeartbeatDomainClock` values (decode failure → `None`,
/// treated as "no domain clock to advertise").
pub(crate) fn heartbeat_source_from_provider(
    provider: Arc<dyn HeartbeatTimestampProvider>,
) -> HeartbeatTimestampSource {
    let clock_id = provider.clock_id();
    let clock_hash = provider.clock_hash();
    let p_for_now = provider.clone();
    let p_for_dc = provider.clone();
    HeartbeatTimestampSource {
        clock_id,
        clock_hash,
        now_ns: Arc::new(move || p_for_now.now_ns()),
        domain_clock: Arc::new(move || {
            p_for_dc.domain_clock_bytes().and_then(|bytes| {
                serde_json::from_slice::<auki_network_rs::heartbeat_protocol::HeartbeatDomainClock>(
                    &bytes,
                )
                .ok()
            })
        }),
    }
}

// ─── spawn_for_swift orchestrator ──────────────────────────────────

/// Errors from [`spawn_for_swift`].
///
/// `swift-bindings`: UniFFI Error. Flattens swarm-build failures to a
/// `message: String` since the underlying types are libp2p-specific.
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum SpawnSwiftError {
    /// `auki_network::swarm::build_swarm` failed (invalid listen
    /// multiaddr, transport bind failure, etc.).
    #[error("swarm build: {message}")]
    SwarmBuild { message: String },
    /// `NetworkRuntime::spawn` failed — currently only one variant
    /// (`NoTokioRuntime`), but propagated as a message string for
    /// consistency.
    #[error("runtime spawn: {message}")]
    RuntimeSpawn { message: String },
}

/// Swift entry point for spawning a `NetworkRuntime`. Builds the libp2p
/// swarm internally, wires the `PeerLivenessListener` to the
/// `PeerLivenessEvent` channel via a drain task, drops the other 8
/// receivers (cluster-orchestration concerns reach for them via
/// `auki-domain-swift::ClusterManager` in PR C).
///
/// ## UniFFI callback-interface note
///
/// UniFFI 0.31 `Lift`-implements `Box<dyn Trait>` for callback
/// interfaces — not `Arc<dyn Trait>`. Each `Box` is immediately promoted
/// to `Arc` inside the function body so that `drain_liveness_events` can
/// hold a `'static` reference across the tokio task boundary.
/// `heartbeat_source_from_provider` and `swift_provider_to_upstream`
/// likewise need `Arc`, so all three are promoted at entry. This is an
/// internal implementation detail — Swift callers see their protocol
/// conformances passed by value, same as any other UniFFI callback
/// interface.
#[uniffi::export(async_runtime = "tokio")]
pub async fn spawn_for_swift(
    identity: Arc<auki_network_rs::PeerIdentity>,
    listen_multiaddrs: Vec<Multiaddr>,
    allowed_peers: Vec<auki_network_rs::AllowedPeer>,
    peer_liveness_listener: Box<dyn PeerLivenessListener>,
    heartbeat_timestamps: Box<dyn HeartbeatTimestampProvider>,
    stream_provider: Box<dyn SwiftStreamProvider>,
) -> Result<Arc<auki_network_rs::NetworkRuntime>, SpawnSwiftError> {
    // Promote Box<dyn ...> to Arc<dyn ...> so the trait objects can be
    // cloned into spawned tasks / closures (UniFFI lifts callback
    // interfaces as Box; Arc::from gives us the 'static-clonable shape
    // the upstream runtime needs).
    let peer_liveness_listener: Arc<dyn PeerLivenessListener> = Arc::from(peer_liveness_listener);
    let heartbeat_timestamps: Arc<dyn HeartbeatTimestampProvider> = Arc::from(heartbeat_timestamps);
    let stream_provider: Arc<dyn SwiftStreamProvider> = Arc::from(stream_provider);

    // 1. Build the swarm.
    let swarm = auki_network_rs::swarm::build_swarm(
        identity.as_ref(),
        auki_network_rs::swarm::SwarmConfig {
            listen_addresses: listen_multiaddrs,
            agent_version: format!(
                "auki-network-swift/{}",
                env!("CARGO_PKG_VERSION")
            ),
            enable_relay_server: false,
        },
    )
    .map_err(|e| SpawnSwiftError::SwarmBuild {
        message: e.to_string(),
    })?;

    // 2. Build the heartbeat source from the Swift provider.
    let heartbeat_source = heartbeat_source_from_provider(heartbeat_timestamps);

    // 3. Build the upstream StreamProvider closure from the Swift provider.
    let upstream_provider = swift_provider_to_upstream(stream_provider);

    // 4. Spawn the runtime. The 9-element tuple destructure: (Self,
    //    join_rx, liveness_rx, membership_rx, info_rx, resources_rx,
    //    sensors_rx, registry_rx, diagnostic_rx). At v0 we only wire
    //    liveness_rx to the Swift listener; the others are dropped
    //    (their senders' errors are swallowed by run_task).
    let (rt, _join_rx, liveness_rx, _membership_rx, _info_rx, _resources_rx, _sensors_rx, _registry_rx, _diagnostic_rx) =
        auki_network_rs::NetworkRuntime::spawn(
            swarm,
            allowed_peers,
            upstream_provider,
            heartbeat_source,
        )
        .map_err(|e| SpawnSwiftError::RuntimeSpawn {
            message: e.to_string(),
        })?;

    // 5. Drain liveness events to the Swift listener.
    tokio::spawn(drain_liveness_events(liveness_rx, peer_liveness_listener));

    Ok(Arc::new(rt))
}

/// Drains the upstream `PeerLivenessEvent` receiver, forwarding each
/// `is_v0_forwardable` event to the Swift `PeerLivenessListener`.
async fn drain_liveness_events(
    mut rx: tokio::sync::mpsc::Receiver<auki_network_rs::PeerLivenessEvent>,
    listener: Arc<dyn PeerLivenessListener>,
) {
    while let Some(event) = rx.recv().await {
        if SwiftPeerLivenessEvent::is_v0_forwardable(&event) {
            listener.on_event(SwiftPeerLivenessEvent::from_upstream(&event));
        }
        // Else drop the heartbeat-detail variants per v0 design.
    }
}

// ─── Swift stream provider + source traits ─────────────────────────

/// Error type for Swift source-stream callbacks. UniFFI 0.31 doesn't
/// accept a raw `String` as the throw type on exported methods — error
/// values must be a typed `uniffi::Error`. This single-variant error
/// carries the producer-supplied detail message through to the runtime,
/// which maps it to `EndReason::ProducerError { detail }` on the wire.
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum SwiftSourceError {
    #[error("producer error: {message}")]
    Producer { message: String },
}

/// One source-stream item. The opaque `payload_bytes` is prost-encoded
/// against the per-payload-type proto file (`AudioFrame.proto`,
/// `CameraFrame.proto`, etc.); Swift consumers decode via swift-protobuf.
///
/// Shared shape across all 5 payload types — type-distinguishability
/// lives at the trait level (which `Swift*Source` you implement) and
/// the `StreamSubscription*` consumer side, not in the byte payload
/// representation.
#[derive(uniffi::Record, Clone, Debug)]
pub struct StreamItem {
    pub timestamp_ns: i64,
    pub payload_bytes: Vec<u8>,
}

/// Swift-implemented audio source. Returns the next `StreamItem`
/// containing prost-encoded `AudioFrame` bytes, `Ok(None)` for clean
/// end-of-source, or `Err(detail)` for producer error.
#[uniffi::export(callback_interface)]
pub trait SwiftAudioSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, SwiftSourceError>;
}

/// Swift-implemented camera source.
#[uniffi::export(callback_interface)]
pub trait SwiftCameraSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, SwiftSourceError>;
}

/// Swift-implemented point-cloud source.
#[uniffi::export(callback_interface)]
pub trait SwiftPointCloudSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, SwiftSourceError>;
}

/// Swift-implemented joint-encoders source.
#[uniffi::export(callback_interface)]
pub trait SwiftJointEncodersSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, SwiftSourceError>;
}

/// Swift-implemented detection source.
#[uniffi::export(callback_interface)]
pub trait SwiftDetectionSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, SwiftSourceError>;
}

/// Producer's accept/decline decision for one inbound stream request.
/// Each Accept variant names the payload type and carries the prost-
/// encoded manifest bytes only. The source-stream itself is supplied
/// by a follow-up call on the `SwiftStreamProvider`.
///
/// UniFFI 0.31 can't ship callback-interface trait objects inside Enum
/// variants (no `Lower<UniFfiTag>` for `Box<dyn T>` where T is a
/// callback interface), so we split provider into a two-call protocol:
/// `dispatch_decision` first, then the matching `*_source` method.
#[derive(uniffi::Enum)]
pub enum SwiftStreamDecision {
    AcceptAudio { manifest_bytes: Vec<u8> },
    AcceptCamera { manifest_bytes: Vec<u8> },
    AcceptPointCloud { manifest_bytes: Vec<u8> },
    AcceptJointEncoders { manifest_bytes: Vec<u8> },
    AcceptDetection { manifest_bytes: Vec<u8> },
    Decline { reason_bytes: Vec<u8> },
}

/// Swift-implemented stream provider. Two-call protocol per inbound
/// request: the runtime calls `dispatch_decision` to learn the payload
/// type + manifest, then (on Accept) calls the matching `*_source`
/// method to retrieve the source-stream trait object.
///
/// The Swift implementation must keep `dispatch_decision` and the
/// subsequent `*_source` call consistent for the same `(peer_id,
/// request_bytes)` pair. The runtime only invokes the `*_source`
/// method matching the Accept variant `dispatch_decision` returned.
#[uniffi::export(callback_interface)]
pub trait SwiftStreamProvider: Send + Sync {
    /// Decide whether to accept (and which payload type) or decline.
    /// Called once per inbound stream request.
    fn dispatch_decision(&self, peer_id: String, request_bytes: Vec<u8>) -> SwiftStreamDecision;

    /// Provide the audio source for an accepted request. Called
    /// immediately after `dispatch_decision` returns `AcceptAudio`.
    fn audio_source(&self, peer_id: String, request_bytes: Vec<u8>) -> Box<dyn SwiftAudioSource>;

    /// Provide the camera source for an accepted request.
    fn camera_source(
        &self,
        peer_id: String,
        request_bytes: Vec<u8>,
    ) -> Box<dyn SwiftCameraSource>;

    /// Provide the point-cloud source for an accepted request.
    fn point_cloud_source(
        &self,
        peer_id: String,
        request_bytes: Vec<u8>,
    ) -> Box<dyn SwiftPointCloudSource>;

    /// Provide the joint-encoders source for an accepted request.
    fn joint_encoders_source(
        &self,
        peer_id: String,
        request_bytes: Vec<u8>,
    ) -> Box<dyn SwiftJointEncodersSource>;

    /// Provide the detection source for an accepted request.
    fn detection_source(
        &self,
        peer_id: String,
        request_bytes: Vec<u8>,
    ) -> Box<dyn SwiftDetectionSource>;
}

// ─── Source-stream adapters: Swift trait → upstream SourceStream<T> ─

/// Adapter: wraps a Swift `SwiftAudioSource` as an upstream
/// `SourceStream<AudioFrame>`. Spawns a tokio task that polls the trait
/// and pushes prost-decoded items onto an mpsc; returns a
/// `ReceiverStream` wrapper. Cancellation: when the runtime drops the
/// `SourceStream` (e.g. substream closed), the receiver drops, the
/// mpsc send fails, and the task exits.
pub(crate) fn audio_source_to_stream(
    source: Box<dyn SwiftAudioSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::AudioFrame,
> {
    let source: Arc<dyn SwiftAudioSource> = Arc::from(source);
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::AudioFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame =
                        match auki_network_rs::stream_protocol::AudioFrame::decode(
                            item.payload_bytes.as_slice(),
                        ) {
                            Ok(f) => f,
                            Err(e) => {
                                let _ = tx.send(Err(format!("AudioFrame decode: {e}"))).await;
                                break;
                            }
                        };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(SwiftSourceError::Producer { message: detail }) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftCameraSource` → `SourceStream<CameraFrame>`. Same
/// pattern as [`audio_source_to_stream`] with the prost decode target
/// swapped to `CameraFrame`.
pub(crate) fn camera_source_to_stream(
    source: Box<dyn SwiftCameraSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::CameraFrame,
> {
    let source: Arc<dyn SwiftCameraSource> = Arc::from(source);
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::CameraFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame =
                        match auki_network_rs::stream_protocol::CameraFrame::decode(
                            item.payload_bytes.as_slice(),
                        ) {
                            Ok(f) => f,
                            Err(e) => {
                                let _ = tx.send(Err(format!("CameraFrame decode: {e}"))).await;
                                break;
                            }
                        };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(SwiftSourceError::Producer { message: detail }) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftPointCloudSource` → `SourceStream<PointCloudFrame>`.
pub(crate) fn point_cloud_source_to_stream(
    source: Box<dyn SwiftPointCloudSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::PointCloudFrame,
> {
    let source: Arc<dyn SwiftPointCloudSource> = Arc::from(source);
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::PointCloudFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame = match auki_network_rs::stream_protocol::PointCloudFrame::decode(
                        item.payload_bytes.as_slice(),
                    ) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = tx.send(Err(format!("PointCloudFrame decode: {e}"))).await;
                            break;
                        }
                    };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(SwiftSourceError::Producer { message: detail }) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftJointEncodersSource` → `SourceStream<JointEncodersFrame>`.
pub(crate) fn joint_encoders_source_to_stream(
    source: Box<dyn SwiftJointEncodersSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::JointEncodersFrame,
> {
    let source: Arc<dyn SwiftJointEncodersSource> = Arc::from(source);
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::JointEncodersFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame = match auki_network_rs::stream_protocol::JointEncodersFrame::decode(
                        item.payload_bytes.as_slice(),
                    ) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = tx.send(Err(format!("JointEncodersFrame decode: {e}"))).await;
                            break;
                        }
                    };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(SwiftSourceError::Producer { message: detail }) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftDetectionSource` → `SourceStream<DetectionFrame>`.
/// `DetectionFrame` lives in `auki_datatypes::detection`, not in
/// `auki_network`'s stream_protocol module.
pub(crate) fn detection_source_to_stream(
    source: Box<dyn SwiftDetectionSource>,
) -> auki_network_rs::stream_runtime::SourceStream<auki_datatypes::detection::DetectionFrame> {
    let source: Arc<dyn SwiftDetectionSource> = Arc::from(source);
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<auki_datatypes::detection::DetectionFrame>,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame = match auki_datatypes::detection::DetectionFrame::decode(
                        item.payload_bytes.as_slice(),
                    ) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = tx.send(Err(format!("DetectionFrame decode: {e}"))).await;
                            break;
                        }
                    };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(SwiftSourceError::Producer { message: detail }) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

// ─── Swift provider → upstream StreamProvider adapter ──────────────

/// Wraps a Swift `SwiftStreamProvider` as the upstream
/// `StreamProvider` closure type. Each invocation of the closure
/// follows the two-call protocol: dispatch_decision, then (on Accept)
/// the matching `*_source` call. The source-stream-from-callback
/// adapters do the prost decoding.
pub(crate) fn swift_provider_to_upstream(
    provider: Arc<dyn SwiftStreamProvider>,
) -> auki_network_rs::stream_runtime::StreamProvider {
    Arc::new(
        move |peer: libp2p_identity::PeerId, request: auki_network_rs::stream_protocol::StreamRequest| {
            use prost::Message;
            let peer_id_str = peer.to_string();
            let request_bytes = request.encode_to_vec();
            let decision = provider.dispatch_decision(peer_id_str.clone(), request_bytes.clone());

            match decision {
                SwiftStreamDecision::Decline { reason_bytes } => {
                    let reason = auki_network_rs::stream_protocol::DeclineReason::decode(
                        reason_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    auki_network_rs::stream_runtime::StreamDispatch::Decline { reason }
                }
                SwiftStreamDecision::AcceptAudio { manifest_bytes } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    let source = provider.audio_source(peer_id_str, request_bytes);
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptAudio {
                        manifest,
                        source: audio_source_to_stream(source),
                    }
                }
                SwiftStreamDecision::AcceptCamera { manifest_bytes } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    let source = provider.camera_source(peer_id_str, request_bytes);
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptCamera {
                        manifest,
                        source: camera_source_to_stream(source),
                    }
                }
                SwiftStreamDecision::AcceptPointCloud { manifest_bytes } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    let source = provider.point_cloud_source(peer_id_str, request_bytes);
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptPointCloud {
                        manifest,
                        source: point_cloud_source_to_stream(source),
                    }
                }
                SwiftStreamDecision::AcceptJointEncoders { manifest_bytes } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    let source = provider.joint_encoders_source(peer_id_str, request_bytes);
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptJointEncoders {
                        manifest,
                        source: joint_encoders_source_to_stream(source),
                    }
                }
                SwiftStreamDecision::AcceptDetection { manifest_bytes } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    let source = provider.detection_source(peer_id_str, request_bytes);
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptDetection {
                        manifest,
                        source: detection_source_to_stream(source),
                    }
                }
            }
        },
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic PeerId from a fixed ed25519 key — avoids needing
    /// libp2p-identity's `rand` feature in the shipped lib.
    fn test_peer_id() -> PeerId {
        libp2p_identity::Keypair::ed25519_from_bytes([7u8; 32])
            .expect("valid ed25519 seed")
            .public()
            .to_peer_id()
    }

    /// Malformed PeerId / Multiaddr strings fail the `uniffi::custom_type!`
    /// `try_lift` arm. Swift sees these as thrown errors on the affected
    /// method.
    #[test]
    fn malformed_peer_id_lifts_to_error() {
        assert!("not-a-peer-id".parse::<PeerId>().is_err());
    }

    #[test]
    fn malformed_multiaddr_lifts_to_error() {
        assert!("definitely/not/an/addr".parse::<Multiaddr>().is_err());
    }

    /// `PeerId` round-trips through its UniFFI custom-type registration:
    /// canonical string in → `PeerId` → canonical string out (identical).
    #[test]
    fn peer_id_custom_type_round_trips() {
        let pid = test_peer_id();
        let s = pid.to_string();
        let back: PeerId = s.parse().expect("canonical PeerId string parses");
        assert_eq!(back, pid);
    }

    /// `Multiaddr` round-trips through its UniFFI custom-type registration.
    #[test]
    fn multiaddr_custom_type_round_trips() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let s = addr.to_string();
        let back: Multiaddr = s.parse().expect("canonical multiaddr parses");
        assert_eq!(back, addr);
    }

    /// `AllowedPeer` is constructible from canonical PeerId + multiaddr
    /// strings via UniFFI's auto-derived constructor. Exercises the
    /// custom-type lowering chain (String → PeerId → Vec<Multiaddr>).
    #[test]
    fn allowed_peer_constructs_with_string_inputs() {
        let pid = test_peer_id();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let ap = auki_network_rs::AllowedPeer {
            peer_id: pid,
            multiaddrs: vec![addr.clone()],
        };
        assert_eq!(ap.peer_id, pid);
        assert_eq!(ap.multiaddrs, vec![addr]);
    }

    /// `SpawnError` is a Display'd error; UniFFI flattens it as a tagged enum.
    #[test]
    fn spawn_error_is_display_clean() {
        use auki_network_rs::SpawnError;
        let e = SpawnError::NoTokioRuntime;
        assert!(!e.to_string().is_empty());
    }

    /// `UpdateError` round-trips through Display; UniFFI surfaces it as a
    /// tagged enum (no opaque payloads after annotation).
    #[test]
    fn update_error_is_display_clean() {
        use auki_network_rs::UpdateError;
        let e = UpdateError::RuntimeUnavailable;
        assert!(!e.to_string().is_empty());
    }

    /// `SwiftPeerLivenessEvent::from_upstream` translates each upstream
    /// variant to the right Swift variant.
    #[test]
    fn swift_peer_liveness_event_translation() {
        use auki_network_rs::PeerLivenessEvent;

        let pid = test_peer_id();
        let connected = PeerLivenessEvent::Connected { peer_id: pid };
        let s = SwiftPeerLivenessEvent::from_upstream(&connected);
        assert!(matches!(s, SwiftPeerLivenessEvent::Connected { .. }));

        let disconnected = PeerLivenessEvent::Disconnected { peer_id: pid };
        let s = SwiftPeerLivenessEvent::from_upstream(&disconnected);
        assert!(matches!(s, SwiftPeerLivenessEvent::Disconnected { .. }));

        let heartbeat_closed = PeerLivenessEvent::HeartbeatStreamClosed { peer_id: pid };
        let s = SwiftPeerLivenessEvent::from_upstream(&heartbeat_closed);
        assert!(matches!(s, SwiftPeerLivenessEvent::HeartbeatStreamClosed { .. }));
    }

    /// Smoke test: a no-op `PeerLivenessListener` impl compiles and can be
    /// stored as `Arc<dyn PeerLivenessListener>`. Real wire-up tested in
    /// the Task 10 spawn_for_swift smoke test.
    #[test]
    fn peer_liveness_listener_is_object_safe() {
        struct NoopListener;
        impl PeerLivenessListener for NoopListener {
            fn on_event(&self, _event: SwiftPeerLivenessEvent) {}
        }
        let listener: Arc<dyn PeerLivenessListener> = Arc::new(NoopListener);
        // Use it once so the binding isn't dead code.
        listener.on_event(SwiftPeerLivenessEvent::HeartbeatStreamClosed {
            peer_id: "irrelevant".to_string(),
        });
    }

    /// `NetworkRuntime` exposes its annotated method set. We can't spawn one
    /// here (needs a real tokio runtime + swarm), but we can confirm the
    /// types compile.
    #[test]
    fn network_runtime_is_uniffi_object() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<auki_network_rs::NetworkRuntime>();
    }

    /// Smoke test: `spawn_for_swift` constructs a runtime against a no-op
    /// listener + a wall-clock heartbeat provider + a decline-all stream
    /// provider, then shuts it down cleanly. Requires a real tokio runtime.
    #[tokio::test]
    async fn spawn_for_swift_smoke() {
        struct NoopListener;
        impl PeerLivenessListener for NoopListener {
            fn on_event(&self, _event: SwiftPeerLivenessEvent) {}
        }

        struct WallClockProvider;
        impl HeartbeatTimestampProvider for WallClockProvider {
            fn clock_id(&self) -> String {
                "smoke-clock".to_string()
            }
            fn clock_hash(&self) -> String {
                "smoke-hash".to_string()
            }
            fn now_ns(&self) -> i64 {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0)
            }
            fn domain_clock_bytes(&self) -> Option<Vec<u8>> {
                None
            }
        }

        struct DeclineAllProvider;
        impl SwiftStreamProvider for DeclineAllProvider {
            fn dispatch_decision(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> SwiftStreamDecision {
                SwiftStreamDecision::Decline {
                    reason_bytes: vec![],
                }
            }
            fn audio_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftAudioSource> {
                unreachable!("test never accepts")
            }
            fn camera_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftCameraSource> {
                unreachable!("test never accepts")
            }
            fn point_cloud_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftPointCloudSource> {
                unreachable!("test never accepts")
            }
            fn joint_encoders_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftJointEncodersSource> {
                unreachable!("test never accepts")
            }
            fn detection_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftDetectionSource> {
                unreachable!("test never accepts")
            }
        }

        let wallet = auki_identity::Wallet::from_seed(vec![1u8; 32]).expect("32-byte seed");
        let identity =
            std::sync::Arc::new(auki_network_rs::PeerIdentity::from_wallet(wallet));

        let listener: Box<dyn PeerLivenessListener> = Box::new(NoopListener);
        let heartbeat: Box<dyn HeartbeatTimestampProvider> = Box::new(WallClockProvider);
        let stream_provider: Box<dyn SwiftStreamProvider> = Box::new(DeclineAllProvider);

        let rt = spawn_for_swift(
            identity,
            vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            vec![],
            listener,
            heartbeat,
            stream_provider,
        )
        .await
        .expect("spawn succeeds");

        let pid = rt.local_peer_id_string();
        assert!(pid.starts_with("12D3KooW"));
        assert!(rt.connected_peer_id_strings().is_empty());
        rt.shutdown();
    }

    /// All 5 source traits + the provider trait compile and are object-
    /// safe.
    #[test]
    fn swift_stream_provider_object_safety() {
        struct NoopProvider;
        impl SwiftStreamProvider for NoopProvider {
            fn dispatch_decision(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> SwiftStreamDecision {
                SwiftStreamDecision::Decline {
                    reason_bytes: vec![],
                }
            }
            fn audio_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftAudioSource> {
                unreachable!("test never accepts")
            }
            fn camera_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftCameraSource> {
                unreachable!("test never accepts")
            }
            fn point_cloud_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftPointCloudSource> {
                unreachable!("test never accepts")
            }
            fn joint_encoders_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftJointEncodersSource> {
                unreachable!("test never accepts")
            }
            fn detection_source(
                &self,
                _peer_id: String,
                _request_bytes: Vec<u8>,
            ) -> Box<dyn SwiftDetectionSource> {
                unreachable!("test never accepts")
            }
        }
        let _p: Box<dyn SwiftStreamProvider> = Box::new(NoopProvider);
    }

    use futures::StreamExt;

    /// `audio_source_to_stream` drains a Swift source that produces 3
    /// prost-encoded `AudioFrame`s then ends-of-source. Rust side reads
    /// back 3 items + `None`.
    #[tokio::test]
    async fn audio_source_adapter_drains_three_items() {
        use auki_network_rs::stream_protocol::AudioFrame;
        use prost::Message;

        struct ThreeItems {
            counter: std::sync::Mutex<u8>,
        }
        impl SwiftAudioSource for ThreeItems {
            fn next_item(&self) -> Result<Option<StreamItem>, SwiftSourceError> {
                let mut c = self.counter.lock().unwrap();
                if *c >= 3 {
                    return Ok(None);
                }
                *c += 1;
                let frame = AudioFrame {
                    data: vec![*c],
                    ..Default::default()
                };
                Ok(Some(StreamItem {
                    timestamp_ns: *c as i64,
                    payload_bytes: frame.encode_to_vec(),
                }))
            }
        }
        let source: Box<dyn SwiftAudioSource> = Box::new(ThreeItems {
            counter: Default::default(),
        });
        let mut rust_stream = audio_source_to_stream(source);
        for expected in 1u8..=3 {
            let item = rust_stream
                .next()
                .await
                .expect("stream has more items")
                .expect("item is Ok");
            assert_eq!(item.timestamp_ns, expected as i64);
            assert_eq!(item.payload.data, vec![expected]);
        }
        assert!(rust_stream.next().await.is_none(), "source ended");
    }

    #[tokio::test]
    async fn stream_subscription_audio_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{AudioFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![
            Ok(auki_network_rs::stream_runtime::StreamEntry {
                timestamp_ns: 1,
                seq: 0,
                payload: AudioFrame {
                    data: vec![1, 2, 3],
                    ..Default::default()
                },
            }),
            Ok(auki_network_rs::stream_runtime::StreamEntry {
                timestamp_ns: 2,
                seq: 1,
                payload: AudioFrame {
                    data: vec![4, 5],
                    ..Default::default()
                },
            }),
        ]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionAudio::from_inner(sub);

        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.seq, 0);
        let second = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(second.seq, 1);
        let third = wrapped.next_entry().await.expect("ok");
        assert!(third.is_none(), "stream ended");
    }

    #[tokio::test]
    async fn stream_subscription_detection_wraps_typed_subscription() {
        use auki_datatypes::detection::DetectionFrame;
        use auki_network_rs::stream_protocol::StreamManifest;
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 7,
            seq: 0,
            payload: DetectionFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionDetection::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 7);
        assert!(wrapped.next_entry().await.expect("ok").is_none());
    }

    #[tokio::test]
    async fn stream_subscription_joint_encoders_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{JointEncodersFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 7,
            seq: 0,
            payload: JointEncodersFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionJointEncoders::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 7);
        assert!(wrapped.next_entry().await.expect("ok").is_none());
    }

    #[tokio::test]
    async fn stream_subscription_pointcloud_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{PointCloudFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 7,
            seq: 0,
            payload: PointCloudFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionPointCloud::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 7);
        assert!(wrapped.next_entry().await.expect("ok").is_none());
    }

    #[tokio::test]
    async fn stream_subscription_camera_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{CameraFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 7,
            seq: 0,
            payload: CameraFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionCamera::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 7);
        assert!(wrapped.next_entry().await.expect("ok").is_none());
    }

    /// `SwiftStreamDecision::Decline` constructs and matches.
    #[test]
    fn swift_stream_decision_decline_variant() {
        let d = SwiftStreamDecision::Decline {
            reason_bytes: b"reason".to_vec(),
        };
        match d {
            SwiftStreamDecision::Decline { reason_bytes } => assert_eq!(reason_bytes, b"reason"),
            _ => panic!("wrong variant"),
        }
    }

    /// Each `Accept*` variant constructs cleanly with only manifest bytes.
    #[test]
    fn swift_stream_decision_accept_variants() {
        let _ = SwiftStreamDecision::AcceptAudio {
            manifest_bytes: vec![1, 2, 3],
        };
        let _ = SwiftStreamDecision::AcceptCamera {
            manifest_bytes: vec![4, 5, 6],
        };
        let _ = SwiftStreamDecision::AcceptPointCloud {
            manifest_bytes: vec![],
        };
        let _ = SwiftStreamDecision::AcceptJointEncoders {
            manifest_bytes: vec![],
        };
        let _ = SwiftStreamDecision::AcceptDetection {
            manifest_bytes: vec![],
        };
    }

    /// Smoke test: a `HeartbeatTimestampProvider` impl can be converted
    /// into an upstream `HeartbeatTimestampSource` via the adapter.
    #[test]
    fn heartbeat_timestamp_provider_adapter() {
        struct WallClockProvider;
        impl HeartbeatTimestampProvider for WallClockProvider {
            fn clock_id(&self) -> String {
                "test-clock".to_string()
            }
            fn clock_hash(&self) -> String {
                "test-hash".to_string()
            }
            fn now_ns(&self) -> i64 {
                42
            }
            fn domain_clock_bytes(&self) -> Option<Vec<u8>> {
                None
            }
        }
        let provider: Arc<dyn HeartbeatTimestampProvider> = Arc::new(WallClockProvider);
        let src = heartbeat_source_from_provider(provider);
        assert_eq!(src.clock_id, "test-clock");
        assert_eq!(src.clock_hash, "test-hash");
        assert_eq!((src.now_ns)(), 42);
        assert!((src.domain_clock)().is_none());
    }
}
