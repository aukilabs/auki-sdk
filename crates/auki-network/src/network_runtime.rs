//! Runtime that drives a libp2p swarm against a list of known peers.
//!
//! [`NetworkRuntime`] owns a [`Swarm`]`<`[`Behaviour`]`>` and a tokio
//! task internally. It tracks "known peers" (an in-process membership
//! list), auto-dials peers whose multiaddrs are known, reconnects on
//! disconnect with exponential backoff, accepts inbound substreams on
//! `/auki/stream/0.1.0` (handed off to the consumer's
//! `stream_provider`), and accepts inbound `/auki/join/0.0.1`
//! substreams (handed off to the owner via the `JoinEvent` channel
//! returned by [`Self::spawn`]). Consumers interact through the small
//! set of public methods; they don't drive the swarm event loop
//! themselves.
//!
//! ## Cluster trust boundary
//!
//! Connection-level: open by default — libp2p completes handshakes
//! with anyone. Per-protocol gates enforce cluster membership inside
//! their own handlers (the `/auki/stream/0.1.0` accept path filters
//! by `known_peers`; the `/auki/join/0.0.1` path intentionally does
//! NOT gate, since a non-member peer's first contact with a cluster
//! IS the join handshake). The libp2p `block_list` is reserved for
//! evicting misbehaving peers, not for routine membership
//! enforcement.
//!
//! ## Not the home for
//!
//! - Cluster membership semantics (who's in the cluster, who's the
//!   Manager, when peers join/leave). Those live one layer up
//!   (`auki-domain`'s `ClusterMembership` + Manager state machine).
//!   The runtime is the libp2p plumbing the upper layer steers.
//! - Successor tokens, election rules, gossip. Same — those are
//!   `auki-domain` concerns.

use crate::diagnostic_protocol::{
    DIAGNOSTIC_PROTOCOL, DiagnosticMessage, read_diagnostic_message, write_diagnostic_message,
};
use crate::heartbeat_protocol::{
    HEARTBEAT_INTERVAL, HEARTBEAT_PROTOCOL, HEARTBEAT_WRITE_STALL_WARN, Heartbeat,
    HeartbeatDomainClock, HeartbeatEcho, read_heartbeat, write_heartbeat,
};
use crate::info_protocol::{
    INFO_PROTOCOL, InfoProtocolError, InfoRequest, InfoResponse, read_info_request,
    read_info_response, write_info_request, write_info_response,
};
use crate::join_protocol::{
    JOIN_PROTOCOL, JoinProtocolError, JoinRequest, JoinResponse, read_join_request,
    read_join_response, write_join_request, write_join_response,
};
use crate::membership_protocol::{
    MEMBERSHIP_PROTOCOL, MembershipUpdate, read_membership_update, write_membership_update,
};
use crate::message_protocol::{
    MESSAGE_PROTOCOL, MessageChannelRegistration, MessageChannelRouter, MessageChannelSender,
    MessageFrameMemoryBudget, OpenMessageChannelError, RegistrationError,
    handle_inbound_substream as handle_inbound_message, open_message_channel,
};
use crate::registries_protocol::{
    REGISTRIES_PROTOCOL, RegistriesProtocolError, RegistryRequest, RegistryResponse,
    read_registry_request, read_registry_response, write_registry_request, write_registry_response,
};
use crate::blobs_protocol::{
    BLOBS_PROTOCOL, BlobRequest, BlobResponseMeta, BlobsProtocolError, read_blob_request,
    read_blob_response, write_blob_request, write_blob_response,
};
use crate::resources_protocol::{
    RESOURCES_PROTOCOL, ResourcesProtocolError, ResourcesRequest, ResourcesResponse,
    read_resources_request, read_resources_response, write_resources_request,
    write_resources_response,
};
use crate::resources_v3_protocol::{
    RESOURCES_PROTOCOL as RESOURCES_V3_PROTOCOL, ResourceEntry as ResourceEntryV3,
    ResourceVariant as ResourceVariantV3, ResourcesProtocolError as ResourcesV3ProtocolError,
    ResourcesRequest as ResourcesRequestV3, ResourcesResponse as ResourcesResponseV3,
    read_resources_request as read_resources_request_v3,
    read_resources_response as read_resources_response_v3,
    write_resources_request as write_resources_request_v3,
    write_resources_response as write_resources_response_v3,
};
use crate::resources_v4_protocol::{
    RESOURCES_PROTOCOL as RESOURCES_V4_PROTOCOL,
    ResourcesProtocolError as ResourcesV4ProtocolError, ResourcesRequest as ResourcesRequestV4,
    ResourcesResponse as ResourcesResponseV4, read_resources_request as read_resources_request_v4,
    read_resources_response as read_resources_response_v4,
    write_resources_request as write_resources_request_v4,
    write_resources_response as write_resources_response_v4,
};
use crate::resources_v5_protocol::{
    RESOURCES_PROTOCOL as RESOURCES_V5_PROTOCOL,
    ResourcesProtocolError as ResourcesV5ProtocolError, ResourcesRequest as ResourcesRequestV5,
    ResourcesResponse as ResourcesResponseV5, read_resources_request as read_resources_request_v5,
    read_resources_response as read_resources_response_v5,
    write_resources_request as write_resources_request_v5,
    write_resources_response as write_resources_response_v5,
};
#[cfg(test)]
use crate::{
    PeerIdentity,
    swarm::{SwarmConfig, build_swarm},
};
use crate::{
    stream_protocol::STREAM_PROTOCOL,
    stream_runtime::{StreamProvider, handle_inbound_substream},
    swarm::{self, Behaviour, BehaviourEvent},
};
use auki_time::{NtpExchange, NtpSample, compute_ntp_sample};
use futures::StreamExt;
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, swarm::SwarmEvent};
use libp2p_stream::Control;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

/// How long [`NetworkRuntime::shutdown`] gives in-flight inbound
/// substream tasks to flush their final `EndOfStream` before the swarm
/// tears down. 100 ms is comfortably more than the time required to
/// write a single small framed message over a healthy LAN substream.
/// On unclean exit (`Drop`, panic) the grace period is skipped —
/// consumer sees `ConnectionLost` instead of the typed reason.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(100);

/// Initial reconnect backoff. Doubled on each consecutive dial failure
/// or unexpected disconnect, up to [`MAX_BACKOFF`]. Reset on a
/// successful `ConnectionEstablished`.
pub const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Cap on the per-peer reconnect backoff.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Period at which the runtime checks pending reconnects.
pub const RECONNECT_TICK: Duration = Duration::from_millis(500);

/// One entry in the runtime's allow-list / auto-dial schedule.
///
/// `swift-bindings`: derived as a UniFFI Record. `peer_id` and
/// `multiaddrs` cross the FFI as canonical strings via the
/// custom-type registrations in `auki-network-swift`.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedPeer {
    /// libp2p peer-id of this peer.
    pub peer_id: PeerId,
    /// Dialable multiaddrs for this peer. Empty list = the runtime
    /// allows inbound connections from this peer but does not
    /// auto-dial them.
    pub multiaddrs: Vec<Multiaddr>,
}

/// Clock reading callback used when writing heartbeat frames.
pub type HeartbeatNowNs = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Optional domain-clock metadata callback used when writing heartbeat frames.
pub type HeartbeatDomainClockNs = Arc<dyn Fn() -> Option<HeartbeatDomainClock> + Send + Sync>;

/// Sender-clock identity and timestamp source for `/auki/heartbeat/0.0.1`.
///
/// The runtime treats this as required construction input. It does not
/// synthesize a clock identity on its own.
#[derive(Clone)]
pub struct HeartbeatTimestampSource {
    /// Clock Registry id for heartbeat `sent_at_clock_ns` values.
    pub clock_id: String,
    /// Content-addressed hash of `clock_id`'s Clock Registry entry.
    pub clock_hash: String,
    /// Returns the current reading of `clock_id` in nanoseconds.
    pub now_ns: HeartbeatNowNs,
    /// Returns the domain-clock source metadata to carry on this
    /// heartbeat frame, if this peer is currently advertising one.
    pub domain_clock: HeartbeatDomainClockNs,
}

/// Raw timing fact observed when a heartbeat frame is received.
///
/// This is still a transport-level observation. The runtime does not
/// compute an NTP sample or decide which domain clock matters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatTimingObservation {
    /// The peer that sent `heartbeat`.
    pub peer_id: PeerId,
    /// The received heartbeat frame, including sender clock identity
    /// and optional echo fields.
    pub heartbeat: Heartbeat,
    /// Local clock reading when `heartbeat` was received.
    pub received_at_clock_ns: i64,
    /// Local Clock Registry id for `received_at_clock_ns`.
    pub local_clock_id: String,
    /// Content-addressed hash of `local_clock_id`'s Clock Registry entry.
    pub local_clock_hash: String,
}

/// Raw NTP-style sample produced by matching a peer heartbeat echo to
/// a locally sent heartbeat timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatNtpSampleObservation {
    /// The peer whose clock was measured.
    pub peer_id: PeerId,
    /// Local Clock Registry id for the sample's local timestamps.
    pub local_clock_id: String,
    /// Content-addressed hash of `local_clock_id`'s Clock Registry entry.
    pub local_clock_hash: String,
    /// Remote Clock Registry id for the sample's remote timestamps.
    pub remote_clock_id: String,
    /// Content-addressed hash of `remote_clock_id`'s Clock Registry entry.
    pub remote_clock_hash: String,
    /// Estimated `remote_clock - local_clock` sample.
    pub sample: NtpSample,
}

/// Errors from [`NetworkRuntime::spawn`].
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// Constructor was called outside a tokio runtime context — the
    /// runtime needs a tokio handle to spawn its driver task.
    #[error("no current tokio runtime — call from within a tokio runtime context")]
    NoTokioRuntime,
}

/// Inbound `/auki/join/0.0.1` event surfaced by the runtime to its
/// owner via the channel returned from [`NetworkRuntime::spawn`].
///
/// The owner (typically `auki-domain`'s `ClusterManager`) reads the
/// request, decides admit-or-reject, and replies via `ack`. The
/// runtime's per-substream task awaits the reply for up to
/// [`JOIN_RESPONSE_TIMEOUT`] before giving up.
#[derive(Debug)]
pub struct JoinEvent {
    /// The peer-id of the requester. Authenticated by libp2p's noise
    /// handshake at connection-establishment time.
    pub peer: PeerId,
    /// The body of the request.
    pub request: JoinRequest,
    /// One-shot channel to reply on. Dropping it without sending is
    /// equivalent to a timeout from the requester's perspective.
    pub ack: oneshot::Sender<JoinResponse>,
}

/// How long the runtime's per-substream join task waits for the
/// owner to reply via the `JoinEvent::ack` channel before closing
/// the substream. Generous because the owner may need to do I/O
/// (e.g. write to disk in a future Manager-state-machine variant).
const JOIN_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the consumer-side [`NetworkRuntime::send_join_request`]
/// waits for the producer's response before returning a timeout
/// error.
pub const JOIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Number of locally sent heartbeat timestamps retained so later peer
/// echoes can be paired into NTP-style samples.
const SENT_HEARTBEAT_CACHE_CAPACITY: usize = 64;

/// Inbound peer/heartbeat-carrier event surfaced by the runtime to
/// its owner via the channel returned from [`NetworkRuntime::spawn`].
///
/// These are transport facts, not cluster semantics. The runtime
/// reports libp2p connection changes, inbound heartbeat frames, and
/// heartbeat substream closure. The owner decides which peer(s) matter
/// for liveness, when a heartbeat timeout has expired, and what a loss
/// means for the cluster.
#[derive(Debug)]
pub enum PeerLivenessEvent {
    /// A known peer connected at the libp2p connection layer.
    Connected {
        /// The peer-id of the peer.
        peer_id: PeerId,
    },
    /// A known peer disconnected at the libp2p connection layer.
    Disconnected {
        /// The peer-id of the peer.
        peer_id: PeerId,
    },
    /// A heartbeat frame arrived on `/auki/heartbeat/0.0.1`.
    HeartbeatReceived {
        /// The peer-id of the peer.
        peer_id: PeerId,
        /// Raw sender/local clock timing observation for this frame.
        observation: HeartbeatTimingObservation,
    },
    /// A heartbeat echo matched one of our remembered sent frames,
    /// producing a raw NTP-style sample.
    HeartbeatNtpSampleObserved {
        /// The peer-id of the peer.
        peer_id: PeerId,
        /// Raw NTP sample plus clock identities.
        observation: HeartbeatNtpSampleObservation,
    },
    /// A heartbeat substream closed or could not be opened.
    HeartbeatStreamClosed {
        /// The peer-id of the peer.
        peer_id: PeerId,
    },
    /// Flushing one outbound heartbeat to `peer_id` took at least
    /// [`HEARTBEAT_WRITE_STALL_WARN`]. Diagnostic only — the local
    /// peer's outbound liveness is at risk (the remote may declare us
    /// `Lost`), typically because the shared transport is congested and
    /// starving the heartbeat substream's flush. Owners log this; it
    /// drives no state change.
    HeartbeatWriteStalled {
        /// The peer-id the stalled write was addressed to.
        peer_id: PeerId,
        /// How long the single `write_heartbeat` flush took.
        waited: Duration,
    },
}

/// Errors from [`NetworkRuntime::send_join_request`].
#[derive(Debug, thiserror::Error)]
pub enum SendJoinRequestError {
    /// `libp2p_stream::Control::open_stream` failed (peer not
    /// reachable, no allow-list entry, etc.).
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    /// I/O or wire-format error reading/writing the framed request
    /// or response.
    #[error("protocol: {0}")]
    Protocol(#[source] JoinProtocolError),
    /// The full request/response round-trip didn't complete within
    /// [`JOIN_REQUEST_TIMEOUT`].
    #[error("join request timed out after {0:?}")]
    Timeout(Duration),
}

/// Inbound `/auki/info/0.0.1` event surfaced by the runtime to its
/// owner via the channel returned from [`NetworkRuntime::spawn`].
///
/// The owner (typically `auki-domain`'s `ClusterManager`) builds the
/// requesting peer's expected response — a serialized
/// [`crate::ParticipantInfo`] — and replies via `ack`. The runtime's
/// per-substream task awaits the reply for up to
/// [`INFO_RESPONSE_TIMEOUT`] before closing the substream silently.
#[derive(Debug)]
pub struct InfoRequestEvent {
    /// The peer-id of the requester. Authenticated by libp2p's noise
    /// handshake at connection-establishment time.
    pub peer: PeerId,
    /// The body of the request. Empty today — reserved for future
    /// delta-fetching fields.
    pub request: InfoRequest,
    /// One-shot channel to reply on. Send a fully-serialized
    /// `ParticipantInfo` JSON wrapped in an [`InfoResponse`].
    /// Dropping the sender without sending closes the substream
    /// silently — the requester sees an [`InfoProtocolError::Io`]
    /// with `UnexpectedEof`.
    pub ack: oneshot::Sender<InfoResponse>,
}

/// How long the runtime's per-substream info task waits for the
/// owner to reply via the [`InfoRequestEvent::ack`] channel before
/// closing the substream. Short — building a `ParticipantInfo` is
/// reading a few `Arc<Mutex<...>>` fields and constructing a JSON
/// string; >2 s means something is wrong with the handler.
const INFO_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long [`NetworkRuntime::request_participant_info`] waits for
/// the full open-write-read round-trip before returning a timeout
/// error.
pub const INFO_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors from [`NetworkRuntime::request_participant_info`].
#[derive(Debug, thiserror::Error)]
pub enum RequestInfoError {
    /// `libp2p_stream::Control::open_stream` failed (peer not
    /// reachable, not on the allow-list, etc.).
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    /// I/O or wire-format error reading/writing a framed message.
    #[error("protocol: {0}")]
    Protocol(#[source] InfoProtocolError),
    /// The round-trip didn't complete within
    /// [`INFO_REQUEST_TIMEOUT`].
    #[error("info request timed out after {0:?}")]
    Timeout(Duration),
}

/// Inbound `/auki/resources/0.0.1` event surfaced by the runtime to
/// its owner via the channel returned from [`NetworkRuntime::spawn`].
///
/// The owner (typically `auki-domain`'s `ClusterManager`) snapshots
/// the application-supplied resource catalog and replies via `ack`.
/// The runtime's per-substream task awaits the reply for up to
/// [`RESOURCES_RESPONSE_TIMEOUT`] before closing the substream
/// silently.
#[derive(Debug)]
pub struct ResourcesRequestEvent {
    /// The peer-id of the requester. Authenticated by libp2p's noise
    /// handshake at connection-establishment time.
    pub peer: PeerId,
    /// The body of the request.
    pub request: ResourcesRequest,
    /// One-shot channel to reply on. Send a [`ResourcesResponse`]
    /// containing the producer's current resource catalog snapshot.
    /// Dropping the sender without sending closes the substream
    /// silently; the requester sees a
    /// [`ResourcesProtocolError::Io`] with `UnexpectedEof`.
    pub ack: oneshot::Sender<ResourcesResponse>,
}

/// How long the runtime's per-substream resources task waits for the
/// owner to reply via the [`ResourcesRequestEvent::ack`] channel
/// before closing the substream.
const RESOURCES_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long [`NetworkRuntime::request_resources_catalog`] waits for
/// the full open-write-read round-trip before returning a timeout
/// error.
pub const RESOURCES_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Live source for `/auki/resources/0.4.0` Map Log rows.
pub trait MapCatalogProvider: Send + Sync {
    fn map_catalog(&self) -> ResourcesResponseV4;
}

type SharedMapCatalogProvider = Arc<Mutex<Option<Arc<dyn MapCatalogProvider>>>>;

/// Live source for `/auki/resources/0.5.0` Device Model rows.
pub trait DeviceModelCatalogProvider: Send + Sync {
    fn device_model_catalog(&self) -> ResourcesResponseV5;
}

type SharedDeviceModelCatalogProvider = Arc<Mutex<Option<Arc<dyn DeviceModelCatalogProvider>>>>;
pub type SharedBlobAppRoot = Arc<Mutex<Option<PathBuf>>>;

/// Errors from [`NetworkRuntime::request_resources_catalog`].
#[derive(Debug, thiserror::Error)]
pub enum RequestResourcesError {
    /// `libp2p_stream::Control::open_stream` failed (peer not
    /// reachable, not on the allow-list, etc.).
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    /// I/O or wire-format error reading/writing a framed message.
    #[error("protocol: {0}")]
    Protocol(#[source] ResourcesProtocolError),
    /// The round-trip didn't complete within
    /// [`RESOURCES_REQUEST_TIMEOUT`].
    #[error("resources request timed out after {0:?}")]
    Timeout(Duration),
}

#[derive(Debug, thiserror::Error)]
pub enum RequestResourcesV3Error {
    /// The remote peer does not implement Resource Catalog v0.3.
    #[error("remote peer does not support Resource Catalog v0.3")]
    UnsupportedProtocol,
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    #[error("protocol: {0}")]
    Protocol(#[source] ResourcesV3ProtocolError),
    #[error("resources v3 request timed out after {0:?}")]
    Timeout(Duration),
}

fn map_resources_v3_open_error(error: libp2p_stream::OpenStreamError) -> RequestResourcesV3Error {
    match error {
        libp2p_stream::OpenStreamError::UnsupportedProtocol(_) => {
            RequestResourcesV3Error::UnsupportedProtocol
        }
        error => RequestResourcesV3Error::OpenStream(error),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RequestResourcesV4Error {
    #[error("remote peer does not support Resource Catalog v0.4")]
    UnsupportedProtocol,
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    #[error("protocol: {0}")]
    Protocol(#[source] ResourcesV4ProtocolError),
    #[error("resources v4 request timed out after {0:?}")]
    Timeout(Duration),
}

#[derive(Debug, thiserror::Error)]
pub enum RequestResourcesV5Error {
    #[error("remote peer does not support Resource Catalog v0.5")]
    UnsupportedProtocol,
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    #[error("protocol: {0}")]
    Protocol(#[source] ResourcesV5ProtocolError),
    #[error("resources v5 request timed out after {0:?}")]
    Timeout(Duration),
}

fn map_resources_v5_open_error(error: libp2p_stream::OpenStreamError) -> RequestResourcesV5Error {
    match error {
        libp2p_stream::OpenStreamError::UnsupportedProtocol(_) => RequestResourcesV5Error::UnsupportedProtocol,
        error => RequestResourcesV5Error::OpenStream(error),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RequestBlobError {
    #[error("remote peer does not support blob transfer")]
    UnsupportedProtocol,
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    #[error("protocol: {0}")]
    Protocol(#[source] BlobsProtocolError),
    #[error("blob request timed out after {0:?}")]
    Timeout(Duration),
    #[error("blob not found")]
    NotFound,
    #[error("invalid remote blob response: {0}")]
    InvalidResponse(String),
}

fn map_resources_v4_open_error(error: libp2p_stream::OpenStreamError) -> RequestResourcesV4Error {
    match error {
        libp2p_stream::OpenStreamError::UnsupportedProtocol(_) => {
            RequestResourcesV4Error::UnsupportedProtocol
        }
        error => RequestResourcesV4Error::OpenStream(error),
    }
}

/// Inbound `/auki/registries/0.0.1` event surfaced by the runtime to
/// its owner via the channel returned from [`NetworkRuntime::spawn`].
///
/// The owner (typically `auki-domain`'s `ClusterManager`) resolves the
/// requested registry entry from producer-local storage and replies
/// via `ack`. The runtime's per-substream task awaits the reply for
/// up to [`REGISTRIES_RESPONSE_TIMEOUT`] before closing the substream
/// silently.
#[derive(Debug)]
pub struct RegistryRequestEvent {
    /// The peer-id of the requester. Authenticated by libp2p's noise
    /// handshake at connection-establishment time.
    pub peer: PeerId,
    /// Requested registry kind + id + hash.
    pub request: RegistryRequest,
    /// One-shot channel to reply on. Send a [`RegistryResponse`]
    /// containing either the canonical JSON entry or `None` when this
    /// peer does not have the exact `(kind, id, hash)` entry.
    /// Dropping the sender without sending closes the substream
    /// silently — the requester sees a
    /// [`RegistriesProtocolError::Io`] with `UnexpectedEof`.
    pub ack: oneshot::Sender<RegistryResponse>,
}

/// How long the runtime's per-substream registries task waits for the
/// owner to reply via the [`RegistryRequestEvent::ack`] channel before
/// closing the substream.
const REGISTRIES_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long [`NetworkRuntime::request_registry_entry`] waits for the
/// full open-write-read round-trip before returning a timeout error.
pub const REGISTRIES_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors from [`NetworkRuntime::request_registry_entry`].
#[derive(Debug, thiserror::Error)]
pub enum RequestRegistryError {
    /// `libp2p_stream::Control::open_stream` failed (peer not
    /// reachable, not on the allow-list, etc.).
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    /// I/O or wire-format error reading/writing a framed message.
    #[error("protocol: {0}")]
    Protocol(#[source] RegistriesProtocolError),
    /// The round-trip didn't complete within
    /// [`REGISTRIES_REQUEST_TIMEOUT`].
    #[error("registry request timed out after {0:?}")]
    Timeout(Duration),
}

/// Inbound `/auki/membership/0.0.1` event surfaced by the runtime to
/// its owner via the channel returned from [`NetworkRuntime::spawn`].
///
/// The owner (typically `auki-domain`'s `ClusterManager`) parses the
/// `membership_json`, swaps its local membership document, and pushes
/// the updated allow-list to the runtime via
/// [`NetworkRuntime::set_allowed_peers`]. Fire-and-forget — no `ack`
/// channel; receivers apply the gossip silently.
#[derive(Debug)]
pub struct MembershipEvent {
    /// The peer-id of the sender. Authenticated by libp2p's noise
    /// handshake; should be the cluster's current Manager.
    pub peer: PeerId,
    /// The body of the membership update — a serialized
    /// `auki_domain::ClusterMembership` JSON string.
    pub update: MembershipUpdate,
}

/// Inbound `/auki/diagnostic/0.0.1` event surfaced by the runtime.
#[derive(Debug)]
pub struct DiagnosticEvent {
    pub peer: PeerId,
    pub message: DiagnosticMessage,
}

/// Errors from [`NetworkRuntime::broadcast_membership`] /
/// [`NetworkRuntimeHandle::broadcast_membership`]. Per-peer write
/// failures are logged (not collected into this error) since gossip
/// is fire-and-forget — the next gossip will reconverge.
#[derive(Debug, thiserror::Error)]
pub enum BroadcastMembershipError {
    /// `membership_json` exceeded the protocol's frame cap. Indicates
    /// an unreasonably large membership document.
    #[error("membership_json is too large for the gossip frame")]
    PayloadTooLarge,
}

/// Errors from diagnostic message broadcast.
#[derive(Debug, thiserror::Error)]
pub enum BroadcastDiagnosticError {
    #[error("diagnostic message is too large for the frame")]
    PayloadTooLarge,
}

/// Diff applied by [`NetworkRuntime::set_allowed_peers`].
///
/// `added` lists peer-ids in the new list but not the old — the runtime
/// has scheduled them for dialing (if they carry addresses). `removed`
/// lists peer-ids in the old list but not the new — the runtime has
/// dropped their connections and removed them from the allow-list.
/// Peers in both keep their existing connection; their addresses are
/// refreshed for future redials.
///
/// `swift-bindings`: UniFFI Record. All fields cross the FFI via the
/// custom-type registrations in scope (PeerId → String).
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone)]
pub struct UpdateReport {
    /// Peer-ids newly added to the allow-list.
    pub added: Vec<PeerId>,
    /// Peer-ids removed from the allow-list (and disconnected).
    pub removed: Vec<PeerId>,
}

/// Errors from [`NetworkRuntime::set_allowed_peers`] /
/// [`NetworkRuntime::set_heartbeat_targets`].
///
/// `swift-bindings`: flattened — variants that wrap non-FFI inner
/// errors are surfaced as Display'd strings; UniFFI consumers see one
/// tagged-enum case per variant with a `message: String` field where
/// the wrapped error was.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "swift-bindings", uniffi(flat_error))]
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The runtime task isn't accepting commands — typically because
    /// the runtime has shut down or is shutting down.
    #[error("runtime shutting down")]
    RuntimeUnavailable,
}

/// Errors from reserving a relay-mediated listener through a running
/// [`NetworkRuntime`].
#[derive(Debug, thiserror::Error)]
pub enum RuntimeRelayReservationError {
    /// The runtime task isn't accepting commands — typically because
    /// the runtime has shut down or is shutting down.
    #[error("runtime shutting down")]
    RuntimeUnavailable,
    /// The underlying relay reservation failed.
    #[error(transparent)]
    Reservation(#[from] swarm::RelayReservationError),
}

/// Internal command from public methods to the driver task.
enum RuntimeCmd {
    SetAllowedPeers {
        new_peers: Vec<AllowedPeer>,
        ack: oneshot::Sender<Result<UpdateReport, UpdateError>>,
    },
    SetHeartbeatTargets {
        peers: Vec<PeerId>,
        ack: oneshot::Sender<Result<(), UpdateError>>,
    },
    ReserveRelayCircuitAddr {
        relay_dial_multiaddr: Multiaddr,
        relay_advertise_multiaddr: Multiaddr,
        timeout: Duration,
        ack: oneshot::Sender<Result<Multiaddr, RuntimeRelayReservationError>>,
    },
}

enum PendingMessageDriverWork {
    Command(RuntimeCmd),
    Completion { peer_id: PeerId, task_id: u64 },
}

fn take_pending_runtime_shutdown(shutdown_rx: &mut oneshot::Receiver<()>) -> bool {
    match shutdown_rx.try_recv() {
        Ok(()) | Err(oneshot::error::TryRecvError::Closed) => true,
        Err(oneshot::error::TryRecvError::Empty) => false,
    }
}

fn take_pending_message_driver_work(
    command_rx: &mut mpsc::Receiver<RuntimeCmd>,
    message_task_done_rx: &mut mpsc::UnboundedReceiver<(PeerId, u64)>,
) -> Option<PendingMessageDriverWork> {
    if let Ok(command) = command_rx.try_recv() {
        return Some(PendingMessageDriverWork::Command(command));
    }
    message_task_done_rx
        .try_recv()
        .ok()
        .map(|(peer_id, task_id)| PendingMessageDriverWork::Completion { peer_id, task_id })
}

/// Per-peer dial scheduling state.
struct PeerSchedule {
    next_dial_at: Option<Instant>,
    backoff: Duration,
}

const MAX_ACTIVE_MESSAGE_STREAMS: usize = 128;
const MAX_ACTIVE_MESSAGE_STREAMS_PER_PEER: usize = 8;

struct ActiveMessageTask {
    revoke: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct ActiveMessageTasks {
    next_id: u64,
    by_peer: HashMap<PeerId, HashMap<u64, ActiveMessageTask>>,
}

impl ActiveMessageTasks {
    fn len(&self) -> usize {
        self.by_peer.values().map(HashMap::len).sum()
    }

    fn has_capacity(&self, peer: PeerId) -> bool {
        self.len() < MAX_ACTIVE_MESSAGE_STREAMS
            && self.by_peer.get(&peer).map_or(0, HashMap::len) < MAX_ACTIVE_MESSAGE_STREAMS_PER_PEER
    }

    fn allocate_id(&mut self, peer: PeerId) -> Option<u64> {
        if !self.has_capacity(peer) {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        Some(id)
    }

    fn track(
        &mut self,
        peer: PeerId,
        id: u64,
        revoke: watch::Sender<bool>,
        handle: JoinHandle<()>,
    ) {
        self.by_peer
            .entry(peer)
            .or_default()
            .insert(id, ActiveMessageTask { revoke, handle });
    }

    async fn complete(&mut self, peer: PeerId, id: u64) {
        let Some(tasks) = self.by_peer.get_mut(&peer) else {
            return;
        };
        let task = tasks.remove(&id);
        if tasks.is_empty() {
            self.by_peer.remove(&peer);
        }
        if let Some(task) = task {
            let _ = task.handle.await;
        }
    }

    async fn cancel_peer(&mut self, peer: PeerId) {
        let Some(tasks) = self.by_peer.remove(&peer) else {
            return;
        };
        for task in tasks.values() {
            let _ = task.revoke.send(true);
        }
        for task in tasks.values() {
            task.handle.abort();
        }
        for (_, task) in tasks {
            let _ = task.handle.await;
        }
    }

    async fn cancel_all(&mut self) {
        let peers: Vec<PeerId> = self.by_peer.keys().copied().collect();
        for peer in peers {
            self.cancel_peer(peer).await;
        }
    }
}

impl Drop for ActiveMessageTasks {
    fn drop(&mut self) {
        for tasks in self.by_peer.values() {
            for task in tasks.values() {
                let _ = task.revoke.send(true);
                task.handle.abort();
            }
        }
    }
}

fn authorize_inbound_message_stream<S>(is_known_peer: bool, stream: S) -> Option<S> {
    is_known_peer.then_some(stream)
}

/// Drives a libp2p swarm against the allow-list set, auto-dialing
/// peers with known addresses, accepting inbound substreams on
/// `/auki/stream/0.1.0` (handed off to the `stream_provider`),
/// reconnecting on disconnect with exponential backoff. See the
/// module-level docs for the design rationale.
///
/// `swift-bindings`: derived as a UniFFI Object. The curated FFI
/// surface for v0: `local_peer_id_string`, `connected_peer_id_strings`,
/// `set_allowed_peers`, `shutdown`, and the 5 `open_*_stream` methods
/// added in Tasks 14-18.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct NetworkRuntime {
    local_peer_id: PeerId,
    connected: Arc<Mutex<HashSet<PeerId>>>,
    /// Driver-task teardown channel. Wrapped in `Mutex<Option<_>>`
    /// so [`Self::shutdown`] can `.take()` from `&self` and remain
    /// idempotent — second caller observes `None` and no-ops.
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    /// Handle to the driver task. Same `Mutex<Option<_>>` shape as
    /// `shutdown_tx`: `.take()` aborts under a shared reference.
    task: Mutex<Option<JoinHandle<()>>>,
    /// Cloneable handle to the swarm's [`libp2p_stream::Behaviour`].
    /// Used by [`crate::stream_runtime`]'s `open_stream` (consumer
    /// side) to open outbound substreams on `/auki/stream/0.1.0`.
    stream_control: Control,
    /// Watch channel signalling per-substream inbound tasks to flush
    /// a final `EndOfStream { reason: ProducerShuttingDown }` and
    /// exit. Sent to by [`Self::shutdown`] before the swarm teardown
    /// signal.
    inbound_shutdown_tx: watch::Sender<bool>,
    /// Lifeline channel tied to this runtime's lifetime. Helper tasks
    /// (heartbeat opener / pair) hold `subscribe()`d
    /// `Receiver`s and `select!` on `Receiver::changed()`. When this
    /// `Sender` drops with the `NetworkRuntime`, every receiver sees
    /// the channel closed and the helper tasks exit. Closes the
    /// "QUIC-detached-heartbeat" leak — without the lifeline, those
    /// tasks keep their Arc-wrapped substream handles alive across the
    /// runtime's `task.abort()`, so they continue writing heartbeat
    /// frames against a transport that hasn't actually closed at the
    /// QUIC layer (the swarm drop on the local side doesn't translate
    /// to a `CONNECTION_CLOSE` frame on the remote until the QUIC
    /// idle timer fires, tens of seconds later). The lifeline binds
    /// helper-task lifetime to the runtime's lifetime explicitly.
    _lifeline_tx: watch::Sender<()>,
    /// Command channel from public methods to the driver task.
    command_tx: mpsc::Sender<RuntimeCmd>,
    /// Receiver-owned live message channels registered on this peer.
    message_router: MessageChannelRouter,
    map_catalog_provider: SharedMapCatalogProvider,
    device_model_catalog_provider: SharedDeviceModelCatalogProvider,
    blob_app_root: SharedBlobAppRoot,
}

/// Cloneable handle to a [`NetworkRuntime`] for command-style
/// operations (`set_allowed_peers`, `set_heartbeat_targets`,
/// `connected_peers`, `broadcast_membership`). Lets `auki-domain`'s
/// background tasks call back into the runtime without holding the
/// `NetworkRuntime` itself.
#[derive(Clone)]
pub struct NetworkRuntimeHandle {
    command_tx: mpsc::Sender<RuntimeCmd>,
    connected: Arc<Mutex<HashSet<PeerId>>>,
    /// Cloneable libp2p-stream `Control`. The handle holds this so
    /// outbound substream opens (`broadcast_membership`, `send_join_request`) work without
    /// reaching back into the owning `NetworkRuntime`.
    stream_control: Control,
}

impl NetworkRuntimeHandle {
    /// Same semantics as [`NetworkRuntime::set_allowed_peers`].
    pub async fn set_allowed_peers(
        &self,
        new_peers: Vec<AllowedPeer>,
    ) -> Result<UpdateReport, UpdateError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCmd::SetAllowedPeers {
                new_peers,
                ack: ack_tx,
            })
            .await
            .map_err(|_| UpdateError::RuntimeUnavailable)?;
        ack_rx.await.map_err(|_| UpdateError::RuntimeUnavailable)?
    }

    /// Same semantics as [`NetworkRuntime::set_heartbeat_targets`].
    pub async fn set_heartbeat_targets(&self, peers: Vec<PeerId>) -> Result<(), UpdateError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCmd::SetHeartbeatTargets { peers, ack: ack_tx })
            .await
            .map_err(|_| UpdateError::RuntimeUnavailable)?;
        ack_rx.await.map_err(|_| UpdateError::RuntimeUnavailable)?
    }

    /// Reserve a Circuit Relay v2 listener through a relay from the
    /// runtime-owned swarm and return the Manager circuit address to
    /// publish. See
    /// [`swarm::reserve_relay_circuit_addr_with_advertised_addr`] for
    /// dial-vs-advertise semantics.
    pub async fn reserve_relay_circuit_addr(
        &self,
        relay_dial_multiaddr: Multiaddr,
        relay_advertise_multiaddr: Multiaddr,
        timeout: Duration,
    ) -> Result<Multiaddr, RuntimeRelayReservationError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCmd::ReserveRelayCircuitAddr {
                relay_dial_multiaddr,
                relay_advertise_multiaddr,
                timeout,
                ack: ack_tx,
            })
            .await
            .map_err(|_| RuntimeRelayReservationError::RuntimeUnavailable)?;
        ack_rx
            .await
            .map_err(|_| RuntimeRelayReservationError::RuntimeUnavailable)?
    }

    /// Same semantics as [`NetworkRuntime::connected_peers`].
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.connected
            .lock()
            .expect("connected set mutex poisoned")
            .iter()
            .copied()
            .collect()
    }

    /// Same semantics as [`NetworkRuntime::broadcast_membership`].
    pub fn broadcast_membership(
        &self,
        manager_peer_id: PeerId,
        membership_json: String,
    ) -> Result<(), BroadcastMembershipError> {
        broadcast_membership_impl(
            &self.stream_control,
            &self.connected,
            manager_peer_id,
            membership_json,
        )
    }

    /// Same semantics as [`NetworkRuntime::broadcast_diagnostic_message`].
    pub fn broadcast_diagnostic_message(
        &self,
        message: DiagnosticMessage,
    ) -> Result<(), BroadcastDiagnosticError> {
        broadcast_diagnostic_impl(&self.stream_control, &self.connected, message)
    }

    /// Same semantics as [`NetworkRuntime::send_join_request`]: open an
    /// outbound `/auki/join/0.0.1` substream to `peer_id`, write the
    /// request, read the response. The peer must be on the local
    /// allow-list — callers re-joining an evicting Manager add it back
    /// via `set_allowed_peers` first.
    pub async fn send_join_request(
        &self,
        peer_id: PeerId,
        request: JoinRequest,
    ) -> Result<JoinResponse, SendJoinRequestError> {
        send_join_request_impl(self.stream_control.clone(), peer_id, request).await
    }
}

impl NetworkRuntime {
    /// Cloneable [`Control`] handle for outbound stream opens.
    /// Internal — `stream_runtime::open_stream` uses it; external
    /// callers go through `open_stream` itself.
    pub(crate) fn stream_control(&self) -> &Control {
        &self.stream_control
    }
}

impl NetworkRuntime {
    /// Construct a runtime around `swarm`. The swarm's keypair (and
    /// therefore its `PeerId`) becomes the runtime's local identity.
    /// The swarm should already be listening on its configured
    /// addresses.
    ///
    /// `allowed_peers` is the initial cluster trust boundary — only
    /// these peer-ids will complete libp2p handshakes inbound or
    /// outbound. Peers with at least one multiaddr are scheduled for
    /// an immediate dial; address-less entries are accepted as trusted
    /// (the runtime will respond if they dial us) but not auto-dialed.
    ///
    /// Returns the runtime + a receiver for inbound
    /// `/auki/join/0.0.1` events + a receiver for peer/heartbeat
    /// carrier events + a receiver for inbound `/auki/membership/0.0.1`
    /// gossip events + a receiver for inbound `/auki/info/0.0.1`
    /// participant-info requests + a receiver for inbound
    /// `/auki/resources/0.2.0` resource-catalog requests + a receiver for
    /// inbound `/auki/registries/0.0.1` registry-entry requests.
    /// Owners that don't care about any of them (e.g. tests) can drop
    /// the receivers; the runtime drops events with no receiver.
    #[allow(clippy::type_complexity)]
    pub fn spawn(
        swarm: Swarm<Behaviour>,
        allowed_peers: Vec<AllowedPeer>,
        stream_provider: StreamProvider,
        heartbeat_timestamps: HeartbeatTimestampSource,
    ) -> Result<
        (
            Self,
            mpsc::Receiver<JoinEvent>,
            mpsc::Receiver<PeerLivenessEvent>,
            mpsc::Receiver<MembershipEvent>,
            mpsc::Receiver<InfoRequestEvent>,
            mpsc::Receiver<ResourcesRequestEvent>,
            mpsc::Receiver<RegistryRequestEvent>,
            mpsc::Receiver<DiagnosticEvent>,
        ),
        SpawnError,
    > {
        let handle =
            tokio::runtime::Handle::try_current().map_err(|_| SpawnError::NoTokioRuntime)?;
        let local_peer_id = *swarm.local_peer_id();
        let connected = Arc::new(Mutex::new(HashSet::new()));
        let outbound_control = swarm.behaviour().stream.new_control();
        let inbound_control = outbound_control.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (inbound_shutdown_tx, inbound_shutdown_rx) = watch::channel(false);
        let (lifeline_tx, lifeline_rx) = watch::channel(());
        let (command_tx, command_rx) = mpsc::channel::<RuntimeCmd>(16);
        let (join_events_tx, join_events_rx) = mpsc::channel::<JoinEvent>(16);
        let (liveness_tx, liveness_rx) = mpsc::channel::<PeerLivenessEvent>(64);
        let (membership_events_tx, membership_events_rx) = mpsc::channel::<MembershipEvent>(16);
        let (info_events_tx, info_events_rx) = mpsc::channel::<InfoRequestEvent>(16);
        let (resources_events_tx, resources_events_rx) = mpsc::channel::<ResourcesRequestEvent>(16);
        let (registry_events_tx, registry_events_rx) = mpsc::channel::<RegistryRequestEvent>(16);
        let (diagnostic_events_tx, diagnostic_events_rx) = mpsc::channel::<DiagnosticEvent>(64);
        let message_router = MessageChannelRouter::new(local_peer_id);
        let map_catalog_provider: SharedMapCatalogProvider = Arc::new(Mutex::new(None));
        let device_model_catalog_provider: SharedDeviceModelCatalogProvider = Arc::new(Mutex::new(None));
        let blob_app_root: SharedBlobAppRoot = Arc::new(Mutex::new(None));
        let task = handle.spawn(run_task(
            swarm,
            allowed_peers,
            connected.clone(),
            stream_provider,
            heartbeat_timestamps,
            inbound_control,
            inbound_shutdown_rx,
            lifeline_rx,
            shutdown_rx,
            command_rx,
            join_events_tx,
            liveness_tx,
            membership_events_tx,
            info_events_tx,
            resources_events_tx,
            registry_events_tx,
            diagnostic_events_tx,
            message_router.clone(),
            map_catalog_provider.clone(),
            device_model_catalog_provider.clone(),
            blob_app_root.clone(),
        ));
        Ok((
            Self {
                local_peer_id,
                connected,
                shutdown_tx: Mutex::new(Some(shutdown_tx)),
                task: Mutex::new(Some(task)),
                stream_control: outbound_control,
                inbound_shutdown_tx,
                _lifeline_tx: lifeline_tx,
                command_tx,
                message_router,
                map_catalog_provider,
                device_model_catalog_provider,
                blob_app_root,
            },
            join_events_rx,
            liveness_rx,
            membership_events_rx,
            info_events_rx,
            resources_events_rx,
            registry_events_rx,
            diagnostic_events_rx,
        ))
    }

    /// Open an outbound `/auki/join/0.0.1` substream to `peer_id`,
    /// write the request, read the response. Returns once the
    /// full round-trip completes (or fails).
    ///
    /// The peer must be on the local allow-list (`set_allowed_peers`
    /// or the initial `allowed_peers` argument to `spawn`) — libp2p
    /// refuses the noise handshake otherwise. Bootstrap case (first
    /// peer of a cluster joining the Manager): the caller pre-allows
    /// the Manager's peer-id before calling this.
    pub async fn send_join_request(
        &self,
        peer_id: PeerId,
        request: JoinRequest,
    ) -> Result<JoinResponse, SendJoinRequestError> {
        send_join_request_impl(self.stream_control.clone(), peer_id, request).await
    }

    /// Fetch a cluster peer's [`crate::ParticipantInfo`] over the
    /// `/auki/info/0.0.1` libp2p protocol. Returns the response's
    /// serialized JSON; callers deserialize via `serde_json` (the
    /// shape is `auki_network::ParticipantInfo`).
    ///
    /// `peer_id` must be on the local allow-list — libp2p refuses
    /// the substream otherwise. Daemons typically call this against
    /// every entry in their `ClusterMembership` to populate
    /// `/api/cluster/peers` / their own directory views.
    ///
    /// The full open-write-read round-trip is bounded by
    /// [`INFO_REQUEST_TIMEOUT`] (5 s — well above LAN round-trip,
    /// well below any operator-perceptible UI hang).
    pub async fn request_participant_info(
        &self,
        peer_id: PeerId,
    ) -> Result<InfoResponse, RequestInfoError> {
        let mut control = self.stream_control.clone();
        let proto = StreamProtocol::try_from_owned(INFO_PROTOCOL.to_string())
            .expect("INFO_PROTOCOL is a valid libp2p protocol id");

        let open_fut = control.open_stream(peer_id, proto);
        let mut substream = match tokio::time::timeout(INFO_REQUEST_TIMEOUT, open_fut).await {
            Err(_) => return Err(RequestInfoError::Timeout(INFO_REQUEST_TIMEOUT)),
            Ok(Err(e)) => return Err(RequestInfoError::OpenStream(e)),
            Ok(Ok(s)) => s,
        };

        write_info_request(&mut substream, &InfoRequest::default())
            .await
            .map_err(RequestInfoError::Protocol)?;

        let response =
            match tokio::time::timeout(INFO_REQUEST_TIMEOUT, read_info_response(&mut substream))
                .await
            {
                Err(_) => return Err(RequestInfoError::Timeout(INFO_REQUEST_TIMEOUT)),
                Ok(Err(e)) => return Err(RequestInfoError::Protocol(e)),
                Ok(Ok(r)) => r,
            };
        Ok(response)
    }

    /// Fetch a cluster peer's current resource catalog over the
    /// `/auki/resources/0.2.0` libp2p protocol. Returns the response's
    /// list of resource rows.
    pub async fn request_resources_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<ResourcesResponse, RequestResourcesError> {
        self.request_resources_catalog_with(peer_id, ResourcesRequest::all())
            .await
    }

    /// Fetch a cluster peer's current resource catalog using an
    /// explicit [`ResourcesRequest`]. This is the canonical discovery
    /// path for sensor streams, transform edges, and future resource
    /// kinds.
    pub async fn request_resources_catalog_with(
        &self,
        peer_id: PeerId,
        request: ResourcesRequest,
    ) -> Result<ResourcesResponse, RequestResourcesError> {
        let mut control = self.stream_control.clone();
        let proto = RESOURCES_PROTOCOL.clone();

        let open_fut = control.open_stream(peer_id, proto);
        let mut substream = match tokio::time::timeout(RESOURCES_REQUEST_TIMEOUT, open_fut).await {
            Err(_) => return Err(RequestResourcesError::Timeout(RESOURCES_REQUEST_TIMEOUT)),
            Ok(Err(e)) => return Err(RequestResourcesError::OpenStream(e)),
            Ok(Ok(s)) => s,
        };

        write_resources_request(&mut substream, &request)
            .await
            .map_err(RequestResourcesError::Protocol)?;

        let response = match tokio::time::timeout(
            RESOURCES_REQUEST_TIMEOUT,
            read_resources_response(&mut substream),
        )
        .await
        {
            Err(_) => return Err(RequestResourcesError::Timeout(RESOURCES_REQUEST_TIMEOUT)),
            Ok(Err(e)) => return Err(RequestResourcesError::Protocol(e)),
            Ok(Ok(r)) => r,
        };
        Ok(response)
    }

    pub async fn request_resources_catalog_v3_with(
        &self,
        peer_id: PeerId,
        request: ResourcesRequestV3,
    ) -> Result<ResourcesResponseV3, RequestResourcesV3Error> {
        let mut control = self.stream_control.clone();
        let open_fut = control.open_stream(peer_id, RESOURCES_V3_PROTOCOL.clone());
        let mut substream = match tokio::time::timeout(RESOURCES_REQUEST_TIMEOUT, open_fut).await {
            Err(_) => return Err(RequestResourcesV3Error::Timeout(RESOURCES_REQUEST_TIMEOUT)),
            Ok(Err(error)) => return Err(map_resources_v3_open_error(error)),
            Ok(Ok(substream)) => substream,
        };
        write_resources_request_v3(&mut substream, &request)
            .await
            .map_err(RequestResourcesV3Error::Protocol)?;
        tokio::time::timeout(
            RESOURCES_REQUEST_TIMEOUT,
            read_resources_response_v3(&mut substream),
        )
        .await
        .map_err(|_| RequestResourcesV3Error::Timeout(RESOURCES_REQUEST_TIMEOUT))?
        .map_err(RequestResourcesV3Error::Protocol)
    }

    /// Install the live source used to answer inbound Resource Catalog v0.4
    /// requests. Replacing the provider is atomic from a request's point of
    /// view; requests made before a provider is installed receive an empty
    /// catalog.
    pub fn set_map_catalog_provider(&self, provider: Arc<dyn MapCatalogProvider>) {
        *self
            .map_catalog_provider
            .lock()
            .expect("map_catalog_provider lock") = Some(provider);
    }

    pub fn set_device_model_catalog_provider(&self, provider: Arc<dyn DeviceModelCatalogProvider>) {
        *self
            .device_model_catalog_provider
            .lock()
            .expect("device_model_catalog_provider lock") = Some(provider);
    }

    pub fn set_blob_app_root(&self, app_root: impl Into<PathBuf>) {
        *self.blob_app_root.lock().expect("blob_app_root lock") = Some(app_root.into());
    }

    /// Fetch a cluster peer's current Map Log catalog over
    /// `/auki/resources/0.4.0`.
    pub async fn request_map_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<ResourcesResponseV4, RequestResourcesV4Error> {
        let mut control = self.stream_control.clone();
        let open_fut = control.open_stream(peer_id, RESOURCES_V4_PROTOCOL.clone());
        let mut substream = match tokio::time::timeout(RESOURCES_REQUEST_TIMEOUT, open_fut).await {
            Err(_) => return Err(RequestResourcesV4Error::Timeout(RESOURCES_REQUEST_TIMEOUT)),
            Ok(Err(error)) => return Err(map_resources_v4_open_error(error)),
            Ok(Ok(substream)) => substream,
        };

        write_resources_request_v4(&mut substream, &ResourcesRequestV4::all())
            .await
            .map_err(RequestResourcesV4Error::Protocol)?;

        tokio::time::timeout(
            RESOURCES_REQUEST_TIMEOUT,
            read_resources_response_v4(&mut substream),
        )
        .await
        .map_err(|_| RequestResourcesV4Error::Timeout(RESOURCES_REQUEST_TIMEOUT))?
        .map_err(RequestResourcesV4Error::Protocol)
    }

    pub async fn request_device_model_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<ResourcesResponseV5, RequestResourcesV5Error> {
        let mut control = self.stream_control.clone();
        let mut substream = match tokio::time::timeout(
            RESOURCES_REQUEST_TIMEOUT,
            control.open_stream(peer_id, RESOURCES_V5_PROTOCOL.clone()),
        ).await {
            Err(_) => return Err(RequestResourcesV5Error::Timeout(RESOURCES_REQUEST_TIMEOUT)),
            Ok(Err(error)) => return Err(map_resources_v5_open_error(error)),
            Ok(Ok(stream)) => stream,
        };
        write_resources_request_v5(&mut substream, &ResourcesRequestV5::all())
            .await
            .map_err(RequestResourcesV5Error::Protocol)?;
        tokio::time::timeout(RESOURCES_REQUEST_TIMEOUT, read_resources_response_v5(&mut substream))
            .await
            .map_err(|_| RequestResourcesV5Error::Timeout(RESOURCES_REQUEST_TIMEOUT))?
            .map_err(RequestResourcesV5Error::Protocol)
    }

    pub async fn request_blob_chunk(
        &self,
        peer_id: PeerId,
        request: BlobRequest,
    ) -> Result<(BlobResponseMeta, Vec<u8>), RequestBlobError> {
        let mut control = self.stream_control.clone();
        let mut substream = match tokio::time::timeout(
            RESOURCES_REQUEST_TIMEOUT,
            control.open_stream(peer_id, BLOBS_PROTOCOL.clone()),
        ).await {
            Err(_) => return Err(RequestBlobError::Timeout(RESOURCES_REQUEST_TIMEOUT)),
            Ok(Err(libp2p_stream::OpenStreamError::UnsupportedProtocol(_))) => return Err(RequestBlobError::UnsupportedProtocol),
            Ok(Err(error)) => return Err(RequestBlobError::OpenStream(error)),
            Ok(Ok(stream)) => stream,
        };
        write_blob_request(&mut substream, &request).await.map_err(RequestBlobError::Protocol)?;
        let (meta, chunk) = tokio::time::timeout(RESOURCES_REQUEST_TIMEOUT, read_blob_response(&mut substream))
            .await
            .map_err(|_| RequestBlobError::Timeout(RESOURCES_REQUEST_TIMEOUT))?
            .map_err(RequestBlobError::Protocol)?;
        if meta.sha256 != request.sha256 || meta.offset != request.offset {
            return Err(RequestBlobError::InvalidResponse("response address or offset differs from request".into()));
        }
        Ok((meta, chunk))
    }

    pub async fn request_blob(&self, peer_id: PeerId, sha256: String) -> Result<Vec<u8>, RequestBlobError> {
        let mut bytes = Vec::new();
        let mut offset = 0;
        loop {
            let (meta, chunk) = self.request_blob_chunk(peer_id, BlobRequest {
                sha256: sha256.clone(),
                offset,
                max_len: crate::blobs_protocol::MAX_BLOB_CHUNK_BYTES,
            }).await?;
            if meta.total_size == 0 && meta.chunk_len == 0 {
                return Err(RequestBlobError::NotFound);
            }
            if meta.total_size > crate::blobs_protocol::MAX_BLOB_BYTES || chunk.is_empty() {
                return Err(RequestBlobError::InvalidResponse("invalid blob chunk".into()));
            }
            bytes.extend_from_slice(&chunk);
            offset += chunk.len() as u64;
            if offset == meta.total_size { break; }
            if offset > meta.total_size {
                return Err(RequestBlobError::InvalidResponse("chunk exceeds advertised size".into()));
            }
        }
        if auki_registry::sha256_hex(&bytes) != sha256 {
            return Err(RequestBlobError::InvalidResponse("assembled bytes fail SHA-256 verification".into()));
        }
        Ok(bytes)
    }

    /// Atomically register a receiver-owned live message channel and its
    /// Resource Catalog row. The row owner must be this runtime's peer id.
    pub fn register_message_channel(
        &self,
        resource: crate::resources_v3_protocol::MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<MessageChannelRegistration, RegistrationError> {
        self.message_router.register(resource, receiver_capacity)
    }

    /// Open one exact receiver-owned `/auki/message/0.1.0` channel.
    /// The returned sender is live-only and never retries or replays.
    pub async fn open_message_channel(
        &self,
        owner_peer_id: PeerId,
        resource_id: impl Into<String>,
        expected_clock: auki_registry::RegistryRef,
    ) -> Result<MessageChannelSender, OpenMessageChannelError> {
        open_message_channel(
            self.stream_control.clone(),
            owner_peer_id,
            resource_id.into(),
            expected_clock,
            self.inbound_shutdown_tx.subscribe(),
            self._lifeline_tx.subscribe(),
        )
        .await
    }

    /// Fetch one registry entry from a cluster peer over the
    /// `/auki/registries/0.0.1` libp2p protocol. The request names
    /// the registry `kind + id + hash`; the response is either the
    /// canonical JSON entry envelope or `None` when the peer does not
    /// have that exact entry.
    ///
    /// `peer_id` must be on the local allow-list — libp2p refuses
    /// the substream otherwise. Higher layers are responsible for
    /// hashing `canonical_json.as_bytes()` and decoding the typed
    /// registry entry only after the hash matches the request.
    pub async fn request_registry_entry(
        &self,
        peer_id: PeerId,
        request: RegistryRequest,
    ) -> Result<RegistryResponse, RequestRegistryError> {
        let mut control = self.stream_control.clone();
        let proto = REGISTRIES_PROTOCOL.clone();

        let open_fut = control.open_stream(peer_id, proto);
        let mut substream = match tokio::time::timeout(REGISTRIES_REQUEST_TIMEOUT, open_fut).await {
            Err(_) => return Err(RequestRegistryError::Timeout(REGISTRIES_REQUEST_TIMEOUT)),
            Ok(Err(e)) => return Err(RequestRegistryError::OpenStream(e)),
            Ok(Ok(s)) => s,
        };

        write_registry_request(&mut substream, &request)
            .await
            .map_err(RequestRegistryError::Protocol)?;

        let response = match tokio::time::timeout(
            REGISTRIES_REQUEST_TIMEOUT,
            read_registry_response(&mut substream),
        )
        .await
        {
            Err(_) => return Err(RequestRegistryError::Timeout(REGISTRIES_REQUEST_TIMEOUT)),
            Ok(Err(e)) => return Err(RequestRegistryError::Protocol(e)),
            Ok(Ok(r)) => r,
        };
        Ok(response)
    }

    /// The runtime's local libp2p peer-id (derived from the swarm's
    /// keypair).
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Cloneable handle for command-style operations
    /// ([`set_allowed_peers`](NetworkRuntimeHandle::set_allowed_peers),
    /// [`set_heartbeat_targets`](NetworkRuntimeHandle::set_heartbeat_targets),
    /// [`connected_peers`](NetworkRuntimeHandle::connected_peers),
    /// [`broadcast_membership`](NetworkRuntimeHandle::broadcast_membership)).
    /// Background tasks call back into the runtime through this
    /// handle without holding the [`NetworkRuntime`] itself.
    pub fn handle(&self) -> NetworkRuntimeHandle {
        NetworkRuntimeHandle {
            command_tx: self.command_tx.clone(),
            connected: self.connected.clone(),
            stream_control: self.stream_control.clone(),
        }
    }

    /// Broadcast a membership update to every currently-connected
    /// allow-listed peer over `/auki/membership/0.0.1`. Fire-and-
    /// forget — spawns one tokio task per peer that opens a
    /// substream, writes the [`MembershipUpdate`], and closes. Errors
    /// per peer are logged at warn level and dropped; the next
    /// broadcast will re-converge.
    ///
    /// `manager_peer_id` identifies the Manager authoring the update.
    /// `membership_json` is the serialized
    /// `auki_domain::ClusterMembership` (typically built via
    /// `ClusterMembership::to_json`). The runtime doesn't interpret
    /// the JSON — that's the receiver's `ClusterManager` job.
    ///
    /// Returns an error only if `membership_json` exceeds the
    /// protocol frame cap (defense against pathological payloads);
    /// otherwise returns `Ok(())` once the per-peer tasks are
    /// spawned. The broadcast continues asynchronously.
    pub fn broadcast_membership(
        &self,
        manager_peer_id: PeerId,
        membership_json: String,
    ) -> Result<(), BroadcastMembershipError> {
        broadcast_membership_impl(
            &self.stream_control,
            &self.connected,
            manager_peer_id,
            membership_json,
        )
    }

    /// Broadcast one best-effort diagnostic message to connected peers.
    pub fn broadcast_diagnostic_message(
        &self,
        message: DiagnosticMessage,
    ) -> Result<(), BroadcastDiagnosticError> {
        broadcast_diagnostic_impl(&self.stream_control, &self.connected, message)
    }

    /// Snapshot of currently-connected peers.
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.connected
            .lock()
            .expect("connected set mutex poisoned")
            .iter()
            .copied()
            .collect()
    }

    /// Replace the peers this runtime should actively open
    /// `/auki/heartbeat/0.0.1` substreams to.
    ///
    /// This is intentionally carrier-level. The runtime does not know
    /// which peer is Manager, which topology is correct, or when a
    /// heartbeat timeout should become a cluster-level loss. It merely
    /// keeps outbound heartbeat substreams open to these allow-listed,
    /// connected targets and reports frame/closure events upward.
    pub async fn set_heartbeat_targets(&self, peers: Vec<PeerId>) -> Result<(), UpdateError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCmd::SetHeartbeatTargets { peers, ack: ack_tx })
            .await
            .map_err(|_| UpdateError::RuntimeUnavailable)?;
        ack_rx.await.map_err(|_| UpdateError::RuntimeUnavailable)?
    }

    fn cleanup(&self) {
        self.message_router.shutdown();
        if let Some(tx) = self
            .shutdown_tx
            .lock()
            .expect("NetworkRuntime shutdown_tx lock poisoned")
            .take()
        {
            let _ = tx.send(());
        }
        if let Some(task) = self
            .task
            .lock()
            .expect("NetworkRuntime task lock poisoned")
            .take()
        {
            task.abort();
        }
    }
}

// ─── UniFFI-exposed surface ──────────────────────────────────────────────────
//
// Methods Swift consumes. The `_string` / `_strings` suffix on the peer-id
// accessors is the same pattern PR A established for `Wallet::wallet_id_str` —
// explicit so the FFI seam shape is visible at the call site.
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl NetworkRuntime {
    /// Canonical libp2p peer-id string for this runtime's local peer.
    pub fn local_peer_id_string(&self) -> String {
        self.local_peer_id.to_string()
    }

    /// Snapshot of currently-connected peer-ids as canonical libp2p
    /// strings. Mutates as connections open / close in the driver task.
    pub fn connected_peer_id_strings(&self) -> Vec<String> {
        self.connected
            .lock()
            .expect("connected set mutex poisoned")
            .iter()
            .map(|p| p.to_string())
            .collect()
    }

    /// Signal the driver task to shut down. Inbound substream tasks
    /// have [`SHUTDOWN_GRACE`] to flush their final typed
    /// `EndOfStream` before the swarm tears down. Unclean exit (`Drop`
    /// without an explicit `shutdown` call, panic) skips the grace —
    /// consumers see `ConnectionLost` instead of the typed reason.
    ///
    /// Idempotent: the first call broadcasts the grace signal and
    /// aborts the driver task; subsequent calls find `shutdown_tx` /
    /// `task` already taken and no-op. Safe to call from multiple
    /// threads concurrently.
    pub fn shutdown(&self) {
        self.message_router.shutdown();
        let _ = self.inbound_shutdown_tx.send(true);
        if let Some(tx) = self
            .shutdown_tx
            .lock()
            .expect("NetworkRuntime shutdown_tx lock poisoned")
            .take()
        {
            let _ = tx.send(());
        }
    }
}

// ─── UniFFI-exposed async surface ────────────────────────────────────────────
//
// `set_allowed_peers` is async and needs the tokio async runtime annotation.
// Kept in a separate impl block so the non-async UniFFI block above stays
// clean and the annotation is explicit at the seam.
#[cfg_attr(feature = "swift-bindings", uniffi::export(async_runtime = "tokio"))]
impl NetworkRuntime {
    /// Replace the allow-list with `new_peers`. The runtime diffs:
    ///
    /// - peer-ids in `new_peers` but not the old list are added to
    ///   the libp2p allow-list and (if they carry addresses)
    ///   scheduled for dial
    /// - peer-ids in the old list but not `new_peers` are removed from
    ///   the allow-list and their existing connections dropped
    /// - peer-ids in both keep their existing connection; addresses
    ///   are refreshed in case the new list carries different ones
    ///
    /// Returns an [`UpdateReport`] describing the diff.
    pub async fn set_allowed_peers(
        &self,
        new_peers: Vec<AllowedPeer>,
    ) -> Result<UpdateReport, UpdateError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCmd::SetAllowedPeers {
                new_peers,
                ack: ack_tx,
            })
            .await
            .map_err(|_| UpdateError::RuntimeUnavailable)?;
        ack_rx.await.map_err(|_| UpdateError::RuntimeUnavailable)?
    }
}

impl Drop for NetworkRuntime {
    fn drop(&mut self) {
        // Drop never fires the inbound grace signal — only explicit
        // `shutdown()` does. Calling `cleanup()` here is idempotent
        // against prior `shutdown()` and tears the driver task down
        // without the EndOfStream flush window.
        self.cleanup();
    }
}

// ─── UniFFI-exposed stream surface ──────────────────────────────────────────

/// Shared cross-FFI stream entry shape. The opaque `payload_bytes` is
/// prost-encoded against the per-payload `.proto` (`audio.proto`,
/// `CameraFrame.proto`, …); Swift consumers decode via swift-protobuf.
/// Type-distinguishability lives at the `StreamSubscription*` /
/// `open_*_stream` level.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Record, Debug, Clone)]
pub struct StreamEntry {
    pub timestamp_ns: i64,
    pub seq: u64,
    pub payload_bytes: Vec<u8>,
}

/// Cross-FFI stream-error variants. Flattened from
/// `stream_runtime::StreamError`; non-FFI variants surface as Display'd
/// `message: String`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum StreamError {
    #[error("end of stream: {reason}")]
    EndOfStream { reason: String },
    #[error("connection lost")]
    ConnectionLost,
    #[error("protocol error: {message}")]
    Protocol { message: String },
}

#[cfg(feature = "swift-bindings")]
impl From<crate::stream_runtime::StreamError> for StreamError {
    fn from(e: crate::stream_runtime::StreamError) -> Self {
        match e {
            crate::stream_runtime::StreamError::EndOfStream { reason } => Self::EndOfStream {
                reason: format!("{reason:?}"),
            },
            crate::stream_runtime::StreamError::ConnectionLost => Self::ConnectionLost,
            crate::stream_runtime::StreamError::Protocol(p) => Self::Protocol {
                message: p.to_string(),
            },
        }
    }
}

/// Cross-FFI open-stream-error variants. Flattened.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum OpenStreamError {
    #[error("declined: {reason}")]
    Declined { reason: String },
    #[error("libp2p open failed: {message}")]
    LibP2p { message: String },
    #[error("protocol error: {message}")]
    Protocol { message: String },
    #[error("open timed out after {ms} ms")]
    Timeout { ms: u64 },
}

#[cfg(feature = "swift-bindings")]
impl From<crate::stream_runtime::OpenStreamError> for OpenStreamError {
    fn from(e: crate::stream_runtime::OpenStreamError) -> Self {
        use crate::stream_runtime::OpenStreamError as Up;
        match e {
            Up::Declined { reason } => Self::Declined {
                reason: format!("{reason:?}"),
            },
            Up::LibP2p(err) => Self::LibP2p {
                message: err.to_string(),
            },
            Up::Protocol(err) => Self::Protocol {
                message: err.to_string(),
            },
            Up::Timeout(d) => Self::Timeout {
                ms: d.as_millis() as u64,
            },
        }
    }
}

/// Swift-friendly wrapper around `StreamSubscription<audio::Data>`.
/// Exposes `manifest_bytes()` (prost-encoded `StreamManifest`) and
/// `next_entry()` (async; yields one entry per call until the stream
/// ends).
///
/// The wrapper is fail-poisoned: once `next_entry` returns `Err` (a
/// final stream error) or `Ok(None)` (clean end-of-stream), subsequent
/// calls return `Ok(None)`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionAudio {
    inner: tokio::sync::Mutex<
        Option<crate::stream_runtime::StreamSubscription<crate::stream_protocol::audio::Data>>,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionAudio {
    /// Construct from an upstream typed subscription. Encodes the
    /// manifest once at construction.
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<crate::stream_protocol::audio::Data>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionAudio {
    /// Prost-encoded `StreamManifest`. Stable for the lifetime of the
    /// subscription; safe to call multiple times.
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    /// Read the next entry off the wire. Returns `Ok(Some(entry))` for
    /// each entry, `Ok(None)` exactly once when the stream ends
    /// cleanly, or `Err(StreamError)` once when the stream ends with an
    /// error. After `Ok(None)` or `Err`, subsequent calls return
    /// `Ok(None)`.
    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}

/// Swift-friendly wrapper around `StreamSubscription<CameraFrame>`.
/// Exposes `manifest_bytes()` (prost-encoded `StreamManifest`) and
/// `next_entry()` (async; yields one entry per call until the stream
/// ends).
///
/// The wrapper is fail-poisoned: once `next_entry` returns `Err` (a
/// final stream error) or `Ok(None)` (clean end-of-stream), subsequent
/// calls return `Ok(None)`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionCamera {
    inner: tokio::sync::Mutex<
        Option<crate::stream_runtime::StreamSubscription<crate::stream_protocol::CameraFrame>>,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionCamera {
    /// Construct from an upstream typed subscription. Encodes the
    /// manifest once at construction.
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<crate::stream_protocol::CameraFrame>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionCamera {
    /// Prost-encoded `StreamManifest`. Stable for the lifetime of the
    /// subscription; safe to call multiple times.
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    /// Read the next entry off the wire. Returns `Ok(Some(entry))` for
    /// each entry, `Ok(None)` exactly once when the stream ends
    /// cleanly, or `Err(StreamError)` once when the stream ends with an
    /// error. After `Ok(None)` or `Err`, subsequent calls return
    /// `Ok(None)`.
    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}

/// Swift-friendly wrapper around `StreamSubscription<point_cloud::Data>`.
/// Exposes `manifest_bytes()` (prost-encoded `StreamManifest`) and
/// `next_entry()` (async; yields one entry per call until the stream
/// ends).
///
/// The wrapper is fail-poisoned: once `next_entry` returns `Err` (a
/// final stream error) or `Ok(None)` (clean end-of-stream), subsequent
/// calls return `Ok(None)`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionPointCloud {
    inner: tokio::sync::Mutex<
        Option<
            crate::stream_runtime::StreamSubscription<crate::stream_protocol::point_cloud::Data>,
        >,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionPointCloud {
    /// Construct from an upstream typed subscription. Encodes the
    /// manifest once at construction.
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<crate::stream_protocol::point_cloud::Data>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionPointCloud {
    /// Prost-encoded `StreamManifest`. Stable for the lifetime of the
    /// subscription; safe to call multiple times.
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    /// Read the next entry off the wire. Returns `Ok(Some(entry))` for
    /// each entry, `Ok(None)` exactly once when the stream ends
    /// cleanly, or `Err(StreamError)` once when the stream ends with an
    /// error. After `Ok(None)` or `Err`, subsequent calls return
    /// `Ok(None)`.
    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}

/// Swift-friendly wrapper around `StreamSubscription<joint_encoders::Data>`.
/// Exposes `manifest_bytes()` (prost-encoded `StreamManifest`) and
/// `next_entry()` (async; yields one entry per call until the stream
/// ends).
///
/// The wrapper is fail-poisoned: once `next_entry` returns `Err` (a
/// final stream error) or `Ok(None)` (clean end-of-stream), subsequent
/// calls return `Ok(None)`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionJointEncoders {
    inner: tokio::sync::Mutex<
        Option<
            crate::stream_runtime::StreamSubscription<crate::stream_protocol::joint_encoders::Data>,
        >,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionJointEncoders {
    /// Construct from an upstream typed subscription. Encodes the
    /// manifest once at construction.
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<
            crate::stream_protocol::joint_encoders::Data,
        >,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionJointEncoders {
    /// Prost-encoded `StreamManifest`. Stable for the lifetime of the
    /// subscription; safe to call multiple times.
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    /// Read the next entry off the wire. Returns `Ok(Some(entry))` for
    /// each entry, `Ok(None)` exactly once when the stream ends
    /// cleanly, or `Err(StreamError)` once when the stream ends with an
    /// error. After `Ok(None)` or `Err`, subsequent calls return
    /// `Ok(None)`.
    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}

/// Swift-friendly wrapper around `StreamSubscription<DetectionFrame>`.
/// Exposes `manifest_bytes()` (prost-encoded `StreamManifest`) and
/// `next_entry()` (async; yields one entry per call until the stream
/// ends).
///
/// The payload type is `auki_datatypes::detection::DetectionFrame`
/// (opaque-bytes detection payload, Cuba T8).
///
/// The wrapper is fail-poisoned: once `next_entry` returns `Err` (a
/// final stream error) or `Ok(None)` (clean end-of-stream), subsequent
/// calls return `Ok(None)`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionDetection {
    inner: tokio::sync::Mutex<
        Option<
            crate::stream_runtime::StreamSubscription<auki_datatypes::detection::DetectionFrame>,
        >,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionDetection {
    /// Construct from an upstream typed subscription. Encodes the
    /// manifest once at construction.
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<auki_datatypes::detection::DetectionFrame>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionDetection {
    /// Prost-encoded `StreamManifest`. Stable for the lifetime of the
    /// subscription; safe to call multiple times.
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    /// Read the next entry off the wire. Returns `Ok(Some(entry))` for
    /// each entry, `Ok(None)` exactly once when the stream ends
    /// cleanly, or `Err(StreamError)` once when the stream ends with an
    /// error. After `Ok(None)` or `Err`, subsequent calls return
    /// `Ok(None)`.
    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}

// ─── UniFFI-exposed async stream-open surface ────────────────────────────────

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl NetworkRuntime {
    /// Open an outbound audio stream against `peer_id`. `request_bytes` is
    /// a prost-encoded `auki.stream.StreamRequest`. Returns a typed
    /// `StreamSubscriptionAudio` on accept; an `OpenStreamError` on
    /// decline, libp2p failure, or timeout.
    pub async fn open_audio_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionAudio>, OpenStreamError> {
        use prost::Message;
        let wire_req = auki_datatypes::stream::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let request = crate::stream_protocol::stream_request_from_wire(wire_req);
        let sub = self
            .open_stream::<crate::stream_protocol::audio::Data>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionAudio::from_inner(sub))
    }

    /// Open an outbound camera stream against `peer_id`. `request_bytes` is
    /// a prost-encoded `auki.stream.StreamRequest`. Returns a typed
    /// `StreamSubscriptionCamera` on accept; an `OpenStreamError` on
    /// decline, libp2p failure, or timeout.
    pub async fn open_camera_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionCamera>, OpenStreamError> {
        use prost::Message;
        let wire_req = auki_datatypes::stream::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let request = crate::stream_protocol::stream_request_from_wire(wire_req);
        let sub = self
            .open_stream::<crate::stream_protocol::CameraFrame>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionCamera::from_inner(sub))
    }

    /// Open an outbound point-cloud stream against `peer_id`. `request_bytes`
    /// is a prost-encoded `auki.stream.StreamRequest`. Returns a typed
    /// `StreamSubscriptionPointCloud` on accept; an `OpenStreamError` on
    /// decline, libp2p failure, or timeout.
    pub async fn open_pointcloud_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionPointCloud>, OpenStreamError> {
        use prost::Message;
        let wire_req = auki_datatypes::stream::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let request = crate::stream_protocol::stream_request_from_wire(wire_req);
        let sub = self
            .open_stream::<crate::stream_protocol::point_cloud::Data>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionPointCloud::from_inner(sub))
    }

    /// Open an outbound joint-encoders stream against `peer_id`. `request_bytes`
    /// is a prost-encoded `auki.stream.StreamRequest`. Returns a typed
    /// `StreamSubscriptionJointEncoders` on accept; an `OpenStreamError` on
    /// decline, libp2p failure, or timeout.
    pub async fn open_joint_encoders_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionJointEncoders>, OpenStreamError> {
        use prost::Message;
        let wire_req = auki_datatypes::stream::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let request = crate::stream_protocol::stream_request_from_wire(wire_req);
        let sub = self
            .open_stream::<crate::stream_protocol::joint_encoders::Data>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionJointEncoders::from_inner(sub))
    }

    /// Open an outbound detection stream against `peer_id`. `request_bytes`
    /// is a prost-encoded `auki.stream.StreamRequest`. Returns a typed
    /// `StreamSubscriptionDetection` on accept; an `OpenStreamError` on
    /// decline, libp2p failure, or timeout.
    pub async fn open_detection_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionDetection>, OpenStreamError> {
        use prost::Message;
        let wire_req = auki_datatypes::stream::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let request = crate::stream_protocol::stream_request_from_wire(wire_req);
        let sub = self
            .open_stream::<auki_datatypes::detection::DetectionFrame>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionDetection::from_inner(sub))
    }
}

// ─── Driver task ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_task(
    swarm: Swarm<Behaviour>,
    initial_peers: Vec<AllowedPeer>,
    connected: Arc<Mutex<HashSet<PeerId>>>,
    stream_provider: StreamProvider,
    heartbeat_timestamps: HeartbeatTimestampSource,
    mut inbound_control: Control,
    inbound_shutdown_rx: watch::Receiver<bool>,
    lifeline_rx: watch::Receiver<()>,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut command_rx: mpsc::Receiver<RuntimeCmd>,
    join_events_tx: mpsc::Sender<JoinEvent>,
    liveness_tx: mpsc::Sender<PeerLivenessEvent>,
    membership_events_tx: mpsc::Sender<MembershipEvent>,
    info_events_tx: mpsc::Sender<InfoRequestEvent>,
    resources_events_tx: mpsc::Sender<ResourcesRequestEvent>,
    registry_events_tx: mpsc::Sender<RegistryRequestEvent>,
    diagnostic_events_tx: mpsc::Sender<DiagnosticEvent>,
    message_router: MessageChannelRouter,
    map_catalog_provider: SharedMapCatalogProvider,
    device_model_catalog_provider: SharedDeviceModelCatalogProvider,
    blob_app_root: SharedBlobAppRoot,
) {
    let mut swarm = swarm;
    let local_peer_id = *swarm.local_peer_id();
    let mut known_peers: HashMap<PeerId, Vec<Multiaddr>> = initial_peers
        .iter()
        .map(|p| (p.peer_id, p.multiaddrs.clone()))
        .collect();

    // Initial dial schedule. Peers with at least one address are
    // dialed immediately on first tick; address-less entries are
    // honoured as trusted (we'll respond to them if they dial us) but
    // not auto-dialed.
    let mut schedules: HashMap<PeerId, PeerSchedule> = known_peers
        .iter()
        .filter(|(_, addrs)| !addrs.is_empty())
        .map(|(pid, _)| {
            (
                *pid,
                PeerSchedule {
                    next_dial_at: Some(Instant::now()),
                    backoff: INITIAL_BACKOFF,
                },
            )
        })
        .collect();

    let mut tick = tokio::time::interval(RECONNECT_TICK);

    // Active outbound heartbeat carriers, keyed by peer-id. These are
    // reconciled against `heartbeat_targets`.
    let mut outbound_heartbeat_tasks: HashMap<PeerId, JoinHandle<()>> = HashMap::new();
    // Active inbound heartbeat carriers, keyed by peer-id. These are
    // accepted from known peers and are not reconciled against
    // outbound target state.
    let mut inbound_heartbeat_tasks: HashMap<PeerId, JoinHandle<()>> = HashMap::new();
    // Exact peer-ids this runtime should open heartbeat carrier
    // substreams to. Cluster topology lives in `auki-domain`; the
    // runtime only reconciles this target set against connected
    // allow-listed peers.
    let mut heartbeat_targets: HashSet<PeerId> = HashSet::new();

    // Register inbound `/auki/stream/0.1.0` substream acceptance.
    let stream_proto = StreamProtocol::try_from_owned(STREAM_PROTOCOL.to_string())
        .expect("STREAM_PROTOCOL is a valid libp2p stream protocol id");
    let mut incoming_streams: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(stream_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/join/0.0.1` substream acceptance.
    let join_proto = StreamProtocol::try_from_owned(JOIN_PROTOCOL.to_string())
        .expect("JOIN_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_joins: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(join_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/heartbeat/0.0.1` substream acceptance.
    let heartbeat_proto = StreamProtocol::try_from_owned(HEARTBEAT_PROTOCOL.to_string())
        .expect("HEARTBEAT_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_heartbeats: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(heartbeat_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/membership/0.0.1` substream acceptance.
    let membership_proto = StreamProtocol::try_from_owned(MEMBERSHIP_PROTOCOL.to_string())
        .expect("MEMBERSHIP_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_memberships: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(membership_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/info/0.0.1` substream acceptance.
    let info_proto = StreamProtocol::try_from_owned(INFO_PROTOCOL.to_string())
        .expect("INFO_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_infos: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(info_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/resources/0.0.1` substream acceptance.
    let resources_proto = RESOURCES_PROTOCOL.clone();
    let mut incoming_resources: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(resources_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    let mut incoming_resources_v3: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(RESOURCES_V3_PROTOCOL.clone()) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    let mut incoming_resources_v4: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(RESOURCES_V4_PROTOCOL.clone()) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    let mut incoming_resources_v5: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(RESOURCES_V5_PROTOCOL.clone()) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    let mut incoming_blobs: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(BLOBS_PROTOCOL.clone()) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/registries/0.0.1` substream acceptance.
    let registries_proto = REGISTRIES_PROTOCOL.clone();
    let mut incoming_registries: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(registries_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/diagnostic/0.0.1` substream acceptance.
    let diagnostic_proto = StreamProtocol::try_from_owned(DIAGNOSTIC_PROTOCOL.to_string())
        .expect("DIAGNOSTIC_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_diagnostics: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(diagnostic_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    let message_proto = StreamProtocol::try_from_owned(MESSAGE_PROTOCOL.to_owned())
        .expect("MESSAGE_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_messages: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(message_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };
    let (message_task_done_tx, mut message_task_done_rx) =
        mpsc::unbounded_channel::<(PeerId, u64)>();
    let mut active_message_tasks = ActiveMessageTasks::default();
    let message_frame_memory = MessageFrameMemoryBudget::new();

    loop {
        if take_pending_runtime_shutdown(&mut shutdown_rx) {
            active_message_tasks.cancel_all().await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
            return;
        }
        if let Some(work) =
            take_pending_message_driver_work(&mut command_rx, &mut message_task_done_rx)
        {
            match work {
                PendingMessageDriverWork::Command(cmd) => {
                    handle_command(
                        cmd,
                        local_peer_id,
                        &mut swarm,
                        &mut known_peers,
                        &mut schedules,
                        &connected,
                        &inbound_control,
                        &mut outbound_heartbeat_tasks,
                        &mut inbound_heartbeat_tasks,
                        &mut heartbeat_targets,
                        &liveness_tx,
                        &lifeline_rx,
                        &heartbeat_timestamps,
                        &mut active_message_tasks,
                    )
                    .await;
                }
                PendingMessageDriverWork::Completion { peer_id, task_id } => {
                    active_message_tasks.complete(peer_id, task_id).await;
                }
            }
            continue;
        }

        tokio::select! {
            biased;

            _ = &mut shutdown_rx => {
                active_message_tasks.cancel_all().await;
                tokio::time::sleep(SHUTDOWN_GRACE).await;
                return;
            }

            cmd = command_rx.recv() => {
                let Some(cmd) = cmd else { continue; };
                handle_command(
                    cmd,
                    local_peer_id,
                    &mut swarm,
                    &mut known_peers,
                    &mut schedules,
                    &connected,
                    &inbound_control,
                    &mut outbound_heartbeat_tasks,
                    &mut inbound_heartbeat_tasks,
                    &mut heartbeat_targets,
                    &liveness_tx,
                    &lifeline_rx,
                    &heartbeat_timestamps,
                    &mut active_message_tasks,
                ).await;
            }

            completed = message_task_done_rx.recv() => {
                if let Some((peer, task_id)) = completed {
                    active_message_tasks.complete(peer, task_id).await;
                }
            }

            event = swarm.next() => {
                let Some(event) = event else { return; };
                if let Some(disconnected_peer) = handle_event(
                    event,
                    local_peer_id,
                    &mut swarm,
                    &known_peers,
                    &mut schedules,
                    &connected,
                    &inbound_control,
                    &lifeline_rx,
                    &mut outbound_heartbeat_tasks,
                    &mut inbound_heartbeat_tasks,
                    &heartbeat_targets,
                    &liveness_tx,
                    &heartbeat_timestamps,
                ) {
                    active_message_tasks.cancel_peer(disconnected_peer).await;
                }
            }

            inbound = incoming_streams.next() => {
                let Some((peer, substream)) = inbound else { return; };
                // Inbound stream-protocol substreams from peers not on
                // the allow-list are impossible (the libp2p allow-list
                // refuses them at handshake time), but defensive
                // double-check on the substream side belt-and-braces
                // the trust boundary.
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                let provider = stream_provider.clone();
                let task_shutdown = inbound_shutdown_rx.clone();
                tokio::spawn(handle_inbound_substream(
                    peer,
                    substream,
                    provider,
                    task_shutdown,
                ));
            }

            join = incoming_joins.next() => {
                let Some((peer, substream)) = join else { return; };
                // Inbound join substreams come from peers on the
                // allow-list (libp2p enforces). The Manager-side
                // owner decides whether to admit; the runtime just
                // plumbs the request through and ferries the response
                // back.
                let tx = join_events_tx.clone();
                tokio::spawn(handle_inbound_join_substream(peer, substream, tx));
            }

            heartbeat = incoming_heartbeats.next() => {
                let Some((peer, substream)) = heartbeat else { return; };
                // Inbound side of the heartbeat substream. The domain
                // layer decides who should open; the carrier accepts
                // known peers and reports heartbeat frames upward.
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                // Cancel any previous task and start a fresh one.
                if let Some(prev) = inbound_heartbeat_tasks.remove(&peer) {
                    prev.abort();
                }
                let task = tokio::spawn(run_heartbeat_pair(
                    peer,
                    substream,
                    liveness_tx.clone(),
                    lifeline_rx.clone(),
                    heartbeat_timestamps.clone(),
                ));
                inbound_heartbeat_tasks.insert(peer, task);
            }

            membership = incoming_memberships.next() => {
                let Some((peer, substream)) = membership else { return; };
                // Cluster-trust gate identical to `/auki/stream/0.1.0` —
                // non-allow-list peers silently dropped (no `Decline`
                // shape on this protocol, no probe signal). The libp2p
                // connection-level gate is open by default in Hagall;
                // per-protocol enforcement lives here.
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                let tx = membership_events_tx.clone();
                tokio::spawn(handle_inbound_membership_substream(peer, substream, tx));
            }

            info = incoming_infos.next() => {
                let Some((peer, substream)) = info else { return; };
                // Same cluster-trust gate. Non-cluster peers can't
                // fetch a daemon's ParticipantInfo — privacy by
                // membership.
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                let tx = info_events_tx.clone();
                tokio::spawn(handle_inbound_info_substream(peer, substream, tx));
            }

            resources = incoming_resources.next() => {
                let Some((peer, substream)) = resources else { return; };
                // Same cluster-trust gate. Non-cluster peers can't
                // fetch a daemon's generalized resource catalog.
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                let tx = resources_events_tx.clone();
                tokio::spawn(handle_inbound_resources_substream(peer, substream, tx));
            }

            resources = incoming_resources_v3.next() => {
                let Some((peer, substream)) = resources else { return; };
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                tokio::spawn(handle_inbound_resources_v3_substream(
                    peer,
                    substream,
                    resources_events_tx.clone(),
                    message_router.clone(),
                ));
            }

            resources = incoming_resources_v4.next() => {
                let Some((peer, substream)) = resources else { return; };
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                tokio::spawn(handle_inbound_resources_v4_substream(
                    peer,
                    substream,
                    map_catalog_provider.clone(),
                ));
            }

            resources = incoming_resources_v5.next() => {
                let Some((peer, substream)) = resources else { return; };
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                tokio::spawn(handle_inbound_resources_v5_substream(
                    peer, substream, device_model_catalog_provider.clone(),
                ));
            }

            blob = incoming_blobs.next() => {
                let Some((peer, substream)) = blob else { return; };
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                tokio::spawn(handle_inbound_blob_substream(substream, blob_app_root.clone()));
            }

            registry = incoming_registries.next() => {
                let Some((peer, substream)) = registry else { return; };
                // Same cluster-trust gate. Non-cluster peers can't
                // fetch a daemon's registry entries — privacy by
                // membership.
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                let tx = registry_events_tx.clone();
                tokio::spawn(handle_inbound_registry_substream(peer, substream, tx));
            }

            diagnostic = incoming_diagnostics.next() => {
                let Some((peer, substream)) = diagnostic else { return; };
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                let tx = diagnostic_events_tx.clone();
                tokio::spawn(handle_inbound_diagnostic_substream(peer, substream, tx));
            }

            message = incoming_messages.next() => {
                let Some((peer, substream)) = message else { return; };
                // Authorization is complete before the open frame or any
                // Message payload is read.
                let Some(substream) = authorize_inbound_message_stream(
                    known_peers.contains_key(&peer),
                    substream,
                ) else {
                    continue;
                };
                let Some(task_id) = active_message_tasks.allocate_id(peer) else {
                    drop(substream);
                    continue;
                };
                let (revoke_tx, revoke_rx) = watch::channel(false);
                let done_tx = message_task_done_tx.clone();
                let router = message_router.clone();
                let frame_memory = message_frame_memory.clone();
                let task_shutdown = inbound_shutdown_rx.clone();
                let task = tokio::spawn(async move {
                    handle_inbound_message(
                        peer,
                        substream,
                        router,
                        frame_memory,
                        task_shutdown,
                        revoke_rx,
                    )
                    .await;
                    let _ = done_tx.send((peer, task_id));
                });
                active_message_tasks.track(peer, task_id, revoke_tx, task);
            }

            _ = tick.tick() => {
                drive_pending_dials(&mut swarm, &known_peers, &mut schedules);
                reconcile_heartbeat_tasks(
                    local_peer_id,
                    &swarm,
                    &known_peers,
                    &inbound_control,
                    &mut outbound_heartbeat_tasks,
                    &heartbeat_targets,
                    &liveness_tx,
                    &lifeline_rx,
                    &heartbeat_timestamps,
                );
            }

        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_event(
    event: SwarmEvent<BehaviourEvent>,
    local_peer_id: PeerId,
    _swarm: &mut Swarm<Behaviour>,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    stream_control: &Control,
    lifeline_rx: &watch::Receiver<()>,
    outbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    inbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    heartbeat_targets: &HashSet<PeerId>,
    liveness_tx: &mpsc::Sender<PeerLivenessEvent>,
    heartbeat_timestamps: &HeartbeatTimestampSource,
) -> Option<PeerId> {
    match event {
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            if known_peers.contains_key(&peer_id) {
                if let Some(sched) = schedules.get_mut(&peer_id) {
                    sched.next_dial_at = None;
                    sched.backoff = INITIAL_BACKOFF;
                }
                connected
                    .lock()
                    .expect("connected set mutex poisoned")
                    .insert(peer_id);

                // Surface the Connected event to the owner.
                let _ = liveness_tx.try_send(PeerLivenessEvent::Connected { peer_id });

                try_spawn_heartbeat_opener(
                    local_peer_id,
                    peer_id,
                    heartbeat_targets,
                    stream_control,
                    outbound_heartbeat_tasks,
                    liveness_tx,
                    lifeline_rx,
                    heartbeat_timestamps,
                );
            }
            None
        }
        SwarmEvent::ConnectionClosed {
            peer_id,
            num_established,
            ..
        } => handle_connection_closed(
            peer_id,
            num_established,
            known_peers,
            schedules,
            connected,
            outbound_heartbeat_tasks,
            inbound_heartbeat_tasks,
            liveness_tx,
        ),
        SwarmEvent::OutgoingConnectionError {
            peer_id: Some(peer_id),
            ..
        } => {
            if known_peers.contains_key(&peer_id) {
                schedule_retry(schedules, peer_id);
            }
            None
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_connection_closed(
    peer_id: PeerId,
    num_established: u32,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    outbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    inbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    liveness_tx: &mpsc::Sender<PeerLivenessEvent>,
) -> Option<PeerId> {
    if num_established > 0 {
        return None;
    }

    connected
        .lock()
        .expect("connected set mutex poisoned")
        .remove(&peer_id);
    if let Some(task) = outbound_heartbeat_tasks.remove(&peer_id) {
        task.abort();
    }
    if let Some(task) = inbound_heartbeat_tasks.remove(&peer_id) {
        task.abort();
    }
    if known_peers.contains_key(&peer_id) {
        let _ = liveness_tx.try_send(PeerLivenessEvent::Disconnected { peer_id });
        schedule_retry(schedules, peer_id);
    }
    Some(peer_id)
}

#[allow(clippy::too_many_arguments)]
async fn handle_command(
    cmd: RuntimeCmd,
    local_peer_id: PeerId,
    swarm: &mut Swarm<Behaviour>,
    known_peers: &mut HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    stream_control: &Control,
    outbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    inbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    heartbeat_targets: &mut HashSet<PeerId>,
    liveness_tx: &mpsc::Sender<PeerLivenessEvent>,
    lifeline_rx: &watch::Receiver<()>,
    heartbeat_timestamps: &HeartbeatTimestampSource,
    active_message_tasks: &mut ActiveMessageTasks,
) {
    match cmd {
        RuntimeCmd::SetAllowedPeers { new_peers, ack } => {
            let new_peer_ids: HashSet<PeerId> = new_peers.iter().map(|peer| peer.peer_id).collect();
            let removed_peers: Vec<PeerId> = known_peers
                .keys()
                .copied()
                .filter(|peer| !new_peer_ids.contains(peer))
                .collect();
            // Revoke and join active handlers before mutating the allow-list,
            // disconnecting, or acknowledging the update.
            for peer in removed_peers {
                active_message_tasks.cancel_peer(peer).await;
            }
            let report = apply_peer_update(swarm, known_peers, schedules, connected, new_peers);
            prune_inbound_heartbeat_tasks(known_peers, inbound_heartbeat_tasks);
            reconcile_heartbeat_tasks(
                local_peer_id,
                swarm,
                known_peers,
                stream_control,
                outbound_heartbeat_tasks,
                heartbeat_targets,
                liveness_tx,
                lifeline_rx,
                heartbeat_timestamps,
            );
            let _ = ack.send(Ok(report));
        }
        RuntimeCmd::SetHeartbeatTargets { peers, ack } => {
            *heartbeat_targets = peers
                .into_iter()
                .filter(|pid| *pid != local_peer_id)
                .collect();
            reconcile_heartbeat_tasks(
                local_peer_id,
                swarm,
                known_peers,
                stream_control,
                outbound_heartbeat_tasks,
                heartbeat_targets,
                liveness_tx,
                lifeline_rx,
                heartbeat_timestamps,
            );
            let _ = ack.send(Ok(()));
        }
        RuntimeCmd::ReserveRelayCircuitAddr {
            relay_dial_multiaddr,
            relay_advertise_multiaddr,
            timeout,
            ack,
        } => {
            let result = swarm::reserve_relay_circuit_addr_with_advertised_addr(
                swarm,
                relay_dial_multiaddr,
                relay_advertise_multiaddr,
                timeout,
            )
            .await
            .map_err(RuntimeRelayReservationError::Reservation);
            let _ = ack.send(result);
        }
    }
}

fn prune_inbound_heartbeat_tasks(
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    inbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
) {
    let stale: Vec<PeerId> = inbound_heartbeat_tasks
        .keys()
        .copied()
        .filter(|pid| !known_peers.contains_key(pid))
        .collect();
    for pid in stale {
        if let Some(task) = inbound_heartbeat_tasks.remove(&pid) {
            task.abort();
        }
    }
}

fn prune_finished_heartbeat_tasks(heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>) {
    heartbeat_tasks.retain(|_, task| !task.is_finished());
}

/// Reconcile active outbound heartbeat tasks against the current
/// carrier target set. The caller owns cluster semantics; the runtime
/// only opens substreams to connected, allow-listed target peers.
#[allow(clippy::too_many_arguments)]
fn reconcile_heartbeat_tasks(
    local_peer_id: PeerId,
    swarm: &Swarm<Behaviour>,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    stream_control: &Control,
    outbound_heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    heartbeat_targets: &HashSet<PeerId>,
    liveness_tx: &mpsc::Sender<PeerLivenessEvent>,
    lifeline_rx: &watch::Receiver<()>,
    heartbeat_timestamps: &HeartbeatTimestampSource,
) {
    prune_finished_heartbeat_tasks(outbound_heartbeat_tasks);

    let desired: HashSet<PeerId> = heartbeat_targets
        .iter()
        .copied()
        .filter(|pid| *pid != local_peer_id && known_peers.contains_key(pid))
        .collect();
    let stale: Vec<PeerId> = outbound_heartbeat_tasks
        .keys()
        .copied()
        .filter(|pid| !desired.contains(pid))
        .collect();
    for pid in stale {
        if let Some(task) = outbound_heartbeat_tasks.remove(&pid) {
            task.abort();
        }
    }

    for pid in desired {
        if swarm.is_connected(&pid) {
            try_spawn_heartbeat_opener(
                local_peer_id,
                pid,
                heartbeat_targets,
                stream_control,
                outbound_heartbeat_tasks,
                liveness_tx,
                lifeline_rx,
                heartbeat_timestamps,
            );
        }
    }
}

/// Spawn the outbound `/auki/heartbeat/0.0.1` carrier to `peer` if it
/// is in the current target set. Idempotent against a pre-existing
/// task for the same peer.
///
/// Called from three sites in the driver task:
/// 1. `handle_event::ConnectionEstablished` for a peer already on the
///    allow-list at handshake time (the normal path).
/// 2. `handle_command::SetAllowedPeers` after `apply_peer_update`
///    retroactively recognises a connection that predates the peer's
///    addition to the allow-list (the join-protocol race — an inbound
///    joiner whose connection completes before they're admitted into
///    `known_peers`). Without this site, a Manager can miss its chance
///    to open the heartbeat substream for an already-connected joiner.
/// 3. `handle_command::SetHeartbeatTargets` when the domain layer
///    changes which peers should have outbound heartbeat carriers.
#[allow(clippy::too_many_arguments)]
fn try_spawn_heartbeat_opener(
    local_peer_id: PeerId,
    peer_id: PeerId,
    heartbeat_targets: &HashSet<PeerId>,
    stream_control: &Control,
    heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    liveness_tx: &mpsc::Sender<PeerLivenessEvent>,
    lifeline_rx: &watch::Receiver<()>,
    heartbeat_timestamps: &HeartbeatTimestampSource,
) {
    if local_peer_id == peer_id || !heartbeat_targets.contains(&peer_id) {
        return;
    }
    if heartbeat_tasks.contains_key(&peer_id) {
        return;
    }
    let control = stream_control.clone();
    let liveness = liveness_tx.clone();
    let lifeline = lifeline_rx.clone();
    let timestamps = heartbeat_timestamps.clone();
    let task = tokio::spawn(async move {
        open_and_run_heartbeat_pair(peer_id, control, liveness, lifeline, timestamps).await;
    });
    heartbeat_tasks.insert(peer_id, task);
}

fn apply_peer_update(
    swarm: &mut Swarm<Behaviour>,
    known_peers: &mut HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    new_peers: Vec<AllowedPeer>,
) -> UpdateReport {
    let new_set: HashSet<PeerId> = new_peers.iter().map(|p| p.peer_id).collect();

    // Removed peers: drop connection + disallow + clear schedule.
    let removed: Vec<PeerId> = known_peers
        .keys()
        .copied()
        .filter(|pid| !new_set.contains(pid))
        .collect();
    for pid in &removed {
        let _ = swarm.disconnect_peer_id(*pid);
        schedules.remove(pid);
        known_peers.remove(pid);
        connected
            .lock()
            .expect("connected set mutex poisoned")
            .remove(pid);
    }

    // Added peers: allow + (if addresses are present) schedule dial.
    let now = Instant::now();
    let added: Vec<PeerId> = new_peers
        .iter()
        .map(|p| p.peer_id)
        .filter(|pid| !known_peers.contains_key(pid))
        .collect();
    for ap in &new_peers {
        if added.contains(&ap.peer_id) {
            let has_addrs = !ap.multiaddrs.is_empty();
            known_peers.insert(ap.peer_id, ap.multiaddrs.clone());
            if has_addrs {
                schedules.insert(
                    ap.peer_id,
                    PeerSchedule {
                        next_dial_at: Some(now),
                        backoff: INITIAL_BACKOFF,
                    },
                );
            }
            // The peer may already have an active libp2p connection
            // — this happens when a non-cluster peer dials us, the
            // connection-level allow-list (open by default in
            // Hagall) lets the noise handshake complete, but
            // `handle_event`'s ConnectionEstablished branch ignored
            // the `connected` insertion because `known_peers` didn't
            // contain them yet. Now that we're adding them to
            // `known_peers`, retroactively recognise the connection
            // so outbound flows (membership gossip, stream opens)
            // see them as reachable.
            if swarm.is_connected(&ap.peer_id) {
                connected
                    .lock()
                    .expect("connected set mutex poisoned")
                    .insert(ap.peer_id);
            }
        } else {
            // Refresh addresses for existing peers.
            known_peers.insert(ap.peer_id, ap.multiaddrs.clone());
        }
    }

    UpdateReport { added, removed }
}

fn drive_pending_dials(
    swarm: &mut Swarm<Behaviour>,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
) {
    let now = Instant::now();
    let due: Vec<PeerId> = schedules
        .iter()
        .filter_map(|(pid, sched)| sched.next_dial_at.filter(|t| *t <= now).map(|_| *pid))
        .collect();
    for pid in due {
        if let Some(sched) = schedules.get_mut(&pid) {
            sched.next_dial_at = None;
        }
        if swarm.is_connected(&pid) {
            continue;
        }
        if let Some(addrs) = known_peers.get(&pid) {
            let _ = swarm::dial_peer(swarm, pid, addrs.clone());
        }
    }
}

/// Per-substream task for an inbound `/auki/join/0.0.1` request.
///
/// Reads the framed [`JoinRequest`], forwards it to the runtime's
/// owner via a [`JoinEvent`] on the channel, awaits the owner's
/// reply (up to [`JOIN_RESPONSE_TIMEOUT`]), writes the framed
/// [`JoinResponse`] back, closes the substream.
///
/// Errors at any stage are logged to stderr and drop the substream
/// silently — peers retry by opening a fresh substream. (The
/// alternative — surfacing per-substream errors back through the
/// channel — would require the owner to track every in-flight
/// request and provide its own timeout; not worth it.)
async fn handle_inbound_join_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    join_events_tx: mpsc::Sender<JoinEvent>,
) {
    let request = match read_join_request(&mut substream).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("auki-network: join substream from {peer}: read request failed: {e}");
            return;
        }
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    if join_events_tx
        .send(JoinEvent {
            peer,
            request,
            ack: ack_tx,
        })
        .await
        .is_err()
    {
        // Owner has dropped the receiver — no one's listening. Drop
        // the substream silently; the requester sees a closed
        // connection.
        return;
    }

    let response = match tokio::time::timeout(JOIN_RESPONSE_TIMEOUT, ack_rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            // Sender dropped without sending — treat as a reject
            // with a generic reason so the requester gets some
            // signal rather than a silent connection drop.
            JoinResponse::Reject {
                reason: "join handler dropped without replying".into(),
            }
        }
        Err(_) => JoinResponse::Reject {
            reason: format!("join handler timed out after {JOIN_RESPONSE_TIMEOUT:?}"),
        },
    };

    if let Err(e) = write_join_response(&mut substream, &response).await {
        eprintln!("auki-network: join substream to {peer}: write response failed: {e}");
    }
}

/// Open an outbound `/auki/heartbeat/0.0.1` substream to `peer` and
/// run the bidirectional heartbeat loop on it. The accepter calls
/// [`run_heartbeat_pair`] directly with an inbound substream.
///
/// Exits on substream close / fatal I/O error. On exit, emits a
/// `HeartbeatStreamClosed { peer_id }` event so the owner knows this
/// carrier is no longer delivering heartbeat frames.
async fn open_and_run_heartbeat_pair(
    peer: PeerId,
    mut control: Control,
    liveness_tx: mpsc::Sender<PeerLivenessEvent>,
    mut lifeline_rx: watch::Receiver<()>,
    heartbeat_timestamps: HeartbeatTimestampSource,
) {
    let proto = StreamProtocol::try_from_owned(HEARTBEAT_PROTOCOL.to_string())
        .expect("HEARTBEAT_PROTOCOL is a valid libp2p protocol id");
    let mut substream = None;
    for _ in 0..5 {
        let open_fut = control.open_stream(peer, proto.clone());
        tokio::pin!(open_fut);
        tokio::select! {
            biased;
            _ = lifeline_rx.changed() => return,
            r = &mut open_fut => match r {
                Ok(s) => { substream = Some(s); break; }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }
    let Some(substream) = substream else {
        let _ = liveness_tx.try_send(PeerLivenessEvent::HeartbeatStreamClosed { peer_id: peer });
        return;
    };
    run_heartbeat_pair(
        peer,
        substream,
        liveness_tx,
        lifeline_rx,
        heartbeat_timestamps,
    )
    .await;
}

#[derive(Debug, Default)]
struct SentHeartbeatCache {
    sent_at_by_sequence: HashMap<u64, i64>,
    sequence_order: VecDeque<u64>,
}

impl SentHeartbeatCache {
    fn remember(&mut self, sequence: u64, sent_at_clock_ns: i64) {
        if !self.sent_at_by_sequence.contains_key(&sequence) {
            self.sequence_order.push_back(sequence);
        }
        self.sent_at_by_sequence.insert(sequence, sent_at_clock_ns);

        while self.sequence_order.len() > SENT_HEARTBEAT_CACHE_CAPACITY {
            if let Some(oldest) = self.sequence_order.pop_front() {
                self.sent_at_by_sequence.remove(&oldest);
            }
        }
    }

    fn get(&self, sequence: u64) -> Option<i64> {
        self.sent_at_by_sequence.get(&sequence).copied()
    }
}

fn ntp_sample_observation_from_echo(
    peer_id: PeerId,
    heartbeat: &Heartbeat,
    received_at_clock_ns: i64,
    local_clock_id: &str,
    local_clock_hash: &str,
    sent_heartbeats: &SentHeartbeatCache,
) -> Option<HeartbeatNtpSampleObservation> {
    let echo = heartbeat.echo.as_ref()?;
    let local_send_ns = sent_heartbeats.get(echo.sequence)?;
    let sample = compute_ntp_sample(NtpExchange {
        local_send_ns,
        remote_receive_ns: echo.received_at_clock_ns,
        remote_send_ns: heartbeat.sent_at_clock_ns,
        local_receive_ns: received_at_clock_ns,
    })
    .ok()?;

    Some(HeartbeatNtpSampleObservation {
        peer_id,
        local_clock_id: local_clock_id.to_owned(),
        local_clock_hash: local_clock_hash.to_owned(),
        remote_clock_id: heartbeat.clock_id.clone(),
        remote_clock_hash: heartbeat.clock_hash.clone(),
        sample,
    })
}

/// Run the bidirectional heartbeat loop on `substream`. Writes a
/// `Heartbeat` every [`HEARTBEAT_INTERVAL`]; reads continuously and
/// reports each received frame upward. Returns when either side of
/// the substream errors.
async fn run_heartbeat_pair(
    peer: PeerId,
    substream: libp2p::Stream,
    liveness_tx: mpsc::Sender<PeerLivenessEvent>,
    mut lifeline_rx: watch::Receiver<()>,
    heartbeat_timestamps: HeartbeatTimestampSource,
) {
    use futures::AsyncReadExt as _;
    // The substream is bidirectional; split it so we can read on
    // one half and write on the other concurrently.
    let (mut reader, mut writer) = substream.split();

    let mut write_tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut next_sequence: u64 = 1;
    let mut pending_echo: Option<HeartbeatEcho> = None;
    let mut sent_heartbeats = SentHeartbeatCache::default();
    // First tick fires immediately — send a heartbeat right away so
    // the peer sees the liveness signal without waiting an interval.
    loop {
        tokio::select! {
            biased;

            // Runtime is being torn down (NetworkRuntime dropped, or
            // shutdown called). Exit promptly so we stop writing
            // frames against a substream whose connection state is
            // about to vanish at the QUIC / TCP layer regardless.
            _ = lifeline_rx.changed() => break,

            _ = write_tick.tick() => {
                let sequence = next_sequence;
                next_sequence = next_sequence.wrapping_add(1).max(1);
                let sent_at_clock_ns = (heartbeat_timestamps.now_ns)();
                sent_heartbeats.remember(sequence, sent_at_clock_ns);
                let hb = Heartbeat {
                    sent_at_unix_ns: unix_now_ns(),
                    clock_id: heartbeat_timestamps.clock_id.clone(),
                    clock_hash: heartbeat_timestamps.clock_hash.clone(),
                    sequence,
                    sent_at_clock_ns,
                    echo: pending_echo.take(),
                    domain_clock: (heartbeat_timestamps.domain_clock)(),
                };
                // Time the flush. A heartbeat write is normally instant;
                // a slow one means the carrier is congested and our
                // outbound liveness is at risk (the remote may declare us
                // Lost). Surface it for field diagnosis — see #304.
                let write_started = Instant::now();
                let write_result = write_heartbeat(&mut writer, &hb).await;
                let waited = write_started.elapsed();
                if waited >= HEARTBEAT_WRITE_STALL_WARN {
                    let _ = liveness_tx.try_send(PeerLivenessEvent::HeartbeatWriteStalled {
                        peer_id: peer,
                        waited,
                    });
                }
                if write_result.is_err() {
                    break;
                }
            }

            r = read_heartbeat(&mut reader) => {
                match r {
                    Ok(hb) => {
                        let received_at_clock_ns = (heartbeat_timestamps.now_ns)();
                        let sequence = hb.sequence;
                        let ntp_observation = ntp_sample_observation_from_echo(
                            peer,
                            &hb,
                            received_at_clock_ns,
                            &heartbeat_timestamps.clock_id,
                            &heartbeat_timestamps.clock_hash,
                            &sent_heartbeats,
                        );
                        let observation = HeartbeatTimingObservation {
                            peer_id: peer,
                            heartbeat: hb,
                            received_at_clock_ns,
                            local_clock_id: heartbeat_timestamps.clock_id.clone(),
                            local_clock_hash: heartbeat_timestamps.clock_hash.clone(),
                        };
                        pending_echo = Some(HeartbeatEcho {
                            sequence,
                            received_at_clock_ns,
                        });
                        let _ = liveness_tx.try_send(PeerLivenessEvent::HeartbeatReceived {
                            peer_id: peer,
                            observation,
                        });
                        if let Some(observation) = ntp_observation {
                            let _ = liveness_tx.try_send(PeerLivenessEvent::HeartbeatNtpSampleObserved {
                                peer_id: peer,
                                observation,
                            });
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
    let _ = liveness_tx.try_send(PeerLivenessEvent::HeartbeatStreamClosed { peer_id: peer });
}

fn unix_now_ns() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn test_heartbeat_timestamps() -> HeartbeatTimestampSource {
    HeartbeatTimestampSource {
        clock_id: "test/session-monotonic".into(),
        clock_hash: "test-clock-hash".into(),
        now_ns: Arc::new(|| 0),
        domain_clock: Arc::new(|| None),
    }
}

/// Per-substream task for an inbound `/auki/membership/0.0.1` push.
/// Reads exactly one [`MembershipUpdate`] and forwards it to the
/// runtime's owner via [`MembershipEvent`]. Errors are logged and
/// the substream is dropped silently — gossip is fire-and-forget.
async fn handle_inbound_membership_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    membership_events_tx: mpsc::Sender<MembershipEvent>,
) {
    match read_membership_update(&mut substream).await {
        Ok(update) => {
            if membership_events_tx
                .send(MembershipEvent { peer, update })
                .await
                .is_err()
            {
                // Receiver dropped — owner is gone. Nothing to do.
            }
        }
        Err(e) => {
            eprintln!("auki-network: membership substream from {peer}: read failed: {e}");
        }
    }
}

/// Per-substream task for an inbound `/auki/info/0.0.1` request.
/// Reads the framed [`InfoRequest`], forwards it to the runtime's
/// owner via an [`InfoRequestEvent`], awaits the owner's reply (up
/// to [`INFO_RESPONSE_TIMEOUT`]), writes the framed
/// [`InfoResponse`] back, closes the substream.
///
/// Mirrors `handle_inbound_join_substream` in lifecycle — errors
/// at any stage drop the substream silently; the requester sees
/// `UnexpectedEof` on read.
async fn handle_inbound_info_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    info_events_tx: mpsc::Sender<InfoRequestEvent>,
) {
    let request = match read_info_request(&mut substream).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("auki-network: info substream from {peer}: read failed: {e}");
            return;
        }
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    if info_events_tx
        .send(InfoRequestEvent {
            peer,
            request,
            ack: ack_tx,
        })
        .await
        .is_err()
    {
        // Owner has dropped the receiver — drop silently.
        return;
    }

    let response = match tokio::time::timeout(INFO_RESPONSE_TIMEOUT, ack_rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            eprintln!("auki-network: info handler dropped without replying for peer {peer}");
            return;
        }
        Err(_) => {
            eprintln!(
                "auki-network: info handler timed out after {INFO_RESPONSE_TIMEOUT:?} for peer {peer}"
            );
            return;
        }
    };

    if let Err(e) = write_info_response(&mut substream, &response).await {
        eprintln!("auki-network: info substream to {peer}: write response failed: {e}");
    }
}

/// Per-substream task for an inbound `/auki/resources/0.0.1`
/// request. Reads the framed [`ResourcesRequest`], forwards it to the
/// runtime's owner via a [`ResourcesRequestEvent`], awaits the owner's
/// reply (up to [`RESOURCES_RESPONSE_TIMEOUT`]), writes the framed
/// [`ResourcesResponse`] back, closes the substream.
///
/// Errors at any stage drop the substream silently; the requester sees
/// `UnexpectedEof` on read.
async fn handle_inbound_resources_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    resources_events_tx: mpsc::Sender<ResourcesRequestEvent>,
) {
    let request = match read_resources_request(&mut substream).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("auki-network: resources substream from {peer}: read failed: {e}");
            return;
        }
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    if resources_events_tx
        .send(ResourcesRequestEvent {
            peer,
            request,
            ack: ack_tx,
        })
        .await
        .is_err()
    {
        // Owner has dropped the receiver — drop silently.
        return;
    }

    let response = match tokio::time::timeout(RESOURCES_RESPONSE_TIMEOUT, ack_rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            eprintln!("auki-network: resources handler dropped without replying for peer {peer}");
            return;
        }
        Err(_) => {
            eprintln!(
                "auki-network: resources handler timed out after {RESOURCES_RESPONSE_TIMEOUT:?} for peer {peer}"
            );
            return;
        }
    };

    if let Err(e) = write_resources_response(&mut substream, &response).await {
        eprintln!("auki-network: resources substream to {peer}: write response failed: {e}");
    }
}

async fn handle_inbound_resources_v3_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    resources_events_tx: mpsc::Sender<ResourcesRequestEvent>,
    message_router: MessageChannelRouter,
) {
    let request = match read_resources_request_v3(&mut substream).await {
        Ok(request) => request,
        Err(_) => return,
    };

    let wants_v2 = request.variants.is_empty()
        || request
            .variants
            .iter()
            .any(|variant| *variant != ResourceVariantV3::MessageChannel);
    let mut resources = if wants_v2 {
        let variants = request
            .variants
            .iter()
            .filter_map(|variant| match variant {
                ResourceVariantV3::SensorLog => Some(crate::resources_protocol::Variant::SensorLog),
                ResourceVariantV3::PoseLog => Some(crate::resources_protocol::Variant::PoseLog),
                ResourceVariantV3::TimeTransformLog => {
                    Some(crate::resources_protocol::Variant::TimeTransformLog)
                }
                ResourceVariantV3::DetectionLog => {
                    Some(crate::resources_protocol::Variant::DetectionLog)
                }
                ResourceVariantV3::MessageChannel => None,
            })
            .collect();
        let (ack, response) = oneshot::channel();
        if resources_events_tx
            .send(ResourcesRequestEvent {
                peer,
                request: ResourcesRequest { variants },
                ack,
            })
            .await
            .is_err()
        {
            return;
        }
        let response = match tokio::time::timeout(RESOURCES_RESPONSE_TIMEOUT, response).await {
            Ok(Ok(response)) => response,
            _ => return,
        };
        response
            .resources
            .into_iter()
            .map(Box::new)
            .map(ResourceEntryV3::V2)
            .collect()
    } else {
        Vec::new()
    };

    if request.includes(ResourceVariantV3::MessageChannel) {
        resources.extend(
            message_router
                .catalog()
                .into_iter()
                .map(ResourceEntryV3::MessageChannel),
        );
    }
    let response = ResourcesResponseV3 { resources }.filtered(&request);
    let _ = write_resources_response_v3(&mut substream, &response).await;
}

async fn handle_inbound_resources_v4_substream(
    _peer: PeerId,
    mut substream: libp2p::Stream,
    map_catalog_provider: SharedMapCatalogProvider,
) {
    if read_resources_request_v4(&mut substream).await.is_err() {
        return;
    }

    // Clone the provider while holding the lock, then release it before
    // invoking application code or awaiting I/O.
    let provider = map_catalog_provider
        .lock()
        .expect("map_catalog_provider lock")
        .clone();
    let response = provider
        .map(|provider| provider.map_catalog())
        .unwrap_or_else(|| ResourcesResponseV4 {
            resources: Vec::new(),
        });
    let _ = write_resources_response_v4(&mut substream, &response).await;
}

async fn handle_inbound_resources_v5_substream(
    _peer: PeerId,
    mut substream: libp2p::Stream,
    device_model_catalog_provider: SharedDeviceModelCatalogProvider,
) {
    if read_resources_request_v5(&mut substream).await.is_err() {
        return;
    }
    let provider = device_model_catalog_provider
        .lock()
        .expect("device_model_catalog_provider lock")
        .clone();
    let response = provider
        .map(|provider| provider.device_model_catalog())
        .unwrap_or_else(|| ResourcesResponseV5 { resources: Vec::new() });
    let _ = write_resources_response_v5(&mut substream, &response).await;
}

async fn handle_inbound_blob_substream(
    mut substream: libp2p::Stream,
    blob_app_root: SharedBlobAppRoot,
) {
    let Ok(request) = read_blob_request(&mut substream).await else {
        return;
    };
    let app_root = blob_app_root.lock().expect("blob_app_root lock").clone();
    let Some(app_root) = app_root else {
        let meta = BlobResponseMeta { sha256: request.sha256, offset: request.offset, total_size: 0, chunk_len: 0 };
        let _ = write_blob_response(&mut substream, &meta, &[]).await;
        return;
    };
    let Ok(Some(blob)) = auki_registry::get_blob(&app_root, &request.sha256) else {
        let meta = BlobResponseMeta { sha256: request.sha256, offset: request.offset, total_size: 0, chunk_len: 0 };
        let _ = write_blob_response(&mut substream, &meta, &[]).await;
        return;
    };
    if request.offset >= blob.len() as u64 {
        let meta = BlobResponseMeta {
            sha256: request.sha256,
            offset: request.offset,
            total_size: blob.len() as u64,
            chunk_len: 0,
        };
        let _ = write_blob_response(&mut substream, &meta, &[]).await;
        return;
    }
    let start = request.offset as usize;
    let end = (start + request.max_len as usize).min(blob.len());
    let chunk = &blob[start..end];
    let meta = BlobResponseMeta {
        sha256: request.sha256,
        offset: request.offset,
        total_size: blob.len() as u64,
        chunk_len: chunk.len() as u32,
    };
    let _ = write_blob_response(&mut substream, &meta, chunk).await;
}

/// Per-substream task for an inbound `/auki/registries/0.0.1`
/// request. Reads the framed [`RegistryRequest`], forwards it to the
/// runtime's owner via a [`RegistryRequestEvent`], awaits the owner's
/// reply (up to [`REGISTRIES_RESPONSE_TIMEOUT`]), writes the framed
/// [`RegistryResponse`] back, closes the substream.
///
/// Errors at any stage drop the substream silently; the requester sees
/// `UnexpectedEof` on read.
async fn handle_inbound_registry_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    registry_events_tx: mpsc::Sender<RegistryRequestEvent>,
) {
    let request = match read_registry_request(&mut substream).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("auki-network: registry substream from {peer}: read failed: {e}");
            return;
        }
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    if registry_events_tx
        .send(RegistryRequestEvent {
            peer,
            request,
            ack: ack_tx,
        })
        .await
        .is_err()
    {
        // Owner has dropped the receiver — drop silently.
        return;
    }

    let response = match tokio::time::timeout(REGISTRIES_RESPONSE_TIMEOUT, ack_rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            eprintln!("auki-network: registry handler dropped without replying for peer {peer}");
            return;
        }
        Err(_) => {
            eprintln!(
                "auki-network: registry handler timed out after {REGISTRIES_RESPONSE_TIMEOUT:?} for peer {peer}"
            );
            return;
        }
    };

    if let Err(e) = write_registry_response(&mut substream, &response).await {
        eprintln!("auki-network: registry substream to {peer}: write response failed: {e}");
    }
}

/// Reads exactly one diagnostic message and forwards it to the owner.
async fn handle_inbound_diagnostic_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    diagnostic_events_tx: mpsc::Sender<DiagnosticEvent>,
) {
    match read_diagnostic_message(&mut substream).await {
        Ok(message) => {
            if diagnostic_events_tx
                .send(DiagnosticEvent { peer, message })
                .await
                .is_err()
            {
                // Receiver dropped — owner is gone.
            }
        }
        Err(e) => {
            eprintln!("auki-network: diagnostic substream from {peer}: read failed: {e}");
        }
    }
}

/// Shared implementation of `broadcast_membership` reachable from both
/// [`NetworkRuntime::broadcast_membership`] and
/// [`NetworkRuntimeHandle::broadcast_membership`]. Spawns one fire-
/// and-forget task per currently-connected peer that opens an outbound
/// `/auki/membership/0.0.1` substream and writes the update.
fn broadcast_membership_impl(
    stream_control: &Control,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    manager_peer_id: PeerId,
    membership_json: String,
) -> Result<(), BroadcastMembershipError> {
    if membership_json.len() as u64 > crate::membership_protocol::MAX_MEMBERSHIP_FRAME_BYTES as u64
    {
        return Err(BroadcastMembershipError::PayloadTooLarge);
    }
    let peers: Vec<PeerId> = connected
        .lock()
        .expect("connected set mutex poisoned")
        .iter()
        .copied()
        .collect();
    for peer in peers {
        let mut control = stream_control.clone();
        let json = membership_json.clone();
        tokio::spawn(async move {
            let proto = StreamProtocol::try_from_owned(MEMBERSHIP_PROTOCOL.to_string())
                .expect("MEMBERSHIP_PROTOCOL is a valid libp2p stream protocol id");
            match control.open_stream(peer, proto).await {
                Ok(mut substream) => {
                    let msg = MembershipUpdate {
                        manager_peer_id,
                        membership_json: json,
                    };
                    if let Err(e) = write_membership_update(&mut substream, &msg).await {
                        eprintln!(
                            "auki-network: membership broadcast to {peer}: write failed: {e}"
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "auki-network: membership broadcast to {peer}: open_stream failed: {e}"
                    );
                }
            }
        });
    }
    Ok(())
}

fn broadcast_diagnostic_impl(
    stream_control: &Control,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    message: DiagnosticMessage,
) -> Result<(), BroadcastDiagnosticError> {
    if message.topic.len() + message.payload_json.len()
        > crate::diagnostic_protocol::MAX_DIAGNOSTIC_FRAME_BYTES as usize
    {
        return Err(BroadcastDiagnosticError::PayloadTooLarge);
    }
    let peers: Vec<PeerId> = connected
        .lock()
        .expect("connected set mutex poisoned")
        .iter()
        .copied()
        .collect();
    for peer in peers {
        let mut control = stream_control.clone();
        let message = message.clone();
        tokio::spawn(async move {
            let proto = StreamProtocol::try_from_owned(DIAGNOSTIC_PROTOCOL.to_string())
                .expect("DIAGNOSTIC_PROTOCOL is a valid libp2p stream protocol id");
            match control.open_stream(peer, proto).await {
                Ok(mut substream) => {
                    if let Err(e) = write_diagnostic_message(&mut substream, &message).await {
                        eprintln!(
                            "auki-network: diagnostic broadcast to {peer}: write failed: {e}"
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "auki-network: diagnostic broadcast to {peer}: open_stream failed: {e}"
                    );
                }
            }
        });
    }
    Ok(())
}

/// Shared implementation for [`NetworkRuntime::send_join_request`] and
/// [`NetworkRuntimeHandle::send_join_request`]. Opens an outbound
/// `/auki/join/0.0.1` substream via `control`, writes `request`, and
/// reads the response.
async fn send_join_request_impl(
    mut control: Control,
    peer_id: PeerId,
    request: JoinRequest,
) -> Result<JoinResponse, SendJoinRequestError> {
    let proto = StreamProtocol::try_from_owned(JOIN_PROTOCOL.to_string())
        .expect("JOIN_PROTOCOL is a valid libp2p protocol id");

    let open_fut = control.open_stream(peer_id, proto);
    let mut substream = match tokio::time::timeout(JOIN_REQUEST_TIMEOUT, open_fut).await {
        Err(_) => return Err(SendJoinRequestError::Timeout(JOIN_REQUEST_TIMEOUT)),
        Ok(Err(e)) => return Err(SendJoinRequestError::OpenStream(e)),
        Ok(Ok(s)) => s,
    };

    write_join_request(&mut substream, &request)
        .await
        .map_err(SendJoinRequestError::Protocol)?;

    let response = match tokio::time::timeout(
        JOIN_REQUEST_TIMEOUT,
        read_join_response(&mut substream),
    )
    .await
    {
        Err(_) => return Err(SendJoinRequestError::Timeout(JOIN_REQUEST_TIMEOUT)),
        Ok(Err(e)) => return Err(SendJoinRequestError::Protocol(e)),
        Ok(Ok(r)) => r,
    };
    Ok(response)
}

fn schedule_retry(schedules: &mut HashMap<PeerId, PeerSchedule>, peer_id: PeerId) {
    let sched = schedules.entry(peer_id).or_insert(PeerSchedule {
        next_dial_at: None,
        backoff: INITIAL_BACKOFF,
    });
    sched.next_dial_at = Some(Instant::now() + sched.backoff);
    sched.backoff = std::cmp::min(sched.backoff.mul_f32(2.0), MAX_BACKOFF);
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_runtime::decline_all_streams;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EagerRawMessageStream {
        bytes: Vec<u8>,
        offset: usize,
        reads: Arc<AtomicUsize>,
    }

    impl futures::AsyncRead for EagerRawMessageStream {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            output: &mut [u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let remaining = &self.bytes[self.offset..];
            let read = remaining.len().min(output.len());
            output[..read].copy_from_slice(&remaining[..read]);
            self.offset += read;
            std::task::Poll::Ready(Ok(read))
        }
    }

    impl Drop for EagerRawMessageStream {
        fn drop(&mut self) {
            assert_eq!(
                self.reads.load(Ordering::SeqCst),
                0,
                "unknown-peer message bytes must not be consumed"
            );
        }
    }

    async fn build_test_swarm() -> Swarm<Behaviour> {
        let identity = PeerIdentity::from_seed(&[7u8; 32]);
        let cfg = SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            ..SwarmConfig::default()
        };
        build_swarm(&identity, cfg).expect("build_swarm succeeds")
    }

    #[test]
    fn resources_v3_unsupported_protocol_is_explicit_and_never_a_v2_response() {
        let protocol = RESOURCES_V3_PROTOCOL.clone();
        let error = map_resources_v3_open_error(
            libp2p_stream::OpenStreamError::UnsupportedProtocol(protocol),
        );
        assert!(matches!(
            error,
            RequestResourcesV3Error::UnsupportedProtocol
        ));
    }

    async fn wait_for_test_listen_addr(swarm: &mut Swarm<Behaviour>) -> Multiaddr {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                    return address;
                }
            }
        })
        .await
        .expect("listen address did not appear within timeout")
    }

    async fn wait_for_watch_true(receiver: &mut watch::Receiver<bool>) {
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }

    #[tokio::test]
    async fn spawn_with_empty_allow_list_starts_invisible() {
        let swarm = build_test_swarm().await;
        let (
            rt,
            _join_events,
            _liveness,
            _membership_events,
            _info_events,
            _resources_events,
            _registry_events,
            _diagnostic_events,
        ) = NetworkRuntime::spawn(
            swarm,
            vec![],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .expect("spawn succeeds");
        assert!(rt.connected_peers().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn local_peer_id_matches_swarm_identity() {
        let identity = PeerIdentity::from_seed(&[42u8; 32]);
        let cfg = SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            ..SwarmConfig::default()
        };
        let swarm = build_swarm(&identity, cfg).expect("build_swarm succeeds");
        let expected = identity.peer_id();
        let (
            rt,
            _join_events,
            _liveness,
            _membership_events,
            _info_events,
            _resources_events,
            _registry_events,
            _diagnostic_events,
        ) = NetworkRuntime::spawn(
            swarm,
            vec![],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .expect("spawn succeeds");
        assert_eq!(rt.local_peer_id(), expected);
        rt.shutdown();
    }

    #[test]
    fn unknown_peer_gate_drops_an_eager_raw_message_stream_without_reading_it() {
        let reads = Arc::new(AtomicUsize::new(0));
        let stream = EagerRawMessageStream {
            // Complete open and payload-like bytes are already available, as
            // they would be from an eager writer that does not await accept.
            bytes: vec![0, 0, 0, 3, 1, 0xff, 0xff, 0, 0, 0, 5, 3, 1, 2, 3, 4],
            offset: 0,
            reads: reads.clone(),
        };

        assert!(authorize_inbound_message_stream(false, stream).is_none());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn active_message_substreams_are_capped_per_peer_and_globally() {
        let peer = PeerIdentity::from_seed(&[101; 32]).peer_id();
        let mut tasks = ActiveMessageTasks::default();
        for _ in 0..MAX_ACTIVE_MESSAGE_STREAMS_PER_PEER {
            let (revoke, _) = watch::channel(false);
            let handle = tokio::spawn(futures::future::pending());
            let id = tasks.allocate_id(peer).unwrap();
            tasks.track(peer, id, revoke, handle);
        }
        assert!(!tasks.has_capacity(peer));
        assert!(tasks.has_capacity(PeerIdentity::from_seed(&[102; 32]).peer_id()));
        tasks.cancel_all().await;

        for seed in 0..MAX_ACTIVE_MESSAGE_STREAMS {
            let peer = PeerIdentity::from_seed(&[(seed % 200) as u8; 32]).peer_id();
            let (revoke, _) = watch::channel(false);
            let handle = tokio::spawn(futures::future::pending());
            let id = tasks.allocate_id(peer).unwrap();
            tasks.track(peer, id, revoke, handle);
        }
        assert!(!tasks.has_capacity(PeerIdentity::from_seed(&[250; 32]).peer_id()));
        tasks.cancel_all().await;
    }

    #[tokio::test]
    async fn completed_message_task_is_reaped_by_its_stable_id() {
        let peer = PeerIdentity::from_seed(&[103; 32]).peer_id();
        let mut tasks = ActiveMessageTasks::default();
        let (revoke, _) = watch::channel(false);
        let (completion_sent, completion_seen) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let _ = completion_sent.send(());
            let _ = released.await;
        });
        let id = tasks.allocate_id(peer).unwrap();
        tasks.track(peer, id, revoke, handle);
        completion_seen.await.unwrap();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            let _ = release.send(());
        });

        tasks.complete(peer, id).await;
        assert_eq!(tasks.len(), 0);
    }

    #[tokio::test]
    async fn closing_one_of_multiple_connections_keeps_peer_tasks_and_presence() {
        let peer = PeerIdentity::from_seed(&[104; 32]).peer_id();
        let known_peers = HashMap::from([(peer, Vec::new())]);
        let mut schedules = HashMap::from([(
            peer,
            PeerSchedule {
                next_dial_at: None,
                backoff: INITIAL_BACKOFF,
            },
        )]);
        let connected = Arc::new(Mutex::new(HashSet::from([peer])));
        let mut outbound_heartbeat_tasks =
            HashMap::from([(peer, tokio::spawn(futures::future::pending()))]);
        let mut inbound_heartbeat_tasks =
            HashMap::from([(peer, tokio::spawn(futures::future::pending()))]);
        let (liveness_tx, mut liveness_rx) = mpsc::channel(4);

        assert_eq!(
            handle_connection_closed(
                peer,
                1,
                &known_peers,
                &mut schedules,
                &connected,
                &mut outbound_heartbeat_tasks,
                &mut inbound_heartbeat_tasks,
                &liveness_tx,
            ),
            None
        );
        assert!(connected.lock().unwrap().contains(&peer));
        assert!(outbound_heartbeat_tasks.contains_key(&peer));
        assert!(inbound_heartbeat_tasks.contains_key(&peer));
        assert!(matches!(
            liveness_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(schedules[&peer].next_dial_at.is_none());

        assert_eq!(
            handle_connection_closed(
                peer,
                0,
                &known_peers,
                &mut schedules,
                &connected,
                &mut outbound_heartbeat_tasks,
                &mut inbound_heartbeat_tasks,
                &liveness_tx,
            ),
            Some(peer)
        );
        assert!(!connected.lock().unwrap().contains(&peer));
        assert!(!outbound_heartbeat_tasks.contains_key(&peer));
        assert!(!inbound_heartbeat_tasks.contains_key(&peer));
        assert!(matches!(
            liveness_rx.recv().await,
            Some(PeerLivenessEvent::Disconnected { peer_id }) if peer_id == peer
        ));
        assert!(schedules[&peer].next_dial_at.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sustained_message_opens_do_not_starve_revocation_or_task_slot_recovery() {
        use crate::resources_v3_protocol::MessageChannelResource;
        use auki_registry::RegistryRef;

        let receiver_identity = PeerIdentity::from_seed(&[105; 32]);
        let sender_identity = PeerIdentity::from_seed(&[106; 32]);
        let receiver_peer = receiver_identity.peer_id();
        let sender_peer = sender_identity.peer_id();
        let mut receiver_swarm = build_swarm(
            &receiver_identity,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                ..SwarmConfig::default()
            },
        )
        .unwrap();
        let receiver_addr = wait_for_test_listen_addr(&mut receiver_swarm).await;
        let sender_swarm = build_swarm(
            &sender_identity,
            SwarmConfig {
                listen_addresses: vec![],
                ..SwarmConfig::default()
            },
        )
        .unwrap();
        let (
            receiver_runtime,
            _receiver_joins,
            _receiver_liveness,
            _receiver_memberships,
            _receiver_info,
            _receiver_resources,
            _receiver_registries,
            _receiver_diagnostics,
        ) = NetworkRuntime::spawn(
            receiver_swarm,
            vec![AllowedPeer {
                peer_id: sender_peer,
                multiaddrs: vec![],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();
        let (
            sender_runtime,
            _sender_joins,
            _sender_liveness,
            _sender_memberships,
            _sender_info,
            _sender_resources,
            _sender_registries,
            _sender_diagnostics,
        ) = NetworkRuntime::spawn(
            sender_swarm,
            vec![AllowedPeer {
                peer_id: receiver_peer,
                multiaddrs: vec![receiver_addr],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();
        let fairness_resource = MessageChannelResource {
            owner_peer_id: receiver_peer,
            resource_id: "fairness".into(),
            clock: RegistryRef {
                peer_id: receiver_peer.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        };
        let mut registration = receiver_runtime
            .register_message_channel(fairness_resource.clone(), 16)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !sender_runtime.connected_peers().contains(&receiver_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("initial connection");

        let protocol = StreamProtocol::try_from_owned(MESSAGE_PROTOCOL.to_owned()).unwrap();
        let mut held_streams = Vec::new();
        let mut control = sender_runtime.stream_control().clone();
        for _ in 0..MAX_ACTIVE_MESSAGE_STREAMS_PER_PEER {
            let mut stream = control
                .open_stream(receiver_peer, protocol.clone())
                .await
                .unwrap();
            crate::message_protocol::write_open_frame(
                &mut stream,
                receiver_peer,
                "fairness",
                &fairness_resource.clock,
            )
            .await
            .unwrap();
            held_streams.push(stream);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        let (stop_flood_tx, stop_flood_rx) = watch::channel(false);
        let mut flood_tasks = Vec::new();
        for _ in 0..32 {
            let mut flood_control = sender_runtime.stream_control().clone();
            let flood_protocol = protocol.clone();
            let mut stop = stop_flood_rx.clone();
            flood_tasks.push(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = wait_for_watch_true(&mut stop) => return,
                        stream = flood_control.open_stream(receiver_peer, flood_protocol.clone()) => {
                            drop(stream);
                        }
                    }
                }
            }));
        }

        tokio::time::timeout(
            Duration::from_millis(500),
            receiver_runtime.set_allowed_peers(Vec::new()),
        )
        .await
        .expect("revocation must outrank sustained message accepts")
        .unwrap();
        stop_flood_tx.send(true).unwrap();
        for task in flood_tasks {
            task.await.unwrap();
        }
        drop(held_streams);
        tokio::time::timeout(Duration::from_secs(5), async {
            while sender_runtime.connected_peers().contains(&receiver_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disconnect after revocation");

        receiver_runtime
            .set_allowed_peers(vec![AllowedPeer {
                peer_id: sender_peer,
                multiaddrs: vec![],
            }])
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !sender_runtime.connected_peers().contains(&receiver_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("reconnection after revocation");
        let channel = sender_runtime
            .open_message_channel(receiver_peer, "fairness", fairness_resource.clock.clone())
            .await
            .unwrap();
        channel.send("after-flood", 1, vec![1]).await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), registration.recv())
                .await
                .unwrap()
                .unwrap()
                .message
                .r#type,
            "after-flood"
        );

        receiver_runtime.shutdown();
        sender_runtime.shutdown();
        tokio::time::sleep(SHUTDOWN_GRACE + Duration::from_millis(50)).await;
    }

    #[test]
    fn queued_commands_and_completions_precede_more_message_accepts() {
        let peer = PeerIdentity::from_seed(&[107; 32]).peer_id();
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        shutdown_tx.send(()).unwrap();
        let (command_tx, mut command_rx) = mpsc::channel(2);
        let (ack, _ack_rx) = oneshot::channel();
        command_tx
            .try_send(RuntimeCmd::SetAllowedPeers {
                new_peers: Vec::new(),
                ack,
            })
            .unwrap();
        let (done_tx, mut done_rx) = mpsc::unbounded_channel();
        done_tx.send((peer, 7)).unwrap();

        assert!(take_pending_runtime_shutdown(&mut shutdown_rx));
        assert!(matches!(
            take_pending_message_driver_work(&mut command_rx, &mut done_rx),
            Some(PendingMessageDriverWork::Command(
                RuntimeCmd::SetAllowedPeers { .. }
            ))
        ));
        assert!(matches!(
            take_pending_message_driver_work(&mut command_rx, &mut done_rx),
            Some(PendingMessageDriverWork::Completion {
                peer_id,
                task_id: 7
            }) if peer_id == peer
        ));
    }

    #[tokio::test]
    async fn set_allowed_peers_diff_reports_added_and_removed() {
        let swarm = build_test_swarm().await;
        let pid_a = PeerIdentity::from_seed(&[1u8; 32]).peer_id();
        let pid_b = PeerIdentity::from_seed(&[2u8; 32]).peer_id();
        let pid_c = PeerIdentity::from_seed(&[3u8; 32]).peer_id();

        let (
            rt,
            _join_events,
            _liveness,
            _membership_events,
            _info_events,
            _resources_events,
            _registry_events,
            _diagnostic_events,
        ) = NetworkRuntime::spawn(
            swarm,
            vec![
                AllowedPeer {
                    peer_id: pid_a,
                    multiaddrs: vec![],
                },
                AllowedPeer {
                    peer_id: pid_b,
                    multiaddrs: vec![],
                },
            ],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .expect("spawn succeeds");

        // Swap b → c, keep a.
        let report = rt
            .set_allowed_peers(vec![
                AllowedPeer {
                    peer_id: pid_a,
                    multiaddrs: vec![],
                },
                AllowedPeer {
                    peer_id: pid_c,
                    multiaddrs: vec![],
                },
            ])
            .await
            .expect("set_allowed_peers succeeds");

        assert_eq!(report.added, vec![pid_c]);
        assert_eq!(report.removed, vec![pid_b]);
        rt.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn handle_reserves_relay_circuit_addr_after_runtime_spawn() {
        let relay_identity = PeerIdentity::from_seed(&[31u8; 32]);
        let manager_identity = PeerIdentity::from_seed(&[32u8; 32]);

        let mut relay_swarm = build_swarm(
            &relay_identity,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "relay/0".into(),
                enable_relay_server: true,
            },
        )
        .expect("relay swarm builds");
        let relay_addr = wait_for_test_listen_addr(&mut relay_swarm).await;
        relay_swarm.add_external_address(relay_addr.clone());
        let relay_addr_with_peer =
            relay_addr.with(libp2p::multiaddr::Protocol::P2p(relay_identity.peer_id()));
        let advertised_relay_addr: Multiaddr = format!(
            "/ip4/203.0.113.31/tcp/4002/ws/p2p/{}",
            relay_identity.peer_id()
        )
        .parse()
        .unwrap();
        let expected = advertised_relay_addr
            .clone()
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(manager_identity.peer_id()));

        let manager_swarm = build_swarm(
            &manager_identity,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "manager/0".into(),
                enable_relay_server: false,
            },
        )
        .expect("manager swarm builds");
        let (
            rt,
            _join_events,
            _liveness,
            _membership_events,
            _info_events,
            _resources_events,
            _registry_events,
            _diagnostic_events,
        ) = NetworkRuntime::spawn(
            manager_swarm,
            vec![],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .expect("spawn succeeds");
        let handle = rt.handle();

        let circuit_addr = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::select! {
                reservation = handle.reserve_relay_circuit_addr(
                    relay_addr_with_peer,
                    advertised_relay_addr,
                    Duration::from_secs(10),
                ) => reservation,
                _ = async {
                    loop {
                        let _ = relay_swarm.next().await;
                    }
                } => unreachable!("relay swarm stream ended"),
            }
        })
        .await
        .expect("runtime relay reservation timed out")
        .expect("runtime relay reservation succeeds");

        assert_eq!(circuit_addr, expected);
        rt.shutdown();
    }

    #[test]
    fn heartbeat_received_event_carries_timing_observation() {
        let peer_id = PeerIdentity::from_seed(&[9u8; 32]).peer_id();
        let heartbeat = Heartbeat {
            sent_at_unix_ns: 1_715_423_400_000_000_000,
            clock_id: "12D3KooWPeer/session-123/monotonic".into(),
            clock_hash: "remote-clock-hash".into(),
            sequence: 7,
            sent_at_clock_ns: 10_000,
            echo: None,
            domain_clock: None,
        };
        let observation = HeartbeatTimingObservation {
            peer_id,
            heartbeat: heartbeat.clone(),
            received_at_clock_ns: 10_150,
            local_clock_id: "12D3KooWLocal/session-456/monotonic".into(),
            local_clock_hash: "local-clock-hash".into(),
        };

        let event = PeerLivenessEvent::HeartbeatReceived {
            peer_id,
            observation: observation.clone(),
        };

        match event {
            PeerLivenessEvent::HeartbeatReceived {
                peer_id: event_peer,
                observation: observed,
            } => {
                assert_eq!(event_peer, peer_id);
                assert_eq!(observed, observation);
                assert_eq!(observed.heartbeat, heartbeat);
                assert_eq!(observed.received_at_clock_ns, 10_150);
            }
            other => panic!("expected HeartbeatReceived, got {other:?}"),
        }
    }

    #[test]
    fn heartbeat_echo_builds_ntp_sample_from_remembered_sent_sequence() {
        let peer_id = PeerIdentity::from_seed(&[10u8; 32]).peer_id();
        let mut sent = SentHeartbeatCache::default();
        sent.remember(6, 1_000);
        let heartbeat = Heartbeat {
            sent_at_unix_ns: 1_715_423_400_000_000_000,
            clock_id: "12D3KooWPeer/session-123/monotonic".into(),
            clock_hash: "remote-clock-hash".into(),
            sequence: 7,
            sent_at_clock_ns: 1_001_080,
            echo: Some(HeartbeatEcho {
                sequence: 6,
                received_at_clock_ns: 1_001_050,
            }),
            domain_clock: None,
        };

        let observation = ntp_sample_observation_from_echo(
            peer_id,
            &heartbeat,
            1_130,
            "12D3KooWLocal/session-456/monotonic",
            "local-clock-hash",
            &sent,
        )
        .expect("echoed sequence should yield sample");

        assert_eq!(observation.peer_id, peer_id);
        assert_eq!(
            observation.local_clock_id,
            "12D3KooWLocal/session-456/monotonic"
        );
        assert_eq!(observation.remote_clock_id, heartbeat.clock_id);
        assert_eq!(observation.sample.offset_ns, 1_000_000);
        assert_eq!(observation.sample.uncertainty_ns, 100);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_peer_fetches_live_map_catalog_v4() {
        use crate::resources_v4_protocol::MapLogResource;
        use auki_registry::RegistryRef;

        #[derive(Clone)]
        struct TestMapCatalog(ResourcesResponseV4);

        impl MapCatalogProvider for TestMapCatalog {
            fn map_catalog(&self) -> ResourcesResponseV4 {
                self.0.clone()
            }
        }

        let producer_identity = PeerIdentity::from_seed(&[79; 32]);
        let consumer_identity = PeerIdentity::from_seed(&[80; 32]);
        let producer_peer = producer_identity.peer_id();
        let consumer_peer = consumer_identity.peer_id();

        let mut producer_swarm = build_swarm(
            &producer_identity,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                ..SwarmConfig::default()
            },
        )
        .unwrap();
        let producer_addr = wait_for_test_listen_addr(&mut producer_swarm).await;
        let consumer_swarm = build_swarm(
            &consumer_identity,
            SwarmConfig {
                listen_addresses: vec![],
                ..SwarmConfig::default()
            },
        )
        .unwrap();

        let (
            producer_runtime,
            _producer_joins,
            _producer_liveness,
            _producer_memberships,
            _producer_info,
            _producer_resources,
            _producer_registries,
            _producer_diagnostics,
        ) = NetworkRuntime::spawn(
            producer_swarm,
            vec![AllowedPeer {
                peer_id: consumer_peer,
                multiaddrs: vec![],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();
        let (
            consumer_runtime,
            _consumer_joins,
            _consumer_liveness,
            _consumer_memberships,
            _consumer_info,
            _consumer_resources,
            _consumer_registries,
            _consumer_diagnostics,
        ) = NetworkRuntime::spawn(
            consumer_swarm,
            vec![AllowedPeer {
                peer_id: producer_peer,
                multiaddrs: vec![producer_addr],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();

        let expected = ResourcesResponseV4 {
            resources: vec![MapLogResource {
                source_peer_id: producer_peer.to_string(),
                writer_peer_id: producer_peer.to_string(),
                resource_id: "occupancy".into(),
                map: RegistryRef {
                    peer_id: producer_peer.to_string(),
                    id: "occupancy".into(),
                    hash: "map-hash".into(),
                },
                clock: RegistryRef {
                    peer_id: producer_peer.to_string(),
                    id: "session/monotonic".into(),
                    hash: "clock-hash".into(),
                },
            }],
        };
        producer_runtime.set_map_catalog_provider(Arc::new(TestMapCatalog(expected.clone())));

        tokio::time::timeout(Duration::from_secs(5), async {
            while !consumer_runtime.connected_peers().contains(&producer_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("peers connect");

        let received = consumer_runtime
            .request_map_catalog(producer_peer)
            .await
            .unwrap();
        assert_eq!(received, expected);

        producer_runtime.shutdown();
        consumer_runtime.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_peer_sends_distinct_opaque_messages_with_transport_ack() {
        use crate::resources_v3_protocol::{
            MessageChannelResource, ResourceEntry as V3ResourceEntry, ResourceVariant,
            ResourcesRequest as V3ResourcesRequest,
        };
        use auki_registry::RegistryRef;

        let receiver_identity = PeerIdentity::from_seed(&[81; 32]);
        let sender_identity = PeerIdentity::from_seed(&[82; 32]);
        let receiver_peer = receiver_identity.peer_id();
        let sender_peer = sender_identity.peer_id();

        let mut receiver_swarm = build_swarm(
            &receiver_identity,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                ..SwarmConfig::default()
            },
        )
        .unwrap();
        let receiver_addr = wait_for_test_listen_addr(&mut receiver_swarm).await;
        let sender_swarm = build_swarm(
            &sender_identity,
            SwarmConfig {
                listen_addresses: vec![],
                ..SwarmConfig::default()
            },
        )
        .unwrap();

        let (
            receiver_runtime,
            _receiver_joins,
            _receiver_liveness,
            _receiver_memberships,
            _receiver_info,
            _receiver_resources,
            _receiver_registries,
            _receiver_diagnostics,
        ) = NetworkRuntime::spawn(
            receiver_swarm,
            vec![AllowedPeer {
                peer_id: sender_peer,
                multiaddrs: vec![],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();
        let (
            sender_runtime,
            _sender_joins,
            _sender_liveness,
            _sender_memberships,
            _sender_info,
            _sender_resources,
            _sender_registries,
            _sender_diagnostics,
        ) = NetworkRuntime::spawn(
            sender_swarm,
            vec![AllowedPeer {
                peer_id: receiver_peer,
                multiaddrs: vec![receiver_addr],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();

        let live_events_resource = MessageChannelResource {
            owner_peer_id: receiver_peer,
            resource_id: "live-events".into(),
            clock: RegistryRef {
                peer_id: receiver_peer.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        };
        let mut receiver: crate::MessageChannelRegistration = receiver_runtime
            .register_message_channel(live_events_resource.clone(), 16)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            while !sender_runtime.connected_peers().contains(&receiver_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("peers connect");

        let catalog = sender_runtime
            .request_resources_catalog_v3_with(
                receiver_peer,
                V3ResourcesRequest {
                    variants: vec![ResourceVariant::MessageChannel],
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            catalog.resources.as_slice(),
            [V3ResourceEntry::MessageChannel(row)]
                if row.owner_peer_id == receiver_peer && row.resource_id == "live-events"
        ));

        let channel = sender_runtime
            .open_message_channel(
                receiver_peer,
                "live-events",
                live_events_resource.clock.clone(),
            )
            .await
            .unwrap();

        // Neither send waits for application-level consumption. Each resolves
        // when the receiver runtime queues the frame and returns its transport ack.
        channel
            .send("vendor/type-a", 100, vec![0x00, 0xff])
            .await
            .unwrap();
        channel
            .send("another.type/b", 101, vec![0xde, 0xad, 0xbe, 0xef])
            .await
            .unwrap();

        let first = receiver.recv().await.unwrap();
        let second = receiver.recv().await.unwrap();
        assert_eq!(first.sender, sender_peer);
        assert_eq!(first.message.r#type, "vendor/type-a");
        assert_eq!(first.message.payload, vec![0x00, 0xff]);
        assert_eq!(second.sender, sender_peer);
        assert_eq!(second.message.r#type, "another.type/b");
        assert_eq!(second.message.payload, vec![0xde, 0xad, 0xbe, 0xef]);

        // Receiver-side membership revocation is synchronous with respect to
        // active message streams: after this returns, no later frame can be
        // delivered or acknowledged.
        receiver_runtime
            .set_allowed_peers(Vec::new())
            .await
            .unwrap();
        let revoked_send = tokio::time::timeout(
            Duration::from_secs(1),
            channel.send("after-revocation", 102, vec![1]),
        )
        .await
        .expect("peer revocation closes the channel promptly");
        assert!(revoked_send.is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), receiver.recv())
                .await
                .is_err()
        );

        drop(channel);
        receiver_runtime.shutdown();
        sender_runtime.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_peer_is_rejected_before_it_can_deliver_a_message_payload() {
        use crate::resources_v3_protocol::MessageChannelResource;
        use auki_datatypes::message::Message;
        use auki_registry::RegistryRef;
        use futures::AsyncWriteExt;

        let receiver_identity = PeerIdentity::from_seed(&[83; 32]);
        let member_identity = PeerIdentity::from_seed(&[84; 32]);
        let unknown_identity = PeerIdentity::from_seed(&[85; 32]);
        let receiver_peer = receiver_identity.peer_id();

        let mut receiver_swarm = build_swarm(
            &receiver_identity,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                ..SwarmConfig::default()
            },
        )
        .unwrap();
        let receiver_addr = wait_for_test_listen_addr(&mut receiver_swarm).await;
        let unknown_swarm = build_swarm(
            &unknown_identity,
            SwarmConfig {
                listen_addresses: vec![],
                ..SwarmConfig::default()
            },
        )
        .unwrap();

        let (
            receiver_runtime,
            _receiver_joins,
            _receiver_liveness,
            _receiver_memberships,
            _receiver_info,
            _receiver_resources,
            _receiver_registries,
            _receiver_diagnostics,
        ) = NetworkRuntime::spawn(
            receiver_swarm,
            vec![AllowedPeer {
                peer_id: member_identity.peer_id(),
                multiaddrs: vec![],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();
        let (
            unknown_runtime,
            _unknown_joins,
            _unknown_liveness,
            _unknown_memberships,
            _unknown_info,
            _unknown_resources,
            _unknown_registries,
            _unknown_diagnostics,
        ) = NetworkRuntime::spawn(
            unknown_swarm,
            vec![AllowedPeer {
                peer_id: receiver_peer,
                multiaddrs: vec![receiver_addr],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();

        let private_resource = MessageChannelResource {
            owner_peer_id: receiver_peer,
            resource_id: "private".into(),
            clock: RegistryRef {
                peer_id: receiver_peer.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        };
        let mut receiver = receiver_runtime
            .register_message_channel(private_resource.clone(), 16)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !unknown_runtime.connected_peers().contains(&receiver_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("transport connection forms before protocol gate");

        let mut control = unknown_runtime.stream_control().clone();
        let protocol = StreamProtocol::try_from_owned(MESSAGE_PROTOCOL.to_owned()).unwrap();
        let mut raw_stream = tokio::time::timeout(
            Duration::from_secs(1),
            control.open_stream(receiver_peer, protocol),
        )
        .await
        .expect("raw message stream opens")
        .expect("transport permits the connection before protocol authorization");
        let mut eager_bytes = Vec::new();
        crate::message_protocol::write_open_frame(
            &mut eager_bytes,
            receiver_peer,
            "private",
            &private_resource.clock,
        )
        .await
        .unwrap();
        crate::message_protocol::write_message_frame(
            &mut eager_bytes,
            1,
            &Message {
                r#type: "must-not-decode".into(),
                timestamp_ns: 77,
                payload: vec![0xde, 0xad, 0xbe, 0xef],
            },
        )
        .await
        .unwrap();
        // One eager write puts the open header and payload bytes in flight
        // without waiting for an open response. The receiver's known-peer
        // gate must drop the substream before either frame is consumed.
        let _ = raw_stream.write_all(&eager_bytes).await;
        let _ = raw_stream.flush().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(200), receiver.recv())
                .await
                .is_err()
        );

        receiver_runtime.shutdown();
        unknown_runtime.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disconnected_send_fails_and_is_not_replayed_after_reconnect() {
        use crate::resources_v3_protocol::MessageChannelResource;
        use auki_registry::RegistryRef;

        let receiver_identity = PeerIdentity::from_seed(&[86; 32]);
        let sender_identity = PeerIdentity::from_seed(&[87; 32]);
        let receiver_peer = receiver_identity.peer_id();
        let sender_peer = sender_identity.peer_id();
        let mut receiver_swarm = build_swarm(
            &receiver_identity,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                ..SwarmConfig::default()
            },
        )
        .unwrap();
        let receiver_addr = wait_for_test_listen_addr(&mut receiver_swarm).await;
        let sender_swarm = build_swarm(
            &sender_identity,
            SwarmConfig {
                listen_addresses: vec![],
                ..SwarmConfig::default()
            },
        )
        .unwrap();

        let (
            receiver_runtime,
            _receiver_joins,
            mut receiver_liveness,
            _receiver_memberships,
            _receiver_info,
            _receiver_resources,
            _receiver_registries,
            _receiver_diagnostics,
        ) = NetworkRuntime::spawn(
            receiver_swarm,
            vec![AllowedPeer {
                peer_id: sender_peer,
                multiaddrs: vec![],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();
        let (
            sender_runtime,
            _sender_joins,
            _sender_liveness,
            _sender_memberships,
            _sender_info,
            _sender_resources,
            _sender_registries,
            _sender_diagnostics,
        ) = NetworkRuntime::spawn(
            sender_swarm,
            vec![AllowedPeer {
                peer_id: receiver_peer,
                multiaddrs: vec![receiver_addr.clone()],
            }],
            decline_all_streams(),
            test_heartbeat_timestamps(),
        )
        .unwrap();
        let ephemeral_resource = MessageChannelResource {
            owner_peer_id: receiver_peer,
            resource_id: "ephemeral".into(),
            clock: RegistryRef {
                peer_id: receiver_peer.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        };
        let mut receiver = receiver_runtime
            .register_message_channel(ephemeral_resource.clone(), 16)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            while !sender_runtime.connected_peers().contains(&receiver_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("initial connection");
        let channel = sender_runtime
            .open_message_channel(receiver_peer, "ephemeral", ephemeral_resource.clock.clone())
            .await
            .unwrap();

        sender_runtime.set_allowed_peers(Vec::new()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(
                    receiver_liveness.recv().await,
                    Some(PeerLivenessEvent::Disconnected { peer_id }) if peer_id == sender_peer
                ) {
                    break;
                }
            }
        })
        .await
        .expect("disconnect event");
        assert!(
            tokio::time::timeout(
                Duration::from_secs(1),
                channel.send("not-replayed", 200, vec![9, 8, 7]),
            )
            .await
            .expect("failed send resolves promptly")
            .is_err()
        );

        sender_runtime
            .set_allowed_peers(vec![AllowedPeer {
                peer_id: receiver_peer,
                multiaddrs: vec![receiver_addr],
            }])
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !sender_runtime.connected_peers().contains(&receiver_peer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("reconnect");
        assert!(
            tokio::time::timeout(Duration::from_millis(250), receiver.recv())
                .await
                .is_err(),
            "failed live send must not be replayed after reconnect"
        );

        receiver_runtime.shutdown();
        sender_runtime.shutdown();
    }
}
