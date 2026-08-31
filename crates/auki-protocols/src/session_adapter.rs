//! Native adapter from [`auki_session`] logs to the portable Catalog and
//! Stream endpoints.
//!
//! This module contains only mechanical projection and replay policy. The
//! endpoints continue to own authentication and bounded network I/O, while
//! applications remain responsible for deciding which authenticated peers may
//! see or subscribe to a session.

#![forbid(unsafe_code)]

use auki_datatypes::{detection::DetectionFrame, map::MapUpdate};
use auki_registry::SensorBody;
use auki_sdk::AuthenticatedPeer;
use auki_session::{
    DetectionLogHandle, HeadSpec, MapLogHandle, Peer, PeerRegistries, Session, SessionLogs,
};
use futures::{StreamExt, stream};

use crate::{
    catalog::{
        CatalogProvider,
        v2::{
            Available, DetectionManifestPointer, Head, PoseBlock, PoseManifestPointer,
            ResourceEntry as V2ResourceEntry, SensorBlock, SensorKind, SensorManifestPointer,
            TimeTransformManifestPointer, VariantContent,
        },
        v3, v4,
    },
    stream::{
        SourceStream, StreamDispatch, StreamItem, StreamProvider,
        v2::{DeclineReason, ReadFrom, StreamManifest, StreamRequest},
    },
};

/// Native provider that projects one [`Session`] onto Catalog v3/v4 and
/// Stream v2.
///
/// Construction requires the exact [`Peer`] instance that created the
/// session. Catalog snapshots are generated on demand, and stream requests
/// subscribe directly to the session's durable Map and Detection Logs.
#[derive(Clone)]
pub struct SessionProtocolProvider {
    logs: SessionLogs,
    registries: PeerRegistries,
}

impl SessionProtocolProvider {
    /// Bind the provider to a session owned by this exact peer instance.
    pub fn new(peer: &Peer, session: &Session) -> Result<Self, SessionProtocolProviderError> {
        if !peer.owns_session(session) {
            return Err(SessionProtocolProviderError::SessionPeerMismatch);
        }
        Ok(Self {
            logs: session.logs(),
            registries: peer.registries(),
        })
    }

    fn resource_rows(&self) -> Vec<v3::ResourceEntry> {
        let mut rows = Vec::new();
        rows.extend(self.logs.sensor_logs().iter().filter_map(|handle| {
            sensor_log_row(handle, &self.registries)
                .map(Box::new)
                .map(v3::ResourceEntry::V2)
        }));
        rows.extend(
            self.logs
                .pose_logs()
                .iter()
                .map(|handle| v3::ResourceEntry::V2(Box::new(pose_log_row(handle)))),
        );
        rows.extend(
            self.logs
                .time_logs()
                .iter()
                .map(|handle| v3::ResourceEntry::V2(Box::new(time_transform_log_row(handle)))),
        );
        rows.extend(
            self.logs
                .detection_logs()
                .iter()
                .map(|handle| v3::ResourceEntry::V2(Box::new(detection_log_row(handle)))),
        );
        rows
    }

    fn map_rows(&self) -> Vec<v4::MapLogResource> {
        self.logs
            .map_logs()
            .into_iter()
            .map(|handle| v4::MapLogResource {
                source_peer_id: handle.manifest.source_peer_id.clone(),
                writer_peer_id: handle.manifest.writer_peer_id.clone(),
                resource_id: handle.resource_id.clone(),
                map: handle.manifest.map.clone(),
                clock: handle.manifest.clock.clone(),
            })
            .collect()
    }

    fn dispatch_request(&self, request: StreamRequest) -> StreamDispatch {
        if let Some(handle) = self.logs.map_logs().into_iter().find(|handle| {
            handle.resource_id == request.resource_id
                && source_matches(&request.source_peer_id, &handle.manifest.source_peer_id)
        }) {
            return match map_log_source(&handle, request.from) {
                Ok(source) => StreamDispatch::AcceptMap {
                    manifest: map_stream_manifest(&handle),
                    source,
                },
                Err(detail) => StreamDispatch::Decline {
                    reason: DeclineReason::other(detail),
                },
            };
        }

        if let Some(handle) = self.logs.detection_logs().into_iter().find(|handle| {
            handle.resource_id == request.resource_id
                && source_matches(&request.source_peer_id, &handle.manifest.source_peer_id)
        }) {
            return match detection_log_source(&handle, request.from) {
                Ok(source) => StreamDispatch::AcceptDetection {
                    manifest: detection_stream_manifest(&handle),
                    source,
                },
                Err(detail) => StreamDispatch::Decline {
                    reason: DeclineReason::other(detail),
                },
            };
        }

        StreamDispatch::Decline {
            reason: DeclineReason::sensor_not_found(),
        }
    }
}

/// Failure to bind a Session adapter.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SessionProtocolProviderError {
    /// The session belongs to another independently-created `Peer`, even if
    /// that peer happens to use the same textual peer ID.
    #[error("the Session was not created by the supplied Peer")]
    SessionPeerMismatch,
}

impl CatalogProvider for SessionProtocolProvider {
    fn resources(
        &self,
        _requester: &AuthenticatedPeer,
        _request: &v3::ResourcesRequest,
    ) -> v3::ResourcesResponse {
        v3::ResourcesResponse {
            resources: self.resource_rows(),
        }
    }

    fn maps(&self, _requester: &AuthenticatedPeer) -> v4::ResourcesResponse {
        v4::ResourcesResponse {
            resources: self.map_rows(),
        }
    }
}

impl StreamProvider for SessionProtocolProvider {
    fn dispatch(&self, _requester: &AuthenticatedPeer, request: StreamRequest) -> StreamDispatch {
        self.dispatch_request(request)
    }
}

fn source_matches(requested: &str, actual: &str) -> bool {
    requested.is_empty() || requested == actual
}

fn map_log_source(
    handle: &MapLogHandle,
    from: ReadFrom,
) -> Result<SourceStream<MapUpdate>, String> {
    let (history, receiver) = match from {
        ReadFrom::Latest => (Vec::new(), handle.subscribe()),
        ReadFrom::FromStart | ReadFrom::FromTimestamp(_) => handle
            .snapshot_and_subscribe()
            .map_err(|error| error.to_string())?,
    };
    let history = history
        .into_iter()
        .filter(move |entry| timestamp_matches(from, entry.timestamp_ns))
        .map(|entry| {
            Ok(StreamItem {
                timestamp_ns: entry.timestamp_ns,
                payload: entry.payload,
            })
        });
    let live = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok((timestamp_ns, payload)) => Some((
                Ok(StreamItem {
                    timestamp_ns,
                    payload,
                }),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => Some((
                Err(format!("map log subscriber lagged by {count} updates")),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Ok(Box::pin(stream::iter(history).chain(live)))
}

fn detection_log_source(
    handle: &DetectionLogHandle,
    from: ReadFrom,
) -> Result<SourceStream<DetectionFrame>, String> {
    let (history, receiver) = match from {
        ReadFrom::Latest => (Vec::new(), handle.subscribe()),
        ReadFrom::FromStart | ReadFrom::FromTimestamp(_) => handle
            .snapshot_and_subscribe()
            .map_err(|error| error.to_string())?,
    };
    let history = history
        .into_iter()
        .filter(move |entry| timestamp_matches(from, entry.timestamp_ns))
        .map(|entry| {
            Ok(StreamItem {
                timestamp_ns: entry.timestamp_ns,
                payload: entry.payload,
            })
        });
    let live = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok((timestamp_ns, payload)) => Some((
                Ok(StreamItem {
                    timestamp_ns,
                    payload,
                }),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => Some((
                Err(format!(
                    "detection log subscriber lagged by {count} updates"
                )),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Ok(Box::pin(stream::iter(history).chain(live)))
}

fn timestamp_matches(from: ReadFrom, timestamp_ns: i64) -> bool {
    match from {
        ReadFrom::FromTimestamp(start) => timestamp_ns >= start,
        ReadFrom::Latest | ReadFrom::FromStart => true,
    }
}

fn map_stream_manifest(handle: &MapLogHandle) -> StreamManifest {
    StreamManifest {
        resource_id: handle.resource_id.clone(),
        payload: "map_update".into(),
        map_peer_id: handle.manifest.map.peer_id.clone(),
        map_id: handle.manifest.map.id.clone(),
        map_hash: handle.manifest.map.hash.clone(),
        clock_peer_id: handle.manifest.clock.peer_id.clone(),
        clock_id: handle.manifest.clock.id.clone(),
        clock_hash: handle.manifest.clock.hash.clone(),
        ..Default::default()
    }
}

fn detection_stream_manifest(handle: &DetectionLogHandle) -> StreamManifest {
    StreamManifest {
        resource_id: handle.resource_id.clone(),
        payload: "detection".into(),
        sensor_id: handle.manifest.input_sensor.id.clone(),
        sensor_hash: handle.manifest.input_sensor.hash.clone(),
        clock_peer_id: handle.manifest.clock.peer_id.clone(),
        clock_id: handle.manifest.clock.id.clone(),
        clock_hash: handle.manifest.clock.hash.clone(),
        ..Default::default()
    }
}

fn head_from_spec(spec: &HeadSpec) -> Head {
    match spec {
        HeadSpec::Rolling { retention_ns } => Head::Rolling {
            retention_ns: *retention_ns,
        },
        HeadSpec::Fixed => Head::Fixed { started_at_ns: 0 },
    }
}

fn sensor_kind_and_type(body: &SensorBody) -> (SensorKind, String) {
    match body {
        SensorBody::Camera(body) => (SensorKind::Camera, body.r#type.clone()),
        SensorBody::Rangefinder(body) => (SensorKind::Rangefinder, body.r#type.clone()),
        SensorBody::Rf(body) => (SensorKind::Rf, body.r#type.clone()),
        SensorBody::Audio(body) => (SensorKind::Audio, body.r#type.clone()),
        SensorBody::JointEncoders(body) => (SensorKind::JointEncoders, body.r#type.clone()),
        SensorBody::Scalar(body) => (SensorKind::Scalar, body.r#type.clone()),
    }
}

fn base_row(
    source_peer_id: String,
    writer_peer_id: String,
    resource_id: String,
    head: Head,
    sensor: Option<SensorBlock>,
    pose: Option<PoseBlock>,
    variant_content: VariantContent,
) -> V2ResourceEntry {
    V2ResourceEntry {
        source_peer_id,
        writer_peer_id,
        resource_id,
        state: "live".into(),
        head: Some(head),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor,
        pose,
        variant_content,
    }
}

fn sensor_log_row(
    handle: &auki_session::SensorLogHandle,
    registries: &PeerRegistries,
) -> Option<V2ResourceEntry> {
    let entry = registries.sensor(&handle.manifest.sensor.id)?;
    if entry.peer_id != handle.manifest.sensor.peer_id
        || entry.sensor_id != handle.manifest.sensor.id
        || entry.hash() != handle.manifest.sensor.hash
    {
        return None;
    }
    let (kind, r#type) = sensor_kind_and_type(&entry.body);
    Some(base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        Some(SensorBlock {
            kind,
            r#type,
            sensor_id: handle.manifest.sensor.id.clone(),
            sensor_hash: handle.manifest.sensor.hash.clone(),
        }),
        None,
        VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: handle.manifest.clock.clone(),
                frame: handle.manifest.frame.clone(),
            },
        },
    ))
}

fn pose_log_row(handle: &auki_session::PoseLogHandle) -> V2ResourceEntry {
    base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        None,
        Some(PoseBlock {
            writer_mode: handle.writer_mode,
        }),
        VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: handle.manifest.from_frame.clone(),
                to_frame: handle.manifest.to_frame.clone(),
                clock: handle.manifest.clock.clone(),
                source: handle.manifest.source.clone(),
                expected_rate_hz: handle.manifest.expected_rate_hz,
            },
        },
    )
}

fn time_transform_log_row(handle: &auki_session::TimeTransformLogHandle) -> V2ResourceEntry {
    base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        None,
        None,
        VariantContent::TimeTransformLog {
            manifest: TimeTransformManifestPointer {
                from_clock: handle.manifest.from_clock.clone(),
                to_clock: handle.manifest.to_clock.clone(),
                source: handle.manifest.source.clone(),
            },
        },
    )
}

fn detection_log_row(handle: &DetectionLogHandle) -> V2ResourceEntry {
    base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        None,
        None,
        VariantContent::DetectionLog {
            manifest: DetectionManifestPointer {
                instance_id: handle.manifest.instance_id.clone(),
                detector: handle.manifest.detector.clone(),
                input_log: handle.manifest.input_log.clone(),
                input_sensor: handle.manifest.input_sensor.clone(),
                clock: handle.manifest.clock.clone(),
                cadence: handle.manifest.cadence,
            },
        },
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use auki_manifests::DetectionCadence;
    use auki_registry::{
        DetectorBody, FiniteF64, LogRef, MapBody, ObjectDetection, Scalar, SensorBody, VoxelMap,
        VoxelValueModel,
    };
    use auki_session::{DetectionLogSpec, FrameDef, MapLogSpec, SensorLogSpec};
    use futures::StreamExt;
    use tempfile::tempdir;

    use super::*;

    fn peer_and_session(root: &std::path::Path) -> (Peer, Session) {
        let peer = Peer::new("galbot", "test-app").with_storage_root(root.to_path_buf());
        let session = peer.start_session().unwrap();
        (peer, session)
    }

    #[test]
    fn construction_requires_the_exact_peer_instance() {
        let tmp = tempdir().unwrap();
        let (peer, session) = peer_and_session(&tmp.path().join("owner"));
        let lookalike =
            Peer::new(peer.peer_id(), peer.app_id()).with_storage_root(tmp.path().join("other"));

        assert!(matches!(
            SessionProtocolProvider::new(&lookalike, &session),
            Err(SessionProtocolProviderError::SessionPeerMismatch)
        ));
        assert!(SessionProtocolProvider::new(&peer, &session).is_ok());
    }

    #[test]
    fn catalog_wraps_v2_sensor_rows_and_omits_stale_metadata() {
        let tmp = tempdir().unwrap();
        let (peer, session) = peer_and_session(tmp.path());
        let sensor = peer
            .register_sensor(
                "battery",
                SensorBody::Scalar(Scalar {
                    r#type: "battery_charge".into(),
                    unit: "percent".into(),
                    expected_rate_hz: 1,
                }),
            )
            .unwrap();
        session
            .register_sensor_log(SensorLogSpec {
                sensor: sensor.clone(),
                clock: session.monotonic_clock(),
                frame: None,
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();
        let provider = SessionProtocolProvider::new(&peer, &session).unwrap();

        let rows = provider.resource_rows();
        assert_eq!(rows.len(), 1);
        let v3::ResourceEntry::V2(row) = &rows[0] else {
            panic!("session sensor must be represented by a v2 row");
        };
        assert_eq!(row.resource_id, "battery");
        assert_eq!(
            row.sensor.as_ref().map(|sensor| sensor.kind),
            Some(SensorKind::Scalar)
        );
        assert_eq!(
            row.sensor.as_ref().map(|sensor| sensor.r#type.as_str()),
            Some("battery_charge")
        );

        peer.register_sensor(
            "battery",
            SensorBody::Scalar(Scalar {
                r#type: "battery_charge".into(),
                unit: "percent".into(),
                expected_rate_hz: 2,
            }),
        )
        .unwrap();
        assert!(provider.resource_rows().is_empty());
    }

    #[tokio::test]
    async fn map_and_detection_streams_replay_history_then_follow_live_appends() {
        let tmp = tempdir().unwrap();
        let (peer, session) = peer_and_session(tmp.path());
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map_ref = peer
            .register_map(
                "occupancy",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(0.05),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    color_model: None,
                    semantic_classes: Vec::new(),
                }),
            )
            .unwrap();
        let map = session
            .register_map_log(MapLogSpec {
                map: map_ref.clone(),
                clock: session.monotonic_clock(),
                head: HeadSpec::Fixed,
                segment_duration: Duration::from_secs(1),
                retention: Duration::ZERO,
            })
            .unwrap();
        map.append(10, &MapUpdate::default()).unwrap();

        let sensor = peer
            .register_sensor(
                "camera",
                SensorBody::Scalar(Scalar {
                    r#type: "test_input".into(),
                    unit: "value".into(),
                    expected_rate_hz: 1,
                }),
            )
            .unwrap();
        let detector = peer
            .register_detector(
                "detector",
                DetectorBody::ObjectDetection(ObjectDetection {
                    model: "test".into(),
                }),
                vec!["object".into()],
            )
            .unwrap();
        let detection = session
            .register_detection_log(DetectionLogSpec {
                instance_id: "detections".into(),
                detector,
                input_log: LogRef {
                    source_peer_id: peer.peer_id(),
                    resource_id: sensor.id.clone(),
                },
                input_sensor: sensor,
                clock: session.monotonic_clock(),
                cadence: DetectionCadence::EveryFrame,
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();
        detection.append(11, &DetectionFrame::default()).unwrap();

        let provider = SessionProtocolProvider::new(&peer, &session).unwrap();
        assert_eq!(provider.map_rows()[0].map, map_ref);

        let StreamDispatch::AcceptMap {
            manifest,
            mut source,
        } = provider.dispatch_request(StreamRequest {
            source_peer_id: peer.peer_id(),
            resource_id: "occupancy".into(),
            from: ReadFrom::FromStart,
        })
        else {
            panic!("map log should be served");
        };
        assert_eq!(manifest.payload, "map_update");
        assert_eq!(manifest.map_hash, map_ref.hash);
        map.append(20, &MapUpdate::default()).unwrap();
        assert_eq!(source.next().await.unwrap().unwrap().timestamp_ns, 10);
        assert_eq!(source.next().await.unwrap().unwrap().timestamp_ns, 20);

        let StreamDispatch::AcceptDetection {
            manifest,
            mut source,
        } = provider.dispatch_request(StreamRequest {
            source_peer_id: String::new(),
            resource_id: "detections".into(),
            from: ReadFrom::FromTimestamp(11),
        })
        else {
            panic!("detection log should be served");
        };
        assert_eq!(manifest.payload, "detection");
        detection.append(21, &DetectionFrame::default()).unwrap();
        assert_eq!(source.next().await.unwrap().unwrap().timestamp_ns, 11);
        assert_eq!(source.next().await.unwrap().unwrap().timestamp_ns, 21);

        assert!(matches!(
            provider.dispatch_request(StreamRequest {
                source_peer_id: "another-peer".into(),
                resource_id: "occupancy".into(),
                from: ReadFrom::Latest,
            }),
            StreamDispatch::Decline { .. }
        ));
    }
}
