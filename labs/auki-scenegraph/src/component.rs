//! Map state owner. Network exports are explicitly mounted by the host application.

use crate::catalog::MapCatalogData;
use crate::{MapDefinition, MapSnapshot, QrAnchor, SNAPSHOT_SCHEMA, Scenegraph};
use auki_components::{
    BufferLimits, BufferProductCapture, Component, ComponentRuntime, ComponentSpec,
    ConfiguredObservable, ConfiguredObservableSpec, ContractType, Exposure, InvocationContext,
    InvocationError, ObservableContract, Observation, ObservationAccess, Operable,
    OperableContract, PayloadContract, ProductReference, RetainedProduct,
    StructuredPayloadContract,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub const LOOKUP_QR: &str = "lookup_qr";
pub const FIND_QR: &str = "find_qr";
pub const REMOVE_QR: &str = "remove_qr";

pub const UPSERT_QR: &str = "upsert_qr";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotReference {
    pub product: ProductReference,
    pub sequence: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpsertQr {
    /// Compare-and-set prevents a stale mapper from silently overwriting newer state.
    pub expected_snapshot: SnapshotReference,
    pub anchor: QrAnchor,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoveQr {
    pub expected_snapshot: SnapshotReference,
    pub anchor_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupQr {
    pub anchor_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupQrResult {
    pub snapshot: SnapshotReference,
    pub map: MapDefinition,
    /// None explicitly means not found in this snapshot. The pose is in the map root frame.
    pub anchor: Option<QrAnchor>,
}

/// Lookup by the exact decoded payload; multiple matches are explicitly ambiguous.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FindQr {
    pub payload: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FindQrResult {
    pub snapshot: SnapshotReference,
    pub map: MapDefinition,
    pub anchors: Vec<QrAnchor>,
}

macro_rules! contract {
    ($t:ty, $name:literal) => {
        impl ContractType for $t {
            const DATATYPE: &'static str = $name;
        }
    };
}
contract!(MapSnapshot, "auki.scenegraph.qr-snapshot/v2");
contract!(RemoveQr, "auki.scenegraph.remove-qr/v1");
contract!(UpsertQr, "auki.scenegraph.upsert-qr/v2");
contract!(FindQr, "auki.scenegraph.find-qr/v1");
contract!(FindQrResult, "auki.scenegraph.find-qr-result/v2");
contract!(LookupQr, "auki.scenegraph.lookup-qr/v1");
contract!(LookupQrResult, "auki.scenegraph.lookup-qr-result/v2");
contract!(SnapshotReference, "auki.scenegraph.snapshot-reference/v1");

#[derive(Debug, thiserror::Error)]
#[error("Map Component: {0}")]
pub struct MapComponentError(String);

fn error(value: impl std::fmt::Display) -> MapComponentError {
    MapComponentError(value.to_string())
}
fn reject(value: impl std::fmt::Display) -> InvocationError {
    InvocationError::Rejected(value.to_string())
}

/// A publication ID must be fresh for each run, even when map_id persists.
/// The supplied clock callback reports snapshot publication time on clock_id.
pub struct MapComponentConfig {
    pub component_id: String,
    pub publication_id: String,
    pub clock_id: String,
    pub map: MapDefinition,
}

struct State {
    current: Observation<MapSnapshot>,
    output: ConfiguredObservable<MapSnapshot>,
    capture: BufferProductCapture<MapSnapshot>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    closed: bool,
}

impl State {
    fn reference(&self) -> SnapshotReference {
        SnapshotReference {
            product: self.capture.product().reference(),
            sequence: self.current.sequence,
        }
    }

    fn upsert(&mut self, request: UpsertQr) -> Result<SnapshotReference, InvocationError> {
        if self.closed {
            return Err(InvocationError::TargetUnavailable);
        }
        if request.expected_snapshot != self.reference() {
            return Err(reject(
                "snapshot conflict; fetch current state before retrying",
            ));
        }
        request.anchor.validate().map_err(reject)?;
        let mut scene = self.current.payload.scenegraph.clone();
        if scene.anchors.get(&request.anchor.anchor_id) == Some(&request.anchor) {
            return Ok(self.reference());
        }
        scene
            .anchors
            .insert(request.anchor.anchor_id.clone(), request.anchor);
        self.publish_scene(scene)
    }

    fn remove(&mut self, request: RemoveQr) -> Result<SnapshotReference, InvocationError> {
        if self.closed {
            return Err(InvocationError::TargetUnavailable);
        }
        if request.expected_snapshot != self.reference() {
            return Err(reject(
                "snapshot conflict; fetch current state before retrying",
            ));
        }
        let mut scene = self.current.payload.scenegraph.clone();
        if scene.anchors.remove(&request.anchor_id).is_none() {
            return Ok(self.reference());
        }
        self.publish_scene(scene)
    }

    fn publish_scene(&mut self, scene: Scenegraph) -> Result<SnapshotReference, InvocationError> {
        let snapshot = MapSnapshot::new(scene).map_err(reject)?;
        // Preflight the complete relevance list before publishing anything. Reserve
        // maximum sequence width so a later sequence cannot overflow this bound.
        MapCatalogData::from_snapshot(&snapshot)
            .metadata(u64::MAX)
            .map_err(reject)?;
        let timestamp = (self.clock)();
        if timestamp <= self.current.timestamp_ns {
            return Err(reject("publication clock must advance"));
        }
        let (observation, _) = self
            .output
            .publish(timestamp, Arc::new(snapshot))
            .map_err(reject)?;
        // Capture is synchronous and bounded. Never acknowledge or serve unretained state.
        if self.capture.product().buffer().range().last_sequence != Some(observation.sequence) {
            self.closed = true;
            self.capture.cancel();
            return Err(reject("snapshot retention failed; publication closed"));
        }
        self.current = observation;
        Ok(self.reference())
    }
}

/// Maintains one map; independent authorizers protect reads and writes.
/// A Domain association is never treated as authorization to edit.
pub struct MapComponent {
    component: Component,
    lookup: Operable<LookupQr, LookupQrResult>,
    find: Operable<FindQr, FindQrResult>,
    upsert: Operable<UpsertQr, SnapshotReference>,
    remove: Operable<RemoveQr, SnapshotReference>,
    state: Arc<Mutex<Option<State>>>,
}

impl MapComponent {
    pub fn new(
        runtime: &ComponentRuntime,
        config: MapComponentConfig,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
        authorize_read: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
        authorize_write: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
    ) -> Result<Self, MapComponentError> {
        let snapshot =
            MapSnapshot::new(Scenegraph::new(config.map.clone()).map_err(error)?).map_err(error)?;
        if config.publication_id.is_empty() || config.clock_id.is_empty() {
            return Err(error("publication and clock IDs are required"));
        }
        let spec = ComponentSpec::new(config.component_id)
            .observable(ObservableContract {
                name: "scenegraph".into(),
                datatype: MapSnapshot::DATATYPE.into(),
                schema: SNAPSHOT_SCHEMA.into(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            })
            .operable(OperableContract {
                name: LOOKUP_QR.into(),
                instruction: LookupQr::DATATYPE.into(),
                result: LookupQrResult::DATATYPE.into(),
                exposure: Exposure::Cluster,
            })
            .operable(OperableContract {
                name: FIND_QR.into(),
                instruction: FindQr::DATATYPE.into(),
                result: FindQrResult::DATATYPE.into(),
                exposure: Exposure::Cluster,
            })
            .operable(OperableContract {
                name: REMOVE_QR.into(),
                instruction: RemoveQr::DATATYPE.into(),
                result: SnapshotReference::DATATYPE.into(),
                exposure: Exposure::Cluster,
            })
            .operable(OperableContract {
                name: UPSERT_QR.into(),
                instruction: UpsertQr::DATATYPE.into(),
                result: SnapshotReference::DATATYPE.into(),
                exposure: Exposure::Cluster,
            });
        let component = runtime.component(spec).map_err(error)?;
        let output = component
            .configured_observable::<MapSnapshot>(
                ConfiguredObservableSpec::new(
                    "scenegraph",
                    format!("{}/scenegraph", config.publication_id),
                    config.clock_id,
                    PayloadContract::Structured(StructuredPayloadContract {
                        modality: "scenegraph".into(),
                        datatype: MapSnapshot::DATATYPE.into(),
                        schema: SNAPSHOT_SCHEMA.into(),
                        observes: config.map.map_id,
                        unit: None,
                    }),
                )
                .in_spatial_frame(config.map.frame.id),
            )
            .map_err(error)?;
        let state: Arc<Mutex<Option<State>>> = Arc::new(Mutex::new(None));
        let authorize_read = Arc::new(authorize_read);
        let authorize_lookup = Arc::clone(&authorize_read);
        let reader = Arc::clone(&state);
        let lookup = component
            .operable(
                LOOKUP_QR,
                move |c| authorize_lookup(c),
                move |_, request: LookupQr| {
                    let guard = reader
                        .lock()
                        .map_err(|_| InvocationError::TargetUnavailable)?;
                    let state = guard.as_ref().ok_or(InvocationError::TargetUnavailable)?;
                    if state.closed {
                        return Err(InvocationError::TargetUnavailable);
                    }
                    let scene = &state.current.payload.scenegraph;
                    Ok(LookupQrResult {
                        snapshot: state.reference(),
                        map: scene.map.clone(),
                        anchor: scene.anchors.get(&request.anchor_id).cloned(),
                    })
                },
            )
            .map_err(error)?;
        let reader = Arc::clone(&state);
        let find = component
            .operable(
                FIND_QR,
                move |c| authorize_read(c),
                move |_, request: FindQr| {
                    let guard = reader
                        .lock()
                        .map_err(|_| InvocationError::TargetUnavailable)?;
                    let state = guard.as_ref().ok_or(InvocationError::TargetUnavailable)?;
                    if state.closed {
                        return Err(InvocationError::TargetUnavailable);
                    }
                    let scene = &state.current.payload.scenegraph;
                    Ok(FindQrResult {
                        snapshot: state.reference(),
                        map: scene.map.clone(),
                        anchors: scene
                            .anchors
                            .values()
                            .filter(|a| a.payload == request.payload)
                            .cloned()
                            .collect(),
                    })
                },
            )
            .map_err(error)?;
        let authorize_write = Arc::new(authorize_write);
        let authorize_remove = Arc::clone(&authorize_write);
        let writer = Arc::clone(&state);
        let remove = component
            .operable(
                REMOVE_QR,
                move |c| authorize_remove(c),
                move |_, request: RemoveQr| {
                    writer
                        .lock()
                        .map_err(|_| InvocationError::TargetUnavailable)?
                        .as_mut()
                        .ok_or(InvocationError::TargetUnavailable)?
                        .remove(request)
                },
            )
            .map_err(error)?;
        let writer = Arc::clone(&state);
        let upsert = component
            .operable(
                UPSERT_QR,
                move |c| authorize_write(c),
                move |_, request: UpsertQr| {
                    writer
                        .lock()
                        .map_err(|_| InvocationError::TargetUnavailable)?
                        .as_mut()
                        .ok_or(InvocationError::TargetUnavailable)?
                        .upsert(request)
                },
            )
            .map_err(error)?;
        component.expose().map_err(error)?;
        let capture = runtime
            .capture_buffer_with_metadata(
                format!("{}/snapshots", config.publication_id),
                &output,
                BufferLimits::entries(1),
                MapSnapshot::encoded_size,
                |observation| {
                    MapCatalogData::from_snapshot(&observation.payload)
                        .metadata(observation.sequence)
                        .map(Some)
                },
            )
            .map_err(error)?;
        let (current, _) = output.publish(clock(), Arc::new(snapshot)).map_err(error)?;
        if !capture.errors().is_empty() {
            return Err(error("initial snapshot retention failed"));
        }
        *state.lock().unwrap() = Some(State {
            current,
            output,
            capture,
            clock: Arc::new(clock),
            closed: false,
        });
        Ok(Self {
            component,
            lookup,
            find,
            upsert,
            remove,
            state,
        })
    }

    pub fn component(&self) -> &Component {
        &self.component
    }
    pub fn lookup_qr(&self) -> &Operable<LookupQr, LookupQrResult> {
        &self.lookup
    }
    pub fn find_qr(&self) -> &Operable<FindQr, FindQrResult> {
        &self.find
    }
    pub fn upsert_qr(&self) -> &Operable<UpsertQr, SnapshotReference> {
        &self.upsert
    }
    pub fn remove_qr(&self) -> &Operable<RemoveQr, SnapshotReference> {
        &self.remove
    }
    pub fn product(&self) -> RetainedProduct<MapSnapshot> {
        self.state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .capture
            .product()
    }
    pub fn snapshot_reference(&self) -> SnapshotReference {
        self.state.lock().unwrap().as_ref().unwrap().reference()
    }
    /// Stops updates and closes readers; the last retained snapshot remains readable.
    /// Host applications close/unexport the network endpoint separately.
    pub fn close(&self) {
        if let Ok(mut guard) = self.state.lock()
            && let Some(state) = guard.as_mut()
        {
            state.closed = true;
            state.capture.cancel();
        }
    }
}

impl Drop for MapComponent {
    fn drop(&mut self) {
        self.close();
    }
}
