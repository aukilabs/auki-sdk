//! Deterministic Map state built by replaying SDK [`MapUpdate`]s.
//!
//! Mappers produce updates; this crate consumes them. It deliberately has no
//! network, robot, ROS, renderer, or storage dependency. A viewer can replay a
//! local or remote Map Log into [`VoxelMapAccumulator`] and request either a
//! complete [`ViewerSnapshot`] or only the chunks named by [`ApplySummary`].
//! Portal Maps use [`PortalMapAccumulator`] to retain idempotent,
//! provenance-keyed pose observations for later fusion.

use std::collections::{BTreeMap, BTreeSet};

use auki_datatypes::map::{
    ColorEvidence, ColorEvidenceDelta, MapUpdate, PortalMapCheckpoint, PortalObservation,
    SemanticEvidence, VoxelChunkSnapshot, VoxelMapCheckpoint, VoxelSnapshot,
};
use auki_registry::{
    MapBody, MapRegistryEntry, PortalMap, PortalObservationModel, RegistryRef, VoxelColorModel,
    VoxelMap, VoxelValueModel,
};

mod viewer;

pub use viewer::{
    ChunkRenderUpdate, ViewerAdapterError, VoxelInstance, VoxelViewerAdapter, VoxelViewerStyle,
};

/// Signed coordinate of a fixed-size chunk in the unbounded map grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkCoord {
    /// Chunk index on the map frame's x axis.
    pub x: i32,
    /// Chunk index on the map frame's y axis.
    pub y: i32,
    /// Chunk index on the map frame's z axis.
    pub z: i32,
}

/// Zero-based coordinate of a voxel inside one chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalVoxelCoord {
    /// Local x coordinate, less than the map's chunk dimension.
    pub x: u32,
    /// Local y coordinate, less than the map's chunk dimension.
    pub y: u32,
    /// Local z coordinate, less than the map's chunk dimension.
    pub z: u32,
}

/// Accumulated evidence for one semantic class.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticEvidenceSnapshot {
    /// Index into the Map Registry's `semantic_classes` list.
    pub class_id: u32,
    /// Sum of all evidence deltas replayed for this class and voxel.
    pub evidence: f64,
}

/// Viewer-facing state for one non-empty voxel.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerVoxel {
    /// Coordinate inside the containing chunk.
    pub local: LocalVoxelCoord,
    /// Centre of the voxel in metres in the Map Registry frame.
    pub center_m: [f64; 3],
    /// Sum of occupancy evidence deltas. This remains unclamped so the viewer
    /// can choose its own probability/color mapping.
    pub occupancy_evidence: f64,
    /// Weighted average source color in linear RGB, when color evidence has
    /// been accumulated for this voxel.
    pub linear_rgb: Option<[f64; 3]>,
    /// Sparse semantic evidence, sorted by class id.
    pub semantics: Vec<SemanticEvidenceSnapshot>,
}

/// Immutable viewer-facing snapshot of one chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerChunkSnapshot {
    /// Signed chunk coordinate.
    pub coord: ChunkCoord,
    /// Accumulator revision at which this chunk last changed.
    pub revision: u64,
    /// Sparse voxels sorted by their local coordinate.
    pub voxels: Vec<ViewerVoxel>,
}

/// Complete render input detached from the mutable accumulator.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerSnapshot {
    /// Accumulator revision represented by this snapshot.
    pub revision: u64,
    /// Exact Map Registry identity represented by this snapshot.
    pub map: RegistryRef,
    /// Coordinate frame in which voxel centres are expressed.
    pub frame: RegistryRef,
    /// Edge length of one voxel in metres.
    pub voxel_size_m: f64,
    /// Number of voxels along each chunk edge.
    pub chunk_dimension: u32,
    /// Semantic labels indexed by [`SemanticEvidenceSnapshot::class_id`].
    pub semantic_classes: Vec<String>,
    /// Non-empty chunks sorted by signed chunk coordinate.
    pub chunks: Vec<ViewerChunkSnapshot>,
}

/// Result of atomically applying one [`MapUpdate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplySummary {
    /// Revision after the apply. Zero-only or empty updates do not advance it.
    pub revision: u64,
    /// Chunks whose render state changed, sorted and deduplicated. A named
    /// chunk may now be absent, which tells an incremental viewer to remove it.
    pub changed_chunks: Vec<ChunkCoord>,
    /// Number of sparse voxel deltas in the accepted payload.
    pub voxel_deltas_applied: usize,
}

/// Stable identity of one Portal observation within its source Detection Log.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortalObservationKey {
    pub source_peer_id: String,
    pub source_resource_id: String,
    pub source_timestamp_ns: i64,
    pub source_sequence: u64,
    pub source_detection_index: u32,
}

impl From<&PortalObservation> for PortalObservationKey {
    fn from(observation: &PortalObservation) -> Self {
        Self {
            source_peer_id: observation.source_peer_id.clone(),
            source_resource_id: observation.source_resource_id.clone(),
            source_timestamp_ns: observation.source_timestamp_ns,
            source_sequence: observation.source_sequence,
            source_detection_index: observation.source_detection_index,
        }
    }
}

/// Result of atomically applying one Portal Map update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalApplySummary {
    pub revision: u64,
    pub observations_added: usize,
    pub checkpoint_applied: bool,
}

/// Deterministic materialization of provenance-keyed Portal observations.
#[derive(Debug, Clone, PartialEq)]
pub struct PortalMapAccumulator {
    map: RegistryRef,
    contract: PortalMap,
    revision: u64,
    observations: BTreeMap<PortalObservationKey, PortalObservation>,
}

impl PortalMapAccumulator {
    pub fn new(map: RegistryRef, contract: PortalMap) -> Result<Self, PortalAccumulatorError> {
        if contract.observation_model != PortalObservationModel::AppendOnlyPoseObservations {
            return Err(PortalAccumulatorError::UnsupportedObservationModel);
        }
        let registry_entry = MapRegistryEntry {
            peer_id: map.peer_id.clone(),
            map_id: map.id.clone(),
            body: MapBody::Portal(contract.clone()),
        };
        if registry_entry.registry_ref() != map {
            return Err(PortalAccumulatorError::MapIdentityMismatch);
        }
        Ok(Self {
            map,
            contract,
            revision: 0,
            observations: BTreeMap::new(),
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn map_ref(&self) -> &RegistryRef {
        &self.map
    }

    pub fn contract(&self) -> &PortalMap {
        &self.contract
    }

    pub fn observations(&self) -> impl Iterator<Item = &PortalObservation> {
        self.observations.values()
    }

    pub fn apply(
        &mut self,
        update: &MapUpdate,
    ) -> Result<PortalApplySummary, PortalAccumulatorError> {
        if !update.voxel_chunks.is_empty() || update.checkpoint.is_some() {
            return Err(PortalAccumulatorError::UnexpectedVoxelData);
        }
        if update.portal_checkpoint.is_some() && !update.portal_observations.is_empty() {
            return Err(PortalAccumulatorError::MixedCheckpointAndObservations);
        }

        if let Some(checkpoint) = &update.portal_checkpoint {
            let replacement = validated_observation_set(&checkpoint.observations, true)?;
            let changed = replacement != self.observations;
            if changed {
                self.observations = replacement;
                self.revision = self.revision.wrapping_add(1);
            }
            return Ok(PortalApplySummary {
                revision: self.revision,
                observations_added: 0,
                checkpoint_applied: changed,
            });
        }

        let incoming = validated_observation_set(&update.portal_observations, false)?;
        validate_portal_sizes(self.observations.values().chain(incoming.values()))?;
        for (key, observation) in &incoming {
            if self
                .observations
                .get(key)
                .is_some_and(|existing| existing != observation)
            {
                return Err(PortalAccumulatorError::ConflictingObservation(key.clone()));
            }
        }
        let mut observations_added = 0;
        for (key, observation) in incoming {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.observations.entry(key)
            {
                entry.insert(observation);
                observations_added += 1;
            }
        }
        if observations_added > 0 {
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(PortalApplySummary {
            revision: self.revision,
            observations_added,
            checkpoint_applied: false,
        })
    }

    pub fn checkpoint_update(&self) -> MapUpdate {
        MapUpdate {
            voxel_chunks: vec![],
            checkpoint: None,
            portal_observations: vec![],
            portal_checkpoint: Some(PortalMapCheckpoint {
                observations: self.observations.values().cloned().collect(),
            }),
        }
    }
}

fn validated_observation_set(
    observations: &[PortalObservation],
    reject_duplicate_keys: bool,
) -> Result<BTreeMap<PortalObservationKey, PortalObservation>, PortalAccumulatorError> {
    let mut validated = BTreeMap::new();
    for observation in observations {
        validate_portal_observation(observation)?;
        let key = PortalObservationKey::from(observation);
        if let Some(existing) = validated.insert(key.clone(), observation.clone())
            && (reject_duplicate_keys || existing != *observation)
        {
            return Err(PortalAccumulatorError::ConflictingObservation(key));
        }
    }
    validate_portal_sizes(validated.values())?;
    Ok(validated)
}

fn validate_portal_sizes<'a>(
    observations: impl IntoIterator<Item = &'a PortalObservation>,
) -> Result<(), PortalAccumulatorError> {
    let mut sizes = BTreeMap::<&str, f64>::new();
    for observation in observations {
        match sizes.insert(&observation.portal_id, observation.physical_size_m) {
            Some(size) if size != observation.physical_size_m => {
                return Err(PortalAccumulatorError::ConflictingPortalSize(
                    observation.portal_id.clone(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_portal_observation(
    observation: &PortalObservation,
) -> Result<(), PortalAccumulatorError> {
    if observation.portal_id.is_empty()
        || observation.source_peer_id.is_empty()
        || observation.source_resource_id.is_empty()
        || observation.camera_frame_peer_id.is_empty()
        || observation.camera_frame_id.is_empty()
        || observation.camera_frame_hash.is_empty()
    {
        return Err(PortalAccumulatorError::MissingIdentity);
    }
    if !observation.physical_size_m.is_finite() || observation.physical_size_m <= 0.0 {
        return Err(PortalAccumulatorError::InvalidPhysicalSize);
    }
    if !observation.confidence.is_finite()
        || !(0.0..=1.0).contains(&observation.confidence)
        || !observation.normalized_corner_error.is_finite()
        || observation.normalized_corner_error < 0.0
    {
        return Err(PortalAccumulatorError::InvalidQuality);
    }
    let transform = observation
        .portal_to_map
        .as_ref()
        .ok_or(PortalAccumulatorError::IncompletePose)?;
    let translation = transform
        .translation
        .as_ref()
        .ok_or(PortalAccumulatorError::IncompletePose)?;
    let orientation = transform
        .orientation
        .as_ref()
        .ok_or(PortalAccumulatorError::IncompletePose)?;
    if ![
        translation.x,
        translation.y,
        translation.z,
        orientation.x,
        orientation.y,
        orientation.z,
        orientation.w,
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        return Err(PortalAccumulatorError::NonFinitePose);
    }
    let norm_squared = orientation.x * orientation.x
        + orientation.y * orientation.y
        + orientation.z * orientation.z
        + orientation.w * orientation.w;
    if norm_squared <= f64::EPSILON {
        return Err(PortalAccumulatorError::ZeroQuaternion);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PortalAccumulatorError {
    #[error("Map Registry reference does not match the Portal contract")]
    MapIdentityMismatch,
    #[error("unsupported Portal observation model")]
    UnsupportedObservationModel,
    #[error("Portal MapUpdate contains voxel data")]
    UnexpectedVoxelData,
    #[error("Portal MapUpdate cannot contain both a checkpoint and observations")]
    MixedCheckpointAndObservations,
    #[error("Portal observation is missing required identity provenance")]
    MissingIdentity,
    #[error("Portal observation physical size must be finite and positive")]
    InvalidPhysicalSize,
    #[error("Portal observation confidence or error is invalid")]
    InvalidQuality,
    #[error("Portal observation pose is incomplete")]
    IncompletePose,
    #[error("Portal observation pose contains NaN or infinity")]
    NonFinitePose,
    #[error("Portal observation pose quaternion has zero magnitude")]
    ZeroQuaternion,
    #[error("conflicting Portal observation for provenance key {0:?}")]
    ConflictingObservation(PortalObservationKey),
    #[error("Portal {0:?} has conflicting canonical physical sizes")]
    ConflictingPortalSize(String),
}

#[derive(Debug, Clone, PartialEq)]
struct VoxelEvidence {
    occupancy: f64,
    semantics: BTreeMap<u32, f64>,
    color: Option<AccumulatedColor>,
}

#[derive(Debug, Clone, PartialEq)]
struct AccumulatedColor {
    sums: [f64; 3],
    weight: f64,
}

impl AccumulatedColor {
    fn from_delta(color: &ColorEvidenceDelta) -> Self {
        Self {
            sums: [
                f64::from(color.red_sum_delta),
                f64::from(color.green_sum_delta),
                f64::from(color.blue_sum_delta),
            ],
            weight: f64::from(color.weight_delta),
        }
    }

    fn from_checkpoint(color: &ColorEvidence) -> Self {
        Self {
            sums: [color.red_sum, color.green_sum, color.blue_sum],
            weight: color.weight,
        }
    }

    fn add_delta(&mut self, color: &ColorEvidenceDelta) {
        self.sums[0] += f64::from(color.red_sum_delta);
        self.sums[1] += f64::from(color.green_sum_delta);
        self.sums[2] += f64::from(color.blue_sum_delta);
        self.weight += f64::from(color.weight_delta);
    }

    fn linear_rgb(&self) -> [f64; 3] {
        self.sums.map(|sum| (sum / self.weight).clamp(0.0, 1.0))
    }
}

impl Default for VoxelEvidence {
    fn default() -> Self {
        Self {
            occupancy: 0.0,
            semantics: BTreeMap::new(),
            color: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ChunkState {
    revision: u64,
    voxels: BTreeMap<LocalVoxelCoord, VoxelEvidence>,
}

/// Sparse materialized state of one voxel Map resource.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelMapAccumulator {
    map: RegistryRef,
    contract: VoxelMap,
    revision: u64,
    chunks: BTreeMap<ChunkCoord, ChunkState>,
}

impl VoxelMapAccumulator {
    /// Create empty state pinned to an immutable Map Registry contract.
    pub fn new(map: RegistryRef, contract: VoxelMap) -> Result<Self, AccumulatorError> {
        if !contract.voxel_size_m.0.is_finite()
            || contract.voxel_size_m.0 <= 0.0
            || contract.chunk_dimension == 0
        {
            return Err(AccumulatorError::InvalidContract);
        }
        if contract.value_model != VoxelValueModel::AdditiveOccupancyEvidence {
            return Err(AccumulatorError::UnsupportedValueModel);
        }
        let registry_entry = MapRegistryEntry {
            peer_id: map.peer_id.clone(),
            map_id: map.id.clone(),
            body: MapBody::Voxel(contract.clone()),
        };
        if registry_entry.registry_ref() != map {
            return Err(AccumulatorError::MapIdentityMismatch);
        }
        Ok(Self {
            map,
            contract,
            revision: 0,
            chunks: BTreeMap::new(),
        })
    }

    /// Current monotonically increasing state revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Exact content-addressed Map resource represented by this state.
    pub fn map_ref(&self) -> &RegistryRef {
        &self.map
    }

    /// Immutable Map Registry contract used by this state.
    pub fn contract(&self) -> &VoxelMap {
        &self.contract
    }

    /// Atomically validate and apply one update. Validation completes before
    /// any chunk is mutated, so malformed remote input cannot partially alter
    /// viewer state.
    pub fn apply(&mut self, update: &MapUpdate) -> Result<ApplySummary, AccumulatorError> {
        self.validate_update(update)?;

        if let Some(checkpoint) = &update.checkpoint {
            return self.apply_checkpoint(checkpoint);
        }

        let mut changed = BTreeSet::new();
        let mut delta_count = 0usize;
        for chunk_update in &update.voxel_chunks {
            let chunk_coord = ChunkCoord {
                x: chunk_update.chunk_x,
                y: chunk_update.chunk_y,
                z: chunk_update.chunk_z,
            };
            for delta in &chunk_update.voxels {
                delta_count += 1;
                let changes_value = delta.occupancy_delta != 0.0
                    || delta
                        .semantics
                        .iter()
                        .any(|semantic| semantic.evidence_delta != 0.0)
                    || delta
                        .color
                        .as_ref()
                        .is_some_and(|color| color.weight_delta != 0.0);
                if !changes_value {
                    continue;
                }
                changed.insert(chunk_coord);
                let local = LocalVoxelCoord {
                    x: delta.x,
                    y: delta.y,
                    z: delta.z,
                };
                let voxel = self
                    .chunks
                    .entry(chunk_coord)
                    .or_insert_with(|| ChunkState {
                        revision: self.revision,
                        voxels: BTreeMap::new(),
                    })
                    .voxels
                    .entry(local)
                    .or_default();
                voxel.occupancy += f64::from(delta.occupancy_delta);
                for semantic in &delta.semantics {
                    let evidence = voxel.semantics.entry(semantic.class_id).or_default();
                    *evidence += f64::from(semantic.evidence_delta);
                    if *evidence == 0.0 {
                        voxel.semantics.remove(&semantic.class_id);
                    }
                }
                if let Some(color) = &delta.color {
                    match &mut voxel.color {
                        Some(accumulated) => accumulated.add_delta(color),
                        None => voxel.color = Some(AccumulatedColor::from_delta(color)),
                    }
                }
            }
        }

        if !changed.is_empty() {
            self.revision = self.revision.wrapping_add(1);
            for coord in &changed {
                let remove_voxels = self
                    .chunks
                    .get(coord)
                    .map(|chunk| {
                        chunk
                            .voxels
                            .iter()
                            .filter_map(|(coord, voxel)| {
                                (voxel.occupancy == 0.0
                                    && voxel.semantics.is_empty()
                                    && voxel.color.is_none())
                                .then_some(*coord)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if let Some(chunk) = self.chunks.get_mut(coord) {
                    for voxel in remove_voxels {
                        chunk.voxels.remove(&voxel);
                    }
                    chunk.revision = self.revision;
                }
                if self
                    .chunks
                    .get(coord)
                    .is_some_and(|chunk| chunk.voxels.is_empty())
                {
                    self.chunks.remove(coord);
                }
            }
        }

        Ok(ApplySummary {
            revision: self.revision,
            changed_chunks: changed.into_iter().collect(),
            voxel_deltas_applied: delta_count,
        })
    }

    /// Serialize the complete sparse state as an ordered checkpoint barrier.
    /// The resulting update has no additive chunks, so older protobuf readers
    /// safely treat it as an empty update rather than adding absolute values.
    pub fn checkpoint_update(&self) -> MapUpdate {
        MapUpdate {
            voxel_chunks: Vec::new(),
            checkpoint: Some(VoxelMapCheckpoint {
                voxel_chunks: self
                    .chunks
                    .iter()
                    .map(|(coord, chunk)| VoxelChunkSnapshot {
                        chunk_x: coord.x,
                        chunk_y: coord.y,
                        chunk_z: coord.z,
                        voxels: chunk
                            .voxels
                            .iter()
                            .map(|(local, voxel)| VoxelSnapshot {
                                x: local.x,
                                y: local.y,
                                z: local.z,
                                occupancy_evidence: voxel.occupancy,
                                semantics: voxel
                                    .semantics
                                    .iter()
                                    .map(|(class_id, evidence)| SemanticEvidence {
                                        class_id: *class_id,
                                        evidence: *evidence,
                                    })
                                    .collect(),
                                color: voxel.color.as_ref().map(|color| ColorEvidence {
                                    red_sum: color.sums[0],
                                    green_sum: color.sums[1],
                                    blue_sum: color.sums[2],
                                    weight: color.weight,
                                }),
                            })
                            .collect(),
                    })
                    .collect(),
            }),
            portal_observations: Vec::new(),
            portal_checkpoint: None,
        }
    }

    /// Snapshot one changed chunk for incremental rendering. `None` means the
    /// chunk is empty and should be removed from the viewer.
    pub fn viewer_chunk(
        &self,
        coord: ChunkCoord,
        minimum_occupancy_evidence: f64,
    ) -> Result<Option<ViewerChunkSnapshot>, AccumulatorError> {
        self.validate_threshold(minimum_occupancy_evidence)?;
        Ok(self
            .chunks
            .get(&coord)
            .map(|chunk| self.viewer_chunk_from_state(coord, chunk, minimum_occupancy_evidence)))
    }

    /// Produce a stable, complete snapshot for a voxel renderer. Voxels below
    /// `minimum_occupancy_evidence` are omitted without changing map state.
    pub fn viewer_snapshot(
        &self,
        minimum_occupancy_evidence: f64,
    ) -> Result<ViewerSnapshot, AccumulatorError> {
        self.validate_threshold(minimum_occupancy_evidence)?;
        let chunks = self
            .chunks
            .iter()
            .map(|(coord, chunk)| {
                self.viewer_chunk_from_state(*coord, chunk, minimum_occupancy_evidence)
            })
            .filter(|chunk| !chunk.voxels.is_empty())
            .collect();
        Ok(ViewerSnapshot {
            revision: self.revision,
            map: self.map.clone(),
            frame: self.contract.frame.clone(),
            voxel_size_m: self.contract.voxel_size_m.0,
            chunk_dimension: self.contract.chunk_dimension,
            semantic_classes: self.contract.semantic_classes.clone(),
            chunks,
        })
    }

    fn validate_threshold(&self, threshold: f64) -> Result<(), AccumulatorError> {
        if threshold.is_finite() {
            Ok(())
        } else {
            Err(AccumulatorError::InvalidViewerThreshold)
        }
    }

    fn validate_update(&self, update: &MapUpdate) -> Result<(), AccumulatorError> {
        if !update.portal_observations.is_empty() || update.portal_checkpoint.is_some() {
            return Err(AccumulatorError::UnexpectedPortalData);
        }
        if update.checkpoint.is_some() && !update.voxel_chunks.is_empty() {
            return Err(AccumulatorError::MixedCheckpointAndDeltas);
        }
        let dimension = self.contract.chunk_dimension;
        let semantic_class_count = self.contract.semantic_classes.len();
        for chunk in &update.voxel_chunks {
            for voxel in &chunk.voxels {
                if voxel.x >= dimension || voxel.y >= dimension || voxel.z >= dimension {
                    return Err(AccumulatorError::VoxelOutsideChunk {
                        x: voxel.x,
                        y: voxel.y,
                        z: voxel.z,
                        dimension,
                    });
                }
                if !voxel.occupancy_delta.is_finite() {
                    return Err(AccumulatorError::NonFiniteEvidence);
                }
                for semantic in &voxel.semantics {
                    if semantic.class_id as usize >= semantic_class_count {
                        return Err(AccumulatorError::UnknownSemanticClass {
                            class_id: semantic.class_id,
                            class_count: semantic_class_count,
                        });
                    }
                    if !semantic.evidence_delta.is_finite() {
                        return Err(AccumulatorError::NonFiniteEvidence);
                    }
                }
                self.validate_color_delta(voxel.color.as_ref())?;
            }
        }
        if let Some(checkpoint) = &update.checkpoint {
            let mut chunks = BTreeSet::new();
            for chunk in &checkpoint.voxel_chunks {
                let chunk_coord = (chunk.chunk_x, chunk.chunk_y, chunk.chunk_z);
                if !chunks.insert(chunk_coord) {
                    return Err(AccumulatorError::DuplicateCheckpointChunk {
                        x: chunk.chunk_x,
                        y: chunk.chunk_y,
                        z: chunk.chunk_z,
                    });
                }
                let mut voxels = BTreeSet::new();
                for voxel in &chunk.voxels {
                    if voxel.x >= dimension || voxel.y >= dimension || voxel.z >= dimension {
                        return Err(AccumulatorError::VoxelOutsideChunk {
                            x: voxel.x,
                            y: voxel.y,
                            z: voxel.z,
                            dimension,
                        });
                    }
                    if !voxels.insert((voxel.x, voxel.y, voxel.z)) {
                        return Err(AccumulatorError::DuplicateCheckpointVoxel {
                            x: voxel.x,
                            y: voxel.y,
                            z: voxel.z,
                        });
                    }
                    if !voxel.occupancy_evidence.is_finite() {
                        return Err(AccumulatorError::NonFiniteEvidence);
                    }
                    let mut semantics = BTreeSet::new();
                    for semantic in &voxel.semantics {
                        if semantic.class_id as usize >= semantic_class_count {
                            return Err(AccumulatorError::UnknownSemanticClass {
                                class_id: semantic.class_id,
                                class_count: semantic_class_count,
                            });
                        }
                        if !semantics.insert(semantic.class_id) {
                            return Err(AccumulatorError::DuplicateCheckpointSemanticClass {
                                class_id: semantic.class_id,
                            });
                        }
                        if !semantic.evidence.is_finite() {
                            return Err(AccumulatorError::NonFiniteEvidence);
                        }
                    }
                    self.validate_color_checkpoint(voxel.color.as_ref())?;
                }
            }
        }
        Ok(())
    }

    fn apply_checkpoint(
        &mut self,
        checkpoint: &VoxelMapCheckpoint,
    ) -> Result<ApplySummary, AccumulatorError> {
        let next_revision = self.revision.wrapping_add(1);
        let mut replacement = BTreeMap::new();
        let mut delta_count = 0usize;
        for chunk in &checkpoint.voxel_chunks {
            let coord = ChunkCoord {
                x: chunk.chunk_x,
                y: chunk.chunk_y,
                z: chunk.chunk_z,
            };
            let mut voxels = BTreeMap::new();
            for snapshot in &chunk.voxels {
                delta_count += 1;
                let semantics: BTreeMap<u32, f64> = snapshot
                    .semantics
                    .iter()
                    .filter_map(|semantic| {
                        (semantic.evidence != 0.0).then_some((semantic.class_id, semantic.evidence))
                    })
                    .collect();
                let color = snapshot
                    .color
                    .as_ref()
                    .map(AccumulatedColor::from_checkpoint);
                if snapshot.occupancy_evidence != 0.0 || !semantics.is_empty() || color.is_some() {
                    voxels.insert(
                        LocalVoxelCoord {
                            x: snapshot.x,
                            y: snapshot.y,
                            z: snapshot.z,
                        },
                        VoxelEvidence {
                            occupancy: snapshot.occupancy_evidence,
                            semantics,
                            color,
                        },
                    );
                }
            }
            if !voxels.is_empty() {
                replacement.insert(
                    coord,
                    ChunkState {
                        revision: next_revision,
                        voxels,
                    },
                );
            }
        }
        let changed_chunks = self
            .chunks
            .keys()
            .chain(replacement.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.chunks = replacement;
        self.revision = next_revision;
        Ok(ApplySummary {
            revision: self.revision,
            changed_chunks,
            voxel_deltas_applied: delta_count,
        })
    }

    fn viewer_chunk_from_state(
        &self,
        coord: ChunkCoord,
        chunk: &ChunkState,
        threshold: f64,
    ) -> ViewerChunkSnapshot {
        let dimension = f64::from(self.contract.chunk_dimension);
        let voxel_size = self.contract.voxel_size_m.0;
        let voxels = chunk
            .voxels
            .iter()
            .filter(|(_, evidence)| evidence.occupancy >= threshold)
            .map(|(local, evidence)| {
                let center = |chunk_axis: i32, local_axis: u32| {
                    (f64::from(chunk_axis) * dimension + f64::from(local_axis) + 0.5) * voxel_size
                };
                ViewerVoxel {
                    local: *local,
                    center_m: [
                        center(coord.x, local.x),
                        center(coord.y, local.y),
                        center(coord.z, local.z),
                    ],
                    occupancy_evidence: evidence.occupancy,
                    linear_rgb: evidence.color.as_ref().map(AccumulatedColor::linear_rgb),
                    semantics: evidence
                        .semantics
                        .iter()
                        .map(|(class_id, evidence)| SemanticEvidenceSnapshot {
                            class_id: *class_id,
                            evidence: *evidence,
                        })
                        .collect(),
                }
            })
            .collect();
        ViewerChunkSnapshot {
            coord,
            revision: chunk.revision,
            voxels,
        }
    }

    fn validate_color_delta(
        &self,
        color: Option<&ColorEvidenceDelta>,
    ) -> Result<(), AccumulatorError> {
        let Some(color) = color else {
            return Ok(());
        };
        if self.contract.color_model != Some(VoxelColorModel::AdditiveLinearRgbEvidence) {
            return Err(AccumulatorError::UnexpectedColorEvidence);
        }
        validate_color_components(
            [
                f64::from(color.red_sum_delta),
                f64::from(color.green_sum_delta),
                f64::from(color.blue_sum_delta),
            ],
            f64::from(color.weight_delta),
            true,
        )
    }

    fn validate_color_checkpoint(
        &self,
        color: Option<&ColorEvidence>,
    ) -> Result<(), AccumulatorError> {
        let Some(color) = color else {
            return Ok(());
        };
        if self.contract.color_model != Some(VoxelColorModel::AdditiveLinearRgbEvidence) {
            return Err(AccumulatorError::UnexpectedColorEvidence);
        }
        validate_color_components(
            [color.red_sum, color.green_sum, color.blue_sum],
            color.weight,
            false,
        )
    }
}

fn validate_color_components(
    sums: [f64; 3],
    weight: f64,
    allow_empty: bool,
) -> Result<(), AccumulatorError> {
    if !weight.is_finite() || sums.into_iter().any(|sum| !sum.is_finite()) {
        return Err(AccumulatorError::NonFiniteColorEvidence);
    }
    if weight < 0.0 || (!allow_empty && weight == 0.0) {
        return Err(AccumulatorError::InvalidColorWeight);
    }
    if sums.into_iter().any(|sum| sum < 0.0 || sum > weight) {
        return Err(AccumulatorError::InvalidColorChannelSum);
    }
    Ok(())
}

/// Invalid contract, update, or viewer query.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AccumulatorError {
    /// Voxel size or chunk dimension is invalid.
    #[error("invalid voxel map contract")]
    InvalidContract,
    /// The supplied Map reference does not hash to the supplied contract.
    #[error("Map Registry reference does not match the voxel contract")]
    MapIdentityMismatch,
    /// The registry declares a value model this accumulator cannot merge.
    #[error("unsupported voxel value model")]
    UnsupportedValueModel,
    /// A sparse local coordinate lies outside the registered chunk cube.
    #[error("voxel ({x}, {y}, {z}) lies outside chunk dimension {dimension}")]
    VoxelOutsideChunk {
        /// Invalid local x.
        x: u32,
        /// Invalid local y.
        y: u32,
        /// Invalid local z.
        z: u32,
        /// Exclusive coordinate upper bound.
        dimension: u32,
    },
    /// Occupancy or semantic evidence is NaN or infinite.
    #[error("map evidence must be finite")]
    NonFiniteEvidence,
    /// A MapUpdate carries color but its Map Registry contract does not.
    #[error("MapUpdate contains color evidence for an occupancy-only voxel Map")]
    UnexpectedColorEvidence,
    /// A color channel sum or weight is NaN or infinite.
    #[error("voxel color evidence must be finite")]
    NonFiniteColorEvidence,
    /// Color weights are additive and cannot be negative; checkpoints require
    /// a positive weight whenever color is present.
    #[error("voxel color evidence has an invalid weight")]
    InvalidColorWeight,
    /// Weighted channel sums must remain inside `[0, weight]`.
    #[error("voxel color channel sums must be within [0, weight]")]
    InvalidColorChannelSum,
    /// A semantic delta refers beyond the registry's label list.
    #[error("semantic class {class_id} is outside class count {class_count}")]
    UnknownSemanticClass {
        /// Invalid class index.
        class_id: u32,
        /// Number of registered semantic classes.
        class_count: usize,
    },
    /// Viewer filtering threshold is NaN or infinite.
    #[error("viewer occupancy threshold must be finite")]
    InvalidViewerThreshold,
    /// A checkpoint barrier cannot also carry commutative deltas.
    #[error("MapUpdate cannot contain both a checkpoint and additive voxel chunks")]
    MixedCheckpointAndDeltas,
    /// A voxel Map cannot consume Portal observation fields.
    #[error("voxel MapUpdate contains Portal observation data")]
    UnexpectedPortalData,
    /// A full-state checkpoint named one chunk more than once.
    #[error("checkpoint contains duplicate chunk ({x}, {y}, {z})")]
    DuplicateCheckpointChunk {
        /// Duplicate chunk x.
        x: i32,
        /// Duplicate chunk y.
        y: i32,
        /// Duplicate chunk z.
        z: i32,
    },
    /// A checkpoint named one local voxel more than once in a chunk.
    #[error("checkpoint contains duplicate voxel ({x}, {y}, {z})")]
    DuplicateCheckpointVoxel {
        /// Duplicate local x.
        x: u32,
        /// Duplicate local y.
        y: u32,
        /// Duplicate local z.
        z: u32,
    },
    /// A checkpoint named one semantic class more than once in a voxel.
    #[error("checkpoint contains duplicate semantic class {class_id}")]
    DuplicateCheckpointSemanticClass {
        /// Duplicate semantic class id.
        class_id: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_datatypes::{
        map::{ColorEvidenceDelta, SemanticDelta, VoxelChunkUpdate, VoxelDelta},
        pose::{Quat, SpatialTransform, Vec3},
    };
    use auki_registry::{FiniteF64, PortalObservationModel, VoxelColorModel};

    fn contract() -> VoxelMap {
        VoxelMap {
            frame: RegistryRef {
                peer_id: "galbot".into(),
                id: "world".into(),
                hash: "frame-hash".into(),
            },
            voxel_size_m: FiniteF64(0.5),
            chunk_dimension: 64,
            value_model: VoxelValueModel::AdditiveOccupancyEvidence,
            color_model: None,
            semantic_classes: vec!["wall".into(), "door".into()],
        }
    }

    fn accumulator() -> VoxelMapAccumulator {
        let contract = contract();
        let map = MapRegistryEntry {
            peer_id: "galbot".into(),
            map_id: "occupancy".into(),
            body: MapBody::Voxel(contract.clone()),
        }
        .registry_ref();
        VoxelMapAccumulator::new(map, contract).unwrap()
    }

    fn colored_accumulator() -> VoxelMapAccumulator {
        let mut contract = contract();
        contract.color_model = Some(VoxelColorModel::AdditiveLinearRgbEvidence);
        let map = MapRegistryEntry {
            peer_id: "galbot".into(),
            map_id: "colored".into(),
            body: MapBody::Voxel(contract.clone()),
        }
        .registry_ref();
        VoxelMapAccumulator::new(map, contract).unwrap()
    }

    fn color_update(color: [f32; 3], weight: f32) -> MapUpdate {
        MapUpdate {
            voxel_chunks: vec![VoxelChunkUpdate {
                chunk_x: 0,
                chunk_y: 0,
                chunk_z: 0,
                voxels: vec![VoxelDelta {
                    x: 1,
                    y: 2,
                    z: 3,
                    occupancy_delta: 1.0,
                    semantics: vec![],
                    color: Some(ColorEvidenceDelta {
                        red_sum_delta: color[0] * weight,
                        green_sum_delta: color[1] * weight,
                        blue_sum_delta: color[2] * weight,
                        weight_delta: weight,
                    }),
                }],
            }],
            checkpoint: None,
            portal_observations: vec![],
            portal_checkpoint: None,
        }
    }

    fn update(chunk_x: i32, occupancy_delta: f32, semantic_delta: f32) -> MapUpdate {
        MapUpdate {
            voxel_chunks: vec![VoxelChunkUpdate {
                chunk_x,
                chunk_y: 0,
                chunk_z: 0,
                voxels: vec![VoxelDelta {
                    x: 63,
                    y: 1,
                    z: 2,
                    occupancy_delta,
                    semantics: vec![SemanticDelta {
                        class_id: 1,
                        evidence_delta: semantic_delta,
                    }],
                    color: None,
                }],
            }],
            checkpoint: None,
            portal_observations: vec![],
            portal_checkpoint: None,
        }
    }

    #[test]
    fn replay_accumulates_evidence_and_emits_viewer_coordinates() {
        let mut map = accumulator();
        let summary = map.apply(&update(-1, 0.8, 0.25)).unwrap();
        assert_eq!(summary.revision, 1);
        assert_eq!(
            summary.changed_chunks,
            vec![ChunkCoord { x: -1, y: 0, z: 0 }]
        );

        map.apply(&update(-1, -0.3, 0.5)).unwrap();
        let snapshot = map.viewer_snapshot(0.0).unwrap();
        assert_eq!(snapshot.map, *map.map_ref());
        let voxel = &snapshot.chunks[0].voxels[0];
        assert!((voxel.occupancy_evidence - 0.5).abs() < 1e-6);
        assert_eq!(voxel.center_m, [-0.25, 0.75, 1.25]);
        assert!((voxel.semantics[0].evidence - 0.75).abs() < 1e-6);
    }

    #[test]
    fn weighted_color_commutes_and_survives_checkpoint_replay() {
        let red = color_update([1.0, 0.0, 0.0], 1.0);
        let blue = color_update([0.0, 0.0, 1.0], 3.0);

        let mut red_blue = colored_accumulator();
        red_blue.apply(&red).unwrap();
        red_blue.apply(&blue).unwrap();
        let mut blue_red = colored_accumulator();
        blue_red.apply(&blue).unwrap();
        blue_red.apply(&red).unwrap();
        let expected = [0.25, 0.0, 0.75];
        assert_eq!(
            red_blue.viewer_snapshot(0.0).unwrap().chunks[0].voxels[0].linear_rgb,
            Some(expected)
        );
        assert_eq!(
            red_blue.viewer_snapshot(0.0).unwrap().chunks,
            blue_red.viewer_snapshot(0.0).unwrap().chunks
        );

        let checkpoint = red_blue.checkpoint_update();
        let mut replayed = colored_accumulator();
        replayed.apply(&checkpoint).unwrap();
        assert_eq!(
            replayed.viewer_snapshot(0.0).unwrap().chunks[0].voxels,
            red_blue.viewer_snapshot(0.0).unwrap().chunks[0].voxels
        );
        let stored = checkpoint.checkpoint.unwrap().voxel_chunks[0].voxels[0]
            .color
            .unwrap();
        assert_eq!(stored.weight, 4.0);
        assert_eq!(
            [stored.red_sum, stored.green_sum, stored.blue_sum],
            [1.0, 0.0, 3.0]
        );
    }

    #[test]
    fn color_evidence_requires_a_colored_contract_and_valid_weighted_sums() {
        let colored = color_update([1.0, 0.0, 0.0], 1.0);
        assert_eq!(
            accumulator().apply(&colored).unwrap_err(),
            AccumulatorError::UnexpectedColorEvidence
        );

        let mut invalid = colored;
        let color = invalid.voxel_chunks[0].voxels[0].color.as_mut().unwrap();
        color.red_sum_delta = 2.0;
        assert_eq!(
            colored_accumulator().apply(&invalid).unwrap_err(),
            AccumulatorError::InvalidColorChannelSum
        );
    }

    #[test]
    fn accumulator_rejects_a_stale_map_identity() {
        let contract = contract();
        let mut map = MapRegistryEntry {
            peer_id: "galbot".into(),
            map_id: "occupancy".into(),
            body: MapBody::Voxel(contract.clone()),
        }
        .registry_ref();
        map.hash = "stale".into();
        assert_eq!(
            VoxelMapAccumulator::new(map, contract).unwrap_err(),
            AccumulatorError::MapIdentityMismatch
        );
    }

    #[test]
    fn independent_updates_commute_for_chunk_state() {
        let a = update(-1, 0.75, 0.25);
        let b = update(-1, -0.5, 0.5);
        let mut ab = accumulator();
        ab.apply(&a).unwrap();
        ab.apply(&b).unwrap();
        let mut ba = accumulator();
        ba.apply(&b).unwrap();
        ba.apply(&a).unwrap();
        assert_eq!(
            ab.viewer_snapshot(f64::NEG_INFINITY).unwrap_err(),
            AccumulatorError::InvalidViewerThreshold
        );
        assert_eq!(
            ab.viewer_snapshot(-1_000.0).unwrap().chunks,
            ba.viewer_snapshot(-1_000.0).unwrap().chunks
        );
    }

    #[test]
    fn malformed_update_is_rejected_without_partial_mutation() {
        let mut map = accumulator();
        let mut malformed = update(0, 1.0, 0.5);
        malformed.voxel_chunks.push(VoxelChunkUpdate {
            chunk_x: 1,
            chunk_y: 0,
            chunk_z: 0,
            voxels: vec![VoxelDelta {
                x: 64,
                y: 0,
                z: 0,
                occupancy_delta: 1.0,
                semantics: vec![],
                color: None,
            }],
        });
        assert!(matches!(
            map.apply(&malformed),
            Err(AccumulatorError::VoxelOutsideChunk { .. })
        ));
        assert_eq!(map.revision(), 0);
        assert!(map.viewer_snapshot(0.0).unwrap().chunks.is_empty());
    }

    #[test]
    fn cancelling_evidence_removes_empty_chunk_for_incremental_viewer() {
        let mut map = accumulator();
        map.apply(&update(2, 1.0, 0.5)).unwrap();
        let summary = map.apply(&update(2, -1.0, -0.5)).unwrap();
        let coord = ChunkCoord { x: 2, y: 0, z: 0 };
        assert_eq!(summary.changed_chunks, vec![coord]);
        assert!(map.viewer_chunk(coord, 0.0).unwrap().is_none());
    }

    #[test]
    fn unknown_semantic_class_and_non_finite_evidence_are_rejected() {
        let mut map = accumulator();
        let mut bad_class = update(0, 1.0, 1.0);
        bad_class.voxel_chunks[0].voxels[0].semantics[0].class_id = 2;
        assert!(matches!(
            map.apply(&bad_class),
            Err(AccumulatorError::UnknownSemanticClass { .. })
        ));
        let mut nan = update(0, f32::NAN, 0.0);
        assert_eq!(map.apply(&nan), Err(AccumulatorError::NonFiniteEvidence));
        nan.voxel_chunks[0].voxels[0].occupancy_delta = 0.0;
        nan.voxel_chunks[0].voxels[0].semantics[0].evidence_delta = f32::INFINITY;
        assert_eq!(map.apply(&nan), Err(AccumulatorError::NonFiniteEvidence));
    }

    #[test]
    fn checkpoint_atomically_replaces_state_then_accepts_deltas() {
        let mut source = accumulator();
        source.apply(&update(4, 2.0, 0.75)).unwrap();
        let checkpoint = source.checkpoint_update();

        let mut replay = accumulator();
        replay.apply(&update(-2, 9.0, 1.0)).unwrap();
        let summary = replay.apply(&checkpoint).unwrap();
        assert_eq!(
            summary.changed_chunks,
            vec![
                ChunkCoord { x: -2, y: 0, z: 0 },
                ChunkCoord { x: 4, y: 0, z: 0 },
            ]
        );
        let mut replayed = replay.viewer_snapshot(-1_000.0).unwrap().chunks;
        let mut expected = source.viewer_snapshot(-1_000.0).unwrap().chunks;
        replayed.iter_mut().for_each(|chunk| chunk.revision = 0);
        expected.iter_mut().for_each(|chunk| chunk.revision = 0);
        assert_eq!(replayed, expected);

        replay.apply(&update(4, -0.5, 0.25)).unwrap();
        let voxel = &replay.viewer_snapshot(-1_000.0).unwrap().chunks[0].voxels[0];
        assert!((voxel.occupancy_evidence - 1.5).abs() < 1e-9);
        assert!((voxel.semantics[0].evidence - 1.0).abs() < 1e-9);
    }

    #[test]
    fn retained_window_from_checkpoint_matches_continuous_replay() {
        let first = update(0, 0.75, 0.25);
        let second = update(1, 1.25, 0.5);
        let tail = update(0, -0.5, 0.25);

        let mut continuous = accumulator();
        continuous.apply(&first).unwrap();
        continuous.apply(&second).unwrap();
        let checkpoint = continuous.checkpoint_update();
        continuous.apply(&checkpoint).unwrap();
        continuous.apply(&tail).unwrap();

        let mut retained = accumulator();
        retained.apply(&checkpoint).unwrap();
        retained.apply(&tail).unwrap();
        let mut replayed = retained.viewer_snapshot(-1_000.0).unwrap().chunks;
        let mut expected = continuous.viewer_snapshot(-1_000.0).unwrap().chunks;
        replayed.iter_mut().for_each(|chunk| chunk.revision = 0);
        expected.iter_mut().for_each(|chunk| chunk.revision = 0);
        assert_eq!(replayed, expected);
    }

    #[test]
    fn malformed_checkpoint_does_not_partially_replace_state() {
        let mut map = accumulator();
        map.apply(&update(0, 1.0, 0.5)).unwrap();
        let before = map.viewer_snapshot(-1_000.0).unwrap();
        let mut checkpoint = map.checkpoint_update();
        checkpoint.checkpoint.as_mut().unwrap().voxel_chunks[0].voxels[0].x = 64;
        assert!(matches!(
            map.apply(&checkpoint),
            Err(AccumulatorError::VoxelOutsideChunk { .. })
        ));
        assert_eq!(map.viewer_snapshot(-1_000.0).unwrap(), before);
    }

    fn portal_accumulator() -> PortalMapAccumulator {
        let contract = PortalMap {
            frame: RegistryRef {
                peer_id: "bracketbot".into(),
                id: "map".into(),
                hash: "map-frame-hash".into(),
            },
            observation_model: PortalObservationModel::AppendOnlyPoseObservations,
        };
        let map = MapRegistryEntry {
            peer_id: "park".into(),
            map_id: "portals".into(),
            body: MapBody::Portal(contract.clone()),
        }
        .registry_ref();
        PortalMapAccumulator::new(map, contract).unwrap()
    }

    fn portal_observation(index: u32, x: f64) -> PortalObservation {
        PortalObservation {
            portal_id: "portal:office".into(),
            physical_size_m: 0.2,
            portal_to_map: Some(SpatialTransform {
                translation: Some(Vec3 { x, y: 2.0, z: 3.0 }),
                orientation: Some(Quat {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                }),
            }),
            confidence: 0.98,
            normalized_corner_error: 0.01,
            source_peer_id: "bracketbot".into(),
            source_resource_id: "qr/head_left".into(),
            source_timestamp_ns: 42,
            source_sequence: 7,
            source_detection_index: index,
            camera_frame_peer_id: "bracketbot".into(),
            camera_frame_id: "head_left_camera_optical".into(),
            camera_frame_hash: "camera-frame-hash".into(),
        }
    }

    fn portal_update(observations: Vec<PortalObservation>) -> MapUpdate {
        MapUpdate {
            voxel_chunks: vec![],
            checkpoint: None,
            portal_observations: observations,
            portal_checkpoint: None,
        }
    }

    #[test]
    fn portal_observations_are_idempotent_and_sorted_by_provenance() {
        let mut map = portal_accumulator();
        let update = portal_update(vec![portal_observation(1, 2.0), portal_observation(0, 1.0)]);

        let first = map.apply(&update).unwrap();
        let replay = map.apply(&update).unwrap();
        assert_eq!(first.observations_added, 2);
        assert_eq!(first.revision, 1);
        assert_eq!(replay.observations_added, 0);
        assert_eq!(replay.revision, 1);
        assert_eq!(
            map.observations()
                .map(|observation| observation.source_detection_index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn equal_timestamps_from_distinct_log_sequences_are_distinct_observations() {
        let mut map = portal_accumulator();
        let first = portal_observation(0, 1.0);
        let mut second = portal_observation(0, 2.0);
        second.source_sequence += 1;

        let summary = map.apply(&portal_update(vec![first, second])).unwrap();

        assert_eq!(summary.observations_added, 2);
        assert_eq!(map.observations().count(), 2);
    }

    #[test]
    fn conflicting_portal_provenance_fails_without_mutation() {
        let mut map = portal_accumulator();
        map.apply(&portal_update(vec![portal_observation(0, 1.0)]))
            .unwrap();
        let conflict = portal_update(vec![portal_observation(0, 9.0)]);

        assert!(matches!(
            map.apply(&conflict),
            Err(PortalAccumulatorError::ConflictingObservation(_))
        ));
        assert_eq!(
            map.observations()
                .next()
                .unwrap()
                .portal_to_map
                .as_ref()
                .unwrap()
                .translation
                .as_ref()
                .unwrap()
                .x,
            1.0
        );
    }

    #[test]
    fn conflicting_canonical_portal_sizes_fail_without_mutation() {
        let mut map = portal_accumulator();
        map.apply(&portal_update(vec![portal_observation(0, 1.0)]))
            .unwrap();
        let mut conflicting_size = portal_observation(1, 2.0);
        conflicting_size.physical_size_m = 0.25;

        assert_eq!(
            map.apply(&portal_update(vec![conflicting_size])),
            Err(PortalAccumulatorError::ConflictingPortalSize(
                "portal:office".into()
            ))
        );
        assert_eq!(map.observations().count(), 1);
    }

    #[test]
    fn portal_checkpoint_round_trips_materialized_state() {
        let mut source = portal_accumulator();
        source
            .apply(&portal_update(vec![
                portal_observation(2, 3.0),
                portal_observation(0, 1.0),
            ]))
            .unwrap();

        let mut replay = portal_accumulator();
        let summary = replay.apply(&source.checkpoint_update()).unwrap();
        assert!(summary.checkpoint_applied);
        assert_eq!(
            replay.observations().cloned().collect::<Vec<_>>(),
            source.observations().cloned().collect::<Vec<_>>()
        );
    }

    #[test]
    fn voxel_and_portal_accumulators_reject_the_other_map_kind() {
        assert_eq!(
            accumulator().apply(&portal_update(vec![portal_observation(0, 1.0)])),
            Err(AccumulatorError::UnexpectedPortalData)
        );
        assert_eq!(
            portal_accumulator().apply(&update(0, 1.0, 0.0)),
            Err(PortalAccumulatorError::UnexpectedVoxelData)
        );
    }
}
