//! Native detector-to-localizer adapter with per-observation anchor resolution.
use crate::*;
use auki_components::*;
use auki_qr_detector::{
    QR_DETECTION_SCHEMA_VERSION, QR_DETECTIONS_SCHEMA, QrDetections, QrSourceFrame,
};
use auki_scenegraph::{
    component::SnapshotReference,
    resolution::{QrResolver, ResolvedQr},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub const LOCALIZATION_SCHEMA: &str = "auki.qr-localization/v3";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalizationBatch {
    pub calibration: Calibration,
    pub detection_product: ProductReference,
    pub detection_sequence: u64,
    pub timestamp_ns: u64,
    pub source_frame: Option<QrSourceFrame>,
    /// Nonempty means the entire batch is unusable; no poses are emitted.
    pub rejection: Option<String>,
    pub estimates: Vec<LocalizedQr>,
    /// Detection indices absent from the resolver; hand these to the Mapper.
    pub needs_mapping: Vec<usize>,
    /// Index in the source detection batch and reason for rejecting that code.
    pub rejected_codes: Vec<RejectedCode>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalizedQr {
    pub detection_index: usize,
    pub qr: ResolvedQr,
    pub pose: CameraPoseEstimate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectedCode {
    pub index: usize,
    pub reason: String,
}
impl ContractType for LocalizationBatch {
    const DATATYPE: &'static str = "auki.qr-localization";
}

#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error(transparent)]
    Localization(#[from] LocalizationError),
    #[error(transparent)]
    Component(Box<ComponentBuildError>),
}

impl From<ComponentBuildError> for BindError {
    fn from(error: ComponentBuildError) -> Self {
        Self::Component(Box::new(error))
    }
}

pub const PAUSE: &str = "pause";
pub const RESUME: &str = "resume";
pub const LOCALIZE_ONCE: &str = "localize_once";
pub const STATUS: &str = "status";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ControlRequest {}
impl ContractType for ControlRequest {
    const DATATYPE: &'static str = "auki.qr-localizer.control/v1";
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalizeOnce {
    pub detection_product: ProductReference,
    pub detection_sequence: u64,
    /// Index in the original detection batch, not an anchor ID.
    pub detection_index: usize,
    /// Exact target map Product and revision; its definition declares the target frame.
    pub target_map: SnapshotReference,
}
impl ContractType for LocalizeOnce {
    const DATATYPE: &'static str = "auki.qr-localizer.localize-once/v2";
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessingMode {
    Running,
    Paused,
    Closed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizerStatus {
    pub mode: ProcessingMode,
    pub continuous_processed: u64,
    pub one_shot_processed: u64,
    pub skipped: u64,
    pub last_detection_sequence: Option<u64>,
}
impl ContractType for LocalizerStatus {
    const DATATYPE: &'static str = "auki.qr-localizer.status/v1";
}
struct ControlState {
    status: LocalizerStatus,
    /// Resume is future-only: drain but do not process observations already retained.
    resume_after: Option<u64>,
}
struct Processor {
    resolver: Arc<dyn QrResolver>,
    calibration: Calibration,
    gate: QualityGate,
    detections: RetainedProduct<QrDetections>,
}
impl Processor {
    fn process(&self, observation: &Observation<QrDetections>) -> LocalizationBatch {
        let mut batch = localize_batch(
            self.resolver.as_ref(),
            &self.calibration,
            self.gate,
            self.detections.reference(),
            observation,
        );
        if observation.output != self.detections.manifest.producer {
            batch.estimates.clear();
            batch.needs_mapping.clear();
            batch.rejection = Some("detection_producer_mismatch".into());
        }
        batch
    }
}

pub struct QrLocalizerComponent {
    input: Option<ConfiguredBufferInput<QrDetections>>,
    component: Component,
    poses: ConfiguredObservable<LocalizationBatch>,
    state: Arc<Mutex<ControlState>>,
    pause: Operable<ControlRequest, LocalizerStatus>,
    resume: Operable<ControlRequest, LocalizerStatus>,
    status: Operable<ControlRequest, LocalizerStatus>,
    localize_once: Operable<LocalizeOnce, LocalizationBatch>,
}
impl QrLocalizerComponent {
    /// Paused by default: localization requires an explicit request or resume.
    /// Control authorization defaults to the owning peer only.
    pub fn bind(
        runtime: &ComponentRuntime,
        component_id: impl Into<String>,
        output_id: impl Into<String>,
        detections: &RetainedProduct<QrDetections>,
        resolver: Arc<dyn QrResolver>,
        calibration: Calibration,
        gate: QualityGate,
    ) -> Result<Self, BindError> {
        let owner = runtime.peer_id().to_string();
        Self::bind_with_controls(
            runtime,
            component_id,
            output_id,
            detections,
            resolver,
            calibration,
            gate,
            true,
            move |ctx| ctx.caller_peer_id == owner,
        )
    }
    /// Application-controlled admission; starting paused avoids processing buffered detections.
    #[allow(clippy::too_many_arguments)]
    pub fn bind_with_controls(
        runtime: &ComponentRuntime,
        component_id: impl Into<String>,
        output_id: impl Into<String>,
        detections: &RetainedProduct<QrDetections>,
        resolver: Arc<dyn QrResolver>,
        calibration: Calibration,
        gate: QualityGate,
        start_paused: bool,
        authorize: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
    ) -> Result<Self, BindError> {
        calibration.camera()?;
        gate.validate()?;
        if detections.producer.clock_id != calibration.clock_id
            || detections.producer.spatial_frame_id.as_deref() != Some(&calibration.camera_frame_id)
            || detections.producer.payload.schema() != QR_DETECTIONS_SCHEMA
        {
            return Err(LocalizationError::Calibration.into());
        }
        let mut spec = ComponentSpec::new(component_id)
            .product_input(ProductInputContract {
                name: "detections".into(),
                form: ProductForm::Buffer,
                datatype: QrDetections::DATATYPE.into(),
                schema: QR_DETECTIONS_SCHEMA.into(),
                exposure: Exposure::Cluster,
            })
            .observable(ObservableContract {
                name: "poses".into(),
                datatype: LocalizationBatch::DATATYPE.into(),
                schema: LOCALIZATION_SCHEMA.into(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            });
        for name in [PAUSE, RESUME, STATUS] {
            spec = spec.operable(OperableContract {
                name: name.into(),
                instruction: ControlRequest::DATATYPE.into(),
                result: LocalizerStatus::DATATYPE.into(),
                exposure: Exposure::Cluster,
            });
        }
        spec = spec.operable(OperableContract {
            name: LOCALIZE_ONCE.into(),
            instruction: LocalizeOnce::DATATYPE.into(),
            result: LocalizationBatch::DATATYPE.into(),
            exposure: Exposure::Cluster,
        });
        let component = runtime.component(spec)?;
        let poses =
            component.configured_observable::<LocalizationBatch>(ConfiguredObservableSpec::new(
                "poses",
                output_id,
                calibration.clock_id.clone(),
                PayloadContract::Structured(StructuredPayloadContract {
                    modality: "pose".into(),
                    datatype: LocalizationBatch::DATATYPE.into(),
                    schema: LOCALIZATION_SCHEMA.into(),
                    observes: "Camera poses tagged with their resolved anchor and map frame".into(),
                    unit: None,
                }),
            ))?;
        let state = Arc::new(Mutex::new(ControlState {
            status: LocalizerStatus {
                mode: if start_paused {
                    ProcessingMode::Paused
                } else {
                    ProcessingMode::Running
                },
                continuous_processed: 0,
                one_shot_processed: 0,
                skipped: 0,
                last_detection_sequence: None,
            },
            resume_after: None,
        }));
        let processor = Arc::new(Processor {
            resolver,
            calibration,
            gate,
            detections: detections.clone(),
        });
        let authorize = Arc::new(authorize);
        let auth = authorize.clone();
        let control = state.clone();
        let pause = component.operable(
            PAUSE,
            move |ctx| auth(ctx),
            move |_, _: ControlRequest| {
                let mut guard = control
                    .try_lock()
                    .map_err(|_| InvocationError::Rejected("localizer busy".into()))?;
                if guard.status.mode == ProcessingMode::Closed {
                    return Err(InvocationError::TargetUnavailable);
                }
                guard.status.mode = ProcessingMode::Paused;
                Ok(guard.status.clone())
            },
        )?;
        let auth = authorize.clone();
        let control = state.clone();
        let source = detections.clone();
        let resume = component.operable(
            RESUME,
            move |ctx| auth(ctx),
            move |_, _: ControlRequest| {
                let mut guard = control
                    .try_lock()
                    .map_err(|_| InvocationError::Rejected("localizer busy".into()))?;
                if guard.status.mode == ProcessingMode::Closed {
                    return Err(InvocationError::TargetUnavailable);
                }
                if guard.status.mode == ProcessingMode::Paused {
                    guard.resume_after = source.buffer().range().last_sequence;
                    guard.status.mode = ProcessingMode::Running;
                }
                Ok(guard.status.clone())
            },
        )?;
        let auth = authorize.clone();
        let control = state.clone();
        let status = component.operable(
            STATUS,
            move |ctx| auth(ctx),
            move |_, _: ControlRequest| {
                let guard = control
                    .try_lock()
                    .map_err(|_| InvocationError::Rejected("localizer busy".into()))?;
                Ok(guard.status.clone())
            },
        )?;
        let auth = authorize.clone();
        let control = state.clone();
        let worker = processor.clone();
        let localize_once = component.operable(
            LOCALIZE_ONCE,
            move |ctx| auth(ctx),
            move |_, request: LocalizeOnce| {
                // No private work queue: a competing operation is rejected as busy.
                let mut guard = control
                    .try_lock()
                    .map_err(|_| InvocationError::Rejected("localizer busy".into()))?;
                if guard.status.mode == ProcessingMode::Closed {
                    return Err(InvocationError::TargetUnavailable);
                }
                if request.detection_product != worker.detections.reference() {
                    return Err(InvocationError::Rejected(
                        "detection Product mismatch".into(),
                    ));
                }
                let observation = worker
                    .detections
                    .buffer()
                    .snapshot(request.detection_sequence, request.detection_sequence)
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        InvocationError::Rejected("requested detection is not retained".into())
                    })?;
                if observation.payload.sequence != request.detection_sequence {
                    return Err(InvocationError::Rejected(
                        "detection sequence mismatch".into(),
                    ));
                }
                if observation.payload.output != worker.detections.manifest.producer {
                    return Err(InvocationError::Rejected(
                        "detection producer mismatch".into(),
                    ));
                }
                let batch = localize_selected_batch(
                    worker.resolver.as_ref(),
                    &worker.calibration,
                    worker.gate,
                    worker.detections.reference(),
                    &observation.payload,
                    Some((request.detection_index, &request.target_map)),
                );
                if let Some(reason) = &batch.rejection {
                    return Err(InvocationError::Rejected(reason.clone()));
                }
                if let Some(rejected) = batch.rejected_codes.first() {
                    return Err(InvocationError::Rejected(rejected.reason.clone()));
                }
                guard.status.one_shot_processed = guard.status.one_shot_processed.saturating_add(1);
                guard.status.last_detection_sequence = Some(batch.detection_sequence);
                // Historical requests return directly, without disturbing the continuous output clock.
                Ok(batch)
            },
        )?;
        let output = poses.clone();
        let control = state.clone();
        let port = InputPort::try_new(
            "localize",
            move |entry: &Envelope<Observation<QrDetections>>| {
                let mut guard = control
                    .lock()
                    .map_err(|_| "localizer state poisoned".to_string())?;
                let observation = &entry.payload;
                if guard.status.mode != ProcessingMode::Running
                    || guard
                        .resume_after
                        .is_some_and(|seq| observation.sequence <= seq)
                {
                    guard.status.skipped = guard.status.skipped.saturating_add(1);
                    return Ok(());
                }
                let batch = processor.process(observation);
                output
                    .publish(observation.timestamp_ns, Arc::new(batch))
                    .map_err(|e| e.to_string())?;
                guard.status.continuous_processed =
                    guard.status.continuous_processed.saturating_add(1);
                guard.status.last_detection_sequence = Some(observation.sequence);
                Ok(())
            },
        );
        let input = component.configured_buffer_input(
            "detections",
            detections,
            CursorStart::FromSequence(0),
            &port,
        )?;
        component.expose()?;
        Ok(Self {
            input: Some(input),
            component,
            poses,
            state,
            pause,
            resume,
            status,
            localize_once,
        })
    }
    pub fn component(&self) -> &Component {
        &self.component
    }
    pub fn poses(&self) -> &ConfiguredObservable<LocalizationBatch> {
        &self.poses
    }
    pub fn input(&self) -> Option<&ConfiguredBufferInput<QrDetections>> {
        self.input.as_ref()
    }
    pub fn pause(&self) -> &Operable<ControlRequest, LocalizerStatus> {
        &self.pause
    }
    pub fn resume(&self) -> &Operable<ControlRequest, LocalizerStatus> {
        &self.resume
    }
    pub fn status(&self) -> &Operable<ControlRequest, LocalizerStatus> {
        &self.status
    }
    pub fn localize_once(&self) -> &Operable<LocalizeOnce, LocalizationBatch> {
        &self.localize_once
    }
    /// Returns after admitted processing completes; no later continuous work can publish.
    pub fn close(&mut self, timestamp_ns: u64) -> Result<ObservationEnd, PublishError> {
        if let Ok(mut state) = self.state.lock() {
            state.status.mode = ProcessingMode::Closed;
        }
        drop(self.input.take());
        self.poses.end(
            timestamp_ns,
            ObservationEndReason::Reconfigured { replacement: None },
        )
    }
}
impl Drop for QrLocalizerComponent {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.status.mode = ProcessingMode::Closed;
        }
        drop(self.input.take());
    }
}

fn localize_batch(
    resolver: &dyn QrResolver,
    calibration: &Calibration,
    gate: QualityGate,
    detection_product: ProductReference,
    observation: &Observation<QrDetections>,
) -> LocalizationBatch {
    localize_selected_batch(
        resolver,
        calibration,
        gate,
        detection_product,
        observation,
        None,
    )
}
fn localize_selected_batch(
    resolver: &dyn QrResolver,
    calibration: &Calibration,
    gate: QualityGate,
    detection_product: ProductReference,
    observation: &Observation<QrDetections>,
    selection: Option<(usize, &SnapshotReference)>,
) -> LocalizationBatch {
    let data = &observation.payload;
    let mut batch = LocalizationBatch {
        calibration: calibration.clone(),
        detection_product,
        detection_sequence: observation.sequence,
        timestamp_ns: observation.timestamp_ns,
        source_frame: data.source_frame.clone(),
        rejection: None,
        estimates: vec![],
        needs_mapping: vec![],
        rejected_codes: vec![],
    };
    batch.rejection = if data.schema_version != QR_DETECTION_SCHEMA_VERSION {
        Some("unsupported_detection_schema")
    } else if data.codes.len() > 256 {
        Some("too_many_codes")
    } else {
        match &data.source_frame {
            None => Some("missing_camera_provenance"),
            Some(source) if source.camera_product != calibration.camera_product => {
                Some("camera_product_mismatch")
            }
            Some(source) if source.timestamp_ns != observation.timestamp_ns => {
                Some("camera_timestamp_mismatch")
            }
            _ => None,
        }
    }
    .map(str::to_owned);
    if batch.rejection.is_some() {
        return batch;
    }
    if selection.is_some_and(|(index, _)| index >= data.codes.len()) {
        batch.rejection = Some("detection index out of range".into());
        return batch;
    }
    for (index, code) in data.codes.iter().enumerate() {
        if selection.is_some_and(|(selected, _)| selected != index) {
            continue;
        }
        let resolved = if code.mirrored {
            Err("mirrored_qr_unsupported".to_string())
        } else if let Some((_, target)) = selection {
            resolver
                .resolve_in(&code.payload, target)
                .and_then(|qr| {
                    if &qr.snapshot != target {
                        return Err(auki_scenegraph::resolution::ResolutionError(
                            "resolver returned another map revision".into(),
                        ));
                    }
                    Ok(vec![qr])
                })
                .map_err(|e| e.to_string())
        } else {
            resolver.resolve(&code.payload).map_err(|e| e.to_string())
        };
        let anchors = match resolved {
            Err(reason) => {
                batch.rejected_codes.push(RejectedCode { index, reason });
                continue;
            }
            Ok(anchors) if anchors.is_empty() => {
                batch.needs_mapping.push(index);
                continue;
            }
            Ok(anchors) => anchors,
        };
        // Keep arbitrary resolver implementations bounded and reject ambiguous per-map answers.
        let mut maps = std::collections::BTreeSet::new();
        if anchors.len() > 64
            || anchors.iter().any(|qr| {
                qr.anchor.payload != code.payload
                    || qr.validate().is_err()
                    || !maps.insert((
                        qr.snapshot.product.peer_id.clone(),
                        qr.snapshot.product.product_id.clone(),
                    ))
            })
        {
            batch.rejected_codes.push(RejectedCode {
                index,
                reason: "invalid_or_ambiguous_anchor_resolution".into(),
            });
            continue;
        }
        let corners = code
            .refined_corners_px
            .as_ref()
            .unwrap_or(&code.corners_px)
            .map(|p| [p.x, p.y]);
        for qr in anchors {
            match estimate_camera_pose(&qr.map, &qr.anchor, corners, calibration, gate) {
                Ok(pose) => batch.estimates.push(LocalizedQr {
                    detection_index: index,
                    qr,
                    pose,
                }),
                Err(error) => batch.rejected_codes.push(RejectedCode {
                    index,
                    reason: error.to_string(),
                }),
            }
        }
    }
    batch
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone)]
    struct FixtureResolver(Vec<ResolvedQr>);
    impl QrResolver for FixtureResolver {
        fn resolve(
            &self,
            payload: &str,
        ) -> Result<Vec<ResolvedQr>, auki_scenegraph::resolution::ResolutionError> {
            Ok(self
                .0
                .iter()
                .filter(|r| r.anchor.payload == payload)
                .cloned()
                .collect())
        }
    }
    fn fixture() -> (FixtureResolver, Calibration, Observation<QrDetections>) {
        let product: ProductReference = serde_json::from_value(
            serde_json::json!({"peer_id":"p","product_id":"camera","manifest_hash":"hash"}),
        )
        .unwrap();
        let calibration = Calibration {
            id: "c".into(),
            camera_product: product.clone(),
            camera_frame_id: "optical".into(),
            clock_id: "clock".into(),
            width: 640,
            height: 480,
            fx: 500.,
            fy: 500.,
            cx: 320.,
            cy: 240.,
            distortion: Distortion::None,
        };
        let mut scene = Scenegraph::new(auki_scenegraph::MapDefinition {
            map_id: "m".into(),
            name: None,
            domain_reference: None,
            frame: auki_scenegraph::MapFrame::z_up_meters("map", "origin"),
        })
        .unwrap();
        scene.anchors.insert(
            "qr".into(),
            auki_scenegraph::QrAnchor {
                anchor_id: "qr".into(),
                payload: "payload".into(),
                side_length_m: 0.2,
                pose_in_map: RigidTransform::identity("qr-frame", "map"),
            },
        );
        let map = FixtureResolver(vec![ResolvedQr {
            snapshot: auki_scenegraph::component::SnapshotReference {
                product: product.clone(),
                sequence: 3,
            },
            map: scene.map.clone(),
            anchor: scene.anchors["qr"].clone(),
        }]);
        let output=serde_json::from_value(serde_json::json!({"peer_id":"p","component_id":"detector","component_manifest_hash":"hash","slot":"detections","output_id":"out","manifest_hash":"hash"})).unwrap();
        let data = QrDetections {
            schema_version: QR_DETECTION_SCHEMA_VERSION,
            source_frame: Some(QrSourceFrame {
                camera_product: product,
                sequence: 8,
                timestamp_ns: 100,
            }),
            codes: vec![auki_qr_detector::QrDetection {
                payload: "payload".into(),
                version: 1,
                ecc: 'M',
                mirrored: false,
                inverted: false,
                corners_px: [[270., 190.], [370., 190.], [370., 290.], [270., 290.]]
                    .map(|[x, y]| auki_qr_detector::PixelCorner { x, y }),
                refined_corners_px: None,
                scanner_stage: 0,
            }],
        };
        (
            map,
            calibration,
            Observation {
                output,
                sequence: 2,
                timestamp_ns: 100,
                payload: Arc::new(data),
            },
        )
    }
    #[test]
    fn one_detection_can_localize_in_several_independent_map_frames() {
        let (mut resolver, cal, obs) = fixture();
        let mut second = resolver.0[0].clone();
        second.snapshot.product.product_id = "other-map".into();
        second.map.map_id = "other-map".into();
        second.map.frame.id = "other-frame".into();
        second.anchor.pose_in_map.to_frame_id = "other-frame".into();
        second.anchor.pose_in_map.translation = [10.0, 0.0, 0.0];
        resolver.0.push(second);
        let batch = localize_batch(
            &resolver,
            &cal,
            QualityGate::default(),
            cal.camera_product.clone(),
            &obs,
        );
        assert_eq!(batch.estimates.len(), 2);
        assert_ne!(
            batch.estimates[0].qr.map.frame.id,
            batch.estimates[1].qr.map.frame.id
        );
        assert!(
            (batch.estimates[1].pose.camera_pose_in_map.translation[0]
                - batch.estimates[0].pose.camera_pose_in_map.translation[0]
                - 10.0)
                .abs()
                < 1e-6
        );
        assert!(batch.needs_mapping.is_empty());
    }

    #[test]
    fn rejects_wrong_or_missing_camera_provenance_and_schema() {
        for case in 0..4 {
            let (map, cal, mut obs) = fixture();
            let data = Arc::make_mut(&mut obs.payload);
            match case {
                0 => data.source_frame = None,
                1 => {
                    data.source_frame
                        .as_mut()
                        .unwrap()
                        .camera_product
                        .product_id = "replacement".into()
                }
                2 => data.source_frame.as_mut().unwrap().timestamp_ns = 99,
                _ => data.schema_version = 99,
            };
            let batch = localize_batch(
                &map,
                &cal,
                QualityGate::default(),
                cal.camera_product.clone(),
                &obs,
            );
            assert!(batch.rejection.is_some());
            assert!(batch.estimates.is_empty());
        }
    }
    #[test]
    fn rejects_unknown_duplicate_and_mirrored_markers() {
        for case in 0..3 {
            let (mut map, cal, mut obs) = fixture();
            match case {
                0 => Arc::make_mut(&mut obs.payload).codes[0].payload = "unknown".into(),
                1 => {
                    let mut duplicate = map.0[0].clone();
                    duplicate.anchor.anchor_id = "duplicate".into();
                    map.0.push(duplicate);
                }
                _ => Arc::make_mut(&mut obs.payload).codes[0].mirrored = true,
            }
            let batch = localize_batch(
                &map,
                &cal,
                QualityGate::default(),
                cal.camera_product.clone(),
                &obs,
            );
            assert!(batch.rejection.is_none());
            assert!(batch.estimates.is_empty());
            if case == 0 {
                assert_eq!(batch.needs_mapping, vec![0]);
            } else {
                assert_eq!(batch.rejected_codes.len(), 1);
            }
        }
    }
    fn ctx() -> InvocationContext {
        InvocationContext {
            invocation_id: "test".into(),
            caller_peer_id: "owner".into(),
            caller_component_id: "controller".into(),
        }
    }
    struct Controlled {
        _runtime: ComponentRuntime,
        _source_component: Component,
        source: ConfiguredObservable<QrDetections>,
        detections: BufferProductCapture<QrDetections>,
        localizer: QrLocalizerComponent,
        output: BufferProductCapture<LocalizationBatch>,
        data: QrDetections,
    }
    fn controlled(start_paused: bool) -> Controlled {
        let (mut resolver, cal, observation) = fixture();
        let mut second = resolver.0[0].clone();
        second.snapshot.product.product_id = "second-map".into();
        second.map.map_id = "second-map".into();
        second.map.frame.id = "second-frame".into();
        second.anchor.pose_in_map.to_frame_id = "second-frame".into();
        second.anchor.pose_in_map.translation = [10., 0., 0.];
        resolver.0.push(second);
        let runtime = ComponentRuntime::new("owner");
        let c = runtime
            .component(
                ComponentSpec::new("detector").observable(ObservableContract {
                    name: "detections".into(),
                    datatype: QrDetections::DATATYPE.into(),
                    schema: QR_DETECTIONS_SCHEMA.into(),
                    access: vec![ObservationAccess::FollowNew],
                    exposure: Exposure::Cluster,
                }),
            )
            .unwrap();
        let source = c
            .configured_observable(
                ConfiguredObservableSpec::new(
                    "detections",
                    "detections-run",
                    "clock",
                    PayloadContract::Structured(StructuredPayloadContract {
                        modality: "qr".into(),
                        datatype: QrDetections::DATATYPE.into(),
                        schema: QR_DETECTIONS_SCHEMA.into(),
                        observes: "camera".into(),
                        unit: None,
                    }),
                )
                .in_spatial_frame("optical"),
            )
            .unwrap();
        c.expose().unwrap();
        let detections = runtime
            .capture_buffer(
                "detections-product",
                &source,
                BufferLimits::entries(2),
                |_| 4096,
            )
            .unwrap();
        let localizer = if start_paused {
            QrLocalizerComponent::bind(
                &runtime,
                "localizer",
                "poses-run",
                &detections.product(),
                Arc::new(resolver),
                cal,
                QualityGate::default(),
            )
        } else {
            QrLocalizerComponent::bind_with_controls(
                &runtime,
                "localizer",
                "poses-run",
                &detections.product(),
                Arc::new(resolver),
                cal,
                QualityGate::default(),
                false,
                |c| c.caller_peer_id == "owner" && c.caller_component_id == "controller",
            )
        }
        .unwrap();
        let output = runtime
            .capture_buffer(
                "pose-product",
                localizer.poses(),
                BufferLimits::entries(8),
                |_| 4096,
            )
            .unwrap();
        Controlled {
            _runtime: runtime,
            _source_component: c,
            source,
            detections,
            localizer,
            output,
            data: (*observation.payload).clone(),
        }
    }
    fn emit(f: &mut Controlled, timestamp: u64) {
        let source = f.data.source_frame.as_mut().unwrap();
        source.timestamp_ns = timestamp;
        source.sequence = timestamp;
        f.source
            .publish(timestamp, Arc::new(f.data.clone()))
            .unwrap();
    }
    fn drained(f: &Controlled, delivered: u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while f.localizer.input().unwrap().stats().delivered < delivered {
            assert!(std::time::Instant::now() < deadline, "input did not drain");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn one(f: &Controlled, sequence: u64) -> LocalizeOnce {
        LocalizeOnce {
            detection_product: f.detections.product().reference(),
            detection_sequence: sequence,
            detection_index: 0,
            target_map: fixture().0.0[0].snapshot.clone(),
        }
    }
    #[test]
    fn paused_one_shot_uses_exact_observation_without_publishing_or_changing_mode() {
        let mut f = controlled(true);
        emit(&mut f, 100);
        emit(&mut f, 200);
        drained(&f, 2);
        assert!(f.output.product().latest_existing().unwrap().is_none());
        let response = InMemoryTransport
            .invoke(f.localizer.localize_once(), ctx(), one(&f, 0))
            .unwrap()
            .result;
        assert_eq!(response.detection_sequence, 0);
        assert_eq!(response.timestamp_ns, 100);
        assert_eq!(response.source_frame.as_ref().unwrap().timestamp_ns, 100);
        assert_eq!(response.estimates.len(), 1);
        assert!(f.output.product().latest_existing().unwrap().is_none());
        let status = InMemoryTransport
            .invoke(f.localizer.status(), ctx(), ControlRequest {})
            .unwrap()
            .result;
        assert_eq!(status.mode, ProcessingMode::Paused);
        assert_eq!(status.one_shot_processed, 1);
        let mut wrong = one(&f, 1);
        wrong.detection_product.manifest_hash = "other".into();
        assert!(
            InMemoryTransport
                .invoke(f.localizer.localize_once(), ctx(), wrong)
                .is_err()
        );
        assert!(
            InMemoryTransport
                .invoke(f.localizer.localize_once(), ctx(), one(&f, 99))
                .is_err()
        );
        emit(&mut f, 300);
        drained(&f, 3);
        assert!(
            InMemoryTransport
                .invoke(f.localizer.localize_once(), ctx(), one(&f, 0))
                .is_err()
        );
        f.localizer.close(300).unwrap();
        assert!(
            InMemoryTransport
                .invoke(f.localizer.localize_once(), ctx(), one(&f, 2))
                .is_err()
        );
        assert!(
            InMemoryTransport
                .invoke(f.localizer.resume(), ctx(), ControlRequest {})
                .is_err()
        );
    }
    #[test]
    fn controls_are_authorized_and_resume_only_processes_future_observations() {
        let mut f = controlled(false);
        emit(&mut f, 100);
        drained(&f, 1);
        assert_eq!(
            f.output
                .product()
                .latest_existing()
                .unwrap()
                .unwrap()
                .payload
                .detection_sequence,
            0
        );
        let mut denied = ctx();
        denied.caller_peer_id = "stranger".into();
        assert!(matches!(
            InMemoryTransport.invoke(f.localizer.pause(), denied.clone(), ControlRequest {}),
            Err(InvocationError::Unauthorized)
        ));
        assert!(matches!(
            InMemoryTransport.invoke(f.localizer.localize_once(), denied, one(&f, 0)),
            Err(InvocationError::Unauthorized)
        ));
        assert_eq!(
            InMemoryTransport
                .invoke(f.localizer.pause(), ctx(), ControlRequest {})
                .unwrap()
                .result
                .mode,
            ProcessingMode::Paused
        );
        emit(&mut f, 200);
        drained(&f, 2);
        assert_eq!(
            f.output
                .product()
                .latest_existing()
                .unwrap()
                .unwrap()
                .payload
                .detection_sequence,
            0
        );
        InMemoryTransport
            .invoke(f.localizer.resume(), ctx(), ControlRequest {})
            .unwrap();
        emit(&mut f, 300);
        drained(&f, 3);
        assert_eq!(
            f.output
                .product()
                .latest_existing()
                .unwrap()
                .unwrap()
                .payload
                .detection_sequence,
            2
        );
        let count = f.output.product().buffer().range().entries;
        let historic = InMemoryTransport
            .invoke(f.localizer.localize_once(), ctx(), one(&f, 1))
            .unwrap()
            .result;
        assert_eq!(historic.timestamp_ns, 200);
        assert_eq!(f.output.product().buffer().range().entries, count);
        assert_eq!(
            f.output
                .product()
                .latest_existing()
                .unwrap()
                .unwrap()
                .timestamp_ns,
            300
        );
        let status = InMemoryTransport
            .invoke(f.localizer.status(), ctx(), ControlRequest {})
            .unwrap()
            .result;
        assert_eq!(status.continuous_processed, 2);
        assert_eq!(status.skipped, 1);
        let names: Vec<_> = f
            .localizer
            .component()
            .manifest()
            .operables
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert!(
            names.contains(&PAUSE) && names.contains(&RESUME) && names.contains(&LOCALIZE_ONCE)
        );
    }
    #[test]
    fn competing_calls_are_busy_and_retained_handles_cannot_process_after_drop() {
        let mut f = controlled(true);
        emit(&mut f, 100);
        drained(&f, 1);
        let guard = f.localizer.state.lock().unwrap();
        assert!(matches!(
            InMemoryTransport.invoke(f.localizer.localize_once(), ctx(), one(&f, 0)),
            Err(InvocationError::Rejected(_))
        ));
        assert!(matches!(
            InMemoryTransport.invoke(f.localizer.pause(), ctx(), ControlRequest {}),
            Err(InvocationError::Rejected(_))
        ));
        drop(guard);
        let op = f.localizer.localize_once().clone();
        let request = one(&f, 0);
        drop(f.localizer);
        assert!(matches!(
            InMemoryTransport.invoke(&op, ctx(), request),
            Err(InvocationError::TargetUnavailable)
        ));
    }
    #[test]
    fn one_shot_selects_one_qr_and_exact_map_without_fallback() {
        let mut f = controlled(true);
        let known = f.data.codes[0].clone();
        f.data.codes[0].payload = "unknown".into();
        f.data.codes.push(known);
        emit(&mut f, 100);
        drained(&f, 1);
        assert!(f.output.product().latest_existing().unwrap().is_none());
        let mut request = one(&f, 0);
        request.detection_index = 1;
        let first = InMemoryTransport
            .invoke(f.localizer.localize_once(), ctx(), request.clone())
            .unwrap()
            .result;
        assert_eq!(first.estimates.len(), 1);
        assert_eq!(first.estimates[0].detection_index, 1);
        assert_eq!(
            first.estimates[0].pose.camera_pose_in_map.to_frame_id,
            "map"
        );
        assert!(first.needs_mapping.is_empty());
        request.target_map.product.product_id = "second-map".into();
        let second = InMemoryTransport
            .invoke(f.localizer.localize_once(), ctx(), request.clone())
            .unwrap()
            .result;
        assert_eq!(second.estimates.len(), 1);
        assert_eq!(second.estimates[0].qr.snapshot, request.target_map);
        assert_eq!(
            second.estimates[0].pose.camera_pose_in_map.to_frame_id,
            "second-frame"
        );
        assert!(
            (second.estimates[0].pose.camera_pose_in_map.translation[0]
                - first.estimates[0].pose.camera_pose_in_map.translation[0]
                - 10.)
                .abs()
                < 1e-6
        );
        for case in 0..5 {
            let mut bad = request.clone();
            match case {
                0 => bad.target_map.sequence += 1,
                1 => bad.target_map.product.manifest_hash = "other".into(),
                2 => bad.target_map.product.product_id = "absent".into(),
                3 => bad.detection_index = 99,
                _ => bad.detection_index = 0,
            }
            assert!(
                InMemoryTransport
                    .invoke(f.localizer.localize_once(), ctx(), bad)
                    .is_err()
            );
        }
        assert!(f.output.product().latest_existing().unwrap().is_none());
    }
}
