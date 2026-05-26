use crate::core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = clusterMembershipNewJson)]
pub fn cluster_membership_new_json(cluster_name: String) -> String {
    core::cluster_membership_new_json(&cluster_name)
}

#[wasm_bindgen(js_name = clusterMembershipFilenameJson)]
pub fn cluster_membership_filename_json(membership_json: String) -> Result<String, JsValue> {
    core::cluster_membership_filename_json(&membership_json).map_err(domain_error)
}

#[wasm_bindgen(js_name = clusterMembershipPeerCountJson)]
pub fn cluster_membership_peer_count_json(membership_json: String) -> Result<u64, JsValue> {
    core::cluster_membership_peer_count_json(&membership_json).map_err(domain_error)
}

#[wasm_bindgen(js_name = clusterMembershipAdmitMemberJson)]
pub fn cluster_membership_admit_member_json(
    membership_json: String,
    member_json: String,
) -> Result<String, JsValue> {
    core::cluster_membership_admit_member_json(&membership_json, &member_json).map_err(domain_error)
}

#[wasm_bindgen(js_name = electSuccessorJson)]
pub fn elect_successor_json(
    membership_json: String,
    local_peer_id: String,
    connected_peer_ids: Vec<String>,
) -> Result<Option<String>, JsValue> {
    core::elect_successor_json(&membership_json, &local_peer_id, connected_peer_ids)
        .map_err(domain_error)
}

#[wasm_bindgen(js_name = validateMembershipJson)]
pub fn validate_membership_json(membership_json: String) -> Result<String, JsValue> {
    core::validate_membership_json(&membership_json).map_err(domain_error)
}

#[wasm_bindgen(js_name = validateParticipantInfoJson)]
pub fn validate_participant_info_json(json: String) -> Result<String, JsValue> {
    core::validate_participant_info_json(&json).map_err(domain_error)
}

#[wasm_bindgen(js_name = validateSensorCatalogJson)]
pub fn validate_sensor_catalog_json(json: String) -> Result<String, JsValue> {
    core::validate_sensor_catalog_json(&json).map_err(domain_error)
}

#[wasm_bindgen(js_name = validateResourceCatalogJson)]
pub fn validate_resource_catalog_json(json: String) -> Result<String, JsValue> {
    core::validate_resource_catalog_json(&json).map_err(domain_error)
}

#[wasm_bindgen(js_name = validateRegistryEntryJson)]
pub fn validate_registry_entry_json(json: String) -> Result<String, JsValue> {
    core::validate_registry_entry_json(&json).map_err(domain_error)
}

#[wasm_bindgen(js_name = domainSuccessorJson)]
pub fn domain_successor_json(membership_json: String, peer_id: String) -> Result<String, JsValue> {
    core::domain_successor_json(&membership_json, &peer_id).map_err(domain_error)
}

fn domain_error(err: core::DomainDataError) -> JsValue {
    JsValue::from_str(&err.to_string())
}
