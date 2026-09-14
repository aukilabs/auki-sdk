"""Robot admission and task P2P against loopback DDS/DMS/data and real local peers."""
import asyncio
import base64
import hashlib
import json
import time
from datetime import datetime, timezone

import pytest
import auki_sdk
from test_tasks import worker_services, CAPABILITY, DOMAIN, DATA, ROBOT


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


@pytest.fixture
def p2p():
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import ec, ed25519, utils

    key = ec.generate_private_key(ec.SECP256R1())
    public = key.public_key()
    pem = public.public_bytes(serialization.Encoding.PEM, serialization.PublicFormat.SubjectPublicKeyInfo)
    der = public.public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    issued = int(time.time()) - 1

    def grant(state):
        claims = {"type": "p2p-access", "iss": "dds", "aud": ["auki-p2p"], "sub": ROBOT,
                  "peer_type": "robot" if state.get("robot") else "compute",
                  "peer_id": state["peer_id"], "domain_ids": [DOMAIN], "scopes": ["domain-data:r"],
                  "iat": state.get("peer_issued", issued), "exp": state.get("peer_issued", issued) + 1800}
        claims.update(state.get("peer_claims_override", {}))
        message = b64(b'{"alg":"ES256","typ":"JWT"}') + "." + b64(json.dumps(claims).encode())
        r, s = utils.decode_dss_signature(key.sign(message.encode(), ec.ECDSA(hashes.SHA256())))
        signature = b64(r.to_bytes(32, "big") + s.to_bytes(32, "big"))
        if state.get("bad_signature"):
            signature = b64(bytes(64))
        return {"p2p_access_token": message + "." + signature,
                "p2p_access_expires_at": datetime.fromtimestamp(claims["exp"], timezone.utc).isoformat()}

    def verify_proof(encoded_key, signature):
        protobuf = base64.urlsafe_b64decode(encoded_key + "=" * (-len(encoded_key) % 4))
        assert protobuf[:4] == b"\x08\x01\x12\x20"
        raw_signature = base64.urlsafe_b64decode(signature + "=" * (-len(signature) % 4))
        ed25519.Ed25519PublicKey.from_public_bytes(protobuf[4:]).verify(raw_signature, b"fixture")

    return {"grant": grant, "verify_proof": verify_proof, "pem": pem,
            "keys": {"version": 1, "generation": 1, "previous_key_overlap_seconds": 1860,
                     "keys": [{"id": hashlib.sha256(der).hexdigest(), "status": "current",
                               "signing_method": "ES256", "public_key": pem.decode()}]}}


def runtime(services, handler, kind="robot", peer_file=None):
    options = dict(dds_url=services["base"], dms_url=services["base"], version="1.0.0",
                   client_id="machine-fixture", request_timeout=2, registration_interval=0.2)
    if peer_file is not None:
        options.update(peer_identity_file=str(peer_file),
                       peer_config=auki_sdk.AukiPeerConfig.new(services["base"]).direct_only()
                       .with_listen_addresses(["/ip4/127.0.0.1/tcp/0"]))
    if kind == "robot":
        services["robot"] = True
        credential = auki_sdk.AukiRobotCredential(**options, registration="fixture-robot-registration",
            audience=services["base"] + "/robots", capabilities=[CAPABILITY])
    else:
        credential = auki_sdk.AukiComputeCredential(**options, registration="fixture-registration", wallet_key="01" * 32)
    tasks = auki_sdk.AukiDmsTasks(credential, {CAPABILITY: handler},
        poll_interval=0.02, heartbeat_interval=0.05, request_timeout=2)
    return credential, tasks


def test_robot_idle_reads_and_task_writes_use_separate_authority(worker_services):
    async def scenario():
        async def handler(task):
            assert task.peer() is None
            await task.data().write(b"robot-result", data_id=DATA)
            return {"output_cids": [DATA]}
        robot, tasks = runtime(worker_services, handler)
        idle = robot.data(DOMAIN)
        try:
            assert await robot.assigned_domain_id() == DOMAIN
            assert await idle.read(DATA) == b"hello"
            calls = list(worker_services["calls"])
            with pytest.raises(auki_sdk.DomainDataError, match="only permits reads"):
                await idle.write(b"forbidden", data_id=DATA)
            with pytest.raises(auki_sdk.DomainDataError, match="only permits reads"):
                await idle.delete(DATA)
            async def source(maximum):
                pytest.fail("read-only credential must not consume an upload source")
            with pytest.raises(auki_sdk.DomainDataError, match="only permits reads"):
                await idle.write_stream(1, source, data_id=DATA)
            assert calls == worker_services["calls"]
            assert await tasks.run_once() == "completed"
            assert await idle.read(DATA) == b"robot-result"
            assert not any("siwe" in path or "register-wallet" in path for _, path in worker_services["calls"])
        finally:
            await idle.close()
            await tasks.close()
            await robot.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("override", [{"scopes": ["domain:rw"]}, {"domain_id": DATA},
                                      {"iss": "wrong"}, {"aud": ["https://wrong.invalid"]}, {"exp": 1}])
def test_robot_idle_read_rejects_invalid_grant(worker_services, override):
    async def scenario():
        worker_services["read_claims_override"] = override
        async def handler(task):
            pytest.fail("idle read must not execute tasks")
        robot, tasks = runtime(worker_services, handler)
        idle = robot.data(DOMAIN)
        try:
            with pytest.raises(auki_sdk.DomainDataError):
                await idle.read(DATA)
            assert not any("/api/v1/domains/" in path for _, path in worker_services["calls"])
        finally:
            await idle.close()
            await tasks.close()
            await robot.close()
    asyncio.run(scenario())


@pytest.mark.skipif(not hasattr(auki_sdk, "AukiInfoEndpoint"), reason="Info is an optional application protocol")
@pytest.mark.parametrize("kind", ["compute", "robot"])
def test_task_peer_exchanges_authenticated_info_with_another_local_peer(worker_services, p2p, tmp_path, kind):
    async def scenario():
        worker_services["p2p"] = p2p
        identity = auki_sdk.Identity.from_ed25519_seed(bytes([42]) * 32)
        grant = p2p["grant"]({"peer_id": identity.peer_id})
        update = auki_sdk.ExternalAuthorityUpdate(DOMAIN, identity.peer_id,
            auki_sdk.DdsVerificationKeys(1, p2p["pem"]),
            auki_sdk.SignedP2pCredential(grant["p2p_access_token"]), grant["p2p_access_expires_at"])
        config = auki_sdk.AukiPeerConfig.new(worker_services["base"]).direct_only().with_listen_addresses(["/ip4/127.0.0.1/tcp/0"])
        remote, control = await auki_sdk.AukiPeer.start_external(identity, update, config)
        requesters = []
        def info(requester):
            requesters.append(requester)
            return {"app": "fixture", "app_version": "1.0.0", "name": "local peer",
                    "session_id": "session", "session_clock_id": "clock", "session_clock_hash": "00" * 32,
                    "session_now_ns": 0, "peer_id": remote.peer_id, "app_instance": "fixture"}
        endpoint = auki_sdk.AukiInfoEndpoint.mount(remote, info)
        async def handler(task):
            peer = task.peer()
            client = auki_sdk.AukiInfoClient(peer)
            result = await client.fetch_exact(remote.peer_id, remote.listen_addresses[0])
            assert result["name"] == "local peer"
            assert await task.data().read(DATA) == b"hello"
        machine, tasks = runtime(worker_services, handler, kind, tmp_path / "worker.identity")
        try:
            assert await asyncio.wait_for(tasks.run_once(), 10) == "completed"
            assert len(requesters) == 1
        finally:
            await tasks.close()
            await machine.close()
            await endpoint.close()
            await remote.shutdown()
    asyncio.run(scenario())


@pytest.mark.parametrize("assignment", [None, DATA])
def test_unassigned_or_other_domain_robot_cannot_read_or_execute(worker_services, assignment):
    async def scenario():
        worker_services["assignment"] = assignment
        async def handler(task):
            pytest.fail("wrong-assignment task must not execute")
        robot, tasks = runtime(worker_services, handler)
        idle = robot.data(DOMAIN)
        try:
            assert await robot.assigned_domain_id() == assignment
            with pytest.raises(auki_sdk.DomainDataError):
                await idle.read(DATA)
            if assignment is None:
                assert await tasks.run_once() == "no_work"
                assert worker_services["claims"] == 0
            else:
                with pytest.raises(auki_sdk.TaskRuntimeError, match="assigned Domain"):
                    await tasks.run_once()
            assert worker_services["complete"] == []
        finally:
            await idle.close()
            await tasks.close()
            await robot.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("override", [{"aud": ["wrong"]}, {"iss": "wrong"}, {"node_type": "compute"},
                                      {"exp": 1}, {"sub": DATA}])
def test_robot_invalid_machine_profile_is_rejected_before_claim(worker_services, override):
    async def scenario():
        worker_services["machine_claims_override"] = override
        async def handler(task):
            pytest.fail("invalid robot authority")
        robot, tasks = runtime(worker_services, handler)
        try:
            with pytest.raises(auki_sdk.TaskRuntimeError):
                await tasks.run_once()
            assert worker_services["claims"] == 0
        finally:
            await tasks.close()
            await robot.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("kind", ["robot", "compute"])
def test_cancelled_authentication_can_be_awaited_by_the_next_run(worker_services, kind):
    async def scenario():
        worker_services["auth_delay"] = 0.3
        async def handler(task):
            return None
        machine, tasks = runtime(worker_services, handler, kind)
        first = asyncio.ensure_future(tasks.run_once())
        try:
            key = "register" if kind == "robot" else "verify"
            async def authenticating():
                while not worker_services[key]:
                    await asyncio.sleep(0.005)
            await asyncio.wait_for(authenticating(), 3)
            first.cancel()
            with pytest.raises(asyncio.CancelledError):
                await first
            # Cancellation requests are delivered to the owned native operation.
            async def retry():
                while True:
                    outcome = await tasks.run_once()
                    if outcome != "busy":
                        return outcome
                    await asyncio.sleep(0.005)
            assert await asyncio.wait_for(retry(), 5) == "completed"
        finally:
            await tasks.close()
            await machine.close()
    asyncio.run(scenario())


def test_robot_idle_renewal_coalesces_and_preserves_dds_denial(worker_services):
    async def scenario():
        worker_services["reject_read_once"] = True
        async def handler(task):
            return None
        machine, tasks = runtime(worker_services, handler)
        idle = machine.data(DOMAIN)
        try:
            assert await asyncio.gather(*(idle.read(DATA) for _ in range(4))) == [b"hello"] * 4
            assert worker_services["read_exchanges"] == 2
            assert worker_services["verify"] == 1
            worker_services["generation"] += 1
            worker_services["read_exchange_status"] = 403
            with pytest.raises(auki_sdk.DomainDataError) as denied:
                await idle.read(DATA)
            assert denied.value.status == 403
        finally:
            await idle.close()
            await tasks.close()
            await machine.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("setting", ["bad_signature", "missing_peer_grant"])
def test_missing_or_forged_task_peer_credential_never_runs(worker_services, p2p, tmp_path, setting):
    async def scenario():
        worker_services.update(p2p=p2p)
        worker_services[setting] = True
        async def handler(task):
            pytest.fail("task peer authority must be verified first")
        machine, tasks = runtime(worker_services, handler, "compute", tmp_path / "identity")
        try:
            with pytest.raises(auki_sdk.TaskRuntimeError):
                await asyncio.wait_for(tasks.run_once(), 8)
            assert worker_services["complete"] == []
        finally:
            await tasks.close()
            await machine.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("kind", ["robot", "compute"])
def test_task_peer_binding_rotation_and_awaited_shutdown(worker_services, p2p, tmp_path, kind):
    async def scenario():
        worker_services["p2p"] = p2p
        retained = []
        async def handler(task):
            peer = task.peer()
            assert peer.domain_id == DOMAIN and peer.peer_id == worker_services["peer_id"]
            assert peer.listen_addresses
            retained.append(peer)
            with pytest.raises(RuntimeError, match="task runtime owns"):
                peer.shutdown()
            await task.progress({"phase": "peer-ready"})
            worker_services["peer_issued"] = int(time.time())
            await asyncio.sleep(0.15)
            assert await task.data().read(DATA) == b"hello"
            return None
        machine, tasks = runtime(worker_services, handler, kind, tmp_path / "peer.identity")
        try:
            assert await asyncio.wait_for(tasks.run_once(), 8) == "completed"
            await asyncio.wait_for(retained[0].wait_stopped(), 2)
            with pytest.raises(RuntimeError):
                _ = retained[0].routes
            paths = [path for _, path in worker_services["calls"]]
            assert paths.index("/internal/v1/auth/p2p/verify") < paths.index("/tasks")
            assert paths.count("/service/p2p-verification-keys") >= 2
            assert ("/internal/v1/auth/robot/p2p-token" in paths) == (kind == "robot")
        finally:
            await tasks.close()
            await machine.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("override", [{"domain_ids": [DATA]}, {"peer_id": "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"},
    {"iss": "wrong"}, {"aud": ["wrong"]}, {"scopes": []}, {"peer_type": "user"}, {"exp": 1}])
def test_task_peer_rejects_invalid_authority_before_handler(worker_services, p2p, tmp_path, override):
    async def scenario():
        worker_services.update(p2p=p2p, peer_claims_override=override)
        async def handler(task):
            pytest.fail("invalid peer authority must not reach application")
        machine, tasks = runtime(worker_services, handler, "compute", tmp_path / "peer.identity")
        try:
            with pytest.raises(auki_sdk.TaskRuntimeError):
                await asyncio.wait_for(tasks.run_once(), 8)
            assert worker_services["complete"] == []
        finally:
            await tasks.close()
            await machine.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("cause", ["cancel", "lease_loss", "bad_rotation", "registration_loss"])
def test_peer_failure_cancels_handler_and_awaits_cleanup(worker_services, p2p, tmp_path, cause):
    async def scenario():
        worker_services["p2p"] = p2p
        started, cleaning, release = asyncio.Event(), asyncio.Event(), asyncio.Event()
        retained = []
        async def handler(task):
            retained.append(task.peer())
            started.set()
            try:
                await asyncio.sleep(60)
            finally:
                assert task.is_cancelled()
                cleaning.set()
                await release.wait()
        machine, tasks = runtime(worker_services, handler, "robot", tmp_path / "peer.identity")
        operation = asyncio.ensure_future(tasks.run_once())
        try:
            await asyncio.wait_for(started.wait(), 8)
            if cause == "cancel":
                operation.cancel()
            elif cause == "lease_loss":
                worker_services["heartbeat_status"] = 409
            elif cause == "bad_rotation":
                worker_services["peer_claims_override"] = {"domain_ids": [DATA]}
            else:
                worker_services["registration_status"] = 403
            await asyncio.wait_for(cleaning.wait(), 5)
            closing = asyncio.ensure_future(tasks.close())
            await asyncio.sleep(0.05)
            assert not closing.done()
            with pytest.raises(RuntimeError):
                _ = retained[0].routes
            release.set()
            await asyncio.wait_for(closing, 5)
            await asyncio.wait_for(retained[0].wait_stopped(), 2)
            with pytest.raises((asyncio.CancelledError, auki_sdk.TaskRuntimeError)):
                await operation
            assert worker_services["complete"] == [] and worker_services["fail"] == []
        finally:
            release.set()
            await tasks.close()
            await machine.close()
            await asyncio.gather(operation, return_exceptions=True)
    asyncio.run(scenario())
