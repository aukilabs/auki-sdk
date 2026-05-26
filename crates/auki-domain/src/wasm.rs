use crate::core;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

#[wasm_bindgen]
pub struct AukiBrowserDomainPeerCore {
    state: BrowserPeerState,
}

#[wasm_bindgen]
impl AukiBrowserDomainPeerCore {
    #[wasm_bindgen(constructor)]
    pub fn new(config_json: String) -> Result<AukiBrowserDomainPeerCore, JsValue> {
        let config: BrowserPeerConfig = serde_json::from_str(&config_json).map_err(json_error)?;
        let mut participants = BTreeMap::new();
        participants.insert(
            config.peer_id.clone(),
            BrowserParticipant {
                peer_id: config.peer_id.clone(),
                app: Some(config.app_id.clone()),
                name: Some(config.display_name.clone()),
                is_self: true,
                is_manager: false,
                connected: true,
                multiaddrs: config.multiaddrs.clone(),
                sensors: Vec::new(),
            },
        );
        Ok(Self {
            state: BrowserPeerState {
                self_peer_id: config.peer_id,
                app_id: config.app_id,
                display_name: config.display_name,
                advertised_multiaddrs: config.multiaddrs,
                domain_name: None,
                manager_peer_id: None,
                role: BrowserRole::Idle,
                participants,
            },
        })
    }

    #[wasm_bindgen(js_name = snapshotJson)]
    pub fn snapshot_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.state.snapshot()).map_err(json_error)
    }

    #[wasm_bindgen(js_name = debugStateJson)]
    pub fn debug_state_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(&BrowserDebugState {
            self_peer_id: self.state.self_peer_id.clone(),
            app_id: self.state.app_id.clone(),
            display_name: self.state.display_name.clone(),
            advertised_multiaddrs: self.state.advertised_multiaddrs.clone(),
            domain_name: self.state.domain_name.clone(),
            manager_peer_id: self.state.manager_peer_id.clone(),
            role: self.state.role.as_str().to_string(),
            participant_count: self.state.participants.len(),
        })
        .map_err(json_error)
    }

    #[wasm_bindgen(js_name = declareSensorsJson)]
    pub fn declare_sensors_json(&mut self, sensors_json: String) -> Result<String, JsValue> {
        let sensors = parse_sensors(&sensors_json)?;
        if let Some(self_participant) = self.state.participants.get_mut(&self.state.self_peer_id) {
            self_participant.sensors = sensors;
        }
        self.snapshot_json()
    }

    #[wasm_bindgen(js_name = sensorCatalogJson)]
    pub fn sensor_catalog_json(&self) -> Result<String, JsValue> {
        let sensors = self
            .state
            .participants
            .get(&self.state.self_peer_id)
            .map(|participant| participant.sensors.clone())
            .unwrap_or_default();
        serde_json::to_string(&serde_json::json!({ "sensors": sensors })).map_err(json_error)
    }

    #[wasm_bindgen(js_name = resourceCatalogJson)]
    pub fn resource_catalog_json(&self) -> Result<String, JsValue> {
        let resources = self
            .state
            .participants
            .get(&self.state.self_peer_id)
            .map(|participant| {
                participant
                    .sensors
                    .iter()
                    .map(|sensor| {
                        serde_json::json!({
                            "kind": "sensor_stream",
                            "id": sensor.sensor_id,
                            "sensor_id": sensor.sensor_id,
                            "sensor_hash": sensor.sensor_hash,
                            "sensor_kind": sensor.kind,
                            "stream_protocol": "/auki/stream/0.1.0",
                            "payload": sensor.kind,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        serde_json::to_string(&serde_json::json!({ "resources": resources })).map_err(json_error)
    }

    #[wasm_bindgen(js_name = mergeParticipantJson)]
    pub fn merge_participant_json(&mut self, participant_json: String) -> Result<String, JsValue> {
        let update: BrowserParticipantUpdate =
            serde_json::from_str(&participant_json).map_err(json_error)?;
        let participant = self
            .state
            .participants
            .entry(update.peer_id.clone())
            .or_insert_with(|| BrowserParticipant {
                peer_id: update.peer_id.clone(),
                app: None,
                name: None,
                is_self: update.peer_id == self.state.self_peer_id,
                is_manager: false,
                connected: false,
                multiaddrs: Vec::new(),
                sensors: Vec::new(),
            });
        if update.app.is_some() {
            participant.app = update.app;
        }
        if update.name.is_some() {
            participant.name = update.name;
        }
        if let Some(is_manager) = update.is_manager {
            participant.is_manager = is_manager;
        }
        if let Some(connected) = update.connected {
            participant.connected = connected;
        }
        if let Some(multiaddrs) = update.multiaddrs {
            participant.multiaddrs = multiaddrs;
        }
        if let Some(sensors) = update.sensors {
            participant.sensors = sensors;
        }
        self.snapshot_json()
    }

    #[wasm_bindgen(js_name = createDomainJson)]
    pub fn create_domain_json(&mut self, cluster_name: String) -> Result<String, JsValue> {
        self.state.domain_name = Some(cluster_name);
        self.state.manager_peer_id = Some(self.state.self_peer_id.clone());
        self.state.role = BrowserRole::Manager;
        if let Some(self_participant) = self.state.participants.get_mut(&self.state.self_peer_id) {
            self_participant.is_manager = true;
        }
        self.snapshot_json()
    }

    #[wasm_bindgen(js_name = applyJoinAcceptJson)]
    pub fn apply_join_accept_json(&mut self, response_json: String) -> Result<String, JsValue> {
        let response: BrowserJoinResponse =
            serde_json::from_str(&response_json).map_err(json_error)?;
        if response.kind != "accept" {
            return Err(JsValue::from_str(
                response.reason.as_deref().unwrap_or("join rejected"),
            ));
        }
        let membership: BrowserMembership =
            serde_json::from_str(&response.membership_json).map_err(json_error)?;
        let existing_self = self
            .state
            .participants
            .get(&self.state.self_peer_id)
            .cloned();
        self.state.domain_name = Some(membership.cluster_name);
        self.state.manager_peer_id = Some(response.manager_peer_id.clone());
        self.state.role = BrowserRole::Member;
        self.state.participants.clear();
        for member in membership.peers {
            let mut participant = BrowserParticipant {
                peer_id: member.peer_id.clone(),
                app: None,
                name: None,
                is_self: member.peer_id == self.state.self_peer_id,
                is_manager: member.peer_id == response.manager_peer_id,
                connected: true,
                multiaddrs: member.multiaddrs,
                sensors: Vec::new(),
            };
            if participant.is_self {
                if let Some(existing_self) = existing_self.clone() {
                    participant.app = existing_self.app;
                    participant.name = existing_self.name;
                    participant.sensors = existing_self.sensors;
                }
            }
            self.state
                .participants
                .insert(participant.peer_id.clone(), participant);
        }
        self.snapshot_json()
    }

    #[wasm_bindgen(js_name = handleJoinRequestJson)]
    pub fn handle_join_request_json(&mut self, request_json: String) -> Result<String, JsValue> {
        if self.state.role != BrowserRole::Manager {
            return serde_json::to_string(&serde_json::json!({
                "kind": "reject",
                "reason": "not_manager",
            }))
            .map_err(json_error);
        }
        let request: BrowserJoinRequest =
            serde_json::from_str(&request_json).map_err(json_error)?;
        self.state
            .participants
            .insert(request.peer_id.clone(), request.into_participant());
        serde_json::to_string(&serde_json::json!({
            "kind": "accept",
            "membership_json": self.state.membership_json()?,
            "manager_peer_id": self.state.self_peer_id,
        }))
        .map_err(json_error)
    }
}

#[derive(Debug, Deserialize)]
struct BrowserPeerConfig {
    peer_id: String,
    app_id: String,
    display_name: String,
    #[serde(default)]
    multiaddrs: Vec<String>,
}

#[derive(Debug)]
struct BrowserPeerState {
    self_peer_id: String,
    app_id: String,
    display_name: String,
    advertised_multiaddrs: Vec<String>,
    domain_name: Option<String>,
    manager_peer_id: Option<String>,
    role: BrowserRole,
    participants: BTreeMap<String, BrowserParticipant>,
}

impl BrowserPeerState {
    fn snapshot(&self) -> BrowserPeerSnapshot {
        BrowserPeerSnapshot {
            self_peer_id: self.self_peer_id.clone(),
            domain_name: self.domain_name.clone(),
            manager_peer_id: self.manager_peer_id.clone(),
            role: self.role.as_str().to_string(),
            participants: self.participants.values().cloned().collect(),
        }
    }

    fn membership_json(&self) -> Result<String, JsValue> {
        let peers = self
            .participants
            .values()
            .enumerate()
            .map(|(idx, participant)| {
                serde_json::json!({
                    "peer_id": participant.peer_id,
                    "multiaddrs": participant.multiaddrs,
                    "join_ts_ns": idx as u64 + 1,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&serde_json::json!({
            "cluster_name": self.domain_name.clone().unwrap_or_default(),
            "peers": peers,
        }))
        .map_err(json_error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserRole {
    Idle,
    Manager,
    Member,
}

impl BrowserRole {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Manager => "manager",
            Self::Member => "member",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserPeerSnapshot {
    self_peer_id: String,
    domain_name: Option<String>,
    manager_peer_id: Option<String>,
    role: String,
    participants: Vec<BrowserParticipant>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserDebugState {
    self_peer_id: String,
    app_id: String,
    display_name: String,
    advertised_multiaddrs: Vec<String>,
    domain_name: Option<String>,
    manager_peer_id: Option<String>,
    role: String,
    participant_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserParticipant {
    peer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    is_self: bool,
    is_manager: bool,
    connected: bool,
    #[serde(default)]
    multiaddrs: Vec<String>,
    #[serde(default)]
    sensors: Vec<BrowserSensor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserSensor {
    sensor_id: String,
    sensor_hash: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserJoinRequest {
    peer_id: String,
    #[serde(default)]
    multiaddrs: Vec<String>,
    participant_info_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserParticipantUpdate {
    peer_id: String,
    app: Option<String>,
    name: Option<String>,
    is_manager: Option<bool>,
    connected: Option<bool>,
    multiaddrs: Option<Vec<String>>,
    sensors: Option<Vec<BrowserSensor>>,
}

impl BrowserJoinRequest {
    fn into_participant(self) -> BrowserParticipant {
        let mut participant = BrowserParticipant {
            peer_id: self.peer_id,
            app: None,
            name: None,
            is_self: false,
            is_manager: false,
            connected: true,
            multiaddrs: self.multiaddrs,
            sensors: Vec::new(),
        };
        if let Some(info_json) = self.participant_info_json {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&info_json) {
                participant.app = value
                    .get("app")
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string);
                participant.name = value
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string);
            }
        }
        participant
    }
}

#[derive(Debug, Deserialize)]
struct BrowserJoinResponse {
    kind: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    membership_json: String,
    #[serde(default)]
    manager_peer_id: String,
}

#[derive(Debug, Deserialize)]
struct BrowserMembership {
    cluster_name: String,
    peers: Vec<BrowserMembershipPeer>,
}

#[derive(Debug, Deserialize)]
struct BrowserMembershipPeer {
    peer_id: String,
    #[serde(default)]
    multiaddrs: Vec<String>,
}

fn parse_sensors(json: &str) -> Result<Vec<BrowserSensor>, JsValue> {
    if let Ok(sensors) = serde_json::from_str::<Vec<BrowserSensor>>(json) {
        return Ok(sensors);
    }
    #[derive(Deserialize)]
    struct SensorCatalog {
        sensors: Vec<BrowserSensor>,
    }
    serde_json::from_str::<SensorCatalog>(json)
        .map(|catalog| catalog.sensors)
        .map_err(json_error)
}

fn json_error(err: serde_json::Error) -> JsValue {
    JsValue::from_str(&err.to_string())
}

fn domain_error(err: core::DomainDataError) -> JsValue {
    JsValue::from_str(&err.to_string())
}
