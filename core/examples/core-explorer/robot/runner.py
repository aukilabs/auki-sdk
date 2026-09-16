"""Idle diagnostic robot: Echo only, no task polling or execution."""
import asyncio
import json
import os
from pathlib import Path
import signal
import stat
import sys
import time
from urllib.parse import urlsplit


def phase(label, status):
    if os.environ.get("AUKI_SHUTDOWN_TRACE") == "1":
        print(json.dumps({"phase": label, "status": status,
                          "elapsed": round(time.monotonic() - TRACE_START, 3)}),
              file=sys.stderr, flush=True)


TRACE_START = time.monotonic()
CAPABILITY = "/example/core-explorer/idle-only/1"


def configuration(env):
    result = {}
    for name in ("DDS_URL", "DMS_URL", "ROBOT_AUDIENCE", "IDENTITY_FILE", "ROBOT_REGISTRATION"):
        value = env.get("AUKI_" + name, "")
        if not value or value != value.strip():
            raise ValueError("missing or invalid explicit robot configuration")
        result[name] = value
    for name in ("DDS_URL", "DMS_URL"):
        url = urlsplit(result[name])
        if (url.scheme not in ("http", "https") or not url.hostname or url.username
                or url.password or url.query or url.fragment
                or (url.scheme == "http" and url.hostname not in ("127.0.0.1", "::1", "localhost"))):
            raise ValueError("invalid service endpoint")
    identity = Path(result["IDENTITY_FILE"]).absolute()
    identity.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    if identity.parent.is_symlink() or identity.parent.stat().st_mode & 0o077:
        raise ValueError("identity directory must be private")
    if identity.is_symlink() or (identity.exists() and (not stat.S_ISREG(identity.stat().st_mode)
                              or identity.stat().st_mode & 0o077)):
        raise ValueError("identity file must be private")
    result["IDENTITY_FILE"] = str(identity)
    return result


async def reject_task(_task):
    raise RuntimeError("idle diagnostic runner does not execute tasks")


async def serve(config, stop, sdk=None, ready=None):
    if sdk is None:
        import auki_portable_echo as sdk
    credential = tasks = echo = None
    try:
        peer_config = sdk.AukiPeerConfig.new(config["DMS_URL"]).with_dds_tracker(
            "discover_and_advertise", config["DDS_URL"])
        credential = sdk.AukiRobotCredential(
            dds_url=config["DDS_URL"], dms_url=config["DMS_URL"],
            registration=config["ROBOT_REGISTRATION"], audience=config["ROBOT_AUDIENCE"],
            version="0.1.0", client_id="core-explorer-idle-robot", capabilities=[CAPABILITY],
            request_timeout=5, registration_interval=60,
            peer_identity_file=config["IDENTITY_FILE"], peer_config=peer_config)
        tasks = sdk.AukiDmsTasks(credential, {CAPABILITY: reject_task}, request_timeout=5)
        await asyncio.wait_for(tasks.start(), 30)
        peer = tasks.peer()
        if peer is None:
            raise RuntimeError("robot has no assigned peer")
        echo = await asyncio.wait_for(sdk.AukiEcho.mount(peer), 10)
        if ready:
            ready({"state": "ready", "peer": peer.peer_id, "tcp": peer.routes.tcp,
                   "wss": peer.routes.wss})
        stopped = asyncio.ensure_future(peer.wait_stopped())
        requested = asyncio.create_task(stop.wait())
        try:
            done, _ = await asyncio.wait([stopped, requested], return_when=asyncio.FIRST_COMPLETED)
            if stopped in done:
                await stopped
        finally:
            phase("waiter gather", "begin")
            for task in (stopped, requested):
                task.cancel()
            await asyncio.gather(stopped, requested, return_exceptions=True)
            phase("waiter gather", "end")
    finally:
        await close_resources(echo, tasks, credential)


async def close_resources(echo, tasks, credential):
    async def ordered():
        failed = False
        for label, resource in (("Echo close", echo), ("task close", tasks),
                                ("credential close", credential)):
            if resource is not None:
                phase(label, "begin")
                try:
                    await resource.close()
                    phase(label, "end")
                except Exception:
                    phase(label, "failed")
                    failed = True
        if failed:
            raise RuntimeError("robot cleanup failed")

    # Native close owns its work even when Python observation is cancelled.
    # Keep observing the entire ordered sequence before propagating cancellation.
    cleanup = asyncio.create_task(ordered())
    cancelled = False
    while not cleanup.done():
        try:
            await asyncio.shield(cleanup)
        except asyncio.CancelledError:
            cancelled = True
    cleanup.result()
    if cancelled:
        raise asyncio.CancelledError


def failure_category(error):
    # Only fixed labels leave this process; exception text is never printed.
    kind = getattr(error, "kind", None)
    category = kind if kind in ("authority", "authentication", "cleanup", "service", "configuration", "http") else "unclassified"
    for marker, label in (("task peer startup failed", "peer_startup"),
                  ("invalid signed task peer credential", "signed_peer_credential"),
                  ("invalid robot token profile", "robot_token_profile"),
                  ("expiration must be exactly 30 minutes", "p2p_lifetime"),
                  ("invalid machine claims", "machine_claims"),
                  ("robot cleanup failed", "cleanup"),
                  ("timed out", "timeout")):
        if marker in str(error):
            category = label
            break
    return category


async def main():
    config = configuration(os.environ)
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        def received():
            phase("signal receipt", "received")
            stop.set()
        loop.add_signal_handler(sig, received)
    try:
        await serve(config, stop, ready=lambda value: print(json.dumps(value), flush=True))
    finally:
        phase("serve return", "settled")


if __name__ == "__main__":
    try:
        try:
            asyncio.run(main())
        finally:
            phase("asyncio.run return", "settled")
    except Exception as error:
        category = failure_category(error)
        # Backend bodies and credentials must never reach logs.
        print(json.dumps({"state": "failed", "error": "robot operation failed", "category": category}), flush=True)
        raise SystemExit(1)
