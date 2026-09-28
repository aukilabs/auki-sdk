//! Portal-aligned voxel evidence. Single-writer ingestion; frame identities are never inferred.
use auki_components::*;
use auki_datatypes::{
    map::MapUpdate,
    pose::{Quat, SpatialTransform, Vec3},
};
use auki_geometry::convention_matrix;
use auki_mappers::Voxelizer;
use auki_maps::VoxelMapAccumulator;
use auki_registry::{
    AxisDirection, FiniteF64, FrameRegistryEntry, Handedness, LengthUnit, MapBody,
    MapRegistryEntry, RegistryRef, VoxelMap, VoxelValueModel,
};
use auki_scenegraph::{
    MapDefinition, MapSnapshot, RigidTransform, UpAxis, component::SnapshotReference,
};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const VOXEL_SCHEMA: &str = "auki.voxel-map.snapshot/v2";
pub const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SOURCES: usize = 64;
pub const MAX_POINTS: usize = 1024;
pub const MAX_RAY_SAMPLES: usize = 65_536;
pub const MAX_VOXELS: usize = 65_536;

#[derive(Debug, thiserror::Error)]
#[error("voxel map: {0}")]
pub struct VoxelError(pub String);
fn error(e: impl std::fmt::Display) -> VoxelError {
    VoxelError(e.to_string())
}
fn require(ok: bool, message: &str) -> Result<(), VoxelError> {
    if ok { Ok(()) } else { Err(error(message)) }
}
fn text(s: &str) -> Result<(), VoxelError> {
    require(
        !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control),
        "invalid identifier",
    )
}
fn reference(r: &ProductReference) -> Result<(), VoxelError> {
    text(&r.peer_id)?;
    text(&r.product_id)?;
    text(&r.manifest_hash)
}
fn frame_ref(f: &FrameRegistryEntry) -> RegistryRef {
    RegistryRef {
        peer_id: f.peer_id.clone(),
        id: f.frame_id.clone(),
        hash: f.hash(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoxelMapDefinition {
    pub registry: MapRegistryEntry,
    /// The grid anchors grid index (0,0,0) at the declared map frame origin.
    pub grid_origin_m: [f64; 3],
    /// Same physical frame as the portal map, with an explicit full axis convention.
    pub frame: FrameRegistryEntry,
    pub portal_map: MapDefinition,
    pub portal_snapshot: SnapshotReference,
    /// Explicitly admitted sources. Transport identity and authorization are host responsibilities.
    pub sources: Vec<SensorBinding>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SensorBinding {
    pub sensor_product: ProductReference,
    pub sensor_frame: FrameRegistryEntry,
    pub observation_clock_id: String,
}
impl VoxelMapDefinition {
    /// Choose to use the portal map's frame directly; grid origin is explicitly [0,0,0].
    #[allow(clippy::too_many_arguments)]
    pub fn from_portal_map(
        owner: &str,
        map_id: &str,
        portal_snapshot: SnapshotReference,
        portal_map: &MapSnapshot,
        frame: FrameRegistryEntry,
        sensor_product: ProductReference,
        sensor_frame: FrameRegistryEntry,
        observation_clock_id: String,
        voxel_size_m: f64,
    ) -> Result<Self, VoxelError> {
        portal_map.validate().map_err(error)?;
        require(
            !portal_map.scenegraph.anchors.is_empty(),
            "portal map must have an anchor",
        )?;
        let registry = MapRegistryEntry {
            peer_id: owner.into(),
            map_id: map_id.into(),
            body: MapBody::Voxel(VoxelMap {
                frame: frame_ref(&frame),
                voxel_size_m: FiniteF64(voxel_size_m),
                chunk_dimension: 16,
                value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                color_model: None,
                semantic_classes: vec![],
            }),
        };
        let value = Self {
            registry,
            grid_origin_m: [0.; 3],
            frame,
            portal_map: portal_map.scenegraph.map.clone(),
            portal_snapshot,
            sources: vec![SensorBinding {
                sensor_product,
                sensor_frame,
                observation_clock_id,
            }],
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), VoxelError> {
        require(
            self.grid_origin_m == [0.; 3],
            "grid origin must explicitly be zero",
        )?;
        text(&self.registry.peer_id)?;
        text(&self.registry.map_id)?;
        require(
            !self.sources.is_empty() && self.sources.len() <= MAX_SOURCES,
            "invalid source count",
        )?;
        for (index, source) in self.sources.iter().enumerate() {
            reference(&source.sensor_product)?;
            text(&source.observation_clock_id)?;
            require(
                !self.sources[..index]
                    .iter()
                    .any(|s| s.sensor_product == source.sensor_product),
                "duplicate source binding",
            )?;
        }
        reference(&self.portal_snapshot.product)?;
        auki_scenegraph::Scenegraph::new(self.portal_map.clone()).map_err(error)?;
        for f in std::iter::once(&self.frame).chain(self.sources.iter().map(|s| &s.sensor_frame)) {
            text(&f.peer_id)?;
            text(&f.frame_id)?;
            f.validate().map_err(error)?;
            convention_matrix(f, f).map_err(error)?;
            require(
                f.units == LengthUnit::Meters && f.handedness == Handedness::Right,
                "adapter requires right-handed meter frames",
            )?;
        }
        let up = match self.portal_map.frame.up_axis {
            UpAxis::Y => self.frame.axes.y,
            UpAxis::Z => self.frame.axes.z,
        };
        require(
            self.frame.frame_id == self.portal_map.frame.id
                && self.portal_map.frame.meters_per_unit == 1.
                && up == AxisDirection::Up,
            "voxel frame must explicitly match the portal-map frame and up axis",
        )?;
        let MapBody::Voxel(grid) = &self.registry.body;
        require(
            grid.frame == frame_ref(&self.frame)
                && (0.01..=10.).contains(&grid.voxel_size_m.0)
                && grid.chunk_dimension == 16,
            "invalid grid contract",
        )?;
        require(
            grid.color_model.is_none() && grid.semantic_classes.is_empty(),
            "adapter is occupancy only",
        )?;
        VoxelMapAccumulator::new(self.registry.registry_ref(), grid.clone()).map_err(error)?;
        Ok(())
    }
    pub fn grid(&self) -> &VoxelMap {
        let MapBody::Voxel(grid) = &self.registry.body;
        grid
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimedSensorPose {
    pub timestamp_ns: u64,
    pub clock_id: String,
    pub sensor_to_map: RigidTransform,
    /// Exact portal-map state used to establish this pose; corrections require rebuilding.
    pub portal_snapshot: SnapshotReference,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DepthObservation {
    pub source: ProductReference,
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub clock_id: String,
    pub sensor_frame_id: String,
    /// Measured hit endpoints in meters, relative to the sensor optical centre.
    /// No-return/max-range values must not be represented as occupied hits.
    pub hit_points_m: Vec<[f64; 3]>,
    pub pose: TimedSensorPose,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservationProvenance {
    pub source: ProductReference,
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub clock_id: String,
    pub pose: TimedSensorPose,
}
impl From<&DepthObservation> for ObservationProvenance {
    fn from(o: &DepthObservation) -> Self {
        Self {
            source: o.source.clone(),
            sequence: o.sequence,
            timestamp_ns: o.timestamp_ns,
            clock_id: o.clock_id.clone(),
            pose: o.pose.clone(),
        }
    }
}
/// Pure adapter to the existing voxelizer. The host supplies decoded depth and a time-matched pose.
pub struct PortalVoxelMapper {
    definition: VoxelMapDefinition,
}
impl PortalVoxelMapper {
    pub fn new(definition: VoxelMapDefinition) -> Result<Self, VoxelError> {
        definition.validate()?;
        Ok(Self { definition })
    }
    pub fn map_observation(&self, o: &DepthObservation) -> Result<MapUpdate, VoxelError> {
        let d = &self.definition;
        let binding = d
            .sources
            .iter()
            .find(|s| s.sensor_product == o.source)
            .ok_or_else(|| error("unadmitted sensor Product"))?;
        require(
            o.sensor_frame_id == binding.sensor_frame.frame_id,
            "sensor binding mismatch",
        )?;
        require(
            o.clock_id == binding.observation_clock_id
                && o.pose.clock_id == o.clock_id
                && o.pose.timestamp_ns == o.timestamp_ns,
            "pose must match the capture timestamp and clock exactly",
        )?;
        require(
            o.pose.portal_snapshot == d.portal_snapshot,
            "portal-map provenance changed; rebuild with corrected poses",
        )?;
        let t = &o.pose.sensor_to_map;
        require(
            t.from_frame_id == binding.sensor_frame.frame_id && t.to_frame_id == d.frame.frame_id,
            "pose frame endpoints do not match the bound sensor and map",
        )?;
        require(
            t.translation
                .iter()
                .chain(t.rotation_wxyz.iter())
                .all(|x| x.is_finite())
                && (t.rotation_wxyz.iter().map(|x| x * x).sum::<f64>() - 1.).abs() < 1e-9,
            "invalid rigid pose",
        )?;
        require(
            !o.hit_points_m.is_empty() && o.hit_points_m.len() <= MAX_POINTS,
            "empty or oversized depth observation",
        )?;
        let mut steps = 0usize;
        for p in &o.hit_points_m {
            let range = p[0].hypot(p[1]).hypot(p[2]);
            require(
                p.iter().all(|x| x.is_finite()) && range > 0. && range <= 100.,
                "invalid or excessive measured range",
            )?;
            steps = steps.saturating_add(3 * (range / d.grid().voxel_size_m.0).ceil() as usize + 3);
            require(steps <= MAX_RAY_SAMPLES, "ray work bound exceeded")?;
        }
        let [x, y, z] = t.translation;
        let [w, qx, qy, qz] = t.rotation_wxyz;
        let numeric = SpatialTransform {
            translation: Some(Vec3 { x, y, z }),
            orientation: Some(Quat {
                x: qx,
                y: qy,
                z: qz,
                w,
            }),
        };
        Voxelizer::new(d.grid().voxel_size_m.0, d.grid().chunk_dimension)
            .map_err(error)?
            .map_sensor_rays(
                o.hit_points_m.iter().map(|p| Vec3 {
                    x: p[0],
                    y: p[1],
                    z: p[2],
                }),
                &numeric,
                -1.,
                1.,
            )
            .map_err(error)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoxelSnapshot {
    pub definition: VoxelMapDefinition,
    /// Existing SDK MapUpdate protobuf containing a full checkpoint (not additive deltas).
    pub checkpoint: Vec<u8>,
    pub integrated_observations: u64,
    pub latest_observation: Option<ObservationProvenance>,
}
impl ContractType for VoxelSnapshot {
    const DATATYPE: &'static str = VOXEL_SCHEMA;
}
impl VoxelSnapshot {
    /// Inspection export of an explicitly matching Portal map and positive voxel
    /// evidence. No alignment is inferred; free/unknown cells are not rendered.
    pub fn to_usda_with_portals(
        &self,
        portals: &MapSnapshot,
        reference: &SnapshotReference,
    ) -> Result<String, VoxelError> {
        use std::fmt::Write;
        portals.validate().map_err(error)?;
        let view = self.accumulator()?.viewer_snapshot(0.).map_err(error)?;
        require(
            reference == &self.definition.portal_snapshot
                && portals.scenegraph.map == self.definition.portal_map,
            "Portal snapshot/frame does not match voxel-map alignment",
        )?;
        let mut stage = portals
            .scenegraph
            .to_usda_with_portal_geometry()
            .map_err(error)?;
        // The canonical exporter owns the enclosing /Map root; add a sibling
        // voxel layer inside that root, with no extra spatial transform.
        require(stage.ends_with("}\n"), "invalid stage root")?;
        stage.truncate(stage.len() - 2);
        let grid = self.definition.grid();
        let size = grid.voxel_size_m.0;
        writeln!(stage, "    def Xform \"Voxels\"\n    {{\n        custom string auki:frameId = {}\n        custom double auki:voxelSizeMeters = {size}",
            serde_json::to_string(&self.definition.frame.frame_id).map_err(error)?).unwrap();
        let mut index = 0;
        for chunk in view.chunks {
            for voxel in chunk.voxels {
                let [x, y, z] = voxel.center_m;
                writeln!(stage, "        def Cube \"Cell_{index}\"\n        {{\n            double size = {size}\n            double3 xformOp:translate = ({x}, {y}, {z})\n            uniform token[] xformOpOrder = [\"xformOp:translate\"]\n            color3f[] primvars:displayColor = [(0.12, 0.7, 0.8)]\n            custom double auki:occupancyEvidence = {}\n        }}", voxel.occupancy_evidence).unwrap();
                index += 1;
            }
        }
        stage.push_str("    }\n}\n");
        Ok(stage)
    }
    pub fn encoded_size(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |v| v.len())
    }
    pub fn accumulator(&self) -> Result<VoxelMapAccumulator, VoxelError> {
        self.definition.validate()?;
        require(
            self.encoded_size() <= MAX_SNAPSHOT_BYTES,
            "snapshot too large",
        )?;
        let update = MapUpdate::decode(self.checkpoint.as_slice()).map_err(error)?;
        require(
            update.checkpoint.is_some() && update.voxel_chunks.is_empty(),
            "snapshot requires a full checkpoint",
        )?;
        let count = update
            .checkpoint
            .as_ref()
            .unwrap()
            .voxel_chunks
            .iter()
            .map(|c| c.voxels.len())
            .sum::<usize>();
        require(count <= MAX_VOXELS, "voxel capacity exceeded")?;
        require(
            (self.integrated_observations == 0) == self.latest_observation.is_none(),
            "inconsistent observation provenance",
        )?;
        require(
            self.integrated_observations != 0 || count == 0,
            "empty history cannot contain voxel evidence",
        )?;
        if let Some(last) = &self.latest_observation {
            let binding = self
                .definition
                .sources
                .iter()
                .find(|s| s.sensor_product == last.source)
                .ok_or_else(|| error("unadmitted snapshot source"))?;
            let pose = &last.pose.sensor_to_map;
            require(
                pose.translation
                    .iter()
                    .chain(pose.rotation_wxyz.iter())
                    .all(|v| v.is_finite())
                    && (pose.rotation_wxyz.iter().map(|v| v * v).sum::<f64>() - 1.).abs() < 1e-9,
                "invalid snapshot pose",
            )?;
            require(
                last.clock_id == binding.observation_clock_id
                    && last.pose.clock_id == last.clock_id
                    && last.pose.timestamp_ns == last.timestamp_ns
                    && last.pose.portal_snapshot == self.definition.portal_snapshot
                    && last.pose.sensor_to_map.from_frame_id == binding.sensor_frame.frame_id
                    && last.pose.sensor_to_map.to_frame_id == self.definition.frame.frame_id,
                "invalid snapshot pose provenance",
            )?;
        }
        let mut accumulator = VoxelMapAccumulator::new(
            self.definition.registry.registry_ref(),
            self.definition.grid().clone(),
        )
        .map_err(error)?;
        accumulator.apply(&update).map_err(error)?;
        Ok(accumulator)
    }
    fn metadata(&self, sequence: u64) -> Result<Option<CatalogProductMetadata>, String> {
        let metadata = CatalogProductMetadata {
            schema: "auki.voxel-map.catalog/v2".into(),
            source_sequence: sequence,
            value: serde_json::json!({"definition":self.definition,"integrated_observations":self.integrated_observations}),
        };
        metadata.validate()?;
        Ok(Some(metadata))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrationResult {
    Applied,
    Duplicate,
}
/// Local single-writer Component. Host controls ingestion; only snapshots are exported.
pub struct VoxelMapComponent {
    component: Component,
    mapper: PortalVoxelMapper,
    accumulator: VoxelMapAccumulator,
    output: ConfiguredObservable<VoxelSnapshot>,
    capture: BufferProductCapture<VoxelSnapshot>,
    current: Observation<VoxelSnapshot>,
    last: Vec<Option<DepthObservation>>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    closed: bool,
}
impl VoxelMapComponent {
    pub fn new(
        runtime: &ComponentRuntime,
        component_id: &str,
        publication_id: &str,
        publication_clock_id: &str,
        definition: VoxelMapDefinition,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Result<Self, VoxelError> {
        text(publication_id)?;
        text(publication_clock_id)?;
        let mapper = PortalVoxelMapper::new(definition.clone())?;
        let accumulator = VoxelMapAccumulator::new(
            definition.registry.registry_ref(),
            definition.grid().clone(),
        )
        .map_err(error)?;
        let initial = VoxelSnapshot {
            definition: definition.clone(),
            checkpoint: accumulator.checkpoint_update().encode_to_vec(),
            integrated_observations: 0,
            latest_observation: None,
        };
        initial.accumulator()?;
        initial.metadata(u64::MAX).map_err(error)?;
        let component = runtime
            .component(
                ComponentSpec::new(component_id).observable(ObservableContract {
                    name: "voxels".into(),
                    datatype: VOXEL_SCHEMA.into(),
                    schema: VOXEL_SCHEMA.into(),
                    access: vec![ObservationAccess::FollowNew],
                    exposure: Exposure::Cluster,
                }),
            )
            .map_err(error)?;
        let output = component
            .configured_observable::<VoxelSnapshot>(
                ConfiguredObservableSpec::new(
                    "voxels",
                    format!("{publication_id}/voxels"),
                    publication_clock_id,
                    PayloadContract::Structured(StructuredPayloadContract {
                        modality: "voxel_map".into(),
                        datatype: VOXEL_SCHEMA.into(),
                        schema: VOXEL_SCHEMA.into(),
                        observes: definition.registry.map_id.clone(),
                        unit: None,
                    }),
                )
                .in_spatial_frame(definition.frame.frame_id.clone()),
            )
            .map_err(error)?;
        require(
            component.manifest().peer_id == definition.registry.peer_id,
            "map owner differs from component peer",
        )?;
        component.expose().map_err(error)?;
        let capture = runtime
            .capture_buffer_with_metadata(
                format!("{publication_id}/snapshots"),
                &output,
                BufferLimits::entries(1),
                VoxelSnapshot::encoded_size,
                |o| o.payload.metadata(o.sequence),
            )
            .map_err(error)?;
        let (current, _) = output.publish(clock(), Arc::new(initial)).map_err(error)?;
        require(capture.errors().is_empty(), "initial retention failed")?;
        Ok(Self {
            component,
            mapper,
            accumulator,
            output,
            capture,
            current,
            last: vec![None; definition.sources.len()],
            clock: Box::new(clock),
            closed: false,
        })
    }
    pub fn integrate(
        &mut self,
        observation: DepthObservation,
    ) -> Result<IntegrationResult, VoxelError> {
        require(!self.closed, "map closed")?;
        let source_index = self
            .mapper
            .definition
            .sources
            .iter()
            .position(|s| s.sensor_product == observation.source)
            .ok_or_else(|| error("unadmitted sensor Product"))?;
        if let Some(last) = &self.last[source_index] {
            if last == &observation {
                return Ok(IntegrationResult::Duplicate);
            }
            require(
                observation.sequence > last.sequence
                    && observation.timestamp_ns > last.timestamp_ns,
                "replayed, revised or out-of-order observation",
            )?;
        }
        let update = self.mapper.map_observation(&observation)?;
        let mut next = self.accumulator.clone();
        next.apply(&update).map_err(error)?;
        let snapshot = VoxelSnapshot {
            definition: self.mapper.definition.clone(),
            checkpoint: next.checkpoint_update().encode_to_vec(),
            integrated_observations: self
                .current
                .payload
                .integrated_observations
                .checked_add(1)
                .ok_or_else(|| error("observation count overflow"))?,
            latest_observation: Some((&observation).into()),
        };
        snapshot.accumulator()?;
        snapshot.metadata(u64::MAX).map_err(error)?;
        let timestamp = (self.clock)();
        require(
            timestamp > self.current.timestamp_ns,
            "publication clock must advance",
        )?;
        let (published, _) = self
            .output
            .publish(timestamp, Arc::new(snapshot))
            .map_err(error)?;
        if self.capture.product().buffer().range().last_sequence != Some(published.sequence) {
            self.close();
            return Err(error("retention failed; map closed"));
        }
        self.accumulator = next;
        self.current = published;
        self.last[source_index] = Some(observation);
        Ok(IntegrationResult::Applied)
    }
    pub fn product(&self) -> RetainedProduct<VoxelSnapshot> {
        self.capture.product()
    }
    pub fn component(&self) -> &Component {
        &self.component
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.capture.cancel();
    }
}
impl Drop for VoxelMapComponent {
    fn drop(&mut self) {
        self.close();
    }
}
