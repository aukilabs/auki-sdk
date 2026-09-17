"""Real extension round trips to loopback fixtures; no peer or shared services."""
import asyncio
import importlib.util
from pathlib import Path

import auki_sdk
import pytest

spec = importlib.util.spec_from_file_location(
    "fleet_fixture", Path(__file__).resolve().parents[5] / "test-support/fleet_fixture.py"
)
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


async def login(services):
    return await auki_sdk.AukiSession.login_with_environment(
        services.endpoint, services.endpoint, services.endpoint + "/v1/",
        "fixture@example.test", "fixture-password",
    )


def test_inventory_pool_partial_results_and_shared_session():
    async def scenario(services):
        session = await login(services)
        fleet = session.fleet(fixture.DOMAIN)
        assert fleet.domain_id == fixture.DOMAIN
        try:
            inventory = await fleet.list(capabilities=[fixture.CAPABILITY], match_all_capabilities=True)
            assert inventory["complete"]
            assert inventory["view"] == "domain"
            robot, = inventory["machines"]
            assert (robot["id"], robot["kind"], robot["association"], robot["work_state"]) == (
                fixture.ROBOT, "robot", "assigned", "idle")
            pool = await fleet.compute_pool(mode="dedicated", capabilities=[fixture.CAPABILITY])
            node, = pool["machines"]
            assert (node["id"], node["association"], node["last_seen_at"]) == (fixture.NODE, "candidate", None)
            services.deny_busy = True
            partial = await fleet.list()
            assert not partial["complete"]
            assert partial["machines"][0]["work_state"] == "unknown"
            busy, = [s for s in partial["sources"] if s["source"] == "busy"]
            assert (busy["state"], busy["http_status"]) == ("denied", 403)
            services.wrong_domain = True
            with pytest.raises(auki_sdk.AukiFleetError) as invalid:
                await fleet.list()
            assert invalid.value.code == "invalid_response"
            services.wrong_domain = False
            await fleet.close()
            with pytest.raises(auki_sdk.AukiFleetError) as closed:
                await fleet.list()
            assert closed.value.kind == "closed"
            other = session.fleet(fixture.DOMAIN)
            await other.list()
            await other.close()
        finally:
            await fleet.close()
            await session.close()
        assert all(method == "GET" or path.endswith("/auth") or path in (
            "/user/login", "/service/domains-access-token") for method, path in services.requests)

    with fixture.FleetFixture() as services:
        asyncio.run(scenario(services))


def test_cancellation_and_close_drain_the_real_extension():
    async def scenario(services):
        session = await login(services)
        fleet = session.fleet(fixture.DOMAIN)
        services.block_jobs = True
        operation = asyncio.ensure_future(fleet.list())
        for _ in range(200):
            if services.jobs_started.is_set():
                break
            await asyncio.sleep(0.01)
        assert services.jobs_started.is_set()
        operation.cancel()
        with pytest.raises(asyncio.CancelledError):
            await operation
        await asyncio.wait_for(fleet.close(), 1)
        await session.close()

    with fixture.FleetFixture() as services:
        asyncio.run(scenario(services))
