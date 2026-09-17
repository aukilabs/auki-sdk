"""Offline HTTP contract tests for peer-free DMS job bindings."""
import asyncio
import base64
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import auki_sdk
import pytest


DOMAIN = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
JOB = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
TASK = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
RECEIPT = "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
DATE = "2026-09-16T00:00:00Z"
CAPABILITY = "com.example.third-party.convert.v1"


@pytest.fixture
def services():
    state = {"requests": [], "block_list": False, "deny_jobs": False}

    def handler(role):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reply(self, value=None, status=200):
                body = json.dumps(value or {}).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except BrokenPipeError:
                    pass

            def handle_request(self):
                url = urlparse(self.path)
                query = parse_qs(url.query, keep_blank_values=True)
                raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                body = json.loads(raw) if raw else None
                state["requests"].append((role, self.command, url.path, query, body))

                if url.path == "/user/login":
                    return self.reply({"access_token": "user", "refresh_token": "refresh"})
                if url.path == "/service/domains-access-token":
                    return self.reply({"access_token": "service"})
                if url.path.endswith("/auth"):
                    claims = {
                        "iss": "dds",
                        "domain_id": DOMAIN,
                        "aud": ["dds", state["endpoint"]],
                        "exp": int(time.time()) + 3600,
                    }
                    payload = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=")
                    return self.reply({
                        "id": DOMAIN,
                        "domain_server": {"url": state["endpoint"]},
                        "access_token": f"e30.{payload}.sig",
                    })

                assert role == "dms"
                assert self.headers["Authorization"].startswith("Bearer ")
                assert self.headers["posemesh-client-id"] == "python-jobs-fixture"
                if state["deny_jobs"]:
                    return self.reply({}, 403)

                if url.path == "/jobs/estimate":
                    assert body["domain_id"] == DOMAIN
                    task = body["tasks"][0]
                    assert task["capability"] == CAPABILITY
                    assert task["mode"] == "dedicated"
                    assert task["capability_filters"] == {"format": "glb"}
                    return self.reply({
                        "total": "2.50",
                        "tasks": [{
                            "label": task["label"], "stage": task["stage"],
                            "capability": task["capability"], "mode": task["mode"],
                            "billing_units": "1.0", "estimated_credit_cost": "2.50",
                        }],
                    })
                if url.path == "/jobs" and self.command == "POST":
                    return self.reply({"job_id": JOB})
                if url.path == "/jobs" and self.command == "GET":
                    if state["block_list"]:
                        time.sleep(1)
                        return self.reply({"items": [], "next_cursor": None})
                    assert query["domain_id"] == [DOMAIN]
                    assert query["capabilities"] == [CAPABILITY]
                    assert query["match_all_capabilities"] == ["true"]
                    return self.reply({"items": [{"job": job(), "tasks_summary": summary()}], "next_cursor": "next"})
                if url.path == f"/jobs/{JOB}" and self.command == "GET":
                    return self.reply(details())
                if url.path == f"/jobs/{JOB}/cancel" and self.command == "POST":
                    return self.reply({"id": JOB, "status": "canceled", "updated_at": DATE})
                return self.reply({}, 404)

            do_GET = do_POST = handle_request

        return Handler

    servers = [ThreadingHTTPServer(("127.0.0.1", 0), handler(role)) for role in ("auth", "dms")]
    state["auth"], state["endpoint"] = [f"http://127.0.0.1:{server.server_port}" for server in servers]
    threads = [threading.Thread(target=server.serve_forever, daemon=True) for server in servers]
    for thread in threads:
        thread.start()
    try:
        yield state
    finally:
        for server, thread in zip(servers, threads):
            server.shutdown()
            server.server_close()
            thread.join()


def job():
    return {
        "id": JOB, "label": "prepare-assets", "domain_id": DOMAIN, "status": "running",
        "priority": 0, "created_at": DATE, "updated_at": DATE, "organization_id": None,
        "meta": {}, "credit_lock_id": None, "credit_lock_amount": "2.50",
        "credit_locked_at": DATE, "credit_released_at": None,
    }


def summary():
    return {"queued": 0, "leased": 0, "running": 1, "completed": 0, "failed": 0, "canceled": 0}


def details():
    task = {
        "id": TASK, "job_id": JOB, "label": "convert", "stage": "convert",
        "capability": CAPABILITY, "capability_filters": {"format": "glb"},
        "status": "running", "deps_remaining": 0, "priority": 0,
        "inputs_cids": ["bafy-input"], "outputs_prefix": "converted/",
        "organization_id": None, "attempts": 1, "max_attempts": 3,
        "lease_expires_at": DATE, "reserved_by": None,
        "meta": {"progress": 0.5}, "cancel_requested_at": None,
        "last_heartbeat_at": DATE, "created_at": DATE, "updated_at": DATE,
        "mode": "dedicated", "billing_units": "1.0",
        "estimated_credit_cost": "2.50", "debited_amount": None, "debited_at": None,
    }
    receipt = {
        "id": RECEIPT, "job_id": JOB, "task_id": TASK, "node_id": None,
        "outputs": ["bafy-output"], "meta": {"attempt": 1}, "created_at": DATE,
    }
    return {"job": job(), "tasks_summary": summary(), "tasks": [task], "receipts": [receipt]}


async def login(services):
    return await auki_sdk.AukiSession.login_with_environment(
        services["auth"], services["auth"], services["endpoint"],
        "fixture@example.com", "fixture", client_id="python-jobs-fixture",
    )


def test_custom_dedicated_job_round_trip_and_close(services):
    async def scenario():
        session = await login(services)
        jobs = session.jobs(DOMAIN)
        spec = {
            "label": "prepare-assets",
            "tasks": [{
                "label": "convert", "stage": "convert", "capability": CAPABILITY,
                "mode": "dedicated", "capability_filters": {"format": "glb"},
                "inputs_cids": ["bafy-input"], "outputs_prefix": "converted/",
            }],
        }
        try:
            assert jobs.domain_id == DOMAIN
            with pytest.raises(ValueError, match="meta must be JSON objects"):
                await jobs.estimate({**spec, "meta": []})
            estimate = await jobs.estimate(spec)
            assert estimate["total"] == "2.50"
            assert await jobs.submit(spec) == JOB
            page = await jobs.list(capabilities=[CAPABILITY], match_all_capabilities=True)
            assert page["next_cursor"] == "next"
            result = await jobs.get(JOB)
            assert result["tasks"][0]["meta"]["progress"] == 0.5
            assert result["receipts"][0]["outputs"] == ["bafy-output"]
            assert (await jobs.cancel(JOB))["status"] == "canceled"
            services["deny_jobs"] = True
            with pytest.raises(auki_sdk.AukiJobsError) as denied:
                await jobs.get(JOB)
            assert denied.value.kind == "http"
            assert denied.value.code == "http_status"
            assert denied.value.status == 403
        finally:
            await jobs.close()
            await session.close()

        with pytest.raises(auki_sdk.AukiJobsError) as closed:
            await jobs.get(JOB)
        assert closed.value.kind == "closed"
        assert closed.value.code == "closed"

    asyncio.run(scenario())


def test_asyncio_cancellation_reaches_native_request(services):
    async def scenario():
        session = await login(services)
        jobs = session.jobs(DOMAIN)
        services["block_list"] = True
        operation = asyncio.ensure_future(jobs.list())
        await asyncio.sleep(0.05)
        operation.cancel()
        with pytest.raises(asyncio.CancelledError):
            await operation
        await jobs.close()
        await session.close()

    asyncio.run(scenario())
