//! wasm-bindgen bindings for [`auki-dsc`](../../../auki-dsc).
//!
//! Throwaway-friendly surface for gotu-web PathfindDebug:
//! `bake(obj, radius)` once, then `find_path(waypoints)` on drag.

use auki_dsc::{BakeProfile, NavMesh, Vec3, parse_obj};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct JsVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<Vec3> for JsVec3 {
    fn from(v: Vec3) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }
}

impl From<JsVec3> for Vec3 {
    fn from(v: JsVec3) -> Self {
        Vec3::new(v.x, v.y, v.z)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsOnMeshPoint {
    pub original: JsVec3,
    pub adjusted: JsVec3,
    #[serde(rename = "isOffMesh")]
    pub is_off_mesh: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsPathResult {
    pub full: Vec<JsVec3>,
    #[serde(rename = "totalDistance")]
    pub total_distance: f32,
    pub waypoints: Vec<JsOnMeshPoint>,
}

/// Baked navmesh handle. Keep one per domain OBJ.
#[wasm_bindgen]
pub struct WasmNavMesh {
    inner: NavMesh,
}

#[wasm_bindgen]
impl WasmNavMesh {
    /// Parse OBJ text and bake with [`BakeProfile::dsc`]. `radius` is **metres**
    /// (converted to voxels via ceil). Pass `0.0` only to bit-match legacy DSC's
    /// broken float→int truncation.
    #[wasm_bindgen]
    pub fn bake(obj_text: &str, radius: f32) -> Result<WasmNavMesh, JsValue> {
        let mesh = parse_obj(obj_text).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let profile = BakeProfile::dsc(radius);
        let inner =
            NavMesh::bake(&mesh, &profile).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self { inner })
    }

    /// Path through `waypoints` (`[{x,y,z}, …]`, Y-up metres). Returns DSC-shaped JSON.
    #[wasm_bindgen(js_name = findPath)]
    pub fn find_path(&self, waypoints: JsValue) -> Result<JsValue, JsValue> {
        let pts: Vec<JsVec3> = serde_wasm_bindgen::from_value(waypoints)
            .map_err(|e| JsValue::from_str(&format!("waypoints: {e}")))?;
        if pts.len() < 2 {
            return Err(JsValue::from_str("need at least 2 waypoints"));
        }
        let wps: Vec<Vec3> = pts.into_iter().map(Into::into).collect();
        let path = self
            .inner
            .find_path(&wps)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let out = JsPathResult {
            full: path.full.into_iter().map(Into::into).collect(),
            total_distance: path.total_distance,
            waypoints: path
                .waypoints
                .into_iter()
                .map(|p| JsOnMeshPoint {
                    original: p.original.into(),
                    adjusted: p.adjusted.into(),
                    is_off_mesh: p.is_off_mesh,
                })
                .collect(),
        };
        serde_wasm_bindgen::to_value(&out).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(getter, js_name = agentRadius)]
    pub fn agent_radius(&self) -> f32 {
        self.inner.agent_radius()
    }
}
