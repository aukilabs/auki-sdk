//! Synthetic, fixed-heading Portal maps. Transport and rendering belong to the browser host.
#![forbid(unsafe_code)]
use auki_components::{
    CatalogSnapshot, ComponentRuntime, InMemoryTransport, InvocationContext, ProductReference,
};
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

pub fn definition(peer: &str, domain: &str, session: &str) -> MapDefinition {
    MapDefinition {
        map_id: format!("grid-{}", portal_id(session, peer)),
        name: Some(format!("Grid {session}")),
        domain_reference: Some(domain.into()),
        frame: MapFrame::z_up_meters(
            format!("grid-frame-{}", portal_id(session, peer)),
            "Independent simulated origin; XY plane, fixed +X portal heading, Z out of grid",
        ),
    }
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
    let expected = definition(peer, domain, session);
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
        if data.map != expected {
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
    selected: Option<ProductReference>,
    remote: Option<PublishedMap>,
    checker: MapAlignmentChecker,
    closed: bool,
}

impl DemoMap {
    pub fn new(peer: String, domain: String, session: String) -> Result<Self> {
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
                map: definition(&peer, &domain, &session),
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
            selected: None,
            remote: None,
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
        // Explicit selection starts a new relationship; retries preserve the existing evidence.
        if self.selected.as_ref() != Some(&product) {
            self.checker.remove("remote");
            self.remote = None;
            self.selected = Some(product);
        }
        Ok(())
    }

    pub fn receive(&mut self, incoming: PublishedMap) -> Result<()> {
        self.ensure_open()?;
        if self.selected.as_ref() != Some(&incoming.reference.product) {
            return Err("Snapshot is not from the selected exact Product".into());
        }
        incoming.snapshot.validate().map_err(|e| e.to_string())?;
        let scene = &incoming.snapshot.scenegraph;
        if scene.map
            != definition(
                &incoming.reference.product.peer_id,
                &self.domain,
                &self.session,
            )
        {
            return Err(
                "Snapshot belongs to a different demo session, Domain or frame contract".into(),
            );
        }
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
                || pose.rotation_wxyz != [1., 0., 0., 0.]
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
                "remote",
                incoming.reference.clone(),
                incoming.snapshot.clone(),
            )
            .map_err(|e| e.to_string())?;
        self.remote = Some(incoming);
        Ok(())
    }

    /// Coordinates carry an explicit frame, so a UI cannot submit stale pre-alignment coordinates.
    pub fn place(&mut self, name: &str, x: f64, y: f64, frame: &str) -> Result<()> {
        self.ensure_open()?;
        validate_name(name)?;
        let view = self.view()?;
        if view.display_frame != frame && view.local_frame != frame {
            return Err("Display frame changed; place the Portal again".into());
        }
        let pose = &view.local_to_display;
        // The demo validates every input as XY, fixed-heading: only translation is possible.
        let offset = if frame == view.local_frame {
            [0.; 3]
        } else {
            pose.translation
        };
        let x = x - offset[0];
        let y = y - offset[1];
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
                            rotation_wxyz: [1., 0., 0., 0.],
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

    pub fn view(&mut self) -> Result<View> {
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
        if let Some(remote) = &self.remote {
            for (id, anchor) in &local.snapshot.scenegraph.anchors {
                if let Some(other) = remote.snapshot.scenegraph.anchors.get(id) {
                    shared_names.push(anchor.payload.clone());
                    offsets.push((
                        anchor.payload.clone(),
                        [
                            anchor.pose_in_map.translation[0] - other.pose_in_map.translation[0],
                            anchor.pose_in_map.translation[1] - other.pose_in_map.translation[1],
                        ],
                    ));
                }
            }
            state = "separate".into();
            match self.checker.check("remote", "local") {
                AlignmentResult::Available { transform, .. } => {
                    state = "aligned".into();
                    if self.peer < remote.reference.product.peer_id {
                        remote_to_display = Some(transform);
                    } else {
                        let AlignmentResult::Available { transform, .. } =
                            self.checker.check("local", "remote")
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
        let local_portals = portals(&local, [0., 0., 0.]);
        let remote_portals = self
            .remote
            .as_ref()
            .map(|r| portals(r, [0., 0., 0.]))
            .unwrap_or_default();
        let mut combined: BTreeMap<String, PortalView> =
            portals(&local, local_to_display.translation)
                .into_iter()
                .map(|p| (p.id.clone(), p))
                .collect();
        if let (Some(remote), Some(transform)) = (&self.remote, remote_to_display) {
            for portal in portals(remote, transform.translation) {
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
            remote_peer: self
                .remote
                .as_ref()
                .map(|r| r.reference.product.peer_id.clone()),
            local_frame: frame.clone(),
            remote_frame: self
                .remote
                .as_ref()
                .map(|r| r.snapshot.scenegraph.map.frame.id.clone()),
            conflicts,
            display_frame: local_to_display.to_frame_id.clone(),
            local_to_display,
            local_reference: local.reference,
            remote_reference: self.remote.as_ref().map(|r| r.reference.clone()),
            shared_names,
            portals: combined.into_values().collect(),
            local_portals,
            remote_portals,
        })
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
fn portals(map: &PublishedMap, offset: [f64; 3]) -> Vec<PortalView> {
    map.snapshot
        .scenegraph
        .anchors
        .values()
        .map(|anchor| PortalView {
            id: anchor.anchor_id.clone(),
            name: anchor.payload.clone(),
            x: anchor.pose_in_map.translation[0] + offset[0],
            y: anchor.pose_in_map.translation[1] + offset[1],
            contributors: vec![map.reference.product.peer_id.clone()],
        })
        .collect()
}
