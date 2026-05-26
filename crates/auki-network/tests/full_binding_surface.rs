#[cfg(feature = "swarm")]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(feature = "swarm")]
static NEXT_RUNTIME_SEED: AtomicU8 = AtomicU8::new(17);

#[test]
fn native_peer_identity_and_derivation_is_exposed() {
    // binding-surface: native peer identity and derivation
    let seed = vec![3u8; 32];
    let identity = auki_network::BindingPeerIdentity::from_wallet_seed(seed.clone()).unwrap();
    assert_eq!(auki_network::peer_derivation_label(), "peer/v1");
    assert_eq!(
        identity.peer_id(),
        "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
    );
    assert_eq!(
        auki_network::peer_id_from_wallet_seed(seed).unwrap(),
        identity.peer_id()
    );
    assert!(!identity.public_key_protobuf().is_empty());
    match auki_network::BindingPeerIdentity::from_seed(vec![1, 2, 3]) {
        Err(auki_network::NetworkError::InvalidSeedLength { .. }) => {}
        Err(other) => panic!("unexpected peer identity error: {other:?}"),
        Ok(_) => panic!("short peer seed unexpectedly succeeded"),
    }
}

#[test]
#[cfg(feature = "swarm")]
fn native_runtime_lifecycle_is_exposed() {
    // binding-surface: native runtime lifecycle
    let runtime = spawn_binding_runtime(vec![]);
    assert!(!runtime.local_peer_id().is_empty());
    assert!(runtime.connected_peers().is_empty());
    assert!(
        wait_for_listen_multiaddr(&runtime)
            .iter()
            .any(|addr| addr.contains("/tcp/") && !addr.ends_with("/tcp/0"))
    );
    runtime.shutdown().unwrap();
}

#[test]
#[cfg(not(feature = "swarm"))]
#[ignore = "requires swarm feature"]
fn native_runtime_lifecycle_is_exposed() {
    // binding-surface: native runtime lifecycle
}

#[test]
#[cfg(feature = "swarm")]
fn native_runtime_control_is_exposed() {
    // binding-surface: native runtime control
    let runtime = spawn_binding_runtime(vec![]);
    assert!(runtime.connected_peers().is_empty());
    runtime.set_heartbeat_targets(vec![]).unwrap();
    runtime.shutdown().unwrap();
}

#[test]
#[cfg(not(feature = "swarm"))]
#[ignore = "requires swarm feature"]
fn native_runtime_control_is_exposed() {
    // binding-surface: native runtime control
}

#[test]
#[cfg(feature = "swarm")]
fn native_allowed_peer_updates_are_exposed() {
    let runtime = spawn_binding_runtime(vec![]);
    let peer = auki_network::PeerIdentity::from_seed(&[41u8; 32])
        .peer_id()
        .to_string();

    let report = runtime
        .set_allowed_peers(vec![auki_network::BindingAllowedPeer {
            peer_id: peer.clone(),
            multiaddrs: vec!["/ip4/127.0.0.1/tcp/49001".into()],
        }])
        .unwrap();

    assert_eq!(report.accepted, vec![peer]);
    assert_eq!(report.rejected_json, "[]");
    runtime.shutdown().unwrap();
}

#[test]
#[cfg(feature = "swarm")]
fn native_heartbeat_targets_are_exposed() {
    let runtime = spawn_binding_runtime(vec![]);
    let peer = auki_network::PeerIdentity::from_seed(&[42u8; 32])
        .peer_id()
        .to_string();

    runtime.set_heartbeat_targets(vec![peer]).unwrap();
    runtime.shutdown().unwrap();
}

#[test]
#[cfg(feature = "swarm")]
fn native_event_draining_is_exposed() {
    // binding-surface: native event draining
    let runtime = spawn_binding_runtime(vec![]);

    assert!(runtime.drain_runtime_events(10).is_empty());
    assert!(runtime.drain_membership_events(10).is_empty());
    assert!(runtime.drain_liveness_events(10).is_empty());
    assert!(runtime.drain_diagnostic_events(10).is_empty());
    assert!(runtime.drain_join_requests(10).is_empty());
    assert!(runtime.drain_participant_info_requests(10).is_empty());
    assert!(runtime.drain_sensor_catalog_requests(10).is_empty());
    assert!(runtime.drain_resource_catalog_requests(10).is_empty());
    assert!(runtime.drain_registry_entry_requests(10).is_empty());

    runtime.shutdown().unwrap();
}

#[test]
#[cfg(not(feature = "swarm"))]
#[ignore = "requires swarm feature"]
fn native_event_draining_is_exposed() {
    // binding-surface: native event draining
}

#[test]
#[cfg(feature = "swarm")]
fn native_request_response_protocols_are_exposed() {
    // binding-surface: native request/response protocols
    let runtime = spawn_binding_runtime(vec![]);

    assert!(matches!(
        runtime
            .send_join_request_json("not-a-peer".into(), "{}".into(), 1)
            .unwrap_err(),
        auki_network::BindingNetworkError::InvalidPeerId { .. }
    ));
    assert!(matches!(
        runtime
            .request_participant_info_json("not-a-peer".into(), "{}".into(), 1)
            .unwrap_err(),
        auki_network::BindingNetworkError::InvalidPeerId { .. }
    ));
    assert!(matches!(
        runtime
            .request_sensor_catalog_json("not-a-peer".into(), "{}".into(), 1)
            .unwrap_err(),
        auki_network::BindingNetworkError::InvalidPeerId { .. }
    ));
    assert!(matches!(
        runtime
            .request_resource_catalog_json("not-a-peer".into(), "{}".into(), 1)
            .unwrap_err(),
        auki_network::BindingNetworkError::InvalidPeerId { .. }
    ));
    assert!(matches!(
        runtime
            .request_registry_entry_json("not-a-peer".into(), "{}".into(), 1)
            .unwrap_err(),
        auki_network::BindingNetworkError::InvalidPeerId { .. }
    ));

    assert!(matches!(
        runtime
            .respond_join_json(999, r#"{"kind":"reject","reason":"closed"}"#.into())
            .unwrap_err(),
        auki_network::BindingNetworkError::Closed
    ));
    assert!(matches!(
        runtime
            .respond_participant_info_json(999, r#"{"participant_info_json":"{}"}"#.into())
            .unwrap_err(),
        auki_network::BindingNetworkError::Closed
    ));
    assert!(matches!(
        runtime
            .respond_sensor_catalog_json(999, r#"{"sensors":[]}"#.into())
            .unwrap_err(),
        auki_network::BindingNetworkError::Closed
    ));
    assert!(matches!(
        runtime
            .respond_resource_catalog_json(999, r#"{"resources":[]}"#.into())
            .unwrap_err(),
        auki_network::BindingNetworkError::Closed
    ));
    assert!(matches!(
        runtime
            .respond_registry_entry_json(999, r#"{"entry":null}"#.into())
            .unwrap_err(),
        auki_network::BindingNetworkError::Closed
    ));

    runtime.shutdown().unwrap();
}

#[test]
#[cfg(not(feature = "swarm"))]
#[ignore = "requires swarm feature"]
fn native_request_response_protocols_are_exposed() {
    // binding-surface: native request/response protocols
}

#[test]
#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn native_discovery_client_is_exposed() {
    // binding-surface: native discovery client
    let server = MockDiscoveryServer::spawn();
    let client = auki_network::discovery_client(server.base_url()).unwrap();
    let manager_peer_id = auki_network::PeerIdentity::from_seed(&[53u8; 32])
        .peer_id()
        .to_string();

    let created = client
        .register_peer_json(
            serde_json::json!({
                "name": "demo",
                "manager_peer_id": manager_peer_id,
                "manager_multiaddrs": ["/ip4/127.0.0.1/tcp/4001"],
                "relay_multiaddrs": ["/ip4/127.0.0.1/tcp/4002"]
            })
            .to_string(),
            2_000,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&created).unwrap()["kind"],
        "created"
    );

    let discovered = client.discover_peers_json("{}".into(), 2_000).unwrap();
    let discovered: serde_json::Value = serde_json::from_str(&discovered).unwrap();
    assert_eq!(discovered["clusters"][0]["name"], "demo");
    assert_eq!(
        discovered["clusters"][0]["relay_multiaddrs"][0],
        "/ip4/127.0.0.1/tcp/4002"
    );

    let nodes = client
        .discover_nodes_json(serde_json::json!({ "type": "relay" }).to_string(), 2_000)
        .unwrap();
    let nodes: serde_json::Value = serde_json::from_str(&nodes).unwrap();
    assert_eq!(nodes["nodes"][0]["node_type"], "relay");
    assert_eq!(
        nodes["nodes"][0]["multiaddrs"][0],
        "/ip4/127.0.0.1/tcp/4002"
    );

    client.unregister_peer_json("demo".into(), 2_000).unwrap();
}

#[test]
#[cfg(not(all(feature = "discovery_client", feature = "swarm")))]
#[ignore = "requires discovery_client and swarm features"]
fn native_discovery_client_is_exposed() {
    // binding-surface: native discovery client
}

#[test]
#[cfg(all(feature = "app_instance", feature = "swarm"))]
fn native_app_instance_derivation_is_exposed() {
    // binding-surface: native app-instance derivation
    let peer_id = auki_network::PeerIdentity::from_seed(&[54u8; 32])
        .peer_id()
        .to_string();
    let json = serde_json::json!({
        "app_id": "binding-test",
        "app_instance": "00163eabcdef",
        "peer_id": peer_id,
        "peer_derivation_label": "peer/v1"
    })
    .to_string();
    assert_eq!(auki_network::app_instance_peer_id(json).unwrap(), peer_id);
    assert!(matches!(
        auki_network::derive_app_instance_json(vec![1, 2, 3], "binding-test".into()).unwrap_err(),
        auki_network::BindingNetworkError::InvalidSeedLength { .. }
    ));
    match auki_network::derive_app_instance_json(vec![54u8; 32], "binding-test".into()) {
        Ok(json) => {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["app_id"], "binding-test");
            assert_eq!(value["peer_derivation_label"], "peer/v1");
            assert!(
                value["peer_id"]
                    .as_str()
                    .is_some_and(|peer| !peer.is_empty())
            );
        }
        Err(auki_network::BindingNetworkError::Runtime { .. }) => {}
        Err(other) => panic!("unexpected app-instance derivation error: {other:?}"),
    }
}

#[test]
#[cfg(not(all(feature = "app_instance", feature = "swarm")))]
#[ignore = "requires app_instance and swarm features"]
fn native_app_instance_derivation_is_exposed() {
    // binding-surface: native app-instance derivation
}

#[test]
#[cfg(feature = "swarm")]
fn native_byte_streams_are_exposed() {
    // binding-surface: native byte streams
    let runtime = spawn_binding_runtime(vec![]);
    let result = runtime.open_stream_bytes(auki_network::BindingStreamRequest {
        peer_id: "not-a-peer".into(),
        request_json: r#"{"sensor_id":"camera-1"}"#.into(),
        payload_kind: "camera".into(),
        timeout_ms: 1,
    });
    match result {
        Err(auki_network::BindingNetworkError::InvalidPeerId { .. }) => {}
        Err(other) => panic!("expected invalid peer id, got {other:?}"),
        Ok(_) => panic!("invalid peer id unexpectedly opened a stream"),
    }
    runtime.shutdown().unwrap();
}

#[test]
#[cfg(not(feature = "swarm"))]
#[ignore = "requires swarm feature"]
fn native_byte_streams_are_exposed() {
    // binding-surface: native byte streams
}

#[test]
#[cfg(feature = "swarm")]
fn native_diagnostics_are_exposed() {
    // binding-surface: native diagnostics
    let pair = BindingRuntimePair::spawn();

    pair.a
        .broadcast_diagnostic_message_json(
            r#"{"topic":"binding.smoke","payload_json":"{\"ok\":true}"}"#.into(),
        )
        .unwrap();

    let event = wait_for_binding_event(|| pair.b.drain_diagnostic_events(10), "diagnostic");
    assert_eq!(event.peer_id.as_deref(), Some(pair.a_peer_id.as_str()));
    let payload: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
    assert_eq!(payload["topic"], "binding.smoke");
    assert_eq!(payload["payload_json"], r#"{"ok":true}"#);
}

#[test]
#[cfg(not(feature = "swarm"))]
#[ignore = "requires swarm feature"]
fn native_diagnostics_are_exposed() {
    // binding-surface: native diagnostics
}

#[test]
#[cfg(feature = "swarm")]
fn native_join_request_response_is_exposed() {
    let pair = BindingRuntimePair::spawn();
    let requester = pair.a.clone();
    let responder = pair.b.clone();
    let responder_peer = pair.b_peer_id.clone();
    let requester_peer = pair.a_peer_id.clone();

    let request = std::thread::spawn(move || {
        requester.send_join_request_json(responder_peer, r#"{"multiaddrs":[]}"#.into(), 5_000)
    });

    let event = wait_for_binding_event(|| responder.drain_join_requests(10), "join_request");
    assert_eq!(event.peer_id.as_deref(), Some(requester_peer.as_str()));
    assert_eq!(event.payload_json, r#"{"multiaddrs":[]}"#);
    responder
        .respond_join_json(
            event.responder_id.expect("join responder id"),
            r#"{"kind":"reject","reason":"binding-smoke"}"#.into(),
        )
        .unwrap();

    let response = request.join().unwrap().unwrap();
    let payload: serde_json::Value = serde_json::from_str(&response.payload_json).unwrap();
    assert_eq!(payload["kind"], "reject");
    assert_eq!(payload["reason"], "binding-smoke");
}

#[test]
#[cfg(feature = "swarm")]
fn native_participant_info_request_response_is_exposed() {
    let pair = BindingRuntimePair::spawn();
    let requester = pair.a.clone();
    let responder = pair.b.clone();
    let responder_peer = pair.b_peer_id.clone();
    let requester_peer = pair.a_peer_id.clone();

    let request = std::thread::spawn(move || {
        requester.request_participant_info_json(responder_peer, "{}".into(), 5_000)
    });

    let event = wait_for_binding_event(
        || responder.drain_participant_info_requests(10),
        "participant_info_request",
    );
    assert_eq!(event.peer_id.as_deref(), Some(requester_peer.as_str()));
    responder
        .respond_participant_info_json(
            event.responder_id.expect("participant-info responder id"),
            r#"{"participant_info_json":"{\"peer_id\":\"binding-peer\"}"}"#.into(),
        )
        .unwrap();

    let response = request.join().unwrap().unwrap();
    let payload: serde_json::Value = serde_json::from_str(&response.payload_json).unwrap();
    assert_eq!(
        payload["participant_info_json"],
        r#"{"peer_id":"binding-peer"}"#
    );
}

#[test]
#[cfg(feature = "swarm")]
fn native_catalog_request_response_is_exposed() {
    let pair = BindingRuntimePair::spawn();

    let sensors_requester = pair.a.clone();
    let sensors_responder = pair.b.clone();
    let sensors_peer = pair.b_peer_id.clone();
    let sensors = std::thread::spawn(move || {
        sensors_requester.request_sensor_catalog_json(sensors_peer, "{}".into(), 5_000)
    });
    let sensor_event = wait_for_binding_event(
        || sensors_responder.drain_sensor_catalog_requests(10),
        "sensor_catalog_request",
    );
    sensors_responder
        .respond_sensor_catalog_json(
            sensor_event.responder_id.expect("sensor responder id"),
            r#"{"sensors":[]}"#.into(),
        )
        .unwrap();
    let sensor_response = sensors.join().unwrap().unwrap();
    let sensor_payload: serde_json::Value =
        serde_json::from_str(&sensor_response.payload_json).unwrap();
    assert_eq!(sensor_payload["sensors"].as_array().unwrap().len(), 0);

    let resources_requester = pair.a.clone();
    let resources_responder = pair.b.clone();
    let resources_peer = pair.b_peer_id.clone();
    let resources = std::thread::spawn(move || {
        resources_requester.request_resource_catalog_json(resources_peer, "{}".into(), 5_000)
    });
    let resource_event = wait_for_binding_event(
        || resources_responder.drain_resource_catalog_requests(10),
        "resource_catalog_request",
    );
    resources_responder
        .respond_resource_catalog_json(
            resource_event.responder_id.expect("resource responder id"),
            r#"{"resources":[]}"#.into(),
        )
        .unwrap();
    let resource_response = resources.join().unwrap().unwrap();
    let resource_payload: serde_json::Value =
        serde_json::from_str(&resource_response.payload_json).unwrap();
    assert_eq!(resource_payload["resources"].as_array().unwrap().len(), 0);
}

#[test]
#[cfg(feature = "swarm")]
fn native_registry_request_response_is_exposed() {
    let pair = BindingRuntimePair::spawn();
    let requester = pair.a.clone();
    let responder = pair.b.clone();
    let responder_peer = pair.b_peer_id.clone();

    let request = std::thread::spawn(move || {
        requester.request_registry_entry_json(
            responder_peer,
            r#"{"kind":"sensor","id":"sensor-1","hash":"hash-1"}"#.into(),
            5_000,
        )
    });

    let event = wait_for_binding_event(
        || responder.drain_registry_entry_requests(10),
        "registry_entry_request",
    );
    let payload: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
    assert_eq!(payload["kind"], "sensor");
    assert_eq!(payload["id"], "sensor-1");
    responder
        .respond_registry_entry_json(
            event.responder_id.expect("registry responder id"),
            r#"{"entry":null}"#.into(),
        )
        .unwrap();

    let response = request.join().unwrap().unwrap();
    let response_payload: serde_json::Value = serde_json::from_str(&response.payload_json).unwrap();
    assert!(response_payload["entry"].is_null());
}

#[test]
#[cfg(feature = "swarm")]
fn native_camera_stream_bytes_are_exposed() {
    use prost::Message as _;

    let pair = BindingRuntimePair::spawn();
    let consumer = pair.a.clone();
    let producer = pair.b.clone();
    let producer_peer = pair.b_peer_id.clone();
    let camera_bytes = auki_proto::camera::CameraFrame {
        dynamic_intrinsics: None,
        frame: vec![1, 2, 3, 4],
    }
    .encode_to_vec();

    let open = std::thread::spawn(move || {
        consumer.open_stream_bytes(auki_network::BindingStreamRequest {
            peer_id: producer_peer,
            request_json: r#"{"sensor_id":"camera-1"}"#.into(),
            payload_kind: "camera".into(),
            timeout_ms: 5_000,
        })
    });

    let event = wait_for_binding_event(
        || producer.drain_stream_open_requests(10),
        "stream_open_request",
    );
    assert_eq!(event.payload_json, r#"{"sensor_id":"camera-1"}"#);
    let stream_id = producer
        .accept_stream_open(event.responder_id.unwrap(), camera_manifest_json())
        .unwrap();
    producer
        .push_stream_entry(
            stream_id,
            auki_network::BindingStreamEntry {
                sequence: 0,
                timestamp_ns: 123,
                payload_kind: "camera".into(),
                payload: camera_bytes.clone(),
            },
        )
        .unwrap();
    producer.finish_stream(stream_id).unwrap();

    let subscription = open.join().unwrap().unwrap();
    assert_eq!(subscription.manifest_json(), camera_manifest_json());
    let entry = subscription.next_entry(5_000).unwrap().unwrap();
    assert_eq!(entry.sequence, 0);
    assert_eq!(entry.timestamp_ns, 123);
    assert_eq!(entry.payload_kind, "camera");
    assert_eq!(entry.payload, camera_bytes);
    assert!(subscription.next_entry(5_000).unwrap().is_none());
}

#[test]
#[cfg(feature = "swarm")]
fn native_detection_stream_bytes_are_exposed() {
    use prost::Message as _;

    let pair = BindingRuntimePair::spawn();
    let consumer = pair.a.clone();
    let producer = pair.b.clone();
    let producer_peer = pair.b_peer_id.clone();
    let detection_bytes = auki_proto::detection::DetectionFrame {
        data: vec![9, 8, 7],
        sensor_hash: "camera-hash".into(),
        r#type: "boxes/v1".into(),
    }
    .encode_to_vec();

    let open = std::thread::spawn(move || {
        consumer.open_stream_bytes(auki_network::BindingStreamRequest {
            peer_id: producer_peer,
            request_json: r#"{"sensor_id":"detector-1"}"#.into(),
            payload_kind: "detection".into(),
            timeout_ms: 5_000,
        })
    });

    let event = wait_for_binding_event(
        || producer.drain_stream_open_requests(10),
        "stream_open_request",
    );
    let stream_id = producer
        .accept_stream_open(event.responder_id.unwrap(), detection_manifest_json())
        .unwrap();
    producer
        .push_stream_entry(
            stream_id,
            auki_network::BindingStreamEntry {
                sequence: 0,
                timestamp_ns: 456,
                payload_kind: "detection".into(),
                payload: detection_bytes.clone(),
            },
        )
        .unwrap();
    producer.finish_stream(stream_id).unwrap();

    let subscription = open.join().unwrap().unwrap();
    assert_eq!(subscription.manifest_json(), detection_manifest_json());
    let entry = subscription.next_entry(5_000).unwrap().unwrap();
    assert_eq!(entry.sequence, 0);
    assert_eq!(entry.timestamp_ns, 456);
    assert_eq!(entry.payload_kind, "detection");
    assert_eq!(entry.payload, detection_bytes);
    assert!(subscription.next_entry(5_000).unwrap().is_none());
}

#[test]
#[cfg(feature = "swarm")]
fn native_stream_decline_is_exposed() {
    let pair = BindingRuntimePair::spawn();
    let consumer = pair.a.clone();
    let producer = pair.b.clone();
    let producer_peer = pair.b_peer_id.clone();

    let open = std::thread::spawn(move || {
        consumer.open_stream_bytes(auki_network::BindingStreamRequest {
            peer_id: producer_peer,
            request_json: r#"{"sensor_id":"missing"}"#.into(),
            payload_kind: "camera".into(),
            timeout_ms: 5_000,
        })
    });

    let event = wait_for_binding_event(
        || producer.drain_stream_open_requests(10),
        "stream_open_request",
    );
    producer
        .decline_stream_open(event.responder_id.unwrap(), "sensor_not_found".into())
        .unwrap();

    let err = match open.join().unwrap() {
        Err(err) => err,
        Ok(_) => panic!("declined stream unexpectedly opened"),
    };
    assert!(matches!(
        err,
        auki_network::BindingNetworkError::Runtime { .. }
    ));
}

#[test]
fn browser_peer_identity_and_derivation_is_exposed() {
    // binding-surface: browser peer identity and derivation
    let wasm = include_str!("../src/wasm.rs");
    assert!(wasm.contains("peerIdFromWalletSeed"));
    assert!(wasm.contains("peerPrivateKeyProtobufFromWalletSeed"));
}

#[test]
fn browser_protocol_constants_are_exposed() {
    // binding-surface: browser protocol constants
    let wasm = include_str!("../src/wasm.rs");
    assert!(wasm.contains("aukiNetworkProtocolsJson"));
    assert!(wasm.contains("joinProtocol"));
    assert!(wasm.contains("resourcesProtocol"));
}

#[test]
fn browser_probe_is_exposed() {
    // binding-surface: browser browser probe
    let js = include_str!("../bindings/javascript/index.js.tmpl");
    assert!(js.contains("dialBrowserProbe"));
    assert!(js.contains("browserProbeProtocol"));
}

#[test]
fn browser_message_protocol_is_exposed() {
    // binding-surface: browser message protocol
    let js = include_str!("../bindings/javascript/index.js.tmpl");
    let wasm = include_str!("../src/wasm.rs");
    assert!(js.contains("sendMessageEnvelope"));
    assert!(wasm.contains("encodeMessageEnvelopeBytes"));
    assert!(wasm.contains("decodeMessageEnvelopeJson"));
}

#[test]
fn browser_request_response_dto_encoding_helpers_are_exposed() {
    // binding-surface: browser request/response dto encoding helpers
    let wasm = include_str!("../src/wasm.rs");
    assert!(wasm.contains("encodeJoinRequestBytes"));
    assert!(wasm.contains("decodeJoinResponseJson"));
    assert!(wasm.contains("encodeCatalogRequestBytes"));
    assert!(wasm.contains("decodeCatalogResponseJson"));
}

#[test]
fn browser_javascript_owned_libp2p_peer_facade_is_exposed() {
    // binding-surface: browser javascript-owned libp2p peer facade
    let js = include_str!("../bindings/javascript/index.js.tmpl");
    assert!(js.contains("export class AukiNetworkPeer"));
    assert!(js.contains("static async create"));
    assert!(js.contains("requestJoin"));
    assert!(js.contains("requestCatalog"));
}

#[cfg(feature = "swarm")]
fn spawn_binding_runtime(
    allowed_peers: Vec<auki_network::BindingAllowedPeer>,
) -> std::sync::Arc<auki_network::AukiNetworkRuntime> {
    auki_network::AukiNetworkRuntime::spawn(auki_network::BindingSwarmConfig {
        wallet_seed: vec![7u8; 32],
        listen_multiaddrs: vec!["/ip4/127.0.0.1/tcp/0".into()],
        agent_version: "binding-surface-test/0.1".into(),
        allowed_peers,
        heartbeat_clock_id: None,
        heartbeat_clock_hash_hex: None,
    })
    .expect("binding runtime spawns")
}

#[cfg(feature = "swarm")]
fn wait_for_listen_multiaddr(runtime: &auki_network::AukiNetworkRuntime) -> Vec<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let addrs = runtime.listen_multiaddrs();
        if !addrs.is_empty() {
            return addrs;
        }
        if std::time::Instant::now() >= deadline {
            panic!("binding runtime did not report a listen multiaddr");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(feature = "swarm")]
struct BindingRuntimePair {
    a: std::sync::Arc<auki_network::AukiNetworkRuntime>,
    b: std::sync::Arc<auki_network::AukiNetworkRuntime>,
    a_peer_id: String,
    b_peer_id: String,
}

#[cfg(feature = "swarm")]
impl BindingRuntimePair {
    fn spawn() -> Self {
        let a_seed = NEXT_RUNTIME_SEED.fetch_add(2, Ordering::Relaxed);
        let b_seed = a_seed
            .checked_add(1)
            .expect("binding runtime seed range has enough test values");
        let a = spawn_binding_runtime_with_seed(a_seed, vec![]);
        let b = spawn_binding_runtime_with_seed(b_seed, vec![]);
        let a_peer_id = a.local_peer_id();
        let b_peer_id = b.local_peer_id();
        let a_addrs = wait_for_listen_multiaddr(&a);
        let b_addrs = wait_for_listen_multiaddr(&b);

        let a_allowed = vec![auki_network::BindingAllowedPeer {
            peer_id: b_peer_id.clone(),
            multiaddrs: b_addrs,
        }];
        let b_allowed = vec![auki_network::BindingAllowedPeer {
            peer_id: a_peer_id.clone(),
            multiaddrs: a_addrs,
        }];
        a.set_allowed_peers(a_allowed.clone()).unwrap();
        b.set_allowed_peers(b_allowed.clone()).unwrap();

        wait_until("runtime connected peers", || {
            let connected = a.connected_peers().contains(&b_peer_id)
                && b.connected_peers().contains(&a_peer_id);
            if !connected {
                let _ = a.set_allowed_peers(a_allowed.clone());
                let _ = b.set_allowed_peers(b_allowed.clone());
            }
            connected
        });

        Self {
            a,
            b,
            a_peer_id,
            b_peer_id,
        }
    }
}

#[cfg(feature = "swarm")]
impl Drop for BindingRuntimePair {
    fn drop(&mut self) {
        let _ = self.a.shutdown();
        let _ = self.b.shutdown();
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

#[cfg(feature = "swarm")]
fn spawn_binding_runtime_with_seed(
    seed_byte: u8,
    allowed_peers: Vec<auki_network::BindingAllowedPeer>,
) -> std::sync::Arc<auki_network::AukiNetworkRuntime> {
    auki_network::AukiNetworkRuntime::spawn(auki_network::BindingSwarmConfig {
        wallet_seed: vec![seed_byte; 32],
        listen_multiaddrs: vec!["/ip4/127.0.0.1/tcp/0".into()],
        agent_version: "binding-surface-test/0.1".into(),
        allowed_peers,
        heartbeat_clock_id: None,
        heartbeat_clock_hash_hex: None,
    })
    .expect("binding runtime spawns")
}

#[cfg(feature = "swarm")]
fn wait_for_binding_event(
    mut drain: impl FnMut() -> Vec<auki_network::BindingRuntimeEvent>,
    kind: &str,
) -> auki_network::BindingRuntimeEvent {
    let mut found = None;
    wait_until("binding event", || {
        for event in drain() {
            if event.kind == kind {
                found = Some(event);
                return true;
            }
        }
        false
    });
    found.expect("binding event found")
}

#[cfg(feature = "swarm")]
fn wait_until(label: &str, mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if ready() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("timed out waiting for {label}");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(feature = "swarm")]
fn camera_manifest_json() -> String {
    r#"{"sensor_id":"camera-1","sensor_hash":"camera-hash","clock_id":"clock-1","clock_hash":"clock-hash","frame_id":"frame-1","frame_hash":"frame-hash"}"#.into()
}

#[cfg(feature = "swarm")]
fn detection_manifest_json() -> String {
    r#"{"sensor_id":"detector-1","sensor_hash":"detector-hash","clock_id":"clock-1","clock_hash":"clock-hash","frame_id":"frame-1","frame_hash":"frame-hash"}"#.into()
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
struct MockDiscoveryServer {
    addr: std::net::SocketAddr,
    _handle: std::thread::JoinHandle<()>,
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
impl MockDiscoveryServer {
    fn spawn() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                let (status, body) = if request.starts_with("POST /clusters/demo ") {
                    ("201 Created", discovery_entry_body())
                } else if request.starts_with("GET /clusters ") {
                    (
                        "200 OK",
                        format!(r#"{{"clusters":[{}]}}"#, discovery_entry_body()),
                    )
                } else if request.starts_with("DELETE /clusters/demo ") {
                    ("204 No Content", String::new())
                } else if request.starts_with("GET /nodes?type=relay ") {
                    ("200 OK", discovery_nodes_body())
                } else {
                    (
                        "404 Not Found",
                        r#"{"error":"unexpected mock discovery request"}"#.into(),
                    )
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

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read as _;

    let mut bytes = Vec::new();
    let mut buffer = [0u8; 512];
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

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn write_http_response(stream: &mut std::net::TcpStream, status: &str, body: &str) {
    use std::io::Write as _;

    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn discovery_entry_body() -> String {
    let peer_id = auki_network::PeerIdentity::from_seed(&[53u8; 32])
        .peer_id()
        .to_string();
    serde_json::json!({
        "name": "demo",
        "manager_peer_id": peer_id,
        "manager_multiaddrs": ["/ip4/127.0.0.1/tcp/4001"],
        "relay_multiaddrs": ["/ip4/127.0.0.1/tcp/4002"],
        "peer_count": 1,
        "created_ns": 1,
        "last_liveness_check_ns": 1
    })
    .to_string()
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn discovery_nodes_body() -> String {
    let peer_id = auki_network::PeerIdentity::from_seed(&[54u8; 32])
        .peer_id()
        .to_string();
    serde_json::json!({
        "nodes": [{
            "peer_id": peer_id,
            "node_type": "relay",
            "multiaddrs": ["/ip4/127.0.0.1/tcp/4002"],
            "created_ns": 2,
            "last_liveness_check_ns": 3
        }]
    })
    .to_string()
}
