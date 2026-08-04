//! Renderer-neutral conversion from accumulated chunks to cube instances.

use crate::{ApplySummary, ChunkCoord, LocalVoxelCoord, ViewerChunkSnapshot, VoxelMapAccumulator};

/// Appearance controls shared by native and web voxel renderers.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelViewerStyle {
    /// Evidence at or below this value maps to `low_evidence_rgba`.
    pub minimum_occupancy_evidence: f64,
    /// Evidence at or above this value maps to `high_evidence_rgba`.
    pub maximum_occupancy_evidence: f64,
    /// RGBA color at the low end of the evidence range.
    pub low_evidence_rgba: [f32; 4],
    /// RGBA color at the high end of the evidence range.
    pub high_evidence_rgba: [f32; 4],
    /// Cube edge as a fraction of voxel size. Values below one leave a grid
    /// gap that makes individual voxels legible.
    pub cube_fill_fraction: f32,
    /// Optional RGBA color indexed by semantic class id.
    pub semantic_palette: Vec<[f32; 4]>,
    /// Constant blend from occupancy color toward the dominant semantic color.
    pub semantic_blend: f32,
}

impl Default for VoxelViewerStyle {
    fn default() -> Self {
        Self {
            minimum_occupancy_evidence: 0.0,
            maximum_occupancy_evidence: 3.0,
            low_evidence_rgba: [0.12, 0.42, 1.0, 0.28],
            high_evidence_rgba: [1.0, 0.18, 0.04, 1.0],
            cube_fill_fraction: 0.92,
            semantic_palette: Vec::new(),
            semantic_blend: 0.65,
        }
    }
}

impl VoxelViewerStyle {
    fn validate(&self) -> Result<(), ViewerAdapterError> {
        if !self.minimum_occupancy_evidence.is_finite()
            || !self.maximum_occupancy_evidence.is_finite()
            || self.maximum_occupancy_evidence <= self.minimum_occupancy_evidence
        {
            return Err(ViewerAdapterError::InvalidEvidenceRange);
        }
        if !self.cube_fill_fraction.is_finite()
            || self.cube_fill_fraction <= 0.0
            || self.cube_fill_fraction > 1.0
        {
            return Err(ViewerAdapterError::InvalidCubeFillFraction);
        }
        if !self.semantic_blend.is_finite() || !(0.0..=1.0).contains(&self.semantic_blend) {
            return Err(ViewerAdapterError::InvalidSemanticBlend);
        }
        if !valid_color(self.low_evidence_rgba)
            || !valid_color(self.high_evidence_rgba)
            || !self.semantic_palette.iter().copied().all(valid_color)
        {
            return Err(ViewerAdapterError::InvalidColor);
        }
        Ok(())
    }
}

fn valid_color(color: [f32; 4]) -> bool {
    color
        .into_iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
}

/// One unit-cube instance ready for a renderer's instance buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelInstance {
    /// Stable chunk identity used for batching and picking.
    pub chunk: ChunkCoord,
    /// Stable voxel identity within the chunk.
    pub local: LocalVoxelCoord,
    /// Translation of the cube centre in map-frame metres.
    pub center_m: [f32; 3],
    /// Uniform cube edge length in metres.
    pub edge_length_m: f32,
    /// Final occupancy/semantic color.
    pub rgba: [f32; 4],
    /// Unclamped occupancy evidence for tooltips and alternate shaders.
    pub occupancy_evidence: f32,
    /// Semantic class with the strongest positive evidence, if one exists.
    pub dominant_semantic_class: Option<u32>,
}

/// Incremental operation consumed by the voxel renderer.
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkRenderUpdate {
    /// Replace all instances for this chunk. Chunk-sized replacement keeps GPU
    /// buffer ownership simple and bounds remeshing work after each update.
    Replace {
        /// Signed map chunk coordinate.
        coord: ChunkCoord,
        /// Accumulator revision represented by the instances.
        revision: u64,
        /// Complete current instance list for the chunk.
        instances: Vec<VoxelInstance>,
    },
    /// Remove this chunk's instance buffer because it is empty or entirely
    /// below the current viewer threshold.
    Remove {
        /// Signed map chunk coordinate.
        coord: ChunkCoord,
        /// Accumulator revision that caused the removal.
        revision: u64,
    },
}

/// Stateless conversion from accumulator snapshots to renderer operations.
#[derive(Debug, Clone)]
pub struct VoxelViewerAdapter {
    style: VoxelViewerStyle,
}

impl VoxelViewerAdapter {
    /// Create an adapter after validating all style parameters.
    pub fn new(style: VoxelViewerStyle) -> Result<Self, ViewerAdapterError> {
        style.validate()?;
        Ok(Self { style })
    }

    /// Current appearance controls.
    pub fn style(&self) -> &VoxelViewerStyle {
        &self.style
    }

    /// Convert exactly the chunks named by an accumulator apply result. The
    /// returned operations are sorted in the same order as `changed_chunks`.
    pub fn changed_chunks(
        &self,
        accumulator: &VoxelMapAccumulator,
        applied: &ApplySummary,
    ) -> Result<Vec<ChunkRenderUpdate>, ViewerAdapterError> {
        let mut updates = Vec::with_capacity(applied.changed_chunks.len());
        for coord in &applied.changed_chunks {
            let snapshot = accumulator
                .viewer_chunk(*coord, self.style.minimum_occupancy_evidence)
                .map_err(ViewerAdapterError::Accumulator)?;
            updates.push(match snapshot {
                Some(chunk) => {
                    let revision = chunk.revision;
                    let instances =
                        self.chunk_instances(&chunk, accumulator.contract().voxel_size_m.0)?;
                    if instances.is_empty() {
                        ChunkRenderUpdate::Remove {
                            coord: *coord,
                            revision,
                        }
                    } else {
                        ChunkRenderUpdate::Replace {
                            coord: *coord,
                            revision,
                            instances,
                        }
                    }
                }
                None => ChunkRenderUpdate::Remove {
                    coord: *coord,
                    revision: applied.revision,
                },
            });
        }
        Ok(updates)
    }

    /// Convert a detached chunk snapshot into a complete instance list.
    pub fn chunk_instances(
        &self,
        chunk: &ViewerChunkSnapshot,
        voxel_size_m: f64,
    ) -> Result<Vec<VoxelInstance>, ViewerAdapterError> {
        let edge = f64_to_f32(voxel_size_m * f64::from(self.style.cube_fill_fraction))?;
        let evidence_span =
            self.style.maximum_occupancy_evidence - self.style.minimum_occupancy_evidence;
        chunk
            .voxels
            .iter()
            .filter(|voxel| voxel.occupancy_evidence >= self.style.minimum_occupancy_evidence)
            .map(|voxel| {
                let t = ((voxel.occupancy_evidence - self.style.minimum_occupancy_evidence)
                    / evidence_span)
                    .clamp(0.0, 1.0) as f32;
                let mut rgba = lerp_color(
                    self.style.low_evidence_rgba,
                    self.style.high_evidence_rgba,
                    t,
                );
                let dominant_semantic_class = voxel
                    .semantics
                    .iter()
                    .filter(|semantic| semantic.evidence > 0.0)
                    .max_by(|left, right| left.evidence.total_cmp(&right.evidence))
                    .map(|semantic| semantic.class_id);
                if let Some(semantic_color) = dominant_semantic_class
                    .and_then(|class_id| self.style.semantic_palette.get(class_id as usize))
                {
                    rgba = lerp_color(rgba, *semantic_color, self.style.semantic_blend);
                }
                Ok(VoxelInstance {
                    chunk: chunk.coord,
                    local: voxel.local,
                    center_m: [
                        f64_to_f32(voxel.center_m[0])?,
                        f64_to_f32(voxel.center_m[1])?,
                        f64_to_f32(voxel.center_m[2])?,
                    ],
                    edge_length_m: edge,
                    rgba,
                    occupancy_evidence: f64_to_f32(voxel.occupancy_evidence)?,
                    dominant_semantic_class,
                })
            })
            .collect()
    }
}

fn f64_to_f32(value: f64) -> Result<f32, ViewerAdapterError> {
    let converted = value as f32;
    if value.is_finite() && converted.is_finite() {
        Ok(converted)
    } else {
        Err(ViewerAdapterError::CoordinateNotRepresentable)
    }
}

fn lerp_color(low: [f32; 4], high: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|index| low[index] + (high[index] - low[index]) * t)
}

/// Invalid viewer style, snapshot coordinate, or accumulator query.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ViewerAdapterError {
    /// Evidence bounds are non-finite or not increasing.
    #[error("viewer evidence range must be finite and increasing")]
    InvalidEvidenceRange,
    /// Cube fill must be in `(0, 1]`.
    #[error("viewer cube fill fraction must be in (0, 1]")]
    InvalidCubeFillFraction,
    /// Semantic blend must be in `[0, 1]`.
    #[error("viewer semantic blend must be in [0, 1]")]
    InvalidSemanticBlend,
    /// A color component is non-finite or outside `[0, 1]`.
    #[error("viewer RGBA components must be finite and in [0, 1]")]
    InvalidColor,
    /// A metre-space value cannot be safely placed in a GPU `f32` buffer.
    #[error("viewer coordinate is not representable as finite f32")]
    CoordinateNotRepresentable,
    /// Accumulator snapshot query failed.
    #[error("accumulator: {0}")]
    Accumulator(#[source] crate::AccumulatorError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_datatypes::map::{MapUpdate, SemanticDelta, VoxelChunkUpdate, VoxelDelta};
    use auki_registry::{
        FiniteF64, MapBody, MapRegistryEntry, RegistryRef, VoxelMap, VoxelValueModel,
    };

    fn accumulator() -> VoxelMapAccumulator {
        let contract = VoxelMap {
            frame: RegistryRef {
                peer_id: "galbot".into(),
                id: "world".into(),
                hash: "frame-hash".into(),
            },
            voxel_size_m: FiniteF64(0.5),
            chunk_dimension: 64,
            value_model: VoxelValueModel::AdditiveOccupancyEvidence,
            semantic_classes: vec!["wall".into(), "door".into()],
        };
        let map = MapRegistryEntry {
            peer_id: "galbot".into(),
            map_id: "occupancy".into(),
            body: MapBody::Voxel(contract.clone()),
        }
        .registry_ref();
        VoxelMapAccumulator::new(map, contract).unwrap()
    }

    fn update(occupancy: f32, wall: f32, door: f32) -> MapUpdate {
        MapUpdate {
            voxel_chunks: vec![VoxelChunkUpdate {
                chunk_x: -1,
                chunk_y: 0,
                chunk_z: 0,
                voxels: vec![VoxelDelta {
                    x: 63,
                    y: 1,
                    z: 2,
                    occupancy_delta: occupancy,
                    semantics: vec![
                        SemanticDelta {
                            class_id: 0,
                            evidence_delta: wall,
                        },
                        SemanticDelta {
                            class_id: 1,
                            evidence_delta: door,
                        },
                    ],
                }],
            }],
            checkpoint: None,
        }
    }

    #[test]
    fn changed_chunk_becomes_pickable_cube_instance() {
        let mut map = accumulator();
        let applied = map.apply(&update(1.5, 0.25, 1.0)).unwrap();
        let adapter = VoxelViewerAdapter::new(VoxelViewerStyle {
            minimum_occupancy_evidence: 0.0,
            maximum_occupancy_evidence: 3.0,
            low_evidence_rgba: [0.0, 0.0, 1.0, 0.25],
            high_evidence_rgba: [1.0, 0.0, 0.0, 1.0],
            cube_fill_fraction: 0.8,
            semantic_palette: vec![[0.0, 1.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0]],
            semantic_blend: 1.0,
        })
        .unwrap();

        let updates = adapter.changed_chunks(&map, &applied).unwrap();
        let [
            ChunkRenderUpdate::Replace {
                coord,
                revision,
                instances,
            },
        ] = updates.as_slice()
        else {
            panic!("expected one chunk replacement")
        };
        assert_eq!(*coord, ChunkCoord { x: -1, y: 0, z: 0 });
        assert_eq!(*revision, 1);
        assert_eq!(instances[0].center_m, [-0.25, 0.75, 1.25]);
        assert_eq!(instances[0].edge_length_m, 0.4);
        assert_eq!(instances[0].dominant_semantic_class, Some(1));
        assert_eq!(instances[0].rgba, [1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn cancelling_chunk_emits_remove_operation() {
        let mut map = accumulator();
        map.apply(&update(1.0, 1.0, 0.0)).unwrap();
        let applied = map.apply(&update(-1.0, -1.0, 0.0)).unwrap();
        let adapter = VoxelViewerAdapter::new(VoxelViewerStyle::default()).unwrap();
        assert!(matches!(
            adapter.changed_chunks(&map, &applied).unwrap().as_slice(),
            [ChunkRenderUpdate::Remove {
                coord: ChunkCoord { x: -1, y: 0, z: 0 },
                revision: 2
            }]
        ));
    }

    #[test]
    fn invalid_style_is_rejected_before_rendering() {
        let style = VoxelViewerStyle {
            cube_fill_fraction: 1.1,
            ..Default::default()
        };
        assert_eq!(
            VoxelViewerAdapter::new(style).unwrap_err(),
            ViewerAdapterError::InvalidCubeFillFraction
        );
    }
}
