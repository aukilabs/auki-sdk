use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use auki_datatypes::camera::CameraFrame;
use auki_protocols::{
    blob::v1::{
        BlobClient, BlobEndpoint, BlobProvider, BlobProviderError, BlobProviderFuture, BlobRequest,
        ProvidedBlobChunk,
    },
    catalog::{
        CatalogClient, CatalogEndpoint, CatalogProvider,
        v2::{SensorKind, VariantContent},
        v3::{self as catalog_v3, ResourceEntry, ResourceVariant},
        v4 as catalog_v4,
    },
    info::{InfoClient, InfoEndpoint, v1::AuthenticatedParticipantInfo},
    message::{MessageChannelResource, MessageClient, MessageEndpoint, MessageEvent},
    registry::{
        RegistryClient, RegistryEndpoint,
        v3::{RegistryRequest, RegistryResponse},
    },
    stream::{
        StreamClient, StreamDispatch, StreamEndpoint, StreamEntry, StreamItem, SubscriptionEntries,
        v2::{DeclineReason, ReadFrom, StreamRequest},
    },
};
use auki_registry::{
    AxisDirection, ClockBody, ClockRegistryEntry, FrameRegistryEntry, Handedness, LengthUnit,
    Scope, SensorBody, SensorRegistryEntry,
};
use auki_sdk::{
    AukiDiscovery, AukiDiscoveryCandidate, AukiDiscoverySource, AukiPeer, AukiPeerProtocols,
    AuthenticatedPeer, Multiaddr, PeerId,
};
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::contract::{
    APP, APP_VERSION, CAMERA_CLOCK_ID, CAMERA_CONTROL_RESOURCE_ID, CAMERA_FRAME_ID, CAMERA_HEIGHT,
    CAMERA_RATE_HZ, CAMERA_RESOURCE_ID, CAMERA_WIDTH, CameraMetadata, CameraRole, MAX_BLOB_BYTES,
    PeerCard, PeerRoutes, camera_catalog, control_channel, decode_snapshot_ready,
    decode_snapshot_request, deterministic_jpeg, encode_snapshot_ready, encode_snapshot_request,
    metadata, protocol_ids_for_role, reply_channel, sha256_hex, stream_manifest,
};

const MAX_STAGED_BLOBS: usize = 8;
const MAX_PENDING_SNAPSHOTS: usize = 16;
const MESSAGE_QUEUE_CAPACITY: usize = 16;
const CAMERA_EVENT_QUEUE_CAPACITY: usize = 64;
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(45);
const FRAME_PERIOD: Duration = Duration::from_millis(1_000 / CAMERA_RATE_HZ as u64);
const LIVE_EXERCISE_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
const LIVE_EXERCISE_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const LIVE_EXERCISE_PAUSE_QUIET: Duration = Duration::from_millis(800);
const LIVE_EXERCISE_INITIAL_FRAMES: usize = 2;
const LIVE_EXERCISE_MAX_IN_FLIGHT_FRAMES: usize = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CameraEvent {
    ApprovalRequired {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    ControlReceived {
        control: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    SnapshotStaged {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "peerId")]
        peer_id: String,
        sha256: String,
        size: usize,
    },
    RuntimeError {
        error: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPeer {
    pub peer_id: String,
    pub routes: Vec<String>,
    pub served_protocols: Vec<String>,
    pub expires_at: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewReport {
    pub target_peer_id: String,
    pub checks: BTreeMap<String, bool>,
    pub frames: usize,
    pub frame_sha256: String,
    pub frame_bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotReport {
    pub request_id: String,
    pub target_peer_id: String,
    pub sha256: String,
    pub size: usize,
}

/// One bounded end-to-end report produced while a single Stream subscription
/// remains open across pause, resume, and snapshot controls.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveExerciseReport {
    pub target_peer_id: String,
    pub checks: BTreeMap<String, bool>,
    pub initial_frames: usize,
    pub paused_in_flight_frames: usize,
    pub pause_quiet_ms: u64,
    pub pre_pause_sequence: u64,
    pub resumed_sequence: u64,
    pub total_frames: usize,
    pub frame_sha256: String,
    pub frame_bytes: usize,
    pub snapshot: SnapshotReport,
}

#[derive(Clone, Copy)]
enum LiveExercisePhase {
    Initial,
    PausedInFlight,
    Resumed,
}

#[derive(Debug, Eq, PartialEq)]
struct LiveFrameObservation {
    sequence: u64,
    sha256: String,
    bytes: usize,
}

#[derive(Default)]
struct LiveFrameTracker {
    last_sequence: Option<u64>,
    total_frames: usize,
    paused_in_flight_frames: usize,
}

impl LiveFrameTracker {
    fn observe(
        &mut self,
        entry: StreamEntry<CameraFrame>,
        phase: LiveExercisePhase,
    ) -> Result<LiveFrameObservation> {
        ensure!(
            entry.payload.dynamic_intrinsics.is_none(),
            "Camera frame unexpectedly overrides the locked no-calibration contract"
        );
        ensure_jpeg(&entry.payload.frame)?;
        if let Some(previous) = self.last_sequence {
            ensure!(
                entry.seq > previous,
                "Camera Stream sequence did not advance: {previous} -> {}",
                entry.seq
            );
        }
        if matches!(phase, LiveExercisePhase::PausedInFlight) {
            ensure!(
                self.paused_in_flight_frames < LIVE_EXERCISE_MAX_IN_FLIGHT_FRAMES,
                "Camera emitted more than one in-flight frame after pause"
            );
            self.paused_in_flight_frames += 1;
        }
        self.last_sequence = Some(entry.seq);
        self.total_frames += 1;
        Ok(LiveFrameObservation {
            sequence: entry.seq,
            sha256: sha256_hex(&entry.payload.frame),
            bytes: entry.payload.frame.len(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct RemoteCamera {
    pub peer_id: PeerId,
    pub route: Multiaddr,
    pub info: AuthenticatedParticipantInfo,
    pub sensor: SensorRegistryEntry,
    pub clock: ClockRegistryEntry,
    pub frame: FrameRegistryEntry,
    pub control_channel: MessageChannelResource,
}

struct PendingSnapshot {
    registration_id: Uuid,
    peer_id: PeerId,
    route: Multiaddr,
    result: oneshot::Sender<Result<SnapshotReport, String>>,
}

struct PendingSnapshotGuard {
    state: Arc<SharedState>,
    request_id: String,
    registration_id: Uuid,
}

impl Drop for PendingSnapshotGuard {
    fn drop(&mut self) {
        let mut snapshots = lock(&self.state.pending_snapshots);
        if snapshots
            .get(&self.request_id)
            .is_some_and(|pending| pending.registration_id == self.registration_id)
        {
            snapshots.remove(&self.request_id);
        }
    }
}

#[derive(Clone)]
struct LiveFrame {
    bytes: Arc<[u8]>,
}

struct SharedState {
    role: CameraRole,
    domain_id: Uuid,
    local_peer_id: PeerId,
    allowed: Mutex<HashSet<PeerId>>,
    pending_approvals: Mutex<HashSet<PeerId>>,
    blobs: Mutex<VecDeque<(String, Arc<[u8]>)>>,
    pending_snapshots: Mutex<HashMap<String, PendingSnapshot>>,
    latest_frame: Mutex<LiveFrame>,
    paused: AtomicBool,
    camera_available: AtomicBool,
    events: mpsc::Sender<CameraEvent>,
}

impl SharedState {
    fn same_domain(&self, peer: &AuthenticatedPeer) -> bool {
        peer.domain_ids.contains(&self.domain_id)
    }

    fn allowed(&self, peer: &AuthenticatedPeer) -> bool {
        self.same_domain(peer) && lock(&self.allowed).contains(&peer.peer_id)
    }

    fn request_approval(&self, peer: &AuthenticatedPeer) {
        if self.same_domain(peer) && lock(&self.pending_approvals).insert(peer.peer_id) {
            self.emit(CameraEvent::ApprovalRequired {
                peer_id: peer.peer_id.to_string(),
            });
        }
    }

    fn emit(&self, event: CameraEvent) {
        // Camera events are observational. Never let a slow JSONL/UI consumer
        // apply transport backpressure or grow process memory without bound.
        let _ = self.events.try_send(event);
    }

    fn stage_blob(&self, bytes: Vec<u8>) -> Result<(String, usize)> {
        ensure!(!bytes.is_empty(), "cannot stage an empty snapshot");
        ensure!(
            bytes.len() <= MAX_BLOB_BYTES,
            "snapshot exceeds the Camera Mesh limit"
        );
        let sha256 = sha256_hex(&bytes);
        let size = bytes.len();
        let mut blobs = lock(&self.blobs);
        if !blobs.iter().any(|(known, _)| known == &sha256) {
            if blobs.len() == MAX_STAGED_BLOBS {
                blobs.pop_front();
            }
            blobs.push_back((sha256.clone(), Arc::from(bytes)));
        }
        Ok((sha256, size))
    }

    fn latest_frame(&self) -> LiveFrame {
        lock(&self.latest_frame).clone()
    }

    fn replace_frame(&self, bytes: Vec<u8>) -> Result<()> {
        validate_camera_jpeg(&bytes)?;
        *lock(&self.latest_frame) = LiveFrame {
            bytes: Arc::from(bytes),
        };
        self.camera_available.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Clone)]
struct CameraCatalogProvider {
    state: Arc<SharedState>,
    resources: catalog_v3::ResourcesResponse,
}

impl CatalogProvider for CameraCatalogProvider {
    fn resources(
        &self,
        requester: &AuthenticatedPeer,
        _request: &catalog_v3::ResourcesRequest,
    ) -> catalog_v3::ResourcesResponse {
        if self.state.role != CameraRole::Publisher || !self.state.same_domain(requester) {
            return catalog_v3::ResourcesResponse { resources: vec![] };
        }
        if !self.state.allowed(requester) {
            self.state.request_approval(requester);
            return catalog_v3::ResourcesResponse { resources: vec![] };
        }
        if !self.state.camera_available.load(Ordering::SeqCst) {
            return catalog_v3::ResourcesResponse { resources: vec![] };
        }
        self.resources.clone()
    }

    fn maps(&self, _requester: &AuthenticatedPeer) -> catalog_v4::ResourcesResponse {
        catalog_v4::ResourcesResponse { resources: vec![] }
    }
}

#[derive(Clone)]
struct CameraBlobProvider {
    state: Arc<SharedState>,
}

impl BlobProvider for CameraBlobProvider {
    fn provide<'a>(
        &'a self,
        remote_peer: &'a AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a> {
        Box::pin(async move {
            if !self.state.allowed(remote_peer) {
                return Ok(None);
            }
            let bytes = lock(&self.state.blobs)
                .iter()
                .find(|(sha256, _)| sha256 == &request.sha256)
                .map(|(_, bytes)| Arc::clone(bytes));
            let Some(bytes) = bytes else {
                return Ok(None);
            };
            let start = usize::try_from(request.offset)
                .map_err(|_| BlobProviderError::new("blob offset does not fit this platform"))?;
            if start > bytes.len() {
                return Err(BlobProviderError::new("blob offset exceeds snapshot size"));
            }
            let requested = usize::try_from(request.max_len)
                .map_err(|_| BlobProviderError::new("blob range does not fit this platform"))?;
            let end = start.saturating_add(requested).min(bytes.len());
            Ok(Some(ProvidedBlobChunk::new(
                bytes.len() as u64,
                bytes[start..end].to_vec(),
            )))
        })
    }
}

#[derive(Clone)]
struct ProtocolClients {
    info: InfoClient,
    catalog: CatalogClient,
    registry: RegistryClient,
    blob: BlobClient,
    message: MessageClient,
    stream: StreamClient,
}

pub struct CameraProtocols {
    role: CameraRole,
    card: PeerCard,
    metadata: CameraMetadata,
    state: Arc<SharedState>,
    remote_cache: Mutex<HashMap<(PeerId, String), RemoteCamera>>,
    clients: ProtocolClients,
    info: InfoEndpoint,
    catalog: CatalogEndpoint,
    registry: RegistryEndpoint,
    blob: BlobEndpoint,
    message: MessageEndpoint,
    stream: Option<StreamEndpoint>,
    message_task: tokio::task::JoinHandle<()>,
}

impl CameraProtocols {
    pub async fn mount(
        peer: &AukiPeer,
        role: CameraRole,
        display_name: impl Into<String>,
    ) -> Result<(Self, mpsc::Receiver<CameraEvent>)> {
        let local_peer_id = peer.peer_id();
        let routes = peer_routes(peer)?;
        Self::mount_context(
            peer.protocols(),
            local_peer_id,
            peer.domain_id(),
            routes,
            role,
            display_name,
            "native",
            deterministic_jpeg()?,
        )
        .await
    }

    /// Mount Camera Mesh on an already-running peer context.
    ///
    /// Platform bindings use this seam to keep the application protocol and
    /// exact-viewer approval policy in Rust while supplying platform-owned
    /// camera frames.
    #[allow(clippy::too_many_arguments)]
    pub async fn mount_context(
        protocols: AukiPeerProtocols,
        local_peer_id: PeerId,
        domain_id: Uuid,
        routes: PeerRoutes,
        role: CameraRole,
        display_name: impl Into<String>,
        runtime: impl Into<String>,
        initial_frame: Vec<u8>,
    ) -> Result<(Self, mpsc::Receiver<CameraEvent>)> {
        validate_camera_jpeg(&initial_frame)?;
        let session_id = new_session_id();
        let metadata = metadata(local_peer_id, &session_id);
        let runtime = runtime.into();
        ensure!(!runtime.is_empty(), "camera runtime must not be empty");
        let card = PeerCard {
            version: 1,
            runtime: runtime.clone(),
            domain_id: domain_id.to_string(),
            peer_id: local_peer_id.to_string(),
            protocols: protocol_ids_for_role(role),
            routes,
        };
        let initial_frame = Arc::<[u8]>::from(initial_frame);
        let (event_tx, event_rx) = mpsc::channel(CAMERA_EVENT_QUEUE_CAPACITY);
        let state = Arc::new(SharedState {
            role,
            domain_id,
            local_peer_id,
            allowed: Mutex::new(HashSet::new()),
            pending_approvals: Mutex::new(HashSet::new()),
            blobs: Mutex::new(VecDeque::new()),
            pending_snapshots: Mutex::new(HashMap::new()),
            latest_frame: Mutex::new(LiveFrame {
                bytes: Arc::clone(&initial_frame),
            }),
            paused: AtomicBool::new(false),
            camera_available: AtomicBool::new(role == CameraRole::Publisher),
            events: event_tx,
        });
        let display_name = display_name.into();

        let info_state = Arc::clone(&state);
        let info_metadata = metadata.clone();
        let info = InfoEndpoint::mount(protocols.clone(), move |requester: &AuthenticatedPeer| {
            info_state
                .same_domain(requester)
                .then(|| AuthenticatedParticipantInfo {
                    app: APP.into(),
                    app_version: APP_VERSION.into(),
                    name: display_name.clone(),
                    session_id: session_id.clone(),
                    session_clock_id: info_metadata.clock_ref.id.clone(),
                    session_clock_hash: info_metadata.clock_ref.hash.clone(),
                    session_now_ns: utc_now_ns_u64(),
                    peer_id: local_peer_id,
                    app_instance: format!("{runtime}/{}", role.as_str()),
                })
        })?;
        let catalog = CatalogEndpoint::mount(
            protocols.clone(),
            CameraCatalogProvider {
                state: Arc::clone(&state),
                resources: camera_catalog(local_peer_id, &metadata),
            },
        )?;
        let registry_state = Arc::clone(&state);
        let registry_metadata = metadata.clone();
        let registry = RegistryEndpoint::mount(
            protocols.clone(),
            move |requester: &AuthenticatedPeer, request: &RegistryRequest| {
                if registry_state.allowed(requester) {
                    registry_metadata.response(request)
                } else {
                    RegistryResponse::Error {
                        reason: "access_denied".into(),
                    }
                }
            },
        )?;
        let blob = BlobEndpoint::mount(
            protocols.clone(),
            CameraBlobProvider {
                state: Arc::clone(&state),
            },
        )?;
        let message = MessageEndpoint::mount(protocols.clone())?;
        let channel = match role {
            CameraRole::Publisher => control_channel(local_peer_id, &metadata),
            CameraRole::Viewer => reply_channel(local_peer_id, &metadata),
        };
        let receiver = message.declare(channel, MESSAGE_QUEUE_CAPACITY)?;
        let stream = if role == CameraRole::Publisher {
            let stream_state = Arc::clone(&state);
            let stream_metadata = metadata.clone();
            Some(StreamEndpoint::mount(
                protocols.clone(),
                move |requester: &AuthenticatedPeer, request: StreamRequest| {
                    stream_dispatch(&stream_state, &stream_metadata, requester, request)
                },
            )?)
        } else {
            None
        };
        let clients = ProtocolClients {
            info: info.client(),
            catalog: catalog.client(),
            registry: registry.client(),
            blob: blob.client(),
            message: message.client(),
            stream: stream
                .as_ref()
                .map(StreamEndpoint::client)
                .unwrap_or_else(|| StreamClient::new(protocols)),
        };
        let message_task = tokio::spawn(drain_messages(
            receiver,
            Arc::clone(&state),
            clients.message.clone(),
            clients.blob.clone(),
        ));

        Ok((
            Self {
                role,
                card,
                metadata,
                state,
                remote_cache: Mutex::new(HashMap::new()),
                clients,
                info,
                catalog,
                registry,
                blob,
                message,
                stream,
                message_task,
            },
            event_rx,
        ))
    }

    pub const fn role(&self) -> CameraRole {
        self.role
    }

    pub fn card(&self) -> &PeerCard {
        &self.card
    }

    pub fn approve(&self, peer_id: PeerId) {
        lock(&self.state.pending_approvals).remove(&peer_id);
        lock(&self.state.allowed).insert(peer_id);
    }

    pub fn revoke(&self, peer_id: PeerId) {
        lock(&self.state.allowed).remove(&peer_id);
        lock(&self.state.pending_approvals).remove(&peer_id);
    }

    pub fn pending_approvals(&self) -> Vec<PeerId> {
        let mut peers = lock(&self.state.pending_approvals)
            .iter()
            .copied()
            .collect::<Vec<_>>();
        peers.sort_unstable_by_key(ToString::to_string);
        peers
    }

    /// Replace the newest bounded JPEG used by future Stream items and
    /// snapshots. Slow consumers observe the newest frame rather than growing
    /// an application-side queue.
    pub fn replace_frame(&self, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            self.role == CameraRole::Publisher,
            "only a publisher accepts camera frames"
        );
        self.state.replace_frame(bytes)
    }

    pub fn paused(&self) -> bool {
        self.state.paused.load(Ordering::SeqCst)
    }

    pub async fn discover(
        &self,
        discovery: &AukiDiscovery,
        protocol: Option<&str>,
    ) -> Result<Vec<DiscoveryPeer>> {
        let protocol = protocol.unwrap_or(auki_protocols::stream::v2::ID);
        let candidates = discovery.discover_protocol(protocol).await?;
        Ok(candidates.into_iter().map(discovery_peer).collect())
    }

    pub async fn resolve_remote(&self, target: &PeerCard) -> Result<RemoteCamera> {
        let peer_id = target.peer_id()?;
        let route = target.tcp_route()?;
        let cache_key = remote_cache_key(peer_id, &route);
        let info = self
            .clients
            .info
            .fetch_exact(peer_id, route.clone())
            .await
            .context("fetch Camera Mesh participant info")?;
        ensure!(info.peer_id == peer_id, "Info authenticated the wrong peer");
        ensure!(
            info.app == APP && info.app_version == APP_VERSION,
            "target is not a compatible Camera Mesh peer"
        );
        let cached = { lock(&self.remote_cache).get(&cache_key).cloned() };
        if let Some(mut remote) = cached {
            if remote.info.session_id == info.session_id
                && remote.info.session_clock_id == info.session_clock_id
                && remote.info.session_clock_hash == info.session_clock_hash
            {
                remote.info = info;
                lock(&self.remote_cache).insert(cache_key, remote.clone());
                return Ok(remote);
            }
            lock(&self.remote_cache).remove(&cache_key);
        }

        let response = self
            .clients
            .catalog
            .fetch_resources_exact(
                peer_id,
                route.clone(),
                catalog_v3::ResourcesRequest {
                    variants: vec![ResourceVariant::SensorLog, ResourceVariant::MessageChannel],
                },
            )
            .await
            .context("fetch Camera Mesh Catalog")?;
        let (sensor_ref, clock_ref, frame_ref, control_channel) =
            parse_catalog(peer_id, &response)?;
        ensure!(
            info.session_clock_id == clock_ref.id,
            "Info and Catalog use different clocks"
        );
        ensure!(
            info.session_clock_hash == clock_ref.hash,
            "Info and Catalog clock hashes differ"
        );

        let sensor = self
            .clients
            .registry
            .fetch_sensor_exact(peer_id, route.clone(), &sensor_ref.id, &sensor_ref.hash)
            .await
            .context("fetch Camera Sensor Registry entry")?;
        let clock = self
            .clients
            .registry
            .fetch_clock_exact(peer_id, route.clone(), &clock_ref.id, &clock_ref.hash)
            .await
            .context("fetch Camera Clock Registry entry")?;
        let frame = self
            .clients
            .registry
            .fetch_frame_exact(peer_id, route.clone(), &frame_ref.id, &frame_ref.hash)
            .await
            .context("fetch Camera Frame Registry entry")?;
        validate_remote_metadata(peer_id, &info, &sensor, &clock, &frame)?;
        let remote = RemoteCamera {
            peer_id,
            route,
            info,
            sensor,
            clock,
            frame,
            control_channel,
        };
        lock(&self.remote_cache).insert(cache_key, remote.clone());
        Ok(remote)
    }

    pub async fn view(&self, target: &PeerCard, frame_count: usize) -> Result<ViewReport> {
        ensure!(
            self.role == CameraRole::Viewer,
            "only a viewer consumes camera streams"
        );
        ensure!(
            (1..=64).contains(&frame_count),
            "frames must be within 1..=64"
        );
        let remote = self.resolve_remote(target).await?;
        let mut subscription = self
            .clients
            .stream
            .subscribe_exact::<CameraFrame>(
                remote.peer_id,
                remote.route.clone(),
                StreamRequest {
                    source_peer_id: remote.peer_id.to_string(),
                    resource_id: CAMERA_RESOURCE_ID.into(),
                    from: ReadFrom::Latest,
                },
            )
            .await
            .context("subscribe to Camera Stream")?;
        validate_remote_manifest(&remote, &subscription.manifest)?;

        let mut final_hash = String::new();
        let mut final_bytes = 0;
        for _ in 0..frame_count {
            let entry = subscription
                .entries
                .next()
                .await
                .ok_or_else(|| anyhow!("Camera Stream ended before the requested frames"))??;
            ensure!(
                entry.payload.dynamic_intrinsics.is_none(),
                "Camera frame unexpectedly overrides the locked no-calibration contract"
            );
            ensure_jpeg(&entry.payload.frame)?;
            final_hash = sha256_hex(&entry.payload.frame);
            final_bytes = entry.payload.frame.len();
        }
        Ok(ViewReport {
            target_peer_id: remote.peer_id.to_string(),
            checks: ["info", "catalog", "registry", "stream", "message", "blob"]
                .into_iter()
                .map(|check| (check.into(), true))
                .collect(),
            frames: frame_count,
            frame_sha256: final_hash,
            frame_bytes: final_bytes,
        })
    }

    /// Exercise one live subscription without dropping it between controls.
    ///
    /// This is intentionally a single bounded operation for physical-device
    /// acceptance runners: it receives two frames, proves pause quiescence
    /// while tolerating one already in-flight frame, proves resume with a
    /// later sequence, and fetches a SHA-256-verified snapshot.
    pub async fn exercise_live(
        &self,
        target: &PeerCard,
        request_id: Option<String>,
    ) -> Result<LiveExerciseReport> {
        ensure!(
            self.role == CameraRole::Viewer,
            "only a viewer exercises camera streams"
        );
        let remote = tokio::time::timeout(LIVE_EXERCISE_FRAME_TIMEOUT, self.resolve_remote(target))
            .await
            .context("live exercise metadata resolution timed out")??;
        let mut subscription = tokio::time::timeout(
            LIVE_EXERCISE_FRAME_TIMEOUT,
            self.clients.stream.subscribe_exact::<CameraFrame>(
                remote.peer_id,
                remote.route.clone(),
                StreamRequest {
                    source_peer_id: remote.peer_id.to_string(),
                    resource_id: CAMERA_RESOURCE_ID.into(),
                    from: ReadFrom::Latest,
                },
            ),
        )
        .await
        .context("live exercise Stream subscription timed out")?
        .context("subscribe to Camera Stream")?;
        validate_remote_manifest(&remote, &subscription.manifest)?;

        let mut tracker = LiveFrameTracker::default();
        for index in 0..LIVE_EXERCISE_INITIAL_FRAMES {
            let entry = next_live_exercise_entry(
                &mut subscription.entries,
                &format!("initial Camera frame {}", index + 1),
            )
            .await?;
            tracker.observe(entry, LiveExercisePhase::Initial)?;
        }
        let pre_pause_sequence = tracker
            .last_sequence
            .ok_or_else(|| anyhow!("live exercise received no initial Camera frames"))?;

        let pause_result = match tokio::time::timeout(
            LIVE_EXERCISE_CONTROL_TIMEOUT,
            self.send_control_remote(&remote, "camera.pause", Vec::new()),
        )
        .await
        {
            Ok(result) => result.context("send live exercise camera.pause"),
            Err(error) => Err(error).context("live exercise camera.pause timed out"),
        };
        if let Err(pause_error) = pause_result {
            let resume_result = tokio::time::timeout(
                LIVE_EXERCISE_CONTROL_TIMEOUT,
                self.send_control_remote(&remote, "camera.resume", Vec::new()),
            )
            .await;
            return match resume_result {
                Ok(Ok(())) => Err(pause_error),
                Ok(Err(resume_error)) => Err(anyhow!(
                    "{pause_error:#}; additionally failed to restore camera.resume: {resume_error:#}"
                )),
                Err(resume_error) => Err(anyhow!(
                    "{pause_error:#}; additionally timed out restoring camera.resume: {resume_error}"
                )),
            };
        }

        let paused_result = observe_live_exercise_pause(&mut subscription.entries, &mut tracker)
            .await
            .context("prove live exercise pause quiescence");
        let resume_result = match tokio::time::timeout(
            LIVE_EXERCISE_CONTROL_TIMEOUT,
            self.send_control_remote(&remote, "camera.resume", Vec::new()),
        )
        .await
        {
            Ok(result) => result.context("send live exercise camera.resume"),
            Err(error) => Err(error).context("live exercise camera.resume timed out"),
        };
        if let Err(resume_error) = resume_result {
            return match paused_result {
                Ok(()) => Err(resume_error),
                Err(paused_error) => Err(anyhow!(
                    "{paused_error:#}; additionally failed to resume camera: {resume_error:#}"
                )),
            };
        }
        paused_result?;

        let entry =
            next_live_exercise_entry(&mut subscription.entries, "Camera frame after resume")
                .await?;
        let resumed = tracker.observe(entry, LiveExercisePhase::Resumed)?;

        let snapshot = self
            .request_snapshot_remote(
                &remote,
                request_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            )
            .await
            .context("request live exercise snapshot")?;

        // Keep the Stream route alive through the completed Blob transfer.
        drop(subscription);
        Ok(LiveExerciseReport {
            target_peer_id: remote.peer_id.to_string(),
            checks: ["info", "catalog", "registry", "stream"]
                .into_iter()
                .map(|check| (check.into(), true))
                .collect(),
            initial_frames: LIVE_EXERCISE_INITIAL_FRAMES,
            paused_in_flight_frames: tracker.paused_in_flight_frames,
            pause_quiet_ms: LIVE_EXERCISE_PAUSE_QUIET.as_millis() as u64,
            pre_pause_sequence,
            resumed_sequence: resumed.sequence,
            total_frames: tracker.total_frames,
            frame_sha256: resumed.sha256,
            frame_bytes: resumed.bytes,
            snapshot,
        })
    }

    pub async fn send_pause(&self, target: &PeerCard) -> Result<()> {
        self.send_control(target, "camera.pause", Vec::new()).await
    }

    pub async fn send_resume(&self, target: &PeerCard) -> Result<()> {
        self.send_control(target, "camera.resume", Vec::new()).await
    }

    pub async fn request_snapshot(
        &self,
        target: &PeerCard,
        request_id: Option<String>,
    ) -> Result<SnapshotReport> {
        ensure!(
            self.role == CameraRole::Viewer,
            "only a viewer requests snapshots"
        );
        let remote = self.resolve_remote(target).await?;
        let request_id = request_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        self.request_snapshot_remote(&remote, request_id).await
    }

    async fn request_snapshot_remote(
        &self,
        remote: &RemoteCamera,
        request_id: String,
    ) -> Result<SnapshotReport> {
        let payload = encode_snapshot_request(
            &request_id,
            &self.card,
            &reply_channel(self.state.local_peer_id, &self.metadata),
        )?;
        let (result_tx, result_rx) = oneshot::channel();
        let _pending_guard = register_pending_snapshot(
            &self.state,
            request_id.clone(),
            remote.peer_id,
            remote.route.clone(),
            result_tx,
        )?;
        send_message(
            &self.clients.message,
            remote.peer_id,
            remote.route.clone(),
            &remote.control_channel,
            "camera.request_snapshot",
            payload,
        )
        .await?;
        match tokio::time::timeout(SNAPSHOT_TIMEOUT, result_rx).await {
            Ok(Ok(Ok(report))) => Ok(report),
            Ok(Ok(Err(error))) => bail!(error),
            Ok(Err(_)) => bail!("snapshot reply task stopped"),
            Err(_) => bail!("snapshot reply timed out"),
        }
    }

    async fn send_control(
        &self,
        target: &PeerCard,
        control: &'static str,
        payload: Vec<u8>,
    ) -> Result<()> {
        ensure!(
            self.role == CameraRole::Viewer,
            "only a viewer sends camera controls"
        );
        let remote = self.resolve_remote(target).await?;
        self.send_control_remote(&remote, control, payload).await
    }

    async fn send_control_remote(
        &self,
        remote: &RemoteCamera,
        control: &'static str,
        payload: Vec<u8>,
    ) -> Result<()> {
        send_message(
            &self.clients.message,
            remote.peer_id,
            remote.route.clone(),
            &remote.control_channel,
            control,
            payload,
        )
        .await
    }

    pub async fn close(self) -> Result<()> {
        self.state.camera_available.store(false, Ordering::SeqCst);
        lock(&self.state.allowed).clear();
        lock(&self.state.pending_approvals).clear();
        let mut errors = Vec::new();
        if let Some(stream) = self.stream {
            collect_close(&mut errors, "Stream", stream.close().await);
        }
        collect_close(&mut errors, "Message", self.message.close().await);
        if let Err(error) = self.message_task.await {
            errors.push(format!("Message receiver task: {error}"));
        }
        collect_close(&mut errors, "Blob", self.blob.close().await);
        collect_close(&mut errors, "Registry", self.registry.close().await);
        collect_close(&mut errors, "Catalog", self.catalog.close().await);
        collect_close(&mut errors, "Info", self.info.close().await);
        if errors.is_empty() {
            Ok(())
        } else {
            bail!("Camera endpoint shutdown failed: {}", errors.join("; "))
        }
    }
}

async fn next_live_exercise_entry(
    entries: &mut SubscriptionEntries<CameraFrame>,
    description: &str,
) -> Result<StreamEntry<CameraFrame>> {
    tokio::time::timeout(LIVE_EXERCISE_FRAME_TIMEOUT, entries.next())
        .await
        .with_context(|| format!("timed out waiting for {description}"))?
        .ok_or_else(|| anyhow!("Camera Stream ended before {description}"))?
        .with_context(|| format!("read {description}"))
}

async fn observe_live_exercise_pause(
    entries: &mut SubscriptionEntries<CameraFrame>,
    tracker: &mut LiveFrameTracker,
) -> Result<()> {
    loop {
        let next = match tokio::time::timeout(LIVE_EXERCISE_PAUSE_QUIET, entries.next()).await {
            Err(_) => return Ok(()),
            Ok(next) => next,
        };
        let entry = next
            .ok_or_else(|| anyhow!("Camera Stream ended while proving pause quiescence"))?
            .context("read Camera frame while proving pause quiescence")?;
        tracker.observe(entry, LiveExercisePhase::PausedInFlight)?;
    }
}

async fn drain_messages(
    mut receiver: auki_protocols::message::MessageChannelReceiver,
    state: Arc<SharedState>,
    message_client: MessageClient,
    blob_client: BlobClient,
) {
    while let Some(event) = receiver.recv().await {
        let result = match state.role {
            CameraRole::Publisher => handle_publisher_message(&state, &message_client, event).await,
            CameraRole::Viewer => handle_viewer_message(&state, &blob_client, event).await,
        };
        if let Err(error) = result {
            state.emit(CameraEvent::RuntimeError {
                error: format!("Camera Message failed: {error:#}"),
            });
        }
    }
}

async fn handle_publisher_message(
    state: &Arc<SharedState>,
    message_client: &MessageClient,
    event: MessageEvent,
) -> Result<()> {
    ensure!(
        state.allowed(&event.sender),
        "camera control sender is not allowed"
    );
    match event.message_type() {
        "camera.pause" => {
            ensure!(event.payload().is_empty(), "pause payload must be empty");
            state.paused.store(true, Ordering::SeqCst);
            state.emit(CameraEvent::ControlReceived {
                control: "camera.pause".into(),
                peer_id: event.sender.peer_id.to_string(),
            });
        }
        "camera.resume" => {
            ensure!(event.payload().is_empty(), "resume payload must be empty");
            state.paused.store(false, Ordering::SeqCst);
            state.emit(CameraEvent::ControlReceived {
                control: "camera.resume".into(),
                peer_id: event.sender.peer_id.to_string(),
            });
        }
        "camera.request_snapshot" => {
            let request = decode_snapshot_request(event.payload(), event.sender.peer_id)?;
            let (channel, route) = request.reply.native_route_for(event.sender.peer_id)?;
            let (sha256, size) = state.stage_blob(state.latest_frame().bytes.to_vec())?;
            send_message(
                message_client,
                event.sender.peer_id,
                route,
                &channel,
                "camera.snapshot_ready",
                encode_snapshot_ready(&request.request_id, &sha256, size)?,
            )
            .await?;
            state.emit(CameraEvent::SnapshotStaged {
                request_id: request.request_id,
                peer_id: event.sender.peer_id.to_string(),
                sha256,
                size,
            });
        }
        other => bail!("unsupported camera control type {other:?}"),
    }
    Ok(())
}

async fn handle_viewer_message(
    state: &Arc<SharedState>,
    blob_client: &BlobClient,
    event: MessageEvent,
) -> Result<()> {
    ensure!(
        state.same_domain(&event.sender),
        "snapshot sender belongs to another Domain"
    );
    ensure!(
        event.message_type() == "camera.snapshot_ready",
        "unsupported camera reply type"
    );
    let payload = decode_snapshot_ready(event.payload())?;
    let pending = {
        let mut snapshots = lock(&state.pending_snapshots);
        let expected = snapshots
            .get(&payload.request_id)
            .ok_or_else(|| anyhow!("snapshot reply has no pending request"))?;
        ensure!(
            expected.peer_id == event.sender.peer_id,
            "snapshot reply came from the wrong peer"
        );
        snapshots
            .remove(&payload.request_id)
            .expect("pending snapshot was checked above")
    };
    let result = async {
        let receipt = blob_client
            .fetch_exact(pending.peer_id, pending.route, &payload.sha256)
            .await
            .context("fetch announced snapshot Blob")?;
        ensure!(
            receipt.remote_peer_id == pending.peer_id,
            "Blob authenticated the wrong peer"
        );
        ensure!(
            receipt.bytes.len() == payload.size,
            "snapshot Blob size does not match announcement"
        );
        ensure!(
            sha256_hex(&receipt.bytes) == payload.sha256,
            "snapshot Blob hash does not match announcement"
        );
        ensure_jpeg(&receipt.bytes)?;
        Ok(SnapshotReport {
            request_id: payload.request_id,
            target_peer_id: pending.peer_id.to_string(),
            sha256: payload.sha256,
            size: payload.size,
        })
    }
    .await;
    let wire_result = result
        .as_ref()
        .map(Clone::clone)
        .map_err(|error| format!("{error:#}"));
    let _ = pending.result.send(wire_result);
    result.map(|_| ())
}

fn stream_dispatch(
    state: &Arc<SharedState>,
    metadata: &CameraMetadata,
    requester: &AuthenticatedPeer,
    request: StreamRequest,
) -> StreamDispatch {
    if state.role != CameraRole::Publisher
        || request.source_peer_id != state.local_peer_id.to_string()
        || request.resource_id != CAMERA_RESOURCE_ID
    {
        return StreamDispatch::Decline {
            reason: DeclineReason::sensor_not_found(),
        };
    }
    if !state.allowed(requester) {
        state.request_approval(requester);
        return StreamDispatch::Decline {
            reason: DeclineReason::other("approval_required"),
        };
    }
    if !state.camera_available.load(Ordering::SeqCst) {
        return StreamDispatch::Decline {
            reason: DeclineReason::sensor_unavailable(),
        };
    }
    let paused = Arc::clone(state);
    let source = futures::stream::unfold((), move |()| {
        let state = Arc::clone(&paused);
        async move {
            loop {
                tokio::time::sleep(FRAME_PERIOD).await;
                if !state.paused.load(Ordering::SeqCst) {
                    break;
                }
            }
            let frame = state.latest_frame();
            Some((
                Ok(StreamItem {
                    timestamp_ns: utc_now_ns_i64(),
                    payload: CameraFrame {
                        dynamic_intrinsics: None,
                        frame: frame.bytes.to_vec(),
                    },
                }),
                (),
            ))
        }
    });
    StreamDispatch::AcceptCamera {
        manifest: stream_manifest(metadata),
        source: Box::pin(source),
    }
}

async fn send_message(
    client: &MessageClient,
    peer_id: PeerId,
    route: Multiaddr,
    channel: &MessageChannelResource,
    message_type: &str,
    payload: Vec<u8>,
) -> Result<()> {
    let sender = client
        .open_exact(peer_id, route, channel)
        .await
        .with_context(|| format!("open {message_type} Message channel"))?;
    ensure!(
        sender.remote_peer().peer_id == peer_id,
        "Message authenticated the wrong peer"
    );
    let send = sender.send(message_type, utc_now_ns_i64(), payload).await;
    let close = sender.close().await;
    send.with_context(|| format!("send {message_type}"))?;
    close.with_context(|| format!("close {message_type} Message channel"))?;
    Ok(())
}

fn parse_catalog(
    peer_id: PeerId,
    response: &catalog_v3::ResourcesResponse,
) -> Result<(
    auki_registry::RegistryRef,
    auki_registry::RegistryRef,
    auki_registry::RegistryRef,
    MessageChannelResource,
)> {
    let mut camera = None;
    let mut control = None;
    for row in &response.resources {
        match row {
            ResourceEntry::V2(row) if row.resource_id == CAMERA_RESOURCE_ID => {
                ensure!(camera.is_none(), "Camera Catalog row is duplicated");
                ensure!(
                    row.source_peer_id == peer_id.to_string(),
                    "Camera Catalog source owner mismatch"
                );
                ensure!(
                    row.writer_peer_id == peer_id.to_string(),
                    "Camera Catalog writer owner mismatch"
                );
                ensure!(row.state == "live", "Camera Catalog row is not live");
                let sensor = row
                    .sensor
                    .as_ref()
                    .ok_or_else(|| anyhow!("Camera Catalog row has no sensor"))?;
                ensure!(
                    sensor.kind == SensorKind::Camera && sensor.r#type == "rgb",
                    "Camera Catalog sensor kind/type mismatch"
                );
                let VariantContent::SensorLog { manifest } = &row.variant_content else {
                    bail!("Camera Catalog row is not a Sensor Log")
                };
                let frame = manifest
                    .frame
                    .clone()
                    .ok_or_else(|| anyhow!("Camera Catalog row has no frame"))?;
                camera = Some((
                    auki_registry::RegistryRef {
                        peer_id: peer_id.to_string(),
                        id: sensor.sensor_id.clone(),
                        hash: sensor.sensor_hash.clone(),
                    },
                    manifest.clock.clone(),
                    frame,
                ));
            }
            ResourceEntry::MessageChannel(channel)
                if channel.resource_id == CAMERA_CONTROL_RESOURCE_ID =>
            {
                ensure!(control.is_none(), "Camera control channel is duplicated");
                ensure!(
                    channel.owner_peer_id == peer_id,
                    "Camera control channel owner mismatch"
                );
                control = Some(channel.clone());
            }
            _ => {}
        }
    }
    let (sensor, clock, frame) = camera.ok_or_else(|| {
        anyhow!("approval_required: Catalog has no visible live camera/main Sensor Log")
    })?;
    let control =
        control.ok_or_else(|| anyhow!("Catalog has no camera/control Message channel"))?;
    ensure!(
        sensor.peer_id == peer_id.to_string(),
        "Sensor Registry owner mismatch"
    );
    ensure!(
        clock.peer_id == peer_id.to_string(),
        "Clock Registry owner mismatch"
    );
    ensure!(
        frame.peer_id == peer_id.to_string(),
        "Frame Registry owner mismatch"
    );
    ensure!(
        control.clock == clock,
        "Camera control channel uses a different clock"
    );
    Ok((sensor, clock, frame, control))
}

fn validate_remote_metadata(
    peer_id: PeerId,
    info: &AuthenticatedParticipantInfo,
    sensor: &SensorRegistryEntry,
    clock: &ClockRegistryEntry,
    frame: &FrameRegistryEntry,
) -> Result<()> {
    ensure!(
        sensor.peer_id == peer_id.to_string() && sensor.sensor_id == CAMERA_RESOURCE_ID,
        "unexpected Camera Sensor Registry identity"
    );
    let SensorBody::Camera(camera) = &sensor.body else {
        bail!("Camera Sensor Registry entry is not a camera")
    };
    camera
        .validate_image_layout()
        .context("invalid Camera image layout")?;
    camera
        .validate_calibration()
        .context("invalid Camera calibration")?;
    ensure!(camera.r#type == "rgb", "Camera type is not rgb");
    ensure!(
        camera.width == CAMERA_WIDTH
            && camera.height == CAMERA_HEIGHT
            && camera.frame_rate_hz == CAMERA_RATE_HZ,
        "Camera dimensions or cadence do not match the Camera Mesh contract"
    );
    ensure!(
        camera.image_encoding == "jpeg"
            && camera.pixel_format == "rgb8"
            && camera.row_stride_bytes == 0
            && camera.color_space == "srgb",
        "Camera image bytes do not match the JPEG/rgb8/sRGB contract"
    );
    ensure!(
        camera.intrinsics_model == "none"
            && camera.distortion_model == "none"
            && camera.calibration.is_none(),
        "Camera calibration does not match the locked no-calibration contract"
    );
    ensure!(
        camera.frame.peer_id == peer_id.to_string(),
        "Camera frame owner mismatch"
    );
    ensure!(
        camera.frame.id == frame.frame_id && camera.frame.hash == frame.hash(),
        "Camera frame reference mismatch"
    );
    ensure!(
        clock.peer_id == peer_id.to_string()
            && clock.clock_id == CAMERA_CLOCK_ID
            && clock.session_id == info.session_id,
        "Camera clock identity or session mismatch"
    );
    let ClockBody::UtcClock(clock_meta) = &clock.body else {
        bail!("Camera clock is not UTC")
    };
    ensure!(
        clock_meta.unit == "ns"
            && !clock_meta.monotonic
            && clock_meta.epoch.as_deref() == Some("1970-01-01T00:00:00Z")
            && clock_meta.scope == Scope::Global,
        "Camera clock does not match the UTC/global nanosecond contract"
    );
    ensure!(
        frame.peer_id == peer_id.to_string()
            && frame.frame_id == CAMERA_FRAME_ID
            && frame.handedness == Handedness::Right
            && frame.axes.x == AxisDirection::Right
            && frame.axes.y == AxisDirection::Down
            && frame.axes.z == AxisDirection::Forward
            && frame.units == LengthUnit::Meters,
        "Camera frame does not match the ROS-optical convention"
    );
    Ok(())
}

fn validate_remote_manifest(
    remote: &RemoteCamera,
    manifest: &auki_protocols::stream::v2::StreamManifest,
) -> Result<()> {
    ensure!(
        manifest.resource_id == CAMERA_RESOURCE_ID,
        "Stream manifest resource mismatch"
    );
    ensure!(
        manifest.payload == "camera_frame",
        "Stream manifest payload mismatch"
    );
    ensure!(
        manifest.sensor_id == remote.sensor.sensor_id
            && manifest.sensor_hash == remote.sensor.hash(),
        "Stream manifest Sensor Registry mismatch"
    );
    ensure!(
        manifest.clock_peer_id == remote.peer_id.to_string()
            && manifest.clock_id == remote.clock.clock_id
            && manifest.clock_hash == remote.clock.hash(),
        "Stream manifest Clock Registry mismatch"
    );
    ensure!(
        manifest.frame_id == remote.frame.frame_id && manifest.frame_hash == remote.frame.hash(),
        "Stream manifest Frame Registry mismatch"
    );
    ensure!(
        manifest.writer_mode == "live" && manifest.expected_rate_hz == CAMERA_RATE_HZ,
        "Stream manifest live cadence mismatch"
    );
    ensure!(
        manifest.from_frame_id.is_empty()
            && manifest.from_frame_hash.is_empty()
            && manifest.to_frame_id.is_empty()
            && manifest.to_frame_hash.is_empty()
            && manifest.map_peer_id.is_empty()
            && manifest.map_id.is_empty()
            && manifest.map_hash.is_empty(),
        "Camera Stream manifest contains fields outside the locked contract"
    );
    Ok(())
}

fn ensure_jpeg(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9]),
        "Camera frame is not a JPEG"
    );
    Ok(())
}

fn remote_cache_key(peer_id: PeerId, route: &Multiaddr) -> (PeerId, String) {
    (peer_id, route.to_string())
}

fn new_session_id() -> String {
    Uuid::new_v4().to_string()
}

fn register_pending_snapshot(
    state: &Arc<SharedState>,
    request_id: String,
    peer_id: PeerId,
    route: Multiaddr,
    result: oneshot::Sender<Result<SnapshotReport, String>>,
) -> Result<PendingSnapshotGuard> {
    let registration_id = Uuid::new_v4();
    {
        let mut snapshots = lock(&state.pending_snapshots);
        ensure!(
            !snapshots.contains_key(&request_id),
            "snapshot requestId is already pending"
        );
        ensure!(
            snapshots.len() < MAX_PENDING_SNAPSHOTS,
            "too many snapshot requests are awaiting replies"
        );
        snapshots.insert(
            request_id.clone(),
            PendingSnapshot {
                registration_id,
                peer_id,
                route,
                result,
            },
        );
    }
    Ok(PendingSnapshotGuard {
        state: Arc::clone(state),
        request_id,
        registration_id,
    })
}

fn peer_routes(peer: &AukiPeer) -> Result<PeerRoutes> {
    let published = peer
        .protocol_context()
        .routes()
        .snapshot()?
        .relay_routes
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("Auki peer has no confirmed relay route"))?;
    Ok(PeerRoutes {
        tcp: published.routes.tcp().to_string(),
        wss: published.routes.wss().to_string(),
    })
}

fn validate_camera_jpeg(bytes: &[u8]) -> Result<()> {
    ensure!(!bytes.is_empty(), "camera JPEG must not be empty");
    ensure!(
        bytes.len() <= MAX_BLOB_BYTES,
        "camera JPEG exceeds the Camera Mesh limit"
    );
    ensure_jpeg(bytes)
}

fn discovery_peer(candidate: AukiDiscoveryCandidate) -> DiscoveryPeer {
    DiscoveryPeer {
        peer_id: candidate.peer_id().to_string(),
        routes: candidate.routes().iter().map(ToString::to_string).collect(),
        served_protocols: candidate.served_protocols().to_vec(),
        expires_at: candidate.expires_at().to_rfc3339(),
        source: match candidate.source() {
            AukiDiscoverySource::DdsTracker => "dds_tracker".into(),
        },
    }
}

fn utc_now_ns_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn utc_now_ns_i64() -> i64 {
    utc_now_ns_u64().try_into().unwrap_or(i64::MAX)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn collect_close<T: std::fmt::Display>(
    errors: &mut Vec<String>,
    name: &str,
    result: std::result::Result<(), T>,
) {
    if let Err(error) = result {
        errors.push(format!("{name}: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }

    fn fixture_remote() -> (CameraMetadata, RemoteCamera) {
        let peer_id = peer();
        let metadata = metadata(peer_id, "camera-test-session");
        let info = AuthenticatedParticipantInfo {
            app: APP.into(),
            app_version: APP_VERSION.into(),
            name: "test camera".into(),
            session_id: "camera-test-session".into(),
            session_clock_id: metadata.clock_ref.id.clone(),
            session_clock_hash: metadata.clock_ref.hash.clone(),
            session_now_ns: 0,
            peer_id,
            app_instance: "native/publisher".into(),
        };
        let remote = RemoteCamera {
            peer_id,
            route: format!("/ip4/127.0.0.1/tcp/9000/p2p/{peer_id}")
                .parse()
                .unwrap(),
            info,
            sensor: metadata.sensor.clone(),
            clock: metadata.clock.clone(),
            frame: metadata.frame.clone(),
            control_channel: control_channel(peer_id, &metadata),
        };
        (metadata, remote)
    }

    fn fixture_state() -> (Arc<SharedState>, mpsc::Receiver<CameraEvent>) {
        let (events, receiver) = mpsc::channel(CAMERA_EVENT_QUEUE_CAPACITY);
        (
            Arc::new(SharedState {
                role: CameraRole::Viewer,
                domain_id: Uuid::nil(),
                local_peer_id: peer(),
                allowed: Mutex::new(HashSet::new()),
                pending_approvals: Mutex::new(HashSet::new()),
                blobs: Mutex::new(VecDeque::new()),
                pending_snapshots: Mutex::new(HashMap::new()),
                latest_frame: Mutex::new(LiveFrame {
                    bytes: Arc::from(deterministic_jpeg().unwrap()),
                }),
                paused: AtomicBool::new(false),
                camera_available: AtomicBool::new(false),
                events,
            }),
            receiver,
        )
    }

    fn fixture_route() -> Multiaddr {
        format!("/ip4/127.0.0.1/tcp/9000/p2p/{}", peer())
            .parse()
            .unwrap()
    }

    #[test]
    fn deterministic_frame_passes_camera_guard() {
        validate_camera_jpeg(&deterministic_jpeg().unwrap()).unwrap();
    }

    #[test]
    fn live_frame_tracker_allows_only_one_paused_frame_and_requires_progress() {
        fn entry(sequence: u64) -> StreamEntry<CameraFrame> {
            StreamEntry {
                timestamp_ns: 7,
                seq: sequence,
                payload: CameraFrame {
                    dynamic_intrinsics: None,
                    frame: deterministic_jpeg().unwrap(),
                },
            }
        }

        let mut tracker = LiveFrameTracker::default();
        tracker
            .observe(entry(4), LiveExercisePhase::Initial)
            .unwrap();
        tracker
            .observe(entry(5), LiveExercisePhase::PausedInFlight)
            .unwrap();
        let overflow = tracker
            .observe(entry(6), LiveExercisePhase::PausedInFlight)
            .unwrap_err();
        assert!(
            overflow
                .to_string()
                .contains("more than one in-flight frame")
        );

        let resumed = tracker
            .observe(entry(6), LiveExercisePhase::Resumed)
            .unwrap();
        assert_eq!(resumed.sequence, 6);
        assert_eq!(tracker.total_frames, 3);
        assert_eq!(tracker.paused_in_flight_frames, 1);
        assert!(
            tracker
                .observe(entry(6), LiveExercisePhase::Resumed)
                .unwrap_err()
                .to_string()
                .contains("sequence did not advance")
        );
    }

    #[test]
    fn live_exercise_report_uses_stable_camel_case_fields() {
        let report = LiveExerciseReport {
            target_peer_id: peer().to_string(),
            checks: [("stream".into(), true)].into_iter().collect(),
            initial_frames: 2,
            paused_in_flight_frames: 1,
            pause_quiet_ms: 800,
            pre_pause_sequence: 7,
            resumed_sequence: 9,
            total_frames: 4,
            frame_sha256: "a".repeat(64),
            frame_bytes: 128,
            snapshot: SnapshotReport {
                request_id: "snapshot-live".into(),
                target_peer_id: peer().to_string(),
                sha256: "b".repeat(64),
                size: 256,
            },
        };
        let encoded = serde_json::to_value(report).unwrap();

        assert_eq!(encoded["initialFrames"], 2);
        assert_eq!(encoded["pausedInFlightFrames"], 1);
        assert_eq!(encoded["prePauseSequence"], 7);
        assert_eq!(encoded["resumedSequence"], 9);
        assert_eq!(encoded["snapshot"]["requestId"], "snapshot-live");
    }

    #[test]
    fn latest_camera_frame_is_bounded_validated_and_replaceable() {
        let (state, _) = fixture_state();
        let replacement = vec![0xff, 0xd8, 1, 2, 3, 0xff, 0xd9];

        state.replace_frame(replacement.clone()).unwrap();

        let latest = state.latest_frame();
        assert_eq!(latest.bytes.as_ref(), replacement);
        assert!(state.camera_available.load(Ordering::SeqCst));
        assert!(state.replace_frame(vec![]).is_err());
        assert_eq!(state.latest_frame().bytes.as_ref(), replacement);
    }

    #[test]
    fn accepts_only_the_locked_remote_registry_contract() {
        let (_, remote) = fixture_remote();
        validate_remote_metadata(
            remote.peer_id,
            &remote.info,
            &remote.sensor,
            &remote.clock,
            &remote.frame,
        )
        .unwrap();

        let mut wrong_sensor = remote.sensor.clone();
        let SensorBody::Camera(camera) = &mut wrong_sensor.body else {
            unreachable!()
        };
        camera.width += 1;
        assert!(
            validate_remote_metadata(
                remote.peer_id,
                &remote.info,
                &wrong_sensor,
                &remote.clock,
                &remote.frame,
            )
            .is_err()
        );

        let mut wrong_clock = remote.clock.clone();
        let ClockBody::UtcClock(clock) = &mut wrong_clock.body else {
            unreachable!()
        };
        clock.scope = Scope::DomainLocal;
        assert!(
            validate_remote_metadata(
                remote.peer_id,
                &remote.info,
                &remote.sensor,
                &wrong_clock,
                &remote.frame,
            )
            .is_err()
        );

        let mut wrong_frame = remote.frame.clone();
        wrong_frame.axes.y = AxisDirection::Up;
        assert!(
            validate_remote_metadata(
                remote.peer_id,
                &remote.info,
                &remote.sensor,
                &remote.clock,
                &wrong_frame,
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_only_the_locked_live_stream_manifest() {
        let (metadata, remote) = fixture_remote();
        let expected = stream_manifest(&metadata);
        validate_remote_manifest(&remote, &expected).unwrap();

        let mut wrong_rate = expected.clone();
        wrong_rate.expected_rate_hz += 1;
        assert!(validate_remote_manifest(&remote, &wrong_rate).is_err());

        let mut unexpected_field = expected;
        unexpected_field.from_frame_id = "unexpected".into();
        assert!(validate_remote_manifest(&remote, &unexpected_field).is_err());
    }

    #[test]
    fn rejects_duplicate_camera_and_control_catalog_rows() {
        let (metadata, _) = fixture_remote();
        let catalog = camera_catalog(peer(), &metadata);
        parse_catalog(peer(), &catalog).unwrap();

        let mut duplicate_camera = catalog.clone();
        duplicate_camera
            .resources
            .push(catalog.resources[0].clone());
        assert!(
            parse_catalog(peer(), &duplicate_camera)
                .unwrap_err()
                .to_string()
                .contains("duplicated")
        );

        let mut duplicate_control = catalog.clone();
        duplicate_control
            .resources
            .push(catalog.resources[1].clone());
        assert!(
            parse_catalog(peer(), &duplicate_control)
                .unwrap_err()
                .to_string()
                .contains("duplicated")
        );
    }

    #[test]
    fn remote_cache_identity_includes_the_exact_route() {
        let peer_id = peer();
        let first: Multiaddr = format!("/ip4/127.0.0.1/tcp/9000/p2p/{peer_id}")
            .parse()
            .unwrap();
        let second: Multiaddr = format!("/ip4/127.0.0.1/tcp/9001/p2p/{peer_id}")
            .parse()
            .unwrap();
        assert_eq!(
            remote_cache_key(peer_id, &first),
            remote_cache_key(peer_id, &first)
        );
        assert_ne!(
            remote_cache_key(peer_id, &first),
            remote_cache_key(peer_id, &second)
        );
    }

    #[test]
    fn camera_session_ids_are_fresh_uuid_v4_values() {
        let first = new_session_id();
        let second = new_session_id();
        assert_ne!(first, second);
        assert_eq!(Uuid::parse_str(&first).unwrap().get_version_num(), 4);
        assert_eq!(Uuid::parse_str(&second).unwrap().get_version_num(), 4);
    }

    #[test]
    fn duplicate_snapshot_registration_preserves_the_original_waiter() {
        let (state, _events) = fixture_state();
        let (first_result, _first_receiver) = oneshot::channel();
        let first = register_pending_snapshot(
            &state,
            "same-request".into(),
            peer(),
            fixture_route(),
            first_result,
        )
        .unwrap();
        let first_registration = first.registration_id;

        let (duplicate_result, _duplicate_receiver) = oneshot::channel();
        let error = register_pending_snapshot(
            &state,
            "same-request".into(),
            peer(),
            fixture_route(),
            duplicate_result,
        )
        .err()
        .expect("duplicate registration must fail");
        assert!(error.to_string().contains("already pending"));
        assert_eq!(
            lock(&state.pending_snapshots)
                .get("same-request")
                .unwrap()
                .registration_id,
            first_registration
        );

        drop(first);
        assert!(lock(&state.pending_snapshots).is_empty());
    }

    #[test]
    fn stale_snapshot_guard_does_not_remove_a_reused_request_id() {
        let (state, _events) = fixture_state();
        let (first_result, _first_receiver) = oneshot::channel();
        let first = register_pending_snapshot(
            &state,
            "reused-request".into(),
            peer(),
            fixture_route(),
            first_result,
        )
        .unwrap();
        lock(&state.pending_snapshots).remove("reused-request");

        let (second_result, _second_receiver) = oneshot::channel();
        let second = register_pending_snapshot(
            &state,
            "reused-request".into(),
            peer(),
            fixture_route(),
            second_result,
        )
        .unwrap();
        let second_registration = second.registration_id;
        drop(first);
        assert_eq!(
            lock(&state.pending_snapshots)
                .get("reused-request")
                .unwrap()
                .registration_id,
            second_registration
        );
        drop(second);
        assert!(lock(&state.pending_snapshots).is_empty());
    }

    #[test]
    fn pending_snapshot_capacity_is_bounded() {
        let (state, _events) = fixture_state();
        let mut guards = Vec::new();
        for index in 0..MAX_PENDING_SNAPSHOTS {
            let (result, _receiver) = oneshot::channel();
            guards.push(
                register_pending_snapshot(
                    &state,
                    format!("request-{index}"),
                    peer(),
                    fixture_route(),
                    result,
                )
                .unwrap(),
            );
        }
        let (overflow_result, _overflow_receiver) = oneshot::channel();
        let error = register_pending_snapshot(
            &state,
            "overflow".into(),
            peer(),
            fixture_route(),
            overflow_result,
        )
        .err()
        .expect("capacity overflow must fail");
        assert!(error.to_string().contains("too many snapshot requests"));
        assert_eq!(lock(&state.pending_snapshots).len(), MAX_PENDING_SNAPSHOTS);

        drop(guards);
        assert!(lock(&state.pending_snapshots).is_empty());
    }

    #[test]
    fn camera_event_queue_drops_overflow_without_growing() {
        let (state, mut events) = fixture_state();
        for index in 0..(CAMERA_EVENT_QUEUE_CAPACITY + 8) {
            state.emit(CameraEvent::RuntimeError {
                error: format!("event-{index}"),
            });
        }
        let mut received = 0;
        while events.try_recv().is_ok() {
            received += 1;
        }
        assert_eq!(received, CAMERA_EVENT_QUEUE_CAPACITY);
    }
}
