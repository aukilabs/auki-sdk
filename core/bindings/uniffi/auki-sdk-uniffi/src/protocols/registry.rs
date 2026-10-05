//! Swift adapter for finite Registry v3 clients and in-memory serving.

use std::{collections::BTreeMap, sync::Arc};

use auki_protocols::registry::{
    RegistryClient, RegistryEndpoint, RegistryProvider,
    v3::{
        ID, MAX_REGISTRIES_FRAME_BYTES, RegistryEntryEnvelope, RegistryKind, RegistryListEntry,
        RegistryRequest, RegistryResponse,
    },
};
use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, DeviceModelRegistryEntry, FrameRegistryEntry,
    MapRegistryEntry, SensorRegistryEntry,
};
use auki_sdk_rs::{AuthenticatedPeer, Multiaddr, PeerId};
use parking_lot::RwLock;
use serde::de::DeserializeOwned;

use crate::{AukiPeer, AukiPeerTarget, AukiSdkError, operation_error, wait_cleanup};

use super::finite_support::{CloseFuture, EndpointOwner, exact_target, parse_bounded_json};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, uniffi::Enum)]
pub enum AukiRegistryKind {
    Sensor,
    Clock,
    Frame,
    Detector,
    Map,
    DeviceModel,
}

impl From<AukiRegistryKind> for RegistryKind {
    fn from(value: AukiRegistryKind) -> Self {
        match value {
            AukiRegistryKind::Sensor => Self::Sensor,
            AukiRegistryKind::Clock => Self::Clock,
            AukiRegistryKind::Frame => Self::Frame,
            AukiRegistryKind::Detector => Self::Detector,
            AukiRegistryKind::Map => Self::Map,
            AukiRegistryKind::DeviceModel => Self::DeviceModel,
        }
    }
}

impl From<RegistryKind> for AukiRegistryKind {
    fn from(value: RegistryKind) -> Self {
        match value {
            RegistryKind::Sensor => Self::Sensor,
            RegistryKind::Clock => Self::Clock,
            RegistryKind::Frame => Self::Frame,
            RegistryKind::Detector => Self::Detector,
            RegistryKind::Map => Self::Map,
            RegistryKind::DeviceModel => Self::DeviceModel,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiRegistryListEntry {
    pub id: String,
    pub hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiStoredRegistryEntry {
    pub kind: AukiRegistryKind,
    pub id: String,
    pub hash: String,
    pub canonical_json: String,
}

trait SwiftRegistryEntry: DeserializeOwned {
    fn peer_id(&self) -> &str;
    fn registry_id(&self) -> &str;
    fn validate_for_serving(&self) -> Result<(), String> {
        Ok(())
    }
    fn canonical_bytes(&self) -> Vec<u8>;
    fn content_hash(&self) -> String;
}

macro_rules! simple_registry_entry {
    ($entry:ty, $id:ident) => {
        impl SwiftRegistryEntry for $entry {
            fn peer_id(&self) -> &str {
                &self.peer_id
            }

            fn registry_id(&self) -> &str {
                &self.$id
            }

            fn canonical_bytes(&self) -> Vec<u8> {
                <$entry>::canonical_bytes(self)
            }

            fn content_hash(&self) -> String {
                <$entry>::hash(self)
            }
        }
    };
}

simple_registry_entry!(SensorRegistryEntry, sensor_id);
simple_registry_entry!(ClockRegistryEntry, clock_id);
simple_registry_entry!(DetectorRegistryEntry, detector_id);

macro_rules! validated_registry_entry {
    ($entry:ty, $id:ident) => {
        impl SwiftRegistryEntry for $entry {
            fn peer_id(&self) -> &str {
                &self.peer_id
            }

            fn registry_id(&self) -> &str {
                &self.$id
            }

            fn validate_for_serving(&self) -> Result<(), String> {
                self.validate().map_err(|error| error.to_string())
            }

            fn canonical_bytes(&self) -> Vec<u8> {
                <$entry>::canonical_bytes(self)
            }

            fn content_hash(&self) -> String {
                <$entry>::hash(self)
            }
        }
    };
}

validated_registry_entry!(FrameRegistryEntry, frame_id);
validated_registry_entry!(MapRegistryEntry, map_id);
validated_registry_entry!(DeviceModelRegistryEntry, device_model_id);

fn prepare_typed_entry<T: SwiftRegistryEntry>(
    kind: AukiRegistryKind,
    json: &str,
    local_peer_id: &str,
) -> Result<RegistryEntryEnvelope, AukiSdkError> {
    let entry: T = parse_bounded_json(
        "read Registry entry JSON",
        json,
        MAX_REGISTRIES_FRAME_BYTES as usize,
    )?;
    entry
        .validate_for_serving()
        .map_err(|error| operation_error("validate Registry entry", error))?;
    if entry.peer_id() != local_peer_id {
        return Err(operation_error(
            "validate Registry entry owner",
            format!(
                "entry owner {:?} does not match local peer {local_peer_id}",
                entry.peer_id()
            ),
        ));
    }
    auki_registry::validate_registry_id(entry.registry_id())
        .map_err(|error| operation_error("validate Registry entry ID", error))?;
    let canonical_json = String::from_utf8(entry.canonical_bytes())
        .map_err(|error| operation_error("encode canonical Registry JSON", error))?;
    let envelope = RegistryEntryEnvelope {
        kind: kind.into(),
        id: entry.registry_id().into(),
        hash: entry.content_hash(),
        canonical_json,
    };
    ensure_response_fits(&RegistryResponse::Get {
        entry: Some(envelope.clone()),
    })?;
    Ok(envelope)
}

fn prepare_entry(
    kind: AukiRegistryKind,
    json: &str,
    local_peer_id: &str,
) -> Result<RegistryEntryEnvelope, AukiSdkError> {
    match kind {
        AukiRegistryKind::Sensor => {
            prepare_typed_entry::<SensorRegistryEntry>(kind, json, local_peer_id)
        }
        AukiRegistryKind::Clock => {
            prepare_typed_entry::<ClockRegistryEntry>(kind, json, local_peer_id)
        }
        AukiRegistryKind::Frame => {
            prepare_typed_entry::<FrameRegistryEntry>(kind, json, local_peer_id)
        }
        AukiRegistryKind::Detector => {
            prepare_typed_entry::<DetectorRegistryEntry>(kind, json, local_peer_id)
        }
        AukiRegistryKind::Map => prepare_typed_entry::<MapRegistryEntry>(kind, json, local_peer_id),
        AukiRegistryKind::DeviceModel => {
            prepare_typed_entry::<DeviceModelRegistryEntry>(kind, json, local_peer_id)
        }
    }
}

fn ensure_response_fits(response: &RegistryResponse) -> Result<(), AukiSdkError> {
    let bytes = serde_json::to_vec(response)
        .map_err(|error| operation_error("encode Registry response", error))?;
    if bytes.len() > MAX_REGISTRIES_FRAME_BYTES as usize {
        return Err(operation_error(
            "validate Registry response bound",
            format!(
                "encoded response is {} bytes; maximum is {}",
                bytes.len(),
                MAX_REGISTRIES_FRAME_BYTES
            ),
        ));
    }
    Ok(())
}

fn canonical_entry<T: SwiftRegistryEntry>(entry: T) -> Result<String, AukiSdkError> {
    String::from_utf8(entry.canonical_bytes())
        .map_err(|error| operation_error("encode fetched Registry JSON", error))
}

async fn fetch_entry(
    client: &RegistryClient,
    remote_peer_id: PeerId,
    route: Multiaddr,
    kind: AukiRegistryKind,
    id: String,
    hash: String,
) -> Result<String, AukiSdkError> {
    match kind {
        AukiRegistryKind::Sensor => canonical_entry(
            client
                .fetch_sensor_exact(remote_peer_id, route, id, hash)
                .await
                .map_err(|error| operation_error("fetch Sensor Registry entry", error))?,
        ),
        AukiRegistryKind::Clock => canonical_entry(
            client
                .fetch_clock_exact(remote_peer_id, route, id, hash)
                .await
                .map_err(|error| operation_error("fetch Clock Registry entry", error))?,
        ),
        AukiRegistryKind::Frame => canonical_entry(
            client
                .fetch_frame_exact(remote_peer_id, route, id, hash)
                .await
                .map_err(|error| operation_error("fetch Frame Registry entry", error))?,
        ),
        AukiRegistryKind::Detector => canonical_entry(
            client
                .fetch_detector_exact(remote_peer_id, route, id, hash)
                .await
                .map_err(|error| operation_error("fetch Detector Registry entry", error))?,
        ),
        AukiRegistryKind::Map => canonical_entry(
            client
                .fetch_map_exact(remote_peer_id, route, id, hash)
                .await
                .map_err(|error| operation_error("fetch Map Registry entry", error))?,
        ),
        AukiRegistryKind::DeviceModel => canonical_entry(
            client
                .fetch_device_model_exact(remote_peer_id, route, id, hash)
                .await
                .map_err(|error| operation_error("fetch Device Model Registry entry", error))?,
        ),
    }
}

type EntryKey = (AukiRegistryKind, String);

#[derive(Clone, Default)]
struct RegistrySnapshot {
    entries: Arc<RwLock<BTreeMap<EntryKey, RegistryEntryEnvelope>>>,
}

impl RegistrySnapshot {
    fn put(
        &self,
        kind: AukiRegistryKind,
        entry: RegistryEntryEnvelope,
    ) -> Result<(), AukiSdkError> {
        let key = (kind, entry.id.clone());
        let mut entries = self.entries.write();
        let previous = entries.insert(key.clone(), entry);
        let response = self.list_response(kind, &entries);
        if let Err(error) = ensure_response_fits(&response) {
            match previous {
                Some(previous) => {
                    entries.insert(key, previous);
                }
                None => {
                    entries.remove(&key);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn remove(&self, kind: AukiRegistryKind, id: &str) -> bool {
        self.entries.write().remove(&(kind, id.into())).is_some()
    }

    fn clear(&self) {
        self.entries.write().clear();
    }

    fn list_response(
        &self,
        kind: AukiRegistryKind,
        entries: &BTreeMap<EntryKey, RegistryEntryEnvelope>,
    ) -> RegistryResponse {
        RegistryResponse::List {
            entries: entries
                .iter()
                .filter(|((stored_kind, _), _)| *stored_kind == kind)
                .map(|(_, entry)| RegistryListEntry {
                    id: entry.id.clone(),
                    hash: entry.hash.clone(),
                })
                .collect(),
        }
    }
}

impl RegistryProvider for RegistrySnapshot {
    fn respond(
        &self,
        _requester: &AuthenticatedPeer,
        request: &RegistryRequest,
    ) -> RegistryResponse {
        let entries = self.entries.read();
        match request {
            RegistryRequest::List { kind } => self.list_response((*kind).into(), &entries),
            RegistryRequest::Get { kind, id, hash } => RegistryResponse::Get {
                entry: entries
                    .get(&((*kind).into(), id.clone()))
                    .filter(|entry| entry.hash == *hash)
                    .cloned(),
            },
        }
    }
}

fn close_endpoint(endpoint: RegistryEndpoint) -> CloseFuture {
    Box::pin(async move { endpoint.close().await.map_err(|error| error.to_string()) })
}

#[derive(uniffi::Object)]
pub struct AukiRegistryClient {
    inner: RegistryClient,
    domain_id: String,
}

impl AukiRegistryClient {
    fn from_inner(inner: RegistryClient, domain_id: String) -> Arc<Self> {
        Arc::new(Self { inner, domain_id })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiRegistryClient {
    #[uniffi::constructor]
    pub fn new(peer: Arc<AukiPeer>) -> Arc<Self> {
        Self::from_inner(RegistryClient::new(peer.rust_protocols()), peer.domain_id())
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    pub async fn list_exact(
        &self,
        target: AukiPeerTarget,
        kind: AukiRegistryKind,
    ) -> Result<Vec<AukiRegistryListEntry>, AukiSdkError> {
        let (peer_id, route) = exact_target(&self.domain_id, target)?;
        self.inner
            .list_exact(peer_id, route, kind.into())
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| AukiRegistryListEntry {
                        id: entry.id,
                        hash: entry.hash,
                    })
                    .collect()
            })
            .map_err(|error| operation_error("list Registry entries", error))
    }

    /// Fetch a typed, owner-bound, hash-validated entry as canonical JSON.
    pub async fn fetch_exact(
        &self,
        target: AukiPeerTarget,
        kind: AukiRegistryKind,
        id: String,
        hash: String,
    ) -> Result<String, AukiSdkError> {
        let (peer_id, route) = exact_target(&self.domain_id, target)?;
        fetch_entry(&self.inner, peer_id, route, kind, id, hash).await
    }
}

/// Registry v3 endpoint backed by one bounded current entry per kind and ID.
#[derive(uniffi::Object)]
pub struct AukiRegistryEndpoint {
    owner: EndpointOwner<RegistryEndpoint>,
    snapshot: RegistrySnapshot,
    client: Arc<AukiRegistryClient>,
    peer_id: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiRegistryEndpoint {
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiSdkError> {
        let snapshot = RegistrySnapshot::default();
        let endpoint = RegistryEndpoint::mount(peer.rust_protocols(), snapshot.clone())
            .map_err(|error| operation_error("mount Registry endpoint", error))?;
        let client = AukiRegistryClient::from_inner(endpoint.client(), peer.domain_id());
        Ok(Arc::new(Self {
            owner: EndpointOwner::new(endpoint, close_endpoint),
            snapshot,
            client,
            peer_id: peer.peer_id(),
        }))
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    pub fn client(&self) -> Arc<AukiRegistryClient> {
        Arc::clone(&self.client)
    }

    /// Canonicalize, validate, and atomically replace the current entry for its ID.
    pub fn put_json(
        &self,
        kind: AukiRegistryKind,
        json: String,
    ) -> Result<AukiStoredRegistryEntry, AukiSdkError> {
        self.owner.ensure_open("put Registry entry")?;
        let entry = prepare_entry(kind, &json, &self.peer_id)?;
        self.snapshot.put(kind, entry.clone())?;
        Ok(AukiStoredRegistryEntry {
            kind,
            id: entry.id,
            hash: entry.hash,
            canonical_json: entry.canonical_json,
        })
    }

    pub fn remove(&self, kind: AukiRegistryKind, id: String) -> Result<bool, AukiSdkError> {
        self.owner.ensure_open("remove Registry entry")?;
        auki_registry::validate_registry_id(&id)
            .map_err(|error| operation_error("validate Registry entry ID", error))?;
        Ok(self.snapshot.remove(kind, &id))
    }

    pub fn clear(&self) -> Result<(), AukiSdkError> {
        self.owner.ensure_open("clear Registry entries")?;
        self.snapshot.clear();
        Ok(())
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Registry endpoint", error))
    }
}

#[cfg(test)]
mod tests {
    use auki_sdk_rs::Identity;

    use super::*;

    #[test]
    fn put_canonicalizes_hashes_lists_and_replaces_one_current_id() {
        let peer_id = Identity::generate().peer_id();
        let entry = FrameRegistryEntry::ros_body(peer_id.to_string(), "base");
        let source = serde_json::to_string_pretty(&entry).unwrap();
        let prepared = prepare_entry(AukiRegistryKind::Frame, &source, &peer_id.to_string())
            .expect("prepare frame");
        assert_eq!(prepared.hash, entry.hash());
        assert_eq!(prepared.canonical_json.as_bytes(), entry.canonical_bytes());

        let snapshot = RegistrySnapshot::default();
        snapshot
            .put(AukiRegistryKind::Frame, prepared.clone())
            .unwrap();
        let requester =
            super::super::finite_support::authenticated_peer(Identity::generate().peer_id());
        assert_eq!(
            snapshot.respond(&requester, &RegistryRequest::list(RegistryKind::Frame)),
            RegistryResponse::List {
                entries: vec![RegistryListEntry {
                    id: "base".into(),
                    hash: prepared.hash.clone(),
                }],
            }
        );
        assert_eq!(
            snapshot.respond(
                &requester,
                &RegistryRequest::get(RegistryKind::Frame, "base", prepared.hash.clone())
            ),
            RegistryResponse::Get {
                entry: Some(prepared)
            }
        );
    }

    #[test]
    fn put_rejects_entries_owned_by_another_peer() {
        let local = Identity::generate().peer_id();
        let other = Identity::generate().peer_id();
        let entry = FrameRegistryEntry::ros_body(other.to_string(), "base");
        assert!(
            prepare_entry(
                AukiRegistryKind::Frame,
                &serde_json::to_string(&entry).unwrap(),
                &local.to_string(),
            )
            .is_err()
        );
    }

    #[test]
    fn all_six_kinds_map_to_the_locked_registry_namespaces() {
        let cases = [
            (AukiRegistryKind::Sensor, RegistryKind::Sensor),
            (AukiRegistryKind::Clock, RegistryKind::Clock),
            (AukiRegistryKind::Frame, RegistryKind::Frame),
            (AukiRegistryKind::Detector, RegistryKind::Detector),
            (AukiRegistryKind::Map, RegistryKind::Map),
            (AukiRegistryKind::DeviceModel, RegistryKind::DeviceModel),
        ];
        for (binding, protocol) in cases {
            assert_eq!(RegistryKind::from(binding), protocol);
            assert_eq!(AukiRegistryKind::from(protocol), binding);
        }
    }
}
