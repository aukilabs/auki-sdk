//! wasm-bindgen surface for [`auki-navigation`](../../../auki-navigation).
//!
//! Mesh ingest (`parseObj` / `fromIndexed`) is [`auki-geometry`]. Bake/path
//! is compute. `bakeObj` is a non-canonical debug helper.

use auki_geometry::mesh::Vec3 as MeshVec3;
use auki_geometry::{TriangleMesh, parse_obj};
use auki_navigation::{BakeProfile, NavMesh, Vec3};
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

/// Ingested triangle mesh. Pass to [`WasmNavMesh::bake`].
#[wasm_bindgen]
pub struct WasmTriangleMesh {
    inner: TriangleMesh,
}

#[wasm_bindgen]
impl WasmTriangleMesh {
    /// OBJ format adapter → mesh. Not a compute op.
    #[wasm_bindgen(js_name = parseObj)]
    pub fn parse_obj(obj_text: &str) -> Result<WasmTriangleMesh, JsValue> {
        let inner = parse_obj(obj_text).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self { inner })
    }

    /// Verts `[{x,y,z}, …]` + tris `[[i,j,k], …]`.
    #[wasm_bindgen(js_name = fromIndexed)]
    pub fn from_indexed(vertices: JsValue, indices: JsValue) -> Result<WasmTriangleMesh, JsValue> {
        let pts: Vec<JsVec3> = serde_wasm_bindgen::from_value(vertices)
            .map_err(|e| JsValue::from_str(&format!("vertices: {e}")))?;
        let tris: Vec<[u32; 3]> = serde_wasm_bindgen::from_value(indices)
            .map_err(|e| JsValue::from_str(&format!("indices: {e}")))?;
        let verts: Vec<MeshVec3> = pts
            .into_iter()
            .map(|p| MeshVec3::new(p.x, p.y, p.z))
            .collect();
        let inner = TriangleMesh::from_indexed(verts, tris, Vec::new())
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self { inner })
    }

    #[wasm_bindgen(getter, js_name = vertexCount)]
    pub fn vertex_count(&self) -> usize {
        self.inner.vertices.len()
    }

    #[wasm_bindgen(getter, js_name = triangleCount)]
    pub fn triangle_count(&self) -> usize {
        self.inner.indices.len()
    }
}

/// Baked navmesh handle. Keep one per [`WasmTriangleMesh`] bake.
#[wasm_bindgen]
pub struct WasmNavMesh {
    inner: NavMesh,
}

#[wasm_bindgen]
impl WasmNavMesh {
    /// Bake [`WasmTriangleMesh`] with [`BakeProfile::dsc`]. `radius` is **metres**.
    #[wasm_bindgen]
    pub fn bake(mesh: &WasmTriangleMesh, radius: f32) -> Result<WasmNavMesh, JsValue> {
        let profile = BakeProfile::dsc(radius);
        let inner =
            NavMesh::bake(&mesh.inner, &profile).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self { inner })
    }

    /// Non-canonical debug helper: parse OBJ then bake. Prefer `parseObj` + `bake`.
    #[wasm_bindgen(js_name = bakeObj)]
    pub fn bake_obj(obj_text: &str, radius: f32) -> Result<WasmNavMesh, JsValue> {
        let mesh = WasmTriangleMesh::parse_obj(obj_text)?;
        Self::bake(&mesh, radius)
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
