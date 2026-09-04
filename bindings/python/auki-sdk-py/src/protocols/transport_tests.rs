use std::{str::FromStr, time::SystemTime};

use auki_sdk_rs::{
    AukiPeer, AukiPeerConfig, DdsVerificationKeys, ExternalAuthorityControl,
    ExternalAuthorityUpdate, Identity, Multiaddr, P2PAccessClaims, SignedP2pCredential,
};
use chrono::{TimeZone, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyModule};
use uuid::Uuid;

use crate::{PyAukiPeer, register_sdk};

use super::support::CancelablePythonAwaitable;

const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

const PYTHON_TRANSPORT_EXERCISE: &str = r#"
import hashlib
import struct

async def exercise(
    sdk,
    server,
    client,
    server_peer_id,
    client_peer_id,
    client_subject,
    domain_id,
    server_route,
):
    endpoints = []
    resources = []

    def assert_client(requester):
        assert requester["peer_id"] == client_peer_id
        assert requester["subject"] == client_subject
        assert requester["peer_type"] == "python-client"
        assert requester["domain_ids"] == [domain_id]
        assert requester["scopes"] == ["domain-data:r"]
        assert requester["application"] is None
        assert "credential" not in requester
        assert "token" not in requester

    try:
        info_requesters = []

        def info_provider(requester):
            assert_client(requester)
            info_requesters.append(requester)
            return {
                "app": "python-transport-test",
                "app_version": "1.0.0",
                "name": "local-server",
                "session_id": "session",
                "session_clock_id": "clock",
                "session_clock_hash": "clock-hash",
                "session_now_ns": 7,
                "peer_id": server_peer_id,
                "app_instance": "test",
            }

        info_endpoint = sdk.AukiInfoEndpoint.mount(server, info_provider)
        endpoints.append(info_endpoint)
        info = await sdk.AukiInfoClient(client).fetch_exact(
            server_peer_id, server_route
        )
        assert info["peer_id"] == server_peer_id
        assert info["name"] == "local-server"
        assert len(info_requesters) == 1

        blob_bytes = b"python blob over authenticated local transport"
        blob_hash = hashlib.sha256(blob_bytes).hexdigest()
        blob_calls = 0

        async def blob_provider(requester, request):
            nonlocal blob_calls
            assert_client(requester)
            assert request["sha256"] == blob_hash
            blob_calls += 1
            start = request["offset"]
            end = min(len(blob_bytes), start + request["max_len"])
            return {"total_size": len(blob_bytes), "bytes": blob_bytes[start:end]}

        blob_endpoint = sdk.AukiBlobEndpoint.mount(server, blob_provider)
        endpoints.append(blob_endpoint)
        blob = await sdk.AukiBlobClient(client).fetch(server_peer_id, blob_hash)
        assert blob["remote_peer_id"] == server_peer_id
        assert blob["sha256"] == blob_hash
        assert blob["bytes"] == blob_bytes
        assert blob["relayed"] is False
        assert blob_calls >= 1

        channel = {
            "variant": "message_channel",
            "owner_peer_id": server_peer_id,
            "resource_id": "events",
            "clock": {
                "peer_id": server_peer_id,
                "id": "session/monotonic",
                "hash": "clock-hash",
            },
        }
        message_endpoint = sdk.AukiMessageEndpoint.mount(server)
        endpoints.append(message_endpoint)
        receiver = message_endpoint.declare(channel, 4)
        resources.append((receiver, "close"))
        assert message_endpoint.catalog() == [channel]
        sender = await sdk.AukiMessageClient(client).open_exact(
            server_peer_id, server_route, channel
        )
        resources.append((sender, "close"))
        assert sender.remote_peer["peer_id"] == server_peer_id
        assert sender.remote_peer["peer_type"] == "python-server"
        assert sender.relayed is False
        await sender.send("example.event", 42, b"hello")
        event = await receiver.next()
        assert event["channel"] == channel
        assert event["sender"]["peer_id"] == client_peer_id
        assert event["sender"]["subject"] == client_subject
        assert event["type"] == "example.event"
        assert event["timestamp_ns"] == 42
        assert event["payload"] == b"hello"
        await sender.close()
        await receiver.close()
        assert await receiver.next() is None

        scalar_payload = b"\x09" + struct.pack("<d", 12.5)
        stream_requests = []

        async def scalar_source():
            yield {"timestamp_ns": 99, "payload": scalar_payload}

        def stream_provider(requester, request):
            assert_client(requester)
            assert request == {
                "source_peer_id": server_peer_id,
                "resource_id": "temperature",
                "from": {"kind": "latest"},
            }
            stream_requests.append(request)
            return {
                "kind": "accept",
                "payload_kind": "scalar",
                "manifest": {"resource_id": "temperature", "payload": "scalar"},
                "source": scalar_source(),
            }

        stream_endpoint = sdk.AukiStreamEndpoint.mount(server, stream_provider)
        endpoints.append(stream_endpoint)
        subscription = await sdk.AukiStreamClient(client).subscribe(
            server_peer_id,
            "scalar",
            {
                "source_peer_id": server_peer_id,
                "resource_id": "temperature",
                "from": {"kind": "latest"},
            },
        )
        resources.append((subscription, "cancel"))
        assert subscription.payload_kind == "scalar"
        assert subscription.manifest["resource_id"] == "temperature"
        assert subscription.manifest["payload"] == "scalar"
        entry = await subscription.next()
        assert entry == {
            "kind": "entry",
            "entry": {
                "timestamp_ns": 99,
                "sequence": 0,
                "payload": scalar_payload,
            },
        }
        terminal = await subscription.next()
        assert terminal == {
            "kind": "end",
            "reason": {"kind": "source_ended"},
        }
        await subscription.cancel()
        assert await subscription.next() is None
        assert len(stream_requests) == 1
    finally:
        cleanup_errors = []
        for resource, method in reversed(resources):
            try:
                await getattr(resource, method)()
            except BaseException as error:
                cleanup_errors.append(error)
        for endpoint in reversed(endpoints):
            try:
                await endpoint.close()
            except BaseException as error:
                cleanup_errors.append(error)
        for peer in (client, server):
            try:
                await peer.shutdown()
            except BaseException as error:
                cleanup_errors.append(error)
        if cleanup_errors:
            raise cleanup_errors[0]
"#;

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system clock must follow the Unix epoch")
        .as_secs()
}

fn signed_authority(
    identity: &Identity,
    domain_id: Uuid,
    subject: Uuid,
    peer_type: &str,
) -> ExternalAuthorityUpdate {
    let issued_at = unix_time();
    let expires_at = issued_at + 30 * 60;
    let claims = P2PAccessClaims {
        token_type: "p2p-access".into(),
        iss: "dds".into(),
        aud: vec!["auki-p2p".into()],
        sub: subject.to_string(),
        organization_id: None,
        peer_type: Some(peer_type.into()),
        peer_id: identity.peer_id().to_string(),
        domain_ids: vec![domain_id.to_string()],
        scopes: vec!["domain-data:r".into()],
        application: None,
        iat: issued_at,
        nbf: None,
        exp: expires_at,
    };
    let compact = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_DDS_PRIVATE_KEY).expect("test DDS key must parse"),
    )
    .expect("test DDS credential must sign");
    ExternalAuthorityUpdate::new(
        domain_id,
        identity.peer_id(),
        DdsVerificationKeys::new(0, TEST_DDS_PUBLIC_KEY.to_vec(), None),
        SignedP2pCredential::new(compact).expect("signed test credential must be bounded"),
        Utc.timestamp_opt(expires_at as i64, 0)
            .single()
            .expect("test expiration must be representable"),
    )
}

fn direct_config() -> AukiPeerConfig {
    AukiPeerConfig::new("http://127.0.0.1:9")
        .expect("loopback DMS URL must be valid")
        .direct_only()
        .with_listen_addresses([
            Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").expect("test listen address must parse")
        ])
        .expect("test listen address must be accepted")
}

async fn start_peer(
    identity: Identity,
    authority: ExternalAuthorityUpdate,
    config: AukiPeerConfig,
) -> PyResult<(AukiPeer, ExternalAuthorityControl)> {
    AukiPeer::start_external(identity, authority, config)
        .await
        .map_err(|error| PyRuntimeError::new_err(format!("start local test peer: {error}")))
}

#[test]
fn python_protocols_round_trip_over_two_authenticated_local_peers() {
    pyo3::prepare_freethreaded_python();
    let event_loop = Python::with_gil(|py| {
        py.import_bound("asyncio")
            .expect("asyncio must import")
            .call_method0("new_event_loop")
            .expect("test event loop must construct")
            .unbind()
    });

    Python::with_gil(|py| {
        pyo3_async_runtimes::tokio::run_until_complete(event_loop.bind(py).clone(), async move {
            let domain_id = Uuid::new_v4();
            let server_subject = Uuid::new_v4();
            let client_subject = Uuid::new_v4();
            let server_identity = Identity::generate();
            let client_identity = Identity::generate();
            let server_peer_id = server_identity.peer_id();
            let client_peer_id = client_identity.peer_id();

            let (server, _server_authority) = start_peer(
                server_identity.clone(),
                signed_authority(&server_identity, domain_id, server_subject, "python-server"),
                direct_config(),
            )
            .await?;
            let server_route = server
                .listen_addresses()
                .first()
                .cloned()
                .ok_or_else(|| PyRuntimeError::new_err("server has no local listen route"))?;
            let client_config = direct_config()
                .with_peer_routes(server_peer_id, [server_route.clone()])
                .map_err(|error| {
                    PyRuntimeError::new_err(format!("configure local peer route: {error}"))
                })?;
            let (client, _client_authority) = start_peer(
                client_identity.clone(),
                signed_authority(&client_identity, domain_id, client_subject, "python-client"),
                client_config,
            )
            .await?;

            let python_task = Python::with_gil(|py| -> PyResult<_> {
                let sdk = PyModule::new_bound(py, "auki_sdk")?;
                register_sdk(&sdk)?;
                let script = PyModule::from_code_bound(
                    py,
                    PYTHON_TRANSPORT_EXERCISE,
                    "python_protocol_transport_test.py",
                    "python_protocol_transport_test",
                )?;
                let server = Py::new(py, PyAukiPeer::from_test_peer(server))?;
                let client = Py::new(py, PyAukiPeer::from_test_peer(client))?;
                let coroutine = script.getattr("exercise")?.call1((
                    sdk,
                    server,
                    client,
                    server_peer_id.to_string(),
                    client_peer_id.to_string(),
                    client_subject.to_string(),
                    domain_id.to_string(),
                    server_route.to_string(),
                ))?;
                let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?;
                CancelablePythonAwaitable::schedule(py, &locals, coroutine)
            })?;

            tokio::time::timeout(std::time::Duration::from_secs(30), python_task)
                .await
                .map_err(|_| PyRuntimeError::new_err("local Python protocol test timed out"))??;
            Ok(())
        })
    })
    .expect("authenticated local Python protocol round trip must succeed");
}
