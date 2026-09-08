use std::{
    collections::BTreeMap,
    env,
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use auki_datatypes::scalar;
use auki_protocols::{
    blob::v1::{
        BlobClient, BlobEndpoint, BlobProvider, BlobProviderError, BlobProviderFuture, BlobRequest,
        ProvidedBlobChunk,
    },
    catalog::{
        CatalogClient, CatalogEndpoint, CatalogProvider,
        v3::{self as catalog_v3, ResourceEntry},
        v4 as catalog_v4,
    },
    info::{InfoClient, InfoEndpoint, v1::AuthenticatedParticipantInfo},
    message::{MessageChannelResource, MessageClient, MessageEndpoint, MessageEvent},
    registry::{
        RegistryClient, RegistryEndpoint,
        v3::{RegistryKind, RegistryListEntry, RegistryRequest, RegistryResponse},
    },
    stream::{
        StreamClient, StreamDispatch, StreamEndpoint, StreamItem,
        v2::{ReadFrom, StreamManifest, StreamRequest},
    },
};
use auki_registry::RegistryRef;
use auki_sdk::{
    AukiDiscovery, AukiDiscoveryCandidate, AukiDiscoverySource, AukiPeer, AukiPeerBootstrap,
    Credentials, DdsTrackerMode, DomainSelection, Multiaddr, PeerId,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

const APP: &str = "standard-protocols";
const APP_VERSION: &str = "0.1.0";
const BLOB_BYTES: &[u8] = b"auki-standard-protocols-v1";
const MESSAGE_RESOURCE_ID: &str = "playground/events";
const MESSAGE_CLOCK_ID: &str = "playground/clock";
const MESSAGE_CLOCK_HASH: &str = "playground-clock-v1";
const MESSAGE_TYPE: &str = "playground.message";
const MESSAGE_TIMESTAMP_NS: i64 = 42;
const MESSAGE_BYTES: &[u8] = b"hello from the standard protocol playground";
const STREAM_RESOURCE_ID: &str = "playground/scalar";
const STREAM_TIMESTAMP_NS: i64 = 99;
const STREAM_VALUE: f64 = 12.5;
const REGISTRY_ID: &str = "playground/base";
const REGISTRY_HASH: &str = "00000000000000000000000000000000";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerCard {
    version: u8,
    runtime: String,
    domain_id: String,
    peer_id: String,
    protocols: Vec<String>,
    routes: PeerRoutes,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PeerRoutes {
    tcp: String,
    wss: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Command {
    Discover {
        id: String,
        #[serde(default)]
        protocol: Option<String>,
    },
    ProbeAll {
        id: String,
        target: PeerCard,
    },
    ProbeDiscovered {
        id: String,
        target: DiscoveredPeer,
    },
    Shutdown {
        id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredPeer {
    peer_id: String,
    routes: Vec<String>,
    served_protocols: Vec<String>,
    expires_at: String,
    source: String,
}

struct ProbeTarget {
    peer_id: PeerId,
    route: Multiaddr,
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

struct MountedProtocols {
    clients: ProtocolClients,
    info: InfoEndpoint,
    catalog: CatalogEndpoint,
    registry: RegistryEndpoint,
    blob: BlobEndpoint,
    message: MessageEndpoint,
    stream: StreamEndpoint,
    message_drain: tokio::task::JoinHandle<Result<()>>,
}

#[derive(Default)]
struct PartialMountedProtocols {
    info: Option<InfoEndpoint>,
    catalog: Option<CatalogEndpoint>,
    registry: Option<RegistryEndpoint>,
    blob: Option<BlobEndpoint>,
    message: Option<MessageEndpoint>,
    stream: Option<StreamEndpoint>,
    message_drain: Option<tokio::task::JoinHandle<Result<()>>>,
}

#[derive(Clone)]
struct CatalogFixture {
    message_channel: MessageChannelResource,
}

impl CatalogProvider for CatalogFixture {
    fn resources(
        &self,
        _requester: &auki_sdk::AuthenticatedPeer,
        _request: &catalog_v3::ResourcesRequest,
    ) -> catalog_v3::ResourcesResponse {
        catalog_v3::ResourcesResponse {
            resources: vec![ResourceEntry::MessageChannel(self.message_channel.clone())],
        }
    }

    fn maps(&self, _requester: &auki_sdk::AuthenticatedPeer) -> catalog_v4::ResourcesResponse {
        catalog_v4::ResourcesResponse { resources: vec![] }
    }
}

#[derive(Clone)]
struct BlobFixture {
    sha256: String,
    bytes: Arc<[u8]>,
}

impl BlobProvider for BlobFixture {
    fn provide<'a>(
        &'a self,
        _remote_peer: &'a auki_sdk::AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a> {
        Box::pin(async move {
            if request.sha256 != self.sha256 {
                return Ok(None);
            }
            let start = usize::try_from(request.offset)
                .map_err(|_| BlobProviderError::new("blob offset does not fit this platform"))?;
            if start > self.bytes.len() {
                return Err(BlobProviderError::new("blob offset exceeds fixture size"));
            }
            let requested = usize::try_from(request.max_len)
                .map_err(|_| BlobProviderError::new("blob range does not fit this platform"))?;
            let end = start.saturating_add(requested).min(self.bytes.len());
            Ok(Some(ProvidedBlobChunk::new(
                self.bytes.len() as u64,
                self.bytes[start..end].to_vec(),
            )))
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let domain_id = required_env("AUKI_DOMAIN_ID")?
        .parse::<Uuid>()
        .context("AUKI_DOMAIN_ID must be a UUID")?;
    let identity_file = PathBuf::from(required_env("AUKI_IDENTITY_FILE")?);
    let discovery_mode = discovery_mode_from_env()?;
    let bootstrap = AukiPeerBootstrap::dev(credentials_from_env()?)
        .await?
        .with_dds_tracker(discovery_mode);
    let peer = bootstrap
        .start_persistent_peer(DomainSelection::new(domain_id), identity_file)
        .await?;

    let protocols = match MountedProtocols::mount(&peer).await {
        Ok(protocols) => protocols,
        Err(error) => {
            let peer_shutdown = peer.shutdown().await.map_err(anyhow::Error::from);
            return finish(Err(error), [("Auki peer", peer_shutdown)]);
        }
    };
    let operation = async {
        let card = peer_card(&peer)?;
        emit(&serde_json::json!({
            "event": "ready",
            "runtime": "native",
            "card": card,
        }))?;

        let discovery = peer.discovery_handle()?;
        tokio::select! {
            operation = command_loop(&protocols.clients, &discovery) => operation,
            signal = tokio::signal::ctrl_c() => signal.context("wait for Ctrl-C"),
        }
    }
    .await;
    let protocol_shutdown = protocols.close().await;
    let peer_shutdown = peer.shutdown().await.map_err(anyhow::Error::from);
    finish(
        operation,
        [
            ("standard protocol endpoints", protocol_shutdown),
            ("Auki peer", peer_shutdown),
        ],
    )?;
    emit(&serde_json::json!({"event": "stopped", "runtime": "native"}))?;
    Ok(())
}

impl MountedProtocols {
    async fn mount(peer: &AukiPeer) -> Result<Self> {
        let protocols = peer.protocols();
        let local_peer_id = peer.peer_id();
        let node_name = env::var("AUKI_NODE_NAME").unwrap_or_else(|_| "native-playground".into());
        let message_channel = message_channel(local_peer_id);
        let mut mounted = PartialMountedProtocols::default();
        let mounting: Result<()> = async {
            mounted.info = Some(InfoEndpoint::mount(
                protocols.clone(),
                move |_requester: &auki_sdk::AuthenticatedPeer| {
                    Some(AuthenticatedParticipantInfo {
                        app: APP.into(),
                        app_version: APP_VERSION.into(),
                        name: node_name.clone(),
                        session_id: "playground-session".into(),
                        session_clock_id: MESSAGE_CLOCK_ID.into(),
                        session_clock_hash: MESSAGE_CLOCK_HASH.into(),
                        session_now_ns: 0,
                        peer_id: local_peer_id,
                        app_instance: "native".into(),
                    })
                },
            )?);
            mounted.catalog = Some(CatalogEndpoint::mount(
                protocols.clone(),
                CatalogFixture {
                    message_channel: message_channel.clone(),
                },
            )?);
            mounted.registry = Some(RegistryEndpoint::mount(
                protocols.clone(),
                |_requester: &auki_sdk::AuthenticatedPeer, request: &RegistryRequest| {
                    registry_response(request)
                },
            )?);
            mounted.blob = Some(BlobEndpoint::mount(
                protocols.clone(),
                BlobFixture {
                    sha256: blob_sha256(),
                    bytes: Arc::from(BLOB_BYTES),
                },
            )?);
            mounted.message = Some(MessageEndpoint::mount(protocols.clone())?);
            let mut receiver = mounted
                .message
                .as_ref()
                .expect("Message endpoint was just mounted")
                .declare(message_channel, 16)?;
            mounted.message_drain = Some(tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    validate_message_event(&event)?;
                }
                Ok(())
            }));
            mounted.stream = Some(StreamEndpoint::mount(
                protocols,
                move |_requester: &auki_sdk::AuthenticatedPeer, request| {
                    stream_dispatch(local_peer_id, request)
                },
            )?);
            Ok(())
        }
        .await;

        if let Err(error) = mounting {
            return match mounted.close().await {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(error.context(format!(
                    "ordered mount rollback also failed: {cleanup_error:#}"
                ))),
            };
        }
        Ok(mounted.complete())
    }

    async fn close(self) -> Result<()> {
        let Self {
            clients: _,
            info,
            catalog,
            registry,
            blob,
            message,
            stream,
            message_drain,
        } = self;
        PartialMountedProtocols {
            info: Some(info),
            catalog: Some(catalog),
            registry: Some(registry),
            blob: Some(blob),
            message: Some(message),
            stream: Some(stream),
            message_drain: Some(message_drain),
        }
        .close()
        .await
    }
}

impl PartialMountedProtocols {
    fn complete(mut self) -> MountedProtocols {
        let info = self.info.take().expect("Info endpoint is mounted");
        let catalog = self.catalog.take().expect("Catalog endpoint is mounted");
        let registry = self.registry.take().expect("Registry endpoint is mounted");
        let blob = self.blob.take().expect("Blob endpoint is mounted");
        let message = self.message.take().expect("Message endpoint is mounted");
        let stream = self.stream.take().expect("Stream endpoint is mounted");
        let message_drain = self.message_drain.take().expect("Message drain is running");
        MountedProtocols {
            clients: ProtocolClients {
                info: info.client(),
                catalog: catalog.client(),
                registry: registry.client(),
                blob: blob.client(),
                message: message.client(),
                stream: stream.client(),
            },
            info,
            catalog,
            registry,
            blob,
            message,
            stream,
            message_drain,
        }
    }

    async fn close(mut self) -> Result<()> {
        let mut errors = Vec::new();
        if let Some(stream) = self.stream.take() {
            collect_close(&mut errors, "Stream", stream.close().await);
        }
        if let Some(message) = self.message.take() {
            collect_close(&mut errors, "Message", message.close().await);
        }
        if let Some(message_drain) = self.message_drain.take() {
            match message_drain.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("Message receiver: {error:#}")),
                Err(error) => errors.push(format!("Message receiver task: {error}")),
            }
        }
        if let Some(blob) = self.blob.take() {
            collect_close(&mut errors, "Blob", blob.close().await);
        }
        if let Some(registry) = self.registry.take() {
            collect_close(&mut errors, "Registry", registry.close().await);
        }
        if let Some(catalog) = self.catalog.take() {
            collect_close(&mut errors, "Catalog", catalog.close().await);
        }
        if let Some(info) = self.info.take() {
            collect_close(&mut errors, "Info", info.close().await);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            bail!("endpoint shutdown failed: {}", errors.join("; "))
        }
    }
}

async fn command_loop(clients: &ProtocolClients, discovery: &AukiDiscovery) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await.context("read JSONL command")? {
        let command = match serde_json::from_str::<Command>(&line) {
            Ok(command) => command,
            Err(error) => {
                emit(&serde_json::json!({
                    "event": "command_error",
                    "error": format!("invalid command: {error}"),
                }))?;
                continue;
            }
        };
        match command {
            Command::Discover { id, protocol } => {
                emit_discovery_result(
                    &id,
                    protocol.as_deref(),
                    discover(discovery, protocol.as_deref()).await,
                )?;
            }
            Command::ProbeAll { id, target } => {
                let results = match manual_target(&target) {
                    Ok(target) => probe_all(clients, target).await,
                    Err(error) => failed_checks(format!("{error:#}")),
                };
                emit_probe_result(&id, &target.peer_id, results)?;
            }
            Command::ProbeDiscovered { id, target } => {
                let results = match discovered_target(&target) {
                    Ok(target) => probe_all(clients, target).await,
                    Err(error) => failed_checks(format!("{error:#}")),
                };
                emit_probe_result(&id, &target.peer_id, results)?;
            }
            Command::Shutdown { id } => {
                emit(&serde_json::json!({"event": "shutdown_ack", "id": id}))?;
                return Ok(());
            }
        }
    }
    Ok(())
}

async fn discover(
    discovery: &AukiDiscovery,
    protocol: Option<&str>,
) -> Result<Vec<DiscoveredPeer>> {
    let candidates = match protocol {
        Some(protocol) => discovery.discover_protocol(protocol).await?,
        None => discovery.discover().await?,
    };
    Ok(candidates.into_iter().map(discovered_peer).collect())
}

fn discovered_peer(candidate: AukiDiscoveryCandidate) -> DiscoveredPeer {
    DiscoveredPeer {
        peer_id: candidate.peer_id().to_string(),
        routes: candidate.routes().iter().map(ToString::to_string).collect(),
        served_protocols: candidate.served_protocols().to_vec(),
        expires_at: candidate.expires_at().to_rfc3339(),
        source: match candidate.source() {
            AukiDiscoverySource::DdsTracker => "dds_tracker".into(),
        },
    }
}

fn emit_discovery_result(
    id: &str,
    protocol: Option<&str>,
    result: Result<Vec<DiscoveredPeer>>,
) -> Result<()> {
    match result {
        Ok(candidates) => emit(&serde_json::json!({
            "event": "discovery_result",
            "id": id,
            "ok": true,
            "protocol": protocol,
            "candidates": candidates,
        })),
        Err(error) => emit(&serde_json::json!({
            "event": "discovery_result",
            "id": id,
            "ok": false,
            "protocol": protocol,
            "candidates": [],
            "error": format!("{error:#}"),
        })),
    }
}

async fn probe_all(
    clients: &ProtocolClients,
    target: ProbeTarget,
) -> BTreeMap<&'static str, Result<()>> {
    let ProbeTarget { peer_id, route } = target;
    let mut checks = BTreeMap::new();
    checks.insert(
        "info",
        probe_info(&clients.info, peer_id, route.clone()).await,
    );
    checks.insert(
        "catalog",
        probe_catalog(&clients.catalog, peer_id, route.clone()).await,
    );
    checks.insert(
        "registry",
        probe_registry(&clients.registry, peer_id, route.clone()).await,
    );
    checks.insert(
        "blob",
        probe_blob(&clients.blob, peer_id, route.clone()).await,
    );
    checks.insert(
        "message",
        probe_message(&clients.message, peer_id, route.clone()).await,
    );
    checks.insert(
        "stream",
        probe_stream(&clients.stream, peer_id, route).await,
    );
    checks
}

fn manual_target(target: &PeerCard) -> Result<ProbeTarget> {
    Ok(ProbeTarget {
        peer_id: target
            .peer_id
            .parse::<PeerId>()
            .context("invalid target Peer ID")?,
        route: target
            .routes
            .tcp
            .parse::<Multiaddr>()
            .context("invalid target TCP route")?,
    })
}

fn discovered_target(target: &DiscoveredPeer) -> Result<ProbeTarget> {
    let missing = protocol_ids()
        .into_iter()
        .filter(|protocol| !target.served_protocols.contains(protocol))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "discovered peer {} does not advertise every standard protocol: {}",
            target.peer_id,
            missing.join(", ")
        );
    }
    let route = target
        .routes
        .iter()
        .filter(|route| route.contains("/tcp/") && !route.contains("/wss/"))
        .min_by_key(|route| !route.contains("/p2p-circuit/"))
        .ok_or_else(|| anyhow!("discovered peer {} has no native TCP route", target.peer_id))?;
    Ok(ProbeTarget {
        peer_id: target
            .peer_id
            .parse::<PeerId>()
            .context("invalid discovered Peer ID")?,
        route: route
            .parse::<Multiaddr>()
            .context("invalid discovered TCP route")?,
    })
}

async fn probe_info(client: &InfoClient, peer_id: PeerId, route: Multiaddr) -> Result<()> {
    let info = client.fetch_exact(peer_id, route).await?;
    if info.peer_id != peer_id || info.app != APP || info.app_version != APP_VERSION {
        bail!("unexpected participant info from {peer_id}")
    }
    Ok(())
}

async fn probe_catalog(client: &CatalogClient, peer_id: PeerId, route: Multiaddr) -> Result<()> {
    let resources = client
        .fetch_all_resources_exact(peer_id, route.clone())
        .await?;
    let expected = message_channel(peer_id);
    match resources.resources.as_slice() {
        [ResourceEntry::MessageChannel(actual)] if actual == &expected => {}
        actual => bail!("unexpected Catalog v3 resources: {actual:?}"),
    }
    let maps = client.fetch_maps_exact(peer_id, route).await?;
    if !maps.resources.is_empty() {
        bail!("Catalog v4 fixture unexpectedly advertised maps")
    }
    Ok(())
}

async fn probe_registry(client: &RegistryClient, peer_id: PeerId, route: Multiaddr) -> Result<()> {
    let entries = client
        .list_exact(peer_id, route, RegistryKind::Frame)
        .await?;
    if !matches!(
        entries.as_slice(),
        [entry]
            if entry.id == REGISTRY_ID
                && entry.hash.len() == 32
                && entry.hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    ) {
        bail!("unexpected Registry list: {entries:?}")
    }
    Ok(())
}

async fn probe_blob(client: &BlobClient, peer_id: PeerId, route: Multiaddr) -> Result<()> {
    let receipt = client.fetch_exact(peer_id, route, blob_sha256()).await?;
    if receipt.remote_peer_id != peer_id || receipt.bytes != BLOB_BYTES || !receipt.relayed {
        bail!("unexpected Blob receipt from {peer_id}")
    }
    Ok(())
}

async fn probe_message(client: &MessageClient, peer_id: PeerId, route: Multiaddr) -> Result<()> {
    let sender = client
        .open_exact(peer_id, route, &message_channel(peer_id))
        .await?;
    let metadata = if sender.remote_peer().peer_id == peer_id && sender.is_relayed() {
        Ok(())
    } else {
        Err(anyhow!(
            "unexpected Message channel metadata from {peer_id}"
        ))
    };
    let send = sender
        .send(MESSAGE_TYPE, MESSAGE_TIMESTAMP_NS, MESSAGE_BYTES)
        .await;
    let close = sender.close().await;
    metadata?;
    send?;
    close?;
    Ok(())
}

async fn probe_stream(client: &StreamClient, peer_id: PeerId, route: Multiaddr) -> Result<()> {
    let mut subscription = client
        .subscribe_exact::<scalar::Data>(
            peer_id,
            route,
            StreamRequest {
                source_peer_id: peer_id.to_string(),
                resource_id: STREAM_RESOURCE_ID.into(),
                from: ReadFrom::Latest,
            },
        )
        .await?;
    if subscription.manifest.resource_id != STREAM_RESOURCE_ID
        || subscription.manifest.payload != "scalar"
    {
        bail!("unexpected Stream manifest")
    }
    let entry = subscription
        .entries
        .next()
        .await
        .ok_or_else(|| anyhow!("Stream ended before its fixture entry"))??;
    if entry.seq != 0
        || entry.timestamp_ns != STREAM_TIMESTAMP_NS
        || entry.payload.value != STREAM_VALUE
    {
        bail!("unexpected Stream fixture entry")
    }
    Ok(())
}

fn stream_dispatch(local_peer_id: PeerId, request: StreamRequest) -> StreamDispatch {
    if request.source_peer_id != local_peer_id.to_string()
        || request.resource_id != STREAM_RESOURCE_ID
    {
        return StreamDispatch::Decline {
            reason: auki_protocols::stream::v2::DeclineReason::sensor_not_found(),
        };
    }
    StreamDispatch::AcceptScalar {
        manifest: StreamManifest {
            resource_id: STREAM_RESOURCE_ID.into(),
            payload: "scalar".into(),
            ..Default::default()
        },
        source: Box::pin(futures::stream::iter([Ok(StreamItem {
            timestamp_ns: STREAM_TIMESTAMP_NS,
            payload: scalar::Data {
                value: STREAM_VALUE,
            },
        })])),
    }
}

fn registry_response(request: &RegistryRequest) -> RegistryResponse {
    match request {
        RegistryRequest::List { .. } => RegistryResponse::List {
            entries: vec![RegistryListEntry {
                id: REGISTRY_ID.into(),
                hash: REGISTRY_HASH.into(),
            }],
        },
        RegistryRequest::Get { .. } => RegistryResponse::Get { entry: None },
    }
}

fn message_channel(peer_id: PeerId) -> MessageChannelResource {
    MessageChannelResource {
        owner_peer_id: peer_id,
        resource_id: MESSAGE_RESOURCE_ID.into(),
        clock: RegistryRef {
            peer_id: peer_id.to_string(),
            id: MESSAGE_CLOCK_ID.into(),
            hash: MESSAGE_CLOCK_HASH.into(),
        },
    }
}

fn blob_sha256() -> String {
    hex::encode(Sha256::digest(BLOB_BYTES))
}

fn validate_message_event(event: &MessageEvent) -> Result<()> {
    if event.message_type() != MESSAGE_TYPE
        || event.timestamp_ns() != MESSAGE_TIMESTAMP_NS
        || event.payload() != MESSAGE_BYTES
    {
        bail!(
            "received an invalid Message fixture from {}",
            event.sender.peer_id
        );
    }
    eprintln!(
        "message received from {} type={} bytes={}",
        event.sender.peer_id,
        event.message_type(),
        event.payload().len()
    );
    Ok(())
}

fn peer_card(peer: &AukiPeer) -> Result<PeerCard> {
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
        protocols: protocol_ids(),
        routes: PeerRoutes {
            tcp: published.routes.tcp().to_string(),
            wss: published.routes.wss().to_string(),
        },
    })
}

fn protocol_ids() -> Vec<String> {
    [
        auki_protocols::info::v1::ID,
        catalog_v3::ID,
        catalog_v4::ID,
        auki_protocols::registry::v3::ID,
        auki_protocols::blob::v1::ID,
        auki_protocols::message::v1::ID,
        auki_protocols::stream::v2::ID,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn emit(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    io::stdout().flush().context("flush JSONL event")
}

fn emit_probe_result(
    id: &str,
    target_peer_id: &str,
    results: BTreeMap<&'static str, Result<()>>,
) -> Result<()> {
    let checks = results
        .iter()
        .map(|(name, result)| (*name, result.is_ok()))
        .collect::<BTreeMap<_, _>>();
    let errors = results
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|error| (name, format!("{error:#}"))))
        .collect::<BTreeMap<_, _>>();
    emit(&serde_json::json!({
        "event": "probe_result",
        "id": id,
        "targetPeerId": target_peer_id,
        "ok": errors.is_empty(),
        "checks": checks,
        "errors": errors,
    }))
}

fn failed_checks(reason: String) -> BTreeMap<&'static str, Result<()>> {
    ["info", "catalog", "registry", "blob", "message", "stream"]
        .into_iter()
        .map(|name| (name, Err(anyhow!(reason.clone()))))
        .collect()
}

fn collect_close<T: std::fmt::Display>(
    errors: &mut Vec<String>,
    name: &str,
    result: Result<(), T>,
) {
    if let Err(error) = result {
        errors.push(format!("{name}: {error}"));
    }
}

fn finish<const N: usize>(operation: Result<()>, cleanup: [(&str, Result<()>); N]) -> Result<()> {
    let cleanup_errors = cleanup
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|error| format!("{name}: {error:#}")))
        .collect::<Vec<_>>();
    match (operation, cleanup_errors.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Ok(()), false) => bail!("ordered shutdown failed: {}", cleanup_errors.join("; ")),
        (Err(error), true) => Err(error),
        (Err(error), false) => Err(error.context(format!(
            "cleanup also failed: {}",
            cleanup_errors.join("; ")
        ))),
    }
}

fn credentials_from_env() -> Result<Credentials> {
    let user = optional_pair("AUKI_EMAIL", "AUKI_PASSWORD")?;
    let app = optional_pair("AUKI_APP_ACCESS_KEY", "AUKI_APP_SECRET")?;
    match (user, app) {
        (Some((email, password)), None) => Ok(Credentials::user_password(email, password)),
        (None, Some((access_key, secret))) => Ok(Credentials::app(access_key, secret)),
        (None, None) => {
            bail!("set AUKI_EMAIL/AUKI_PASSWORD or AUKI_APP_ACCESS_KEY/AUKI_APP_SECRET")
        }
        (Some(_), Some(_)) => bail!("configure either User or App credentials, not both"),
    }
}

fn discovery_mode_from_env() -> Result<DdsTrackerMode> {
    match env::var("AUKI_DISCOVERY_MODE") {
        Ok(value) if value == "discover_only" => Ok(DdsTrackerMode::DiscoverOnly),
        Ok(value) if value == "discover_and_advertise" => Ok(DdsTrackerMode::DiscoverAndAdvertise),
        Ok(value) => bail!(
            "AUKI_DISCOVERY_MODE must be discover_only or discover_and_advertise, got {value:?}"
        ),
        Err(env::VarError::NotPresent) => Ok(DdsTrackerMode::DiscoverAndAdvertise),
        Err(error) => Err(error).context("read AUKI_DISCOVERY_MODE"),
    }
}

fn optional_pair(first: &'static str, second: &'static str) -> Result<Option<(String, String)>> {
    match (env::var(first), env::var(second)) {
        (Ok(first_value), Ok(second_value)) => Ok(Some((first_value, second_value))),
        (Err(env::VarError::NotPresent), Err(env::VarError::NotPresent)) => Ok(None),
        (Err(error), _) => Err(error).with_context(|| format!("read {first}")),
        (_, Err(error)) => Err(error).with_context(|| format!("read {second}")),
    }
}

fn required_env(name: &'static str) -> Result<String> {
    env::var(name).with_context(|| format!("{name} is required"))
}
