//! Timestamp-aligned live orchestration for detector-agnostic Portal mapping.

use std::collections::VecDeque;

use auki_datatypes::{camera::CameraFrame, map::MapUpdate, pose::SpatialTransform};
use auki_geometry::compose_spatial_transforms;
use auki_registry::{
    Camera, FrameRegistryEntry, LogRef, PortalMap, PortalObservationModel, RegistryRef,
};
use futures::{StreamExt, future::BoxFuture};

use crate::{
    ImagePoint, MapSinkError, MapUpdateSink, MapperInput, MapperInputError, PortalDefinition,
    PortalPnpError, TimedSdkSample, estimate_portal_observation,
    runner::{PoseBuffer, PoseResolution},
};

/// One detector result normalized for Portal mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct PortalCandidate {
    /// Detector-specific payload passed unchanged to [`PortalResolver`].
    pub payload: String,
    /// Image-space corners in strict `TL, TR, BR, BL` order.
    pub corners_px: [ImagePoint; 4],
}

/// All normalized candidates derived from one Detection Log sample.
#[derive(Debug, Clone, PartialEq)]
pub struct PortalDetectionBatch {
    /// Exact Camera Sensor Registry hash recorded by the detector envelope.
    pub sensor_hash: String,
    pub detections: Vec<PortalCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct PortalResolverError {
    pub detail: String,
}

impl PortalResolverError {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

/// Application-supplied boundary for Portal recognition and canonical size
/// lookup. `Ok(None)` means the detector payload is not a Portal.
pub trait PortalResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        payload: &'a str,
    ) -> BoxFuture<'a, Result<Option<PortalDefinition>, PortalResolverError>>;
}

/// Bounded buffering while Detection, Camera, and Pose streams arrive
/// independently on one SDK clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalMapperAlignmentConfig {
    pub maximum_pending_detection_batches: usize,
    pub maximum_buffered_camera_frames: usize,
    pub maximum_buffered_poses: usize,
}

impl Default for PortalMapperAlignmentConfig {
    fn default() -> Self {
        Self {
            maximum_pending_detection_batches: 32,
            maximum_buffered_camera_frames: 64,
            maximum_buffered_poses: 256,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortalMapperRunner {
    camera_sensor: RegistryRef,
    camera: Camera,
    camera_frame: FrameRegistryEntry,
    alignment: PortalMapperAlignmentConfig,
}

impl PortalMapperRunner {
    /// Bind the exact Camera, Camera Frame, Pose, and Portal Map contracts.
    pub fn from_sdk_contract(
        camera_sensor: RegistryRef,
        camera: Camera,
        camera_frame: FrameRegistryEntry,
        pose_from_frame: RegistryRef,
        pose_to_frame: RegistryRef,
        map: &PortalMap,
        alignment: PortalMapperAlignmentConfig,
    ) -> Result<Self, PortalMapperRunError> {
        if alignment.maximum_pending_detection_batches == 0
            || alignment.maximum_buffered_camera_frames == 0
            || alignment.maximum_buffered_poses < 2
        {
            return Err(PortalMapperRunError::InvalidConfiguration);
        }
        if camera.frame != pose_from_frame {
            return Err(PortalMapperRunError::CameraPoseFrameMismatch {
                camera_frame: Box::new(camera.frame.clone()),
                pose_from_frame: Box::new(pose_from_frame),
            });
        }
        if pose_to_frame != map.frame {
            return Err(PortalMapperRunError::MapFrameMismatch {
                pose_to_frame: Box::new(pose_to_frame),
                map_frame: Box::new(map.frame.clone()),
            });
        }
        if map.observation_model != PortalObservationModel::AppendOnlyPoseObservations {
            return Err(PortalMapperRunError::UnsupportedObservationModel);
        }
        if camera.frame.peer_id != camera_frame.peer_id
            || camera.frame.id != camera_frame.frame_id
            || camera.frame.hash != camera_frame.hash()
        {
            return Err(PortalMapperRunError::CameraFrameReferenceMismatch);
        }
        Ok(Self {
            camera_sensor,
            camera,
            camera_frame,
            alignment,
        })
    }

    /// Run until the Detection input ends and every resolvable pending batch
    /// has been written. Detection and Camera samples match at exactly the
    /// same timestamp; Camera→Map poses are interpolated at that timestamp.
    pub async fn run<R: PortalResolver, S: MapUpdateSink>(
        &self,
        mut detections: MapperInput<PortalDetectionBatch>,
        mut camera_frames: MapperInput<CameraFrame>,
        mut poses: MapperInput<SpatialTransform>,
        resolver: &R,
        sink: &S,
    ) -> Result<PortalMapperRunReport, PortalMapperRunError> {
        if detections.clock != camera_frames.clock || detections.clock != poses.clock {
            return Err(PortalMapperRunError::InputClockMismatch);
        }
        sink.validate_alignment_clock(&detections.clock)
            .map_err(PortalMapperRunError::Sink)?;

        let mut report = PortalMapperRunReport {
            detection_source: detections.log_ref.clone(),
            camera_source: camera_frames.log_ref.clone(),
            pose_source: poses.log_ref.clone(),
            map_destination: sink.log_ref().clone(),
            alignment_clock: detections.clock.clone(),
            map_clock: sink.clock_ref().clone(),
            detection_batches_received: 0,
            camera_frames_received: 0,
            poses_received: 0,
            candidates_received: 0,
            non_portal_candidates: 0,
            candidates_rejected_by_pnp: 0,
            observations_written: 0,
            map_updates_written: 0,
            detection_batches_without_camera: 0,
            detection_batches_without_pose: 0,
            detection_batches_dropped_for_backpressure: 0,
            camera_frames_dropped_for_backpressure: 0,
            poses_dropped_for_backpressure: 0,
        };
        let detection_source = detections.log_ref.clone();
        let mut pending = VecDeque::new();
        let mut cameras = VecDeque::new();
        let mut pose_buffer = PoseBuffer::default();
        let mut detections_done = false;
        let mut cameras_done = false;
        let mut poses_done = false;
        let mut last_detection_timestamp = None;
        let mut last_camera_timestamp = None;

        loop {
            resolve_pending(
                self,
                &detection_source,
                &mut pending,
                &mut cameras,
                &mut pose_buffer,
                cameras_done,
                poses_done,
                resolver,
                sink,
                &mut report,
            )
            .await?;

            if detections_done && pending.is_empty() {
                return Ok(report);
            }

            tokio::select! {
                detection = detections.samples.next(), if !detections_done => {
                    match detection {
                        Some(Ok(sample)) => {
                            if last_detection_timestamp.is_some_and(|last| sample.timestamp_ns < last) {
                                return Err(PortalMapperRunError::OutOfOrderDetection {
                                    previous: last_detection_timestamp.expect("checked above"),
                                    received: sample.timestamp_ns,
                                });
                            }
                            last_detection_timestamp = Some(sample.timestamp_ns);
                            if sample.payload.sensor_hash != self.camera_sensor.hash {
                                return Err(PortalMapperRunError::DetectionSensorMismatch {
                                    expected_hash: self.camera_sensor.hash.clone(),
                                    received_hash: sample.payload.sensor_hash,
                                });
                            }
                            report.detection_batches_received += 1;
                            report.candidates_received += sample.payload.detections.len() as u64;
                            pending.push_back(sample);
                            if pending.len() > self.alignment.maximum_pending_detection_batches {
                                pending.pop_front();
                                report.detection_batches_dropped_for_backpressure += 1;
                            }
                        }
                        Some(Err(error)) => return Err(PortalMapperRunError::DetectionInput(error)),
                        None => detections_done = true,
                    }
                }
                camera = camera_frames.samples.next(), if !cameras_done => {
                    match camera {
                        Some(Ok(sample)) => {
                            if last_camera_timestamp.is_some_and(|last| sample.timestamp_ns < last) {
                                return Err(PortalMapperRunError::OutOfOrderCamera {
                                    previous: last_camera_timestamp.expect("checked above"),
                                    received: sample.timestamp_ns,
                                });
                            }
                            last_camera_timestamp = Some(sample.timestamp_ns);
                            report.camera_frames_received += 1;
                            if cameras.back().is_some_and(|last: &TimedSdkSample<CameraFrame>| last.timestamp_ns == sample.timestamp_ns) {
                                cameras.pop_back();
                            }
                            cameras.push_back(sample);
                            if cameras.len() > self.alignment.maximum_buffered_camera_frames {
                                cameras.pop_front();
                                report.camera_frames_dropped_for_backpressure += 1;
                            }
                        }
                        Some(Err(error)) => return Err(PortalMapperRunError::CameraInput(error)),
                        None => cameras_done = true,
                    }
                }
                pose = poses.samples.next(), if !poses_done => {
                    match pose {
                        Some(Ok(sample)) => {
                            report.poses_received += 1;
                            if pose_buffer
                                .push(sample, self.alignment.maximum_buffered_poses)
                                .map_err(|error| PortalMapperRunError::PoseAlignment(error.to_string()))?
                            {
                                report.poses_dropped_for_backpressure += 1;
                            }
                        }
                        Some(Err(error)) => return Err(PortalMapperRunError::PoseInput(error)),
                        None => poses_done = true,
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_pending<R: PortalResolver, S: MapUpdateSink>(
    runner: &PortalMapperRunner,
    detection_source: &LogRef,
    pending: &mut VecDeque<TimedSdkSample<PortalDetectionBatch>>,
    cameras: &mut VecDeque<TimedSdkSample<CameraFrame>>,
    poses: &mut PoseBuffer,
    cameras_done: bool,
    poses_done: bool,
    resolver: &R,
    sink: &S,
    report: &mut PortalMapperRunReport,
) -> Result<(), PortalMapperRunError> {
    loop {
        let Some(batch) = pending.front() else {
            return Ok(());
        };
        while cameras
            .front()
            .is_some_and(|camera| camera.timestamp_ns < batch.timestamp_ns)
        {
            cameras.pop_front();
        }
        match cameras.front() {
            Some(camera) if camera.timestamp_ns == batch.timestamp_ns => {}
            Some(camera) if camera.timestamp_ns > batch.timestamp_ns => {
                pending.pop_front();
                report.detection_batches_without_camera += 1;
                continue;
            }
            _ if cameras_done => {
                pending.pop_front();
                report.detection_batches_without_camera += 1;
                continue;
            }
            _ => return Ok(()),
        }
        let camera_to_map = match poses
            .resolve(batch.timestamp_ns)
            .map_err(|error| PortalMapperRunError::PoseAlignment(error.to_string()))?
        {
            PoseResolution::Ready(pose) => pose,
            PoseResolution::BeforeAvailableRange => {
                pending.pop_front();
                report.detection_batches_without_pose += 1;
                continue;
            }
            PoseResolution::Waiting if !poses_done => return Ok(()),
            PoseResolution::Waiting => {
                pending.pop_front();
                report.detection_batches_without_pose += 1;
                continue;
            }
        };

        let batch = pending.pop_front().expect("front checked above");
        let camera = cameras.pop_front().expect("matching front checked above");
        let mut observations = Vec::new();
        for (index, candidate) in batch.payload.detections.into_iter().enumerate() {
            let Some(portal) = resolver
                .resolve(&candidate.payload)
                .await
                .map_err(PortalMapperRunError::Resolver)?
            else {
                report.non_portal_candidates += 1;
                continue;
            };
            let observation = match estimate_portal_observation(
                &runner.camera,
                &runner.camera_frame,
                &camera.payload,
                &portal,
                candidate.corners_px,
            ) {
                Ok(observation) => observation,
                Err(PortalPnpError::PoseEstimationFailed) => {
                    report.candidates_rejected_by_pnp += 1;
                    continue;
                }
                Err(error) => return Err(PortalMapperRunError::Pnp(error)),
            };
            let portal_to_map =
                compose_spatial_transforms(&observation.portal_to_camera, &camera_to_map)
                    .map_err(|error| PortalMapperRunError::Geometry(error.to_string()))?;
            observations.push(auki_datatypes::map::PortalObservation {
                portal_id: observation.portal_id,
                physical_size_m: observation.physical_size_m,
                portal_to_map: Some(portal_to_map),
                confidence: observation.confidence,
                normalized_corner_error: observation.normalized_corner_error,
                source_peer_id: detection_source.source_peer_id.clone(),
                source_resource_id: detection_source.resource_id.clone(),
                source_timestamp_ns: batch.timestamp_ns,
                source_sequence: batch.sequence,
                source_detection_index: u32::try_from(index)
                    .map_err(|_| PortalMapperRunError::TooManyDetections)?,
                camera_frame_peer_id: observation.camera_frame.peer_id,
                camera_frame_id: observation.camera_frame.id,
                camera_frame_hash: observation.camera_frame.hash,
            });
        }
        if observations.is_empty() {
            continue;
        }
        report.observations_written += observations.len() as u64;
        let update = MapUpdate {
            voxel_chunks: vec![],
            checkpoint: None,
            portal_observations: observations,
            portal_checkpoint: None,
        };
        sink.append_from(&report.alignment_clock, batch.timestamp_ns, &update)
            .await
            .map_err(PortalMapperRunError::Sink)?;
        report.map_updates_written += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalMapperRunReport {
    pub detection_source: LogRef,
    pub camera_source: LogRef,
    pub pose_source: LogRef,
    pub map_destination: LogRef,
    pub alignment_clock: RegistryRef,
    pub map_clock: RegistryRef,
    pub detection_batches_received: u64,
    pub camera_frames_received: u64,
    pub poses_received: u64,
    pub candidates_received: u64,
    pub non_portal_candidates: u64,
    pub candidates_rejected_by_pnp: u64,
    pub observations_written: u64,
    pub map_updates_written: u64,
    pub detection_batches_without_camera: u64,
    pub detection_batches_without_pose: u64,
    pub detection_batches_dropped_for_backpressure: u64,
    pub camera_frames_dropped_for_backpressure: u64,
    pub poses_dropped_for_backpressure: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PortalMapperRunError {
    #[error("invalid Portal Mapper runner configuration")]
    InvalidConfiguration,
    #[error("Camera frame does not match the pose source frame")]
    CameraPoseFrameMismatch {
        camera_frame: Box<RegistryRef>,
        pose_from_frame: Box<RegistryRef>,
    },
    #[error("pose destination frame does not match the Portal Map frame")]
    MapFrameMismatch {
        pose_to_frame: Box<RegistryRef>,
        map_frame: Box<RegistryRef>,
    },
    #[error("Camera Frame Registry entry does not match the Camera reference")]
    CameraFrameReferenceMismatch,
    #[error("unsupported Portal observation model")]
    UnsupportedObservationModel,
    #[error("Detection, Camera, and Pose inputs must use one exact SDK clock")]
    InputClockMismatch,
    #[error(
        "Detection Sensor Registry hash mismatch: expected {expected_hash}, received {received_hash}"
    )]
    DetectionSensorMismatch {
        expected_hash: String,
        received_hash: String,
    },
    #[error("Detection input: {0}")]
    DetectionInput(MapperInputError),
    #[error("Camera input: {0}")]
    CameraInput(MapperInputError),
    #[error("Pose input: {0}")]
    PoseInput(MapperInputError),
    #[error("Detection timestamps out of order: {received} after {previous}")]
    OutOfOrderDetection { previous: i64, received: i64 },
    #[error("Camera timestamps out of order: {received} after {previous}")]
    OutOfOrderCamera { previous: i64, received: i64 },
    #[error("Pose alignment failed: {0}")]
    PoseAlignment(String),
    #[error("Portal resolver: {0}")]
    Resolver(PortalResolverError),
    #[error("Portal PnP: {0}")]
    Pnp(PortalPnpError),
    #[error("spatial composition failed: {0}")]
    Geometry(String),
    #[error("one Detection batch contains more than u32::MAX candidates")]
    TooManyDetections,
    #[error("map sink: {0}")]
    Sink(MapSinkError),
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use auki_datatypes::{
        camera::CameraFrame,
        pose::{Quat, SpatialTransform, Vec3},
    };
    use auki_registry::{CameraCalibration, FiniteF64};
    use futures::FutureExt;
    use pnp_core::{Camera as PnpCamera, Vector3};

    struct StaticResolver;

    impl PortalResolver for StaticResolver {
        fn resolve<'a>(
            &'a self,
            payload: &'a str,
        ) -> BoxFuture<'a, Result<Option<PortalDefinition>, PortalResolverError>> {
            futures::future::ready(Ok((payload == "auki://portal/office").then(|| {
                PortalDefinition {
                    portal_id: "portal:office".into(),
                    physical_size_m: 0.2,
                }
            })))
            .boxed()
        }
    }

    struct RecordingSink {
        destination: LogRef,
        clock: RegistryRef,
        updates: Mutex<Vec<(i64, MapUpdate)>>,
    }

    impl MapUpdateSink for RecordingSink {
        fn log_ref(&self) -> &LogRef {
            &self.destination
        }

        fn clock_ref(&self) -> &RegistryRef {
            &self.clock
        }

        fn append_from<'a>(
            &'a self,
            _alignment_clock: &'a RegistryRef,
            timestamp_ns: i64,
            update: &'a MapUpdate,
        ) -> BoxFuture<'a, Result<(), MapSinkError>> {
            self.updates
                .lock()
                .unwrap()
                .push((timestamp_ns, update.clone()));
            futures::future::ready(Ok(())).boxed()
        }
    }

    fn registry_ref(peer: &str, id: &str, hash: &str) -> RegistryRef {
        RegistryRef {
            peer_id: peer.into(),
            id: id.into(),
            hash: hash.into(),
        }
    }

    fn log_ref(peer: &str, id: &str) -> LogRef {
        LogRef {
            source_peer_id: peer.into(),
            resource_id: id.into(),
        }
    }

    fn input<T: Send + 'static>(
        source: LogRef,
        clock: RegistryRef,
        samples: Vec<TimedSdkSample<T>>,
    ) -> MapperInput<T> {
        MapperInput::new(
            source,
            clock,
            Box::pin(futures::stream::iter(samples.into_iter().map(Ok))),
        )
    }

    fn calibration() -> CameraCalibration {
        CameraCalibration {
            fx: FiniteF64(420.0),
            fy: FiniteF64(420.0),
            cx: FiniteF64(640.0),
            cy: FiniteF64(360.0),
            distortion_coefficients: vec![
                FiniteF64(0.012),
                FiniteF64(-0.004),
                FiniteF64(0.0007),
                FiniteF64(-0.0001),
            ],
        }
    }

    fn corners() -> [ImagePoint; 4] {
        let calibration = calibration();
        let model = PnpCamera::opencv_fisheye(
            calibration.fx.0,
            calibration.fy.0,
            calibration.cx.0,
            calibration.cy.0,
            &calibration
                .distortion_coefficients
                .iter()
                .map(|value| value.0)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let half = 0.1;
        [
            Vector3::new(-half, -half, 2.0),
            Vector3::new(half, -half, 2.0),
            Vector3::new(half, half, 2.0),
            Vector3::new(-half, half, 2.0),
        ]
        .map(|point| {
            let pixel = model.project(point).unwrap();
            ImagePoint {
                x: pixel.x,
                y: pixel.y,
            }
        })
    }

    fn pose(x: f64) -> SpatialTransform {
        SpatialTransform {
            translation: Some(Vec3 { x, y: 0.0, z: 0.0 }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        }
    }

    struct Contract {
        runner: PortalMapperRunner,
        clock: RegistryRef,
        map: PortalMap,
    }

    fn contract() -> Contract {
        let camera_frame = FrameRegistryEntry::ros_optical("bracketbot", "head_left_optical");
        let camera_frame_ref = RegistryRef {
            peer_id: camera_frame.peer_id.clone(),
            id: camera_frame.frame_id.clone(),
            hash: camera_frame.hash(),
        };
        let camera = Camera {
            r#type: "rgb".into(),
            width: 1280,
            height: 720,
            frame_rate_hz: 30,
            image_encoding: "jpeg".into(),
            pixel_format: "rgb8".into(),
            row_stride_bytes: 0,
            color_space: "srgb".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "opencv_fisheye".into(),
            calibration: Some(calibration()),
            frame: camera_frame_ref.clone(),
        };
        let map_frame = registry_ref("bracketbot", "map", "map-frame-hash");
        let map = PortalMap {
            frame: map_frame.clone(),
            observation_model: PortalObservationModel::AppendOnlyPoseObservations,
        };
        let runner = PortalMapperRunner::from_sdk_contract(
            registry_ref("bracketbot", "head_left_rgb", "camera-sensor-hash"),
            camera,
            camera_frame,
            camera_frame_ref,
            map_frame,
            &map,
            PortalMapperAlignmentConfig::default(),
        )
        .unwrap();
        Contract {
            runner,
            clock: registry_ref("bracketbot", "clock", "clock-hash"),
            map,
        }
    }

    #[tokio::test]
    async fn aligns_detection_camera_and_pose_into_portal_map_update() {
        let Contract { runner, clock, map } = contract();
        let detection_source = log_ref("bracketbot", "qr/head_left");
        let detections = input(
            detection_source.clone(),
            clock.clone(),
            vec![TimedSdkSample {
                sequence: 7,
                timestamp_ns: 10,
                payload: PortalDetectionBatch {
                    sensor_hash: "camera-sensor-hash".into(),
                    detections: vec![
                        PortalCandidate {
                            payload: "ordinary qr".into(),
                            corners_px: corners(),
                        },
                        PortalCandidate {
                            payload: "auki://portal/office".into(),
                            corners_px: corners(),
                        },
                    ],
                },
            }],
        );
        let cameras = input(
            log_ref("bracketbot", "camera/head_left"),
            clock.clone(),
            vec![TimedSdkSample {
                sequence: 3,
                timestamp_ns: 10,
                payload: CameraFrame {
                    frame: vec![],
                    dynamic_intrinsics: None,
                },
            }],
        );
        let poses = input(
            log_ref("bracketbot", "pose/head_left_to_map"),
            clock.clone(),
            vec![
                TimedSdkSample {
                    sequence: 0,
                    timestamp_ns: 0,
                    payload: pose(10.0),
                },
                TimedSdkSample {
                    sequence: 1,
                    timestamp_ns: 20,
                    payload: pose(12.0),
                },
            ],
        );
        let sink = RecordingSink {
            destination: log_ref("park", "portal-map"),
            clock: clock.clone(),
            updates: Mutex::new(vec![]),
        };

        let report = runner
            .run(detections, cameras, poses, &StaticResolver, &sink)
            .await
            .unwrap();

        assert_eq!(report.candidates_received, 2);
        assert_eq!(report.non_portal_candidates, 1);
        assert_eq!(report.observations_written, 1);
        assert_eq!(report.map_updates_written, 1);
        let updates = sink.updates.lock().unwrap();
        assert_eq!(updates[0].0, 10);
        let observation = &updates[0].1.portal_observations[0];
        assert_eq!(observation.portal_id, "portal:office");
        assert_eq!(observation.source_peer_id, detection_source.source_peer_id);
        assert_eq!(observation.source_resource_id, detection_source.resource_id);
        assert_eq!(observation.source_sequence, 7);
        assert_eq!(observation.source_detection_index, 1);
        let translation = observation
            .portal_to_map
            .as_ref()
            .unwrap()
            .translation
            .as_ref()
            .unwrap();
        assert!((translation.x - 11.0).abs() < 1e-5);
        assert!(translation.y.abs() < 1e-5);
        assert!((translation.z - 2.0).abs() < 1e-5);

        let map_ref = auki_registry::MapRegistryEntry {
            peer_id: "park".into(),
            map_id: "portal-map".into(),
            body: auki_registry::MapBody::Portal(map.clone()),
        }
        .registry_ref();
        let mut accumulator = auki_maps::PortalMapAccumulator::new(map_ref, map).unwrap();
        assert_eq!(
            accumulator.apply(&updates[0].1).unwrap().observations_added,
            1
        );
    }

    #[tokio::test]
    async fn drops_detection_without_exact_camera_timestamp() {
        let Contract { runner, clock, .. } = contract();
        let detections = input(
            log_ref("bracketbot", "qr/head_left"),
            clock.clone(),
            vec![TimedSdkSample {
                sequence: 7,
                timestamp_ns: 10,
                payload: PortalDetectionBatch {
                    sensor_hash: "camera-sensor-hash".into(),
                    detections: vec![PortalCandidate {
                        payload: "auki://portal/office".into(),
                        corners_px: corners(),
                    }],
                },
            }],
        );
        let cameras = input(
            log_ref("bracketbot", "camera/head_left"),
            clock.clone(),
            vec![TimedSdkSample {
                sequence: 3,
                timestamp_ns: 11,
                payload: CameraFrame {
                    frame: vec![],
                    dynamic_intrinsics: None,
                },
            }],
        );
        let poses = input(
            log_ref("bracketbot", "pose/head_left_to_map"),
            clock.clone(),
            vec![TimedSdkSample {
                sequence: 0,
                timestamp_ns: 10,
                payload: pose(0.0),
            }],
        );
        let sink = RecordingSink {
            destination: log_ref("park", "portal-map"),
            clock,
            updates: Mutex::new(vec![]),
        };

        let report = runner
            .run(detections, cameras, poses, &StaticResolver, &sink)
            .await
            .unwrap();

        assert_eq!(report.detection_batches_without_camera, 1);
        assert_eq!(report.map_updates_written, 0);
    }
}
