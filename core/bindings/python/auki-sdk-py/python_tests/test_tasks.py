"""Compute/handler acceptance tests. All DDS, DMS and data traffic is loopback."""
import asyncio
import base64
import contextvars
from datetime import datetime, timedelta, timezone
from email.parser import BytesParser
from email.policy import default
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import pytest
import auki_sdk

DOMAIN = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
TASK = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
DATA = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
ROBOT = "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
CAPABILITY = "/example/uppercase/v1"
TRACE = contextvars.ContextVar("task_trace", default="unset")


def expires(seconds=60):
    return (datetime.now(timezone.utc) + timedelta(seconds=seconds)).isoformat()


@pytest.fixture
def worker_services():
    state = {"calls": [], "heartbeats": [], "complete": [], "fail": [], "register": 0,
             "verify": 0, "claims": 0, "generation": 0, "bytes": b"hello", "cancel": False,
             "heartbeat_status": 200, "claim_status": 200, "registration_status": 200,
             "data_status": 200, "rotate": False}

    def token(read_only=False):
        claims = {"iss": "dds", "domain_id": DOMAIN, "aud": [state["base"]],
                  "exp": int(time.time()) + 120, "generation": state["generation"]}
        claims.update(state.get("claims_override", {}))
        if read_only:
            claims.update({"type": "robot", "sub": ROBOT, "scopes": ["domain:r"]})
            claims.update(state.get("read_claims_override", {}))
        body = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=")
        return "e30." + body + ".fixture"

    def machine_token(bound=False):
        if not state.get("robot"):
            return "bound-machine-fixture" if bound else "machine-fixture"
        claims = {"iss": "dds", "aud": [state["base"] + "/robots"], "sub": ROBOT,
                  "node_id": ROBOT, "organization_id": DATA, "node_type": "robot",
                  "node_mode": "dedicated", "assigned_domain_id": state.get("assignment", DOMAIN),
                  "iat": int(time.time()) - 1, "exp": int(time.time()) + 120}
        if bound:
            claims["peer_id"] = state["peer_id"]
        claims.update(state.get("machine_claims_override", {}))
        body = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=")
        return "e30." + body + ".fixture"

    def grant():
        response = {"domain_id": DOMAIN, "domain_server_url": state["base"], "access_token": token(),
                "access_token_expires_at": expires(120), "lease_expires_at": expires(state.get("ttl", 60))}
        if state.get("p2p") and not state.get("robot") and not state.get("missing_peer_grant"):
            peer = state["p2p"]["grant"](state)
            response.update(p2p_access_token=peer["p2p_access_token"],
                            p2p_access_token_expires_at=peer["p2p_access_expires_at"])
        return response

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, value=None, status=200):
            body = value if isinstance(value, bytes) else json.dumps(value).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/octet-stream" if isinstance(value, bytes) else "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def handle_request(self):
            path = urlparse(self.path)
            query = parse_qs(path.query)
            body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            state["calls"].append((self.command, path.path))
            if path.path in ("/internal/v1/robots/register", "/internal/v1/auth/robot/verify"):
                payload = json.loads(body)
                assert payload["registration_credentials"] == "fixture-robot-registration"
                if path.path.endswith("/register"):
                    assert payload["capabilities"] == [CAPABILITY]
                    state["register"] += 1
                    if state["register"] == 1:
                        time.sleep(state.get("auth_delay", 0))
                else:
                    state["verify"] += 1
                return self.reply({"robot_id": ROBOT, "access_token": machine_token(),
                                   "access_expires_at": expires(120)}, state["registration_status"])
            if path.path == "/internal/v1/auth/robot/domain-token":
                assert json.loads(body) == {"domain_id": DOMAIN}
                state["read_exchanges"] = state.get("read_exchanges", 0) + 1
                if state.get("reject_read_once") and state["read_exchanges"] == 1:
                    return self.reply({}, 401)
                if state.get("read_exchange_status", 200) != 200:
                    return self.reply({}, state["read_exchange_status"])
                return self.reply({"domain_id": DOMAIN, "domain_server_url": state["base"],
                                   "access_token": token(True), "access_expires_at": expires(120)})
            if path.path == "/internal/v1/auth/p2p/challenge":
                payload = json.loads(body)
                state["peer_id"] = payload["peer_id"]
                state["peer_public_key"] = payload["public_key"]
                return self.reply({"challenge_id": "fixture-challenge", "challenge": "Zml4dHVyZQ",
                                   "expires_at": expires(60)})
            if path.path == "/internal/v1/auth/p2p/verify":
                payload = json.loads(body)
                state["p2p"]["verify_proof"](state["peer_public_key"], payload["signature"])
                return self.reply({"peer_id": state.get("binding_peer_override", state["peer_id"]),
                                   "access_token": machine_token(True), "access_expires_at": expires(120)})
            if path.path == "/internal/v1/auth/robot/p2p-token":
                assert json.loads(body) == {"domain_id": DOMAIN}
                assert "Authorization" in self.headers
                state["peer_exchanges"] = state.get("peer_exchanges", 0) + 1
                time.sleep(state.get("peer_exchange_delay", 0))
                return self.reply(state["p2p"]["grant"](state), state.get("peer_exchange_status", 200))
            if path.path == "/service/p2p-verification-keys":
                return self.reply(state["p2p"]["keys"])
            if path.path.endswith("/siwe/request"):
                return self.reply({"nonce": "local-fixture", "domain": "localhost", "uri": state["base"],
                                   "version": "1", "chainId": 1, "issuedAt": expires(0)})
            if path.path.endswith("/register-wallet"):
                payload = json.loads(body)
                assert payload["capabilities"] == [CAPABILITY]
                assert payload["registration_credentials"] == "fixture-registration"
                assert len(payload["signature"]) == 132
                state["register"] += 1
                return self.reply({}, state["registration_status"])
            if path.path.endswith("/siwe/verify"):
                assert len(json.loads(body)["signature"]) == 132
                state["verify"] += 1
                if state["verify"] == 1:
                    time.sleep(state.get("auth_delay", 0))
                return self.reply({"access_token": "machine-fixture", "access_expires_at": expires(120)})
            if path.path == "/tasks":
                expected = machine_token(bool(state.get("p2p")))
                # Robot tokens contain issuance time; compare the invariant profile.
                if state.get("robot"):
                    encoded = self.headers["Authorization"].split(".")[1]
                    claims = json.loads(base64.urlsafe_b64decode(encoded + "=" * (-len(encoded) % 4)))
                    assert claims["node_type"] == "robot"
                    if state.get("p2p"):
                        assert claims["peer_id"] == state["peer_id"]
                else:
                    assert self.headers["Authorization"] == "Bearer " + expected
                assert query == {"capability": [CAPABILITY]}
                state["claims"] += 1
                if state.get("reject_claim_once") and state["claims"] == 1:
                    return self.reply({}, 401)
                return self.reply({**grant(), "task": {"id": TASK, "capability": CAPABILITY,
                    "meta": {"input_id": DATA}, "inputs_cids": [DATA]}}, state["claim_status"])
            if path.path.endswith("/heartbeat"):
                heartbeat = json.loads(body)
                state["heartbeats"].append(heartbeat)
                if heartbeat.get("events") and "heartbeat_gate" in state:
                    state["heartbeat_waiting"] = True
                    assert state["heartbeat_gate"].wait(3), "test did not release heartbeat"
                if state["rotate"]:
                    state["generation"] += 1
                response = {**grant(), "task_id": TASK, "cancel": state["cancel"]}
                response.update(state.get("heartbeat_override", {}))
                return self.reply(response, state["heartbeat_status"])
            if path.path.endswith("/complete"):
                state["complete"].append(json.loads(body))
                return self.reply({})
            if path.path.endswith("/fail"):
                state["fail"].append(json.loads(body))
                return self.reply({})
            if path.path == "/api/v1/info":
                assert "Authorization" not in self.headers
                return self.reply({"upload": {"domain_data_max_bytes": 100000, "request_max_bytes": 100000,
                    "multipart": {"enabled": False}}})
            if "/data" in path.path:
                bearer = self.headers.get("Authorization")
                if bearer == "Bearer " + token(True):
                    if self.command != "GET":
                        return self.reply({}, 403)
                elif bearer != "Bearer " + token():
                    return self.reply({}, 401)
                if state["data_status"] != 200:
                    return self.reply({"error": "do not expose this fixture response"}, state["data_status"])
                if query.get("raw") == ["true"]:
                    return self.reply(state["bytes"])
                if self.command == "PUT":
                    message = BytesParser(policy=default).parsebytes(
                        ("Content-Type: " + self.headers["Content-Type"] + "\r\n\r\n").encode() + body)
                    state["bytes"] = next(message.iter_parts()).get_payload(decode=True)
                metadata = {"id": DATA, "domain_id": DOMAIN, "name": "uppercase", "data_type": "text.v1",
                    "size": len(state["bytes"]), "created_at": expires(0), "updated_at": expires(0)}
                return self.reply({"data": [metadata]} if self.command == "PUT" else metadata)
            return self.reply({}, 404)

        do_POST = do_GET = do_PUT = do_DELETE = handle_request

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    state["base"] = f"http://127.0.0.1:{server.server_port}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield state
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def runtime(services, handler):
    credential = auki_sdk.AukiComputeCredential(
        dds_url=services["base"], dms_url=services["base"],
        registration="fixture-registration", wallet_key="01" * 32,
        version="1.0.0", client_id="python-worker-fixture",
        request_timeout=2, registration_interval=0.2)
    tasks = auki_sdk.AukiDmsTasks(credential, {CAPABILITY: handler},
        poll_interval=0.02, heartbeat_interval=0.05, request_timeout=2)
    return credential, tasks


def test_python_compute_data_progress_result_and_registration_shutdown(worker_services):
    async def scenario():
        TRACE.set("from-caller")
        retained = []

        async def handler(task):
            assert TRACE.get() == "from-caller"
            assert task.id == TASK and task.domain_id == DOMAIN
            assert task.inputs_cids == [DATA]
            data = task.data()
            retained.append(data)
            value = await data.read(task.meta["input_id"])
            await task.progress({"phase": "writing"})
            await data.write(value.upper(), data_id=DATA)
            return {"output_cids": [DATA], "meta": {"done": True}}

        credential, tasks = runtime(worker_services, handler)
        assert worker_services["calls"] == []
        try:
            assert await tasks.run_once() == "completed"
            assert worker_services["bytes"] == b"HELLO"
            assert worker_services["complete"] == [{"output_cids": [DATA], "meta": {"done": True}}]
            assert worker_services["heartbeats"][-1]["progress"] == {"phase": "writing"}
            with pytest.raises(auki_sdk.DomainDataError):
                await retained[0].read(DATA)
            await asyncio.sleep(0.25)
            assert worker_services["register"] >= 2
        finally:
            await tasks.close()
            await credential.close()
        counts = (worker_services["register"], len(worker_services["heartbeats"]))
        await asyncio.sleep(0.25)
        assert counts == (worker_services["register"], len(worker_services["heartbeats"]))
    asyncio.run(scenario())


@pytest.mark.parametrize("cause", ["python_cancel", "close", "dms_cancel", "lease_loss", "wrong_domain", "wrong_task", "registration_loss"])
def test_cancellation_waits_for_python_finally_and_skips_completion(worker_services, cause):
    async def scenario():
        started = asyncio.Event()
        cleaning = asyncio.Event()
        release = asyncio.Event()
        cleaned = asyncio.Event()

        async def handler(task):
            token = task.access_token
            assert token.get()
            await task.set_failure("must not report after cancellation", {"artifacts": [DATA]})
            started.set()
            try:
                await asyncio.sleep(100)
            finally:
                assert task.is_cancelled()
                with pytest.raises(auki_sdk.TaskRuntimeError):
                    token.get()
                with pytest.raises(auki_sdk.TaskRuntimeError):
                    await task.log_event({"too_late": True})
                cleaning.set()
                await release.wait()
                cleaned.set()

        credential, tasks = runtime(worker_services, handler)
        operation = asyncio.ensure_future(tasks.run_once())
        closing = None
        try:
            await asyncio.wait_for(started.wait(), 3)
            assert await tasks.run_once() == "busy"
            if cause == "python_cancel":
                operation.cancel()
            elif cause == "close":
                closing = asyncio.ensure_future(tasks.close())
            elif cause == "dms_cancel":
                worker_services["cancel"] = True
            elif cause == "lease_loss":
                worker_services["heartbeat_status"] = 409
            elif cause == "registration_loss":
                worker_services["registration_status"] = 403
            else:
                worker_services["heartbeat_override"] = {
                    "domain_id" if cause == "wrong_domain" else "task_id": DATA}
            await asyncio.wait_for(cleaning.wait(), 3)
            if closing is None:
                closing = asyncio.ensure_future(tasks.close())
            await asyncio.sleep(0.05)
            assert not closing.done() and not cleaned.is_set()
            assert worker_services["claims"] == 1
            release.set()
            await asyncio.wait_for(closing, 3)
            assert cleaned.is_set()
            with pytest.raises((asyncio.CancelledError, auki_sdk.TaskRuntimeError)):
                await operation
            assert worker_services["complete"] == [] and worker_services["fail"] == []
        finally:
            release.set()
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("status,expected", [(204, "no_work"), (409, "busy"), (403, None)])
def test_claim_distinguishes_no_work_busy_and_denied(worker_services, status, expected):
    async def scenario():
        worker_services["claim_status"] = status
        async def handler(task):
            pytest.fail("handler must not run without a lease")
        credential, tasks = runtime(worker_services, handler)
        try:
            if expected is None:
                with pytest.raises(auki_sdk.TaskRuntimeError) as denied:
                    await tasks.run_once()
                assert denied.value.status == 403
            else:
                assert await tasks.run_once() == expected
            assert worker_services["claims"] == 1
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


def test_machine_unauthorized_refreshes_before_one_replay(worker_services):
    async def scenario():
        worker_services["reject_claim_once"] = True
        async def handler(task):
            return None
        credential, tasks = runtime(worker_services, handler)
        try:
            assert await tasks.run_once() == "completed"
            assert worker_services["claims"] == 2
            assert worker_services["verify"] == 2
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


def test_registration_denial_never_authenticates_or_claims(worker_services):
    async def scenario():
        worker_services["registration_status"] = 403
        async def handler(task):
            pytest.fail("handler must not run without registration")
        credential, tasks = runtime(worker_services, handler)
        try:
            with pytest.raises(auki_sdk.TaskRuntimeError) as denied:
                await tasks.run_once()
            assert denied.value.kind == "authentication"
            assert worker_services["verify"] == 0 and worker_services["claims"] == 0
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


def test_managed_idle_loop_surfaces_registration_loss(worker_services):
    async def scenario():
        worker_services["claim_status"] = 204
        async def handler(task):
            pytest.fail("idle worker must not run a handler")
        credential, tasks = runtime(worker_services, handler)
        operation = asyncio.ensure_future(tasks.run())
        try:
            while worker_services["claims"] == 0:
                await asyncio.sleep(0.01)
            with pytest.raises(auki_sdk.TaskRuntimeError) as busy:
                await tasks.run()
            assert busy.value.kind == "busy"
            worker_services["registration_status"] = 403
            with pytest.raises(auki_sdk.TaskRuntimeError) as lost:
                await asyncio.wait_for(operation, 3)
            assert lost.value.kind == "authentication"
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


def test_one_task_runtime_per_compute_credential(worker_services):
    async def scenario():
        async def handler(task):
            return None
        credential, tasks = runtime(worker_services, handler)
        try:
            with pytest.raises(auki_sdk.TaskRuntimeError) as duplicate:
                auki_sdk.AukiDmsTasks(credential, {CAPABILITY: handler})
            assert duplicate.value.kind == "configuration"
            assert worker_services["calls"] == []
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


def test_handler_exception_reports_failure_and_reaches_python(worker_services):
    async def scenario():
        async def handler(task):
            raise ValueError("application-only exception")
        credential, tasks = runtime(worker_services, handler)
        try:
            with pytest.raises(ValueError, match="application-only exception"):
                await tasks.run_once()
            assert worker_services["complete"] == []
            assert worker_services["fail"] == [{"reason": "task handler failed", "details": None}]
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("method", ["run", "run_once"])
def test_managed_events_drain_in_order_and_failure_preserves_artifact_metadata(worker_services, method):
    async def scenario():
        gate = threading.Event()
        worker_services["heartbeat_gate"] = gate
        returned = asyncio.Event()
        retained = []
        details = {"job": {"task_id": TASK}, "artifacts": [
            {"id": DATA, "logical_path": "partial.json", "metadata": {"partial": True}}]}

        async def handler(task):
            retained.append(task.access_token)
            await task.log_event({"sequence": 1})
            while not worker_services.get("heartbeat_waiting"):
                await asyncio.sleep(0.005)
            await task.log_event({"sequence": 2})
            await task.log_event({"sequence": 3})
            await task.progress({"phase": "failed"})
            await task.set_failure("reconstruction failed", details)
            returned.set()
            raise ValueError("private exception text is not a receipt")

        credential, tasks = runtime(worker_services, handler)
        operation = asyncio.ensure_future(getattr(tasks, method)())
        try:
            await asyncio.wait_for(returned.wait(), 2)
            await asyncio.sleep(0.03)
            assert not operation.done()
            assert worker_services["complete"] == worker_services["fail"] == []
            gate.set()
            with pytest.raises(ValueError, match="private exception text"):
                await asyncio.wait_for(operation, 3)
            events = [event for hb in worker_services["heartbeats"] for event in hb["events"]]
            assert events == [{"sequence": 1}, {"sequence": 2}, {"sequence": 3}]
            assert worker_services["heartbeats"][-1]["progress"] == {"phase": "failed"}
            assert worker_services["fail"] == [{"reason": "reconstruction failed", "details": details}]
            assert worker_services["complete"] == []
            with pytest.raises(auki_sdk.TaskRuntimeError):
                retained[0].get()
        finally:
            gate.set()
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("cause", ["cancel", "heartbeat_failure"])
def test_managed_final_drain_does_not_report_or_replay_after_losing_authority(worker_services, cause):
    async def scenario():
        gate = threading.Event()
        worker_services["heartbeat_gate"] = gate
        returned = asyncio.Event()
        retained = []

        async def handler(task):
            retained.append(task.access_token)
            await task.log_event({"sequence": 1})
            while not worker_services.get("heartbeat_waiting"):
                await asyncio.sleep(0.005)
            await task.set_failure("must not report", {"artifacts": [DATA]})
            returned.set()
            raise RuntimeError("handler finished")

        credential, tasks = runtime(worker_services, handler)
        operation = asyncio.ensure_future(tasks.run_once())
        try:
            await asyncio.wait_for(returned.wait(), 2)
            if cause == "cancel":
                operation.cancel()
            else:
                worker_services["heartbeat_status"] = 503
                gate.set()
            with pytest.raises((asyncio.CancelledError, auki_sdk.TaskRuntimeError)):
                await asyncio.wait_for(operation, 3)
            await tasks.close()
            assert worker_services["complete"] == worker_services["fail"] == []
            assert [e for hb in worker_services["heartbeats"] for e in hb["events"]] == [{"sequence": 1}]
            with pytest.raises(auki_sdk.TaskRuntimeError):
                retained[0].get()
        finally:
            gate.set()
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


def test_task_data_uses_rotated_tokens_and_preserves_denied_writes(worker_services):
    async def scenario():
        async def handler(task):
            data = task.data()
            token = task.access_token
            old_token = token.get()
            assert "fixture" not in repr(token)
            # Reject the cached token; data requests wake the one heartbeat owner.
            worker_services["generation"] += 1
            assert await data.read(DATA) == b"hello"
            assert token.get() != old_token
            worker_services["data_status"] = 403
            with pytest.raises(auki_sdk.DomainDataError) as denied:
                await data.write(b"denied", data_id=DATA)
            assert denied.value.status == 403
            assert "fixture response" not in str(denied.value)
            worker_services["data_status"] = 200
        credential, tasks = runtime(worker_services, handler)
        try:
            assert await tasks.run_once() == "completed"
            assert len(worker_services["heartbeats"]) >= 3
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


@pytest.mark.parametrize("override", [{"domain_id": DATA}, {"aud": ["https://wrong.example"]}, {"iss": "wrong"}, {"exp": 1}])
def test_invalid_grants_never_reach_handler(worker_services, override):
    async def scenario():
        worker_services["claims_override"] = override
        entered = False
        async def handler(task):
            nonlocal entered
            entered = True
        credential, tasks = runtime(worker_services, handler)
        try:
            with pytest.raises(auki_sdk.TaskRuntimeError):
                await tasks.run_once()
            assert not entered
            assert worker_services["heartbeats"] == []
        finally:
            await tasks.close()
            await credential.close()
    asyncio.run(scenario())


def test_cancelled_close_keeps_cleanup_owned_and_other_runtime_usable():
    # An in-loop timeout cannot detect a done callback blocking that same loop.
    from test_process_exit import run_completion_child
    run_completion_child("""
import test_tasks
fixture = test_tasks.worker_services.__wrapped__()
services = next(fixture)
try:
    test_tasks._cancelled_close_keeps_cleanup_owned_and_other_runtime_usable(services)
finally:
    try:
        next(fixture)
    except StopIteration:
        pass
    else:
        raise AssertionError("fixture yielded twice")
print("returned", flush=True)
""")


def _cancelled_close_keeps_cleanup_owned_and_other_runtime_usable(worker_services):
    async def scenario():
        started, cleaning, release, cleaned = (asyncio.Event() for _ in range(4))

        async def blocked(task):
            started.set()
            try:
                await asyncio.Event().wait()
            finally:
                cleaning.set()
                await release.wait()
                cleaned.set()

        async def independent(task):
            return {"output_cids": [], "meta": {"independent": True}}

        first, tasks = runtime(worker_services, blocked)
        second, other = runtime(worker_services, independent)
        operation = asyncio.ensure_future(tasks.run_once())
        try:
            await asyncio.wait_for(started.wait(), 3)
            closing = tasks.close()
            await asyncio.wait_for(cleaning.wait(), 3)
            assert not closing.done()
            closing.cancel()
            with pytest.raises(asyncio.CancelledError):
                await closing
            loop = asyncio.get_running_loop()
            progressed = loop.create_future()
            def check_loop_progress():
                progressed.set_result(not release.is_set() and not cleaned.is_set())
            # cancel() queued the native cancellation and drain callbacks first.
            # This callback must run while handler cleanup still needs the loop.
            loop.call_soon(check_loop_progress)
            assert await progressed
            assert not release.is_set()
            assert not cleaned.is_set()
            release.set()
            await asyncio.wait_for(asyncio.gather(tasks.close(), tasks.close()), 3)
            assert cleaned.is_set()
            with pytest.raises((asyncio.CancelledError, auki_sdk.TaskRuntimeError)):
                await operation
            await first.close()
            await first.close()
            assert await other.run_once() == "completed"
        finally:
            release.set()
            await tasks.close()
            await first.close()
            await other.close()
            await second.close()
    asyncio.run(scenario())
