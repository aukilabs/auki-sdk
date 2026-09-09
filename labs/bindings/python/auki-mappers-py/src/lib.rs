//! Python boundary for SDK-native Mappers.

// PyO3 0.22 proc-macro expansions trigger these Rust 2024/Clippy lints. They
// cannot be corrected in handwritten wrapper code without changing the shared
// binding ABI, so scope the compatibility allowance to this crate.
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::useless_conversion)]

use auki_datatypes::{
    point_cloud::Data as PointCloudData,
    pose::{Quat, SpatialTransform, Vec3},
};
use auki_mappers_rs::Voxelizer as RustVoxelizer;
use auki_registry::{SensorBody, SensorRegistryEntry};
use prost::Message;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyModule};

fn parse_sensor_entry(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<SensorRegistryEntry> {
    let json = py.import_bound("json")?;
    let encoded: String = json.call_method1("dumps", (value,))?.extract()?;
    serde_json::from_str(&encoded)
        .map_err(|error| PyValueError::new_err(format!("sensor_registry_entry: {error}")))
}

/// Stateless voxel Mapper algorithm. Applications supply only SDK payloads
/// and registry/pose contracts; this module has no robot-facing surface.
#[pyclass(module = "auki_mappers")]
struct Voxelizer {
    inner: RustVoxelizer,
}

#[pymethods]
impl Voxelizer {
    #[new]
    #[pyo3(signature = (*, voxel_size_m, chunk_dimension))]
    fn new(voxel_size_m: f64, chunk_dimension: u32) -> PyResult<Self> {
        let inner = RustVoxelizer::new(voxel_size_m, chunk_dimension)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self { inner })
    }

    /// Convert one normalized SDK point-cloud payload and its aligned SDK pose
    /// into encoded `auki_datatypes.map.MapUpdate` bytes.
    #[pyo3(signature = (point_cloud_data, sensor_registry_entry, pose, *, free_delta=-0.25, occupied_delta=1.0))]
    fn map_point_cloud<'py>(
        &self,
        py: Python<'py>,
        point_cloud_data: &[u8],
        sensor_registry_entry: &Bound<'_, PyAny>,
        pose: Vec<f64>,
        free_delta: f32,
        occupied_delta: f32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        if pose.len() != 7 {
            return Err(PyValueError::new_err(
                "pose must contain [tx, ty, tz, qx, qy, qz, qw]",
            ));
        }
        let entry = parse_sensor_entry(py, sensor_registry_entry)?;
        let SensorBody::Rangefinder(layout) = entry.body else {
            return Err(PyValueError::new_err(
                "sensor_registry_entry must be a rangefinder",
            ));
        };
        let transform = SpatialTransform {
            translation: Some(Vec3 {
                x: pose[0],
                y: pose[1],
                z: pose[2],
            }),
            orientation: Some(Quat {
                x: pose[3],
                y: pose[4],
                z: pose[5],
                w: pose[6],
            }),
        };
        let update = self
            .inner
            .map_point_cloud(
                &PointCloudData {
                    data: point_cloud_data.to_vec(),
                },
                &layout,
                &transform,
                free_delta,
                occupied_delta,
            )
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(PyBytes::new_bound(py, &update.encode_to_vec()))
    }
}

#[pymodule]
fn auki_mappers(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Voxelizer>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_datatypes::map::MapUpdate;
    use prost::Message;
    use pyo3::types::{PyDict, PyModule};

    #[test]
    fn module_exposes_voxelizer() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_mappers").unwrap();
            auki_mappers(py, &module).unwrap();
            assert!(module.getattr("Voxelizer").is_ok());
        });
    }

    #[test]
    fn rejects_non_rangefinder_registry_entry() {
        Python::with_gil(|py| {
            let voxelizer = Voxelizer::new(0.05, 16).unwrap();
            let entry = PyDict::new_bound(py);
            entry.set_item("peer_id", "peer").unwrap();
            entry.set_item("sensor_id", "camera").unwrap();
            entry.set_item("kind", "audio").unwrap();
            entry.set_item("type", "pcm").unwrap();
            let error = voxelizer
                .map_point_cloud(py, &[], entry.as_any(), vec![0.0; 7], -0.25, 1.0)
                .unwrap_err();
            assert!(error.to_string().contains("sensor_registry_entry"));
        });
    }

    #[test]
    fn maps_normalized_sdk_point_bytes_to_encoded_update() {
        Python::with_gil(|py| {
            let voxelizer = Voxelizer::new(1.0, 16).unwrap();
            let json = py.import_bound("json").unwrap();
            let entry = json
                .call_method1(
                    "loads",
                    (r#"{
                        "peer_id":"peer","sensor_id":"lidar","kind":"rangefinder",
                        "type":"point_cloud","fields":[
                            {"name":"x","offset":0,"datatype":"float32","count":1},
                            {"name":"y","offset":4,"datatype":"float32","count":1},
                            {"name":"z","offset":8,"datatype":"float32","count":1}
                        ],"point_step":12,"is_bigendian":false,"frame":{
                            "peer_id":"peer","id":"lidar_frame","hash":"frame-hash"
                        },"frame_rate_hz":10
                    }"#,),
                )
                .unwrap();
            let mut point = Vec::new();
            point.extend_from_slice(&1.25_f32.to_le_bytes());
            point.extend_from_slice(&2.25_f32.to_le_bytes());
            point.extend_from_slice(&3.25_f32.to_le_bytes());
            let encoded = voxelizer
                .map_point_cloud(
                    py,
                    &point,
                    &entry,
                    vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                    -0.25,
                    1.0,
                )
                .unwrap();
            let update = MapUpdate::decode(encoded.as_bytes()).unwrap();
            assert!(!update.voxel_chunks.is_empty());
        });
    }
}
