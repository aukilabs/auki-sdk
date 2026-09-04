//! Portable [`auki_sdk::AukiPeer`] endpoint for Registry v3.
//!
//! The wire contract remains transport-neutral. This module owns only the
//! mechanical peer integration: registration, fixed operation deadlines,
//! authenticated requester admission, and validation of received entries.

use std::{fmt, future::Future, time::Duration};

use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, DeviceModelRegistryEntry, FrameRegistryEntry,
    MapRegistryEntry, SensorRegistryEntry,
};
use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AuthenticatedPeer, AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::AsyncWriteExt;
use serde::de::DeserializeOwned;

use crate::endpoint_support::{Shared, clone_shared, deadline_after, prefer_primary, share};

use super::v3::{
    ID, MAX_REGISTRIES_FRAME_BYTES, RegistriesProtocolError, RegistryEntryEnvelope, RegistryKind,
    RegistryListEntry, RegistryRequest, RegistryResponse, read_registry_request,
    read_registry_response, write_registry_request, write_registry_response,
};

/// Maximum number of concurrently served Registry requests.
pub const REGISTRY_MAX_CONCURRENCY: usize = 16;

/// Fixed deadline for each open, exchange, and close operation.
pub const REGISTRY_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

type ProviderHandle = Shared<dyn RegistryProvider>;

/// Application-owned source of Registry v3 responses.
///
/// The requester has already completed mutual DDS authentication. Providers
/// should return [`RegistryResponse::Error`] when the authenticated requester
/// is not allowed to perform an otherwise valid request.
#[cfg(not(target_arch = "wasm32"))]
pub trait RegistryProvider: Send + Sync + 'static {
    /// Resolve one validated request for an authenticated requester.
    fn respond(&self, requester: &AuthenticatedPeer, request: &RegistryRequest)
    -> RegistryResponse;
}

/// Application-owned source of Registry v3 responses.
///
/// Browser providers are local to the Wasm executor. The requester has
/// already completed mutual DDS authentication.
#[cfg(target_arch = "wasm32")]
pub trait RegistryProvider: 'static {
    /// Resolve one validated request for an authenticated requester.
    fn respond(&self, requester: &AuthenticatedPeer, request: &RegistryRequest)
    -> RegistryResponse;
}

#[cfg(not(target_arch = "wasm32"))]
impl<F> RegistryProvider for F
where
    F: Fn(&AuthenticatedPeer, &RegistryRequest) -> RegistryResponse + Send + Sync + 'static,
{
    fn respond(
        &self,
        requester: &AuthenticatedPeer,
        request: &RegistryRequest,
    ) -> RegistryResponse {
        self(requester, request)
    }
}

#[cfg(target_arch = "wasm32")]
impl<F> RegistryProvider for F
where
    F: Fn(&AuthenticatedPeer, &RegistryRequest) -> RegistryResponse + 'static,
{
    fn respond(
        &self,
        requester: &AuthenticatedPeer,
        request: &RegistryRequest,
    ) -> RegistryResponse {
        self(requester, request)
    }
}

/// Build the exact bounded Registry v3 registration.
pub fn registry_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(ID, REGISTRY_MAX_CONCURRENCY, MAX_REGISTRIES_FRAME_BYTES)
}

macro_rules! typed_fetch_methods {
    (
        $fetch:ident,
        $fetch_exact:ident,
        $entry:ty,
        $kind:expr,
        $fetch_doc:literal,
        $fetch_exact_doc:literal
    ) => {
        #[doc = $fetch_doc]
        #[cfg(not(target_arch = "wasm32"))]
        pub async fn $fetch(
            &self,
            remote_peer_id: PeerId,
            id: impl Into<String>,
            hash: impl Into<String>,
        ) -> Result<$entry, RegistryEndpointError> {
            let id = id.into();
            let hash = hash.into();
            let response = self
                .request(
                    remote_peer_id,
                    RegistryRequest::get($kind, id.clone(), hash.clone()),
                )
                .await?;
            decode_get_response(remote_peer_id, $kind, &id, &hash, response)
        }

        #[doc = $fetch_exact_doc]
        pub async fn $fetch_exact(
            &self,
            remote_peer_id: PeerId,
            route: Multiaddr,
            id: impl Into<String>,
            hash: impl Into<String>,
        ) -> Result<$entry, RegistryEndpointError> {
            let id = id.into();
            let hash = hash.into();
            let response = self
                .request_exact(
                    remote_peer_id,
                    route,
                    RegistryRequest::get($kind, id.clone(), hash.clone()),
                )
                .await?;
            decode_get_response(remote_peer_id, $kind, &id, &hash, response)
        }
    };
}

/// Cloneable outbound Registry v3 client.
#[derive(Clone)]
pub struct RegistryClient {
    protocols: AukiPeerProtocols,
}

impl RegistryClient {
    /// Construct a client over one running peer's protocol surface.
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    /// Perform one Registry v3 exchange using routes configured on the native peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn request(
        &self,
        remote_peer_id: PeerId,
        request: RegistryRequest,
    ) -> Result<RegistryResponse, RegistryEndpointError> {
        validate_request(&request)?;
        request_opened(request, self.protocols.open(remote_peer_id, ID)).await
    }

    /// Perform one Registry v3 exchange through an exact advertised route.
    pub async fn request_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        request: RegistryRequest,
    ) -> Result<RegistryResponse, RegistryEndpointError> {
        validate_request(&request)?;
        request_opened(
            request,
            self.protocols.open_exact(remote_peer_id, route, ID),
        )
        .await
    }

    /// List one Registry namespace using routes configured on the native peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn list(
        &self,
        remote_peer_id: PeerId,
        kind: RegistryKind,
    ) -> Result<Vec<RegistryListEntry>, RegistryEndpointError> {
        let response = self
            .request(remote_peer_id, RegistryRequest::list(kind))
            .await?;
        decode_list_response(response)
    }

    /// List one Registry namespace through an exact advertised route.
    pub async fn list_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        kind: RegistryKind,
    ) -> Result<Vec<RegistryListEntry>, RegistryEndpointError> {
        let response = self
            .request_exact(remote_peer_id, route, RegistryRequest::list(kind))
            .await?;
        decode_list_response(response)
    }

    typed_fetch_methods!(
        fetch_sensor,
        fetch_sensor_exact,
        SensorRegistryEntry,
        RegistryKind::Sensor,
        "Fetch and validate one Sensor Registry entry using configured native routes.",
        "Fetch and validate one Sensor Registry entry through an exact route."
    );

    typed_fetch_methods!(
        fetch_clock,
        fetch_clock_exact,
        ClockRegistryEntry,
        RegistryKind::Clock,
        "Fetch and validate one Clock Registry entry using configured native routes.",
        "Fetch and validate one Clock Registry entry through an exact route."
    );

    typed_fetch_methods!(
        fetch_frame,
        fetch_frame_exact,
        FrameRegistryEntry,
        RegistryKind::Frame,
        "Fetch and validate one Frame Registry entry using configured native routes.",
        "Fetch and validate one Frame Registry entry through an exact route."
    );

    typed_fetch_methods!(
        fetch_detector,
        fetch_detector_exact,
        DetectorRegistryEntry,
        RegistryKind::Detector,
        "Fetch and validate one Detector Registry entry using configured native routes.",
        "Fetch and validate one Detector Registry entry through an exact route."
    );

    typed_fetch_methods!(
        fetch_map,
        fetch_map_exact,
        MapRegistryEntry,
        RegistryKind::Map,
        "Fetch and validate one Map Registry entry using configured native routes.",
        "Fetch and validate one Map Registry entry through an exact route."
    );

    typed_fetch_methods!(
        fetch_device_model,
        fetch_device_model_exact,
        DeviceModelRegistryEntry,
        RegistryKind::DeviceModel,
        "Fetch and validate one Device Model Registry entry using configured native routes.",
        "Fetch and validate one Device Model Registry entry through an exact route."
    );
}

/// Mounted Registry v3 service plus its outbound client.
pub struct RegistryEndpoint {
    client: RegistryClient,
    registration: AukiProtocolRegistration,
}

impl RegistryEndpoint {
    /// Mount Registry v3 on one running Auki peer.
    pub fn mount<P>(
        protocols: AukiPeerProtocols,
        provider: P,
    ) -> Result<Self, RegistryEndpointError>
    where
        P: RegistryProvider,
    {
        let provider: ProviderHandle = share(provider);
        let registration = protocols.register(registry_protocol_spec()?, move |mut stream| {
            let provider = clone_shared(&provider);
            async move {
                let requester = stream.remote_peer().clone();
                let exchange = deadline(RegistryOperation::Exchange, async {
                    let request = read_registry_request(&mut stream).await?;
                    let response = match validate_request(&request) {
                        Ok(()) => provider.respond(&requester, &request),
                        Err(error) => RegistryResponse::Error {
                            reason: error.to_string(),
                        },
                    };
                    write_bounded_response(&mut stream, &request, &response).await?;
                    Ok::<_, RegistryEndpointError>(())
                })
                .await
                .and_then(|result| result);
                let cleanup = close_stream(&mut stream).await;
                let _ = prefer_primary(exchange, cleanup);
            }
        })?;

        Ok(Self {
            client: RegistryClient::new(protocols),
            registration,
        })
    }

    /// Clone the outbound client without cloning registration ownership.
    pub fn client(&self) -> RegistryClient {
        self.client.clone()
    }

    /// Stop accepting requests and await all admitted handlers.
    pub async fn close(self) -> Result<(), RegistryEndpointError> {
        self.registration
            .close()
            .await
            .map_err(RegistryEndpointError::Sdk)
    }
}

async fn request_opened<F>(
    request: RegistryRequest,
    opening: F,
) -> Result<RegistryResponse, RegistryEndpointError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(RegistryOperation::Open, opening)
        .await?
        .map_err(RegistryEndpointError::Sdk)?;
    let exchange = deadline(RegistryOperation::Exchange, async {
        write_registry_request(&mut stream, &request).await?;
        read_registry_response(&mut stream)
            .await
            .map_err(RegistryEndpointError::Codec)
    })
    .await
    .and_then(|result| result);
    let cleanup = close_stream(&mut stream).await;
    prefer_primary(exchange, cleanup)
}

async fn write_bounded_response<S>(
    stream: &mut S,
    request: &RegistryRequest,
    response: &RegistryResponse,
) -> Result<(), RegistryEndpointError>
where
    S: AsyncWriteExt + Unpin,
{
    match write_registry_response(stream, response).await {
        Ok(()) => Ok(()),
        Err(RegistriesProtocolError::FrameTooLarge { .. }) => {
            let reason = match request {
                RegistryRequest::Get { .. } => "entry too large",
                RegistryRequest::List { .. } => "list too large",
            };
            write_registry_response(
                stream,
                &RegistryResponse::Error {
                    reason: reason.into(),
                },
            )
            .await
            .map_err(RegistryEndpointError::Codec)
        }
        Err(error) => Err(RegistryEndpointError::Codec(error)),
    }
}

async fn close_stream<S>(stream: &mut S) -> Result<(), RegistryEndpointError>
where
    S: AsyncWriteExt + Unpin,
{
    deadline(RegistryOperation::Close, stream.close())
        .await?
        .map_err(|error| RegistryEndpointError::Close(error.to_string()))
}

async fn deadline<T>(
    operation: RegistryOperation,
    future: impl Future<Output = T>,
) -> Result<T, RegistryEndpointError> {
    deadline_after(REGISTRY_OPERATION_TIMEOUT, future, || {
        RegistryEndpointError::Timeout(operation)
    })
    .await
}

fn validate_request(request: &RegistryRequest) -> Result<(), RegistryEndpointError> {
    if let RegistryRequest::Get { id, hash, .. } = request {
        validate_registry_key(id, hash)?;
    }
    Ok(())
}

fn validate_registry_key(id: &str, hash: &str) -> Result<(), RegistryEndpointError> {
    auki_registry::validate_registry_id(id).map_err(|error| {
        RegistryEndpointError::InvalidRequest(format!("invalid registry id: {error}"))
    })?;
    if !auki_registry::is_registry_entry_hash(hash) {
        return Err(RegistryEndpointError::InvalidRequest(
            "registry hash must be 32 lowercase hex characters".into(),
        ));
    }
    Ok(())
}

fn decode_list_response(
    response: RegistryResponse,
) -> Result<Vec<RegistryListEntry>, RegistryEndpointError> {
    match response {
        RegistryResponse::List { entries } => {
            for entry in &entries {
                validate_registry_key(&entry.id, &entry.hash).map_err(|error| {
                    RegistryEndpointError::InvalidEnvelope(format!(
                        "invalid Registry List entry: {error}"
                    ))
                })?;
            }
            Ok(entries)
        }
        RegistryResponse::Get { .. } => Err(RegistryEndpointError::UnexpectedResponse {
            expected: "list",
            actual: "get",
        }),
        RegistryResponse::Error { reason } => Err(RegistryEndpointError::Remote(reason)),
    }
}

fn decode_get_response<T>(
    expected_peer: PeerId,
    expected_kind: RegistryKind,
    expected_id: &str,
    expected_hash: &str,
    response: RegistryResponse,
) -> Result<T, RegistryEndpointError>
where
    T: TypedRegistryEntry,
{
    debug_assert_eq!(T::KIND, expected_kind);
    let envelope = match response {
        RegistryResponse::Get { entry: Some(entry) } => entry,
        RegistryResponse::Get { entry: None } => {
            return Err(RegistryEndpointError::NotFound {
                kind: expected_kind,
                id: expected_id.into(),
                hash: expected_hash.into(),
            });
        }
        RegistryResponse::List { .. } => {
            return Err(RegistryEndpointError::UnexpectedResponse {
                expected: "get",
                actual: "list",
            });
        }
        RegistryResponse::Error { reason } => {
            return Err(RegistryEndpointError::Remote(reason));
        }
    };

    verify_registry_envelope(&envelope, expected_kind, expected_id, expected_hash)?;
    let entry: T = serde_json::from_str(&envelope.canonical_json)?;
    verify_typed_identity(
        expected_peer,
        expected_id,
        entry.owner_peer_id(),
        entry.registry_id(),
    )?;
    entry
        .validate_entry()
        .map_err(RegistryEndpointError::InvalidEnvelope)?;
    Ok(entry)
}

fn verify_registry_envelope(
    envelope: &RegistryEntryEnvelope,
    expected_kind: RegistryKind,
    expected_id: &str,
    expected_hash: &str,
) -> Result<(), RegistryEndpointError> {
    if envelope.kind != expected_kind {
        return Err(RegistryEndpointError::InvalidEnvelope(format!(
            "kind mismatch: expected {}, found {}",
            expected_kind, envelope.kind
        )));
    }
    if envelope.id != expected_id {
        return Err(RegistryEndpointError::InvalidEnvelope(format!(
            "id mismatch: expected {expected_id:?}, found {:?}",
            envelope.id
        )));
    }
    if envelope.hash != expected_hash {
        return Err(RegistryEndpointError::InvalidEnvelope(format!(
            "hash field mismatch: expected {expected_hash}, found {}",
            envelope.hash
        )));
    }
    let actual_hash = auki_hash::hash_jcs_bytes(envelope.canonical_json.as_bytes());
    if actual_hash != expected_hash {
        return Err(RegistryEndpointError::HashMismatch {
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
) -> Result<(), RegistryEndpointError> {
    if actual_peer != expected_peer.to_string() {
        return Err(RegistryEndpointError::OwnerMismatch {
            expected: Box::new(expected_peer),
            actual: actual_peer.into(),
        });
    }
    if actual_id != expected_id {
        return Err(RegistryEndpointError::InvalidEnvelope(format!(
            "decoded registry id mismatch: expected {expected_id:?}, found {actual_id:?}"
        )));
    }
    Ok(())
}

trait TypedRegistryEntry: DeserializeOwned {
    const KIND: RegistryKind;

    fn owner_peer_id(&self) -> &str;
    fn registry_id(&self) -> &str;

    fn validate_entry(&self) -> Result<(), String> {
        Ok(())
    }
}

impl TypedRegistryEntry for SensorRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Sensor;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.sensor_id
    }
}

impl TypedRegistryEntry for ClockRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Clock;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.clock_id
    }
}

impl TypedRegistryEntry for FrameRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Frame;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.frame_id
    }

    fn validate_entry(&self) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())
    }
}

impl TypedRegistryEntry for DetectorRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Detector;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.detector_id
    }
}

impl TypedRegistryEntry for MapRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Map;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.map_id
    }

    fn validate_entry(&self) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())
    }
}

impl TypedRegistryEntry for DeviceModelRegistryEntry {
    const KIND: RegistryKind = RegistryKind::DeviceModel;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.device_model_id
    }

    fn validate_entry(&self) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())
    }
}

/// One bounded Registry endpoint operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryOperation {
    /// Open and authenticate a protocol stream.
    Open,
    /// Exchange the bounded request and response.
    Exchange,
    /// Close the authenticated stream.
    Close,
}

impl fmt::Display for RegistryOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::Exchange => "exchange",
            Self::Close => "close",
        })
    }
}

/// Failure from the portable Registry v3 endpoint.
#[derive(Debug, thiserror::Error)]
pub enum RegistryEndpointError {
    /// The SDK rejected protocol registration or stream opening.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    /// The Registry wire conversation failed.
    #[error("Registry codec failed: {0}")]
    Codec(#[from] RegistriesProtocolError),
    /// A local or remote request used an invalid registry key.
    #[error("invalid Registry request: {0}")]
    InvalidRequest(String),
    /// The remote provider explicitly rejected or could not fulfill the request.
    #[error("remote Registry error: {0}")]
    Remote(String),
    /// The peer returned the wrong response operation.
    #[error("Registry peer replied with {actual} to a {expected} request")]
    UnexpectedResponse {
        /// Expected response operation.
        expected: &'static str,
        /// Received response operation.
        actual: &'static str,
    },
    /// The peer understood the Get but does not have the exact entry.
    #[error("registry entry not found: kind={kind} id={id:?} hash={hash}")]
    NotFound {
        /// Requested registry namespace.
        kind: RegistryKind,
        /// Requested registry identity.
        id: String,
        /// Requested content hash.
        hash: String,
    },
    /// A returned envelope or typed value violated the Registry contract.
    #[error("invalid Registry envelope: {0}")]
    InvalidEnvelope(String),
    /// Canonical JSON bytes did not match the requested content hash.
    #[error("registry hash mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// Requested content hash.
        expected: String,
        /// Hash computed from the returned canonical JSON bytes.
        actual: String,
    },
    /// The typed entry did not belong to the authenticated peer.
    #[error("registry owner mismatch: expected {expected}, found {actual:?}")]
    OwnerMismatch {
        /// Authenticated remote peer.
        expected: Box<PeerId>,
        /// Peer ID repeated inside the typed Registry entry.
        actual: String,
    },
    /// The canonical JSON could not be decoded into its expected Registry type.
    #[error("invalid Registry JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// One endpoint phase exceeded its fixed deadline.
    #[error("Registry {0} timed out after 5 seconds")]
    Timeout(RegistryOperation),
    /// Stream cleanup failed after the exchange.
    #[error("close Registry stream: {0}")]
    Close(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_sdk::Identity;

    #[test]
    fn spec_mounts_only_the_exact_v3_wire_contract() {
        let spec = registry_protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), ID);
        assert_eq!(spec.max_concurrency(), REGISTRY_MAX_CONCURRENCY);
        assert_eq!(spec.max_frame_bytes(), MAX_REGISTRIES_FRAME_BYTES);
    }

    #[test]
    fn registry_keys_are_validated_before_transport_or_provider_access() {
        let valid_hash = "a".repeat(32);
        assert!(validate_registry_key("frame", &valid_hash).is_ok());
        assert!(matches!(
            validate_registry_key("../frame", &valid_hash),
            Err(RegistryEndpointError::InvalidRequest(_))
        ));
        assert!(matches!(
            validate_registry_key("frame", "ABC"),
            Err(RegistryEndpointError::InvalidRequest(_))
        ));
    }

    #[test]
    fn typed_fetch_validates_envelope_hash_owner_id_and_entry_shape() {
        let expected_peer = Identity::generate().peer_id();
        let entry = FrameRegistryEntry::ros_body(expected_peer.to_string(), "base");
        let canonical_json = String::from_utf8(entry.canonical_bytes()).unwrap();
        let hash = auki_hash::hash_jcs_bytes(canonical_json.as_bytes());
        let envelope = RegistryEntryEnvelope {
            kind: RegistryKind::Frame,
            id: "base".into(),
            hash: hash.clone(),
            canonical_json,
        };

        let decoded = decode_get_response::<FrameRegistryEntry>(
            expected_peer,
            RegistryKind::Frame,
            "base",
            &hash,
            RegistryResponse::Get {
                entry: Some(envelope.clone()),
            },
        )
        .unwrap();
        assert_eq!(decoded, entry);

        let mut wrong_hash = envelope.clone();
        wrong_hash.canonical_json.push(' ');
        assert!(matches!(
            decode_get_response::<FrameRegistryEntry>(
                expected_peer,
                RegistryKind::Frame,
                "base",
                &hash,
                RegistryResponse::Get {
                    entry: Some(wrong_hash)
                },
            ),
            Err(RegistryEndpointError::HashMismatch { .. })
        ));

        let other_peer = Identity::generate().peer_id();
        assert!(matches!(
            decode_get_response::<FrameRegistryEntry>(
                other_peer,
                RegistryKind::Frame,
                "base",
                &hash,
                RegistryResponse::Get {
                    entry: Some(envelope)
                },
            ),
            Err(RegistryEndpointError::OwnerMismatch { .. })
        ));
    }

    #[test]
    fn list_entries_are_validated_before_reaching_callers() {
        let valid = RegistryResponse::List {
            entries: vec![RegistryListEntry {
                id: "robot".into(),
                hash: "a".repeat(32),
            }],
        };
        assert_eq!(decode_list_response(valid).unwrap().len(), 1);

        let invalid = RegistryResponse::List {
            entries: vec![RegistryListEntry {
                id: "../robot".into(),
                hash: "a".repeat(32),
            }],
        };
        assert!(matches!(
            decode_list_response(invalid),
            Err(RegistryEndpointError::InvalidEnvelope(_))
        ));
    }
}
