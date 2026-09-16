"""Shared run isolation and worker lifecycle for the paired example."""

import argparse
import asyncio
import math
import signal
import sys
import json
import os
from functools import partial
from uuid import UUID


def build_worker(role, env=None):
    """Construct only; caller must close tasks, then credential, in finally."""
    import auki_sdk
    import compute
    import robot

    env = os.environ if env is None else env
    if role not in ("compute", "robot"):
        raise ValueError("unknown worker role")
    required = ["DDS_BASE_URL", "DMS_BASE_URL", "AUKI_DOMAIN_ID", "AUKI_RUN_ID",
                "AUKI_CLIENT_ID"]
    required += (["NODE_REGISTRATION_CREDENTIAL", "NODE_WALLET_KEY"] if role == "compute"
                 else ["ROBOT_REGISTRATION_CREDENTIAL"])
    if any(not env.get(key) for key in required):
        raise ValueError("missing worker configuration")
    domain_id, run_id = env["AUKI_DOMAIN_ID"], env["AUKI_RUN_ID"]
    for value in (domain_id, run_id):
        if str(UUID(value)) != value:
            raise ValueError("expected canonical UUID")
    capability = f"/examples/compute-robot/{run_id}/{role}/v1"
    options = dict(dds_url=env["DDS_BASE_URL"], dms_url=env["DMS_BASE_URL"],
                   version="1.0.0", client_id=env["AUKI_CLIENT_ID"],
                   peer_identity_file=env.get("AUKI_PEER_IDENTITY_FILE"))
    if options["peer_identity_file"]:
        if not env.get("AUKI_EXPECTED_PEER_ID") or not hasattr(auki_sdk, "AukiInfoEndpoint"):
            raise ValueError("P2P needs the Info build and an expected partner")
        peer_config = auki_sdk.AukiPeerConfig.new(env["DMS_BASE_URL"])
        if env.get("AUKI_P2P_RELAY", "0") not in ("0", "1"):
            raise ValueError("AUKI_P2P_RELAY must be 0 or 1")
        if env.get("AUKI_P2P_RELAY", "0") == "0":
            peer_config = peer_config.direct_only()
        options["peer_config"] = peer_config.with_listen_addresses(["/ip4/127.0.0.1/tcp/0"])
    if role == "compute":
        credential = auki_sdk.AukiComputeCredential(**options,
            registration=env["NODE_REGISTRATION_CREDENTIAL"], wallet_key=env["NODE_WALLET_KEY"])
    else:
        credential = auki_sdk.AukiRobotCredential(**options,
            registration=env["ROBOT_REGISTRATION_CREDENTIAL"],
            audience=env.get("DDS_ROBOT_AUDIENCE"), capabilities=[capability])
    handler = partial(compute.handle if role == "compute" else robot.handle,
                      domain_id=domain_id, run_id=run_id)
    if role == "compute":
        handler = partial(handler, expected_peer_id=env.get("AUKI_EXPECTED_PEER_ID"))
    return credential, auki_sdk.AukiDmsTasks(credential, {capability: handler})


async def run_worker(credential, tasks, role, run_id, *, once=False,
                     timeout=120.0, poll_interval=1.0, expected_peer_id=None):
    endpoint = None
    async def serve():
        nonlocal endpoint
        await tasks.start()
        readiness = {"event": "ready", "role": role, "run_id": run_id}
        if role == "robot" and expected_peer_id:
            import auki_sdk
            from peer_info import info_provider
            peer = tasks.peer()
            if peer is None:
                raise ValueError("robot P2P endpoint requires a configured peer")
            endpoint = auki_sdk.AukiInfoEndpoint.mount(peer,
                info_provider(peer.peer_id, run_id, expected_peer_id))
            readiness.update(peer_id=peer.peer_id,
                             route=peer.routes.tcp or peer.listen_addresses[0])
        print(json.dumps(readiness), flush=True)
        if not once:
            await tasks.run()
            return
        while True:
            outcome = await tasks.run_once()
            if outcome == "completed":
                print(json.dumps({"event": "completed", "role": role}), flush=True)
                return
            await asyncio.sleep(poll_interval)
    try:
        if once:
            await asyncio.wait_for(serve(), timeout=timeout)
        else:
            await serve()
    finally:
        try:
            if endpoint is not None:
                await endpoint.close()
        finally:
            try:
                await tasks.close()
            finally:
                await credential.close()


def main(role):
    parser = argparse.ArgumentParser(description=f"Run the {role} worker on approved services")
    parser.add_argument("--once", action="store_true", help="poll until one task completes")
    parser.add_argument("--timeout", type=float, default=120,
                        help="startup and execution deadline in seconds with --once")
    args = parser.parse_args()

    async def run():
        if not math.isfinite(args.timeout) or args.timeout <= 0:
            raise ValueError("invalid timeout")
        credential, tasks = build_worker(role)
        loop = asyncio.get_running_loop()
        running = asyncio.current_task()
        assert running is not None
        installed = []
        def stop():
            # Ignore repeated signals while awaited cleanup is in progress.
            for sig in installed:
                loop.add_signal_handler(sig, lambda: None)
            running.cancel()
        try:
            for sig in (signal.SIGINT, signal.SIGTERM):
                loop.add_signal_handler(sig, stop)
                installed.append(sig)
            await run_worker(credential, tasks, role, os.environ["AUKI_RUN_ID"],
                             once=args.once, timeout=args.timeout,
                             expected_peer_id=os.environ.get("AUKI_EXPECTED_PEER_ID"))
        except asyncio.CancelledError:
            pass
        finally:
            for sig in installed:
                loop.remove_signal_handler(sig)
    try:
        asyncio.run(run())
        return 0
    except Exception:
        print(json.dumps({"event": "error", "message": "worker failed"}), file=sys.stderr)
        return 1


def validate_task(task, domain_id, run_id):
    meta = task.meta
    if (task.domain_id != domain_id or not isinstance(meta, dict)
            or meta.get("run_id") != run_id
            or not isinstance(meta.get("input_id"), str) or not meta["input_id"]):
        raise ValueError("task outside configured scope or missing input")
