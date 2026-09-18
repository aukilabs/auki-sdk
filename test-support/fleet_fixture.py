"""Loopback-only fleet contract fixture shared by Python and Swift host checks."""
import base64
import json
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

DOMAIN = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
ROBOT = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
NODE = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
ORG = "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
CAPABILITY = "vendor.example/inspect/v7"


class FleetFixture:
    def __init__(self):
        self.requests = []
        self.paginated = False
        self.inventory_requests = []
        self.deny_busy = False
        self.wrong_domain = False
        self.block_jobs = False
        self.jobs_started = threading.Event()
        self.release_jobs = threading.Event()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reply(self, value, status=200):
                body = json.dumps(value).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def inventory_reply(self, url, field, record):
                query = parse_qs(url.query)
                fixture.inventory_requests.append((url.path, query))
                if not fixture.paginated:
                    return self.reply({field: [record]})
                if query.get("limit") != ["100"]:
                    return self.reply({}, 400)
                cursor = query.get("cursor")
                if cursor is None:
                    record = dict(record)
                    record["id"] = record["id"][:-1] + ("a" if field == "robots" else "b")
                    next_cursor = "second"
                elif cursor == ["second"]:
                    next_cursor = ""
                else:
                    return self.reply({}, 400)
                return self.reply({field: [record], "pagination": {
                    "version": 1, "limit": 100, "next_cursor": next_cursor}})

            def handle_request(self):
                url = urlparse(self.path)
                self.rfile.read(int(self.headers.get("Content-Length", "0")))
                fixture.requests.append((self.command, url.path))
                if url.path == "/user/login":
                    return self.reply({"access_token": "fixture-user", "refresh_token": "fixture-refresh"})
                if url.path == "/service/domains-access-token":
                    return self.reply({"access_token": "fixture-service"})
                if url.path == f"/api/v1/domains/{DOMAIN}/auth":
                    claims = {"iss": "dds", "type": "user-access", "org": ORG,
                              "domain_id": DOMAIN, "aud": ["dds", fixture.endpoint],
                              "exp": int(time.time()) + 3600}
                    payload = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=")
                    return self.reply({"id": DOMAIN, "domain_server": {"url": fixture.endpoint},
                                       "access_token": f"e30.{payload}.fixture"})
                if self.command != "GET" or not self.headers.get("Authorization", "").startswith("Bearer "):
                    return self.reply({}, 400)
                if url.path == f"/api/v1/domains/{DOMAIN}/robots":
                    return self.inventory_reply(url, "robots", {"id": ROBOT, "organization_id": ORG,
                        "assigned_domain_id": ORG if fixture.wrong_domain else DOMAIN,
                        "name": "inspector", "capabilities": [CAPABILITY], "status": "online",
                        "last_seen_at": "2026-09-17T00:00:00Z", "active_lease_expires_at": None})
                if url.path == "/api/v1/nodes":
                    query = parse_qs(url.query)
                    query.pop("limit", None)
                    query.pop("cursor", None)
                    if query != {"org": ["all"], "staking_status": ["all"]}:
                        return self.reply({}, 400)
                    return self.inventory_reply(url, "nodes", {"id": NODE, "organization_id": ORG,
                        "name": "compute", "capabilities": [CAPABILITY], "status": "online", "mode": "dedicated"})
                if url.path == "/v1/jobs":
                    fixture.jobs_started.set()
                    if fixture.block_jobs:
                        fixture.release_jobs.wait(10)
                    return self.reply({"items": [], "next_cursor": None})
                if url.path == "/v1/nodes/busy":
                    return self.reply({"nodes": []}, 403 if fixture.deny_busy else 200)
                return self.reply({}, 404)

            do_GET = handle_request
            do_POST = handle_request

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.endpoint = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.release_jobs.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


if __name__ == "__main__":
    with FleetFixture() as services:
        services.paginated = True
        subprocess.run([*sys.argv[1:], services.endpoint], check=True, timeout=60)
