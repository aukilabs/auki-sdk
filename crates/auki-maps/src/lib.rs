//! Deterministic, sparse Map state built by replaying SDK [`MapUpdate`]s.
//!
//! Mappers produce updates; this crate consumes them. It deliberately has no
//! network, robot, ROS, renderer, or storage dependency. A viewer can replay a
//! local or remote Map Log into [`VoxelMapAccumulator`] and request either a
//! complete [`ViewerSnapshot`] or only the chunks named by [`ApplySummary`].

use std::collections::{BTreeMap, BTreeSet};

use auki_datatypes::map::{
    MapUpdate, SemanticEvidence, VoxelChunkSnapshot, VoxelMapCheckpoint, VoxelSnapshot,
};
use auki_registry::{MapBody, MapRegistryEntry, RegistryRef, VoxelMap, VoxelValueModel};

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

#[derive(Debug, Clone, PartialEq)]
struct VoxelEvidence {
    occupancy: f64,
    semantics: BTreeMap<u32, f64>,
}

impl Default for VoxelEvidence {
    fn default() -> Self {
        Self {
            occupancy: 0.0,
            semantics: BTreeMap::new(),
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
                        .any(|semantic| semantic.evidence_delta != 0.0);
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
                                (voxel.occupancy == 0.0 && voxel.semantics.is_empty())
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
                            })
                            .collect(),
                    })
                    .collect(),
            }),
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
                if snapshot.occupancy_evidence != 0.0 || !semantics.is_empty() {
                    voxels.insert(
                        LocalVoxelCoord {
                            x: snapshot.x,
                            y: snapshot.y,
                            z: snapshot.z,
                        },
                        VoxelEvidence {
                            occupancy: snapshot.occupancy_evidence,
                            semantics,
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
    use auki_datatypes::map::{SemanticDelta, VoxelChunkUpdate, VoxelDelta};
    use auki_registry::FiniteF64;

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
                }],
            }],
            checkpoint: None,
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
}
