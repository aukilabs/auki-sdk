use std::{path::Path, time::Duration};

use auki_p2p::PeerId;
use auki_protocols::registry::{
    v2::{
        ID as REGISTRIES_V0_2_0, RegistryRequest as RegistryRequestV2,
        RegistryResponse as RegistryResponseV2, read_registry_request as read_registry_request_v2,
        read_registry_response as read_registry_response_v2,
        write_registry_request as write_registry_request_v2,
        write_registry_response as write_registry_response_v2,
    },
    v3::{
        ID as REGISTRIES_V0_3_0, MAX_REGISTRIES_FRAME_BYTES, RegistriesProtocolError,
        RegistryEntryEnvelope, RegistryKind, RegistryListEntry, RegistryRequest, RegistryResponse,
        read_registry_request, read_registry_response, write_registry_request,
        write_registry_response,
    },
};
use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, DeviceModelRegistryEntry, FrameRegistryEntry,
    MapRegistryEntry, SensorRegistryEntry,
};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::{
    protocols::{
        DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
        DomainProtocols,
    },
    storage::{RegistryBlobStorage, StorageError},
};

const REGISTRIES_MAX_CONCURRENCY: usize = 16;
const REGISTRIES_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Manager-free adapter for both retained Registry payload versions.
///
/// Version 0.3 remains the preferred List/Get protocol. A non-DeviceModel Get
/// may fall back to authenticated version 0.2 only when every configured route
/// reached the peer and rejected the 0.3 protocol ID. Authentication, routing,
/// timeout, and codec failures never trigger fallback.
#[derive(Clone)]
pub(crate) struct Registries {
    protocols: DomainProtocols,
    storage: RegistryBlobStorage,
    lifecycle: CancellationToken,
}

impl Registries {
    pub(super) fn new(
        protocols: DomainProtocols,
        storage: RegistryBlobStorage,
        lifecycle: CancellationToken,
    ) -> Self {
        Self {
            protocols,
            storage,
            lifecycle,
        }
    }

    pub(super) fn register_v2(&self) -> Result<DomainProtocolRegistration, RegistriesError> {
        let spec = DomainProtocolSpec::new(
            REGISTRIES_V0_2_0,
            REGISTRIES_MAX_CONCURRENCY,
            MAX_REGISTRIES_FRAME_BYTES,
        )?;
        let registries = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let registries = registries.clone();
                async move {
                    if let Err(error) = registries.handle_v2(stream).await {
                        tracing::warn!(%error, "authenticated Registry v0.2 request failed");
                    }
                }
            })
            .map_err(RegistriesError::Protocol)
    }

    pub(super) fn register_v3(&self) -> Result<DomainProtocolRegistration, RegistriesError> {
        let spec = DomainProtocolSpec::new(
            REGISTRIES_V0_3_0,
            REGISTRIES_MAX_CONCURRENCY,
            MAX_REGISTRIES_FRAME_BYTES,
        )?;
        let registries = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let registries = registries.clone();
                async move {
                    if let Err(error) = registries.handle_v3(stream).await {
                        tracing::warn!(%error, "authenticated Registry v0.3 request failed");
                    }
                }
            })
            .map_err(RegistriesError::Protocol)
    }

    pub(crate) fn set_app_root(
        &self,
        app_root: impl Into<std::path::PathBuf>,
    ) -> Result<(), RegistriesError> {
        self.storage
            .set_app_root(app_root)
            .map_err(|StorageError::Stopped| RegistriesError::Stopped)?;
        Ok(())
    }

    pub(crate) async fn request_v2(
        &self,
        expected_peer: PeerId,
        request: RegistryRequestV2,
    ) -> Result<RegistryResponseV2, RegistriesError> {
        self.ensure_running()?;
        if request.kind == RegistryKind::DeviceModel {
            return Err(RegistriesError::UnsupportedProtocol);
        }
        validate_registry_key(&request.id, &request.hash)?;
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => Err(RegistriesError::Stopped),
            result = timeout(REGISTRIES_EXCHANGE_TIMEOUT, async {
                let mut stream = self.protocols.open(expected_peer, REGISTRIES_V0_2_0).await?;
                write_registry_request_v2(&mut stream, &request).await?;
                read_registry_response_v2(&mut stream)
                    .await
                    .map_err(RegistriesError::Codec)
            }) => result.map_err(|_| RegistriesError::Timeout(REGISTRIES_EXCHANGE_TIMEOUT))?,
        }
    }

    pub(crate) async fn request_v3(
        &self,
        expected_peer: PeerId,
        request: RegistryRequest,
    ) -> Result<RegistryResponse, RegistriesError> {
        self.ensure_running()?;
        if let RegistryRequest::Get { id, hash, .. } = &request {
            validate_registry_key(id, hash)?;
        }
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => Err(RegistriesError::Stopped),
            result = timeout(REGISTRIES_EXCHANGE_TIMEOUT, async {
                let mut stream = self.protocols.open(expected_peer, REGISTRIES_V0_3_0).await?;
                write_registry_request(&mut stream, &request).await?;
                read_registry_response(&mut stream)
                    .await
                    .map_err(RegistriesError::Codec)
            }) => result.map_err(|_| RegistriesError::Timeout(REGISTRIES_EXCHANGE_TIMEOUT))?,
        }
    }

    pub(crate) async fn list(
        &self,
        expected_peer: PeerId,
        kind: RegistryKind,
    ) -> Result<Vec<RegistryListEntry>, RegistriesError> {
        self.ensure_running()?;
        if kind != RegistryKind::DeviceModel {
            return Err(RegistriesError::InvalidEnvelope(
                "list is only implemented for device_model".into(),
            ));
        }
        let request = RegistryRequest::list(kind);
        let response = if expected_peer == self.storage.local_peer_id() {
            self.local_v3(&request)?
        } else {
            match self.request_v3(expected_peer, request).await {
                Err(RegistriesError::Protocol(error))
                    if error.all_routes_unsupported_protocol() =>
                {
                    return Err(RegistriesError::UnsupportedProtocol);
                }
                result => result?,
            }
        };
        match response {
            RegistryResponse::List { entries } => Ok(entries),
            RegistryResponse::Get { .. } => Err(RegistriesError::InvalidEnvelope(
                "registry peer replied with Get to a List request".into(),
            )),
            RegistryResponse::Error { reason } => Err(RegistriesError::InvalidEnvelope(reason)),
        }
    }

    pub(crate) async fn fetch_sensor(
        &self,
        expected_peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<SensorRegistryEntry, RegistriesError> {
        let id = id.into();
        let envelope = self
            .fetch_envelope(expected_peer, RegistryKind::Sensor, id.clone(), hash.into())
            .await?;
        let entry: SensorRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        verify_typed_identity(expected_peer, &id, &entry.peer_id, &entry.sensor_id)?;
        Ok(entry)
    }

    pub(crate) async fn fetch_clock(
        &self,
        expected_peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<ClockRegistryEntry, RegistriesError> {
        let id = id.into();
        let envelope = self
            .fetch_envelope(expected_peer, RegistryKind::Clock, id.clone(), hash.into())
            .await?;
        let entry: ClockRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        verify_typed_identity(expected_peer, &id, &entry.peer_id, &entry.clock_id)?;
        Ok(entry)
    }

    pub(crate) async fn fetch_frame(
        &self,
        expected_peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<FrameRegistryEntry, RegistriesError> {
        let id = id.into();
        let envelope = self
            .fetch_envelope(expected_peer, RegistryKind::Frame, id.clone(), hash.into())
            .await?;
        let entry: FrameRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        verify_typed_identity(expected_peer, &id, &entry.peer_id, &entry.frame_id)?;
        entry
            .validate()
            .map_err(|error| RegistriesError::InvalidEnvelope(error.to_string()))?;
        Ok(entry)
    }

    pub(crate) async fn fetch_detector(
        &self,
        expected_peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<DetectorRegistryEntry, RegistriesError> {
        let id = id.into();
        let envelope = self
            .fetch_envelope(
                expected_peer,
                RegistryKind::Detector,
                id.clone(),
                hash.into(),
            )
            .await?;
        let entry: DetectorRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        verify_typed_identity(expected_peer, &id, &entry.peer_id, &entry.detector_id)?;
        Ok(entry)
    }

    pub(crate) async fn fetch_map(
        &self,
        expected_peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<MapRegistryEntry, RegistriesError> {
        let id = id.into();
        let envelope = self
            .fetch_envelope(expected_peer, RegistryKind::Map, id.clone(), hash.into())
            .await?;
        let entry: MapRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        verify_typed_identity(expected_peer, &id, &entry.peer_id, &entry.map_id)?;
        entry
            .validate()
            .map_err(|error| RegistriesError::InvalidEnvelope(error.to_string()))?;
        Ok(entry)
    }

    pub(crate) async fn fetch_device_model(
        &self,
        expected_peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<DeviceModelRegistryEntry, RegistriesError> {
        let id = id.into();
        let envelope = self
            .fetch_envelope(
                expected_peer,
                RegistryKind::DeviceModel,
                id.clone(),
                hash.into(),
            )
            .await?;
        let entry: DeviceModelRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        verify_typed_identity(expected_peer, &id, &entry.peer_id, &entry.device_model_id)?;
        entry
            .validate()
            .map_err(|error| RegistriesError::InvalidEnvelope(error.to_string()))?;
        Ok(entry)
    }

    async fn fetch_envelope(
        &self,
        expected_peer: PeerId,
        kind: RegistryKind,
        id: String,
        hash: String,
    ) -> Result<RegistryEntryEnvelope, RegistriesError> {
        self.ensure_running()?;
        validate_registry_key(&id, &hash)?;
        let request = RegistryRequest::get(kind, id.clone(), hash.clone());
        let response = if expected_peer == self.storage.local_peer_id() {
            self.local_v3(&request)?
        } else {
            self.request_v3_with_v2_fallback(expected_peer, request)
                .await?
        };
        let entry = match response {
            RegistryResponse::Get { entry } => entry,
            RegistryResponse::List { .. } => {
                return Err(RegistriesError::InvalidEnvelope(
                    "registry peer replied with List to a Get request".into(),
                ));
            }
            RegistryResponse::Error { reason } => {
                return Err(RegistriesError::InvalidEnvelope(reason));
            }
        };
        let Some(envelope) = entry else {
            return Err(RegistriesError::NotFound { kind, id, hash });
        };
        verify_registry_envelope(&envelope, kind, &id, &hash)?;
        Ok(envelope)
    }

    async fn request_v3_with_v2_fallback(
        &self,
        expected_peer: PeerId,
        request: RegistryRequest,
    ) -> Result<RegistryResponse, RegistriesError> {
        match self.request_v3(expected_peer, request.clone()).await {
            Err(RegistriesError::Protocol(error)) if error.all_routes_unsupported_protocol() => {
                let Some(v2) = v2_fallback_request(&request) else {
                    return Err(RegistriesError::UnsupportedProtocol);
                };
                match self.request_v2(expected_peer, v2).await {
                    Ok(response) => Ok(RegistryResponse::Get {
                        entry: response.entry,
                    }),
                    Err(RegistriesError::Protocol(error))
                        if error.all_routes_unsupported_protocol() =>
                    {
                        Err(RegistriesError::UnsupportedProtocol)
                    }
                    Err(error) => Err(error),
                }
            }
            result => result,
        }
    }

    fn local_v3(&self, request: &RegistryRequest) -> Result<RegistryResponse, RegistriesError> {
        let app_root = self
            .storage
            .app_root()
            .map_err(|StorageError::Stopped| RegistriesError::Stopped)?;
        let response = registry_response_for(
            app_root.as_deref(),
            &self.storage.local_peer_id().to_string(),
            request,
        );
        self.ensure_running()?;
        Ok(response)
    }

    async fn handle_v2(&self, mut stream: DomainProtocolStream) -> Result<(), RegistriesError> {
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => Err(RegistriesError::Stopped),
            result = timeout(REGISTRIES_EXCHANGE_TIMEOUT, async {
                let request = read_registry_request_v2(&mut stream).await?;
                if request.kind == RegistryKind::DeviceModel {
                    return Ok(());
                }
                let response = self.local_v3(&RegistryRequest::get(
                    request.kind,
                    request.id,
                    request.hash,
                ))?;
                if let RegistryResponse::Get { entry } = response {
                    write_registry_response_v2(&mut stream, &RegistryResponseV2 { entry }).await?;
                }
                Ok(())
            }) => result.map_err(|_| RegistriesError::Timeout(REGISTRIES_EXCHANGE_TIMEOUT))?,
        }
    }

    async fn handle_v3(&self, mut stream: DomainProtocolStream) -> Result<(), RegistriesError> {
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => Err(RegistriesError::Stopped),
            result = timeout(REGISTRIES_EXCHANGE_TIMEOUT, async {
                let request = read_registry_request(&mut stream).await?;
                let oversized_reason = match &request {
                    RegistryRequest::Get { .. } => "entry too large",
                    RegistryRequest::List { .. } => "list too large",
                };
                let response = self.local_v3(&request)?;
                match write_registry_response(&mut stream, &response).await {
                    Ok(()) => Ok(()),
                    Err(RegistriesProtocolError::FrameTooLarge { .. }) => {
                        write_registry_response(
                            &mut stream,
                            &RegistryResponse::Error {
                                reason: oversized_reason.into(),
                            },
                        )
                        .await
                        .map_err(RegistriesError::Codec)
                    }
                    Err(error) => Err(RegistriesError::Codec(error)),
                }
            }) => result.map_err(|_| RegistriesError::Timeout(REGISTRIES_EXCHANGE_TIMEOUT))?,
        }
    }

    fn ensure_running(&self) -> Result<(), RegistriesError> {
        self.storage
            .ensure_running()
            .map_err(|StorageError::Stopped| RegistriesError::Stopped)
    }
}

fn v2_fallback_request(request: &RegistryRequest) -> Option<RegistryRequestV2> {
    match request {
        RegistryRequest::Get { kind, id, hash } if *kind != RegistryKind::DeviceModel => {
            Some(RegistryRequestV2 {
                kind: *kind,
                id: id.clone(),
                hash: hash.clone(),
            })
        }
        RegistryRequest::Get { .. } | RegistryRequest::List { .. } => None,
    }
}

fn validate_registry_key(id: &str, hash: &str) -> Result<(), RegistriesError> {
    auki_registry::validate_registry_id(id).map_err(|error| {
        RegistriesError::InvalidEnvelope(format!("invalid registry id: {error}"))
    })?;
    if !auki_registry::is_registry_entry_hash(hash) {
        return Err(RegistriesError::InvalidEnvelope(
            "registry hash must be 32 lowercase hex characters".into(),
        ));
    }
    Ok(())
}

fn registry_response_for(
    app_root: Option<&Path>,
    local_peer_id: &str,
    request: &RegistryRequest,
) -> RegistryResponse {
    match (app_root, request) {
        (Some(root), RegistryRequest::Get { kind, id, hash }) => {
            match read_registry_envelope(root, local_peer_id, *kind, id, hash) {
                Ok(entry) => RegistryResponse::Get { entry },
                Err(error) => RegistryResponse::Error {
                    reason: format!("get failed: {error}"),
                },
            }
        }
        (None, RegistryRequest::Get { .. }) => RegistryResponse::Error {
            reason: "registry not configured".into(),
        },
        (
            Some(root),
            RegistryRequest::List {
                kind: RegistryKind::DeviceModel,
            },
        ) => match list_registry_refs(root, local_peer_id) {
            Ok(entries) => RegistryResponse::List { entries },
            Err(auki_registry::Error::RegistryListLimit) => RegistryResponse::Error {
                reason: "list too large".into(),
            },
            Err(_) => RegistryResponse::Error {
                reason: "list failed".into(),
            },
        },
        (Some(_), RegistryRequest::List { .. }) => RegistryResponse::Error {
            reason: "list is only implemented for device_model".into(),
        },
        (None, RegistryRequest::List { .. }) => RegistryResponse::Error {
            reason: "registry not configured".into(),
        },
    }
}

fn read_registry_envelope(
    app_root: &Path,
    peer_id: &str,
    kind: RegistryKind,
    id: &str,
    hash: &str,
) -> Result<Option<RegistryEntryEnvelope>, RegistrySourceError> {
    // Keep malformed ids away from filesystem path construction. Hash shape,
    // size, and on-disk ownership are enforced by every `auki-registry` read.
    auki_registry::validate_registry_id(id)
        .map_err(|error| RegistrySourceError::InvalidRequest(error.to_string()))?;
    match kind {
        RegistryKind::Sensor => {
            let Some(entry) = auki_registry::read_sensor(app_root, peer_id, id, hash)? else {
                return Ok(None);
            };
            verify_source_owner(peer_id, &entry.peer_id)?;
            Ok(Some(envelope_for_sensor(entry)))
        }
        RegistryKind::Clock => {
            let Some(entry) = auki_registry::read_clock(app_root, peer_id, id, hash)? else {
                return Ok(None);
            };
            verify_source_owner(peer_id, &entry.peer_id)?;
            Ok(Some(envelope_for_clock(entry)))
        }
        RegistryKind::Frame => {
            let Some(entry) = auki_registry::read_frame(app_root, peer_id, id, hash)? else {
                return Ok(None);
            };
            verify_source_owner(peer_id, &entry.peer_id)?;
            Ok(Some(envelope_for_frame(entry)))
        }
        RegistryKind::Detector => {
            let Some(entry) = auki_registry::read_detector(app_root, peer_id, id, hash)? else {
                return Ok(None);
            };
            verify_source_owner(peer_id, &entry.peer_id)?;
            Ok(Some(envelope_for_detector(entry)))
        }
        RegistryKind::Map => {
            let Some(entry) = auki_registry::read_map(app_root, peer_id, id, hash)? else {
                return Ok(None);
            };
            verify_source_owner(peer_id, &entry.peer_id)?;
            Ok(Some(envelope_for_map(entry)))
        }
        RegistryKind::DeviceModel => {
            let Some(entry) = auki_registry::read_device_model(app_root, peer_id, id, hash)? else {
                return Ok(None);
            };
            verify_source_owner(peer_id, &entry.peer_id)?;
            Ok(Some(envelope_for_device_model(entry)))
        }
    }
}

fn list_registry_refs(
    app_root: &Path,
    peer_id: &str,
) -> Result<Vec<RegistryListEntry>, auki_registry::Error> {
    Ok(auki_registry::list_device_models(app_root, peer_id)?
        .into_iter()
        .map(|entry| RegistryListEntry {
            id: entry.id,
            hash: entry.hash,
        })
        .collect())
}

fn verify_source_owner(expected: &str, actual: &str) -> Result<(), RegistrySourceError> {
    if actual == expected {
        Ok(())
    } else {
        Err(RegistrySourceError::OwnerMismatch {
            expected: expected.into(),
            actual: actual.into(),
        })
    }
}

fn verify_registry_envelope(
    envelope: &RegistryEntryEnvelope,
    expected_kind: RegistryKind,
    expected_id: &str,
    expected_hash: &str,
) -> Result<(), RegistriesError> {
    if envelope.kind != expected_kind {
        return Err(RegistriesError::InvalidEnvelope(format!(
            "kind mismatch: expected {}, found {}",
            expected_kind, envelope.kind
        )));
    }
    if envelope.id != expected_id {
        return Err(RegistriesError::InvalidEnvelope(format!(
            "id mismatch: expected {:?}, found {:?}",
            expected_id, envelope.id
        )));
    }
    if envelope.hash != expected_hash {
        return Err(RegistriesError::InvalidEnvelope(format!(
            "hash field mismatch: expected {}, found {}",
            expected_hash, envelope.hash
        )));
    }
    let actual_hash = auki_hash::hash_jcs_bytes(envelope.canonical_json.as_bytes());
    if actual_hash != expected_hash {
        return Err(RegistriesError::HashMismatch {
            expected: expected_hash.into(),
            actual: actual_hash,
        });
    }
    Ok(())
}

fn verify_typed_identity(
    expected_peer: PeerId,
    expected_id: &str,
    actual_peer: &str,
    actual_id: &str,
) -> Result<(), RegistriesError> {
    if actual_peer != expected_peer.to_string() {
        return Err(RegistriesError::OwnerMismatch {
            expected: Box::new(expected_peer),
            actual: actual_peer.into(),
        });
    }
    if actual_id != expected_id {
        return Err(RegistriesError::InvalidEnvelope(format!(
            "decoded registry id mismatch: expected {:?}, found {:?}",
            expected_id, actual_id
        )));
    }
    Ok(())
}

fn envelope_for_sensor(entry: SensorRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Sensor,
        id: entry.sensor_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_clock(entry: ClockRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Clock,
        id: entry.clock_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_frame(entry: FrameRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Frame,
        id: entry.frame_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_detector(entry: DetectorRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Detector,
        id: entry.detector_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_map(entry: MapRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Map,
        id: entry.map_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_device_model(entry: DeviceModelRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::DeviceModel,
        id: entry.device_model_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

#[derive(Debug, thiserror::Error)]
enum RegistrySourceError {
    #[error(transparent)]
    Registry(#[from] auki_registry::Error),
    #[error("invalid registry request: {0}")]
    InvalidRequest(String),
    #[error("registry owner mismatch: expected {expected:?}, found {actual:?}")]
    OwnerMismatch { expected: String, actual: String },
}

#[derive(Debug, thiserror::Error)]
pub enum RegistriesError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("authenticated Registry protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("Registry codec failed: {0}")]
    Codec(#[from] RegistriesProtocolError),
    #[error("the peer supports neither eligible authenticated Registry version")]
    UnsupportedProtocol,
    #[error("Registry exchange exceeded {0:?}")]
    Timeout(Duration),
    #[error("registry entry not found: kind={kind} id={id:?} hash={hash}")]
    NotFound {
        kind: RegistryKind,
        id: String,
        hash: String,
    },
    #[error("invalid registry envelope: {0}")]
    InvalidEnvelope(String),
    #[error("registry hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("registry owner mismatch: expected {expected}, found {actual:?}")]
    OwnerMismatch {
        expected: Box<PeerId>,
        actual: String,
    },
    #[error("invalid registry JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use auki_p2p::Identity;
    use auki_registry::FrameRegistryEntry;

    use super::*;

    fn peer(seed: u8) -> PeerId {
        Identity::from_ed25519_seed(&[seed; 32]).peer_id()
    }

    #[test]
    fn fallback_is_get_only_and_excludes_device_models() {
        assert!(
            v2_fallback_request(&RegistryRequest::get(
                RegistryKind::Frame,
                "frame",
                "a".repeat(32),
            ))
            .is_some()
        );
        assert!(
            v2_fallback_request(&RegistryRequest::get(
                RegistryKind::DeviceModel,
                "robot",
                "a".repeat(32),
            ))
            .is_none()
        );
        assert!(v2_fallback_request(&RegistryRequest::list(RegistryKind::DeviceModel)).is_none());
    }

    #[test]
    fn local_registry_response_preserves_missing_error_and_owner_rules() {
        let root = tempfile::tempdir().unwrap();
        let owner = peer(1).to_string();
        let entry = FrameRegistryEntry::ros_body(owner.clone(), "base");
        let hash = auki_registry::write_frame(root.path(), &entry)
            .unwrap()
            .hash()
            .to_owned();

        assert!(matches!(
            registry_response_for(
                Some(root.path()),
                &owner,
                &RegistryRequest::get(RegistryKind::Frame, "base", &hash),
            ),
            RegistryResponse::Get { entry: Some(_) }
        ));
        assert!(matches!(
            registry_response_for(
                Some(root.path()),
                &owner,
                &RegistryRequest::get(RegistryKind::Frame, "base", "bad"),
            ),
            RegistryResponse::Error { ref reason } if reason.starts_with("get failed:")
        ));
        assert!(matches!(
            registry_response_for(
                None,
                &owner,
                &RegistryRequest::get(RegistryKind::Frame, "base", &hash),
            ),
            RegistryResponse::Error { ref reason } if reason == "registry not configured"
        ));

        let other_entry = FrameRegistryEntry::ros_body(peer(2).to_string(), "other");
        let other_bytes = other_entry.canonical_bytes();
        let other_hash = auki_hash::hash_jcs_bytes(&other_bytes);
        let planted = auki_layout::frame_entry_path(root.path(), &owner, "other", &other_hash);
        std::fs::create_dir_all(planted.parent().unwrap()).unwrap();
        std::fs::write(planted, other_bytes).unwrap();
        assert!(matches!(
            registry_response_for(
                Some(root.path()),
                &owner,
                &RegistryRequest::get(RegistryKind::Frame, "other", other_hash),
            ),
            RegistryResponse::Error { ref reason } if reason.contains("mismatch")
        ));
    }

    #[test]
    fn envelope_verification_rejects_kind_id_field_and_content_hash_changes() {
        let expected_json = r#"{"peer_id":"peer","frame_id":"frame"}"#;
        let hash = auki_hash::hash_jcs_bytes(expected_json.as_bytes());
        let valid = RegistryEntryEnvelope {
            kind: RegistryKind::Frame,
            id: "frame".into(),
            hash: hash.clone(),
            canonical_json: expected_json.into(),
        };
        verify_registry_envelope(&valid, RegistryKind::Frame, "frame", &hash).unwrap();

        let mut wrong = valid.clone();
        wrong.kind = RegistryKind::Clock;
        assert!(matches!(
            verify_registry_envelope(&wrong, RegistryKind::Frame, "frame", &hash),
            Err(RegistriesError::InvalidEnvelope(_))
        ));
        let mut wrong = valid.clone();
        wrong.id = "other".into();
        assert!(matches!(
            verify_registry_envelope(&wrong, RegistryKind::Frame, "frame", &hash),
            Err(RegistriesError::InvalidEnvelope(_))
        ));
        let mut wrong = valid.clone();
        wrong.hash = "0".repeat(32);
        assert!(matches!(
            verify_registry_envelope(&wrong, RegistryKind::Frame, "frame", &hash),
            Err(RegistriesError::InvalidEnvelope(_))
        ));
        let mut wrong = valid;
        wrong.canonical_json.push(' ');
        assert!(matches!(
            verify_registry_envelope(&wrong, RegistryKind::Frame, "frame", &hash),
            Err(RegistriesError::HashMismatch { .. })
        ));
    }

    #[test]
    fn typed_identity_requires_exact_authenticated_owner_and_id() {
        let expected = peer(3);
        verify_typed_identity(expected, "frame", &expected.to_string(), "frame").unwrap();
        assert!(matches!(
            verify_typed_identity(expected, "frame", &peer(4).to_string(), "frame"),
            Err(RegistriesError::OwnerMismatch { .. })
        ));
        assert!(matches!(
            verify_typed_identity(expected, "frame", &expected.to_string(), "other"),
            Err(RegistriesError::InvalidEnvelope(_))
        ));
    }

    #[test]
    fn protocol_ids_and_bounds_are_locked() {
        assert_eq!(REGISTRIES_V0_2_0, "/auki/auth/1/registries/0.2.0");
        assert_eq!(REGISTRIES_V0_3_0, "/auki/auth/1/registries/0.3.0");
        assert_eq!(MAX_REGISTRIES_FRAME_BYTES, 64 * 1024);
        assert_eq!(REGISTRIES_EXCHANGE_TIMEOUT, Duration::from_secs(5));
        assert_eq!(REGISTRIES_MAX_CONCURRENCY, 16);
        PeerId::from_str(&peer(5).to_string()).unwrap();
    }
}
