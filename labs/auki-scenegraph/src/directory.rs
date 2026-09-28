//! One explicitly selected authority for a Domain's map names and default selection.
//! This is an application Component, not a DDS Domain-record mutation.
use crate::{
    MapDefinition, MapSnapshot, SNAPSHOT_SCHEMA, Scenegraph, component::SnapshotReference,
};
use auki_components::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

pub const MAP_DIRECTORY_SCHEMA: &str = "auki.scenegraph.domain-map-directory/v1";
pub const RESOLVE_MAP: &str = "resolve_map";
pub const UPDATE_MAP_DIRECTORY: &str = "update_map_directory";
pub const MAX_DIRECTORY_MAPS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectoryMap {
    pub map: MapDefinition,
    /// Exact current publication. Replacing a publisher requires explicit registration.
    pub product: ProductReference,
}
impl DirectoryMap {
    pub fn from_product(product: &RetainedProduct<MapSnapshot>) -> Result<Self, DirectoryError> {
        let observation = product
            .latest_existing()
            .map_err(error)?
            .ok_or_else(|| error("map has no retained snapshot"))?;
        observation.payload.validate().map_err(error)?;
        if observation.output != product.manifest.producer
            || product.producer.payload.schema() != SNAPSHOT_SCHEMA
            || product.producer.spatial_frame_id.as_deref()
                != Some(&observation.payload.scenegraph.map.frame.id)
        {
            return Err(error("map producer contract mismatch"));
        }
        Ok(Self {
            map: observation.payload.scenegraph.map.clone(),
            product: product.reference(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainMapDirectory {
    pub domain_reference: String,
    pub default_map_id: Option<String>,
    /// Keyed by stable map ID. Names are optional, but unique within this Domain.
    pub maps: BTreeMap<String, DirectoryMap>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MapSelector {
    #[default]
    Default,
    Name(String),
    Id(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolveMap {
    pub domain_reference: String,
    /// Omitted selector means "give me the map for this Domain".
    #[serde(default)]
    pub selector: MapSelector,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedMap {
    pub directory_snapshot: SnapshotReference,
    pub selected: DirectoryMap,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DirectoryChange {
    Register {
        entry: DirectoryMap,
    },
    SetDefault {
        map_id: Option<String>,
    },
    /// Removing the default also clears the pointer; no automatic replacement.
    Remove {
        map_id: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateMapDirectory {
    pub expected_snapshot: SnapshotReference,
    pub change: DirectoryChange,
}
#[derive(Debug, thiserror::Error)]
#[error("Domain map directory: {0}")]
pub struct DirectoryError(String);
fn error(value: impl std::fmt::Display) -> DirectoryError {
    DirectoryError(value.to_string())
}
fn reject(value: impl std::fmt::Display) -> InvocationError {
    InvocationError::Rejected(value.to_string())
}
fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

impl DomainMapDirectory {
    pub fn new(domain_reference: impl Into<String>) -> Result<Self, DirectoryError> {
        let directory = Self {
            domain_reference: domain_reference.into(),
            default_map_id: None,
            maps: BTreeMap::new(),
        };
        directory.validate()?;
        Ok(directory)
    }
    pub fn validate(&self) -> Result<(), DirectoryError> {
        if !valid_text(&self.domain_reference) || self.maps.len() > MAX_DIRECTORY_MAPS {
            return Err(error("invalid Domain or directory size"));
        }
        let mut names = BTreeSet::new();
        for (id, entry) in &self.maps {
            Scenegraph::new(entry.map.clone()).map_err(error)?;
            if id != &entry.map.map_id
                || entry.map.domain_reference.as_deref() != Some(self.domain_reference.as_str())
            {
                return Err(error("map ID or Domain does not match directory"));
            }
            if [
                &entry.product.peer_id,
                &entry.product.product_id,
                &entry.product.manifest_hash,
            ]
            .iter()
            .any(|s| !valid_text(s))
            {
                return Err(error("invalid map Product reference"));
            }
            if let Some(name) = &entry.map.name
                && !names.insert(name)
            {
                return Err(error("duplicate map name in Domain"));
            }
        }
        if let Some(id) = &self.default_map_id
            && !self.maps.contains_key(id)
        {
            return Err(error("default map is not registered in this Domain"));
        }
        Ok(())
    }
    pub fn resolve(&self, request: &ResolveMap) -> Result<&DirectoryMap, DirectoryError> {
        self.validate()?;
        if request.domain_reference != self.domain_reference {
            return Err(error("wrong Domain"));
        }
        match &request.selector {
            MapSelector::Default => {
                let id = self
                    .default_map_id
                    .as_ref()
                    .ok_or_else(|| error("no default map has been selected for this Domain"))?;
                self.maps
                    .get(id)
                    .ok_or_else(|| error("default map is unavailable"))
            }
            MapSelector::Id(id) => self.maps.get(id).ok_or_else(|| error("unknown map ID")),
            MapSelector::Name(name) => self
                .maps
                .values()
                .find(|entry| entry.map.name.as_ref() == Some(name))
                .ok_or_else(|| error("unknown map name")),
        }
    }
    fn changed(&self, change: DirectoryChange) -> Result<Self, DirectoryError> {
        let mut next = self.clone();
        match change {
            DirectoryChange::Register { entry } => {
                next.maps.insert(entry.map.map_id.clone(), entry);
            }
            DirectoryChange::SetDefault { map_id } => {
                next.default_map_id = map_id;
            }
            DirectoryChange::Remove { map_id } => {
                if next.maps.remove(&map_id).is_none() {
                    return Err(error("unknown map ID"));
                }
                if next.default_map_id.as_ref() == Some(&map_id) {
                    next.default_map_id = None;
                }
            }
        }
        next.validate()?;
        next.metadata(u64::MAX)?;
        Ok(next)
    }
    fn metadata(&self, sequence: u64) -> Result<CatalogProductMetadata, DirectoryError> {
        let metadata = CatalogProductMetadata {
            schema: MAP_DIRECTORY_SCHEMA.into(),
            source_sequence: sequence,
            value: serde_json::to_value(self).map_err(error)?,
        };
        metadata.validate().map_err(error)?;
        Ok(metadata)
    }
}
macro_rules! contract {
    ($type:ty,$name:literal) => {
        impl ContractType for $type {
            const DATATYPE: &'static str = $name;
        }
    };
}
contract!(
    DomainMapDirectory,
    "auki.scenegraph.domain-map-directory/v1"
);
contract!(ResolveMap, "auki.scenegraph.resolve-map/v1");
contract!(ResolvedMap, "auki.scenegraph.resolved-map/v1");
contract!(
    UpdateMapDirectory,
    "auki.scenegraph.update-map-directory/v1"
);

pub struct MapDirectoryConfig {
    pub component_id: String,
    /// Fresh for each publisher run, including restarts.
    pub publication_id: String,
    pub clock_id: String,
    pub domain_reference: String,
}
struct DirectoryState {
    current: Observation<DomainMapDirectory>,
    output: ConfiguredObservable<DomainMapDirectory>,
    capture: BufferProductCapture<DomainMapDirectory>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    closed: bool,
}
impl DirectoryState {
    fn reference(&self) -> SnapshotReference {
        SnapshotReference {
            product: self.capture.product().reference(),
            sequence: self.current.sequence,
        }
    }
    fn update(
        &mut self,
        request: UpdateMapDirectory,
    ) -> Result<SnapshotReference, InvocationError> {
        if self.closed {
            return Err(InvocationError::TargetUnavailable);
        }
        if request.expected_snapshot != self.reference() {
            return Err(reject(
                "directory snapshot conflict; refresh before retrying",
            ));
        }
        let next = self
            .current
            .payload
            .changed(request.change)
            .map_err(reject)?;
        if next == *self.current.payload {
            return Ok(self.reference());
        }
        let timestamp = (self.clock)();
        if timestamp <= self.current.timestamp_ns {
            return Err(reject("directory publication clock must advance"));
        }
        let (observation, _) = self
            .output
            .publish(timestamp, Arc::new(next))
            .map_err(reject)?;
        if self.capture.product().buffer().range().last_sequence != Some(observation.sequence) {
            self.closed = true;
            self.capture.cancel();
            return Err(reject("directory retention failed; publication closed"));
        }
        self.current = observation;
        Ok(self.reference())
    }
}

/// One selected publisher controls Domain map selection. Multiple discovered
/// directories are competing authorities; clients must choose one explicitly.
pub struct MapDirectoryComponent {
    component: Component,
    resolve: Operable<ResolveMap, ResolvedMap>,
    update: Operable<UpdateMapDirectory, SnapshotReference>,
    state: Arc<Mutex<Option<DirectoryState>>>,
}
impl MapDirectoryComponent {
    pub fn new(
        runtime: &ComponentRuntime,
        config: MapDirectoryConfig,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
        authorize_read: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
        authorize_write: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
    ) -> Result<Self, DirectoryError> {
        let initial = DomainMapDirectory::new(config.domain_reference.clone())?;
        if !valid_text(&config.publication_id) || !valid_text(&config.clock_id) {
            return Err(error("publication and clock IDs are required and bounded"));
        }
        let component = runtime
            .component(
                ComponentSpec::new(config.component_id)
                    .observable(ObservableContract {
                        name: "directory".into(),
                        datatype: DomainMapDirectory::DATATYPE.into(),
                        schema: MAP_DIRECTORY_SCHEMA.into(),
                        access: vec![ObservationAccess::FollowNew],
                        exposure: Exposure::Cluster,
                    })
                    .operable(OperableContract {
                        name: RESOLVE_MAP.into(),
                        instruction: ResolveMap::DATATYPE.into(),
                        result: ResolvedMap::DATATYPE.into(),
                        exposure: Exposure::Cluster,
                    })
                    .operable(OperableContract {
                        name: UPDATE_MAP_DIRECTORY.into(),
                        instruction: UpdateMapDirectory::DATATYPE.into(),
                        result: SnapshotReference::DATATYPE.into(),
                        exposure: Exposure::Cluster,
                    }),
            )
            .map_err(error)?;
        let output = component
            .configured_observable::<DomainMapDirectory>(ConfiguredObservableSpec::new(
                "directory",
                format!("{}/directory", config.publication_id),
                config.clock_id,
                PayloadContract::Structured(StructuredPayloadContract {
                    modality: "map-directory".into(),
                    datatype: DomainMapDirectory::DATATYPE.into(),
                    schema: MAP_DIRECTORY_SCHEMA.into(),
                    observes: config.domain_reference,
                    unit: None,
                }),
            ))
            .map_err(error)?;
        let state: Arc<Mutex<Option<DirectoryState>>> = Arc::new(Mutex::new(None));
        let reader = state.clone();
        let resolve = component
            .operable(
                RESOLVE_MAP,
                authorize_read,
                move |_, request: ResolveMap| {
                    let guard = reader
                        .lock()
                        .map_err(|_| InvocationError::TargetUnavailable)?;
                    let state = guard.as_ref().ok_or(InvocationError::TargetUnavailable)?;
                    if state.closed {
                        return Err(InvocationError::TargetUnavailable);
                    }
                    Ok(ResolvedMap {
                        directory_snapshot: state.reference(),
                        selected: state
                            .current
                            .payload
                            .resolve(&request)
                            .map_err(reject)?
                            .clone(),
                    })
                },
            )
            .map_err(error)?;
        let writer = state.clone();
        let update = component
            .operable(
                UPDATE_MAP_DIRECTORY,
                authorize_write,
                move |_, request: UpdateMapDirectory| {
                    writer
                        .lock()
                        .map_err(|_| InvocationError::TargetUnavailable)?
                        .as_mut()
                        .ok_or(InvocationError::TargetUnavailable)?
                        .update(request)
                },
            )
            .map_err(error)?;
        component.expose().map_err(error)?;
        let capture = runtime
            .capture_buffer_with_metadata(
                format!("{}/directory-snapshots", config.publication_id),
                &output,
                BufferLimits::entries(1),
                |value| {
                    serde_json::to_vec(value)
                        .map(|v| v.len())
                        .unwrap_or(usize::MAX)
                },
                |observation| {
                    observation
                        .payload
                        .metadata(observation.sequence)
                        .map(Some)
                        .map_err(|e| e.to_string())
                },
            )
            .map_err(error)?;
        let (current, _) = output.publish(clock(), Arc::new(initial)).map_err(error)?;
        if !capture.errors().is_empty() {
            return Err(error("initial directory retention failed"));
        }
        *state.lock().unwrap() = Some(DirectoryState {
            current,
            output,
            capture,
            clock: Arc::new(clock),
            closed: false,
        });
        Ok(Self {
            component,
            resolve,
            update,
            state,
        })
    }
    pub fn component(&self) -> &Component {
        &self.component
    }
    pub fn resolve_map(&self) -> &Operable<ResolveMap, ResolvedMap> {
        &self.resolve
    }
    pub fn update_directory(&self) -> &Operable<UpdateMapDirectory, SnapshotReference> {
        &self.update
    }
    pub fn product(&self) -> RetainedProduct<DomainMapDirectory> {
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
    pub fn close(&self) {
        if let Ok(mut guard) = self.state.lock()
            && let Some(state) = guard.as_mut()
        {
            state.closed = true;
            state.capture.cancel();
        }
    }
}
impl Drop for MapDirectoryComponent {
    fn drop(&mut self) {
        self.close();
    }
}
