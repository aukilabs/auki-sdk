//! Swift adapter for the finite authenticated Info v1 protocol.

use std::sync::Arc;

use auki_protocols::info::{
    InfoClient, InfoEndpoint, InfoProvider,
    v1::{AuthenticatedParticipantInfo, ID, MAX_INFO_FRAME_BYTES},
};
use auki_sdk_rs::AuthenticatedPeer;
use parking_lot::RwLock;

use crate::{AukiPeer, AukiPeerTarget, AukiSdkError, operation_error, wait_cleanup};

use super::finite_support::{
    CloseFuture, EndpointOwner, bounded_json, exact_target, parse_bounded_json,
};

// Leave room for the protobuf response envelope around the JSON document.
const MAX_INFO_JSON_BYTES: usize = MAX_INFO_FRAME_BYTES as usize - 1024;

#[derive(Clone, Default)]
struct InfoSnapshot {
    value: Arc<RwLock<Option<AuthenticatedParticipantInfo>>>,
}

impl InfoSnapshot {
    fn replace(&self, value: Option<AuthenticatedParticipantInfo>) {
        *self.value.write() = value;
    }

    fn get(&self) -> Option<AuthenticatedParticipantInfo> {
        self.value.read().clone()
    }
}

impl InfoProvider for InfoSnapshot {
    fn participant_info(
        &self,
        _requester: &AuthenticatedPeer,
    ) -> Option<AuthenticatedParticipantInfo> {
        self.get()
    }
}

fn read_snapshot(
    json: &str,
    expected_peer_id: &str,
) -> Result<AuthenticatedParticipantInfo, AukiSdkError> {
    let info: AuthenticatedParticipantInfo =
        parse_bounded_json("read Info snapshot JSON", json, MAX_INFO_JSON_BYTES)?;
    if info.peer_id.to_string() != expected_peer_id {
        return Err(operation_error(
            "validate Info snapshot",
            format!(
                "participant Peer ID {} does not match local peer {expected_peer_id}",
                info.peer_id
            ),
        ));
    }
    Ok(info)
}

fn write_snapshot(info: &AuthenticatedParticipantInfo) -> Result<String, AukiSdkError> {
    bounded_json("write Info snapshot JSON", info, MAX_INFO_JSON_BYTES)
}

fn close_endpoint(endpoint: InfoEndpoint) -> CloseFuture {
    Box::pin(async move { endpoint.close().await.map_err(|error| error.to_string()) })
}

/// Outbound Info v1 client over one running native Auki peer.
#[derive(uniffi::Object)]
pub struct AukiInfoClient {
    inner: InfoClient,
    domain_id: String,
}

impl AukiInfoClient {
    fn from_inner(inner: InfoClient, domain_id: String) -> Arc<Self> {
        Arc::new(Self { inner, domain_id })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiInfoClient {
    #[uniffi::constructor]
    pub fn new(peer: Arc<AukiPeer>) -> Arc<Self> {
        Self::from_inner(InfoClient::new(peer.rust_protocols()), peer.domain_id())
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    /// Fetch one Rust-validated participant snapshot as compact canonical-shape JSON.
    pub async fn fetch_exact(&self, target: AukiPeerTarget) -> Result<String, AukiSdkError> {
        let (peer_id, route) = exact_target(&self.domain_id, target)?;
        let info = self
            .inner
            .fetch_exact(peer_id, route)
            .await
            .map_err(|error| operation_error("fetch Info snapshot", error))?;
        write_snapshot(&info)
    }
}

/// Mounted Info service backed by one replaceable in-memory snapshot.
#[derive(uniffi::Object)]
pub struct AukiInfoEndpoint {
    owner: EndpointOwner<InfoEndpoint>,
    snapshot: InfoSnapshot,
    client: Arc<AukiInfoClient>,
    peer_id: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiInfoEndpoint {
    /// Mount Info with no advertised snapshot. Call `replace_json` to opt in.
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiSdkError> {
        let snapshot = InfoSnapshot::default();
        let endpoint = InfoEndpoint::mount(peer.rust_protocols(), snapshot.clone())
            .map_err(|error| operation_error("mount Info endpoint", error))?;
        let client = AukiInfoClient::from_inner(endpoint.client(), peer.domain_id());
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

    pub fn client(&self) -> Arc<AukiInfoClient> {
        Arc::clone(&self.client)
    }

    /// Atomically replace the served snapshot, or pass `nil` to decline requests.
    pub fn replace_json(&self, json: Option<String>) -> Result<(), AukiSdkError> {
        self.owner.ensure_open("replace Info snapshot")?;
        let snapshot = json
            .as_deref()
            .map(|json| read_snapshot(json, &self.peer_id))
            .transpose()?;
        self.snapshot.replace(snapshot);
        Ok(())
    }

    pub fn snapshot_json(&self) -> Result<Option<String>, AukiSdkError> {
        self.snapshot.get().as_ref().map(write_snapshot).transpose()
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Info endpoint", error))
    }
}

#[cfg(test)]
mod tests {
    use auki_sdk_rs::Identity;

    use super::*;

    fn snapshot(peer_id: &str) -> String {
        format!(
            r#"{{"app":"playground","app_version":"0.1.0","name":"swift","session_id":"session","session_clock_id":"clock","session_clock_hash":"hash","session_now_ns":7,"peer_id":"{peer_id}","app_instance":"ios"}}"#
        )
    }

    #[test]
    fn info_snapshot_is_strict_bounded_and_owned_by_the_local_peer() {
        let peer_id = Identity::generate().peer_id().to_string();
        let parsed = read_snapshot(&snapshot(&peer_id), &peer_id).unwrap();
        assert_eq!(parsed.peer_id.to_string(), peer_id);
        assert_eq!(
            read_snapshot(&write_snapshot(&parsed).unwrap(), &peer_id).unwrap(),
            parsed
        );

        let other = Identity::generate().peer_id().to_string();
        assert!(read_snapshot(&snapshot(&other), &peer_id).is_err());
        assert!(read_snapshot(r#"{"app":"missing fields"}"#, &peer_id).is_err());
    }
}
