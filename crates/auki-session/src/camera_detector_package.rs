//! Application-facing registration and instance lifecycle for camera detectors.

use std::collections::HashSet;
use std::sync::Arc;

use auki_registry::{
    DetectorBody, DetectorInput, DetectorRegistryEntry, LogRef, RegistryRef, SensorBody,
    SensorRegistryEntry,
};
use futures::Stream;
use thiserror::Error;

use crate::{
    CameraDetector, CameraFrameSample, CameraInputBinding, DetectionCadence, DetectionLogHandle,
    DetectionLogSpec, DetectorInstanceSpec, DetectorOutput, DetectorRunnerError, DetectorTask,
    Peer, SensorLogHandle, Session, StreamingDetectorTask,
};

type DetectorFactory = Arc<dyn Fn() -> Box<dyn CameraDetector> + Send + Sync>;

/// A registered bring-your-own camera detector implementation.
#[derive(Clone)]
pub struct RegisteredCameraDetector {
    registry_ref: RegistryRef,
    factory: DetectorFactory,
}

/// Resolved immutable identity and Sensor Registry entry for a live stream.
#[derive(Debug, Clone)]
pub struct CameraStreamDescriptor {
    pub log_ref: LogRef,
    pub sensor: SensorRegistryEntry,
    pub clock: RegistryRef,
}

/// A camera detector instance consuming a local Sensor Log.
pub struct CameraDetectorTask {
    detection_log: DetectionLogHandle,
    task: DetectorTask,
}

/// A camera detector instance consuming an asynchronous frame stream.
pub struct StreamingCameraDetectorTask {
    detection_log: DetectionLogHandle,
    task: StreamingDetectorTask,
}

impl RegisteredCameraDetector {
    /// Register a detector implementation and its discoverable contracts.
    pub fn register<D, F>(
        peer: &Peer,
        detector_id: &str,
        body: DetectorBody,
        input_types: Vec<DetectorInput>,
        output_types: Vec<String>,
        factory: F,
    ) -> Result<Self, CameraDetectorPackageError>
    where
        D: CameraDetector,
        F: Fn() -> D + Send + Sync + 'static,
    {
        if input_types.is_empty()
            || output_types.is_empty()
            || output_types.iter().any(|output| output.is_empty())
        {
            return Err(CameraDetectorPackageError::InvalidDescriptor);
        }
        if matches!(&body, DetectorBody::Custom(custom) if custom.kind.is_empty()) {
            return Err(CameraDetectorPackageError::InvalidDescriptor);
        }
        let registry_ref = peer
            .register_detector_with_inputs(detector_id, body, input_types, output_types)
            .map_err(CameraDetectorPackageError::Register)?;
        Ok(Self {
            registry_ref,
            factory: Arc::new(move || Box::new(factory())),
        })
    }

    pub fn registry_ref(&self) -> &RegistryRef {
        &self.registry_ref
    }

    /// Start an independent detector instance on a local Camera Sensor Log.
    pub fn start(
        &self,
        session: &Session,
        instance: DetectorInstanceSpec,
        input_log: &SensorLogHandle,
    ) -> Result<CameraDetectorTask, CameraDetectorPackageError> {
        if input_log.manifest.session_id != session.session_id()
            || input_log.manifest.source_peer_id != session.peer_id()
        {
            return Err(CameraDetectorPackageError::InputSessionMismatch);
        }
        let input_sensor = session
            .sensor_registry_entry(&input_log.manifest.sensor)
            .ok_or(CameraDetectorPackageError::UnknownInputSensorReference)?;
        let SensorBody::Camera(camera) = &input_sensor.body else {
            return Err(CameraDetectorPackageError::IncompatibleSensorKind);
        };
        let detector_entry = self.validate_start(session, &input_sensor, instance.cadence)?;
        let detection_log = self.register_output(
            session,
            instance,
            input_log.log_ref.clone(),
            input_log.manifest.sensor.clone(),
            input_log.manifest.clock.clone(),
        )?;
        let task = DetectorTask::start(
            self.create_checked(&detector_entry),
            camera.clone(),
            input_log,
            &detection_log,
        )?;
        Ok(CameraDetectorTask {
            detection_log,
            task,
        })
    }

    /// Start an independent detector instance on a live asynchronous stream.
    pub fn start_stream<S>(
        &self,
        session: &Session,
        instance: DetectorInstanceSpec,
        input: CameraStreamDescriptor,
        frames: S,
    ) -> Result<StreamingCameraDetectorTask, CameraDetectorPackageError>
    where
        S: Stream<Item = std::result::Result<CameraFrameSample, String>> + Send + 'static,
    {
        if input.log_ref.source_peer_id != input.sensor.peer_id {
            return Err(CameraDetectorPackageError::InputSourcePeerMismatch);
        }
        let SensorBody::Camera(camera) = &input.sensor.body else {
            return Err(CameraDetectorPackageError::IncompatibleSensorKind);
        };
        let detector_entry = self.validate_start(session, &input.sensor, instance.cadence)?;
        let sensor = RegistryRef {
            peer_id: input.sensor.peer_id.clone(),
            id: input.sensor.sensor_id.clone(),
            hash: input.sensor.hash(),
        };
        let detection_log = self.register_output(
            session,
            instance,
            input.log_ref.clone(),
            sensor.clone(),
            input.clock.clone(),
        )?;
        let task = StreamingDetectorTask::start(
            self.create_checked(&detector_entry),
            camera.clone(),
            CameraInputBinding {
                log_ref: input.log_ref,
                sensor,
                clock: input.clock,
            },
            frames,
            &detection_log,
        )?;
        Ok(StreamingCameraDetectorTask {
            detection_log,
            task,
        })
    }

    fn validate_start(
        &self,
        session: &Session,
        input_sensor: &SensorRegistryEntry,
        cadence: DetectionCadence,
    ) -> Result<DetectorRegistryEntry, CameraDetectorPackageError> {
        if matches!(cadence, DetectionCadence::Periodic { period_ns: 0 }) {
            return Err(CameraDetectorPackageError::InvalidCadence);
        }
        let entry = auki_registry::read_detector(
            &session.storage_root(),
            &self.registry_ref.peer_id,
            &self.registry_ref.id,
            &self.registry_ref.hash,
        )
        .map_err(CameraDetectorPackageError::ResolveDetector)?
        .ok_or(CameraDetectorPackageError::UnknownDetectorReference)?;
        if entry.hash() != self.registry_ref.hash {
            return Err(CameraDetectorPackageError::DetectorReferenceMismatch);
        }
        if !entry.accepts_input(&input_sensor.body) {
            return Err(CameraDetectorPackageError::DetectorRejectsInput);
        }
        Ok(entry)
    }

    fn register_output(
        &self,
        session: &Session,
        instance: DetectorInstanceSpec,
        input_log: LogRef,
        input_sensor: RegistryRef,
        clock: RegistryRef,
    ) -> Result<DetectionLogHandle, CameraDetectorPackageError> {
        if let Some(existing) = session
            .logs()
            .detection_logs()
            .into_iter()
            .find(|handle| handle.resource_id() == instance.instance_id)
        {
            let manifest = &existing.manifest;
            let segment_duration_ns =
                instance.segment_duration.as_nanos().min(i64::MAX as u128) as i64;
            let retention_ns = instance.retention.as_nanos().min(i64::MAX as u128) as i64;
            if manifest.detector != self.registry_ref
                || manifest.input_log != input_log
                || manifest.input_sensor != input_sensor
                || manifest.clock != clock
                || manifest.cadence != instance.cadence
                || existing.head_spec != instance.head
                || manifest.segment_duration_ns != segment_duration_ns
                || manifest.retention_ns != retention_ns
            {
                return Err(CameraDetectorPackageError::InstanceContractMismatch(
                    instance.instance_id,
                ));
            }
            return Ok(existing.as_ref().clone());
        }
        session
            .register_detection_log(DetectionLogSpec {
                instance_id: instance.instance_id,
                detector: self.registry_ref.clone(),
                input_log,
                input_sensor,
                clock,
                cadence: instance.cadence,
                head: instance.head,
                segment_duration: instance.segment_duration,
                retention: instance.retention,
            })
            .map_err(CameraDetectorPackageError::StartInstance)
    }

    fn create_checked(&self, entry: &DetectorRegistryEntry) -> OutputCheckedDetector {
        OutputCheckedDetector {
            inner: (self.factory)(),
            allowed_outputs: entry.output_types.iter().cloned().collect(),
        }
    }
}

struct OutputCheckedDetector {
    inner: Box<dyn CameraDetector>,
    allowed_outputs: HashSet<String>,
}

impl CameraDetector for OutputCheckedDetector {
    fn process(
        &mut self,
        frame: &auki_datatypes::camera::CameraFrame,
        camera: &auki_registry::Camera,
    ) -> std::result::Result<Vec<DetectorOutput>, String> {
        let outputs = self.inner.process(frame, camera)?;
        if let Some(output) = outputs
            .iter()
            .find(|output| !self.allowed_outputs.contains(&output.r#type))
        {
            return Err(format!(
                "detector emitted undeclared output type {:?}",
                output.r#type
            ));
        }
        Ok(outputs)
    }
}

impl CameraDetectorTask {
    pub fn detection_log(&self) -> &DetectionLogHandle {
        &self.detection_log
    }

    pub fn request_shutdown(&self) {
        self.task.request_shutdown();
    }

    pub fn shutdown(self) -> Result<(), CameraDetectorPackageError> {
        self.task.shutdown().map_err(Into::into)
    }
}

impl StreamingCameraDetectorTask {
    pub fn detection_log(&self) -> &DetectionLogHandle {
        &self.detection_log
    }

    pub fn request_shutdown(&self) {
        self.task.request_shutdown();
    }

    /// Live frames replaced by fresher pending work while the detector was
    /// busy. Frames skipped by cadence are not included.
    pub fn dropped_frames(&self) -> u64 {
        self.task.dropped_frames()
    }

    pub async fn shutdown(self) -> Result<(), CameraDetectorPackageError> {
        self.task.shutdown().await.map_err(Into::into)
    }
}

/// Errors produced by generic detector registration and instance startup.
#[derive(Debug, Error)]
pub enum CameraDetectorPackageError {
    #[error("detector descriptor must declare inputs, outputs, and a non-empty custom kind")]
    InvalidDescriptor,
    #[error("could not register detector: {0}")]
    Register(crate::SessionError),
    #[error("could not start detector instance: {0}")]
    StartInstance(crate::SessionError),
    #[error("detector cadence period must be positive")]
    InvalidCadence,
    #[error("input Sensor Log does not belong to this session")]
    InputSessionMismatch,
    #[error("input Sensor Log source peer does not match the Sensor Registry peer")]
    InputSourcePeerMismatch,
    #[error("input must be a camera sensor")]
    IncompatibleSensorKind,
    #[error("input Sensor Registry reference does not exist")]
    UnknownInputSensorReference,
    #[error("could not resolve Detector Registry entry: {0}")]
    ResolveDetector(auki_registry::Error),
    #[error("Detector Registry reference does not exist")]
    UnknownDetectorReference,
    #[error("Detector Registry reference hash mismatch")]
    DetectorReferenceMismatch,
    #[error("Detector Registry input contracts reject the selected sensor")]
    DetectorRejectsInput,
    #[error("detector instance {0:?} is already registered with a different contract")]
    InstanceContractMismatch(String),
    #[error("detector runner: {0}")]
    Runner(#[from] DetectorRunnerError),
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use auki_datatypes::camera::CameraFrame;
    use auki_datatypes::detection::DetectionFrame;
    use auki_logs::Log;
    use auki_registry::{Camera, CustomDetector, DetectorInput, SensorBody};
    use tempfile::tempdir;

    use super::*;
    use crate::{DetectionCadence, DetectorInstanceSpec, FrameDef, HeadSpec, SensorLogSpec};

    struct CountingDetector {
        count: u8,
        output_type: &'static str,
    }

    impl CameraDetector for CountingDetector {
        fn process(
            &mut self,
            _frame: &CameraFrame,
            _camera: &Camera,
        ) -> std::result::Result<Vec<DetectorOutput>, String> {
            self.count += 1;
            Ok(vec![DetectorOutput {
                r#type: self.output_type.into(),
                data: vec![self.count],
            }])
        }
    }

    fn camera_input() -> DetectorInput {
        DetectorInput {
            sensor_kind: "camera".into(),
            sensor_type: Some("mono".into()),
            image_encoding: Some("raw".into()),
            pixel_format: Some("luma8".into()),
        }
    }

    fn setup_input(
        peer_id: &str,
    ) -> (
        tempfile::TempDir,
        Peer,
        Session,
        SensorLogHandle,
        Log<CameraFrame>,
    ) {
        let tmp = tempdir().unwrap();
        let peer = Peer::new(peer_id, "detector-test").with_storage_root(tmp.path().to_path_buf());
        let frame = peer
            .register_frame("camera-optical", FrameDef::RosOptical)
            .unwrap();
        let sensor = peer
            .register_sensor(
                "camera",
                SensorBody::Camera(Camera {
                    r#type: "mono".into(),
                    width: 2,
                    height: 2,
                    frame_rate_hz: 30,
                    image_encoding: "raw".into(),
                    pixel_format: "luma8".into(),
                    row_stride_bytes: 2,
                    color_space: "srgb".into(),
                    intrinsics_model: "pinhole".into(),
                    distortion_model: "none".into(),
                    calibration: None,
                    frame: frame.clone(),
                }),
            )
            .unwrap();
        let session = peer.start_session().unwrap();
        let input = session
            .register_sensor_log(SensorLogSpec {
                sensor,
                clock: session.monotonic_clock(),
                frame: Some(frame),
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();
        let writer =
            Log::<CameraFrame>::open(input.root(), serde_json::to_value(&input.manifest).unwrap())
                .unwrap();
        (tmp, peer, session, input, writer)
    }

    fn instance(id: &str) -> DetectorInstanceSpec {
        DetectorInstanceSpec::rolling(
            id,
            DetectionCadence::EveryFrame,
            Duration::from_secs(5),
            Duration::from_secs(1),
        )
    }

    fn wait_for_entry(log: &DetectionLogHandle) -> DetectionFrame {
        for _ in 0..100 {
            if let Ok(reader) = Log::<DetectionFrame>::read(log.root()) {
                if let Ok(mut entries) = reader.entries() {
                    if let Some(entry) = entries.pop() {
                        return entry.payload;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("detector did not write an output");
    }

    #[test]
    fn custom_detector_factory_creates_independent_instances() {
        let (_tmp, peer, session, input, mut writer) = setup_input("custom-peer");
        let detector = RegisteredCameraDetector::register(
            &peer,
            "counter",
            DetectorBody::Custom(CustomDetector {
                kind: "com.example.counter".into(),
                configuration: serde_json::json!({"version": 1}),
            }),
            vec![camera_input()],
            vec!["example.counter".into()],
            || CountingDetector {
                count: 0,
                output_type: "example.counter",
            },
        )
        .unwrap();

        let first = detector.start(&session, instance("first"), &input).unwrap();
        let second = detector
            .start(&session, instance("second"), &input)
            .unwrap();
        writer
            .append(
                1,
                &CameraFrame {
                    dynamic_intrinsics: None,
                    frame: vec![0; 4],
                },
            )
            .unwrap();
        writer.flush().unwrap();

        assert_eq!(wait_for_entry(first.detection_log()).data, vec![1]);
        assert_eq!(wait_for_entry(second.detection_log()).data, vec![1]);
        first.shutdown().unwrap();
        second.shutdown().unwrap();
    }

    #[test]
    fn stopped_instance_reuses_its_registered_detection_log() {
        let (_tmp, peer, session, input, _writer) = setup_input("restart-peer");
        let detector = RegisteredCameraDetector::register(
            &peer,
            "restartable",
            DetectorBody::Custom(CustomDetector {
                kind: "com.example.restartable".into(),
                configuration: serde_json::Value::Null,
            }),
            vec![camera_input()],
            vec!["example.counter".into()],
            || CountingDetector {
                count: 0,
                output_type: "example.counter",
            },
        )
        .unwrap();

        let first = detector
            .start(&session, instance("restartable"), &input)
            .unwrap();
        let first_root = first.detection_log().root().to_path_buf();
        first.shutdown().unwrap();

        let restarted = detector
            .start(&session, instance("restartable"), &input)
            .unwrap();
        assert_eq!(restarted.detection_log().root(), first_root);
        restarted.shutdown().unwrap();
        assert_eq!(session.logs().detection_logs().len(), 1);
    }

    #[test]
    fn undeclared_detector_output_fails_the_instance() {
        let (_tmp, peer, session, input, mut writer) = setup_input("contract-peer");
        let detector = RegisteredCameraDetector::register(
            &peer,
            "bad-output",
            DetectorBody::Custom(CustomDetector {
                kind: "com.example.bad-output".into(),
                configuration: serde_json::Value::Null,
            }),
            vec![camera_input()],
            vec!["declared".into()],
            || CountingDetector {
                count: 0,
                output_type: "undeclared",
            },
        )
        .unwrap();
        let task = detector
            .start(&session, instance("bad-output"), &input)
            .unwrap();
        writer
            .append(
                1,
                &CameraFrame {
                    dynamic_intrinsics: None,
                    frame: vec![0; 4],
                },
            )
            .unwrap();
        writer.flush().unwrap();

        std::thread::sleep(Duration::from_millis(50));
        assert!(matches!(
            task.shutdown(),
            Err(CameraDetectorPackageError::Runner(DetectorRunnerError::Detector(message)))
                if message.contains("undeclared output type")
        ));
    }
}
