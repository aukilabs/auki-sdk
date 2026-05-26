#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_cluster_lifecycle_is_exposed() {
    // binding-surface: native cluster lifecycle
    let (_server, manager) = bootstrap_binding_manager("binding-lifecycle", 91).await;
    assert_eq!(manager.cluster_name(), "binding-lifecycle");
    assert!(!manager.local_peer_id().is_empty());
    assert!(manager.is_manager());
    assert_eq!(manager.peer_count(), 1);
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_manager_admission_is_exposed() {
    // binding-surface: native manager admission
    let (_server, manager) = bootstrap_binding_manager("binding-admit", 92).await;
    let peer_id = binding_peer_id(93);
    let member_json = manager
        .admit_peer(peer_id.clone(), vec!["/ip4/127.0.0.1/tcp/49093".into()])
        .await
        .unwrap();
    let member: serde_json::Value = serde_json::from_str(&member_json).unwrap();
    assert_eq!(member["peer_id"], peer_id);
    assert_eq!(manager.peer_count(), 2);
    assert!(matches!(
        manager
            .admit_peer("not-a-peer".into(), vec![])
            .await
            .unwrap_err(),
        auki_domain::BindingDomainError::InvalidPeerId { .. }
    ));
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_membership_inspection_is_exposed() {
    // binding-surface: native membership inspection
    let (_server, manager) = bootstrap_binding_manager("binding-membership", 94).await;
    let membership: serde_json::Value =
        serde_json::from_str(&manager.membership_json().unwrap()).unwrap();
    assert_eq!(membership["cluster_name"], "binding-membership");
    assert_eq!(membership["peers"].as_array().unwrap().len(), 1);
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_participant_info_is_exposed() {
    // binding-surface: native participant info
    let (_server, manager) = bootstrap_binding_manager("binding-info", 95).await;
    let info: serde_json::Value =
        serde_json::from_str(&manager.participant_info_json().unwrap()).unwrap();
    assert_eq!(info["app"], "binding-test");
    assert_eq!(info["is_manager"], true);
    assert_eq!(info["manager_peer_id"], manager.local_peer_id());
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_clock_estimates_are_exposed() {
    // binding-surface: native domain time and clock estimates
    let (_server, manager) = bootstrap_binding_manager("binding-clock", 96).await;
    let estimates: serde_json::Value =
        serde_json::from_str(&manager.clock_sync_estimates_json().unwrap()).unwrap();
    assert!(estimates["estimates"].as_array().unwrap().is_empty());
    let peer_estimate: serde_json::Value = serde_json::from_str(
        &manager
            .clock_sync_estimate_json(manager.local_peer_id())
            .unwrap(),
    )
    .unwrap();
    assert!(peer_estimate["estimate"].is_null());
    let domain: serde_json::Value =
        serde_json::from_str(&manager.domain_clock_estimate_json().unwrap()).unwrap();
    assert_eq!(domain["cluster_name"], "binding-clock");
    assert_eq!(domain["total_offset_ns"], 0);
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_diagnostics_are_exposed() {
    // binding-surface: native diagnostics
    let (_server, manager) = bootstrap_binding_manager("binding-diagnostics", 97).await;
    manager
        .broadcast_diagnostic_message_json(
            r#"{"topic":"binding.test","payload_json":"{\"ok\":true}"}"#.into(),
        )
        .unwrap();
    assert!(manager.drain_diagnostic_messages_json(10).is_empty());
    let membership_events = manager.drain_membership_events_json(10);
    assert_eq!(membership_events.len(), 1);
    let event: serde_json::Value = serde_json::from_str(&membership_events[0]).unwrap();
    assert_eq!(event["kind"], "membership_snapshot");
    assert_eq!(event["membership"]["cluster_name"], "binding-diagnostics");
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_catalog_and_registry_providers_are_exposed() {
    // binding-surface: native catalog and registry providers
    let (_server, manager) = bootstrap_binding_manager("binding-providers", 98).await;
    let (sensor_json, sensor_hash) = sensor_registry_json("provider-camera");
    let sensor_catalog = serde_json::json!({
        "sensors": [{
            "sensor_id": "provider-camera",
            "sensor_hash": sensor_hash,
            "kind": "camera"
        }]
    })
    .to_string();
    manager
        .set_sensor_catalog_provider(std::sync::Arc::new(JsonSensorProvider {
            json: sensor_catalog,
        }))
        .unwrap();
    manager
        .set_resource_catalog_provider(std::sync::Arc::new(JsonResourceProvider {
            json: r#"{"resources":[]}"#.into(),
        }))
        .unwrap();
    manager
        .set_registry_entry_provider(std::sync::Arc::new(JsonRegistryProvider {
            canonical_json: sensor_json.clone(),
        }))
        .unwrap();
    manager
        .set_static_sensor_catalog_json(r#"{"sensors":[]}"#.into())
        .unwrap();
    manager
        .set_static_resource_catalog_json(r#"{"resources":[]}"#.into())
        .unwrap();
    manager
        .set_static_registry_entries_json(registry_entries_json(
            "sensor",
            "provider-camera",
            &sensor_hash,
            &sensor_json,
        ))
        .unwrap();
    assert!(matches!(
        manager
            .set_static_registry_entries_json(
                r#"{"entries":[{"kind":"sensor","id":"bad","hash":"bad","canonical_json":"{}"}]}"#
                    .into()
            )
            .unwrap_err(),
        auki_domain::BindingDomainError::InvalidJson { .. }
    ));
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_catalog_and_registry_fetches_are_exposed() {
    // binding-surface: native catalog and registry fetches
    let (_servers, consumer, producer) =
        bootstrap_connected_binding_manager_pair("binding-fetch", 99, 100).await;
    let (sensor_json, sensor_hash) = sensor_registry_json("fetch-camera");
    producer
        .set_static_sensor_catalog_json(
            serde_json::json!({
                "sensors": [{
                    "sensor_id": "fetch-camera",
                    "sensor_hash": sensor_hash,
                    "kind": "camera"
                }]
            })
            .to_string(),
        )
        .unwrap();
    producer
        .set_static_resource_catalog_json(r#"{"resources":[]}"#.into())
        .unwrap();
    producer
        .set_static_registry_entries_json(registry_entries_json(
            "sensor",
            "fetch-camera",
            &sensor_hash,
            &sensor_json,
        ))
        .unwrap();

    let sensors = wait_for_binding_json(|| {
        consumer.fetch_sensor_catalog_json(producer.local_peer_id(), 5_000)
    })
    .await;
    let sensors: serde_json::Value = serde_json::from_str(&sensors).unwrap();
    assert_eq!(sensors["sensors"][0]["sensor_id"], "fetch-camera");

    let resources = wait_for_binding_json(|| {
        consumer.fetch_resource_catalog_json(producer.local_peer_id(), 5_000)
    })
    .await;
    let resources: serde_json::Value = serde_json::from_str(&resources).unwrap();
    let resources = resources["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0]["kind"], "sensor_stream");
    assert_eq!(resources[0]["sensor_id"], "fetch-camera");

    let fetched_entry = wait_for_binding_json(|| {
        consumer.fetch_registry_entry_json(
            producer.local_peer_id(),
            serde_json::json!({
                "kind": "sensor",
                "id": "fetch-camera",
                "hash": sensor_hash,
            })
            .to_string(),
            5_000,
        )
    })
    .await;
    let fetched_entry: serde_json::Value = serde_json::from_str(&fetched_entry).unwrap();
    assert_eq!(fetched_entry["sensor_id"], "fetch-camera");
    consumer.shutdown().await.unwrap();
    producer.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_byte_streams_are_exposed() {
    // binding-surface: native byte streams
    let (_servers, consumer, producer) =
        bootstrap_connected_binding_manager_pair("binding-stream", 101, 102).await;
    let producer_peer = producer.local_peer_id();
    let camera_bytes = auki_proto::camera::CameraFrame {
        dynamic_intrinsics: None,
        frame: vec![1, 2, 3, 4],
    }
    .encode_to_vec();

    let open = tokio::spawn({
        let consumer = consumer.clone();
        async move {
            consumer
                .open_stream_bytes(
                    producer_peer,
                    r#"{"sensor_id":"camera-1"}"#.into(),
                    "camera".into(),
                    5_000,
                )
                .await
        }
    });

    let event = wait_for_stream_open(&producer).await;
    assert_eq!(event.payload_json, r#"{"sensor_id":"camera-1"}"#);
    let stream_id = producer
        .accept_stream_open(event.responder_id.unwrap(), camera_manifest_json())
        .unwrap();
    producer
        .push_stream_entry(
            stream_id,
            auki_domain::DomainStreamEntry {
                sequence: 0,
                timestamp_ns: 123,
                payload_kind: "camera".into(),
                payload: camera_bytes.clone(),
            },
        )
        .unwrap();
    producer.finish_stream(stream_id).unwrap();

    let subscription = open.await.unwrap().unwrap();
    assert_eq!(subscription.manifest_json(), camera_manifest_json());
    let entry = subscription.next_entry(5_000).unwrap().unwrap();
    assert_eq!(entry.sequence, 0);
    assert_eq!(entry.timestamp_ns, 123);
    assert_eq!(entry.payload_kind, "camera");
    assert_eq!(entry.payload, camera_bytes);
    assert!(subscription.next_entry(5_000).unwrap().is_none());
    consumer.shutdown().await.unwrap();
    producer.shutdown().await.unwrap();
}

#[test]
fn browser_membership_validation_helpers_are_exposed() {
    // binding-surface: browser membership validation helpers
    let membership = cluster_membership_fixture();
    let validated: serde_json::Value =
        serde_json::from_str(&auki_domain::validate_membership_json(&membership).unwrap()).unwrap();
    assert_eq!(validated["cluster_name"], "browser-fixture");
    assert_eq!(validated["peers"].as_array().unwrap().len(), 2);
    assert!(auki_domain::validate_membership_json("{}").is_err());
}

#[test]
fn browser_manager_election_helpers_are_exposed() {
    // binding-surface: browser manager election helpers
    let membership = cluster_membership_fixture();
    let peer = binding_peer_id(111);
    assert_eq!(
        auki_domain::domain_successor_json(&membership, &peer).unwrap(),
        serde_json::json!(peer).to_string()
    );
}

#[test]
fn browser_domain_dto_validation_helpers_are_exposed() {
    // binding-surface: browser domain dto validation helpers
    let peer = binding_peer_id(112);
    let participant = serde_json::json!({
        "app": "browser-test",
        "name": "peer-112",
        "session_id": "session-112",
        "session_clock_id": "clock-112",
        "session_clock_hash": "clock-hash-112",
        "session_now_ns": 123,
        "cluster_joined_at_ns": null,
        "peer_id": peer,
        "app_instance": "00163eabcdef",
        "is_manager": true,
        "manager_peer_id": peer,
    })
    .to_string();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &auki_domain::validate_participant_info_json(&participant).unwrap()
        )
        .unwrap()["peer_id"],
        peer
    );

    let sensors = serde_json::json!({
        "sensors": [{
            "sensor_id": "browser-camera",
            "sensor_hash": "sensor-hash",
            "kind": "camera"
        }]
    })
    .to_string();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &auki_domain::validate_sensor_catalog_json(&sensors).unwrap()
        )
        .unwrap()["sensors"][0]["kind"],
        "camera"
    );

    let resources = serde_json::json!({
        "resources": [{
            "kind": "sensor_stream",
            "id": "browser-camera",
            "sensor_id": "browser-camera",
            "sensor_hash": "sensor-hash",
            "sensor_kind": "camera",
            "stream_protocol": "/auki/stream/0.1.0",
            "payload": "camera_frame"
        }]
    })
    .to_string();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &auki_domain::validate_resource_catalog_json(&resources).unwrap()
        )
        .unwrap()["resources"][0]["payload"],
        "camera_frame"
    );

    let registry = registry_entries_json(
        "sensor",
        "browser-camera",
        "sensor-hash",
        r#"{"sensor_id":"browser-camera","body":{"type":"camera"}}"#,
    );
    let registry: serde_json::Value = serde_json::from_str(&registry).unwrap();
    let entry = registry["entries"][0].clone();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &auki_domain::validate_registry_entry_json(&entry.to_string()).unwrap()
        )
        .unwrap()["id"],
        "browser-camera"
    );
}

#[test]
fn browser_javascript_domain_client_facade_over_auki_network_browser_transport_is_exposed() {
    // binding-surface: browser javascript domain client facade over auki-network browser transport
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let index = std::fs::read_to_string(crate_dir.join("bindings/javascript/index.js.tmpl"))
        .expect("domain browser facade template exists");
    assert!(index.contains("export class AukiDomainClient"));
    assert!(index.contains("fetchParticipantInfo"));
    assert!(index.contains("/auki/registries/0.0.1"));
    let test_dir = crate_dir.join("bindings/javascript/test");
    assert!(test_dir.join("domain-helpers.test.mjs.tmpl").exists());
    assert!(
        test_dir
            .join("domain-client-request-response.test.mjs.tmpl")
            .exists()
    );
}

fn cluster_membership_fixture() -> String {
    let peer_a = binding_peer_id(111);
    let peer_b = binding_peer_id(112);
    let membership = auki_domain::cluster_membership_new_json("browser-fixture");
    let membership = auki_domain::cluster_membership_admit_member_json(
        &membership,
        &serde_json::json!({
            "peer_id": peer_a,
            "multiaddrs": ["/ip4/127.0.0.1/tcp/40111"],
            "join_ts_ns": 10
        })
        .to_string(),
    )
    .unwrap();
    auki_domain::cluster_membership_admit_member_json(
        &membership,
        &serde_json::json!({
            "peer_id": peer_b,
            "multiaddrs": ["/ip4/127.0.0.1/tcp/40112"],
            "join_ts_ns": 20
        })
        .to_string(),
    )
    .unwrap()
}

async fn bootstrap_binding_manager(
    cluster_name: &str,
    seed_byte: u8,
) -> (
    MockDiscoveryServer,
    std::sync::Arc<auki_domain::DomainClusterManager>,
) {
    let manager_peer_id = binding_peer_id(seed_byte);
    let server = MockDiscoveryServer::spawn(cluster_name.to_string(), manager_peer_id);
    let local_multiaddr = reserve_local_multiaddr();
    let manager = auki_domain::bootstrap_domain_cluster_manager(
        auki_domain::ClusterTargetMode::Create,
        cluster_name.to_string(),
        vec![seed_byte; 32],
        vec![local_multiaddr.clone()],
        vec![local_multiaddr],
        server.base_url(),
        auki_domain::DaemonInfo {
            app: "binding-test".into(),
            name: format!("peer-{seed_byte}"),
            session_id: format!("session-{seed_byte}"),
            session_clock_id: "legacy-clock".into(),
            session_clock_hash: "legacy-clock-hash".into(),
            app_instance: "00163eabcdef".into(),
        },
        "binding-domain-test/0.1".into(),
    )
    .await
    .unwrap();
    (server, manager)
}

async fn bootstrap_connected_binding_manager_pair(
    cluster_prefix: &str,
    seed_a: u8,
    seed_b: u8,
) -> (
    Vec<MockDiscoveryServer>,
    std::sync::Arc<auki_domain::DomainClusterManager>,
    std::sync::Arc<auki_domain::DomainClusterManager>,
) {
    let (server_a, a) = bootstrap_binding_manager(&format!("{cluster_prefix}-a"), seed_a).await;
    let (server_b, b) = bootstrap_binding_manager(&format!("{cluster_prefix}-b"), seed_b).await;
    a.admit_peer(b.local_peer_id(), b.local_multiaddrs())
        .await
        .unwrap();
    b.admit_peer(a.local_peer_id(), a.local_multiaddrs())
        .await
        .unwrap();
    let _ = wait_for_binding_json(|| a.fetch_participant_info_json(b.local_peer_id())).await;
    let _ = wait_for_binding_json(|| b.fetch_participant_info_json(a.local_peer_id())).await;
    (vec![server_a, server_b], a, b)
}

async fn wait_for_binding_json<F, Fut>(mut operation: F) -> String
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<String, auki_domain::BindingDomainError>>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match operation().await {
            Ok(value) => return value,
            Err(err) => {
                let last_error = format!("{err:?}");
                if std::time::Instant::now() >= deadline {
                    panic!("binding operation did not succeed: {last_error}");
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

async fn wait_for_stream_open(
    manager: &auki_domain::DomainClusterManager,
) -> auki_domain::DomainRuntimeEvent {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(event) = manager.drain_stream_open_requests(10).into_iter().next() {
            return event;
        }
        if std::time::Instant::now() >= deadline {
            panic!("stream open request was not observed");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

struct JsonSensorProvider {
    json: String,
}

impl auki_domain::BindingSensorCatalogProvider for JsonSensorProvider {
    fn snapshot_json(&self) -> Result<String, auki_domain::BindingDomainError> {
        Ok(self.json.clone())
    }
}

struct JsonResourceProvider {
    json: String,
}

impl auki_domain::BindingResourceCatalogProvider for JsonResourceProvider {
    fn snapshot_json(&self) -> Result<String, auki_domain::BindingDomainError> {
        Ok(self.json.clone())
    }
}

struct JsonRegistryProvider {
    canonical_json: String,
}

impl auki_domain::BindingRegistryEntryProvider for JsonRegistryProvider {
    fn entry_json(&self, _path: String) -> Result<Option<String>, auki_domain::BindingDomainError> {
        Ok(Some(self.canonical_json.clone()))
    }
}

fn sensor_registry_json(sensor_id: &str) -> (String, String) {
    let entry = auki_registry::SensorRegistryEntry {
        sensor_id: sensor_id.into(),
        body: auki_registry::SensorBody::Camera(auki_registry::Camera {
            width: 640,
            height: 480,
            frame_rate_hz: 30,
            pixel_format: "rgb8".into(),
            color_space: "srgb".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "none".into(),
            frame_id: "camera-frame".into(),
            frame_hash: "frame-hash".into(),
        }),
    };
    (
        String::from_utf8(entry.canonical_bytes()).unwrap(),
        entry.hash(),
    )
}

fn registry_entries_json(kind: &str, id: &str, hash: &str, canonical_json: &str) -> String {
    serde_json::json!({
        "entries": [{
            "kind": kind,
            "id": id,
            "hash": hash,
            "canonical_json": canonical_json,
        }]
    })
    .to_string()
}

fn camera_manifest_json() -> String {
    r#"{"sensor_id":"camera-1","sensor_hash":"camera-hash","clock_id":"clock-1","clock_hash":"clock-hash","frame_id":"frame-1","frame_hash":"frame-hash"}"#.into()
}

fn reserve_local_multiaddr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("/ip4/127.0.0.1/tcp/{port}")
}

fn binding_peer_id(seed_byte: u8) -> String {
    let seed = [seed_byte; 32];
    let wallet = auki_identity::Wallet::from_seed(&seed);
    auki_network::PeerIdentity::from_wallet(&wallet)
        .peer_id()
        .to_string()
}

struct MockDiscoveryServer {
    addr: std::net::SocketAddr,
    _handle: std::thread::JoinHandle<()>,
}

impl MockDiscoveryServer {
    fn spawn(cluster_name: String, manager_peer_id: String) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for _ in 0..32 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let request = read_http_request(&mut stream);
                let path = request_path(&request);
                let entry = discovery_entry_body(&cluster_name, &manager_peer_id);
                let (status, body) = if path == format!("/clusters/{cluster_name}") {
                    if request.starts_with("POST ") {
                        ("201 Created", entry)
                    } else if request.starts_with("DELETE ") {
                        ("204 No Content", String::new())
                    } else {
                        ("404 Not Found", r#"{"error":"unexpected method"}"#.into())
                    }
                } else if path == format!("/clusters/{cluster_name}/liveness") {
                    ("200 OK", entry)
                } else if path == "/clusters" {
                    ("200 OK", format!(r#"{{"clusters":[{entry}]}}"#))
                } else {
                    ("404 Not Found", r#"{"error":"unexpected path"}"#.into())
                };
                write_http_response(&mut stream, status, &body);
            }
        });
        Self {
            addr,
            _handle: handle,
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read as _;

    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(bytes).unwrap()
}

fn request_path(request: &str) -> String {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
}

fn write_http_response(stream: &mut std::net::TcpStream, status: &str, body: &str) {
    use std::io::Write as _;

    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn discovery_entry_body(cluster_name: &str, manager_peer_id: &str) -> String {
    serde_json::json!({
        "name": cluster_name,
        "manager_peer_id": manager_peer_id,
        "manager_multiaddrs": ["/ip4/127.0.0.1/tcp/48000"],
        "peer_count": 1,
        "created_ns": 1,
        "last_liveness_check_ns": 1
    })
    .to_string()
}
use prost::Message as _;
