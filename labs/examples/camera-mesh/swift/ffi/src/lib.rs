//! Thin Swift bridge over the Rust-owned Camera Mesh application policy.
//!
//! AVFoundation supplies bounded JPEGs. Exact-viewer approval, protocol
//! serving, controls, snapshots, and ordered endpoint cleanup remain in the
//! shared Rust Camera Mesh implementation used by native peers.

use std::sync::Arc;

use auki_camera_mesh::{
    CameraEvent, CameraProtocols, CameraQuality, CameraRole, PeerRoutes, camera_profile,
    protocol_ids_for_role,
};
use auki_sdk_binding::{AukiPeer, CleanupResult, DetachedCleanup, wait_cleanup};
use parking_lot::Mutex;
use tokio::sync::{Mutex as AsyncMutex, mpsc, watch};
use uuid::Uuid;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AukiCameraMeshError {
    #[error("{message}")]
    Operation { message: String },
}

fn operation_error(context: &'static str, error: impl std::fmt::Display) -> AukiCameraMeshError {
    AukiCameraMeshError::Operation {
        message: format!("{context}: {error}"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiCameraQuality {
    Low,
    Medium,
    High,
}

impl From<AukiCameraQuality> for CameraQuality {
    fn from(value: AukiCameraQuality) -> Self {
        match value {
            AukiCameraQuality::Low => Self::Low,
            AukiCameraQuality::Medium => Self::Medium,
            AukiCameraQuality::High => Self::High,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiCameraRenditionFrames {
    pub low: Vec<u8>,
    pub medium: Vec<u8>,
    pub high: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiCameraPublisherEventKind {
    ApprovalRequired,
    ControlReceived,
    SnapshotStaged,
    RuntimeError,
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiCameraPublisherEvent {
    pub kind: AukiCameraPublisherEventKind,
    pub peer_id: Option<String>,
    pub control: Option<String>,
    pub request_id: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
    pub error: Option<String>,
}

impl From<CameraEvent> for AukiCameraPublisherEvent {
    fn from(event: CameraEvent) -> Self {
        let mut value = Self {
            kind: AukiCameraPublisherEventKind::RuntimeError,
            peer_id: None,
            control: None,
            request_id: None,
            sha256: None,
            size: None,
            error: None,
        };
        match event {
            CameraEvent::ApprovalRequired { peer_id } => {
                value.kind = AukiCameraPublisherEventKind::ApprovalRequired;
                value.peer_id = Some(peer_id);
            }
            CameraEvent::ControlReceived { control, peer_id } => {
                value.kind = AukiCameraPublisherEventKind::ControlReceived;
                value.peer_id = Some(peer_id);
                value.control = Some(control);
            }
            CameraEvent::SnapshotStaged {
                request_id,
                peer_id,
                sha256,
                size,
            } => {
                value.kind = AukiCameraPublisherEventKind::SnapshotStaged;
                value.peer_id = Some(peer_id);
                value.request_id = Some(request_id);
                value.sha256 = Some(sha256);
                value.size = Some(size as u64);
            }
            CameraEvent::RuntimeError { error } => {
                value.error = Some(error);
            }
        }
        value
    }
}

struct PublisherOwner {
    protocols: Mutex<Option<CameraProtocols>>,
    peer: Arc<AukiPeer>,
    cleanup: DetachedCleanup,
}

impl PublisherOwner {
    fn new(protocols: CameraProtocols, peer: Arc<AukiPeer>) -> Self {
        Self {
            protocols: Mutex::new(Some(protocols)),
            peer,
            cleanup: DetachedCleanup::new(),
        }
    }

    fn with_protocols<T>(
        &self,
        operation: &'static str,
        use_protocols: impl FnOnce(&CameraProtocols) -> Result<T, AukiCameraMeshError>,
    ) -> Result<T, AukiCameraMeshError> {
        let protocols = self.protocols.lock();
        let protocols = protocols
            .as_ref()
            .ok_or_else(|| operation_error(operation, "publisher is stopped"))?;
        use_protocols(protocols)
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let protocols = self.protocols.lock().take();
            let peer = Arc::clone(&self.peer);
            async move {
                let mut failures = Vec::new();
                if let Some(protocols) = protocols
                    && let Err(error) = protocols.close().await
                {
                    failures.push(format!("Camera Mesh protocols: {error:#}"));
                }
                if let Err(error) = peer.shutdown().await {
                    failures.push(format!("Auki peer: {error}"));
                }
                if failures.is_empty() {
                    Ok(())
                } else {
                    Err(failures.join("; "))
                }
            }
        })
    }
}

impl Drop for PublisherOwner {
    fn drop(&mut self) {
        if self.protocols.get_mut().is_some() {
            let _ = self.begin_close();
        }
    }
}

/// Foreground Camera Mesh publisher backed by the shared Rust application.
#[derive(uniffi::Object)]
pub struct AukiCameraPublisher {
    owner: PublisherOwner,
    events: AsyncMutex<mpsc::Receiver<CameraEvent>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiCameraPublisher {
    #[uniffi::constructor]
    pub async fn mount(
        peer: Arc<AukiPeer>,
        display_name: String,
        initial_frames: AukiCameraRenditionFrames,
    ) -> Result<Arc<Self>, AukiCameraMeshError> {
        let peer_id = peer
            .peer_id()
            .parse()
            .map_err(|error| operation_error("parse local Camera Mesh Peer ID", error))?;
        let domain_id = Uuid::parse_str(&peer.domain_id())
            .map_err(|error| operation_error("parse local Camera Mesh Domain ID", error))?;
        let routes = peer
            .routes()
            .map_err(|error| operation_error("read Camera Mesh relay routes", error))?;
        let (protocols, events) = CameraProtocols::mount_renditions_context(
            peer.rust_protocols(),
            peer_id,
            domain_id,
            PeerRoutes {
                tcp: routes.tcp,
                wss: routes.wss,
            },
            CameraRole::Publisher,
            display_name,
            "swift",
            vec![
                (camera_profile(CameraQuality::Low), initial_frames.low),
                (camera_profile(CameraQuality::Medium), initial_frames.medium),
                (camera_profile(CameraQuality::High), initial_frames.high),
            ],
        )
        .await
        .map_err(|error| operation_error("mount Camera Mesh publisher", error))?;
        Ok(Arc::new(Self {
            owner: PublisherOwner::new(protocols, peer),
            events: AsyncMutex::new(events),
        }))
    }

    pub fn peer_id(&self) -> Result<String, AukiCameraMeshError> {
        self.owner
            .with_protocols("read Camera Mesh publisher Peer ID", |protocols| {
                Ok(protocols.card().peer_id.clone())
            })
    }

    pub fn card_json(&self) -> Result<String, AukiCameraMeshError> {
        self.owner
            .with_protocols("build Camera Mesh publisher card", |protocols| {
                serde_json::to_string(protocols.card())
                    .map_err(|error| operation_error("encode Camera Mesh publisher card", error))
            })
    }

    pub fn protocols(&self) -> Vec<String> {
        protocol_ids_for_role(CameraRole::Publisher)
    }

    /// Atomically replace one latest rendition. Rust retains no frame backlog.
    pub fn update_frame(
        &self,
        quality: AukiCameraQuality,
        jpeg: Vec<u8>,
    ) -> Result<(), AukiCameraMeshError> {
        self.owner
            .with_protocols("update Camera Mesh frame", |protocols| {
                protocols
                    .replace_rendition_frame(quality.into(), jpeg)
                    .map_err(|error| operation_error("validate Camera Mesh frame", error))
            })
    }

    pub fn approve(&self, peer_id: String) -> Result<(), AukiCameraMeshError> {
        let peer_id = peer_id
            .parse()
            .map_err(|error| operation_error("parse approved Camera Mesh Peer ID", error))?;
        self.owner
            .with_protocols("approve Camera Mesh viewer", |protocols| {
                protocols.approve(peer_id);
                Ok(())
            })
    }

    pub fn revoke(&self, peer_id: String) -> Result<(), AukiCameraMeshError> {
        let peer_id = peer_id
            .parse()
            .map_err(|error| operation_error("parse revoked Camera Mesh Peer ID", error))?;
        self.owner
            .with_protocols("revoke Camera Mesh viewer", |protocols| {
                protocols.revoke(peer_id);
                Ok(())
            })
    }

    pub fn pending_approvals(&self) -> Result<Vec<String>, AukiCameraMeshError> {
        self.owner
            .with_protocols("list pending Camera Mesh viewers", |protocols| {
                Ok(protocols
                    .pending_approvals()
                    .into_iter()
                    .map(|peer_id| peer_id.to_string())
                    .collect())
            })
    }

    pub fn paused(&self) -> Result<bool, AukiCameraMeshError> {
        self.owner
            .with_protocols("read Camera Mesh pause state", |protocols| {
                Ok(protocols.paused())
            })
    }

    /// Receive one bounded publisher event, or `nil` after shutdown.
    pub async fn next_event(
        &self,
    ) -> Result<Option<AukiCameraPublisherEvent>, AukiCameraMeshError> {
        Ok(self.events.lock().await.recv().await.map(Into::into))
    }

    /// Close Camera Mesh endpoints before releasing relay and peer resources.
    pub async fn close(&self) -> Result<(), AukiCameraMeshError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Camera Mesh publisher", error))
    }
}

impl Drop for AukiCameraPublisher {
    fn drop(&mut self) {
        let _ = self.owner.begin_close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_protocols_are_the_locked_camera_mesh_set() {
        assert_eq!(
            protocol_ids_for_role(CameraRole::Publisher),
            [
                "/auki/auth/1/info/1.0.0",
                "/auki/auth/1/resources/0.3.0",
                "/auki/auth/1/resources/0.4.0",
                "/auki/auth/1/registries/0.3.0",
                "/auki/auth/1/blobs/0.1.0",
                "/auki/auth/1/message/0.1.0",
                "/auki/auth/1/stream/0.2.0",
            ]
        );
    }

    #[test]
    fn publisher_event_mapping_is_lossless() {
        let mapped = AukiCameraPublisherEvent::from(CameraEvent::SnapshotStaged {
            request_id: "snapshot-1".into(),
            peer_id: "peer".into(),
            sha256: "hash".into(),
            size: 42,
        });
        assert_eq!(mapped.kind, AukiCameraPublisherEventKind::SnapshotStaged);
        assert_eq!(mapped.request_id.as_deref(), Some("snapshot-1"));
        assert_eq!(mapped.peer_id.as_deref(), Some("peer"));
        assert_eq!(mapped.sha256.as_deref(), Some("hash"));
        assert_eq!(mapped.size, Some(42));
    }

    #[test]
    fn swift_quality_tiers_map_to_shared_camera_mesh_profiles() {
        assert_eq!(
            CameraQuality::from(AukiCameraQuality::Low),
            CameraQuality::Low
        );
        assert_eq!(
            CameraQuality::from(AukiCameraQuality::Medium),
            CameraQuality::Medium
        );
        assert_eq!(
            CameraQuality::from(AukiCameraQuality::High),
            CameraQuality::High
        );
    }
}
