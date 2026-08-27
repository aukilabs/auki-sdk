//! Peer-agnostic SDK catalog selection for the voxel Mapper.

use auki_datatypes::{point_cloud::Data as PointCloudData, pose::SpatialTransform};
use auki_domain::StreamSubscription;
use auki_protocols::catalog::v2::{ResourceEntry, SensorKind, VariantContent};
use auki_protocols::stream::v2::{ReadFrom, StreamRequest};
use auki_registry::{LogRef, Rangefinder, RegistryRef, VoxelMap};

use crate::{
    MapUpdateSink, MapperInput, MapperInputBindingError, PoseAlignmentConfig, ValidatedFrameAlias,
    VoxelMapperMapFrameBinding, VoxelMapperRunError, VoxelMapperRunReport, VoxelMapperRunner,
    VoxelPersistenceConfig,
};

/// Runtime tuning for the SDK-composed voxel Mapper service.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelMapperServiceConfig {
    /// Additive free-space evidence written along each sensor ray.
    pub free_delta: f32,
    /// Additive occupied evidence written at each ray endpoint.
    pub occupied_delta: f32,
    /// Optional time-based reliability filter for the persistent map.
    pub persistence: Option<VoxelPersistenceConfig>,
    /// Pose-alignment buffer behavior.
    pub alignment: PoseAlignmentConfig,
}

impl Default for VoxelMapperServiceConfig {
    fn default() -> Self {
        Self {
            free_delta: -0.2,
            occupied_delta: 0.8,
            persistence: Some(VoxelPersistenceConfig::default()),
            alignment: PoseAlignmentConfig::default(),
        }
    }
}

/// Optional exact source-log constraints for automatic Mapper discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoxelMapperSourceQuery {
    /// Restrict discovery to one point-cloud source log.
    pub point_cloud: Option<LogRef>,
    /// Restrict discovery to one pose source log.
    pub pose: Option<LogRef>,
}

/// A unique compatible point-cloud/pose pair selected from SDK catalogs.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelMapperSources {
    /// Live point-cloud catalog row, including its independently selectable
    /// writer peer.
    pub point_cloud: ResourceEntry,
    /// Live pose catalog row, including its independently selectable writer
    /// peer.
    pub pose: ResourceEntry,
    /// Exact clock shared by both input logs.
    pub clock: RegistryRef,
    /// Exact point-cloud sensor frame and pose source frame.
    pub sensor_frame: RegistryRef,
    /// Exact destination frame declared by the selected pose log.
    pub pose_frame: RegistryRef,
    /// Exact frame owned by the selected Map.
    pub map_frame: RegistryRef,
    /// Explicit identity-only rebind when `pose_frame` and `map_frame` have
    /// independent owners.
    pub frame_alias: Option<ValidatedFrameAlias>,
}

impl VoxelMapperSources {
    /// Select the unique compatible live input pair from a merged set of SDK
    /// catalog rows. Rows may come from any number of peers.
    pub fn select(
        resources: &[ResourceEntry],
        map: &VoxelMap,
        query: &VoxelMapperSourceQuery,
    ) -> Result<Self, VoxelMapperSourceSelectionError> {
        Self::select_with_frame_alias(resources, map, query, None)
    }

    /// Select inputs while allowing a validated identity-only alias from a
    /// pose destination frame to the independently owned Map frame.
    pub fn select_with_frame_alias(
        resources: &[ResourceEntry],
        map: &VoxelMap,
        query: &VoxelMapperSourceQuery,
        frame_alias: Option<&ValidatedFrameAlias>,
    ) -> Result<Self, VoxelMapperSourceSelectionError> {
        if frame_alias.is_some_and(|alias| alias.target() != &map.frame) {
            return Err(VoxelMapperSourceSelectionError::AliasDoesNotTargetMap);
        }
        let point_clouds = resources.iter().filter_map(|row| {
            if row.state != "live" || !matches_log(row, query.point_cloud.as_ref()) {
                return None;
            }
            let VariantContent::SensorLog { manifest } = &row.variant_content else {
                return None;
            };
            let sensor = row.sensor.as_ref()?;
            if sensor.kind != SensorKind::Rangefinder || sensor.r#type != "point_cloud" {
                return None;
            }
            Some((row, manifest))
        });
        let poses: Vec<_> = resources
            .iter()
            .filter_map(|row| {
                if row.state != "live" || !matches_log(row, query.pose.as_ref()) {
                    return None;
                }
                let VariantContent::PoseLog { manifest } = &row.variant_content else {
                    return None;
                };
                row.pose.as_ref()?;
                Some((row, manifest))
            })
            .collect();

        let mut compatible = Vec::new();
        for (point_row, point_manifest) in point_clouds {
            let Some(sensor_frame) = point_manifest.frame.as_ref() else {
                continue;
            };
            for (pose_row, pose_manifest) in &poses {
                if point_manifest.clock == pose_manifest.clock
                    && *sensor_frame == pose_manifest.from_frame
                    && (pose_manifest.to_frame == map.frame
                        || frame_alias.is_some_and(|alias| {
                            alias.source() == &pose_manifest.to_frame
                                && alias.target() == &map.frame
                        }))
                {
                    let uses_alias = pose_manifest.to_frame != map.frame;
                    compatible.push(Self {
                        point_cloud: point_row.clone(),
                        pose: (*pose_row).clone(),
                        clock: point_manifest.clock.clone(),
                        sensor_frame: sensor_frame.clone(),
                        pose_frame: pose_manifest.to_frame.clone(),
                        map_frame: map.frame.clone(),
                        frame_alias: uses_alias.then(|| {
                            frame_alias
                                .expect("compatible aliased frame requires an alias")
                                .clone()
                        }),
                    });
                }
            }
        }

        match compatible.len() {
            0 => Err(VoxelMapperSourceSelectionError::NoCompatiblePair),
            1 => Ok(compatible.pop().expect("length checked above")),
            count => Err(VoxelMapperSourceSelectionError::Ambiguous { count }),
        }
    }

    /// Canonical point-cloud log identity used in the stream-open request.
    pub fn point_cloud_log_ref(&self) -> LogRef {
        log_ref(&self.point_cloud)
    }

    /// Canonical pose log identity used in the stream-open request.
    pub fn pose_log_ref(&self) -> LogRef {
        log_ref(&self.pose)
    }

    /// SDK request for opening the selected point-cloud log at its chosen
    /// writer peer.
    pub fn point_cloud_request(&self, from: ReadFrom) -> StreamRequest {
        stream_request(&self.point_cloud, from)
    }

    /// SDK request for opening the selected pose log at its chosen writer
    /// peer.
    pub fn pose_request(&self, from: ReadFrom) -> StreamRequest {
        stream_request(&self.pose, from)
    }

    /// Validate and bind both opened SDK subscriptions into runner inputs.
    pub fn bind_inputs(
        &self,
        point_cloud: StreamSubscription<PointCloudData>,
        pose: StreamSubscription<SpatialTransform>,
    ) -> Result<
        (MapperInput<PointCloudData>, MapperInput<SpatialTransform>),
        VoxelMapperInputBindingError,
    > {
        let point_cloud = MapperInput::from_sdk_subscription(
            self.point_cloud_log_ref(),
            self.clock.clone(),
            point_cloud,
        )
        .map_err(VoxelMapperInputBindingError::PointCloud)?;
        let pose =
            MapperInput::from_sdk_subscription(self.pose_log_ref(), self.clock.clone(), pose)
                .map_err(VoxelMapperInputBindingError::Pose)?;
        Ok((point_cloud, pose))
    }
}

fn matches_log(row: &ResourceEntry, selected: Option<&LogRef>) -> bool {
    selected.is_none_or(|selected| {
        row.source_peer_id == selected.source_peer_id && row.resource_id == selected.resource_id
    })
}

fn log_ref(row: &ResourceEntry) -> LogRef {
    LogRef {
        source_peer_id: row.source_peer_id.clone(),
        resource_id: row.resource_id.clone(),
    }
}

fn stream_request(row: &ResourceEntry, from: ReadFrom) -> StreamRequest {
    StreamRequest {
        source_peer_id: row.source_peer_id.clone(),
        resource_id: row.resource_id.clone(),
        from,
    }
}

/// One of the selected streams did not accept its discovered identity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VoxelMapperInputBindingError {
    /// Point-cloud stream manifest mismatch.
    #[error("point-cloud input: {0}")]
    PointCloud(MapperInputBindingError),
    /// Pose stream manifest mismatch.
    #[error("pose input: {0}")]
    Pose(MapperInputBindingError),
}

/// Select-bound SDK streams can be run without any robot-specific adapter.
pub async fn run_sdk_voxel_mapper<S: MapUpdateSink>(
    sources: &VoxelMapperSources,
    point_layout: Rangefinder,
    map: &VoxelMap,
    point_cloud: StreamSubscription<PointCloudData>,
    pose: StreamSubscription<SpatialTransform>,
    sink: &S,
    config: VoxelMapperServiceConfig,
) -> Result<VoxelMapperRunReport, VoxelMapperServiceError> {
    let map_frame_binding = sources.frame_alias.clone().map_or_else(
        || VoxelMapperMapFrameBinding::Exact(sources.pose_frame.clone()),
        VoxelMapperMapFrameBinding::Aliased,
    );
    let mut runner = VoxelMapperRunner::from_sdk_contract_with_frame_binding(
        point_layout,
        sources.sensor_frame.clone(),
        map_frame_binding,
        map,
        config.free_delta,
        config.occupied_delta,
        config.alignment,
    )?;
    if let Some(persistence) = config.persistence {
        runner = runner.with_persistence(persistence)?;
    }
    let (point_cloud, pose) = sources.bind_inputs(point_cloud, pose)?;
    Ok(runner.run(point_cloud, pose, sink).await?)
}

/// Failure while binding or running the complete SDK voxel Mapper service.
#[derive(Debug, thiserror::Error)]
pub enum VoxelMapperServiceError {
    /// An accepted stream did not match the selected catalog row.
    #[error(transparent)]
    Bind(#[from] VoxelMapperInputBindingError),
    /// Contract validation, pose alignment, voxelization, or output failed.
    #[error(transparent)]
    Run(#[from] VoxelMapperRunError),
}

/// Automatic input discovery could not make one safe selection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VoxelMapperSourceSelectionError {
    /// No live point-cloud/pose pair shares a clock and connects the sensor
    /// frame to the selected Map frame.
    #[error("no compatible live point-cloud and pose logs for the selected Map")]
    NoCompatiblePair,
    /// Multiple compatible pairs remain; the app must constrain the query.
    #[error("{count} compatible voxel Mapper input pairs found; select exact source logs")]
    Ambiguous {
        /// Number of pairs which satisfy the full contract.
        count: usize,
    },
    /// The supplied alias is valid but targets a different Map frame.
    #[error("voxel Mapper frame alias does not target the selected Map frame")]
    AliasDoesNotTargetMap,
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use auki_datatypes::{
        map::MapUpdate,
        pose::{Quat, Vec3},
    };
    use auki_domain::StreamEntry;
    use auki_manifests::{PoseSource, PoseWriterMode};
    use auki_protocols::catalog::v2::{
        Available, Head, PoseBlock, PoseManifestPointer, SensorBlock, SensorManifestPointer,
    };
    use auki_protocols::stream::v2::StreamManifest;
    use auki_registry::{
        FiniteF64, FrameRegistryEntry, PointField, PointFieldDataType, VoxelValueModel,
    };
    use futures::{FutureExt, future::BoxFuture};

    struct RecordingSink {
        log_ref: LogRef,
        clock: RegistryRef,
        updates: Mutex<Vec<MapUpdate>>,
    }

    impl MapUpdateSink for RecordingSink {
        fn log_ref(&self) -> &LogRef {
            &self.log_ref
        }

        fn clock_ref(&self) -> &RegistryRef {
            &self.clock
        }

        fn append_from<'a>(
            &'a self,
            _alignment_clock: &'a RegistryRef,
            _timestamp_ns: i64,
            update: &'a MapUpdate,
        ) -> BoxFuture<'a, Result<(), crate::MapSinkError>> {
            self.updates.lock().unwrap().push(update.clone());
            futures::future::ready(Ok(())).boxed()
        }
    }

    fn reference(peer: &str, id: &str, hash: &str) -> RegistryRef {
        RegistryRef {
            peer_id: peer.into(),
            id: id.into(),
            hash: hash.into(),
        }
    }

    fn frame_reference(entry: &FrameRegistryEntry) -> RegistryRef {
        RegistryRef {
            peer_id: entry.peer_id.clone(),
            id: entry.frame_id.clone(),
            hash: entry.hash(),
        }
    }

    fn common() -> (Available, Option<Head>) {
        (
            Available {
                bytes: 0,
                entries: 0,
                duration_ns: 0,
            },
            Some(Head::Rolling {
                retention_ns: 1_000_000_000,
            }),
        )
    }

    fn point_row(clock: RegistryRef, frame: RegistryRef) -> ResourceEntry {
        let (available, head) = common();
        ResourceEntry {
            source_peer_id: "sensor-peer".into(),
            writer_peer_id: "point-cache-peer".into(),
            resource_id: "lidar/points".into(),
            state: "live".into(),
            head,
            extent: None,
            available,
            sensor: Some(SensorBlock {
                kind: SensorKind::Rangefinder,
                r#type: "point_cloud".into(),
                sensor_id: "lidar".into(),
                sensor_hash: "lidar-hash".into(),
            }),
            pose: None,
            variant_content: VariantContent::SensorLog {
                manifest: SensorManifestPointer {
                    clock,
                    frame: Some(frame),
                },
            },
        }
    }

    fn pose_row(clock: RegistryRef, from: RegistryRef, to: RegistryRef) -> ResourceEntry {
        let (available, head) = common();
        ResourceEntry {
            source_peer_id: "pose-peer".into(),
            writer_peer_id: "pose-peer".into(),
            resource_id: "lidar->world".into(),
            state: "live".into(),
            head,
            extent: None,
            available,
            sensor: None,
            pose: Some(PoseBlock {
                writer_mode: PoseWriterMode::Movable,
            }),
            variant_content: VariantContent::PoseLog {
                manifest: PoseManifestPointer {
                    from_frame: from,
                    to_frame: to,
                    clock,
                    source: PoseSource::Manual,
                    expected_rate_hz: 30,
                },
            },
        }
    }

    fn map(frame: RegistryRef) -> VoxelMap {
        VoxelMap {
            frame,
            voxel_size_m: FiniteF64(0.1),
            chunk_dimension: 32,
            value_model: VoxelValueModel::AdditiveOccupancyEvidence,
            color_model: None,
            semantic_classes: vec![],
        }
    }

    #[test]
    fn selects_compatible_logs_from_independent_peers() {
        let clock = reference("clock-peer", "session/monotonic", "clock-hash");
        let sensor_frame = reference("sensor-peer", "lidar", "lidar-frame-hash");
        let map_frame = reference("map-peer", "world", "world-frame-hash");
        let rows = vec![
            point_row(clock.clone(), sensor_frame.clone()),
            pose_row(clock.clone(), sensor_frame, map_frame.clone()),
        ];

        let selected =
            VoxelMapperSources::select(&rows, &map(map_frame), &Default::default()).unwrap();
        assert_eq!(selected.point_cloud.writer_peer_id, "point-cache-peer");
        assert_eq!(selected.pose.writer_peer_id, "pose-peer");
        assert_eq!(selected.clock, clock);
        assert_eq!(
            selected.point_cloud_log_ref(),
            LogRef {
                source_peer_id: "sensor-peer".into(),
                resource_id: "lidar/points".into(),
            }
        );
        assert_eq!(
            selected.point_cloud_request(ReadFrom::Latest),
            StreamRequest {
                source_peer_id: "sensor-peer".into(),
                resource_id: "lidar/points".into(),
                from: ReadFrom::Latest,
            }
        );
        assert_eq!(
            selected.pose_request(ReadFrom::FromStart).source_peer_id,
            "pose-peer"
        );
    }

    #[test]
    fn selects_remote_pose_frame_through_validated_local_alias() {
        let clock = reference("clock-peer", "session/monotonic", "clock-hash");
        let sensor_frame = reference("sensor-peer", "lidar", "lidar-frame-hash");
        let remote_entry = FrameRegistryEntry::ros_body("bracketbot", "local_world");
        let local_entry = FrameRegistryEntry::ros_body("park", "voxel/world");
        let remote_frame = frame_reference(&remote_entry);
        let local_frame = frame_reference(&local_entry);
        let alias = ValidatedFrameAlias::new(
            remote_frame.clone(),
            &remote_entry,
            local_frame.clone(),
            &local_entry,
        )
        .unwrap();
        let rows = vec![
            point_row(clock.clone(), sensor_frame.clone()),
            pose_row(clock, sensor_frame, remote_frame.clone()),
        ];

        assert_eq!(
            VoxelMapperSources::select(&rows, &map(local_frame.clone()), &Default::default()),
            Err(VoxelMapperSourceSelectionError::NoCompatiblePair)
        );
        let selected = VoxelMapperSources::select_with_frame_alias(
            &rows,
            &map(local_frame.clone()),
            &Default::default(),
            Some(&alias),
        )
        .unwrap();
        assert_eq!(selected.pose_frame, remote_frame);
        assert_eq!(selected.map_frame, local_frame);
        assert_eq!(selected.frame_alias, Some(alias));
    }

    #[test]
    fn rejects_cross_clock_pair() {
        let point_clock = reference("clock-peer", "session/monotonic", "clock-a");
        let pose_clock = reference("clock-peer", "session/monotonic", "clock-b");
        let sensor_frame = reference("sensor-peer", "lidar", "lidar-frame-hash");
        let map_frame = reference("map-peer", "world", "world-frame-hash");
        let rows = vec![
            point_row(point_clock, sensor_frame.clone()),
            pose_row(pose_clock, sensor_frame, map_frame.clone()),
        ];

        assert_eq!(
            VoxelMapperSources::select(&rows, &map(map_frame), &Default::default()),
            Err(VoxelMapperSourceSelectionError::NoCompatiblePair)
        );
    }

    #[test]
    fn ambiguity_requires_exact_log_selection() {
        let clock = reference("clock-peer", "session/monotonic", "clock-hash");
        let sensor_frame = reference("sensor-peer", "lidar", "lidar-frame-hash");
        let map_frame = reference("map-peer", "world", "world-frame-hash");
        let first_pose = pose_row(clock.clone(), sensor_frame.clone(), map_frame.clone());
        let mut second_pose = first_pose.clone();
        second_pose.source_peer_id = "other-pose-peer".into();
        second_pose.writer_peer_id = "other-pose-peer".into();
        second_pose.resource_id = "other-lidar->world".into();
        let rows = vec![point_row(clock, sensor_frame), first_pose, second_pose];

        assert_eq!(
            VoxelMapperSources::select(&rows, &map(map_frame.clone()), &Default::default()),
            Err(VoxelMapperSourceSelectionError::Ambiguous { count: 2 })
        );
        let selected = VoxelMapperSources::select(
            &rows,
            &map(map_frame),
            &VoxelMapperSourceQuery {
                point_cloud: None,
                pose: Some(LogRef {
                    source_peer_id: "pose-peer".into(),
                    resource_id: "lidar->world".into(),
                }),
            },
        )
        .unwrap();
        assert_eq!(selected.pose.source_peer_id, "pose-peer");
    }

    #[tokio::test]
    async fn selected_sdk_streams_run_all_the_way_to_map_updates() {
        let clock = reference("clock-peer", "session/monotonic", "clock-hash");
        let sensor_frame = reference("sensor-peer", "lidar", "lidar-frame-hash");
        let map_frame = reference("map-peer", "world", "world-frame-hash");
        let voxel_map = map(map_frame.clone());
        let rows = vec![
            point_row(clock.clone(), sensor_frame.clone()),
            pose_row(clock.clone(), sensor_frame.clone(), map_frame),
        ];
        let selected = VoxelMapperSources::select(&rows, &voxel_map, &Default::default()).unwrap();
        let manifest = |resource_id: &str| StreamManifest {
            resource_id: resource_id.into(),
            clock_peer_id: clock.peer_id.clone(),
            clock_id: clock.id.clone(),
            clock_hash: clock.hash.clone(),
            ..Default::default()
        };
        let point_cloud = StreamSubscription {
            manifest: manifest("lidar/points"),
            entries: Box::pin(futures::stream::iter(vec![Ok(StreamEntry {
                timestamp_ns: 5,
                seq: 0,
                payload: PointCloudData {
                    data: [
                        1.0_f32.to_le_bytes(),
                        0.0_f32.to_le_bytes(),
                        0.0_f32.to_le_bytes(),
                    ]
                    .concat(),
                },
            })])),
        };
        let pose = StreamSubscription {
            manifest: manifest("lidar->world"),
            entries: Box::pin(futures::stream::iter(vec![
                Ok(StreamEntry {
                    timestamp_ns: 0,
                    seq: 0,
                    payload: SpatialTransform {
                        translation: Some(Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        }),
                        orientation: Some(Quat {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                            w: 1.0,
                        }),
                    },
                }),
                Ok(StreamEntry {
                    timestamp_ns: 10,
                    seq: 1,
                    payload: SpatialTransform {
                        translation: Some(Vec3 {
                            x: 1.0,
                            y: 0.0,
                            z: 0.0,
                        }),
                        orientation: Some(Quat {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                            w: 1.0,
                        }),
                    },
                }),
            ])),
        };
        let point_layout = Rangefinder {
            r#type: "point_cloud".into(),
            fields: ["x", "y", "z"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| PointField {
                    name: name.into(),
                    offset: (index * 4) as u32,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                })
                .collect(),
            point_step: 12,
            is_bigendian: false,
            frame_rate_hz: 30,
            frame: sensor_frame,
        };
        let sink = RecordingSink {
            log_ref: LogRef {
                source_peer_id: "map-peer".into(),
                resource_id: "occupancy".into(),
            },
            clock,
            updates: Mutex::default(),
        };

        let report = run_sdk_voxel_mapper(
            &selected,
            point_layout,
            &voxel_map,
            point_cloud,
            pose,
            &sink,
            VoxelMapperServiceConfig {
                persistence: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(report.map_updates_written, 1);
        assert_eq!(sink.updates.lock().unwrap().len(), 1);
    }
}
