//! Cluster lifecycle for the Auki SDK.
//!
//! Owns the cluster membership document ([`ClusterMembership`]),
//! the join protocol, the Manager state machine, peer-side heartbeat,
//! successor election, and Manager-handoff orchestration.
//!
//! Not the home for `convert_time` / `convert_pose` — those operate
//! inside a cluster but live elsewhere. Not the home for log-writing
//! session lifecycle either.
//!
//! ## Status
//!
//! Cluster membership document type lands first. Manager state
//! machine, join protocol, heartbeat, election, and handoff follow.

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use serde_json::Value;
use std::str::FromStr;

#[cfg(not(target_arch = "wasm32"))]
#[path = "cluster_manager.rs"]
pub mod cluster_manager;
#[path = "cluster_membership.rs"]
pub mod cluster_membership;
#[cfg(not(target_arch = "wasm32"))]
#[path = "stream_manifest.rs"]
pub mod stream_manifest;

#[cfg(not(target_arch = "wasm32"))]
pub use auki_network::registries_protocol::RegistryKind;
#[cfg(not(target_arch = "wasm32"))]
pub use auki_network::registries_protocol::{RegistryEntryEnvelope, RegistryRequest};
#[cfg(not(target_arch = "wasm32"))]
pub use auki_network::resources_protocol::{
    ResourceEntry, ResourceKind, ResourcePinholeIntrinsics, ResourceQuat, ResourceSpatialTransform,
    ResourceVec3, ResourcesRequest, ResourcesResponse, SensorStreamResource, TransformEdgeResource,
};
#[cfg(not(target_arch = "wasm32"))]
pub use auki_registry::{ClockRegistryEntry, FrameRegistryEntry, SensorRegistryEntry};
#[cfg(not(target_arch = "wasm32"))]
pub use auki_time::{ClockTransformEstimate, DomainClockEstimate};
#[cfg(not(target_arch = "wasm32"))]
pub use cluster_manager::{
    AdmitError, BootstrapError, ClusterManager, ClusterTarget, CreateClusterError, DaemonInfo,
    DiagnosticMessage, DiscoveryClientError, DiscoveryClusterEntry, DomainClockEstimateUnavailable,
    DomainTimeNowError, FetchParticipantInfoError, FetchRegistryEntryError,
    FetchResourcesCatalogError, FetchSensorsCatalogError, InboundDiagnosticMessage,
    JoinClusterError, LIVENESS_CHECK_INTERVAL, RegistryEntryProvider, ResourceCatalogProvider,
    SensorCatalogProvider, SensorEntry, SensorsRequest, SensorsResponse, elect_successor,
};
pub use cluster_membership::{ClusterMember, ClusterMembership};
#[cfg(not(target_arch = "wasm32"))]
pub use stream_manifest::{BuildStreamManifestError, StreamManifestBuilder};

/// Data-shape errors used by generated binding adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainDataError {
    /// JSON input was not valid for the requested domain data shape.
    InvalidJson(String),
    /// A peer id string did not parse as a libp2p peer id.
    InvalidPeerId(String),
    /// A multiaddr string did not parse as a libp2p multiaddr.
    InvalidMultiaddr(String),
}

impl std::fmt::Display for DomainDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(message) => write!(f, "JSON is not valid: {message}"),
            Self::InvalidPeerId(value) => write!(f, "invalid peer id: {value}"),
            Self::InvalidMultiaddr(value) => write!(f, "invalid multiaddr: {value}"),
        }
    }
}

impl std::error::Error for DomainDataError {}

/// Build a JSON membership document for a new cluster.
pub fn cluster_membership_new_json(cluster_name: &str) -> String {
    membership_to_json(&ClusterMembership::new(cluster_name))
}

/// Return the `{cluster_name}.json` filename for a membership JSON document.
pub fn cluster_membership_filename_json(membership_json: &str) -> Result<String, DomainDataError> {
    Ok(parse_membership(membership_json)?.filename())
}

/// Return the peer count for a membership JSON document.
pub fn cluster_membership_peer_count_json(membership_json: &str) -> Result<u64, DomainDataError> {
    Ok(parse_membership(membership_json)?.peers.len() as u64)
}

/// Append a member JSON object to a membership JSON document and return the
/// updated membership JSON.
pub fn cluster_membership_admit_member_json(
    membership_json: &str,
    member_json: &str,
) -> Result<String, DomainDataError> {
    let mut membership = parse_membership(membership_json)?;
    let member = parse_member(member_json)?;
    membership.admit(member);
    Ok(membership_to_json(&membership))
}

/// Elect the deterministic successor from membership JSON, the local peer id,
/// and connected peer ids.
pub fn elect_successor_json(
    membership_json: &str,
    local_peer_id: &str,
    connected_peer_ids: Vec<String>,
) -> Result<Option<String>, DomainDataError> {
    let membership = parse_membership(membership_json)?;
    let local_peer_id = parse_peer_id(local_peer_id)?;
    let connected = connected_peer_ids
        .into_iter()
        .map(|value| parse_peer_id(&value))
        .collect::<Result<Vec<_>, _>>()?;

    let mut sorted = membership.peers.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| {
        a.join_ts_ns
            .cmp(&b.join_ts_ns)
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    for member in sorted {
        if member.peer_id == local_peer_id || connected.contains(&member.peer_id) {
            return Ok(Some(member.peer_id.to_string()));
        }
    }
    Ok(None)
}

/// Validate and normalize a cluster membership JSON document.
pub fn validate_membership_json(membership_json: &str) -> Result<String, DomainDataError> {
    let membership = parse_membership(membership_json)?;
    Ok(membership_to_json(&membership))
}

/// Return the elected successor for `peer_id` as a JSON string or `null`.
pub fn domain_successor_json(
    membership_json: &str,
    peer_id: &str,
) -> Result<String, DomainDataError> {
    let successor = elect_successor_json(membership_json, peer_id, vec![peer_id.to_string()])?;
    serde_json::to_string(&successor).map_err(|err| DomainDataError::InvalidJson(err.to_string()))
}

/// Validate and normalize a participant-info JSON document.
pub fn validate_participant_info_json(json: &str) -> Result<String, DomainDataError> {
    let value = parse_json_value(json)?;
    let object = value_object(&value, "participant info")?;
    require_string(object, "app")?;
    require_string(object, "name")?;
    require_string(object, "session_id")?;
    require_string(object, "session_clock_id")?;
    require_string(object, "session_clock_hash")?;
    require_u64_or_null(object, "session_now_ns", false)?;
    require_u64_or_null(object, "cluster_joined_at_ns", true)?;
    parse_peer_id(require_string(object, "peer_id")?)?;
    require_string(object, "app_instance")?;
    require_bool(object, "is_manager")?;
    let manager_peer_id = require_string(object, "manager_peer_id")?;
    if !manager_peer_id.is_empty() {
        parse_peer_id(manager_peer_id)?;
    }
    compact_json_value(&value)
}

/// Validate and normalize a sensor catalog JSON document.
pub fn validate_sensor_catalog_json(json: &str) -> Result<String, DomainDataError> {
    let value = parse_json_value(json)?;
    let sensors = catalog_array(&value, "sensors")?;
    for sensor in sensors {
        let object = value_object(sensor, "sensor catalog entry")?;
        require_string(object, "sensor_id")?;
        require_string(object, "sensor_hash")?;
        require_string(object, "kind")?;
        require_optional_json_string(object, "sensor_entry_json")?;
        require_optional_json_string(object, "frame_entry_json")?;
    }
    compact_json_value(&value)
}

/// Validate and normalize a resource catalog JSON document.
pub fn validate_resource_catalog_json(json: &str) -> Result<String, DomainDataError> {
    let value = parse_json_value(json)?;
    let resources = catalog_array(&value, "resources")?;
    for resource in resources {
        let object = value_object(resource, "resource catalog entry")?;
        let kind = require_string(object, "kind")?;
        require_string(object, "id")?;
        match kind {
            "sensor_stream" => {
                require_string(object, "sensor_id")?;
                require_string(object, "sensor_hash")?;
                require_string(object, "sensor_kind")?;
                require_string(object, "stream_protocol")?;
                require_string(object, "payload")?;
                require_optional_json_string(object, "sensor_entry_json")?;
                require_optional_json_string(object, "frame_entry_json")?;
            }
            "transform_edge" => {
                require_string(object, "from_frame_id")?;
                require_string(object, "from_frame_hash")?;
                require_string(object, "to_frame_id")?;
                require_string(object, "to_frame_hash")?;
                require_string(object, "writer_mode")?;
                value_object(require_field(object, "transform")?, "resource transform")?;
                require_optional_json_string(object, "from_frame_entry_json")?;
                require_optional_json_string(object, "to_frame_entry_json")?;
            }
            _ => {}
        }
    }
    compact_json_value(&value)
}

/// Validate and normalize a registry entry envelope or registry response JSON.
pub fn validate_registry_entry_json(json: &str) -> Result<String, DomainDataError> {
    let value = parse_json_value(json)?;
    let object = value_object(&value, "registry entry")?;
    if let Some(entry) = object.get("entry") {
        if !entry.is_null() {
            validate_registry_envelope(entry)?;
        }
    } else {
        validate_registry_envelope(&value)?;
    }
    compact_json_value(&value)
}

fn parse_membership(json: &str) -> Result<ClusterMembership, DomainDataError> {
    serde_json::from_str(json).map_err(|err| DomainDataError::InvalidJson(err.to_string()))
}

fn parse_member(json: &str) -> Result<ClusterMember, DomainDataError> {
    serde_json::from_str(json).map_err(|err| DomainDataError::InvalidJson(err.to_string()))
}

fn membership_to_json(membership: &ClusterMembership) -> String {
    serde_json::to_string(membership).expect("cluster membership serializes")
}

fn parse_json_value(json: &str) -> Result<Value, DomainDataError> {
    serde_json::from_str(json).map_err(|err| DomainDataError::InvalidJson(err.to_string()))
}

fn compact_json_value(value: &Value) -> Result<String, DomainDataError> {
    serde_json::to_string(value).map_err(|err| DomainDataError::InvalidJson(err.to_string()))
}

fn parse_peer_id(value: &str) -> Result<PeerId, DomainDataError> {
    PeerId::from_str(value).map_err(|_| DomainDataError::InvalidPeerId(value.to_string()))
}

fn value_object<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, DomainDataError> {
    value
        .as_object()
        .ok_or_else(|| DomainDataError::InvalidJson(format!("{label} must be an object")))
}

fn catalog_array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], DomainDataError> {
    if let Some(array) = value.as_array() {
        return Ok(array);
    }
    let object = value_object(value, field)?;
    object
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| DomainDataError::InvalidJson(format!("{field} must be an array")))
}

fn require_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a Value, DomainDataError> {
    object
        .get(field)
        .ok_or_else(|| DomainDataError::InvalidJson(format!("missing field `{field}`")))
}

fn require_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, DomainDataError> {
    require_field(object, field)?
        .as_str()
        .ok_or_else(|| DomainDataError::InvalidJson(format!("field `{field}` must be a string")))
}

fn require_bool(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<bool, DomainDataError> {
    require_field(object, field)?
        .as_bool()
        .ok_or_else(|| DomainDataError::InvalidJson(format!("field `{field}` must be a boolean")))
}

fn require_u64_or_null(
    object: &serde_json::Map<String, Value>,
    field: &str,
    allow_null: bool,
) -> Result<Option<u64>, DomainDataError> {
    let value = require_field(object, field)?;
    if allow_null && value.is_null() {
        return Ok(None);
    }
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| DomainDataError::InvalidJson(format!("field `{field}` must be a u64")))
}

fn require_optional_json_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<(), DomainDataError> {
    let Some(value) = object.get(field) else {
        return Ok(());
    };
    let Some(json) = value.as_str() else {
        return Err(DomainDataError::InvalidJson(format!(
            "field `{field}` must be a JSON string"
        )));
    };
    let _ = parse_json_value(json)?;
    Ok(())
}

fn validate_registry_envelope(value: &Value) -> Result<(), DomainDataError> {
    let object = value_object(value, "registry envelope")?;
    match require_string(object, "kind")? {
        "sensor" | "clock" | "frame" | "detector" => {}
        kind => {
            return Err(DomainDataError::InvalidJson(format!(
                "unsupported registry kind `{kind}`"
            )));
        }
    }
    require_string(object, "id")?;
    require_string(object, "hash")?;
    let canonical_json = require_string(object, "canonical_json")?;
    let _ = parse_json_value(canonical_json)?;
    Ok(())
}

/// Parse a multiaddr string for binding adapters.
pub fn parse_multiaddr(value: &str) -> Result<Multiaddr, DomainDataError> {
    value
        .parse()
        .map_err(|_| DomainDataError::InvalidMultiaddr(value.to_string()))
}
