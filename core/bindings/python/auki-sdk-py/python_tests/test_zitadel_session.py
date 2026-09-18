"""Offline coverage for host-owned ZITADEL sessions."""

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
OTHER_DOMAIN = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"


@pytest.fixture
def zitadel_services():
    state = {
        "calls": [],
        "deny": False,
        "deny_listing": False,
        "deny_p2p": False,
        "viewer": False,
        "wrong_claim": False,
        "ordinary_exchanges": 0,
        "p2p_exchanges": 0,
        "ordinary_token": None,
        "p2p_token": None,
    }

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, value=None, status=200):
            body = json.dumps(value or {}).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def request(self):
            url = urlparse(self.path)
            path = url.path
            query = parse_qs(url.query)
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length).decode()
            state["calls"].append(
                (self.command, path, self.headers.get("Authorization"), body)
            )
            if path == "/.well-known/openid-configuration":
                return self.reply({
                    "issuer": state["base"],
                    "token_endpoint": state["base"] + "/oauth/v2/token",
                    "token_endpoint_auth_methods_supported": ["none"],
                    "grant_types_supported": ["refresh_token"],
                })
            if path == "/oauth/v2/token":
                assert "refresh_token=refresh-old" in body
                assert "client_id=python-public-client" in body
                return self.reply({
                    "access_token": "access-replacement",
                    "refresh_token": "refresh-replacement",
                    "expires_in": 3600,
                    "token_type": "Bearer",
                })
            if path == "/service/domains-access-token":
                assert self.headers["Authorization"] == "Bearer access-replacement"
                if query.get("purpose") == ["p2p"]:
                    state["p2p_exchanges"] += 1
                    if state["deny_p2p"]:
                        return self.reply(status=403)
                    now = int(time.time())
                    claims = {
                        "type": "user-p2p-access",
                        "iss": "api",
                        "aud": ["domain-service"],
                        "sub": "fixture-user",
                        "org": "11111111-1111-4111-8111-111111111111",
                        "domains": [DOMAIN, OTHER_DOMAIN],
                        "iat": now,
                        "exp": now + 3600,
                    }
                    payload = base64.urlsafe_b64encode(
                        json.dumps(claims).encode()
                    ).decode().rstrip("=")
                    state["p2p_token"] = "e30." + payload + ".sig"
                    return self.reply({"access_token": state["p2p_token"]})
                assert not query
                state["ordinary_exchanges"] += 1
                if state["deny_listing"]:
                    return self.reply(status=403)
                now = int(time.time())
                claims = {
                    "type": "app-access" if state["viewer"] else "user-access",
                    "iss": "api",
                    "aud": ["domain-service"],
                    "sub": "fixture-user",
                    "org": "11111111-1111-4111-8111-111111111111",
                    "iat": now,
                    "exp": now + 3600,
                }
                if not state["viewer"]:
                    # The released owner profile uses null for an org-wide grant.
                    claims["domains"] = None
                payload = base64.urlsafe_b64encode(
                    json.dumps(claims).encode()
                ).decode().rstrip("=")
                token = "e30." + payload + ".sig"
                state["ordinary_token"] = token
                return self.reply({"access_token": token})
            if path == "/api/v1/domain-discovery/zitadel":
                assert self.headers["Authorization"] == "Bearer access-replacement"
                assert query.get("allows") == ["domain-data:r"]
                return self.reply({"domains": [{"id": DOMAIN, "name": "DDS only", "organization_id": "11111111-1111-4111-8111-111111111111", "permissions": ["domain-data:r"]}], "pagination": {"version": 1, "limit": int(query["limit"][0]), "next_cursor": ""}})
            if path == "/api/v1/domains":
                assert self.headers["Authorization"] == "Bearer " + state["ordinary_token"]
                assert query["org"] == ["own"]
                assert query["issue_token"] == ["false"]
                limit = int(query["limit"][0])
                offset = int(query["offset"][0])
                domains = [
                    {
                        "id": DOMAIN,
                        "name": "First",
                        "organization_id": "11111111-1111-4111-8111-111111111111",
                    },
                    {
                        "id": OTHER_DOMAIN,
                        "name": "Second",
                        "organization_id": "11111111-1111-4111-8111-111111111111",
                    },
                ]
                return self.reply({
                    "domains": domains[offset:offset + limit],
                    "total": len(domains),
                    "limit": limit,
                    "offset": offset,
                })
            if path == "/api/v1/accessible-domains":
                assert self.headers["Authorization"] == "Bearer " + state["ordinary_token"]
                limit = int(query["limit"][0])
                offset = int(query["offset"][0])
                domains = [
                    {"id": DOMAIN, "name": "First", "description": "first"},
                    {"id": OTHER_DOMAIN, "name": "Second", "description": "second"},
                ]
                return self.reply({
                    "domains": domains[offset:offset + limit],
                    "total": len(domains),
                    "limit": limit,
                    "offset": offset,
                })
            if path.endswith("/auth/zitadel"):
                assert self.headers["Authorization"] == "Bearer access-replacement"
                if state["deny"] or OTHER_DOMAIN in path:
                    return self.reply(status=403)
                claim_domain = OTHER_DOMAIN if state["wrong_claim"] else DOMAIN
                claims = {
                    "iss": "dds",
                    "type": "zitadel-user-access",
                    "sub": "fixture-user",
                    "org": "11111111-1111-4111-8111-111111111111",
                    "login_provider": "zitadel",
                    "identity_issuer": state["base"],
                    "scopes": ["domain-metadata:r", "domain-data:r", "pose:r"],
                    "domain_id": claim_domain,
                    "aud": [state["base"]],
                    "exp": int(time.time()) + 3600,
                }
                payload = base64.urlsafe_b64encode(
                    json.dumps(claims).encode()
                ).decode().rstrip("=")
                token = "e30." + payload + ".sig"
                return self.reply({
                    "id": DOMAIN,
                    "domain_server": {"url": state["base"]},
                    "access_token": token,
                })
            if path.endswith("/data"):
                assert self.headers["Authorization"].startswith("Bearer e30.")
                return self.reply({"data": []})
            return self.reply(status=404)

        do_GET = do_POST = request

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


def credentials(base):
    return auki_sdk.ZitadelSessionCredentials(
        "access-old",
        "refresh-old",
        "python-public-client",
        base,
        "2000-01-01T00:00:00Z",
    )


def import_session(state, store):
    base = state["base"]
    return auki_sdk.AukiSession.import_zitadel_with_environment(
        base, base, base, credentials(base), store
    )


def test_refresh_snapshot_is_complete_and_persisted_before_domain_data(zitadel_services):
    async def scenario():
        entered = asyncio.Event()
        release = asyncio.Event()
        saved = []

        async def store(snapshot):
            saved.append((
                snapshot.expose_access_token(),
                snapshot.expose_refresh_token(),
                snapshot.client_id,
                snapshot.issuer,
                snapshot.access_token_expires_at,
            ))
            entered.set()
            await release.wait()

        session = import_session(zitadel_services, store)
        data = session.data(DOMAIN)
        operation = asyncio.ensure_future(data.list())
        await asyncio.wait_for(entered.wait(), 2)
        assert not any(
            path == "/service/domains-access-token"
            for _, path, _, _ in zitadel_services["calls"]
        )
        release.set()
        assert await asyncio.wait_for(operation, 2) == []
        assert saved[0][:4] == (
            "access-replacement",
            "refresh-replacement",
            "python-public-client",
            zitadel_services["base"] + "/",
        )
        assert saved[0][4] is not None
        await data.close()
        await session.close()

    asyncio.run(scenario())


def test_failed_save_retries_same_snapshot_without_another_refresh(zitadel_services):
    async def scenario():
        attempts = []

        async def store(snapshot):
            attempts.append((
                snapshot.expose_access_token(),
                snapshot.expose_refresh_token(),
                snapshot.client_id,
                snapshot.issuer,
                snapshot.access_token_expires_at,
            ))
            if len(attempts) == 1:
                raise RuntimeError("host-storage-secret")

        session = import_session(zitadel_services, store)
        data = session.data(DOMAIN)
        with pytest.raises(auki_sdk.DomainDataError) as failed:
            await data.list()
        assert failed.value.kind == "auth"
        assert failed.value.code == "persistence"
        assert "host-storage-secret" not in str(failed.value)
        assert await data.list() == []
        assert attempts[0] == attempts[1]
        paths = [path for _, path, _, _ in zitadel_services["calls"]]
        assert paths.count("/oauth/v2/token") == 1
        await data.close()
        await session.close()

    asyncio.run(scenario())


def test_cancelled_waiter_does_not_cancel_save_and_close_drains_it(zitadel_services):
    async def scenario():
        entered = asyncio.Event()
        release = asyncio.Event()
        completed = asyncio.Event()

        async def store(_snapshot):
            entered.set()
            try:
                await release.wait()
            finally:
                completed.set()

        session = import_session(zitadel_services, store)
        data = session.data(DOMAIN)
        operation = asyncio.ensure_future(data.list())
        await asyncio.wait_for(entered.wait(), 2)
        operation.cancel()
        with pytest.raises(asyncio.CancelledError):
            await operation
        closing = asyncio.ensure_future(session.close())
        await asyncio.sleep(0)
        assert not closing.done()
        assert not completed.is_set()
        release.set()
        await asyncio.wait_for(closing, 2)
        assert completed.is_set()
        await data.close()

    asyncio.run(scenario())


def test_imported_listing_paginates_and_keeps_data_authority_separate(zitadel_services):
    async def scenario():
        saved = []

        async def store(snapshot):
            saved.append((
                snapshot.expose_access_token(),
                snapshot.expose_refresh_token(),
                snapshot.access_token_expires_at,
            ))

        session = import_session(zitadel_services, store)
        domains = session.domains()
        page = await domains.list(limit=1, offset=1)
        assert page == {
            "domains": [{
                "id": OTHER_DOMAIN,
                "name": "Second",
                "organization_id": "11111111-1111-4111-8111-111111111111",
            }],
            "total": 2,
            "limit": 1,
            "offset": 1,
        }
        assert len(saved) == 1

        choices = await session.accessible_domains()
        assert [choice.id for choice in choices] == [DOMAIN, OTHER_DOMAIN]

        zitadel_services["deny_listing"] = True
        with pytest.raises(auki_sdk.DomainDataError) as denied:
            await domains.list(limit=1)
        assert denied.value.kind == "auth"
        assert denied.value.status == 403
        assert denied.value.code == "authorization_denied"

        # An ordinary listing denial is recoverable and does not poison the
        # selected-Domain data path or repeat credential persistence.
        zitadel_services["deny_listing"] = False
        data = session.data(DOMAIN)
        assert await data.list() == []
        await data.close()
        assert len(saved) == 1
        assert zitadel_services["ordinary_exchanges"] == 3
        assert zitadel_services["p2p_exchanges"] == 0

        before = len(zitadel_services["calls"])
        with pytest.raises(auki_sdk.DomainDataError) as unsupported_filter:
            await domains.list(organization=OTHER_DOMAIN)
        assert unsupported_filter.value.code == "configuration"
        with pytest.raises(auki_sdk.DomainDataError) as unsupported_server:
            await domains.list(domain_server_id=DOMAIN)
        assert unsupported_server.value.code == "configuration"
        with pytest.raises(auki_sdk.DomainDataError) as unsupported_portal:
            await domains.for_portal("ABC12345678")
        assert unsupported_portal.value.code == "configuration"
        assert len(zitadel_services["calls"]) == before
        await session.close()

        # A viewer-shaped ordinary profile must fail closed into the narrower
        # P2P grant. Its 403 remains a structured, recoverable listing denial.
        zitadel_services["viewer"] = True
        zitadel_services["deny_p2p"] = True

        async def viewer_store(_snapshot):
            pass

        viewer = import_session(zitadel_services, viewer_store)
        with pytest.raises(auki_sdk.DomainDataError) as denied_viewer:
            await viewer.domains().list(limit=1)
        assert denied_viewer.value.kind == "auth"
        assert denied_viewer.value.status == 403
        assert denied_viewer.value.code == "authorization_denied"
        assert zitadel_services["ordinary_exchanges"] == 4
        assert zitadel_services["p2p_exchanges"] == 1
        await viewer.close()

    asyncio.run(scenario())


def test_invalid_credentials_denial_wrong_domain_and_listing_are_bounded(zitadel_services):
    with pytest.raises(RuntimeError) as invalid:
        auki_sdk.ZitadelSessionCredentials("", "refresh", "client", zitadel_services["base"])
    assert invalid.value.code == "configuration"
    with pytest.raises(RuntimeError):
        auki_sdk.ZitadelSessionCredentials(
            "access", "refresh", "client", "https://issuer.example/?secret=x"
        )
    valid = credentials(zitadel_services["base"])
    assert "access-old" not in repr(valid)
    assert "refresh-old" not in str(valid)

    async def scenario():
        with pytest.raises(RuntimeError):
            auki_sdk.AukiSession.import_zitadel_with_environment(
                zitadel_services["base"],
                zitadel_services["base"],
                zitadel_services["base"],
                valid,
                object(),
            )

        async def store(_snapshot):
            pass

        session = import_session(zitadel_services, store)
        zitadel_services["deny"] = True
        with pytest.raises(auki_sdk.DomainDataError) as denied:
            await session.data(OTHER_DOMAIN).list()
        assert denied.value.status == 403
        assert denied.value.code == "authorization_denied"

        zitadel_services["deny"] = False
        zitadel_services["wrong_claim"] = True
        with pytest.raises(auki_sdk.DomainDataError) as wrong:
            await session.data(DOMAIN).list()
        assert wrong.value.kind == "auth"
        assert wrong.value.code == "transient"
        await session.close()

    asyncio.run(scenario())


def test_permission_discovery_uses_dds_without_api_exchange(zitadel_services):
    async def scenario():
        saved = []

        async def store(snapshot):
            saved.append(snapshot.expose_access_token())

        session = import_session(zitadel_services, store)
        try:
            page = await session.domains().discover(allows=["domain-data:r"], limit=1)
            assert page["domains"][0]["id"] == DOMAIN
            assert page["domains"][0]["permissions"] == ["domain-data:r"]
            assert page["next_cursor"] is None
            assert saved == ["access-replacement"]
            assert zitadel_services["ordinary_exchanges"] == 0
            assert zitadel_services["p2p_exchanges"] == 0
        finally:
            await session.close()

    asyncio.run(scenario())
