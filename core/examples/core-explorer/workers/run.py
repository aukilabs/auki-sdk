"""Private, HTTP-only Jobs Playground workers. --check never imports the SDK."""

import argparse
import asyncio
from dataclasses import dataclass, field
import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import sys
from urllib.parse import urlsplit
from uuid import UUID

sys.dont_write_bytecode = True
MAX_CONFIG = 16384
MAX_INPUT = 65536
MAX_OUTPUT = 65536
TASK_SECONDS = 30


class InvalidConfig(ValueError):
    def __init__(self):
        super().__init__("invalid private worker configuration")


def canonical_uuid(value):
    if not isinstance(value, str) or str(UUID(value)) != value or UUID(value).int == 0:
        raise ValueError("invalid identifier")
    return value


def endpoint(value):
    if not isinstance(value, str) or not value or len(value) > 2048:
        raise InvalidConfig()
    parsed = urlsplit(value)
    if (any(c.isspace() or ord(c) < 32 for c in value)
            or any(c in value for c in ("\\", "?", "#", "<", ">"))
            or not parsed.hostname or parsed.username is not None
            or parsed.password is not None or parsed.port == 0
            or not (parsed.scheme == "https" or
                    (parsed.scheme == "http" and parsed.hostname in ("127.0.0.1", "::1")))):
        raise InvalidConfig()
    return value


@dataclass(frozen=True)
class Config:
    role: str
    domain_id: str
    installation_id: str
    client_id: str
    dds_url: str
    dms_url: str
    registration: str = field(repr=False)
    wallet_key: str = field(default="", repr=False)
    audience: str = ""

    @property
    def capability(self):
        return f"/examples/compute-robot/{self.installation_id}/{self.role}/v1"


def validate_config(value):
    try:
        common = {"role", "domain_id", "installation_id", "client_id",
                  "dds_url", "dms_url", "registration"}
        if not isinstance(value, dict) or value.get("role") not in ("compute", "robot"):
            raise InvalidConfig()
        required = common | ({"wallet_key"} if value["role"] == "compute" else {"audience"})
        if set(value) != required or any(not isinstance(v, str) or not v for v in value.values()):
            raise InvalidConfig()
        for key in ("domain_id", "installation_id", "client_id"):
            canonical_uuid(value[key])
        for key in ("dds_url", "dms_url"):
            endpoint(value[key])
        secret = value["registration"]
        if len(secret) > 8192 or any(c.isspace() for c in secret) or "<" in secret or ">" in secret:
            raise InvalidConfig()
        if value["role"] == "compute":
            wallet = value["wallet_key"].removeprefix("0x")
            if len(wallet) != 64 or any(c not in "0123456789abcdefABCDEF" for c in wallet):
                raise InvalidConfig()
            if not 0 < int(wallet, 16) < 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141:
                raise InvalidConfig()
        else:
            endpoint(value["audience"])
        return Config(**value)
    except (ValueError, TypeError, OverflowError):
        raise InvalidConfig() from None


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise InvalidConfig()
        result[key] = value
    return result


def load_config(filename):
    """Walk directory FDs without following symlinks; require private leaf/parent."""
    directory = None
    try:
        path = Path(filename)
        if not path.is_absolute() or ".." in path.parts:
            raise InvalidConfig()
        directory = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
        for component in path.parts[1:-1]:
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=directory)
            os.close(directory)
            directory = child
        parent = os.fstat(directory)
        if parent.st_uid != os.getuid() or stat.S_IMODE(parent.st_mode) != 0o700:
            raise InvalidConfig()
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        with os.fdopen(fd, "rb") as stream:
            info = os.fstat(stream.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                    or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1
                    or info.st_size > MAX_CONFIG):
                raise InvalidConfig()
            raw = stream.read(MAX_CONFIG + 1)
        if len(raw) > MAX_CONFIG:
            raise InvalidConfig()
        return validate_config(json.loads(raw, object_pairs_hook=unique_object))
    except (OSError, ValueError, TypeError, RecursionError):
        raise InvalidConfig() from None
    finally:
        if directory is not None:
            os.close(directory)


def upstream_handler(role):
    # Load only the existing deterministic functions, avoiding generic module-name
    # collisions and the upstream environment-based runner/P2P configuration.
    root = Path(__file__).resolve().parents[2] / "compute-robot"
    previous = sys.modules.get("common")
    try:
        for name in ("common", role):
            spec = importlib.util.spec_from_file_location(f"_explorer_{name}", root / f"{name}.py")
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            if name == "common":
                sys.modules["common"] = module
        return module.handle
    finally:
        if previous is None:
            sys.modules.pop("common", None)
        else:
            sys.modules["common"] = previous


class BoundedData:
    def __init__(self, data, input_id, output_name, role):
        self.inner, self.input_id = data, input_id
        self.output_name, self.role = output_name, role
        self.read_started = self.write_started = False

    async def read(self, data_id):
        if data_id != self.input_id or self.read_started:
            raise ValueError("input rejected")
        self.read_started = True
        content = bytearray()

        async def sink(chunk):
            if not isinstance(chunk, bytes) or len(content) + len(chunk) > MAX_INPUT:
                raise ValueError("input rejected")
            content.extend(chunk)

        await self.inner.read_to(data_id, sink, max_bytes=MAX_INPUT, max_chunk_bytes=16384)
        return bytes(content)

    async def write(self, content, *, name, data_type):
        expected_type = "example.text.v1" if self.role == "compute" else "example.report.v1"
        if (not isinstance(content, bytes) or len(content) > MAX_OUTPUT
                or name != self.output_name or data_type != expected_type or self.write_started):
            raise ValueError("output rejected")
        self.write_started = True
        # Buffered named creation rejects existing names; never replace by ID or
        # use multipart completion, whose name semantics permit replacement.
        stored = await self.inner.write(content, name=name, data_type=data_type)
        canonical_uuid(stored["id"])
        return {"id": stored["id"]}


class TaskAdapter:
    def __init__(self, task, config):
        meta = task.meta
        if (task.domain_id != config.domain_id or task.capability != config.capability
                or not isinstance(meta, dict) or set(meta) != {"input_id", "run_id"}
                or meta["run_id"] != config.installation_id):
            raise ValueError("task rejected")
        self.id = canonical_uuid(task.id)
        input_id = canonical_uuid(meta["input_id"])
        if task.inputs_cids != [input_id]:
            raise ValueError("task rejected")
        self.domain_id, self.meta = config.domain_id, dict(meta)
        self.task = task
        name = f"sdk-{config.installation_id}-{config.role}-{self.id}"
        self.bounded_data = BoundedData(task.data(), input_id, name, config.role)

    def data(self):
        return self.bounded_data

    async def progress(self, value):
        await self.task.progress(value)

    async def log_event(self, value):
        await self.task.log_event(value)


def make_handler(config):
    handle = upstream_handler(config.role)

    async def bounded(task):
        adapter = None
        try:
            adapter = TaskAdapter(task, config)
            return await asyncio.wait_for(
                handle(adapter, domain_id=config.domain_id, run_id=config.installation_id),
                timeout=TASK_SECONDS)
        except asyncio.CancelledError:
            raise
        except Exception:
            raise RuntimeError("worker task failed") from None
        finally:
            if adapter is not None:
                try:
                    await adapter.bounded_data.inner.close()
                except Exception:
                    raise RuntimeError("worker task failed") from None

    return bounded


async def serve(config, stop, sdk=None):
    if sdk is None:
        import auki_sdk as sdk
    credential = tasks = None
    running = stopping = None
    try:
        options = dict(dds_url=config.dds_url, dms_url=config.dms_url,
                       registration=config.registration, version="1.0.0",
                       client_id=config.client_id, request_timeout=30.0)
        handler = make_handler(config)
        if config.role == "compute":
            credential = sdk.AukiComputeCredential(**options, wallet_key=config.wallet_key)
        else:
            credential = sdk.AukiRobotCredential(**options, audience=config.audience,
                                                capabilities=[config.capability])
        tasks = sdk.AukiDmsTasks(credential, {config.capability: handler},
                               poll_interval=2.0, heartbeat_interval=10.0, request_timeout=30.0)

        async def work():
            if stop.is_set():
                return
            if config.role == "robot":
                assigned = await asyncio.wait_for(credential.assigned_domain_id(), 30)
                if assigned != config.domain_id:
                    raise RuntimeError("worker assignment rejected")
            await asyncio.wait_for(tasks.start(), 30)
            print('{"event":"worker_started"}', flush=True)
            await tasks.run()

        running = asyncio.create_task(work())
        stopping = asyncio.create_task(stop.wait())
        await asyncio.wait((running, stopping), return_when=asyncio.FIRST_COMPLETED)
        if not running.done():
            running.cancel()
        try:
            await running
        except asyncio.CancelledError:
            if not stop.is_set():
                raise
    finally:
        for pending in (running, stopping):
            if pending is not None and not pending.done():
                pending.cancel()
        await asyncio.gather(*(p for p in (running, stopping) if p is not None),
                             return_exceptions=True)
        try:
            if tasks is not None:
                await tasks.close()
        finally:
            if credential is not None:
                await credential.close()


async def run(config):
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    installed = []
    try:
        for sig in (signal.SIGINT, signal.SIGTERM):
            loop.add_signal_handler(sig, stop.set)
            installed.append(sig)
        await serve(config, stop)
    finally:
        for sig in installed:
            loop.remove_signal_handler(sig)


def disable_dumps():
    """Protect private configuration before it is read or native SDK code loads."""
    try:
        import resource
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        if sys.platform == "linux":
            import ctypes
            libc = ctypes.CDLL(None, use_errno=True)
            if libc.prctl(4, 0, 0, 0, 0) != 0:  # PR_SET_DUMPABLE
                raise OSError()
        os.environ["RUST_LOG"] = "off"
    except Exception:
        raise RuntimeError("worker protection failed") from None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, help="absolute private JSON path")
    parser.add_argument("--check", action="store_true", help="validate only; no SDK import or network")
    args = parser.parse_args(argv)
    try:
        disable_dumps()
        config = load_config(args.config)
    except Exception:
        print('{"event":"configuration_rejected"}', file=sys.stderr)
        return 2
    try:
        if args.check:
            print('{"event":"configuration_valid"}')
        else:
            asyncio.run(run(config))
            print('{"event":"worker_stopped"}')
        return 0
    except Exception:
        print('{"event":"worker_failed"}', file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
