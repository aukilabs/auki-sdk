use crate::core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = metersPerUnitJson)]
pub fn meters_per_unit_json(unit: String) -> Result<f64, JsValue> {
    core::meters_per_unit_json(&unit).map_err(geometry_error)
}

#[wasm_bindgen(js_name = axisConventionMatrixJson)]
pub fn axis_convention_matrix_json(
    from_axes_json: String,
    to_axes_json: String,
) -> Result<String, JsValue> {
    core::axis_convention_matrix_json(&from_axes_json, &to_axes_json).map_err(geometry_error)
}

#[wasm_bindgen(js_name = conventionMatrixJson)]
pub fn convention_matrix_json(
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, JsValue> {
    core::convention_matrix_json(&from_frame_json, &to_frame_json).map_err(geometry_error)
}

#[wasm_bindgen(js_name = convertPointConventionJson)]
pub fn convert_point_convention_json(
    point_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, JsValue> {
    core::convert_point_convention_json(&point_json, &from_frame_json, &to_frame_json)
        .map_err(geometry_error)
}

#[wasm_bindgen(js_name = convertVectorConventionJson)]
pub fn convert_vector_convention_json(
    vector_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, JsValue> {
    core::convert_vector_convention_json(&vector_json, &from_frame_json, &to_frame_json)
        .map_err(geometry_error)
}

#[wasm_bindgen(js_name = convertDirectionConventionJson)]
pub fn convert_direction_convention_json(
    direction_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, JsValue> {
    core::convert_direction_convention_json(&direction_json, &from_frame_json, &to_frame_json)
        .map_err(geometry_error)
}

#[wasm_bindgen(js_name = convertPoseConventionJson)]
pub fn convert_pose_convention_json(
    pose_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, JsValue> {
    core::convert_pose_convention_json(&pose_json, &from_frame_json, &to_frame_json)
        .map_err(geometry_error)
}

fn geometry_error(err: core::GeometryError) -> JsValue {
    JsValue::from_str(&err.to_string())
}
