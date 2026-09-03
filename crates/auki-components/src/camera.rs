use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::component::{
    CameraPayloadContract, Catalog, CatalogError, ComponentManifest, ComponentReference, Exposure,
    InvocationError, Observable, ObservableContract, Observation, ObservationAccess,
    ObservationDelivery, ObservationEmitter, ObservationEnd, ObservationEndReason,
    ObservationError, ObservationEvent, ObservationHandle, Operable, OperableContract,
    OutputManifest, OutputReference, PayloadContract, ProductForm, ProductManifest,
    observation_input, output_observable,
};
use crate::{Buffer, BufferError, ContractType, Envelope, RetainedProduct};

const FRAMES_SLOT: &str = "frames";
const RGB8_ENCODING: &str = "rgb8";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub encoding: String,
    pub bytes: Arc<[u8]>,
}

impl ContractType for VideoFrame {
    const DATATYPE: &'static str = "video_frame";
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetResolution {
    pub width: u32,
    pub height: u32,
    pub effective_at_timestamp_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppliedResolution {
    pub changed: bool,
    pub component: ComponentReference,
    pub previous_output: OutputReference,
    pub replacement_output: OutputReference,
    pub previous_last_sequence: Option<u64>,
    pub effective_at_timestamp_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReseedDriver;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DriverReseeded {
    pub reset_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CameraError {
    InvalidResolution,
    FrameContractMismatch {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
        expected_bytes: usize,
        actual_bytes: usize,
    },
    OutputFailed(String),
    Catalog(CatalogError),
}

impl fmt::Display for CameraError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResolution => formatter.write_str("camera resolution must be positive"),
            Self::FrameContractMismatch {
                expected_width,
                expected_height,
                actual_width,
                actual_height,
                expected_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "frame contract mismatch: expected {expected_width}x{expected_height} RGB8 \
                 ({expected_bytes} bytes), got {actual_width}x{actual_height} \
                 ({actual_bytes} bytes)"
            ),
            Self::OutputFailed(reason) => write!(formatter, "camera Output failed: {reason}"),
            Self::Catalog(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CameraError {}

impl From<CatalogError> for CameraError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

struct ActiveCameraOutput {
    manifest: OutputManifest,
    reference: OutputReference,
    observable: Observable<VideoFrame>,
    emitter: ObservationEmitter<VideoFrame>,
    next_sequence: u64,
    failure: Option<String>,
}

impl ActiveCameraOutput {
    fn new(component: &ComponentReference, generation: u64, width: u32, height: u32) -> Self {
        let manifest = OutputManifest {
            schema: "auki.component-output-manifest/v1".to_owned(),
            peer_id: component.peer_id.clone(),
            component_id: component.component_id.clone(),
            component_manifest_hash: component.manifest_hash.clone(),
            slot: FRAMES_SLOT.to_owned(),
            output_id: format!("frames-{generation}"),
            clock_id: format!("{}.session-clock", component.peer_id),
            spatial_frame_id: Some(format!("{}.optical-frame", component.component_id)),
            payload: PayloadContract::Camera(CameraPayloadContract {
                datatype: "video_frame".to_owned(),
                schema: "auki.video-frame/v1".to_owned(),
                encoding: RGB8_ENCODING.to_owned(),
                width,
                height,
                nominal_frame_rate_hz: None,
                observes: "visible_light".to_owned(),
            }),
        };
        let reference = manifest.reference();
        let (observable, emitter) =
            output_observable(reference.clone(), vec![ObservationAccess::FollowNew]);
        Self {
            manifest,
            reference,
            observable,
            emitter,
            next_sequence: 0,
            failure: None,
        }
    }

    fn width(&self) -> u32 {
        let PayloadContract::Camera(contract) = &self.manifest.payload else {
            unreachable!("Camera Output always has a Camera payload contract")
        };
        contract.width
    }

    fn height(&self) -> u32 {
        let PayloadContract::Camera(contract) = &self.manifest.payload else {
            unreachable!("Camera Output always has a Camera payload contract")
        };
        contract.height
    }
}

struct CameraState {
    generation: u64,
    active: ActiveCameraOutput,
    reset_count: u64,
}

struct CameraInner {
    component_manifest: ComponentManifest,
    component_reference: ComponentReference,
    catalog: Catalog,
    allowed_remote_peers: BTreeSet<String>,
    state: Mutex<CameraState>,
}

/// A stable Camera Component whose configured `frames` Output can be replaced.
#[derive(Clone)]
pub struct CameraComponent {
    inner: Arc<CameraInner>,
}

impl fmt::Debug for CameraComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CameraComponent")
            .field("component", &self.inner.component_reference)
            .field("current_output", &self.current_output_reference())
            .finish_non_exhaustive()
    }
}

impl CameraComponent {
    pub fn new(
        peer_id: impl Into<String>,
        component_id: impl Into<String>,
        width: u32,
        height: u32,
        catalog: Catalog,
        allowed_remote_peers: impl IntoIterator<Item = String>,
    ) -> Result<Self, CameraError> {
        if width == 0 || height == 0 {
            return Err(CameraError::InvalidResolution);
        }
        let peer_id = peer_id.into();
        let component_id = component_id.into();
        let component_manifest = ComponentManifest {
            schema: "auki.component-manifest/v1".to_owned(),
            peer_id: peer_id.clone(),
            component_id: component_id.clone(),
            product_inputs: vec![],
            observables: vec![ObservableContract {
                name: FRAMES_SLOT.to_owned(),
                datatype: "video_frame".to_owned(),
                schema: "auki.video-frame/v1".to_owned(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            }],
            operables: vec![OperableContract {
                name: "set_resolution".to_owned(),
                instruction: "camera_resolution".to_owned(),
                result: "applied_camera_resolution".to_owned(),
                exposure: Exposure::Cluster,
            }],
        };
        let component_reference = component_manifest.reference();
        let active = ActiveCameraOutput::new(&component_reference, 1, width, height);

        catalog.register_component(component_manifest.clone())?;
        catalog.set_current_output(active.manifest.clone())?;

        Ok(Self {
            inner: Arc::new(CameraInner {
                component_manifest,
                component_reference,
                catalog,
                allowed_remote_peers: allowed_remote_peers.into_iter().collect(),
                state: Mutex::new(CameraState {
                    generation: 1,
                    active,
                    reset_count: 0,
                }),
            }),
        })
    }

    pub fn component_manifest(&self) -> &ComponentManifest {
        &self.inner.component_manifest
    }

    pub fn component_reference(&self) -> &ComponentReference {
        &self.inner.component_reference
    }

    pub fn current_output(&self) -> Observable<VideoFrame> {
        self.inner.state.lock().unwrap().active.observable.clone()
    }

    pub fn current_output_manifest(&self) -> OutputManifest {
        self.inner.state.lock().unwrap().active.manifest.clone()
    }

    pub fn current_output_reference(&self) -> OutputReference {
        self.inner.state.lock().unwrap().active.reference.clone()
    }

    pub fn set_resolution_operable(&self) -> Operable<SetResolution, AppliedResolution> {
        let weak = Arc::downgrade(&self.inner);
        let owner_peer_id = self.inner.component_reference.peer_id.clone();
        let allowed_remote_peers = self.inner.allowed_remote_peers.clone();
        Operable::new(
            "set_resolution",
            self.inner.component_reference.clone(),
            Exposure::Cluster,
            move |context| {
                context.caller_peer_id == owner_peer_id
                    || allowed_remote_peers.contains(&context.caller_peer_id)
            },
            move |_context, instruction| {
                let inner = weak.upgrade().ok_or(InvocationError::TargetUnavailable)?;
                apply_resolution(&inner, instruction)
                    .map_err(|error| InvocationError::Rejected(error.to_string()))
            },
        )
    }

    /// A real local Component-to-Component Operable that is intentionally not
    /// included in the cluster Component Manifest or Catalog.
    pub fn local_reseed_operable(&self) -> Operable<ReseedDriver, DriverReseeded> {
        let weak = Arc::downgrade(&self.inner);
        let owner_peer_id = self.inner.component_reference.peer_id.clone();
        Operable::new(
            "reseed_driver",
            self.inner.component_reference.clone(),
            Exposure::Local,
            move |context| context.caller_peer_id == owner_peer_id,
            move |_context, _instruction| {
                let inner = weak.upgrade().ok_or(InvocationError::TargetUnavailable)?;
                let mut state = inner.state.lock().unwrap();
                state.reset_count += 1;
                Ok(DriverReseeded {
                    reset_count: state.reset_count,
                })
            },
        )
    }

    pub fn publish_rgb8(
        &self,
        timestamp_ns: u64,
        width: u32,
        height: u32,
        bytes: Arc<[u8]>,
    ) -> Result<Observation<VideoFrame>, CameraError> {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(reason) = &state.active.failure {
            return Err(CameraError::OutputFailed(reason.clone()));
        }
        let expected_width = state.active.width();
        let expected_height = state.active.height();
        let expected_bytes = rgb8_len(expected_width, expected_height)?;
        if width != expected_width || height != expected_height || bytes.len() != expected_bytes {
            return Err(CameraError::FrameContractMismatch {
                expected_width,
                expected_height,
                actual_width: width,
                actual_height: height,
                expected_bytes,
                actual_bytes: bytes.len(),
            });
        }

        let frame = Arc::new(VideoFrame {
            width,
            height,
            encoding: RGB8_ENCODING.to_owned(),
            bytes,
        });
        let observation = Observation {
            output: state.active.reference.clone(),
            sequence: state.active.next_sequence,
            timestamp_ns,
            payload: frame,
        };
        state.active.next_sequence += 1;

        state.active.emitter.emit(
            timestamp_ns,
            ObservationEvent::Observation(observation.clone()),
        );
        Ok(observation)
    }

    /// Reports a terminal failure for the configured Output. Existing
    /// observations remain valid, but this Output emits no further frames.
    pub fn fail_current_output(&self, timestamp_ns: u64, reason: impl Into<String>) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        if state.active.failure.is_some() {
            return false;
        }
        let reason = reason.into();
        state.active.failure = Some(reason.clone());
        let event = ObservationEvent::Ended(ObservationEnd {
            output: state.active.reference.clone(),
            last_sequence: state.active.next_sequence.checked_sub(1),
            timestamp_ns,
            reason: ObservationEndReason::Failed { reason },
        });
        state.active.emitter.emit(timestamp_ns, event);
        true
    }
}

fn apply_resolution(
    inner: &Arc<CameraInner>,
    instruction: SetResolution,
) -> Result<AppliedResolution, CameraError> {
    if instruction.width == 0 || instruction.height == 0 {
        return Err(CameraError::InvalidResolution);
    }

    let mut state = inner.state.lock().unwrap();
    let previous = state.active.reference.clone();
    let previous_last_sequence = state.active.next_sequence.checked_sub(1);
    if state.active.width() == instruction.width && state.active.height() == instruction.height {
        return Ok(AppliedResolution {
            changed: false,
            component: inner.component_reference.clone(),
            previous_output: previous.clone(),
            replacement_output: previous,
            previous_last_sequence,
            effective_at_timestamp_ns: instruction.effective_at_timestamp_ns,
        });
    }

    let next_generation = state.generation + 1;
    let replacement = ActiveCameraOutput::new(
        &inner.component_reference,
        next_generation,
        instruction.width,
        instruction.height,
    );
    inner
        .catalog
        .set_current_output(replacement.manifest.clone())?;

    let end = ObservationEnd {
        output: previous.clone(),
        last_sequence: previous_last_sequence,
        timestamp_ns: instruction.effective_at_timestamp_ns,
        reason: ObservationEndReason::Reconfigured {
            replacement: Some(replacement.reference.clone()),
        },
    };
    state.active.emitter.emit(
        instruction.effective_at_timestamp_ns,
        ObservationEvent::Ended(end),
    );

    let replacement_reference = replacement.reference.clone();
    state.generation = next_generation;
    state.active = replacement;

    Ok(AppliedResolution {
        changed: true,
        component: inner.component_reference.clone(),
        previous_output: previous,
        replacement_output: replacement_reference,
        previous_last_sequence,
        effective_at_timestamp_ns: instruction.effective_at_timestamp_ns,
    })
}

fn rgb8_len(width: u32, height: u32) -> Result<usize, CameraError> {
    let pixels = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(CameraError::InvalidResolution)?;
    Ok(pixels)
}

pub type CameraProductBuffer = RetainedProduct<VideoFrame>;

struct CameraBufferState {
    product: CameraProductBuffer,
    end: Option<ObservationEnd>,
    errors: Vec<String>,
}

#[derive(Debug)]
pub enum CameraBufferError {
    Buffer(BufferError),
    Catalog(CatalogError),
    Observation(ObservationError),
}

impl fmt::Display for CameraBufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buffer(error) => error.fmt(formatter),
            Self::Catalog(error) => error.fmt(formatter),
            Self::Observation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CameraBufferError {}

impl From<BufferError> for CameraBufferError {
    fn from(error: BufferError) -> Self {
        Self::Buffer(error)
    }
}

impl From<ObservationError> for CameraBufferError {
    fn from(error: ObservationError) -> Self {
        Self::Observation(error)
    }
}

impl From<CatalogError> for CameraBufferError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

/// Retains observations from exactly one configured Camera Output.
///
/// Reconfiguration closes this Buffer and ends its subscription. Recording a
/// replacement Output requires an explicit new attachment.
pub struct CameraBufferCapture {
    state: Arc<Mutex<CameraBufferState>>,
    _observation: ObservationHandle<VideoFrame>,
}

impl CameraBufferCapture {
    pub fn attach(camera: &CameraComponent, max_entries: usize) -> Result<Self, CameraBufferError> {
        let catalog = camera.inner.catalog.clone();
        let (output, observable) = {
            let camera_state = camera.inner.state.lock().unwrap();
            (
                camera_state.active.manifest.clone(),
                camera_state.active.observable.clone(),
            )
        };
        let product = create_product_buffer(&output, &catalog, max_entries)?;
        let state = Arc::new(Mutex::new(CameraBufferState {
            product,
            end: None,
            errors: Vec::new(),
        }));

        let input_state = Arc::clone(&state);
        let input = observation_input("camera-buffer", move |event| {
            let mut state = input_state.lock().unwrap();
            match event {
                ObservationEvent::Observation(observation) => {
                    let producer = state.product.manifest.producer.clone();
                    if producer != observation.output {
                        state.errors.push(format!(
                            "observation from {} reached Buffer for {}",
                            observation.output.output_id, producer.output_id
                        ));
                        return;
                    }
                    if let Err(error) = state.product.buffer.append_shared(Arc::new(Envelope::new(
                        observation.sequence,
                        observation.timestamp_ns,
                        observation.clone(),
                    ))) {
                        state.errors.push(error.to_string());
                    }
                }
                ObservationEvent::Ended(end) => {
                    let producer = state.product.manifest.producer.clone();
                    if producer != end.output {
                        state.errors.push(format!(
                            "end notice from {} reached Buffer for {}",
                            end.output.output_id, producer.output_id
                        ));
                        return;
                    }
                    state.product.buffer.close();
                    state.end = Some(end.clone());
                }
            }
        });
        let observation =
            observable.follow_new(&input, ObservationDelivery::inline_every_selected())?;

        Ok(Self {
            state,
            _observation: observation,
        })
    }

    pub fn product(&self) -> CameraProductBuffer {
        self.state.lock().unwrap().product.clone()
    }

    pub fn end_notice(&self) -> Option<ObservationEnd> {
        self.state.lock().unwrap().end.clone()
    }

    pub fn errors(&self) -> Vec<String> {
        self.state.lock().unwrap().errors.clone()
    }
}

fn create_product_buffer(
    output: &OutputManifest,
    catalog: &Catalog,
    max_entries: usize,
) -> Result<CameraProductBuffer, CameraBufferError> {
    let output_reference = output.reference();
    let product_id = format!(
        "{}.{}.buffer",
        output_reference.component_id, output_reference.output_id
    );
    let manifest = ProductManifest {
        schema: "auki.product-manifest/v1".to_owned(),
        peer_id: output_reference.peer_id.clone(),
        product_id: product_id.clone(),
        form: ProductForm::Buffer,
        producer: output_reference,
        access: vec![
            ObservationAccess::LatestExisting,
            ObservationAccess::TimeRange,
        ],
    };
    let manifest_hash = manifest.hash();
    let buffer = Buffer::with_limits(
        product_id,
        crate::BufferLimits::entries(max_entries),
        |observation: &Observation<VideoFrame>| observation.payload.bytes.len(),
    )?;
    catalog.register_product(manifest.clone())?;
    Ok(RetainedProduct {
        manifest,
        manifest_hash,
        producer: output.clone(),
        buffer,
    })
}
