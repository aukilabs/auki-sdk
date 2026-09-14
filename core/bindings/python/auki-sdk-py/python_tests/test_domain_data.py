"""Offline HTTP integration tests for the shared-session data binding."""
import asyncio
import base64
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import pytest
import auki_sdk

DOMAIN = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
DATA = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
UPLOAD = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
DATE = "2026-09-01T00:00:00Z"


@pytest.fixture
def services():
    state = {"bytes": b"abcdefg", "parts": {}, "aborted": 0, "calls": []}
    metadata = {"id": DATA, "domain_id": DOMAIN, "name": "report", "data_type": "report.v1", "size": 7, "created_at": DATE, "updated_at": DATE}
    portal = {"id": DATA, "short_id": "ABC12345678", "name": "Portal", "size": 10, "created_at": DATE, "updated_at": DATE}
    pose = {"id": DATA, "domain_id": DOMAIN, "short_id": "ABC12345678", "reported_size": 10, "px": 1, "py": 2, "pz": 3, "rx": 0, "ry": 0, "rz": 0, "rw": 1, "scanner_device_id": "fixture", "scanner_device_name": "fixture", "scanner_device_model": "fixture", "placed_at": DATE}

    def handler(role):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reply(self, value=None, status=200):
                body = value if isinstance(value, bytes) else json.dumps(value).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/octet-stream" if isinstance(value, bytes) else "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def handle_request(self):
                url = urlparse(self.path)
                query = parse_qs(url.query, keep_blank_values=True)
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                state["calls"].append((role, self.command, url.path))
                if url.path == "/user/login":
                    return self.reply({"access_token": "user", "refresh_token": "refresh"})
                if url.path == "/service/domains-access-token":
                    return self.reply({"access_token": "service"})
                if url.path == "/api/v1/domains":
                    assert query["issue_token"] == ["false"]
                    assert self.headers["posemesh-client-id"] == "python-fixture"
                    return self.reply({"domains": [{"id": DOMAIN, "name": "Domain"}], "limit": int(query["limit"][0]), "offset": int(query["offset"][0]), "total": 1})
                if url.path.endswith("/auth"):
                    claims = {"iss": "dds", "domain_id": DOMAIN, "aud": ["dds", state["server"]], "exp": int(time.time()) + 3600}
                    token = "e30." + base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=") + ".sig"
                    return self.reply({"id": DOMAIN, "domain_server": {"url": state["server"]}, "access_token": token})
                if url.path == "/api/v1/info":
                    assert "Authorization" not in self.headers
                    return self.reply({"upload": {"domain_data_max_bytes": 10000, "request_max_bytes": 4096, "multipart": {"enabled": True, "part_size_bytes": 4}}})
                if url.path.endswith("/domains") and "/lighthouses/" in url.path:
                    assert query["issue_token"] == ["false"]
                    return self.reply({"domains": [{"id": DOMAIN, "name": "Domain", "is_default": True, "added_to_domain_at": DATE}]})
                if "/lighthouses" in url.path:
                    if role == "dds":
                        return self.reply({"lighthouses": [portal]} if url.path.endswith("/lighthouses") else {**portal, "domain_id": DOMAIN})
                    return self.reply({"poses": [pose]} if url.path.endswith("/lighthouses") else pose)
                if url.path.endswith("/multipart"):
                    if "uploads" in query:
                        state["parts"] = {}
                        return self.reply({"upload_id": UPLOAD, "data_id": DATA, "part_size": 4, "expires_at": "2099-01-01T00:00:00Z"})
                    if self.command == "PUT":
                        number = int(query["partNumber"][0])
                        state["parts"][number] = body
                        return self.reply({"etag": str(number)})
                    if self.command == "DELETE":
                        state["aborted"] += 1
                        return self.reply()
                    parts = json.loads(body)["parts"]
                    state["bytes"] = b"".join(state["parts"][part["part_number"]] for part in parts)
                    return self.reply({**metadata, "size": len(state["bytes"])})
                if self.command == "DELETE":
                    state["deleted"] = True
                    return self.reply()
                if query.get("raw") == ["true"]:
                    return self.reply(state["bytes"])
                if query.get("name") == ["denied"]:
                    return self.reply({"error": "fixture secret must not be exposed"}, 403)
                return self.reply({"data": [metadata]} if url.path.endswith("/data") else metadata)

            do_GET = do_POST = do_PUT = do_DELETE = handle_request
        return Handler

    servers = [ThreadingHTTPServer(("127.0.0.1", 0), handler(role)) for role in ("dds", "data")]
    state["dds"], state["server"] = [f"http://127.0.0.1:{server.server_port}" for server in servers]
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


async def login(services):
    endpoint = services["dds"]
    return await auki_sdk.AukiSession.login_with_environment(endpoint, endpoint, endpoint, "fixture@example.com", "fixture", client_id="python-fixture")


def test_shared_session_portals_poses_and_streaming(services):
    async def scenario():
        session = await login(services)
        data = session.data(DOMAIN)
        try:
            domains = session.domains()
            assert (await domains.list(limit=1))["total"] == 1
            assert (await domains.for_portal("abc12345678"))[0]["is_default"]
            assert (await domains.portals(DOMAIN))[0]["id"] == DATA
            assert (await domains.portal(DOMAIN, DATA))["id"] == DATA
            assert (await data.poses())[0]["px"] == 1
            assert (await data.pose(DATA))["domain_id"] == DOMAIN
            received = bytearray()

            async def sink(chunk):
                assert len(chunk) <= 2
                received.extend(chunk)

            assert await data.read_to(DATA, sink, max_chunk_bytes=2) == 7
            assert received == b"abcdefg"
            remaining = bytearray(b"1234567")

            async def source(maximum):
                chunk = bytes(remaining[:maximum])
                del remaining[:maximum]
                return chunk

            assert (await data.write_stream(7, source, data_id=DATA))["size"] == 7
            assert await data.read(DATA) == b"1234567"
            assert services["parts"] == {1: b"1234", 2: b"567"}
            with pytest.raises(auki_sdk.DomainDataError) as denied:
                await data.list(name="denied")
            assert denied.value.status == 403
            assert "fixture secret" not in str(denied.value)
            await data.close()
            other = session.data(DOMAIN)
            assert await other.read(DATA) == b"1234567"
            await other.delete(DATA)
            await other.close()
        finally:
            await data.close()
            await session.close()
    asyncio.run(scenario())


def test_python_cancellation_cancels_callback_and_aborts_upload(services):
    async def scenario():
        session = await login(services)
        data = session.data(DOMAIN)
        entered, stopped = asyncio.Event(), asyncio.Event()

        async def source(_maximum):
            entered.set()
            try:
                await asyncio.Future()
            finally:
                stopped.set()

        try:
            task = asyncio.ensure_future(data.write_stream(7, source, data_id=DATA))
            await asyncio.wait_for(entered.wait(), 2)
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
            await asyncio.wait_for(data.close(), 2)
            await asyncio.wait_for(stopped.wait(), 2)
            assert services["aborted"] == 1
        finally:
            await data.close()
            await session.close()
    asyncio.run(scenario())


def test_source_exception_aborts_and_does_not_expose_callback_details(services):
    async def scenario():
        session = await login(services)
        data = session.data(DOMAIN)

        async def source(_maximum):
            raise RuntimeError("private file content")

        try:
            with pytest.raises(auki_sdk.DomainDataError) as failure:
                await data.write_stream(7, source, data_id=DATA)
            assert failure.value.kind == "callback"
            assert "private file content" not in str(failure.value)
            assert services["aborted"] == 1
        finally:
            await data.close()
            await session.close()
    asyncio.run(scenario())
