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
        StreamClient, StreamDispatch, StreamEndpoint, StreamItem,
        v2::{DeclineReason, ReadFrom, StreamRequest},
    },
};
use auki_registry::{ClockRegistryEntry, FrameRegistryEntry, SensorBody, SensorRegistryEntry};
use auki_sdk::{
    AukiDiscovery, AukiDiscoveryCandidate, AukiDiscoverySource, AukiPeer, AuthenticatedPeer,
    Multiaddr, PeerId,
};
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::contract::{
    APP, APP_VERSION, CAMERA_CONTROL_RESOURCE_ID, CAMERA_RATE_HZ, CAMERA_RESOURCE_ID,
    CameraMetadata, CameraRole, MAX_BLOB_BYTES, PeerCard, PeerRoutes, camera_catalog,
    control_channel, decode_snapshot_ready, decode_snapshot_request, deterministic_jpeg,
    encode_snapshot_ready, encode_snapshot_request, metadata, protocol_ids_for_role, reply_channel,
    sha256_hex, stream_manifest,
};

const MAX_STAGED_BLOBS: usize = 8;
const MESSAGE_QUEUE_CAPACITY: usize = 16;
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(45);
const FRAME_PERIOD: Duration = Duration::from_millis(1_000 / CAMERA_RATE_HZ as u64);
const FIXTURE_TIMESTAMP_NS: i64 = 1_800_000_000_000_000_000;

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
    peer_id: PeerId,
    route: Multiaddr,
    result: oneshot::Sender<Result<SnapshotReport, String>>,
}

struct SharedState {
    role: CameraRole,
    domain_id: Uuid,
    local_peer_id: PeerId,
    allowed: Mutex<HashSet<PeerId>>,
    pending_approvals: Mutex<HashSet<PeerId>>,
    blobs: Mutex<VecDeque<(String, Arc<[u8]>)>>,
    pending_snapshots: Mutex<HashMap<String, PendingSnapshot>>,
    paused: AtomicBool,
    camera_available: AtomicBool,
    events: mpsc::UnboundedSender<CameraEvent>,
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
        let _ = self.events.send(event);
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
    ) -> Result<(Self, mpsc::UnboundedReceiver<CameraEvent>)> {
        let local_peer_id = peer.peer_id();
        let session_id = format!("camera-{local_peer_id}");
        let metadata = metadata(local_peer_id, &session_id);
        let card = peer_card(peer, role)?;
        let fixture = Arc::<[u8]>::from(deterministic_jpeg()?);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let state = Arc::new(SharedState {
            role,
            domain_id: peer.domain_id(),
            local_peer_id,
            allowed: Mutex::new(HashSet::new()),
            pending_approvals: Mutex::new(HashSet::new()),
            blobs: Mutex::new(VecDeque::new()),
            pending_snapshots: Mutex::new(HashMap::new()),
            paused: AtomicBool::new(false),
            camera_available: AtomicBool::new(role == CameraRole::Publisher),
            events: event_tx,
        });
        let protocols = peer.protocols();
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
                    app_instance: format!("native/{}", role.as_str()),
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
            let stream_fixture = Arc::clone(&fixture);
            Some(StreamEndpoint::mount(
                protocols.clone(),
                move |requester: &AuthenticatedPeer, request: StreamRequest| {
                    stream_dispatch(
                        &stream_state,
                        &stream_metadata,
                        Arc::clone(&stream_fixture),
                        requester,
                        request,
                    )
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
            fixture,
        ));

        Ok((
            Self {
                role,
                card,
                metadata,
                state,
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

    pub async fn discover(
        &self,
        discovery: &AukiDiscovery,
        protocol: Option<&str>,
    ) -> Result<Vec<DiscoveryPeer>> {
        let candidates = match protocol {
            Some(protocol) => discovery.discover_protocol(protocol).await?,
            None => discovery.discover().await?,
        };
        Ok(candidates.into_iter().map(discovery_peer).collect())
    }

    pub async fn resolve_remote(&self, target: &PeerCard) -> Result<RemoteCamera> {
        let peer_id = target.peer_id()?;
        let route = target.tcp_route()?;
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
        validate_remote_metadata(peer_id, &sensor, &clock, &frame)?;
        Ok(RemoteCamera {
            peer_id,
            route,
            info,
            sensor,
            clock,
            frame,
            control_channel,
        })
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
            ensure_jpeg(&entry.payload.frame)?;
            final_hash = sha256_hex(&entry.payload.frame);
            final_bytes = entry.payload.frame.len();
        }
        Ok(ViewReport {
            target_peer_id: remote.peer_id.to_string(),
            checks: ["info", "catalog", "registry", "stream"]
                .into_iter()
                .map(|check| (check.into(), true))
                .collect(),
            frames: frame_count,
            frame_sha256: final_hash,
            frame_bytes: final_bytes,
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
        let payload = encode_snapshot_request(
            &request_id,
            &self.card,
            &reply_channel(self.state.local_peer_id, &self.metadata),
        )?;
        let (result_tx, result_rx) = oneshot::channel();
        ensure!(
            lock(&self.state.pending_snapshots)
                .insert(
                    request_id.clone(),
                    PendingSnapshot {
                        peer_id: remote.peer_id,
                        route: remote.route.clone(),
                        result: result_tx,
                    },
                )
                .is_none(),
            "snapshot requestId is already pending"
        );
        if let Err(error) = send_message(
            &self.clients.message,
            remote.peer_id,
            remote.route,
            &remote.control_channel,
            "camera.request_snapshot",
            payload,
        )
        .await
        {
            lock(&self.state.pending_snapshots).remove(&request_id);
            return Err(error);
        }
        match tokio::time::timeout(SNAPSHOT_TIMEOUT, result_rx).await {
            Ok(Ok(Ok(report))) => Ok(report),
            Ok(Ok(Err(error))) => bail!(error),
            Ok(Err(_)) => bail!("snapshot reply task stopped"),
            Err(_) => {
                lock(&self.state.pending_snapshots).remove(&request_id);
                bail!("snapshot reply timed out")
            }
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
        send_message(
            &self.clients.message,
            remote.peer_id,
            remote.route,
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

async fn drain_messages(
    mut receiver: auki_protocols::message::MessageChannelReceiver,
    state: Arc<SharedState>,
    message_client: MessageClient,
    blob_client: BlobClient,
    fixture: Arc<[u8]>,
) {
    while let Some(event) = receiver.recv().await {
        let result = match state.role {
            CameraRole::Publisher => {
                handle_publisher_message(&state, &message_client, &fixture, event).await
            }
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
    fixture: &[u8],
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
            let (sha256, size) = state.stage_blob(fixture.to_vec())?;
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
    let pending = lock(&state.pending_snapshots)
        .remove(&payload.request_id)
        .ok_or_else(|| anyhow!("snapshot reply has no pending request"))?;
    if pending.peer_id != event.sender.peer_id {
        let _ = pending
            .result
            .send(Err("snapshot reply came from the wrong peer".into()));
        bail!("snapshot reply came from the wrong peer");
    }
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
    fixture: Arc<[u8]>,
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
    let source = futures::stream::unfold(0_u64, move |sequence| {
        let state = Arc::clone(&paused);
        let fixture = Arc::clone(&fixture);
        async move {
            loop {
                tokio::time::sleep(FRAME_PERIOD).await;
                if !state.paused.load(Ordering::SeqCst) {
                    break;
                }
            }
            let timestamp_ns = FIXTURE_TIMESTAMP_NS.saturating_add(
                i64::try_from(sequence)
                    .unwrap_or(i64::MAX)
                    .saturating_mul(1_000_000_000_i64 / i64::from(CAMERA_RATE_HZ)),
            );
            Some((
                Ok(StreamItem {
                    timestamp_ns,
                    payload: CameraFrame {
                        dynamic_intrinsics: None,
                        frame: fixture.to_vec(),
                    },
                }),
                sequence.saturating_add(1),
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
        camera.image_encoding == "jpeg",
        "Camera encoding is not jpeg"
    );
    ensure!(
        camera.pixel_format == "rgb8",
        "Camera pixel format is not rgb8"
    );
    ensure!(
        camera.width > 0 && camera.height > 0 && camera.frame_rate_hz > 0,
        "Camera geometry/cadence is invalid"
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
        clock.peer_id == peer_id.to_string(),
        "Camera clock owner mismatch"
    );
    ensure!(
        frame.peer_id == peer_id.to_string(),
        "Camera frame owner mismatch"
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
        manifest.writer_mode == "live",
        "Stream manifest writer mode mismatch"
    );
    ensure!(
        manifest.expected_rate_hz > 0,
        "Stream manifest has no expected cadence"
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

fn peer_card(peer: &AukiPeer, role: CameraRole) -> Result<PeerCard> {
    let published = peer
        .protocol_context()
        .routes()
        .snapshot()?
        .relay_routes
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("Auki peer has no confirmed relay route"))?;
    Ok(PeerCard {
        version: 1,
        runtime: "native".into(),
        domain_id: peer.domain_id().to_string(),
        peer_id: peer.peer_id().to_string(),
        protocols: protocol_ids_for_role(role),
        routes: PeerRoutes {
            tcp: published.routes.tcp().to_string(),
            wss: published.routes.wss().to_string(),
        },
    })
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

    #[test]
    fn deterministic_frame_passes_camera_guard() {
        ensure_jpeg(&deterministic_jpeg().unwrap()).unwrap();
    }
}
