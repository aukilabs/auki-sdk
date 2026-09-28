//! Resolve individual anchors from live peer-owned maps, without exporting snapshots.
use crate::{MapDefinition, QrAnchor, Scenegraph, component::*};
use auki_components::{InMemoryTransport, InvocationContext};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedQr {
    pub snapshot: SnapshotReference,
    pub map: MapDefinition,
    pub anchor: QrAnchor,
}
impl ResolvedQr {
    pub fn validate(&self) -> Result<(), ResolutionError> {
        Scenegraph::new(self.map.clone()).map_err(|e| ResolutionError(e.to_string()))?;
        self.anchor
            .validate_in_map(&self.map)
            .map_err(|e| ResolutionError(e.to_string()))
    }
}
#[derive(Debug, thiserror::Error)]
#[error("QR resolution failed: {0}")]
pub struct ResolutionError(pub String);

/// Empty means absent, not an unavailable map. One result per matching map;
/// transforms from different maps must not be fused without an alignment.
/// Alternative resolvers may explicitly source an individual anchor remotely.
pub trait QrResolver: Send + Sync {
    fn resolve(&self, payload: &str) -> Result<Vec<ResolvedQr>, ResolutionError>;

    /// Resolve one exact map revision. No fallback to another map or revision.
    fn resolve_in(
        &self,
        payload: &str,
        target: &SnapshotReference,
    ) -> Result<ResolvedQr, ResolutionError> {
        let mut matches = self
            .resolve(payload)?
            .into_iter()
            .filter(|qr| &qr.snapshot == target);
        let qr = matches.next().ok_or_else(|| {
            ResolutionError("portal absent or requested map revision unavailable".into())
        })?;
        if matches.next().is_some() {
            return Err(ResolutionError("ambiguous portal in selected map".into()));
        }
        Ok(qr)
    }
}

/// Default resolver: query registered local Map Components at observation time.
/// Clones share the registry; adding anchors/maps needs no localizer rebind.
#[derive(Clone)]
pub struct PortalMaps {
    context: InvocationContext,
    maps: Arc<RwLock<Vec<Arc<MapComponent>>>>,
}
impl PortalMaps {
    pub fn new(context: InvocationContext) -> Self {
        Self {
            context,
            maps: Arc::new(RwLock::new(Vec::new())),
        }
    }
    pub fn context(&self) -> &InvocationContext {
        &self.context
    }
    pub fn register(&self, map: Arc<MapComponent>) -> Result<(), ResolutionError> {
        if map.component().reference().peer_id != self.context.caller_peer_id {
            return Err(ResolutionError(
                "map must be owned by the local peer; import placements explicitly".into(),
            ));
        }
        let mut maps = self
            .maps
            .write()
            .map_err(|_| ResolutionError("registry unavailable".into()))?;
        if maps.iter().any(|m| Arc::ptr_eq(m, &map)) {
            return Ok(());
        }
        if maps.len() >= 64 {
            return Err(ResolutionError("local map limit reached".into()));
        }
        maps.push(map);
        Ok(())
    }
    pub fn contains(&self, map: &Arc<MapComponent>) -> bool {
        self.maps
            .read()
            .is_ok_and(|maps| maps.iter().any(|m| Arc::ptr_eq(m, map)))
    }
    pub fn unregister(&self, map: &Arc<MapComponent>) {
        if let Ok(mut maps) = self.maps.write() {
            maps.retain(|m| !Arc::ptr_eq(m, map));
        }
    }
}
impl QrResolver for PortalMaps {
    fn resolve_in(
        &self,
        payload: &str,
        target: &SnapshotReference,
    ) -> Result<ResolvedQr, ResolutionError> {
        let map = self
            .maps
            .read()
            .map_err(|_| ResolutionError("registry unavailable".into()))?
            .iter()
            .find(|map| map.product().reference() == target.product)
            .cloned()
            .ok_or_else(|| ResolutionError("selected map is not registered locally".into()))?;
        let answer = InMemoryTransport
            .invoke(
                map.find_qr(),
                self.context.clone(),
                FindQr {
                    payload: payload.into(),
                },
            )
            .map_err(|e| ResolutionError(e.to_string()))?
            .result;
        if &answer.snapshot != target {
            return Err(ResolutionError(
                "requested map revision unavailable; choose a current candidate".into(),
            ));
        }
        if answer.anchors.len() != 1 {
            return Err(ResolutionError(
                "portal absent or ambiguous in selected map".into(),
            ));
        }
        let qr = ResolvedQr {
            snapshot: answer.snapshot,
            map: answer.map,
            anchor: answer.anchors.into_iter().next().unwrap(),
        };
        qr.validate()?;
        Ok(qr)
    }
    fn resolve(&self, payload: &str) -> Result<Vec<ResolvedQr>, ResolutionError> {
        let maps = self
            .maps
            .read()
            .map_err(|_| ResolutionError("registry unavailable".into()))?
            .clone();
        let mut results = Vec::new();
        for map in maps {
            let answer = InMemoryTransport
                .invoke(
                    map.find_qr(),
                    self.context.clone(),
                    FindQr {
                        payload: payload.into(),
                    },
                )
                .map_err(|e| ResolutionError(e.to_string()))?
                .result;
            if answer.anchors.len() > 1 {
                return Err(ResolutionError(
                    "ambiguous QR payload within a local map".into(),
                ));
            }
            if let Some(anchor) = answer.anchors.into_iter().next() {
                results.push(ResolvedQr {
                    snapshot: answer.snapshot,
                    map: answer.map,
                    anchor,
                });
            }
        }
        Ok(results)
    }
}
