//! Synthetic, fixed-heading Portal maps. Transport and rendering belong to the browser host.
#![forbid(unsafe_code)]
use auki_components::{
    CatalogSnapshot, ComponentRuntime, InMemoryTransport, InvocationContext, ProductReference,
};
use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_geometry::{
    compose_spatial_transforms, convert_point_convention, convert_transform_target_convention,
    inverse_spatial_transform,
};
use auki_registry::{AxisConvention, AxisDirection, FrameRegistryEntry, Handedness, LengthUnit};
use auki_scenegraph::{
    MapDefinition, MapFrame, MapSnapshot, QrAnchor, RigidTransform,
    alignment::{AlignmentOptions, AlignmentResult, MapAlignmentChecker},
    component::{MapComponent, MapComponentConfig, RemoveQr, SnapshotReference, UpsertQr},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
use uuid::Uuid;

pub const DEMO_VERSION: &str = "auki.collaborative-grid/v1";
pub const MAX_PORTALS: usize = 128;
pub const MAX_COORDINATE: f64 = 10_000.;
pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishedMap {
    pub reference: SnapshotReference,
    pub snapshot: MapSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortalView {
    pub id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub contributors: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PortalConflict {
    pub name: String,
    pub disagrees_with: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub state: String,
    pub reason: Option<String>,
    pub local_peer: String,
    pub remote_peer: Option<String>,
    pub local_frame: String,
    pub remote_frame: Option<String>,
    pub conflicts: Vec<PortalConflict>,
    pub display_frame: String,
    pub local_to_display: RigidTransform,
    pub local_reference: SnapshotReference,
    pub remote_reference: Option<SnapshotReference>,
    pub shared_names: Vec<String>,
    pub portals: Vec<PortalView>,
    pub local_portals: Vec<PortalView>,
    pub remote_portals: Vec<PortalView>,
}

/// Name equality is an explicit synthetic identity rule, never a real DDS Portal lookup.
/// Length-prefixing keeps session/name boundaries unambiguous. UUIDv8 uses 122 SHA-256 bits.
pub fn portal_id(session: &str, name: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(DEMO_VERSION);
    for value in [session, name] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    let mut bytes: [u8; 16] = hash.finalize()[..16].try_into().unwrap();
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || name.trim() != name
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " _-".contains(c))
    {
        return Err(
            "Use 1–64 letters, digits, spaces, underscores or hyphens; no edge spaces".into(),
        );
    }
    Ok(())
}

/// Screen-plane axis orientation. All presets retain right-handed XY, +Z out, meters.
/// The choice is declared in the exact map definition; it does not align independent origins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Convention {
    XRight,
    XUp,
    XLeft,
    XDown,
}
impl Convention {
    pub const ALL: [Self; 4] = [Self::XRight, Self::XUp, Self::XLeft, Self::XDown];
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "x_right" => Ok(Self::XRight),
            "x_up" => Ok(Self::XUp),
            "x_left" => Ok(Self::XLeft),
            "x_down" => Ok(Self::XDown),
            _ => Err("Unknown coordinate convention".into()),
        }
    }
    /// Explicit convention declaration; semantic directions describe the screen plane,
    /// not a physical alignment or a shared origin between peer maps.
    pub fn frame(self, peer: &str, frame_id: &str) -> FrameRegistryEntry {
        use AxisDirection::*;
        let (x, y) = match self {
            Self::XRight => (Right, Up),
            Self::XUp => (Up, Left),
            Self::XLeft => (Left, Down),
            Self::XDown => (Down, Right),
        };
        FrameRegistryEntry {
            peer_id: peer.into(),
            frame_id: frame_id.into(),
            handedness: Handedness::Right,
            axes: AxisConvention { x, y, z: Backward },
            units: LengthUnit::Meters,
        }
    }
    pub fn convert_point(self, to: Self, x: f64, y: f64) -> Result<[f64; 2]> {
        if !x.is_finite() || !y.is_finite() {
            return Err("Non-finite convention coordinates".into());
        }
        let point = convert_point_convention(
            Vec3 { x, y, z: 0. },
            &self.frame("", "source-convention"),
            &to.frame("", "target-convention"),
        )
        .map_err(|e| e.to_string())?;
        Ok([point.x, point.y])
    }
    pub fn to_plane(self, x: f64, y: f64) -> [f64; 2] {
        self.convert_point(Self::XRight, x, y)
            .expect("validated planar coordinates")
    }
    pub fn portal_rotation(self) -> [f64; 4] {
        let pose = convert_transform_target_convention(
            &SpatialTransform {
                translation: None,
                orientation: None,
            },
            &Self::XRight.frame("", "printed-portal-convention"),
            &self.frame("", "map-convention"),
        )
        .expect("right-handed meter presets");
        let q = pose.orientation.expect("geometry returns rotation");
        // Preserve the existing preset wire values despite matrix-to-quaternion
        // roundoff. The rotation itself is calculated by SDK geometry above.
        [q.w, q.x, q.y, q.z].map(|v| {
            let half = std::f64::consts::FRAC_1_SQRT_2;
            if v.abs() < 1e-12 {
                0.
            } else if (v.abs() - half).abs() < 1e-12 {
                v.signum() * half
            } else {
                v
            }
        })
    }
}

pub fn definition(peer: &str, domain: &str, session: &str) -> MapDefinition {
    definition_with_convention(peer, domain, session, Convention::XRight)
}
pub fn definition_with_convention(
    peer: &str,
    domain: &str,
    session: &str,
    convention: Convention,
) -> MapDefinition {
    let mut map = MapDefinition {
        map_id: format!("grid-{}", portal_id(session, peer)),
        name: Some(format!("Grid {session}")),
        domain_reference: Some(domain.into()),
        frame: MapFrame::z_up_meters(
            format!("grid-frame-{}", portal_id(session, peer)),
            "Independent simulated origin; XY plane, fixed +X portal heading, Z out of grid",
        ),
    };
    if convention != Convention::XRight {
        map.frame.id = format!("{}-{:?}", map.frame.id, convention);
        map.frame.origin_description = format!(
            "Independent simulated origin; XY plane; screen convention {:?}; +Z out; Portal printed-right is screen-right",
            convention
        );
    }
    map
}

pub fn map_convention(
    map: &MapDefinition,
    peer: &str,
    domain: &str,
    session: &str,
) -> Result<Convention> {
    Convention::ALL
        .into_iter()
        .find(|c| *map == definition_with_convention(peer, domain, session, *c))
        .ok_or_else(|| {
            "Map does not declare a supported demo session, Domain and coordinate convention".into()
        })
}

/// Select only this demo's exact session/Domain map from an authenticated peer Catalog.
/// Catalog metadata is a relevance hint; the subscription still validates its producer and payload.
pub fn discover_map(
    catalog: &CatalogSnapshot,
    peer: &str,
    domain: &str,
    session: &str,
) -> Result<Option<ProductReference>> {
    use auki_scenegraph::catalog::{MAP_CATALOG_SCHEMA, MapCatalogData};
    let mut found = None;
    for entry in &catalog.products {
        if entry.manifest.peer_id != peer || entry.manifest.hash() != entry.manifest_hash {
            return Err("Catalog Product does not match authenticated peer or manifest".into());
        }
        let Some(metadata) = &entry.metadata else {
            continue;
        };
        if metadata.schema != MAP_CATALOG_SCHEMA {
            continue;
        }
        let Ok(data) = serde_json::from_value::<MapCatalogData>(metadata.value.clone()) else {
            continue;
        };
        if map_convention(&data.map, peer, domain, session).is_err() {
            continue;
        }
        if found.is_some() {
            return Err("Peer exports multiple maps for this demo session".into());
        }
        found = Some(entry.manifest.reference());
    }
    Ok(found)
}

pub struct DemoMap {
    pub runtime: ComponentRuntime,
    pub map: MapComponent,
    peer: String,
    domain: String,
    session: String,
    convention: Convention,
    selected: BTreeMap<String, ProductReference>,
    remotes: BTreeMap<String, PublishedMap>,
    checker: MapAlignmentChecker,
    closed: bool,
}

impl DemoMap {
    pub fn new(peer: String, domain: String, session: String) -> Result<Self> {
        Self::with_convention(peer, domain, session, Convention::XRight)
    }
    pub fn with_convention(
        peer: String,
        domain: String,
        session: String,
        convention: Convention,
    ) -> Result<Self> {
        validate_name(&session)?;
        if peer.is_empty() || peer.len() > 128 || domain.is_empty() || domain.len() > 128 {
            return Err("Invalid peer or Domain".into());
        }
        let runtime = ComponentRuntime::new(peer.as_str());
        let owner = peer.clone();
        // A monotonic logical publication clock on an explicitly named per-run clock.
        let publication = Uuid::new_v4().to_string();
        let clock = AtomicU64::new(1);
        let map = MapComponent::new(
            &runtime,
            MapComponentConfig {
                component_id: "grid-map".into(),
                publication_id: publication.clone(),
                clock_id: format!("grid-logical-{publication}"),
                map: definition_with_convention(&peer, &domain, &session, convention),
            },
            move || clock.fetch_add(1, Ordering::Relaxed),
            |_| false,
            move |context| {
                context.caller_peer_id == owner && context.caller_component_id == "grid-editor"
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            runtime,
            map,
            peer,
            domain,
            session,
            convention,
            selected: BTreeMap::new(),
            remotes: BTreeMap::new(),
            checker: MapAlignmentChecker::new(AlignmentOptions::default())
                .map_err(|e| e.to_string())?,
            closed: false,
        })
    }

    pub fn publication(&self) -> PublishedMap {
        let reference = self.map.snapshot_reference();
        let records = self
            .map
            .product()
            .buffer()
            .snapshot(reference.sequence, reference.sequence);
        PublishedMap {
            reference,
            snapshot: (*records[0].payload.payload).clone(),
        }
    }

    pub fn select_partner(&mut self, product: ProductReference) -> Result<()> {
        self.ensure_open()?;
        if product.peer_id == self.peer || product.peer_id.is_empty() {
            return Err("Select a different peer".into());
        }
        if !self.selected.contains_key(&product.peer_id) && self.selected.len() >= 15 {
            return Err("Session supports at most 16 peers including you".into());
        }
        if self.selected.get(&product.peer_id) != Some(&product) {
            self.checker.remove(&product.peer_id);
            self.remotes.remove(&product.peer_id);
            self.selected.insert(product.peer_id.clone(), product);
        }
        Ok(())
    }

    pub fn receive(&mut self, incoming: PublishedMap) -> Result<()> {
        self.ensure_open()?;
        if self.selected.get(&incoming.reference.product.peer_id)
            != Some(&incoming.reference.product)
        {
            return Err("Snapshot is not from the selected exact Product".into());
        }
        incoming.snapshot.validate().map_err(|e| e.to_string())?;
        let scene = &incoming.snapshot.scenegraph;
        let convention = map_convention(
            &scene.map,
            &incoming.reference.product.peer_id,
            &self.domain,
            &self.session,
        )?;
        if scene.anchors.len() > MAX_PORTALS {
            return Err("Too many portals".into());
        }
        for anchor in scene.anchors.values() {
            validate_name(&anchor.payload)?;
            let id = portal_id(&self.session, &anchor.payload);
            let pose = &anchor.pose_in_map;
            if anchor.anchor_id != id
                || pose.from_frame_id != format!("simulated-portal-{id}")
                || anchor.side_length_m != 0.25
                || pose.rotation_wxyz != convention.portal_rotation()
                || pose.translation[2] != 0.
            {
                return Err(
                    "Snapshot violates synthetic Portal identity or fixed-heading geometry".into(),
                );
            }
            validate_position(pose.translation[0], pose.translation[1])?;
        }
        self.checker
            .receive_snapshot(
                &incoming.reference.product.peer_id,
                incoming.reference.clone(),
                incoming.snapshot.clone(),
            )
            .map_err(|e| e.to_string())?;
        self.remotes
            .insert(incoming.reference.product.peer_id.clone(), incoming);
        Ok(())
    }

    /// Coordinates carry an explicit frame, so a UI cannot submit stale pre-alignment coordinates.
    pub fn place(&mut self, name: &str, x: f64, y: f64, frame: &str) -> Result<()> {
        self.ensure_open()?;
        validate_name(name)?;
        let local_frame = self.publication().snapshot.scenegraph.map.frame.id;
        let [x, y] = if frame == local_frame {
            [x, y]
        } else {
            let peer = self
                .remotes
                .iter()
                .find(|(_, map)| map.snapshot.scenegraph.map.frame.id == frame)
                .map(|(peer, _)| peer.clone())
                .ok_or("Display frame changed; place the Portal again")?;
            self.checker
                .receive_snapshot(
                    "local",
                    self.publication().reference,
                    self.publication().snapshot,
                )
                .map_err(|e| e.to_string())?;
            let AlignmentResult::Available { transform, .. } = self.checker.check("local", &peer)
            else {
                return Err("Display frame is not aligned to your map".into());
            };
            transform_point(&transform, frame, &local_frame, x, y)?
        };
        validate_position(x, y)?;
        let current = self.publication();
        let id = portal_id(&self.session, name);
        if current.snapshot.scenegraph.anchors.len() >= MAX_PORTALS
            && !current.snapshot.scenegraph.anchors.contains_key(&id)
        {
            return Err("This demo supports up to 128 portals per peer".into());
        }
        InMemoryTransport
            .invoke(
                self.map.upsert_qr(),
                InvocationContext {
                    invocation_id: Uuid::new_v4().to_string(),
                    caller_peer_id: self.peer.clone(),
                    caller_component_id: "grid-editor".into(),
                },
                UpsertQr {
                    expected_snapshot: current.reference,
                    anchor: QrAnchor {
                        anchor_id: id.clone(),
                        payload: name.into(),
                        side_length_m: 0.25,
                        pose_in_map: RigidTransform {
                            from_frame_id: format!("simulated-portal-{id}"),
                            to_frame_id: current.snapshot.scenegraph.map.frame.id,
                            translation: [x, y, 0.],
                            rotation_wxyz: self.convention.portal_rotation(),
                        },
                    },
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<()> {
        self.ensure_open()?;
        validate_name(name)?;
        InMemoryTransport
            .invoke(
                self.map.remove_qr(),
                InvocationContext {
                    invocation_id: Uuid::new_v4().to_string(),
                    caller_peer_id: self.peer.clone(),
                    caller_component_id: "grid-editor".into(),
                },
                RemoveQr {
                    expected_snapshot: self.map.snapshot_reference(),
                    anchor_id: portal_id(&self.session, name),
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn clear_evidence(&mut self, peer: &str) {
        self.checker.remove(peer);
        self.remotes.remove(peer);
    }

    pub fn forget_peer(&mut self, peer: &str) {
        self.checker.remove(peer);
        self.remotes.remove(peer);
        self.selected.remove(peer);
    }

    pub fn view(&mut self) -> Result<View> {
        let peer = self.remotes.keys().next().cloned();
        self.view_for(peer.as_deref())
    }

    fn view_for(&mut self, peer: Option<&str>) -> Result<View> {
        let local = self.publication();
        self.checker
            .receive_snapshot("local", local.reference.clone(), local.snapshot.clone())
            .map_err(|e| e.to_string())?;
        let frame = &local.snapshot.scenegraph.map.frame.id;
        let mut local_to_display = RigidTransform::identity(frame, frame);
        let mut remote_to_display = None;
        let mut state = "waiting".to_owned();
        let mut reason = None;
        let mut shared_names = vec![];
        let mut offsets = vec![];
        let remote = peer.and_then(|p| self.remotes.get(p));
        if let Some(remote) = remote {
            for (id, anchor) in &local.snapshot.scenegraph.anchors {
                if let Some(other) = remote.snapshot.scenegraph.anchors.get(id) {
                    shared_names.push(anchor.payload.clone());
                    offsets.push((
                        anchor.payload.clone(),
                        shared_offset(self.convention, self.convention_of(remote)?, anchor, other),
                    ));
                }
            }
            state = "separate".into();
            match self
                .checker
                .check(&remote.reference.product.peer_id, "local")
            {
                AlignmentResult::Available { transform, .. } => {
                    state = "aligned".into();
                    if self.peer < remote.reference.product.peer_id {
                        remote_to_display = Some(transform);
                    } else {
                        let AlignmentResult::Available { transform, .. } = self
                            .checker
                            .check("local", &remote.reference.product.peer_id)
                        else {
                            return Err("Inverse alignment unavailable".into());
                        };
                        local_to_display = transform;
                        let rf = &remote.snapshot.scenegraph.map.frame.id;
                        remote_to_display = Some(RigidTransform::identity(rf, rf));
                    }
                }
                AlignmentResult::Conflict { reason: conflict } => {
                    state = "conflict".into();
                    reason = Some(conflict);
                }
                AlignmentResult::PotentialConnection {
                    reason: pending, ..
                } => {
                    state = "pending".into();
                    reason = Some(pending);
                }
                AlignmentResult::NoConnection => {}
            }
        }
        let local_portals = portals(&local, None);
        let remote_portals = remote.map(|r| portals(r, None)).unwrap_or_default();
        let mut combined: BTreeMap<String, PortalView> = portals(&local, Some(&local_to_display))
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        if let (Some(remote), Some(transform)) = (remote, remote_to_display) {
            for portal in portals(remote, Some(&transform)) {
                match combined.entry(portal.id.clone()) {
                    std::collections::btree_map::Entry::Vacant(e) => {
                        e.insert(portal);
                    }
                    std::collections::btree_map::Entry::Occupied(mut e) => {
                        // Use canonical-frame publisher's exact coordinates for identical rendering.
                        let p = e.get_mut();
                        if remote.reference.product.peer_id < self.peer {
                            p.x = portal.x;
                            p.y = portal.y;
                        }
                        p.contributors.extend(portal.contributors);
                        p.contributors.sort();
                    }
                }
            }
        }
        // Validated demo coordinates are integral with fixed heading. Unequal offsets
        // differ by at least a meter, beyond the checker's 2 cm tolerance. Report
        // pairwise contradictions without choosing an arbitrary "correct" anchor.
        let conflicts = offsets
            .iter()
            .filter_map(|(name, offset)| {
                let disagrees_with: Vec<_> = offsets
                    .iter()
                    .filter(|(_, other)| other != offset)
                    .map(|(other, _)| other.clone())
                    .collect();
                (!disagrees_with.is_empty()).then(|| PortalConflict {
                    name: name.clone(),
                    disagrees_with,
                })
            })
            .collect();
        shared_names.sort();
        Ok(View {
            state,
            reason,
            local_peer: self.peer.clone(),
            remote_peer: remote.map(|r| r.reference.product.peer_id.clone()),
            local_frame: frame.clone(),
            remote_frame: remote.map(|r| r.snapshot.scenegraph.map.frame.id.clone()),
            conflicts,
            display_frame: local_to_display.to_frame_id.clone(),
            local_to_display,
            local_reference: local.reference,
            remote_reference: remote.map(|r| r.reference.clone()),
            shared_names,
            portals: combined.into_values().collect(),
            local_portals,
            remote_portals,
        })
    }
    pub fn session_view(&mut self) -> Result<SessionView> {
        let local = self.publication();
        self.checker
            .receive_snapshot("local", local.reference.clone(), local.snapshot.clone())
            .map_err(|e| e.to_string())?;
        let mut target = "local".to_owned();
        let mut canonical_peer = self.peer.clone();
        for peer in self.remotes.keys() {
            if peer < &canonical_peer
                && matches!(
                    self.checker.check("local", peer),
                    AlignmentResult::Available { .. }
                )
            {
                target = peer.clone();
                canonical_peer = peer.clone();
            }
        }
        let frame = if target == "local" {
            local.snapshot.scenegraph.map.frame.id.clone()
        } else {
            self.remotes[&target]
                .snapshot
                .scenegraph
                .map
                .frame
                .id
                .clone()
        };
        let mut layers = vec![];
        let mut combined: BTreeMap<String, PortalView> = BTreeMap::new();
        let mut maps = vec![("local".to_owned(), local.clone())];
        maps.extend(
            self.remotes
                .iter()
                .map(|(peer, map)| (peer.clone(), map.clone())),
        );
        // Canonical publisher wins shared-coordinate deduplication deterministically.
        maps.sort_by(|a, b| {
            a.1.reference
                .product
                .peer_id
                .cmp(&b.1.reference.product.peer_id)
        });
        for (key, map) in maps {
            let (state, transform) = if key == target {
                ("aligned", Some(RigidTransform::identity(&frame, &frame)))
            } else {
                match self.checker.check(&key, &target) {
                    AlignmentResult::Available { transform, .. } => ("aligned", Some(transform)),
                    AlignmentResult::Conflict { .. } => ("conflict", None),
                    _ => ("separate", None),
                }
            };
            let transformed = transform.as_ref().map(|t| portals(&map, Some(t)));
            if let Some(visible) = &transformed {
                for portal in visible {
                    combined
                        .entry(portal.id.clone())
                        .and_modify(|p| {
                            p.contributors.extend(portal.contributors.clone());
                            p.contributors.sort();
                        })
                        .or_insert_with(|| portal.clone());
                }
            }
            layers.push(MapLayer {
                convention: self.convention_of(&map)?,
                peer: map.reference.product.peer_id.clone(),
                frame: map.snapshot.scenegraph.map.frame.id.clone(),
                sequence: map.reference.sequence,
                state: state.into(),
                portals: portals(&map, None),
                aligned_portals: transformed,
                to_display: transform,
            });
        }
        let mut conflicts = vec![];
        // Pairwise evidence includes conflicts between remote maps, not just with us.
        let mut originals = vec![local];
        originals.extend(self.remotes.values().cloned());
        for (i, a) in originals.iter().enumerate() {
            for b in originals.iter().skip(i + 1) {
                let ac = self.convention_of(a)?;
                let bc = self.convention_of(b)?;
                let offsets: Vec<_> = a
                    .snapshot
                    .scenegraph
                    .anchors
                    .iter()
                    .filter_map(|(id, anchor)| {
                        b.snapshot.scenegraph.anchors.get(id).map(|other| {
                            (anchor.payload.clone(), shared_offset(ac, bc, anchor, other))
                        })
                    })
                    .collect();
                for (name, offset) in &offsets {
                    let disagrees_with: Vec<_> = offsets
                        .iter()
                        .filter(|(_, other)| other != offset)
                        .map(|(name, _)| name.clone())
                        .collect();
                    if !disagrees_with.is_empty() {
                        conflicts.push(SessionConflict {
                            name: name.clone(),
                            disagrees_with,
                            peers: [
                                a.reference.product.peer_id.clone(),
                                b.reference.product.peer_id.clone(),
                            ],
                        });
                    }
                }
            }
        }
        Ok(SessionView {
            local_peer: self.peer.clone(),
            display_frame: frame,
            layers,
            portals: combined.into_values().collect(),
            conflicts,
        })
    }

    fn convention_of(&self, map: &PublishedMap) -> Result<Convention> {
        map_convention(
            &map.snapshot.scenegraph.map,
            &map.reference.product.peer_id,
            &self.domain,
            &self.session,
        )
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.map.close();
    }
    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            Err("Map is closed".into())
        } else {
            Ok(())
        }
    }
}
fn validate_position(x: f64, y: f64) -> Result<()> {
    if [x, y]
        .iter()
        .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE || v.fract() != 0.)
    {
        return Err("Portal position must be an integer grid cell within ±10000 meters".into());
    }
    Ok(())
}
/// Apply an established alignment in either direction. Both endpoints are mandatory;
/// convention conversion alone must never be supplied as evidence of alignment.
pub fn transform_point(
    t: &RigidTransform,
    from: &str,
    to: &str,
    x: f64,
    y: f64,
) -> Result<[f64; 2]> {
    if from.is_empty()
        || to.is_empty()
        || !x.is_finite()
        || !y.is_finite()
        || t.translation
            .iter()
            .chain(t.rotation_wxyz.iter())
            .any(|v| !v.is_finite())
    {
        return Err("Invalid frame transform or point".into());
    }
    let [tx, ty, tz] = t.translation;
    let [w, qx, qy, qz] = t.rotation_wxyz;
    let mut pose = SpatialTransform {
        translation: Some(Vec3 {
            x: tx,
            y: ty,
            z: tz,
        }),
        orientation: Some(Quat {
            w,
            x: qx,
            y: qy,
            z: qz,
        }),
    };
    if from == t.from_frame_id && to == t.to_frame_id {
        // The alignment already has the requested direction.
    } else if from == t.to_frame_id && to == t.from_frame_id {
        pose = inverse_spatial_transform(&pose).map_err(|e| e.to_string())?;
    } else {
        return Err("Point frames do not match alignment endpoints".into());
    }
    let result = compose_spatial_transforms(
        &SpatialTransform {
            translation: Some(Vec3 { x, y, z: 0. }),
            orientation: None,
        },
        &pose,
    )
    .map_err(|e| e.to_string())?;
    let p = result.translation.expect("geometry returns translation");
    if p.z.abs() > 1e-8 {
        return Err("Transformed point is outside the demo map plane".into());
    }
    Ok([snap(p.x), snap(p.y)])
}
fn snap(v: f64) -> f64 {
    if (v - v.round()).abs() < 1e-8 {
        v.round()
    } else {
        v
    }
}
fn shared_offset(a: Convention, b: Convention, aa: &QrAnchor, bb: &QrAnchor) -> [f64; 2] {
    let [ax, ay] = a.to_plane(aa.pose_in_map.translation[0], aa.pose_in_map.translation[1]);
    let [bx, by] = b.to_plane(bb.pose_in_map.translation[0], bb.pose_in_map.translation[1]);
    [ax - bx, ay - by]
}
fn portals(map: &PublishedMap, transform: Option<&RigidTransform>) -> Vec<PortalView> {
    map.snapshot
        .scenegraph
        .anchors
        .values()
        .map(|anchor| {
            let [x, y, _] = anchor.pose_in_map.translation;
            let [x, y] = transform
                .map(|t| {
                    transform_point(
                        t,
                        &map.snapshot.scenegraph.map.frame.id,
                        &t.to_frame_id,
                        x,
                        y,
                    )
                    .expect("validated planar alignment")
                })
                .unwrap_or([x, y]);
            PortalView {
                id: anchor.anchor_id.clone(),
                name: anchor.payload.clone(),
                x,
                y,
                contributors: vec![map.reference.product.peer_id.clone()],
            }
        })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct MapLayer {
    pub convention: Convention,
    pub peer: String,
    pub frame: String,
    pub sequence: u64,
    pub state: String,
    pub portals: Vec<PortalView>,
    pub aligned_portals: Option<Vec<PortalView>>,
    pub to_display: Option<RigidTransform>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SessionConflict {
    pub name: String,
    pub disagrees_with: Vec<String>,
    pub peers: [String; 2],
}
#[derive(Clone, Debug, Serialize)]
pub struct SessionView {
    pub local_peer: String,
    pub display_frame: String,
    pub layers: Vec<MapLayer>,
    pub portals: Vec<PortalView>,
    pub conflicts: Vec<SessionConflict>,
}
