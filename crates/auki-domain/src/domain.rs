//! `Domain` — the network face of a peer + session.
//!
//! A [`Domain`] composes a `&Peer` (eternal identity + registries) and a
//! `&Session` (one timeline's logs) from `auki-session` and gives them a
//! presence on a cluster: it bootstraps a [`ClusterManager`] and serves the
//! resource catalog (`Peer.registries` + `Session.logs`) to remote peers that
//! ask via `/auki/resources/*`.
//!
//! `auki-session` has no network dependencies; everything network-facing lives
//! here. See #274 (D3).

use std::sync::Arc;

use multiaddr::Multiaddr;

use auki_network::resources_protocol::{
    Available, DetectionManifestPointer, Head, PoseBlock, PoseManifestPointer, ResourceEntry,
    SensorBlock, SensorKind, SensorManifestPointer, TimeTransformManifestPointer, VariantContent,
};
use auki_network::stream_runtime::StreamProvider;
use auki_network::swarm::Behaviour;
use auki_network::{PeerIdentity, SessionHandle, Swarm};

use auki_registry::SensorBody;
use auki_session::{
    DetectionLogHandle, HeadSpec, Peer, PeerRegistries, PoseLogHandle, SensorLogHandle, Session,
    SessionLogs, TimeTransformLogHandle,
};

use crate::cluster_manager::{
    BootstrapError, ClusterManager, ClusterTarget, DaemonInfo, DiscoveryClientError,
};

// ─── DomainConfig ─────────────────────────────────────────────────────────────

/// Everything [`Domain::join`] needs that the peer / session don't own:
/// the cluster bootstrap policy, the local libp2p identity, the dialable
/// addresses, the Discovery service URL, the already-built swarm and stream
/// provider, and the daemon identity fields.
pub struct DomainConfig {
    /// Which cluster to create or join.
    pub target: ClusterTarget,
    /// The local libp2p identity (ed25519 keypair + derived `PeerId`).
    pub local_identity: PeerIdentity,
    /// Dialable multiaddrs to advertise in Discovery.
    pub local_multiaddrs: Vec<Multiaddr>,
    /// HTTP base URL of the Hagall Discovery service.
    pub discovery_url: String,
    /// Pre-built libp2p swarm for this peer.
    pub swarm: Swarm<Behaviour>,
    /// Provider for stream substream handling.
    pub stream_provider: StreamProvider,
    /// Static daemon identity fields (app, name, session_id, etc.).
    pub daemon_info: DaemonInfo,
}

// ─── DomainError ──────────────────────────────────────────────────────────────

/// Errors returned by [`Domain::join`] / [`Domain::leave`].
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// The peer's id is not the local network identity. The session's
    /// registered clock is keyed by `peer.peer_id()`, while the cluster's
    /// runtime clock is keyed by `local_identity.peer_id()`; if they differ
    /// the two clocks silently diverge. The peer must be constructed with the
    /// libp2p peer id as its `peer_id` (see the SDK identity convention).
    #[error("peer id {peer:?} != local network identity {identity:?}")]
    IdentityMismatch {
        /// The `peer.peer_id()` the session's clock is registered under.
        peer: String,
        /// The local libp2p identity the cluster's runtime clock would use.
        identity: String,
    },
    /// Cluster bootstrap failed (Discovery unreachable, name collision, join
    /// rejection, etc.).
    #[error("domain bootstrap: {0}")]
    Bootstrap(#[from] BootstrapError),
    /// Discovery deregistration failed while leaving (the local peer was the
    /// last Manager and the HTTP DELETE failed). The manager is dropped
    /// regardless.
    #[error("domain shutdown: {0}")]
    Shutdown(DiscoveryClientError),
}

// ─── Domain ────────────────────────────────────────────────────────────────────

/// Network presence for a peer + its current session.
pub struct Domain {
    manager: ClusterManager,
    catalog: Arc<DomainCatalog>,
}

impl Domain {
    /// Join or create a cluster as described by `config.target`, bootstrap the
    /// [`ClusterManager`], and wire it a [`SessionHandle`] so inbound
    /// `/auki/resources/*` requests return the catalog built from `peer`'s
    /// registries and `session`'s logs.
    ///
    /// Returns `Err(DomainError::Bootstrap(_))` if the cluster bootstrap fails.
    pub async fn join(
        peer: &Peer,
        session: &Session,
        mut config: DomainConfig,
    ) -> Result<Domain, DomainError> {
        // The peer's id must be the local network identity: the session's
        // registered clock is keyed by `peer.peer_id()`, and the cluster's
        // runtime `SessionClock` is keyed by `local_identity.peer_id()`. They
        // have to match or the registered and advertised clocks diverge.
        let local_id = config.local_identity.peer_id().to_string();
        if peer.peer_id() != local_id {
            return Err(DomainError::IdentityMismatch {
                peer: peer.peer_id(),
                identity: local_id,
            });
        }

        // Stamp the session's authoritative clock identity into DaemonInfo.
        // The cluster rebuilds a `SessionClock` from `daemon_info.session_id`
        // (+ the matching peer id), which reconstructs the identical
        // `ClockRegistryEntry` `start_session` registered — so the advertised
        // `(id, hash)` resolves to the registry entry. Replaces callers
        // hand-feeding these (and the `"compat"` placeholders).
        let mono = session.monotonic_clock();
        config.daemon_info.session_id = session.session_id();
        config.daemon_info.session_clock_id = mono.id;
        config.daemon_info.session_clock_hash = mono.hash;

        let manager = ClusterManager::bootstrap(
            config.target,
            config.local_identity,
            config.local_multiaddrs,
            config.discovery_url,
            config.swarm,
            config.stream_provider,
            config.daemon_info,
        )
        .await?;

        let catalog = Arc::new(DomainCatalog {
            logs: session.logs(),
            registries: peer.registries(),
        });
        let handle: Arc<dyn SessionHandle> = catalog.clone();
        manager.set_session_handle(handle);

        Ok(Domain { manager, catalog })
    }

    /// The catalog this domain currently serves: one row per registered log,
    /// in the canonical `/auki/resources/*` wire shape.
    pub fn catalog(&self) -> Vec<ResourceEntry> {
        self.catalog.catalog()
    }

    /// The active [`ClusterManager`].
    pub fn cluster_manager(&self) -> &ClusterManager {
        &self.manager
    }

    /// Shut down the cluster manager and leave the domain.
    ///
    /// Returns `Err(DomainError::Shutdown(_))` only if the local peer was the
    /// last Manager in Discovery and the HTTP DELETE failed; the manager is
    /// dropped regardless.
    pub async fn leave(self) -> Result<(), DomainError> {
        self.manager
            .shutdown()
            .await
            .map_err(DomainError::Shutdown)?;
        Ok(())
    }
}

/// Build the resource catalog for a peer + session without bootstrapping a
/// cluster. Used by tests to assert wire-equivalence; production serving goes
/// through [`Domain::join`]'s installed [`SessionHandle`].
pub fn catalog_of(peer: &Peer, session: &Session) -> Vec<ResourceEntry> {
    DomainCatalog {
        logs: session.logs(),
        registries: peer.registries(),
    }
    .catalog()
}

// ─── Catalog bridge ─────────────────────────────────────────────────────────

/// `SessionHandle` bridge: reads the session's live logs and the peer's
/// registries to build catalog rows on each inbound request.
struct DomainCatalog {
    logs: SessionLogs,
    registries: PeerRegistries,
}

impl DomainCatalog {
    fn catalog(&self) -> Vec<ResourceEntry> {
        let mut out = Vec::new();
        for handle in self.logs.sensor_logs() {
            out.push(sensor_log_row(&handle, &self.registries));
        }
        for handle in self.logs.pose_logs() {
            out.push(pose_log_row(&handle));
        }
        for handle in self.logs.time_logs() {
            out.push(time_transform_row(&handle));
        }
        for handle in self.logs.detection_logs() {
            out.push(detection_log_row(&handle));
        }
        out
    }
}

impl SessionHandle for DomainCatalog {
    fn catalog(&self) -> Vec<ResourceEntry> {
        DomainCatalog::catalog(self)
    }
}

// ─── Row builders ─────────────────────────────────────────────────────────────

fn head_from_spec(spec: &HeadSpec) -> Option<Head> {
    match spec {
        HeadSpec::Rolling { retention_ns } => Some(Head::Rolling {
            retention_ns: *retention_ns,
        }),
        HeadSpec::Fixed => Some(Head::Fixed { started_at_ns: 0 }), // stub; real timestamp when backing Log<T> is wired
    }
}

fn sensor_kind_and_type(body: &SensorBody) -> (SensorKind, String) {
    match body {
        SensorBody::Camera(b) => (SensorKind::Camera, b.r#type.clone()),
        SensorBody::Rangefinder(b) => (SensorKind::Rangefinder, b.r#type.clone()),
        SensorBody::Rf(b) => (SensorKind::Rf, b.r#type.clone()),
        SensorBody::Audio(b) => (SensorKind::Audio, b.r#type.clone()),
        SensorBody::JointEncoders(b) => (SensorKind::JointEncoders, b.r#type.clone()),
    }
}

fn sensor_log_row(handle: &SensorLogHandle, registries: &PeerRegistries) -> ResourceEntry {
    // Kind + type come from the peer's sensor registry (eternal; the log's
    // registration guarantees the sensor is present). Default only guards the
    // unreachable missing-entry case so the catalog handler never panics.
    let (kind, sensor_type) = registries
        .sensor(&handle.manifest.sensor.id)
        .map(|entry| sensor_kind_and_type(&entry.body))
        .unwrap_or((SensorKind::Camera, String::new()));

    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: Some(SensorBlock {
            kind,
            r#type: sensor_type,
            sensor_id: handle.manifest.sensor.id.clone(),
            sensor_hash: handle.manifest.sensor.hash.clone(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: handle.manifest.clock.clone(),
                frame: handle.manifest.frame.clone(),
            },
        },
    }
}

fn pose_log_row(handle: &PoseLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: None,
        pose: Some(PoseBlock {
            writer_mode: handle.writer_mode.clone(),
        }),
        variant_content: VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: handle.manifest.from_frame.clone(),
                to_frame: handle.manifest.to_frame.clone(),
                clock: handle.manifest.clock.clone(),
                source: handle.manifest.source.clone(),
                expected_rate_hz: handle.manifest.expected_rate_hz,
            },
        },
    }
}

fn time_transform_row(handle: &TimeTransformLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: None,
        pose: None,
        variant_content: VariantContent::TimeTransformLog {
            manifest: TimeTransformManifestPointer {
                from_clock: handle.manifest.from_clock.clone(),
                to_clock: handle.manifest.to_clock.clone(),
                source: handle.manifest.source.clone(),
            },
        },
    }
}

fn detection_log_row(handle: &DetectionLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: None,
        pose: None,
        variant_content: VariantContent::DetectionLog {
            manifest: DetectionManifestPointer {
                detector: handle.manifest.detector.clone(),
                input_log: handle.manifest.input_log.clone(),
                input_sensor: handle.manifest.input_sensor.clone(),
                clock: handle.manifest.clock.clone(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_registry::{Camera, ClockBody, ClockMeta, Scope, SensorBody};
    use auki_session::{FrameDef, HeadSpec, Peer, SensorLogSpec};
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn catalog_of_returns_one_wire_row_per_sensor_log() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let frame = peer
            .register_frame("head_left_camera_optical", FrameDef::ros_optical())
            .unwrap();
        let sensor = peer
            .register_sensor(
                "head_left_rgb",
                SensorBody::Camera(Camera {
                    r#type: "rgb".to_string(),
                    width: 1920,
                    height: 1200,
                    frame_rate_hz: 30,
                    pixel_format: "rgb8".to_string(),
                    color_space: "srgb".to_string(),
                    intrinsics_model: "pinhole".to_string(),
                    distortion_model: "brown_conrady".to_string(),
                    frame: frame.clone(),
                }),
            )
            .unwrap();
        let session = peer.start_session().unwrap();
        let clock = session
            .register_clock(
                "session/sdk_clock",
                ClockBody::MonotonicClock(ClockMeta {
                    unit: "ns".to_string(),
                    monotonic: true,
                    epoch: None,
                    scope: Scope::DeviceLocal,
                }),
            )
            .unwrap();
        session
            .register_sensor_log(SensorLogSpec {
                sensor,
                clock,
                frame: Some(frame),
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();

        let rows = catalog_of(&peer, &session);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.source_peer_id, "galbot");
        assert_eq!(row.writer_peer_id, "galbot");
        assert_eq!(row.resource_id, "head_left_rgb");
        assert_eq!(row.state, "live");
        assert!(matches!(
            row.head,
            Some(Head::Rolling {
                retention_ns: 5_000_000_000
            })
        ));
        // Kind + type were derived from the peer's sensor registry.
        let sensor_block = row.sensor.as_ref().unwrap();
        assert_eq!(sensor_block.kind, SensorKind::Camera);
        assert_eq!(sensor_block.r#type, "rgb");
        assert_eq!(sensor_block.sensor_id, "head_left_rgb");
        assert!(row.pose.is_none());
        assert!(matches!(
            row.variant_content,
            VariantContent::SensorLog { .. }
        ));
    }

    #[test]
    fn catalog_of_is_empty_for_a_session_with_no_logs() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("park", "vis").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        assert!(catalog_of(&peer, &session).is_empty());
    }
}
