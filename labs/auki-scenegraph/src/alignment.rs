//! Incremental map alignment discovery. The host supplies authorized data; no fetching or merging.
use crate::{
    MapSnapshot, RigidTransform, Scenegraph, catalog::MapCatalogData, component::SnapshotReference,
};
use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_geometry::{compose_spatial_transforms, inverse_spatial_transform};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX_ALIGNMENT_MAPS: usize = 64;
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentOptions {
    pub automatic: bool,
    pub translation_tolerance_m: f64,
    pub rotation_tolerance_rad: f64,
}
impl Default for AlignmentOptions {
    fn default() -> Self {
        Self {
            automatic: false,
            translation_tolerance_m: 0.02,
            rotation_tolerance_rad: 0.02,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentStep {
    pub from_map: String,
    pub to_map: String,
    pub from_snapshot: SnapshotReference,
    pub to_snapshot: SnapshotReference,
    pub portal_ids: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum AlignmentResult {
    NoConnection,
    /// Catalog overlap is not proof of physical alignment. Fetch current anchor poses.
    PotentialConnection {
        maps: Vec<String>,
        path: Vec<AlignmentStep>,
        reason: String,
    },
    /// A geometric estimate, not approval to merge or proof that portals have not moved.
    Available {
        transform: RigidTransform,
        path: Vec<AlignmentStep>,
    },
    Conflict {
        reason: String,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentEvent {
    pub source_map: String,
    pub target_map: String,
    pub result: AlignmentResult,
}
#[derive(Debug, thiserror::Error)]
#[error("alignment: {0}")]
pub struct AlignmentError(pub String);
#[derive(Clone, PartialEq)]
struct Record {
    reference: SnapshotReference,
    catalog: MapCatalogData,
    snapshot: Option<MapSnapshot>,
}
#[derive(Clone)]
struct Edge {
    transform: RigidTransform,
    step: AlignmentStep,
}
struct Graph {
    potential: BTreeMap<String, Vec<String>>,
    edges: BTreeMap<String, Vec<Edge>>,
    conflicts: BTreeSet<String>,
}
/// Bounded, synchronous host-owned index. Feed catalog/snapshot updates after authorization.
/// Returned events are the notification API; forwarding them to UI/Observables is host policy.
pub struct MapAlignmentChecker {
    options: AlignmentOptions,
    records: BTreeMap<String, Record>,
    previous: BTreeMap<(String, String), AlignmentResult>,
}
impl MapAlignmentChecker {
    pub fn new(options: AlignmentOptions) -> Result<Self, AlignmentError> {
        if !options.translation_tolerance_m.is_finite()
            || options.translation_tolerance_m < 0.
            || !options.rotation_tolerance_rad.is_finite()
            || !(0. ..=std::f64::consts::PI).contains(&options.rotation_tolerance_rad)
        {
            return Err(AlignmentError("invalid tolerances".into()));
        }
        Ok(Self {
            options,
            records: BTreeMap::new(),
            previous: BTreeMap::new(),
        })
    }
    pub fn receive_catalog(
        &mut self,
        key: impl Into<String>,
        reference: SnapshotReference,
        catalog: MapCatalogData,
    ) -> Result<Vec<AlignmentEvent>, AlignmentError> {
        self.receive(
            key.into(),
            Record {
                reference,
                catalog,
                snapshot: None,
            },
        )
    }
    pub fn receive_snapshot(
        &mut self,
        key: impl Into<String>,
        reference: SnapshotReference,
        snapshot: MapSnapshot,
    ) -> Result<Vec<AlignmentEvent>, AlignmentError> {
        snapshot
            .validate()
            .map_err(|e| AlignmentError(e.to_string()))?;
        self.receive(
            key.into(),
            Record {
                reference,
                catalog: MapCatalogData::from_snapshot(&snapshot),
                snapshot: Some(snapshot),
            },
        )
    }
    fn receive(
        &mut self,
        key: String,
        mut record: Record,
    ) -> Result<Vec<AlignmentEvent>, AlignmentError> {
        if key.is_empty() || key.len() > 256 || key.chars().any(char::is_control) {
            return Err(AlignmentError("invalid map key".into()));
        }
        for value in [
            &record.reference.product.peer_id,
            &record.reference.product.product_id,
            &record.reference.product.manifest_hash,
        ] {
            crate::text(value, 1024).map_err(|e| AlignmentError(e.to_string()))?;
        }
        Scenegraph::new(record.catalog.map.clone()).map_err(|e| AlignmentError(e.to_string()))?;
        record
            .catalog
            .metadata(record.reference.sequence)
            .map_err(AlignmentError)?;
        if record.catalog.portals.len() > crate::MAX_ANCHORS {
            return Err(AlignmentError("too many portals".into()));
        }
        let mut ids = BTreeSet::new();
        for p in &record.catalog.portals {
            crate::text(&p.anchor_id, 256).map_err(|e| AlignmentError(e.to_string()))?;
            crate::text(&p.payload, 4096).map_err(|e| AlignmentError(e.to_string()))?;
            if !ids.insert(&p.anchor_id) {
                return Err(AlignmentError("duplicate portal identity".into()));
            }
        }
        if let Some(old) = self.records.get(&key) {
            if old.reference.product != record.reference.product
                || old.catalog.map.map_id != record.catalog.map.map_id
            {
                return Err(AlignmentError(
                    "map identity changed; remove the old publication explicitly".into(),
                ));
            }
            if record.reference.sequence < old.reference.sequence {
                return Err(AlignmentError("stale map information".into()));
            }
            if record.reference.sequence == old.reference.sequence {
                if old.catalog != record.catalog
                    || (old.snapshot.is_some()
                        && record.snapshot.is_some()
                        && old.snapshot != record.snapshot)
                {
                    return Err(AlignmentError(
                        "conflicting data for the same snapshot".into(),
                    ));
                }
                if record.snapshot.is_none() {
                    record.snapshot = old.snapshot.clone();
                }
            }
        } else if self.records.len() >= MAX_ALIGNMENT_MAPS {
            return Err(AlignmentError("map limit reached".into()));
        }
        self.records.insert(key, record);
        Ok(if self.options.automatic {
            self.check_all()
        } else {
            vec![]
        })
    }
    pub fn remove(&mut self, key: &str) -> Vec<AlignmentEvent> {
        self.records.remove(key);
        if self.options.automatic {
            self.check_all()
        } else {
            vec![]
        }
    }
    /// Explicit check, independent of automatic mode. Does not merge or mutate maps.
    pub fn check(&self, source: &str, target: &str) -> AlignmentResult {
        self.resolve(&self.graph(), source, target)
    }
    /// Emits changed results only, including invalidations after update/removal.
    /// One event per unordered pair; the event declares the transform's direction.
    pub fn check_all(&mut self) -> Vec<AlignmentEvent> {
        let graph = self.graph();
        let keys: Vec<_> = self.records.keys().cloned().collect();
        let mut pairs: BTreeSet<_> = self.previous.keys().cloned().collect();
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                pairs.insert((a.clone(), b.clone()));
            }
        }
        let mut events = vec![];
        for (a, b) in pairs {
            let result = self.resolve(&graph, &a, &b);
            if self.previous.get(&(a.clone(), b.clone())) != Some(&result) {
                events.push(AlignmentEvent {
                    source_map: a.clone(),
                    target_map: b.clone(),
                    result: result.clone(),
                });
            }
            if self.records.contains_key(&a) && self.records.contains_key(&b) {
                self.previous.insert((a, b), result);
            } else {
                self.previous.remove(&(a, b));
            }
        }
        events
    }
    fn graph(&self) -> Graph {
        let mut g = Graph {
            potential: BTreeMap::new(),
            edges: BTreeMap::new(),
            conflicts: BTreeSet::new(),
        };
        let records: Vec<_> = self.records.iter().collect();
        for (i, (ak, a)) in records.iter().enumerate() {
            for (bk, b) in &records[i + 1..] {
                let b_ids: BTreeSet<_> = b
                    .catalog
                    .portals
                    .iter()
                    .map(|p| p.anchor_id.as_str())
                    .collect();
                // Only canonical Portal UUIDs identify shared physical markers across publishers.
                let common: Vec<_> = a
                    .catalog
                    .portals
                    .iter()
                    .filter(|p| {
                        uuid::Uuid::parse_str(&p.anchor_id).is_ok()
                            && b_ids.contains(p.anchor_id.as_str())
                    })
                    .map(|p| p.anchor_id.clone())
                    .collect();
                if common.is_empty() {
                    continue;
                }
                g.potential
                    .entry((*ak).clone())
                    .or_default()
                    .push((*bk).clone());
                g.potential
                    .entry((*bk).clone())
                    .or_default()
                    .push((*ak).clone());
                let (Some(sa), Some(sb)) = (&a.snapshot, &b.snapshot) else {
                    continue;
                };
                let af = &a.catalog.map.frame;
                let bf = &b.catalog.map.frame;
                // Cross-convention conversion must be explicitly supplied by a future adapter.
                if af.up_axis != bf.up_axis
                    || af.handedness != bf.handedness
                    || af.meters_per_unit != bf.meters_per_unit
                {
                    continue;
                }
                let mut transforms = vec![];
                let mut bad = false;
                for id in &common {
                    let aa = &sa.scenegraph.anchors[id];
                    let ba = &sb.scenegraph.anchors[id];
                    if (aa.side_length_m - ba.side_length_m).abs() > 1e-9 {
                        bad = true;
                        break;
                    }
                    let t = compose_spatial_transforms(
                        &inverse_spatial_transform(&numeric(&aa.pose_in_map))
                            .expect("validated pose"),
                        &numeric(&ba.pose_in_map),
                    )
                    .expect("validated pose");
                    let t = labelled(t, &af.id, &bf.id);
                    if !finite(&t) {
                        bad = true;
                        break;
                    }
                    transforms.push(t);
                }
                if !bad {
                    bad = transforms
                        .iter()
                        .skip(1)
                        .any(|t| !self.close(&transforms[0], t, bf.meters_per_unit));
                }
                if bad {
                    g.conflicts.insert((*ak).clone());
                    g.conflicts.insert((*bk).clone());
                    continue;
                }
                let forward = transforms[0].clone();
                let reverse = labelled(
                    inverse_spatial_transform(&numeric(&forward)).expect("validated pose"),
                    &bf.id,
                    &af.id,
                );
                if !finite(&reverse) {
                    g.conflicts.insert((*ak).clone());
                    g.conflicts.insert((*bk).clone());
                    continue;
                }
                g.edges.entry((*ak).clone()).or_default().push(Edge {
                    transform: forward,
                    step: AlignmentStep {
                        from_map: (*ak).clone(),
                        to_map: (*bk).clone(),
                        from_snapshot: a.reference.clone(),
                        to_snapshot: b.reference.clone(),
                        portal_ids: common.clone(),
                    },
                });
                g.edges.entry((*bk).clone()).or_default().push(Edge {
                    transform: reverse,
                    step: AlignmentStep {
                        from_map: (*bk).clone(),
                        to_map: (*ak).clone(),
                        from_snapshot: b.reference.clone(),
                        to_snapshot: a.reference.clone(),
                        portal_ids: common,
                    },
                });
            }
        }
        g
    }
    fn close(&self, a: &RigidTransform, b: &RigidTransform, scale: f64) -> bool {
        let distance = a
            .translation
            .iter()
            .zip(b.translation)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f64>()
            .sqrt()
            * scale;
        let dot = a
            .rotation_wxyz
            .iter()
            .zip(b.rotation_wxyz)
            .map(|(x, y)| x * y)
            .sum::<f64>()
            .abs()
            .clamp(0., 1.);
        distance <= self.options.translation_tolerance_m
            && 2. * dot.acos() <= self.options.rotation_tolerance_rad
    }
    fn resolve(&self, g: &Graph, source: &str, target: &str) -> AlignmentResult {
        let (Some(src), Some(_)) = (self.records.get(source), self.records.get(target)) else {
            return AlignmentResult::NoConnection;
        };
        let mut potential = BTreeMap::from([(source.to_string(), vec![source.to_string()])]);
        let mut queue = VecDeque::from([source.to_string()]);
        while let Some(node) = queue.pop_front() {
            for next in g.potential.get(&node).into_iter().flatten() {
                if !potential.contains_key(next) {
                    let mut path = potential[&node].clone();
                    path.push(next.clone());
                    potential.insert(next.clone(), path);
                    queue.push_back(next.clone());
                }
            }
        }
        if !potential.contains_key(target) {
            return AlignmentResult::NoConnection;
        }
        // Fail closed across a connected candidate component with contradictory shared anchors.
        if potential.keys().any(|key| g.conflicts.contains(key)) {
            return AlignmentResult::Conflict {
                reason: "shared Portal placements or physical sizes disagree".into(),
            };
        }
        let mut known = BTreeMap::from([(
            source.to_string(),
            (
                RigidTransform::identity(&src.catalog.map.frame.id, &src.catalog.map.frame.id),
                Vec::<AlignmentStep>::new(),
            ),
        )]);
        let mut queue = VecDeque::from([source.to_string()]);
        while let Some(node) = queue.pop_front() {
            for edge in g.edges.get(&node).into_iter().flatten() {
                let (pose, path) = &known[&node];
                if pose.to_frame_id != edge.transform.from_frame_id {
                    return AlignmentResult::Conflict {
                        reason: "frame endpoint mismatch".into(),
                    };
                }
                let composed = labelled(
                    compose_spatial_transforms(&numeric(pose), &numeric(&edge.transform))
                        .expect("validated pose"),
                    &pose.from_frame_id,
                    &edge.transform.to_frame_id,
                );
                if !finite(&composed) {
                    return AlignmentResult::Conflict {
                        reason: "alignment overflow".into(),
                    };
                }
                let next = &edge.step.to_map;
                if let Some((existing, _)) = known.get(next) {
                    if !self.close(
                        existing,
                        &composed,
                        self.records[next].catalog.map.frame.meters_per_unit,
                    ) {
                        return AlignmentResult::Conflict {
                            reason: "different transform paths disagree".into(),
                        };
                    }
                } else {
                    let mut steps = path.clone();
                    steps.push(edge.step.clone());
                    known.insert(next.clone(), (composed, steps));
                    queue.push_back(next.clone());
                }
            }
        }
        if let Some((transform, path)) = known.remove(target) {
            AlignmentResult::Available { transform, path }
        } else {
            let maps = potential.remove(target).unwrap();
            let path = maps
                .windows(2)
                .map(|pair| {
                    let a = &self.records[&pair[0]];
                    let b = &self.records[&pair[1]];
                    let ids: BTreeSet<_> = b.catalog.portals.iter().map(|p| &p.anchor_id).collect();
                    AlignmentStep {
                        from_map: pair[0].clone(),
                        to_map: pair[1].clone(),
                        from_snapshot: a.reference.clone(),
                        to_snapshot: b.reference.clone(),
                        portal_ids: a
                            .catalog
                            .portals
                            .iter()
                            .filter(|p| {
                                uuid::Uuid::parse_str(&p.anchor_id).is_ok()
                                    && ids.contains(&p.anchor_id)
                            })
                            .map(|p| p.anchor_id.clone())
                            .collect(),
                    }
                })
                .collect();
            AlignmentResult::PotentialConnection {maps,path,reason:"fetch matching snapshots; differing conventions or units require explicit conversion".into()}
        }
    }
}
fn numeric(t: &RigidTransform) -> SpatialTransform {
    let [x, y, z] = t.translation;
    let [w, qx, qy, qz] = t.rotation_wxyz;
    SpatialTransform {
        translation: Some(Vec3 { x, y, z }),
        orientation: Some(Quat {
            x: qx,
            y: qy,
            z: qz,
            w,
        }),
    }
}
fn labelled(t: SpatialTransform, from: &str, to: &str) -> RigidTransform {
    let p = t.translation.expect("geometry returns translation");
    let q = t.orientation.expect("geometry returns rotation");
    RigidTransform {
        from_frame_id: from.into(),
        to_frame_id: to.into(),
        translation: [p.x, p.y, p.z],
        rotation_wxyz: [q.w, q.x, q.y, q.z],
    }
}
fn finite(t: &RigidTransform) -> bool {
    t.translation
        .iter()
        .chain(t.rotation_wxyz.iter())
        .all(|v| v.is_finite())
}
